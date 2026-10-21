//! Export selected items to a folder: as originals or converted to
//! JPEG / PNG / TIFF / HEIC / WebP, optionally resized, with capture dates
//! written to the file times.
//!
//! Safety rules: originals are only read. Every output gets a name nobody else
//! has (`create_new`, so an existing file is never overwritten), is written to
//! a `.partial` file, checked, flushed to disk and only then renamed into
//! place. Exporting into a Takeout folder is refused.

use rayon::prelude::*;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::diskio::{self, Prio, DRIVE};
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

fn file_name(stem: &str, ext: &str, n: u32) -> String {
    let stem = if n == 1 { stem.to_string() } else { format!("{stem} ({n})") };
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Claim a free file name in `dest` ("name (2).jpg" if taken) by creating it
/// empty. `create_new` is atomic, so a file that already exists, whoever made
/// it, is never overwritten.
fn reserve(dest: &Path, stem: &str, ext: &str) -> std::io::Result<PathBuf> {
    for n in 1..100_000 {
        let p = dest.join(file_name(stem, ext, n));
        match OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(ErrorKind::AlreadyExists, "too many files with this name"))
}

/// Write one output: into `<name>.partial`, flushed to disk, then renamed over
/// the empty file `reserve` made. On failure both are removed (they are ours),
/// so a cancelled or failed export never leaves half-written files behind.
fn write_reserved(out: &Path, write: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    let name = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mut tmp = None;
    for n in 0..100 {
        let p = out.with_file_name(if n == 0 { format!("{name}.partial") } else { format!("{name}.{n}.partial") });
        match OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(_) => {
                tmp = Some(p);
                break;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                let _ = std::fs::remove_file(out);
                return Err(e.to_string());
            }
        }
    }
    let Some(tmp) = tmp else {
        let _ = std::fs::remove_file(out);
        return Err("could not create a temporary file".into());
    };
    let res = write(&tmp).and_then(|_| {
        OpenOptions::new().write(true).open(&tmp).and_then(|f| f.sync_all()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, out).map_err(|e| e.to_string())
    });
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::remove_file(out);
    }
    res
}

fn set_times(p: &Path, ts: i64) {
    let t = filetime::FileTime::from_unix_time(ts, 0);
    let _ = filetime::set_file_times(p, t, t);
}

/// Copy `src` from `offset` to the end into `dst`, and check every byte arrived.
pub fn copy_verified(src: &Path, offset: u64, dst: &Path) -> std::io::Result<()> {
    let mut f = File::open(src)?;
    let expected = f.metadata()?.len().saturating_sub(offset);
    f.seek(SeekFrom::Start(offset))?;
    let mut out = OpenOptions::new().write(true).truncate(true).open(dst)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut copied = 0u64;
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        out.write_all(&buf[..n])?;
        copied += n as u64;
    }
    out.flush()?;
    if copied != expected || out.metadata()?.len() != expected {
        return Err(std::io::Error::other(format!("copy incomplete ({copied} of {expected} bytes)")));
    }
    Ok(())
}

/// A readable error, stopping the whole export when the destination is full.
fn io_fail(state: &AppState, what: &str, e: std::io::Error) -> String {
    if e.kind() == ErrorKind::StorageFull {
        state.export_cancel.store(true, Ordering::SeqCst);
        return "The destination is full, so the export was stopped.".into();
    }
    format!("{what}: {e}")
}

/// Copy a whole original (or the clip inside a motion photo) to a new file.
fn export_copy(state: &AppState, src: &Path, offset: u64, dest: &Path, stem: &str, ext: &str, what: &str) -> Result<PathBuf, String> {
    let out = reserve(dest, stem, ext).map_err(|e| io_fail(state, what, e))?;
    let mut copy_err = None;
    write_reserved(&out, |tmp| {
        copy_verified(src, offset, tmp).map_err(|e| {
            let msg = e.to_string();
            copy_err = Some(e);
            msg
        })
    })
    .map_err(|e| match copy_err.take() {
        Some(io) => io_fail(state, what, io),
        None => format!("{what}: {e}"),
    })?;
    Ok(out)
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

fn export_one(state: &AppState, it: &ExportItem, dest: &Path, o: &ExportOptions) -> Result<(), String> {
    let _drive = DRIVE.acquire(Prio::Background);
    if !it.path.exists() {
        return Err(format!("{}: the drive with this file isn't connected", it.name));
    }
    let src_ext = takeout::ext_of(&it.name);
    let stem = takeout::stem_of(&it.name).to_string();
    let keep_dates = o.keep_dates.unwrap_or(true);
    let fmt = FORMATS.iter().find(|f| f.id == o.format);

    if it.kind == 1 {
        if !o.include_videos.unwrap_or(true) {
            return Ok(());
        }
        let out = export_copy(state, &it.path, 0, dest, &stem, &src_ext, &it.name)?;
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
            let out = reserve(dest, &stem, f.ext).map_err(|e| io_fail(state, &it.name, e))?;
            write_reserved(&out, |tmp| convert(&it.path, tmp, f, o.quality.unwrap_or(0.92), o.max_px))
                .map_err(|e| format!("{}: {e}", it.name))?;
            out
        }
        _ => export_copy(state, &it.path, 0, dest, &stem, &src_ext, &it.name)?,
    };
    if keep_dates {
        set_times(&out, it.taken);
    }

    if o.include_motion.unwrap_or(false) {
        if let Some((vpath, vname)) = &it.live_video {
            let vout = export_copy(state, vpath, 0, dest, &stem, &takeout::ext_of(vname), vname)?;
            if keep_dates {
                set_times(&vout, it.taken);
            }
        } else if it.motion {
            if let Some(off) = crate::protocol::motion_offset(state, it.id, &it.path, it.motion_offset) {
                let vout = export_copy(state, &it.path, off, dest, &stem, "mp4", &it.name)?;
                if keep_dates {
                    set_times(&vout, it.taken);
                }
            }
        }
    }
    Ok(())
}

/// Exports go next to your library, never into it.
fn check_destination(state: &AppState, dest: &Path) -> Result<(), String> {
    let sources: Vec<String> = state.db.read(|c| {
        let mut st = c.prepare("SELECT path FROM sources")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    })?;
    if sources.iter().any(|s| diskio::inside(dest, Path::new(s))) {
        return Err("Choose a folder outside your Takeout folder. Takeout Viewer never writes into your originals.".into());
    }
    if diskio::inside(dest, &state.data_dir) {
        return Err("Choose another folder: that one holds Takeout Viewer's own index and thumbnails.".into());
    }
    Ok(())
}

pub fn start(state: &Arc<AppState>, ids: Vec<i64>, dest: PathBuf, opts: ExportOptions) -> Result<(), String> {
    if state.export.lock().unwrap().running {
        return Err("An export is already running".into());
    }
    check_destination(state, &dest)?;
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
        let done = AtomicUsize::new(0);
        let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build().expect("export pool");
        pool.install(|| {
            ids.par_iter().for_each(|&id| {
                if st.export_cancel.load(Ordering::SeqCst) {
                    return;
                }
                let res = match load(&st, id) {
                    Some(it) => export_one(&st, &it, &dest, &opts).map(|_| it.name),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tv-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn never_overwrites() {
        let d = tmpdir("reserve");
        std::fs::write(d.join("a.jpg"), b"precious").unwrap();
        assert_eq!(reserve(&d, "a", "jpg").unwrap(), d.join("a (2).jpg"));
        assert_eq!(reserve(&d, "a", "jpg").unwrap(), d.join("a (3).jpg"));
        assert_eq!(std::fs::read(d.join("a.jpg")).unwrap(), b"precious");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn failed_write_leaves_nothing() {
        let d = tmpdir("partial");
        let out = reserve(&d, "x", "jpg").unwrap();
        let r = write_reserved(&out, |tmp| {
            std::fs::write(tmp, b"half").unwrap();
            Err("boom".into())
        });
        assert!(r.is_err());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn verified_copy() {
        let d = tmpdir("copy");
        let src = d.join("src.bin");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&src, &data).unwrap();
        let out = reserve(&d, "copy", "bin").unwrap();
        write_reserved(&out, |tmp| copy_verified(&src, 0, tmp).map_err(|e| e.to_string())).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), data);
        assert!(!d.join("copy.bin.partial").exists());
        std::fs::remove_dir_all(&d).ok();
    }
}
