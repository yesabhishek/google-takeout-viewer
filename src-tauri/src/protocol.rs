//! `gp://` media protocol (also served over HTTP by the dev bridge):
//!   /thumb/{id}    cached JPEG thumbnail
//!   /media/{id}    original file (byte ranges for video)
//!   /motion/{id}   Live Photo / motion photo clip
//!   /preview/{id}  large JPEG for formats the web view cannot show (RAW …)

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;

use crate::state::AppState;
use crate::takeout;
use crate::thumbs::ThumbResult;

pub enum Body {
    Bytes(Arc<Vec<u8>>),
    File { path: PathBuf, start: u64, len: u64 },
}

pub struct Resp {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

impl Resp {
    fn bytes(status: u16, mime: &str, b: Arc<Vec<u8>>) -> Resp {
        Resp {
            status,
            headers: vec![("Content-Type", mime.to_string()), ("Content-Length", b.len().to_string())],
            body: Body::Bytes(b),
        }
    }
    fn text(status: u16, msg: &str) -> Resp {
        Resp::bytes(status, "text/plain", Arc::new(msg.as_bytes().to_vec()))
    }

    /// Materialize the body (Tauri needs the bytes up front).
    pub fn into_vec(self) -> (u16, Vec<(&'static str, String)>, Vec<u8>) {
        let body = match self.body {
            Body::Bytes(b) => Arc::try_unwrap(b).unwrap_or_else(|a| (*a).clone()),
            Body::File { path, start, len } => {
                let mut buf = vec![0u8; len as usize];
                let ok = File::open(&path)
                    .and_then(|mut f| {
                        f.seek(SeekFrom::Start(start))?;
                        f.read_exact(&mut buf)
                    })
                    .is_ok();
                if !ok {
                    return (500, vec![("Content-Type", "text/plain".into())], b"read error".to_vec());
                }
                buf
            }
        };
        (self.status, self.headers, body)
    }
}

pub type Reply = Box<dyn FnOnce(Resp) + Send>;

/// Per-request chunk for ranged media. WebKit asks for the rest as it plays.
const CHUNK: u64 = 4 * 1024 * 1024;

pub fn mime_for(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" | "jpe" | "jfif" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" | "hif" => "image/heic",
        "heif" => "image/heif",
        "avif" => "image/avif",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "mp4" | "m4v" | "mp" => "video/mp4",
        "mov" | "qt" => "video/quicktime",
        "3gp" => "video/3gpp",
        "3g2" => "video/3gpp2",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wmv" => "video/x-ms-wmv",
        "mpg" | "mpeg" => "video/mpeg",
        "mts" | "m2ts" | "ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

/// Formats the platform web view renders natively.
pub fn web_displayable(ext: &str) -> bool {
    if cfg!(target_os = "macos") {
        matches!(
            ext,
            "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "gif" | "webp" | "heic" | "heif" | "hif" | "avif"
                | "tif" | "tiff" | "bmp" | "ico"
        )
    } else {
        matches!(ext, "jpg" | "jpeg" | "jpe" | "jfif" | "png" | "gif" | "webp" | "avif" | "bmp" | "ico")
    }
}

pub fn web_playable(ext: &str) -> bool {
    if cfg!(target_os = "macos") {
        matches!(ext, "mp4" | "m4v" | "mov" | "qt" | "3gp" | "3g2" | "mp")
    } else {
        matches!(ext, "mp4" | "m4v" | "webm" | "mp")
    }
}

fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?;
    let first = spec.split(',').next()?.trim();
    let (a, b) = first.split_once('-')?;
    if len == 0 {
        return None;
    }
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        let n = n.min(len);
        return Some((len - n, len - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end: u64 = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
    if start > end {
        return None;
    }
    Some((start, end))
}

/// Serve `[base, file_len)` of a file, honoring Range.
fn file_resp(path: PathBuf, base: u64, mime: &str, range: Option<&str>, whole: bool) -> Resp {
    let file_len = match std::fs::metadata(&path) {
        Ok(m) => m.len(),
        Err(_) => return Resp::text(404, "offline"),
    };
    let len = file_len.saturating_sub(base);
    let mut headers = vec![
        ("Content-Type", mime.to_string()),
        ("Accept-Ranges", "bytes".to_string()),
    ];
    let (status, start, end) = match range.map(|r| parse_range(r, len)) {
        Some(None) if !whole => {
            headers.push(("Content-Range", format!("bytes */{len}")));
            return Resp { status: 416, headers, body: Body::Bytes(Arc::new(Vec::new())) };
        }
        Some(Some((s, e))) if !whole => (206, s, e.min(s + CHUNK - 1)),
        _ if !whole && len > 32 * 1024 * 1024 => (206, 0, CHUNK - 1),
        _ => (200, 0, len.saturating_sub(1)),
    };
    let n = if len == 0 { 0 } else { end - start + 1 };
    if status == 206 {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{len}")));
    }
    headers.push(("Content-Length", n.to_string()));
    Resp { status, headers, body: Body::File { path, start: base + start, len: n } }
}

struct MediaRow {
    path: PathBuf,
    name: String,
    kind: i64,
    live_video_id: Option<i64>,
    motion: bool,
    motion_offset: Option<i64>,
}

fn media_row(state: &AppState, id: i64) -> Option<MediaRow> {
    state
        .db
        .read(|c| {
            c.query_row(
                "SELECT path, name, kind, live_video_id, motion, motion_offset FROM items WHERE id=?1",
                [id],
                |r| {
                    Ok(MediaRow {
                        path: PathBuf::from(r.get::<_, String>(0)?),
                        name: r.get(1)?,
                        kind: r.get(2)?,
                        live_video_id: r.get(3)?,
                        motion: r.get(4)?,
                        motion_offset: r.get(5)?,
                    })
                },
            )
        })
        .ok()
}

/// Locate (and remember) the embedded clip of a motion photo.
pub fn motion_offset(state: &AppState, id: i64, row_path: &std::path::Path, known: Option<i64>) -> Option<u64> {
    if let Some(o) = known {
        return if o > 0 { Some(o as u64) } else { None };
    }
    let found = crate::meta::find_motion_offset(row_path);
    let store = found.map(|o| o as i64).unwrap_or(-1);
    let _ = state
        .db
        .write(|c| c.execute("UPDATE items SET motion_offset=?1 WHERE id=?2", [store, id]));
    found
}

pub fn handle(state: Arc<AppState>, path: &str, range: Option<String>, reply: Reply) {
    let path = path.split('?').next().unwrap_or("");
    let mut parts = path.trim_start_matches('/').split('/');
    let route = parts.next().unwrap_or("");
    let id: i64 = match parts.next().and_then(|s| s.parse().ok()) {
        Some(id) => id,
        None => return reply(Resp::text(400, "bad request")),
    };
    match route {
        "thumb" => state.thumbs.request(
            id,
            Box::new(move |r| {
                reply(match r {
                    ThumbResult::Ok(b) => {
                        let mut resp = Resp::bytes(200, "image/jpeg", b);
                        resp.headers.push(("Cache-Control", "max-age=3600".into()));
                        resp
                    }
                    ThumbResult::Busy => Resp::text(503, "busy"),
                    ThumbResult::Failed => Resp::text(404, "no thumbnail"),
                })
            }),
        ),
        "media" => {
            let Some(row) = media_row(&state, id) else {
                return reply(Resp::text(404, "not found"));
            };
            let ext = takeout::ext_of(&row.name);
            reply(file_resp(row.path, 0, mime_for(&ext), range.as_deref(), row.kind == 0))
        }
        "motion" => {
            let Some(row) = media_row(&state, id) else {
                return reply(Resp::text(404, "not found"));
            };
            if let Some(vid) = row.live_video_id {
                if let Some(v) = media_row(&state, vid) {
                    let ext = takeout::ext_of(&v.name);
                    return reply(file_resp(v.path, 0, mime_for(&ext), range.as_deref(), false));
                }
            }
            if row.motion {
                if let Some(off) = motion_offset(&state, id, &row.path, row.motion_offset) {
                    return reply(file_resp(row.path, off, "video/mp4", range.as_deref(), false));
                }
            }
            reply(Resp::text(404, "no motion"))
        }
        "preview" => {
            let Some(row) = media_row(&state, id) else {
                return reply(Resp::text(404, "not found"));
            };
            match crate::thumbs::image_thumb(&row.path, 3072) {
                Some(b) => reply(Resp::bytes(200, "image/jpeg", Arc::new(b))),
                None => reply(Resp::text(415, "cannot decode")),
            }
        }
        _ => reply(Resp::text(404, "unknown route")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-1", 100), Some((0, 1)));
        assert_eq!(parse_range("bytes=10-", 100), Some((10, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=50-500", 100), Some((50, 99)));
        assert_eq!(parse_range("bytes=200-", 100), None);
    }
}
