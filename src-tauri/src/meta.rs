//! Per-file metadata: fingerprint, display size, capture time, camera, GPS,
//! and motion-photo detection. Reads only the head of each file.

use chrono::{Local, NaiveDate, TimeZone};
use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use xxhash_rust::xxh3::xxh3_64;

use crate::video;

const HEAD: usize = 128 * 1024;

#[derive(Debug, Default, Clone)]
pub struct FileMeta {
    pub fp: i64,
    pub width: u32,
    pub height: u32,
    pub meta_ts: Option<i64>,
    pub camera: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub duration_ms: Option<u32>,
    pub motion: bool,
}

fn read_head(f: &mut File) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(HEAD);
    f.take(HEAD as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn fingerprint(head: &[u8], size: u64) -> i64 {
    (xxh3_64(head) ^ size.wrapping_mul(0x9E37_79B9_7F4A_7C15)) as i64
}

/// Read what the index needs from one file. `None` means the file could not be
/// read at all (drive unplugged, I/O error); it is then left for the next scan
/// rather than indexed with made-up values.
pub fn extract(path: &Path, kind: i64, ext: &str, size: u64) -> Option<FileMeta> {
    let mut m = FileMeta::default();
    let mut f = File::open(path).ok()?;
    let head = read_head(&mut f).ok()?;
    if head.is_empty() && size > 0 {
        return None;
    }
    m.fp = fingerprint(&head, size);
    if kind == 1 {
        if let Some(v) = video::probe(path) {
            m.width = v.width;
            m.height = v.height;
            m.duration_ms = Some(v.duration_ms);
            m.meta_ts = v.creation_ts;
        }
        return Some(m);
    }

    // Size from the header bytes (fast path), falling back to the platform decoder.
    let mut orientation = 1u32;
    let native_size = matches!(
        ext,
        "heic" | "heif" | "hif" | "avif" | "dng" | "cr2" | "cr3" | "nef" | "nrw" | "arw" | "srf"
            | "sr2" | "orf" | "rw2" | "raf" | "pef" | "srw" | "raw"
    );
    if !native_size {
        if let Ok(sz) = imagesize::blob_size(&head) {
            m.width = sz.width as u32;
            m.height = sz.height as u32;
        }
    }

    // EXIF
    let exif = exif::Reader::new()
        .read_from_container(&mut Cursor::new(&head))
        .ok()
        .or_else(|| {
            // HEIF/PNG/WebP may keep EXIF beyond the first block.
            if matches!(ext, "jpg" | "jpeg" | "jpe" | "jfif") {
                return None;
            }
            let mut r = BufReader::new(File::open(path).ok()?);
            r.seek(SeekFrom::Start(0)).ok()?;
            exif::Reader::new().read_from_container(&mut r).ok()
        });
    if let Some(ex) = &exif {
        read_exif(ex, &mut m, &mut orientation);
    }

    if m.width == 0 || native_size {
        #[cfg(target_os = "macos")]
        if let Some((w, h, o)) = crate::mac::image_props(path) {
            m.width = w;
            m.height = h;
            orientation = o;
        }
        #[cfg(target_os = "windows")]
        let native = crate::win::image_props(path).map(|(w, h, o)| {
            m.width = w;
            m.height = h;
            if let Some(o) = o {
                orientation = o;
            }
        });
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let native: Option<()> = None;
        #[cfg(not(target_os = "macos"))]
        if native.is_none() {
            if let Ok(sz) = imagesize::size(path) {
                m.width = sz.width as u32;
                m.height = sz.height as u32;
            }
        }
    }
    if (5..=8).contains(&orientation) {
        std::mem::swap(&mut m.width, &mut m.height);
    }

    let lname = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    m.motion = lname.ends_with(".mp.jpg")
        || lname.starts_with("mvimg_")
        || contains(&head, b"MotionPhoto")
        || contains(&head, b"MicroVideo");
    Some(m)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn ascii(f: &exif::Field) -> Option<String> {
    if let exif::Value::Ascii(v) = &f.value {
        let s = v
            .iter()
            .map(|b| String::from_utf8_lossy(b).trim_matches(char::from(0)).trim().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    } else {
        None
    }
}

fn read_exif(ex: &exif::Exif, m: &mut FileMeta, orientation: &mut u32) {
    use exif::{In, Tag};
    let get = |t: Tag| ex.get_field(t, In::PRIMARY);
    if let Some(o) = get(Tag::Orientation).and_then(|f| f.value.get_uint(0)) {
        *orientation = o;
    }
    if m.width == 0 {
        if let (Some(w), Some(h)) = (
            get(Tag::PixelXDimension).and_then(|f| f.value.get_uint(0)),
            get(Tag::PixelYDimension).and_then(|f| f.value.get_uint(0)),
        ) {
            m.width = w;
            m.height = h;
        }
    }
    let date_tags = [
        (Tag::DateTimeOriginal, Tag::OffsetTimeOriginal),
        (Tag::DateTimeDigitized, Tag::OffsetTimeDigitized),
        (Tag::DateTime, Tag::OffsetTime),
    ];
    for (dt_tag, off_tag) in date_tags {
        if let Some(ts) = get(dt_tag).and_then(|f| exif_ts(f, get(off_tag))) {
            m.meta_ts = Some(ts);
            break;
        }
    }
    let make = get(Tag::Make).and_then(ascii);
    let model = get(Tag::Model).and_then(ascii);
    m.camera = match (make, model) {
        (Some(mk), Some(md)) if md.to_lowercase().starts_with(&mk.to_lowercase()) => Some(md),
        (Some(mk), Some(md)) => Some(format!("{mk} {md}")),
        (None, Some(md)) => Some(md),
        (Some(mk), None) => Some(mk),
        _ => None,
    };
    let coord = |v: Tag, r: Tag| -> Option<f64> {
        let f = get(v)?;
        let deg = match &f.value {
            exif::Value::Rational(r) if r.len() >= 3 => {
                r[0].to_f64() + r[1].to_f64() / 60.0 + r[2].to_f64() / 3600.0
            }
            _ => return None,
        };
        let neg = get(r)
            .and_then(ascii)
            .map(|s| s.starts_with('S') || s.starts_with('W'))
            .unwrap_or(false);
        if !deg.is_finite() || deg == 0.0 {
            None
        } else {
            Some(if neg { -deg } else { deg })
        }
    };
    m.lat = coord(Tag::GPSLatitude, Tag::GPSLatitudeRef);
    m.lon = coord(Tag::GPSLongitude, Tag::GPSLongitudeRef);
    if m.lat.is_none() || m.lon.is_none() {
        m.lat = None;
        m.lon = None;
    }
}

fn exif_ts(f: &exif::Field, off: Option<&exif::Field>) -> Option<i64> {
    let raw = match &f.value {
        exif::Value::Ascii(v) => v.first()?.clone(),
        _ => return None,
    };
    let mut dt = exif::DateTime::from_ascii(&raw).ok()?;
    if let Some(o) = off {
        if let exif::Value::Ascii(v) = &o.value {
            if let Some(b) = v.first() {
                let _ = dt.parse_offset(b);
            }
        }
    }
    if dt.year < 1900 || dt.month == 0 || dt.day == 0 {
        return None;
    }
    let naive = NaiveDate::from_ymd_opt(dt.year as i32, dt.month as u32, dt.day as u32)?
        .and_hms_opt(dt.hour as u32, dt.minute as u32, dt.second.min(59) as u32)?;
    match dt.offset {
        Some(mins) => Some(naive.and_utc().timestamp() - mins as i64 * 60),
        // No offset recorded: EXIF time is wall-clock time; treat it as local.
        None => Local.from_local_datetime(&naive).earliest().map(|d| d.timestamp()),
    }
}

/// Find the MP4 embedded in a motion photo (Pixel `.MP.jpg`, Samsung, MVIMG).
/// Streams the file in 1 MB blocks rather than loading it whole.
pub fn find_motion_offset(path: &Path) -> Option<u64> {
    const BRANDS: &[&[u8; 4]] = &[b"mp42", b"isom", b"mp41", b"avc1", b"iso2", b"iso4", b"iso5", b"iso6", b"qt  ", b"MSNV"];
    const BLOCK: usize = 1 << 20;
    const OVERLAP: usize = 12; // a box header split across two blocks: size + "ftyp" + brand
    let mut f = File::open(path).ok()?;
    if f.metadata().ok()?.len() > 200 * 1024 * 1024 {
        return None;
    }
    let mut buf: Vec<u8> = Vec::with_capacity(BLOCK + OVERLAP);
    let mut base = 0u64; // file offset of buf[0]
    let mut block = vec![0u8; BLOCK];
    loop {
        let n = f.read(&mut block).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&block[..n]);
        let mut i = 4usize;
        while i + 8 <= buf.len() {
            let at = base + i as u64 - 4;
            if &buf[i..i + 4] == b"ftyp" && at >= 60 {
                let size = u32::from_be_bytes(buf[i - 4..i].try_into().ok()?);
                if (12..=256).contains(&size) && BRANDS.iter().any(|b| b[..] == buf[i + 4..i + 8]) {
                    return Some(at);
                }
            }
            i += 1;
        }
        let keep = buf.len().min(OVERLAP);
        base += (buf.len() - keep) as u64;
        buf.drain(..buf.len() - keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_offset() {
        let dir = std::env::temp_dir().join(format!("tv-motion-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.MP.jpg");
        let mut data = vec![0xFFu8, 0xD8];
        data.extend(vec![1u8; 500]);
        data.extend([0xFF, 0xD9]);
        let at = data.len();
        data.extend(24u32.to_be_bytes());
        data.extend(b"ftypisom");
        data.extend([0u8; 12]);
        std::fs::write(&p, &data).unwrap();
        assert_eq!(find_motion_offset(&p), Some(at as u64));
        std::fs::remove_dir_all(&dir).ok();
    }
}
