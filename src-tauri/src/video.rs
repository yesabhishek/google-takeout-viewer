//! Minimal ISO-BMFF (MP4 / MOV / 3GP) reader: just enough to get the display
//! size, duration and creation time without touching the media data.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Default, Clone, Copy)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
    pub creation_ts: Option<i64>,
    /// Clockwise rotation (0/90/180/270) the player applies to the stored frames.
    pub rotation: u16,
}

const MAC_EPOCH_OFFSET: i64 = 2_082_844_800; // 1904-01-01 -> 1970-01-01

fn be_u32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn be_i32(b: &[u8], o: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn be_u64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_be_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Iterate child boxes inside `buf`: yields (type, payload).
fn boxes(buf: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        if pos + 8 > buf.len() {
            return None;
        }
        let size32 = be_u32(buf, pos)? as usize;
        let typ: [u8; 4] = buf[pos + 4..pos + 8].try_into().ok()?;
        let (hdr, size) = match size32 {
            0 => (8, buf.len() - pos),
            1 => (16, be_u64(buf, pos + 8)? as usize),
            n => (8, n),
        };
        if size < hdr || pos + size > buf.len() {
            return None;
        }
        let payload = &buf[pos + hdr..pos + size];
        pos += size;
        Some((typ, payload))
    })
}

fn find<'a>(buf: &'a [u8], typ: &[u8; 4]) -> Option<&'a [u8]> {
    boxes(buf).find(|(t, _)| t == typ).map(|(_, p)| p)
}

/// Locate the `moov` box at the top level and read it into memory.
fn read_moov(f: &mut File, len: u64) -> Option<Vec<u8>> {
    let mut pos = 0u64;
    let mut hdr = [0u8; 16];
    while pos + 8 <= len {
        f.seek(SeekFrom::Start(pos)).ok()?;
        f.read_exact(&mut hdr[..8]).ok()?;
        let size32 = u32::from_be_bytes(hdr[0..4].try_into().ok()?) as u64;
        let typ: [u8; 4] = hdr[4..8].try_into().ok()?;
        let (hlen, size) = match size32 {
            0 => (8u64, len - pos),
            1 => {
                f.read_exact(&mut hdr[8..16]).ok()?;
                (16u64, u64::from_be_bytes(hdr[8..16].try_into().ok()?))
            }
            n => (8u64, n),
        };
        if size < hlen {
            return None;
        }
        if &typ == b"moov" {
            let body = size - hlen;
            if body > 256 * 1024 * 1024 {
                return None;
            }
            let mut buf = vec![0u8; body as usize];
            f.read_exact(&mut buf).ok()?;
            return Some(buf);
        }
        pos += size;
    }
    None
}

pub fn probe(path: &Path) -> Option<VideoInfo> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let moov = read_moov(&mut f, len)?;
    probe_moov(&moov)
}

fn probe_moov(moov: &[u8]) -> Option<VideoInfo> {
    let mut info = VideoInfo::default();
    if let Some(mvhd) = find(moov, b"mvhd") {
        let v = *mvhd.first()?;
        let (ctime, timescale, duration) = if v == 1 {
            (be_u64(mvhd, 4)? as i64, be_u32(mvhd, 20)?, be_u64(mvhd, 24)?)
        } else {
            (be_u32(mvhd, 4)? as i64, be_u32(mvhd, 12)?, be_u32(mvhd, 16)? as u64)
        };
        if timescale > 0 {
            info.duration_ms = (duration.saturating_mul(1000) / timescale as u64).min(u32::MAX as u64) as u32;
        }
        if ctime > MAC_EPOCH_OFFSET + 86_400 * 365 {
            info.creation_ts = Some(ctime - MAC_EPOCH_OFFSET);
        }
    }
    for (t, trak) in boxes(moov) {
        if &t != b"trak" {
            continue;
        }
        let is_video = find(trak, b"mdia")
            .and_then(|m| find(m, b"hdlr"))
            .map(|h| h.get(8..12) == Some(b"vide"))
            .unwrap_or(false);
        if !is_video {
            continue;
        }
        let tkhd = match find(trak, b"tkhd") {
            Some(t) => t,
            None => continue,
        };
        let v = *tkhd.first()?;
        // offset of the matrix inside tkhd payload
        let m = if v == 1 { 4 + 8 + 8 + 4 + 4 + 8 + 8 + 8 } else { 4 + 4 + 4 + 4 + 4 + 4 + 8 + 8 };
        let a = be_i32(tkhd, m)?;
        let b = be_i32(tkhd, m + 4)?;
        let w = be_u32(tkhd, m + 36)? >> 16;
        let h = be_u32(tkhd, m + 40)? >> 16;
        if w == 0 || h == 0 {
            continue;
        }
        let rotation = match (a.signum(), b.signum()) {
            (0, 1) => 90,
            (0, -1) => 270,
            (-1, 0) => 180,
            _ => 0,
        };
        let rotated = a == 0 && b != 0; // 90 or 270 degrees
        let (w, h) = if rotated { (h, w) } else { (w, h) };
        if w * h > info.width * info.height {
            info.width = w;
            info.height = h;
            info.rotation = rotation;
        }
    }
    if info.width == 0 && info.duration_ms == 0 {
        None
    } else {
        Some(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bx(t: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn parses_rotated_track() {
        let mut mvhd = vec![0u8; 100];
        mvhd[4..8].copy_from_slice(&((1_600_000_000i64 + MAC_EPOCH_OFFSET) as u32).to_be_bytes());
        mvhd[12..16].copy_from_slice(&600u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&1800u32.to_be_bytes());
        let mut tkhd = vec![0u8; 84];
        let m = 40;
        tkhd[m..m + 4].copy_from_slice(&0i32.to_be_bytes());
        tkhd[m + 4..m + 8].copy_from_slice(&65536i32.to_be_bytes());
        tkhd[m + 36..m + 40].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[m + 40..m + 44].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let mut hdlr = vec![0u8; 24];
        hdlr[8..12].copy_from_slice(b"vide");
        let mdia = bx(b"mdia", &bx(b"hdlr", &hdlr));
        let mut trak_body = bx(b"tkhd", &tkhd);
        trak_body.extend(mdia);
        let mut moov = bx(b"mvhd", &mvhd);
        moov.extend(bx(b"trak", &trak_body));
        let info = probe_moov(&moov).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        assert_eq!(info.rotation, 90);
        assert_eq!(info.duration_ms, 3000);
        assert_eq!(info.creation_ts, Some(1_600_000_000));
    }
}
