use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::db::Db;
use crate::diskio::{self, Mode, DRIVE};
use crate::thumbs::ThumbService;

type Emitter = Box<dyn Fn(&str, Value) + Send + Sync>;

#[derive(Serialize, Clone, Default)]
pub struct ScanStatus {
    pub running: bool,
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub found: usize,
    pub message: String,
    pub error: Option<String>,
    /// Files that could not be read this scan (retried next time).
    pub skipped: usize,
    /// Something the user should know about, even though the scan finished.
    pub warning: Option<String>,
    pub finished_at: Option<i64>,
}

#[derive(Serialize, Clone, Default)]
pub struct ExportStatus {
    pub running: bool,
    pub done: usize,
    pub total: usize,
    pub failed: usize,
    pub current: String,
    pub dest: String,
    pub errors: Vec<String>,
}

pub struct AppState {
    pub db: Arc<Db>,
    pub data_dir: PathBuf,
    pub thumbs: ThumbService,
    pub scan: Mutex<ScanStatus>,
    pub scan_cancel: AtomicBool,
    pub export: Mutex<ExportStatus>,
    pub export_cancel: AtomicBool,
    pub generation: AtomicU64,
    pub timeline_cache: Mutex<HashMap<String, (u64, Arc<Vec<u8>>)>>,
    emitter: RwLock<Option<Emitter>>,
    drive_reason: Mutex<String>,
}

impl AppState {
    pub fn open(data_dir: &Path) -> Result<Arc<AppState>, String> {
        std::fs::create_dir_all(data_dir).map_err(|e| format!("{}: {e}", data_dir.display()))?;
        let db = Arc::new(Db::open(&data_dir.join("library.sqlite3"))?);
        let thumb_dir = data_dir.join("thumbs-v1");
        std::fs::create_dir_all(&thumb_dir).map_err(|e| e.to_string())?;
        let thumbs = ThumbService::new(db.clone(), thumb_dir);
        let paused = db.setting("thumbs_paused").as_deref() == Some("1");
        thumbs.set_paused(paused);
        let state = Arc::new(AppState {
            db,
            data_dir: data_dir.to_path_buf(),
            thumbs,
            scan: Mutex::new(ScanStatus::default()),
            scan_cancel: AtomicBool::new(false),
            export: Mutex::new(ExportStatus::default()),
            export_cancel: AtomicBool::new(false),
            generation: AtomicU64::new(1),
            timeline_cache: Mutex::new(HashMap::new()),
            emitter: RwLock::new(None),
            drive_reason: Mutex::new(String::new()),
        });
        state.apply_drive_mode();
        Ok(state)
    }

    /// Set how hard the drive is read: the saved choice, or for "auto" the
    /// gentlest mode that suits where the library folders are.
    pub fn apply_drive_mode(&self) {
        let choice = self.db.setting("drive_mode").unwrap_or_else(|| "auto".into());
        let (mode, reason) = match Mode::parse(&choice) {
            Some(m) => (m, "You chose this."),
            None => {
                let sources: Vec<String> = self
                    .db
                    .read(|c| {
                        let mut st = c.prepare("SELECT path FROM sources")?;
                        let rows = st.query_map([], |r| r.get(0))?;
                        rows.collect()
                    })
                    .unwrap_or_default();
                let sources: Vec<PathBuf> = sources.into_iter().map(PathBuf::from).collect();
                diskio::auto_mode(&sources)
            }
        };
        DRIVE.set_mode(mode);
        *self.drive_reason.lock().unwrap() = reason.to_string();
    }

    pub fn drive_info(&self) -> Value {
        serde_json::json!({
            "choice": self.db.setting("drive_mode").unwrap_or_else(|| "auto".into()),
            "mode": DRIVE.mode().as_str(),
            "reason": self.drive_reason.lock().unwrap().clone(),
            "struggling": DRIVE.struggling(),
        })
    }

    /// Read-only copies handed to other apps ("Open in Preview"), so an
    /// editor can never save over an original.
    pub fn open_copies_dir(&self) -> PathBuf {
        self.data_dir.join("open-copies")
    }

    /// Remove last session's read-only copies. Only files inside our own
    /// `open-copies` folder are touched.
    pub fn clean_open_copies(&self) {
        let Ok(entries) = std::fs::read_dir(self.open_copies_dir()) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            self.clean_open_copy(&p);
        }
    }

    /// Delete one of our read-only copies (Windows won't delete a read-only file).
    pub fn clean_open_copy(&self, p: &Path) {
        if !p.starts_with(self.open_copies_dir()) {
            return;
        }
        #[cfg(windows)]
        if let Ok(m) = std::fs::metadata(p) {
            let mut perm = m.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perm.set_readonly(false);
            let _ = std::fs::set_permissions(p, perm);
        }
        let _ = std::fs::remove_file(p);
    }

    pub fn set_emitter(&self, f: Emitter) {
        *self.emitter.write().unwrap() = Some(f);
    }

    pub fn emit(&self, name: &str, payload: Value) {
        if let Some(f) = self.emitter.read().unwrap().as_ref() {
            f(name, payload);
        }
    }

    /// The library contents changed: drop caches and tell the UI.
    pub fn changed(&self) {
        let g = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.timeline_cache.lock().unwrap().clear();
        self.emit("library-changed", serde_json::json!({ "generation": g }));
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    pub fn update_scan(&self, f: impl FnOnce(&mut ScanStatus)) {
        let snapshot = {
            let mut s = self.scan.lock().unwrap();
            f(&mut s);
            s.clone()
        };
        self.emit("scan-progress", serde_json::to_value(snapshot).unwrap_or(Value::Null));
    }

    pub fn update_export(&self, f: impl FnOnce(&mut ExportStatus)) {
        let snapshot = {
            let mut s = self.export.lock().unwrap();
            f(&mut s);
            s.clone()
        };
        self.emit("export-progress", serde_json::to_value(snapshot).unwrap_or(Value::Null));
    }

    /// Queue thumbnails for everything visible, newest first.
    pub fn queue_all_thumbs(&self) {
        let items: Vec<(i64, i64)> = self
            .db
            .read(|c| {
                let mut st = c.prepare("SELECT id, fp FROM items WHERE hidden=0 ORDER BY taken_ts DESC")?;
                let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect()
            })
            .unwrap_or_default();
        self.thumbs.enqueue_background(items);
    }
}
