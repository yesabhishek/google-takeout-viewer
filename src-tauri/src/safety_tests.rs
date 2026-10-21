//! End-to-end check of the app's core promise: indexing, thumbnails, export,
//! "open in another app", clearing the cache and removing a folder never
//! change a byte, a timestamp or the file list of the Takeout folder.

use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use crate::api;
use crate::export::{self, ExportOptions};
use crate::state::AppState;

/// A valid 1×1 GIF, small enough to inline and decodable everywhere.
const GIF: &[u8] = &[
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00,
    0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02,
    0x02, 0x44, 0x01, 0x00, 0x3B,
];

fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let md = e.metadata().unwrap();
            (e.path().to_path_buf(), (std::fs::read(e.path()).unwrap(), md.modified().unwrap()))
        })
        .collect()
}

fn wait(what: &str, mut done: impl FnMut() -> bool) {
    let t = Instant::now();
    while !done() {
        assert!(t.elapsed() < Duration::from_secs(60), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn export_to(state: &Arc<AppState>, ids: &[i64], dest: &Path, format: &str) -> Result<(), String> {
    let opts: ExportOptions = serde_json::from_value(json!({ "format": format, "includeMotion": true })).unwrap();
    export::start(state, ids.to_vec(), dest.to_path_buf(), opts)?;
    wait("export", || !state.export.lock().unwrap().running);
    Ok(())
}

#[test]
fn originals_are_never_modified() {
    let base = std::env::temp_dir().join(format!("tv-safety-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let takeout = base.join("Takeout");
    let photos = takeout.join("Google Photos");
    let y2020 = photos.join("Photos from 2020");
    let trip = photos.join("Trip");
    for d in [&y2020, &trip, &photos.join("Photos from 2021")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(y2020.join("IMG_1.gif"), GIF).unwrap();
    std::fs::write(
        y2020.join("IMG_1.gif.json"),
        r#"{"title":"IMG_1.gif","photoTakenTime":{"timestamp":"1600000000"},"favorited":true}"#,
    )
    .unwrap();
    std::fs::write(trip.join("IMG_1.gif"), GIF).unwrap();
    std::fs::write(trip.join("metadata.json"), r#"{"title":"Trip"}"#).unwrap();
    let mut other = GIF.to_vec();
    other.extend_from_slice(b"different");
    std::fs::write(photos.join("Photos from 2021/IMG_2.gif"), &other).unwrap();
    let before = snapshot(&takeout);

    let state = AppState::open(&base.join("data")).unwrap();
    api::add_source(&state, photos.to_str().unwrap()).unwrap();
    wait("scan", || !state.scan.lock().unwrap().running);
    assert_eq!(state.scan.lock().unwrap().error, None);
    let ids: Vec<i64> = state
        .db
        .read(|c| {
            let mut st = c.prepare("SELECT id FROM items WHERE hidden=0")?;
            let rows = st.query_map([], |r| r.get(0))?;
            rows.collect()
        })
        .unwrap();
    assert_eq!(ids.len(), 2, "the album copy is a duplicate of the year-folder photo");

    state.queue_all_thumbs();
    wait("thumbnails", || {
        let (done, total, _) = state.thumbs.progress();
        done >= total
    });

    let out = base.join("Exports");
    export_to(&state, &ids, &out, "original").unwrap();
    export_to(&state, &ids, &out, "original").unwrap();
    export_to(&state, &ids, &out, "png").unwrap();
    let exported: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert!(exported.contains(&"IMG_1 (2).gif".to_string()), "second export got new names: {exported:?}");
    assert!(!exported.iter().any(|n| n.ends_with(".partial")), "no leftovers: {exported:?}");
    assert_eq!(state.export.lock().unwrap().failed, 0);

    let inside = y2020.join("exported");
    assert!(export_to(&state, &ids, &inside, "original").is_err(), "exporting into the library is refused");
    assert!(!inside.exists());

    let copy = api::open_copy(&state, ids[0]).unwrap();
    assert!(copy.starts_with(state.open_copies_dir()));
    assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());

    api::dispatch(&state, "clear_cache", json!({})).map(|_| ()).unwrap();
    let sid: i64 = state.db.read(|c| c.query_row("SELECT id FROM sources", [], |r| r.get(0))).unwrap();
    api::dispatch(&state, "remove_source", json!({ "id": sid })).map(|_| ()).unwrap();

    assert_eq!(before, snapshot(&takeout), "the Takeout folder must be untouched");
    state.clean_open_copies();
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn empty_looking_folder_keeps_its_index() {
    let base = std::env::temp_dir().join(format!("tv-mount-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let lib = base.join("Photos from 2019");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("a.gif"), GIF).unwrap();

    let state = AppState::open(&base.join("data")).unwrap();
    api::add_source(&state, base.to_str().unwrap()).unwrap_err(); // contains our data folder
    api::add_source(&state, lib.to_str().unwrap()).unwrap();
    wait("scan", || !state.scan.lock().unwrap().running);
    let count = |s: &AppState| -> i64 { s.db.read(|c| c.query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))).unwrap() };
    assert_eq!(count(&state), 1);

    // Simulate a drive that mounts with nothing readable on it.
    std::fs::rename(lib.join("a.gif"), base.join("a.gif")).unwrap();
    assert!(crate::scan::start(&state));
    wait("rescan", || !state.scan.lock().unwrap().running);
    assert_eq!(count(&state), 1, "entries are kept when a folder suddenly looks empty");
    let _ = std::fs::remove_dir_all(&base);
}
