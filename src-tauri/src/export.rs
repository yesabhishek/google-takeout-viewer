//! Export selected items to a folder: as originals or converted to
//! JPEG / PNG / TIFF / HEIC / WebP, optionally resized, with capture dates
//! written to the file times.

use rayon::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::state::AppState;
use crate::takeout;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub format: String,
    pub quality: Option<f64>,
    pub max_px: Option<u32>,
    pub include_videos: Option<bool>,
    pub include_motion: Option<bool>,
    pub keep_dates: Option<bool>,
}

pub struct Format {
    pub id: &'static str,
    pub label: &'static str,
    pub uti: &'static str,
    pub ext: &'static str,
}

pub const FORMATS: &[Format] = &[
    Format { id: "jpeg", label: "JPEG", uti: "public.jpeg", ext: "jpg" },
    Format { id: "png", label: "PNG", uti: "public.png", ext: "png" },
    Format { id: "heic", label: "HEIC", uti: "public.heic", ext: "heic" },
    Format { id: "webp", label: "WebP", uti: "org.webmproject.webp", ext: "webp" },
    Format { id: "avif", label: "AVIF", uti: "public.avif", ext: "avif" },
    Format { id: "tiff", label: "TIFF", uti: "public.tiff", ext: "tif" },
];

/// Conversion formats available on this machine.
pub fn available_formats() -> Vec<&'static Format> {
    #[cfg(target_os = "macos")]
    {
        let types = crate::mac::writable_types();
        FORMATS.iter().filter(|f| types.iter().any(|t| t == f.uti)).collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        FORMATS.iter().filter(|f| matches!(f.id, "jpeg" | "png" | "tiff" | "webp")).collect()
    }
}

struct ExportItem {
    path: PathBuf,
    name: String,
    kind: i64,
    taken: i64,
    live_video: Option<(PathBuf, String)>,
    motion: bool,
    motion_offset: Option<i64>,
    id: i64,
}

fn load(state: &AppState, id: i64) -> Option<ExportItem> {
    let (path, name, kind, taken, live_id, motion, motion_offset): (String, String, i64, i64, Option<i64>, bool, Option<i64>) = state
        .db
        .read(|c| {
            c.query_row(
                "SELECT path, name, kind, taken_ts, live_video_id, motion, motion_offset FROM items WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
        })
        .ok()?;
    let live_video = live_id.and_then(|v| {
        state
            .db
            .read(|c| {
                c.query_row("SELECT path, name FROM items WHERE id=?1", [v], |r| {
                    Ok((PathBuf::from(r.get::<_, String>(0)?), r.get::<_, String>(1)?))
                })
            })
            .ok()
    });
    Some(ExportItem { path: PathBuf::from(path), name, kind, taken, live_video, motion, motion_offset, id })
}

/// Reserve a file name in `dest` that does not exist yet ("name (2).jpg").
fn reserve(dest: &Path, stem: &str, ext: &str, taken: &Mutex<HashSet<PathBuf>>) -> PathBuf {
    let mut set = taken.lock().unwrap();
    let mut n = 1;
    loop {
        let file = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        let p = dest.join(file);
        if !set.contains(&p) && !p.exists() {
            set.insert(p.clone());
            return p;
        }
        n += 1;
    }
}

fn set_times(p: &Path, ts: i64) {
    let t = filetime::FileTime::from_unix_time(ts, 0);
    let _ = filetime::set_file_times(p, t, t);
}

fn copy_range(src: &Path, offset: u64, dst: &Path) -> std::io::Result<()> {
    let mut f = File::open(src)?;
    f.seek(SeekFrom::Start(offset))?;
    let mut out = File::create(dst)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    Ok(())
}

fn convert(src: &Path, dst: &Path, fmt: &Format, quality: f64, max_px: Option<u32>) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        crate::mac::convert(src, dst, fmt.uti, quality, max_px)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let img = crate::thumbs::portable::decode(src).ok_or("could not decode image")?;
        let img = match max_px {
            Some(px) if img.width().max(img.height()) > px => img.resize(px, px, image::imageops::FilterType::Lanczos3),
            _ => img,
        };
        let mut out = std::io::BufWriter::new(File::create(dst).map_err(|e| e.to_string())?);
        match fmt.id {
            "jpeg" => {
                let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, (quality * 100.0) as u8);
                image::DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(enc).map_err(|e| e.to_string())
            }
            "png" => img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string()),
            "tiff" => img.write_to(&mut out, image::ImageFormat::Tiff).map_err(|e| e.to_string()),
            "webp" => image::DynamicImage::ImageRgba8(img.to_rgba8())
                .write_to(&mut out, image::ImageFormat::WebP)
                .map_err(|e| e.to_string()),
            _ => Err("format not supported".into()),
        }
    }
}

fn export_one(state: &AppState, it: &ExportItem, dest: &Path, o: &ExportOptions, names: &Mutex<HashSet<PathBuf>>) -> Result<(), String> {
    if !it.path.exists() {
        return Err(format!("{}: file is offline", it.name));
    }
    let src_ext = takeout::ext_of(&it.name);
    let stem = takeout::stem_of(&it.name).to_string();
    let keep_dates = o.keep_dates.unwrap_or(true);
    let fmt = FORMATS.iter().find(|f| f.id == o.format);

    if it.kind == 1 {
        if !o.include_videos.unwrap_or(true) {
            return Ok(());
        }
        let out = reserve(dest, &stem, &src_ext, names);
        std::fs::copy(&it.path, &out).map_err(|e| format!("{}: {e}", it.name))?;
        if keep_dates {
            set_times(&out, it.taken);
        }
        return Ok(());
    }

    let same_format = fmt.is_none_or(|f| {
        f.ext == src_ext || (f.id == "jpeg" && matches!(src_ext.as_str(), "jpeg" | "jpe" | "jfif"))
            || (f.id == "tiff" && src_ext == "tiff")
    });
    let out = match fmt {
        Some(f) if !(same_format && o.max_px.is_none()) => {
            let out = reserve(dest, &stem, f.ext, names);
            convert(&it.path, &out, f, o.quality.unwrap_or(0.92), o.max_px)
                .map_err(|e| format!("{}: {e}", it.name))?;
            out
        }
        _ => {
            let out = reserve(dest, &stem, &src_ext, names);
            std::fs::copy(&it.path, &out).map_err(|e| format!("{}: {e}", it.name))?;
            out
        }
    };
    if keep_dates {
        set_times(&out, it.taken);
    }

    if o.include_motion.unwrap_or(false) {
        if let Some((vpath, vname)) = &it.live_video {
            let ext = takeout::ext_of(vname);
            let vout = reserve(dest, &stem, &ext, names);
            std::fs::copy(vpath, &vout).map_err(|e| format!("{vname}: {e}"))?;
            if keep_dates {
                set_times(&vout, it.taken);
            }
        } else if it.motion {
            if let Some(off) = crate::protocol::motion_offset(state, it.id, &it.path, it.motion_offset) {
                let vout = reserve(dest, &stem, "mp4", names);
                copy_range(&it.path, off, &vout).map_err(|e| format!("{}: {e}", it.name))?;
                if keep_dates {
                    set_times(&vout, it.taken);
                }
            }
        }
    }
    Ok(())
}

pub fn start(state: &Arc<AppState>, ids: Vec<i64>, dest: PathBuf, opts: ExportOptions) -> Result<(), String> {
    if state.export.lock().unwrap().running {
        return Err("An export is already running".into());
    }
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    state.export_cancel.store(false, Ordering::SeqCst);
    let total = ids.len();
    state.update_export(|s| {
        *s = crate::state::ExportStatus {
            running: true,
            total,
            dest: dest.to_string_lossy().to_string(),
            ..Default::default()
        }
    });
    let st = state.clone();
    std::thread::spawn(move || {
        let names = Mutex::new(HashSet::new());
        let done = AtomicUsize::new(0);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().expect("export pool");
        pool.install(|| {
            ids.par_iter().for_each(|&id| {
                if st.export_cancel.load(Ordering::SeqCst) {
                    return;
                }
                let res = match load(&st, id) {
                    Some(it) => export_one(&st, &it, &dest, &opts, &names).map(|_| it.name),
                    None => Err(format!("item {id} not found")),
                };
                let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                st.update_export(|s| {
                    s.done = d;
                    match res {
                        Ok(name) => s.current = name,
                        Err(e) => {
                            s.failed += 1;
                            if s.errors.len() < 50 {
                                s.errors.push(e);
                            }
                        }
                    }
                });
            });
        });
        st.update_export(|s| s.running = false);
        st.emit("export-done", serde_json::to_value(st.export.lock().unwrap().clone()).unwrap_or_default());
    });
    Ok(())
}
