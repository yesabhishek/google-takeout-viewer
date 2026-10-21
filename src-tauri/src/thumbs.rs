//! Thumbnail cache. Visible tiles are served first (LIFO, so the latest
//! scroll position wins); the whole library is pre-generated in the
//! background when the queue is idle and no scan is running. Files are keyed
//! by content fingerprint. Originals are only ever read, through `DRIVE`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::db::Db;
use crate::diskio::{self, Prio, DRIVE};

pub const THUMB_PX: u32 = 512;
const THUMB_Q: f64 = 0.8;
const MAX_URGENT: usize = 400;

#[derive(Clone)]
pub enum ThumbResult {
    Ok(Arc<Vec<u8>>),
    Busy,
    Failed,
}

pub type Reply = Box<dyn FnOnce(ThumbResult) + Send>;

#[derive(Default)]
struct Queue {
    urgent: Vec<i64>,
    bg: VecDeque<i64>,
    waiters: HashMap<i64, Vec<Reply>>,
    inflight: HashSet<i64>,
}

struct Inner {
    db: Arc<Db>,
    dir: PathBuf,
    q: Mutex<Queue>,
    cv: Condvar,
    paused: AtomicBool,
    scanning: AtomicBool,
    bg_total: AtomicUsize,
    bg_done: AtomicUsize,
    failed: Mutex<HashSet<i64>>,
    ffmpeg: Option<PathBuf>,
}

#[derive(Clone)]
pub struct ThumbService {
    inner: Arc<Inner>,
}

pub struct ItemRef {
    pub path: PathBuf,
    pub kind: i64,
    pub fp: i64,
    pub width: i64,
    pub height: i64,
}

pub fn lookup(db: &Db, id: i64) -> Option<ItemRef> {
    db.read(|c| {
        c.query_row(
            "SELECT path, kind, fp, width, height FROM items WHERE id=?1",
            [id],
            |r| {
                Ok(ItemRef {
                    path: PathBuf::from(r.get::<_, String>(0)?),
                    kind: r.get(1)?,
                    fp: r.get(2)?,
                    width: r.get(3)?,
                    height: r.get(4)?,
                })
            },
        )
    })
    .ok()
}

pub fn find_ffmpeg() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = vec![
        "/opt/homebrew/bin/ffmpeg".into(),
        "/usr/local/bin/ffmpeg".into(),
        "/usr/bin/ffmpeg".into(),
    ];
    if let Some(paths) = std::env::var_os("PATH") {
        for p in std::env::split_paths(&paths) {
            cands.push(p.join(if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" }));
        }
    }
    cands.into_iter().find(|p| p.is_file())
}

impl ThumbService {
    pub fn new(db: Arc<Db>, dir: PathBuf) -> ThumbService {
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, 8);
        let inner = Arc::new(Inner {
            db,
            dir,
            q: Mutex::new(Queue::default()),
            cv: Condvar::new(),
            paused: AtomicBool::new(false),
            scanning: AtomicBool::new(false),
            bg_total: AtomicUsize::new(0),
            bg_done: AtomicUsize::new(0),
            failed: Mutex::new(HashSet::new()),
            ffmpeg: find_ffmpeg(),
        });
        for i in 0..workers {
            let inner = inner.clone();
            std::thread::Builder::new()
                .name(format!("thumb-{i}"))
                .spawn(move || worker(inner))
                .expect("spawn thumbnail worker");
        }
        ThumbService { inner }
    }

    pub fn dir(&self) -> &Path {
        &self.inner.dir
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.inner.ffmpeg.is_some()
    }

    /// Serve a thumbnail: from disk if cached, otherwise after generation.
    pub fn request(&self, id: i64, reply: Reply) {
        let inner = &self.inner;
        let Some(item) = lookup(&inner.db, id) else {
            return reply(ThumbResult::Failed);
        };
        if let Ok(bytes) = std::fs::read(cache_path(&inner.dir, item.fp)) {
            return reply(ThumbResult::Ok(Arc::new(bytes)));
        }
        if inner.failed.lock().unwrap().contains(&id) {
            return reply(ThumbResult::Failed);
        }
        let mut dropped: Vec<Reply> = Vec::new();
        {
            let mut q = inner.q.lock().unwrap();
            if let Some(w) = q.waiters.get_mut(&id) {
                w.push(reply);
                return;
            }
            q.waiters.insert(id, vec![reply]);
            q.urgent.push(id);
            while q.urgent.len() > MAX_URGENT {
                let old = q.urgent.remove(0);
                if let Some(ws) = q.waiters.remove(&old) {
                    dropped.extend(ws);
                }
            }
        }
        inner.cv.notify_one();
        for r in dropped {
            r(ThumbResult::Busy);
        }
    }

    /// Queue every item without a cached thumbnail for background generation.
    pub fn enqueue_background(&self, items: Vec<(i64, i64)>) {
        let missing: Vec<i64> = items
            .into_iter()
            .filter(|(_, fp)| !cache_path(&self.inner.dir, *fp).exists())
            .map(|(id, _)| id)
            .collect();
        let mut q = self.inner.q.lock().unwrap();
        q.bg = missing.into_iter().collect();
        self.inner.bg_total.store(q.bg.len(), Ordering::Relaxed);
        self.inner.bg_done.store(0, Ordering::Relaxed);
        drop(q);
        self.inner.cv.notify_all();
    }

    pub fn set_paused(&self, paused: bool) {
        self.inner.paused.store(paused, Ordering::Relaxed);
        self.inner.cv.notify_all();
    }

    /// Hold background generation while a scan reads the drive.
    pub fn set_scanning(&self, scanning: bool) {
        self.inner.scanning.store(scanning, Ordering::Relaxed);
        self.inner.cv.notify_all();
    }

    pub fn progress(&self) -> (usize, usize, bool) {
        let total = self.inner.bg_total.load(Ordering::Relaxed);
        let done = self.inner.bg_done.load(Ordering::Relaxed).min(total);
        (done, total, self.inner.paused.load(Ordering::Relaxed))
    }

    /// Delete cached thumbnails. Only files shaped like our cache
    /// (`ab/abcdef….jpg` and their temporaries) are removed, so a wrongly
    /// configured data folder can't lose anything else.
    pub fn clear_cache(&self) {
        let Ok(buckets) = std::fs::read_dir(&self.inner.dir) else { return };
        for b in buckets.flatten() {
            let name = b.file_name().to_string_lossy().to_string();
            let bucket = name.len() == 2 && name.bytes().all(|c| c.is_ascii_hexdigit());
            if !bucket || !b.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Ok(files) = std::fs::read_dir(b.path()) else { continue };
            for f in files.flatten() {
                let n = f.file_name().to_string_lossy().to_string();
                let ours = n.starts_with(&name) && (n.ends_with(".jpg") || n.contains(".tmp"));
                if ours && f.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
            let _ = std::fs::remove_dir(b.path()); // only succeeds when empty
        }
    }

    pub fn clear_failures(&self) {
        self.inner.failed.lock().unwrap().clear();
    }
}

pub fn cache_path(dir: &Path, fp: i64) -> PathBuf {
    let hex = format!("{:016x}", fp as u64);
    dir.join(&hex[..2]).join(format!("{hex}.jpg"))
}

fn worker(inner: Arc<Inner>) {
    loop {
        let (id, background) = {
            let mut q = inner.q.lock().unwrap();
            loop {
                if let Some(id) = q.urgent.pop() {
                    if q.inflight.contains(&id) {
                        continue; // the running job will answer its waiters
                    }
                    break (id, false);
                }
                if !inner.paused.load(Ordering::Relaxed) && !inner.scanning.load(Ordering::Relaxed) {
                    if let Some(id) = q.bg.pop_front() {
                        if q.inflight.contains(&id) || q.waiters.contains_key(&id) {
                            inner.bg_done.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        break (id, true);
                    }
                }
                q = inner.cv.wait(q).unwrap();
            }
        };
        inner.q.lock().unwrap().inflight.insert(id);
        let result = generate(&inner, id, if background { Prio::Background } else { Prio::Interactive });
        let waiters = {
            let mut q = inner.q.lock().unwrap();
            q.inflight.remove(&id);
            q.waiters.remove(&id).unwrap_or_default()
        };
        if background {
            inner.bg_done.fetch_add(1, Ordering::Relaxed);
        }
        for w in waiters {
            w(result.clone());
        }
    }
}

fn generate(inner: &Inner, id: i64, prio: Prio) -> ThumbResult {
    let Some(item) = lookup(&inner.db, id) else {
        return ThumbResult::Failed;
    };
    let out = cache_path(&inner.dir, item.fp);
    if let Ok(bytes) = std::fs::read(&out) {
        return ThumbResult::Ok(Arc::new(bytes));
    }
    // Keep the short edge sharp for panoramas.
    let (w, h) = (item.width.max(1) as f64, item.height.max(1) as f64);
    let ratio = w.max(h) / w.min(h);
    let max_px = ((360.0 * ratio).ceil() as u32).clamp(THUMB_PX, 2048);

    let bytes = {
        let _drive = DRIVE.acquire(prio);
        if item.kind == 0 {
            image_thumb(&item.path, max_px)
        } else {
            video_thumb(&item.path, max_px, inner.ffmpeg.as_deref())
        }
    };
    match bytes {
        Some(b) if !b.is_empty() => {
            if let Some(parent) = out.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let tmp = out.with_extension(format!("tmp{id}"));
            if std::fs::write(&tmp, &b).is_ok() {
                let _ = std::fs::rename(&tmp, &out);
            }
            ThumbResult::Ok(Arc::new(b))
        }
        _ => {
            // A disconnected drive is not a broken file: allow a retry later.
            if item.path.exists() {
                inner.failed.lock().unwrap().insert(id);
            }
            ThumbResult::Failed
        }
    }
}

pub fn image_thumb(path: &Path, max_px: u32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        crate::mac::image_thumbnail(path, max_px, THUMB_Q)
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        if let Some(b) = crate::win::image_thumbnail(path, max_px, (THUMB_Q * 100.0) as u8) {
            return Some(b);
        }
        portable::image_thumbnail(path, max_px, (THUMB_Q * 100.0) as u8)
    }
}

fn video_thumb(path: &Path, max_px: u32, ffmpeg: Option<&Path>) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    if let Some(b) = crate::mac::video_thumbnail(path, max_px, THUMB_Q) {
        return Some(b);
    }
    #[cfg(target_os = "windows")]
    if let Some(b) = crate::win::video_thumbnail(path, max_px, (THUMB_Q * 100.0) as u8) {
        return Some(b);
    }
    let ffmpeg = ffmpeg?;
    for ss in ["0.5", "0"] {
        let scale = format!(
            "scale=w='if(gt(iw,ih),{max_px},-2)':h='if(gt(iw,ih),-2,{max_px})'"
        );
        // ffmpeg only reads the original and writes the frame to a pipe.
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", "error", "-nostdin", "-ss", ss, "-i"])
            .arg(path)
            .args(["-frames:v", "1", "-vf", &scale, "-f", "image2pipe", "-vcodec", "mjpeg", "-q:v", "4", "-"]);
        match diskio::run_captured(cmd, Duration::from_secs(60)) {
            Some(out) if !out.is_empty() => return Some(out),
            _ => {}
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
pub mod portable {
    use image::{DynamicImage, ImageDecoder, ImageReader};
    use std::path::Path;

    /// Full-size decode with orientation applied. On Windows WIC is tried
    /// first, so HEIC/AVIF/RAW export works when the codec extensions are
    /// installed.
    pub fn decode(path: &Path) -> Option<DynamicImage> {
        #[cfg(target_os = "windows")]
        if let Some(img) = crate::win::decode(path, None) {
            return Some(img);
        }
        decode_portable(path)
    }

    fn decode_portable(path: &Path) -> Option<DynamicImage> {
        let reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = decoder.orientation().ok();
        let mut img = DynamicImage::from_decoder(decoder).ok()?;
        if let Some(o) = orientation {
            img.apply_orientation(o);
        }
        Some(img)
    }

    pub fn image_thumbnail(path: &Path, max_px: u32, quality: u8) -> Option<Vec<u8>> {
        // Windows already tried WIC (see thumbs::image_thumb); don't decode twice.
        encode_jpeg(&decode_portable(path)?.thumbnail(max_px, max_px), quality)
    }

    pub fn encode_jpeg(img: &DynamicImage, quality: u8) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality);
        DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(enc).ok()?;
        Some(out)
    }
}
