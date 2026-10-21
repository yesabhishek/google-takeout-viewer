//! SQLite storage. One writer connection plus a small pool of readers so the
//! media protocol never waits behind a long indexing transaction (WAL mode).

use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub type DbResult<T> = Result<T, String>;

pub fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn is_corrupt(e: &str) -> bool {
    e.contains("not a database") || e.contains("malformed") || e.contains("corrupt")
}

pub struct Db {
    writer: Mutex<Connection>,
    readers: Vec<Mutex<Connection>>,
    next: AtomicUsize,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sources (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  added_at INTEGER NOT NULL DEFAULT 0,
  last_scan INTEGER
);

CREATE TABLE IF NOT EXISTS items (
  id INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL,
  path TEXT NOT NULL UNIQUE,
  dir_key TEXT NOT NULL,
  name TEXT NOT NULL,
  kind INTEGER NOT NULL,            -- 0 image, 1 video
  size INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  fp INTEGER NOT NULL,              -- cheap content fingerprint (dedupe + thumb cache key)
  width INTEGER NOT NULL DEFAULT 0, -- display width (orientation applied)
  height INTEGER NOT NULL DEFAULT 0,
  duration_ms INTEGER,
  meta_ts INTEGER,                  -- capture time from EXIF / video header
  sc_ts INTEGER,                    -- photoTakenTime from Takeout sidecar
  sc_ctime INTEGER,                 -- creationTime from Takeout sidecar
  taken_ts INTEGER NOT NULL,        -- effective time (computed in relink)
  camera TEXT,
  lat REAL,
  lon REAL,
  description TEXT,
  people TEXT,                      -- JSON array of names from sidecar
  sc_fav INTEGER NOT NULL DEFAULT 0,
  sc_archived INTEGER NOT NULL DEFAULT 0,
  sc_trashed INTEGER NOT NULL DEFAULT 0,
  user_fav INTEGER,                 -- NULL = follow sidecar
  motion INTEGER NOT NULL DEFAULT 0,-- 1 = image with embedded motion video
  motion_offset INTEGER,            -- byte offset of embedded video, once located
  sidecar TEXT,
  -- computed by relink:
  hidden INTEGER NOT NULL DEFAULT 0,-- 0 visible, 1 duplicate, 2 live companion, 3 superseded by edit
  canonical_id INTEGER,
  live_video_id INTEGER,
  live_parent_id INTEGER,
  edited_id INTEGER,                -- for originals: the edited version shown instead
  original_id INTEGER,              -- for edited versions: the original
  fav INTEGER NOT NULL DEFAULT 0,
  archived INTEGER NOT NULL DEFAULT 0,
  trashed INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_items_vis ON items(hidden, taken_ts DESC);
CREATE INDEX IF NOT EXISTS idx_items_fp ON items(fp);
CREATE INDEX IF NOT EXISTS idx_items_dir ON items(dir_key);

CREATE TABLE IF NOT EXISTS people (
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL UNIQUE COLLATE NOCASE
);
CREATE TABLE IF NOT EXISTS item_people (
  item_id INTEGER NOT NULL,
  person_id INTEGER NOT NULL,
  PRIMARY KEY (person_id, item_id)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS idx_item_people_item ON item_people(item_id);

CREATE TABLE IF NOT EXISTS albums (
  id INTEGER PRIMARY KEY,
  dir_key TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  description TEXT,
  ts INTEGER
);
CREATE TABLE IF NOT EXISTS album_items (
  album_id INTEGER NOT NULL,
  item_id INTEGER NOT NULL,
  PRIMARY KEY (album_id, item_id)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

fn tune(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;
         PRAGMA temp_store=MEMORY;
         PRAGMA cache_size=-32000;
         PRAGMA mmap_size=268435456;",
    )?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(())
}

impl Db {
    /// Open the index. A damaged index file (power loss, full disk) is moved
    /// aside and rebuilt by the next scan; it only ever holds data derived
    /// from your Takeout, never the photos themselves.
    pub fn open(path: &Path) -> DbResult<Db> {
        match Db::open_once(path) {
            Err(e) if is_corrupt(&e) && path.exists() => {
                let aside = path.with_extension(format!("sqlite3.damaged-{}", crate::scan::now()));
                std::fs::rename(path, &aside).map_err(err)?;
                for suffix in ["-wal", "-shm"] {
                    let from = format!("{}{suffix}", path.display());
                    let _ = std::fs::rename(&from, format!("{}{suffix}", aside.display()));
                }
                eprintln!("index was damaged; moved to {} and starting fresh", aside.display());
                Db::open_once(path)
            }
            r => r,
        }
    }

    fn open_once(path: &Path) -> DbResult<Db> {
        let writer = Connection::open(path).map_err(err)?;
        tune(&writer).map_err(err)?;
        writer.execute_batch(SCHEMA).map_err(err)?;
        let mut readers = Vec::new();
        for _ in 0..4 {
            let r = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(err)?;
            tune(&r).map_err(err)?;
            readers.push(Mutex::new(r));
        }
        Ok(Db {
            writer: Mutex::new(writer),
            readers,
            next: AtomicUsize::new(0),
        })
    }

    pub fn write<T>(&self, f: impl FnOnce(&mut Connection) -> rusqlite::Result<T>) -> DbResult<T> {
        let mut c = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut c).map_err(err)
    }

    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> DbResult<T> {
        let n = self.readers.len();
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        for i in 0..n {
            if let Ok(c) = self.readers[(start + i) % n].try_lock() {
                return f(&c).map_err(err);
            }
        }
        let c = self.readers[start % n].lock().unwrap_or_else(|e| e.into_inner());
        f(&c).map_err(err)
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.read(|c| {
            c.query_row("SELECT value FROM settings WHERE key=?1", [key], |r| r.get(0))
        })
        .ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> DbResult<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [key, value],
            )
            .map(|_| ())
        })
    }
}
