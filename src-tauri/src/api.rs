//! Command surface shared by the Tauri IPC bridge and the HTTP dev bridge.

use chrono::{Local, TimeZone};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::diskio::{self, Mode, Prio, DRIVE};
use crate::export::{self, ExportOptions};
use crate::protocol;
use crate::scan;
use crate::state::AppState;
use crate::takeout;

pub enum Out {
    Json(Value),
    Bytes(Vec<u8>),
}

fn arg<T: for<'de> Deserialize<'de>>(args: &Value, key: &str) -> Result<T, String> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| format!("argument `{key}`: {e}"))
}

pub fn dispatch(state: &Arc<AppState>, name: &str, args: Value) -> Result<Out, String> {
    let j = |v: Value| Ok(Out::Json(v));
    match name {
        "status" => j(status(state)?),
        "add_source" => {
            let path: String = arg(&args, "path")?;
            add_source(state, &path)?;
            j(json!(true))
        }
        "remove_source" => {
            let id: i64 = arg(&args, "id")?;
            remove_source(state, id)?;
            j(json!(true))
        }
        "rescan" => j(json!(scan::start(state))),
        "cancel_scan" => {
            state.scan_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            j(json!(true))
        }
        "timeline" => {
            let f: Filter = serde_json::from_value(args.get("filter").cloned().unwrap_or(json!({})))
                .map_err(|e| e.to_string())?;
            Ok(Out::Bytes(timeline(state, &f)?))
        }
        "item" => j(item(state, arg(&args, "id")?)?),
        "people" => j(people(state)?),
        "albums" => j(albums(state)?),
        "set_favorite" => {
            let ids: Vec<i64> = arg(&args, "ids")?;
            let value: bool = arg(&args, "value")?;
            set_favorite(state, &ids, value)?;
            j(json!(true))
        }
        "export" => {
            let ids: Vec<i64> = arg(&args, "ids")?;
            let dest: String = arg(&args, "dest")?;
            let opts: ExportOptions = arg(&args, "options")?;
            export::start(state, ids, PathBuf::from(dest), opts)?;
            j(json!(true))
        }
        "cancel_export" => {
            state.export_cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            j(json!(true))
        }
        "reveal" => {
            let p = item_path(state, arg(&args, "id")?)?;
            tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| e.to_string())?;
            j(json!(true))
        }
        "open_external" => {
            let p = open_copy(state, arg(&args, "id")?)?;
            tauri_plugin_opener::open_path(&p, None::<&str>).map_err(|e| e.to_string())?;
            j(json!(true))
        }
        "reveal_path" => {
            let p = PathBuf::from(arg::<String>(&args, "path")?);
            if !revealable(state, &p)? {
                return Err("not a library or export folder".into());
            }
            tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| e.to_string())?;
            j(json!(true))
        }
        "open_link" => {
            let url: String = arg(&args, "url")?;
            if !LINKS.iter().any(|l| url == *l || url.starts_with(&format!("{l}/"))) {
                return Err("link not allowed".into());
            }
            tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| e.to_string())?;
            j(json!(true))
        }
        "set_drive_mode" => {
            let mode: String = arg(&args, "mode")?;
            if mode != "auto" && Mode::parse(&mode).is_none() {
                return Err(format!("unknown drive mode {mode}"));
            }
            state.db.set_setting("drive_mode", &mode)?;
            state.apply_drive_mode();
            j(state.drive_info())
        }
        "thumbs_pause" => {
            let paused: bool = arg(&args, "paused")?;
            state.thumbs.set_paused(paused);
            state.db.set_setting("thumbs_paused", if paused { "1" } else { "0" })?;
            j(json!(true))
        }
        "log" => {
            eprintln!("[ui] {}", args.get("message").and_then(|m| m.as_str()).unwrap_or(""));
            j(json!(true))
        }
        "cache_info" => j(cache_info(state)),
        "clear_cache" => {
            state.thumbs.clear_cache();
            state.thumbs.clear_failures();
            state.queue_all_thumbs();
            j(json!(true))
        }
        _ => Err(format!("unknown command {name}")),
    }
}

fn status(state: &AppState) -> Result<Value, String> {
    let sources: Vec<Value> = state.db.read(|c| {
        let mut st = c.prepare(
            "SELECT s.id, s.path, s.last_scan, (SELECT COUNT(*) FROM items i WHERE i.source_id=s.id)
             FROM sources s ORDER BY s.id",
        )?;
        let rows = st.query_map([], |r| {
            let path: String = r.get(1)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "online": std::path::Path::new(&path).is_dir(),
                "path": path,
                "lastScan": r.get::<_, Option<i64>>(2)?,
                "files": r.get::<_, i64>(3)?,
            }))
        })?;
        rows.collect()
    })?;
    let (photos, videos, live, people, albums): (i64, i64, i64, i64, i64) = state.db.read(|c| {
        c.query_row(
            "SELECT
               (SELECT COUNT(*) FROM items WHERE hidden=0 AND trashed=0 AND kind=0),
               (SELECT COUNT(*) FROM items WHERE hidden=0 AND trashed=0 AND kind=1),
               (SELECT COUNT(*) FROM items WHERE hidden=0 AND trashed=0 AND (live_video_id IS NOT NULL OR motion=1)),
               (SELECT COUNT(*) FROM people),
               (SELECT COUNT(*) FROM albums)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
    })?;
    let (tdone, ttotal, tpaused) = state.thumbs.progress();
    let formats: Vec<Value> = export::available_formats()
        .iter()
        .map(|f| json!({ "id": f.id, "label": f.label, "ext": f.ext }))
        .collect();
    Ok(json!({
        "generation": state.generation(),
        "sources": sources,
        "counts": { "photos": photos, "videos": videos, "live": live, "people": people, "albums": albums },
        "scan": state.scan.lock().unwrap().clone(),
        "export": state.export.lock().unwrap().clone(),
        "thumbs": { "done": tdone, "total": ttotal, "paused": tpaused },
        "formats": formats,
        "platform": std::env::consts::OS,
        "dataDir": state.data_dir.to_string_lossy(),
        "ffmpeg": state.thumbs.has_ffmpeg(),
        "drive": state.drive_info(),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// Pages the About section may open in the browser.
const LINKS: &[&str] = &[
    "https://abhishekchoudhury.com",
    "https://github.com/yesabhishek/google-takeout-viewer",
    "https://maps.apple.com",
];

/// "Show in Finder" for paths the UI shows: library folders and the last
/// export destination.
fn revealable(state: &AppState, p: &Path) -> Result<bool, String> {
    let sources: Vec<String> = state.db.read(|c| {
        let mut st = c.prepare("SELECT path FROM sources")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    })?;
    let export_dest = state.export.lock().unwrap().dest.clone();
    Ok(sources.iter().any(|s| Path::new(s) == p) || (!export_dest.is_empty() && Path::new(&export_dest) == p))
}

/// Copy an original into our own folder, read-only, for opening in another
/// app. Editors like Preview save changes in place; this way they can only
/// ever change the copy.
pub(crate) fn open_copy(state: &AppState, id: i64) -> Result<PathBuf, String> {
    let (path, name): (String, String) = state
        .db
        .read(|c| c.query_row("SELECT path, name FROM items WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?))))?;
    let src = PathBuf::from(path);
    let size = std::fs::metadata(&src)
        .map_err(|_| "The drive with this file isn't connected.".to_string())?
        .len();
    let dir = state.open_copies_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join(format!("{id}-{name}"));
    if std::fs::metadata(&out).map(|m| m.len() == size).unwrap_or(false) {
        return Ok(out);
    }
    let tmp = dir.join(format!(".{id}.partial"));
    let _ = std::fs::remove_file(&tmp);
    std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let copied = {
        let _drive = DRIVE.acquire(Prio::Interactive);
        export::copy_verified(&src, 0, &tmp)
    };
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Couldn't read {name}: {e}"));
    }
    let mut perm = std::fs::metadata(&tmp).map_err(|e| e.to_string())?.permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&tmp, perm).map_err(|e| e.to_string())?;
    state.clean_open_copy(&out);
    std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn add_source(state: &Arc<AppState>, path: &str) -> Result<(), String> {
    let p = PathBuf::from(path);
    if !p.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    let p = p.canonicalize().unwrap_or(p);
    if diskio::inside(&state.data_dir, &p) || diskio::inside(&p, &state.data_dir) {
        return Err("That folder contains Takeout Viewer's own data. Choose the folder with your Takeout export instead.".into());
    }
    let ps = p.to_string_lossy().to_string();
    // Refuse nested sources: they would index the same files twice.
    let existing: Vec<String> = state.db.read(|c| {
        let mut st = c.prepare("SELECT path FROM sources")?;
        let rows = st.query_map([], |r| r.get(0))?;
        rows.collect()
    })?;
    for e in &existing {
        let ep = std::path::Path::new(e);
        if p.starts_with(ep) {
            return Err(format!("{ps} is already part of your library ({e})"));
        }
    }
    state.db.write(|c| {
        let tx = c.transaction()?;
        // A parent folder replaces sources inside it.
        for e in &existing {
            if std::path::Path::new(e).starts_with(&p) {
                tx.execute(
                    "UPDATE items SET source_id=-1 WHERE source_id=(SELECT id FROM sources WHERE path=?1)",
                    [e],
                )?;
                tx.execute("DELETE FROM sources WHERE path=?1", [e])?;
            }
        }
        tx.execute(
            "INSERT INTO sources(path, added_at) VALUES(?1, ?2) ON CONFLICT(path) DO NOTHING",
            params![ps, scan::now()],
        )?;
        let sid: i64 = tx.query_row("SELECT id FROM sources WHERE path=?1", [&ps], |r| r.get(0))?;
        tx.execute("UPDATE items SET source_id=?1 WHERE source_id=-1", [sid])?;
        tx.commit()
    })?;
    state.apply_drive_mode();
    scan::start(state);
    Ok(())
}

fn remove_source(state: &Arc<AppState>, id: i64) -> Result<(), String> {
    state.db.write(|c| {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM items WHERE source_id=?1", [id])?;
        tx.execute("DELETE FROM sources WHERE id=?1", [id])?;
        tx.commit()
    })?;
    state.apply_drive_mode();
    scan::relink(state)?;
    state.changed();
    Ok(())
}

#[derive(Deserialize, Default)]
pub struct Filter {
    #[serde(default)]
    view: String,
    id: Option<i64>,
    q: Option<String>,
}

const MONTHS: [&str; 12] = [
    "january", "february", "march", "april", "may", "june", "july", "august", "september", "october",
    "november", "december",
];

fn month_of(tok: &str) -> Option<usize> {
    if tok.len() < 3 {
        return None;
    }
    MONTHS.iter().position(|m| m.starts_with(tok)).map(|i| i + 1)
}

/// Binary timeline: [magic u32][n u32] f64 ts[n] u32 id[n] i32 day[n]
/// u16 w[n] u16 h[n] u16 dur_s[n] u8 flags[n]
fn timeline(state: &AppState, f: &Filter) -> Result<Vec<u8>, String> {
    let cache_key = format!("{}:{:?}:{:?}", f.view, f.id, f.q);
    let gen = state.generation();
    if let Some((g, b)) = state.timeline_cache.lock().unwrap().get(&cache_key) {
        if *g == gen {
            return Ok((**b).clone());
        }
    }

    let mut wh = String::from("i.hidden=0");
    let mut binds: Vec<SqlValue> = Vec::new();
    match f.view.as_str() {
        "videos" => wh.push_str(" AND i.kind=1 AND i.trashed=0 AND i.archived=0"),
        "live" => wh.push_str(" AND (i.live_video_id IS NOT NULL OR i.motion=1) AND i.trashed=0"),
        "favorites" => wh.push_str(" AND i.fav=1 AND i.trashed=0"),
        "archive" => wh.push_str(" AND i.archived=1 AND i.trashed=0"),
        "trash" => wh.push_str(" AND i.trashed=1"),
        "album" => {
            wh.push_str(" AND i.id IN (SELECT item_id FROM album_items WHERE album_id=?) AND i.trashed=0");
            binds.push(SqlValue::Integer(f.id.unwrap_or(-1)));
        }
        "person" => {
            wh.push_str(" AND i.id IN (SELECT item_id FROM item_people WHERE person_id=?) AND i.trashed=0");
            binds.push(SqlValue::Integer(f.id.unwrap_or(-1)));
        }
        "search" => {
            wh.push_str(" AND i.trashed=0");
            let q = f.q.clone().unwrap_or_default().to_lowercase();
            let toks: Vec<&str> = q.split_whitespace().collect();
            if toks.is_empty() {
                wh.push_str(" AND 0");
            }
            for t in toks {
                let like = format!("%{t}%");
                wh.push_str(
                    " AND (i.name LIKE ? OR i.description LIKE ? OR i.camera LIKE ?
                      OR i.id IN (SELECT ip.item_id FROM item_people ip JOIN people p ON p.id=ip.person_id WHERE p.name LIKE ?)
                      OR i.id IN (SELECT ai.item_id FROM album_items ai JOIN albums a ON a.id=ai.album_id WHERE a.title LIKE ?)",
                );
                for _ in 0..5 {
                    binds.push(SqlValue::Text(like.clone()));
                }
                if t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) {
                    wh.push_str(" OR strftime('%Y', i.taken_ts, 'unixepoch', 'localtime') = ?");
                    binds.push(SqlValue::Text(t.to_string()));
                }
                if let Some(m) = month_of(t) {
                    wh.push_str(" OR strftime('%m', i.taken_ts, 'unixepoch', 'localtime') = ?");
                    binds.push(SqlValue::Text(format!("{m:02}")));
                }
                if t == "video" || t == "videos" {
                    wh.push_str(" OR i.kind=1");
                }
                wh.push(')');
            }
        }
        _ => wh.push_str(" AND i.archived=0 AND i.trashed=0"),
    }
    let sql = format!(
        "SELECT i.id, i.taken_ts, i.width, i.height, i.duration_ms, i.kind,
                (i.live_video_id IS NOT NULL OR i.motion=1), i.fav
         FROM items i WHERE {wh} ORDER BY i.taken_ts DESC, i.id DESC"
    );
    struct T {
        id: u32,
        ts: i64,
        w: u32,
        h: u32,
        dur: u32,
        flags: u8,
    }
    let rows: Vec<T> = state.db.read(|c| {
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params_from_iter(binds.iter()), |r| {
            let kind: i64 = r.get(5)?;
            let live: bool = r.get(6)?;
            let fav: bool = r.get(7)?;
            Ok(T {
                id: r.get::<_, i64>(0)? as u32,
                ts: r.get(1)?,
                w: r.get::<_, i64>(2)?.max(0) as u32,
                h: r.get::<_, i64>(3)?.max(0) as u32,
                dur: (r.get::<_, Option<i64>>(4)?.unwrap_or(0) / 1000).clamp(0, 65535) as u32,
                flags: (kind == 1) as u8 | (live as u8) << 1 | (fav as u8) << 2,
            })
        })?;
        rows.collect()
    })?;

    let n = rows.len();
    let mut out = Vec::with_capacity(8 + n * 23);
    out.extend(0x544C_3031u32.to_le_bytes());
    out.extend((n as u32).to_le_bytes());
    for r in &rows {
        out.extend((r.ts as f64).to_le_bytes());
    }
    for r in &rows {
        out.extend(r.id.to_le_bytes());
    }
    // Local calendar day, computed once per hour bucket (DST-safe enough).
    let mut last: Option<(i64, i32, i64)> = None; // (hour bucket, day, offset)
    for r in &rows {
        let bucket = r.ts.div_euclid(3600);
        let day = match last {
            Some((b, _, off)) if b == bucket => (r.ts + off).div_euclid(86_400) as i32,
            _ => {
                let off = Local
                    .timestamp_opt(r.ts, 0)
                    .single()
                    .map(|d| d.offset().local_minus_utc() as i64)
                    .unwrap_or(0);
                let d = (r.ts + off).div_euclid(86_400) as i32;
                last = Some((bucket, d, off));
                d
            }
        };
        out.extend(day.to_le_bytes());
    }
    let dims: Vec<(u16, u16)> = rows
        .iter()
        .map(|r| {
            let (mut w, mut h) = if r.w == 0 || r.h == 0 { (4u32, 3u32) } else { (r.w, r.h) };
            while w > 65535 || h > 65535 {
                w /= 2;
                h /= 2;
            }
            (w.max(1) as u16, h.max(1) as u16)
        })
        .collect();
    for d in &dims {
        out.extend(d.0.to_le_bytes());
    }
    for d in &dims {
        out.extend(d.1.to_le_bytes());
    }
    for r in &rows {
        out.extend((r.dur as u16).to_le_bytes());
    }
    for r in &rows {
        out.push(r.flags);
    }
    state
        .timeline_cache
        .lock()
        .unwrap()
        .insert(cache_key, (gen, Arc::new(out.clone())));
    Ok(out)
}

fn item_path(state: &AppState, id: i64) -> Result<PathBuf, String> {
    state
        .db
        .read(|c| c.query_row("SELECT path FROM items WHERE id=?1", [id], |r| r.get::<_, String>(0)))
        .map(PathBuf::from)
}

fn item(state: &AppState, id: i64) -> Result<Value, String> {
    let mut v: Value = state.db.read(|c| {
        c.query_row(
            "SELECT id, path, name, kind, size, width, height, duration_ms, taken_ts, camera, lat, lon,
                    description, fav, live_video_id, motion, original_id, sidecar, archived, trashed,
                    meta_ts, sc_ts, dir_key,
                    (SELECT COUNT(*) FROM items d WHERE d.canonical_id=items.id)
             FROM items WHERE id=?1",
            [id],
            |r| {
                let name: String = r.get(2)?;
                let path: String = r.get(1)?;
                let ext = takeout::ext_of(&name);
                let kind: i64 = r.get(3)?;
                let live = r.get::<_, Option<i64>>(14)?.is_some() || r.get::<_, bool>(15)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "online": std::path::Path::new(&path).exists(),
                    "path": path,
                    "name": name,
                    "kind": kind,
                    "size": r.get::<_, i64>(4)?,
                    "width": r.get::<_, i64>(5)?,
                    "height": r.get::<_, i64>(6)?,
                    "durationMs": r.get::<_, Option<i64>>(7)?,
                    "taken": r.get::<_, i64>(8)?,
                    "camera": r.get::<_, Option<String>>(9)?,
                    "lat": r.get::<_, Option<f64>>(10)?,
                    "lon": r.get::<_, Option<f64>>(11)?,
                    "description": r.get::<_, Option<String>>(12)?,
                    "favorite": r.get::<_, bool>(13)?,
                    "live": live,
                    "originalId": r.get::<_, Option<i64>>(16)?,
                    "sidecar": r.get::<_, Option<String>>(17)?,
                    "archived": r.get::<_, bool>(18)?,
                    "trashed": r.get::<_, bool>(19)?,
                    "dateSource": if r.get::<_, Option<i64>>(21)?.is_some() { "takeout" }
                                  else if r.get::<_, Option<i64>>(20)?.is_some() { "exif" } else { "file" },
                    "folder": r.get::<_, String>(22)?,
                    "duplicates": r.get::<_, i64>(23)?,
                    "mime": protocol::mime_for(&ext),
                    "ext": ext,
                    "displayable": kind == 1 || protocol::web_displayable(&ext),
                    "playable": kind == 0 || protocol::web_playable(&ext),
                }))
            },
        )
    })?;
    let people: Vec<Value> = state.db.read(|c| {
        let mut st = c.prepare(
            "SELECT p.id, p.name FROM item_people ip JOIN people p ON p.id=ip.person_id WHERE ip.item_id=?1 ORDER BY p.name",
        )?;
        let rows = st.query_map([id], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)? })))?;
        rows.collect()
    })?;
    let albums: Vec<Value> = state.db.read(|c| {
        let mut st = c.prepare(
            "SELECT a.id, a.title FROM album_items ai JOIN albums a ON a.id=ai.album_id WHERE ai.item_id=?1 ORDER BY a.title",
        )?;
        let rows = st.query_map([id], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "title": r.get::<_, String>(1)? })))?;
        rows.collect()
    })?;
    v["people"] = json!(people);
    v["albums"] = json!(albums);
    Ok(v)
}

fn people(state: &AppState) -> Result<Value, String> {
    let rows: Vec<Value> = state.db.read(|c| {
        let mut st = c.prepare(
            "SELECT p.id, p.name, COUNT(*) AS n, MAX(i.taken_ts),
               (SELECT ip2.item_id FROM item_people ip2 JOIN items i2 ON i2.id=ip2.item_id
                 WHERE ip2.person_id=p.id AND i2.trashed=0 ORDER BY i2.kind, i2.taken_ts DESC LIMIT 1)
             FROM people p JOIN item_people ip ON ip.person_id=p.id JOIN items i ON i.id=ip.item_id
             WHERE i.trashed=0 GROUP BY p.id ORDER BY n DESC, p.name",
        )?;
        let rows = st.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "name": r.get::<_, String>(1)?,
                "count": r.get::<_, i64>(2)?,
                "latest": r.get::<_, Option<i64>>(3)?,
                "cover": r.get::<_, Option<i64>>(4)?,
            }))
        })?;
        rows.collect()
    })?;
    Ok(json!(rows))
}

fn albums(state: &AppState) -> Result<Value, String> {
    let rows: Vec<Value> = state.db.read(|c| {
        let mut st = c.prepare(
            "SELECT a.id, a.title, a.description, COUNT(*), MIN(i.taken_ts), MAX(i.taken_ts),
               (SELECT ai2.item_id FROM album_items ai2 JOIN items i2 ON i2.id=ai2.item_id
                 WHERE ai2.album_id=a.id AND i2.trashed=0 ORDER BY i2.taken_ts DESC LIMIT 1)
             FROM albums a JOIN album_items ai ON ai.album_id=a.id JOIN items i ON i.id=ai.item_id
             WHERE i.trashed=0 GROUP BY a.id ORDER BY MAX(i.taken_ts) DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "title": r.get::<_, String>(1)?,
                "description": r.get::<_, Option<String>>(2)?,
                "count": r.get::<_, i64>(3)?,
                "start": r.get::<_, Option<i64>>(4)?,
                "end": r.get::<_, Option<i64>>(5)?,
                "cover": r.get::<_, Option<i64>>(6)?,
            }))
        })?;
        rows.collect()
    })?;
    Ok(json!(rows))
}

fn set_favorite(state: &AppState, ids: &[i64], value: bool) -> Result<(), String> {
    state.db.write(|c| {
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached("UPDATE items SET user_fav=?1, fav=?1 WHERE id=?2")?;
            for id in ids {
                st.execute(params![value, id])?;
            }
        }
        tx.commit()
    })?;
    state.changed();
    Ok(())
}

fn cache_info(state: &AppState) -> Value {
    let mut bytes = 0u64;
    let mut files = 0u64;
    for e in walkdir::WalkDir::new(state.thumbs.dir()).into_iter().flatten() {
        if e.file_type().is_file() {
            files += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    json!({ "bytes": bytes, "files": files, "path": state.thumbs.dir().to_string_lossy() })
}
