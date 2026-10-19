use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::db::Db;
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
        Ok(Arc::new(AppState {
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
        }))
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
