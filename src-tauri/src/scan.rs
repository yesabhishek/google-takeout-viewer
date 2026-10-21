//! Library indexing: walk -> sidecars -> metadata -> link (dedupe, live
//! photos, edits, albums, people). Incremental: unchanged files are skipped.

use rayon::prelude::*;
use rusqlite::params;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use crate::diskio::{Prio, DRIVE};
use crate::meta;
use crate::state::AppState;
use crate::takeout::{self, AlbumMeta, DirSidecars, JsonKind, Sidecar};

struct Entry {
    source_id: i64,
    path: PathBuf,
    dir_key: String,
    name: String,
    ext: String,
    kind: i64,
    size: u64,
    mtime: i64,
}

struct JsonEntry {
    source_id: i64,
    dir_key: String,
    name: String,
    path: PathBuf,
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Start a scan in the background. Returns false if one is already running.
pub fn start(state: &Arc<AppState>) -> bool {
    {
        let mut s = state.scan.lock().unwrap();
        if s.running {
            return false;
        }
        *s = crate::state::ScanStatus { running: true, phase: "walk".into(), ..Default::default() };
    }
    state.scan_cancel.store(false, Ordering::SeqCst);
    // The scan has the drive to itself; thumbnails for visible photos still work.
    state.thumbs.set_scanning(true);
    let st = state.clone();
    std::thread::Builder::new()
        .name("scan".into())
        .spawn(move || {
            let res = run(&st);
            st.thumbs.set_scanning(false);
            st.update_scan(|s| {
                s.running = false;
                s.phase = "done".into();
                s.finished_at = Some(now());
                if let Err(e) = &res {
                    s.error = Some(e.clone());
                }
            });
            st.changed();
            st.queue_all_thumbs();
        })
        .expect("spawn scan thread");
    true
}

/// Run a scan on the current thread (CLI).
pub fn run_blocking(state: &Arc<AppState>) -> Result<(), String> {
    state.update_scan(|s| *s = crate::state::ScanStatus { running: true, ..Default::default() });
    state.thumbs.set_scanning(true);
    let r = run(state);
    state.thumbs.set_scanning(false);
    state.update_scan(|s| s.running = false);
    r
}

fn cancelled(state: &AppState) -> Result<(), String> {
    if state.scan_cancel.load(Ordering::SeqCst) {
        Err("Scan cancelled. Nothing was removed from your library.".into())
    } else {
        Ok(())
    }
}

const DISCONNECTED: &str =
    "The drive was disconnected during the scan. Nothing was removed from your library; reconnect it and press Rescan.";

/// Stop if a library folder vanished mid-scan (drive unplugged or asleep and
/// unmounted): finishing would make every file on it look deleted.
fn still_online(roots: &HashMap<i64, PathBuf>) -> Result<(), String> {
    if roots.values().all(|r| r.is_dir()) {
        Ok(())
    } else {
        Err(DISCONNECTED.into())
    }
}

fn run(state: &Arc<AppState>) -> Result<(), String> {
    let started = Instant::now();
    let sources: Vec<(i64, String)> = state.db.read(|c| {
        let mut st = c.prepare("SELECT id, path FROM sources ORDER BY id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })?;

    // 1. Walk (one thread, so the drive reads directories in order)
    let mut media: Vec<Entry> = Vec::new();
    let mut jsons: Vec<JsonEntry> = Vec::new();
    let mut roots: HashMap<i64, PathBuf> = HashMap::new();
    // Sources with folders that could not be listed: their missing files are
    // not treated as deleted.
    let mut incomplete: HashSet<i64> = HashSet::new();
    for (sid, root) in &sources {
        let root = PathBuf::from(root);
        if !root.is_dir() {
            continue;
        }
        if walk(state, *sid, &root, &mut media, &mut jsons)? > 0 {
            incomplete.insert(*sid);
        }
        roots.insert(*sid, root);
    }
    still_online(&roots)?;
    state.update_scan(|s| {
        s.phase = "sidecars".into();
        s.found = media.len();
        s.done = 0;
        s.total = jsons.len();
        s.message = format!("Reading {} metadata files", jsons.len());
    });

    // 2. Sidecars. A sidecar that exists but can't be read marks its folder, so
    //    dates and people already in the index are kept rather than wiped.
    let read_count = AtomicUsize::new(0);
    let gone = AtomicBool::new(false);
    let parsed: Vec<(usize, Option<JsonKind>)> = jsons
        .par_iter()
        .enumerate()
        .map(|(i, j)| {
            if state.scan_cancel.load(Ordering::Relaxed) || gone.load(Ordering::Relaxed) {
                return (i, None);
            }
            let kind = {
                let _drive = DRIVE.acquire(Prio::Background);
                match std::fs::metadata(&j.path) {
                    Ok(m) if m.len() >= 4 * 1024 * 1024 => Some(JsonKind::Other),
                    Ok(_) => std::fs::read(&j.path).ok().map(|b| takeout::parse_json(&b)),
                    Err(_) => None,
                }
            };
            if kind.is_none() && !roots[&j.source_id].is_dir() {
                gone.store(true, Ordering::Relaxed);
                return (i, None);
            }
            DRIVE.report(kind.is_some());
            let n = read_count.fetch_add(1, Ordering::Relaxed) + 1;
            if n % 500 == 0 {
                state.update_scan(|s| s.done = n);
            }
            (i, kind)
        })
        .collect();
    cancelled(state)?;
    still_online(&roots)?;
    let mut sidecars: Vec<(Sidecar, usize)> = Vec::new();
    let mut dirs: HashMap<String, DirSidecars> = HashMap::new();
    let mut album_meta: HashMap<String, AlbumMeta> = HashMap::new();
    let mut unreadable_dirs: HashSet<String> = HashSet::new();
    for (ji, kind) in parsed {
        let j = &jsons[ji];
        let Some(kind) = kind else {
            unreadable_dirs.insert(j.dir_key.clone());
            continue;
        };
        match kind {
            JsonKind::Sidecar(sc) => {
                let idx = sidecars.len();
                dirs.entry(j.dir_key.clone()).or_default().add(&j.name, sc.title.as_deref(), idx);
                sidecars.push((sc, ji));
            }
            JsonKind::Album(a) => {
                album_meta.insert(j.dir_key.clone(), a);
            }
            JsonKind::Other => {}
        }
    }

    // 3. Match media to sidecars
    let matched: Vec<Option<usize>> = media
        .par_iter()
        .map(|e| dirs.get(&e.dir_key).and_then(|d| d.find(&e.name)))
        .collect();

    // 4. Diff against the database
    let existing: HashMap<String, (i64, i64, i64, i64)> = state.db.read(|c| {
        let mut st = c.prepare("SELECT path, id, size, mtime, source_id FROM items")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?;
        rows.collect()
    })?;
    // A library folder that suddenly looks empty is a mount problem, not a
    // deleted library: keep its entries.
    for sid in roots.keys() {
        let had = existing.values().any(|v| v.3 == *sid);
        if had && !media.iter().any(|e| e.source_id == *sid) {
            incomplete.insert(*sid);
        }
    }
    let mut todo: Vec<usize> = Vec::new();
    let mut unchanged: Vec<usize> = Vec::new();
    for (i, e) in media.iter().enumerate() {
        match existing.get(e.path.to_string_lossy().as_ref()) {
            Some((_, size, mtime, _)) if *size == e.size as i64 && *mtime == e.mtime => unchanged.push(i),
            _ => todo.push(i),
        }
    }
    // Newest first, so the top of the timeline fills in first.
    let sc_ts = |i: usize| matched[i].and_then(|s| sidecars[s].0.taken).unwrap_or(media[i].mtime);
    todo.sort_by_key(|&i| std::cmp::Reverse(sc_ts(i)));

    // 5. Metadata for new / changed files
    state.update_scan(|s| {
        s.phase = "metadata".into();
        s.done = 0;
        s.total = todo.len();
        s.message = format!("Reading {} photos and videos", todo.len());
    });
    let mut last_link = Instant::now();
    let mut done = 0usize;
    let mut unreadable = 0usize;
    for chunk in todo.chunks(64) {
        cancelled(state)?;
        let metas: Vec<Option<meta::FileMeta>> = chunk
            .par_iter()
            .map(|&i| {
                if state.scan_cancel.load(Ordering::Relaxed) {
                    return None;
                }
                let e = &media[i];
                if gone.load(Ordering::Relaxed) {
                    return None;
                }
                let m = {
                    let _drive = DRIVE.acquire(Prio::Background);
                    meta::extract(&e.path, e.kind, &e.ext, e.size)
                };
                // An unplugged drive is not a read error; the check below stops the scan.
                if m.is_none() && !roots[&e.source_id].is_dir() {
                    gone.store(true, Ordering::Relaxed);
                    return None;
                }
                DRIVE.report(m.is_some());
                m
            })
            .collect();
        cancelled(state)?;
        still_online(&roots)?;
        // Files that could not be read stay as they were and are retried next scan.
        unreadable += metas.iter().filter(|m| m.is_none()).count();
        state.db.write(|c| {
            let tx = c.transaction()?;
            {
                let mut st = tx.prepare_cached(UPSERT)?;
                for (k, &i) in chunk.iter().enumerate() {
                    let e = &media[i];
                    let Some(m) = &metas[k] else { continue };
                    let sc = matched[i].map(|s| &sidecars[s].0);
                    let sc_path = matched[i].map(|s| jsons[sidecars[s].1].path.to_string_lossy().to_string());
                    let geo = sc.and_then(|s| s.geo).or(match (m.lat, m.lon) {
                        (Some(a), Some(b)) => Some((a, b)),
                        _ => None,
                    });
                    let taken = sc
                        .and_then(|s| s.taken)
                        .or(m.meta_ts)
                        .or(sc.and_then(|s| s.created))
                        .unwrap_or(e.mtime);
                    let people = sc
                        .filter(|s| !s.people.is_empty())
                        .map(|s| serde_json::to_string(&s.people).unwrap_or_default());
                    st.execute(params![
                        e.source_id,
                        e.path.to_string_lossy(),
                        e.dir_key,
                        e.name,
                        e.kind,
                        e.size as i64,
                        e.mtime,
                        m.fp,
                        m.width,
                        m.height,
                        m.duration_ms,
                        m.meta_ts,
                        sc.and_then(|s| s.taken),
                        sc.and_then(|s| s.created),
                        taken,
                        m.camera,
                        geo.map(|g| g.0),
                        geo.map(|g| g.1),
                        sc.and_then(|s| s.description.clone()),
                        people,
                        sc.map(|s| s.favorite).unwrap_or(false),
                        sc.map(|s| s.archived).unwrap_or(false),
                        sc.map(|s| s.trashed).unwrap_or(false),
                        m.motion,
                        sc_path,
                    ])?;
                }
            }
            tx.commit()
        })?;
        done += chunk.len();
        state.update_scan(|s| {
            s.done = done;
            s.skipped = unreadable;
        });
        // Show progress in the grid while a big import runs.
        if last_link.elapsed().as_secs() >= 8 && done < todo.len() {
            relink(state)?;
            state.changed();
            last_link = Instant::now();
        }
    }

    // 6. Refresh sidecar data for unchanged files (a later archive may have
    //    brought the missing JSON), and drop files that disappeared.
    state.update_scan(|s| {
        s.phase = "linking".into();
        s.message = "Grouping duplicates, Live Photos and albums".into();
    });
    still_online(&roots)?;
    let seen: HashSet<String> = media.iter().map(|e| e.path.to_string_lossy().to_string()).collect();
    state.db.write(|c| {
        let tx = c.transaction()?;
        {
            let mut st = tx.prepare_cached(
                "UPDATE items SET sc_ts=?2, sc_ctime=?3, description=?4, people=?5, sc_fav=?6,
                    sc_archived=?7, sc_trashed=?8, lat=COALESCE(?9, lat), lon=COALESCE(?10, lon),
                    sidecar=?11, dir_key=?12,
                    taken_ts=COALESCE(?2, meta_ts, ?3, mtime)
                 WHERE path=?1",
            )?;
            for &i in &unchanged {
                let e = &media[i];
                if unreadable_dirs.contains(&e.dir_key) {
                    continue;
                }
                let sc = matched[i].map(|s| &sidecars[s].0);
                let people = sc
                    .filter(|s| !s.people.is_empty())
                    .map(|s| serde_json::to_string(&s.people).unwrap_or_default());
                st.execute(params![
                    e.path.to_string_lossy(),
                    sc.and_then(|s| s.taken),
                    sc.and_then(|s| s.created),
                    sc.and_then(|s| s.description.clone()),
                    people,
                    sc.map(|s| s.favorite).unwrap_or(false),
                    sc.map(|s| s.archived).unwrap_or(false),
                    sc.map(|s| s.trashed).unwrap_or(false),
                    sc.and_then(|s| s.geo).map(|g| g.0),
                    sc.and_then(|s| s.geo).map(|g| g.1),
                    matched[i].map(|s| jsons[sidecars[s].1].path.to_string_lossy().to_string()),
                    e.dir_key,
                ])?;
            }
            // Forget files that are gone, but only for folders that were read
            // completely. This only touches the index, never the files.
            let mut del = tx.prepare_cached("DELETE FROM items WHERE id=?1")?;
            for (path, (id, _, _, sid)) in &existing {
                if roots.contains_key(sid) && !incomplete.contains(sid) && !seen.contains(path) {
                    del.execute([id])?;
                }
            }
            // Albums: Takeout album folders carry a metadata.json; plain folders
            // (non-Takeout sources) become albums by name.
            let mut tops: HashSet<&str> = HashSet::new();
            for e in &media {
                let t = takeout::top_dir(&e.dir_key);
                if !t.is_empty() {
                    tops.insert(t);
                }
            }
            let mut ins = tx.prepare_cached(
                "INSERT INTO albums(dir_key, title, description, ts) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(dir_key) DO UPDATE SET title=excluded.title,
                   description=excluded.description, ts=excluded.ts",
            )?;
            for t in tops {
                if let Some(m) = album_meta.get(t) {
                    let title = if m.title.is_empty() { t.to_string() } else { m.title.clone() };
                    ins.execute(params![t, title, m.description, m.ts])?;
                } else if !takeout::is_year_folder(t) && !takeout::is_special_folder(t) {
                    ins.execute(params![t, t, Option::<String>::None, Option::<i64>::None])?;
                }
            }
            let ts = now();
            for sid in roots.keys() {
                tx.execute("UPDATE sources SET last_scan=?1 WHERE id=?2", params![ts, sid])?;
            }
        }
        tx.commit()
    })?;

    relink(state)?;
    let mut problems: Vec<String> = Vec::new();
    if unreadable > 0 {
        problems.push(format!("{unreadable} files couldn't be read and will be retried next scan"));
    }
    if !incomplete.is_empty() || !unreadable_dirs.is_empty() {
        problems.push("some folders couldn't be read, so their existing entries were kept".into());
    }
    state.update_scan(|s| {
        s.message = format!("Indexed {} files in {}", media.len(), fmt_secs(started.elapsed().as_secs()));
        s.skipped = unreadable;
        s.warning = (!problems.is_empty()).then(|| {
            let mut w = problems.join("; ");
            w[..1].make_ascii_uppercase();
            w + ". Check the drive's connection if this keeps happening."
        });
    });
    Ok(())
}

fn fmt_secs(s: u64) -> String {
    match s {
        0 => "under a second".into(),
        1..60 => format!("{s}s"),
        60..3600 => format!("{}m {}s", s / 60, s % 60),
        _ => format!("{}h {}m", s / 3600, s % 3600 / 60),
    }
}

const UPSERT: &str = "INSERT INTO items (source_id, path, dir_key, name, kind, size, mtime, fp, width, height,
    duration_ms, meta_ts, sc_ts, sc_ctime, taken_ts, camera, lat, lon, description, people, sc_fav,
    sc_archived, sc_trashed, motion, sidecar)
  VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)
  ON CONFLICT(path) DO UPDATE SET source_id=excluded.source_id, dir_key=excluded.dir_key,
    name=excluded.name, kind=excluded.kind, size=excluded.size, mtime=excluded.mtime, fp=excluded.fp,
    width=excluded.width, height=excluded.height, duration_ms=excluded.duration_ms,
    meta_ts=excluded.meta_ts, sc_ts=excluded.sc_ts, sc_ctime=excluded.sc_ctime,
    taken_ts=excluded.taken_ts, camera=excluded.camera, lat=excluded.lat, lon=excluded.lon,
    description=excluded.description, people=excluded.people, sc_fav=excluded.sc_fav,
    sc_archived=excluded.sc_archived, sc_trashed=excluded.sc_trashed, motion=excluded.motion,
    motion_offset=NULL, sidecar=excluded.sidecar";

/// List one library folder. Returns how many entries could not be read.
fn walk(
    state: &AppState,
    source_id: i64,
    root: &Path,
    media: &mut Vec<Entry>,
    jsons: &mut Vec<JsonEntry>,
) -> Result<usize, String> {
    let mut errors = 0usize;
    let mut last_dir: Option<(PathBuf, String)> = None;
    let mut last_report = Instant::now();
    let it = WalkDir::new(root).follow_links(false).into_iter().filter_entry(|e| {
        if !e.file_type().is_dir() || e.depth() == 0 {
            return true;
        }
        let name = e.file_name().to_string_lossy();
        if takeout::skip_dir(&name) {
            return false;
        }
        // Inside a Takeout folder only the Google Photos part is interesting.
        let parent_is_takeout = e
            .path()
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| takeout::is_takeout_folder(&n.to_string_lossy()))
            .unwrap_or(false);
        !(parent_is_takeout && !takeout::is_photos_folder(&name) && !takeout::is_takeout_folder(&name))
    });
    for entry in it {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("._") || name.starts_with('.') {
            continue; // AppleDouble files on exFAT drives, .DS_Store …
        }
        let ext = takeout::ext_of(&name);
        let is_json = ext == "json";
        let kind = takeout::media_kind(&ext);
        if !is_json && kind.is_none() {
            continue;
        }
        let parent = entry.path().parent().unwrap_or(root).to_path_buf();
        let dir_key = match &last_dir {
            Some((p, k)) if *p == parent => k.clone(),
            _ => {
                let k = takeout::dir_key(root, &parent);
                last_dir = Some((parent.clone(), k.clone()));
                k
            }
        };
        if is_json {
            jsons.push(JsonEntry { source_id, dir_key, name, path: entry.path().to_path_buf() });
            continue;
        }
        let Ok(md) = entry.metadata() else {
            errors += 1;
            continue;
        };
        if md.len() == 0 {
            continue;
        }
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        media.push(Entry {
            source_id,
            path: entry.path().to_path_buf(),
            dir_key,
            name,
            ext,
            kind: kind.unwrap_or(0),
            size: md.len(),
            mtime,
        });
        if last_report.elapsed().as_millis() > 250 {
            let n = media.len();
            state.update_scan(|s| {
                s.found = n;
                s.message = format!("Found {n} photos and videos");
            });
            cancelled(state)?;
            last_report = Instant::now();
        }
    }
    Ok(errors)
}

// ---------------------------------------------------------------------------
// Linking

struct Row {
    id: i64,
    dir_key: String,
    name: String,
    kind: i64,
    size: i64,
    fp: i64,
    duration_ms: Option<i64>,
    taken: i64,
    people: Option<String>,
    sc_fav: bool,
    sc_archived: bool,
    sc_trashed: bool,
    user_fav: Option<bool>,
    old: (i64, Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>, i64, bool, bool, bool),
}

/// Recompute duplicates, Live Photo pairs, edited versions, album
/// membership and people from the per-file rows.
pub fn relink(state: &AppState) -> Result<(), String> {
    let rows: Vec<Row> = state.db.read(|c| {
        let mut st = c.prepare(
            "SELECT id, dir_key, name, kind, size, fp, duration_ms,
                    COALESCE(sc_ts, meta_ts, sc_ctime, mtime), people, sc_fav, sc_archived, sc_trashed, user_fav,
                    hidden, canonical_id, live_video_id, live_parent_id, edited_id, original_id, taken_ts,
                    fav, archived, trashed
             FROM items",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                dir_key: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                size: r.get(4)?,
                fp: r.get(5)?,
                duration_ms: r.get(6)?,
                taken: r.get(7)?,
                people: r.get(8)?,
                sc_fav: r.get(9)?,
                sc_archived: r.get(10)?,
                sc_trashed: r.get(11)?,
                user_fav: r.get(12)?,
                old: (
                    r.get(13)?,
                    r.get(14)?,
                    r.get(15)?,
                    r.get(16)?,
                    r.get(17)?,
                    r.get(18)?,
                    r.get(19)?,
                    r.get(20)?,
                    r.get(21)?,
                    r.get(22)?,
                ),
            })
        })?;
        rows.collect()
    })?;
    let n = rows.len();

    // Per-directory pairing
    let mut by_dir: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        by_dir.entry(r.dir_key.as_str()).or_default().push(i);
    }
    let mut live_parent: Vec<Option<usize>> = vec![None; n];
    let mut live_video: Vec<Option<usize>> = vec![None; n];
    let mut edited_of: Vec<Option<usize>> = vec![None; n]; // original -> edited
    let mut original_of: Vec<Option<usize>> = vec![None; n]; // edited -> original
    for idxs in by_dir.values() {
        let mut images_by_stem: HashMap<String, Vec<usize>> = HashMap::new();
        for &i in idxs {
            if rows[i].kind == 0 {
                images_by_stem
                    .entry(takeout::stem_of(&rows[i].name).to_lowercase())
                    .or_default()
                    .push(i);
            }
        }
        for &v in idxs {
            let r = &rows[v];
            if r.kind != 1 {
                continue;
            }
            let short = match r.duration_ms {
                Some(d) => d > 0 && d <= 6500,
                None => r.size < 20 * 1024 * 1024,
            };
            if !short {
                continue;
            }
            let lname = r.name.to_lowercase();
            let stem = takeout::stem_of(&lname).to_string();
            let cands = images_by_stem.get(&stem).into_iter().flatten().chain(images_by_stem.get(&lname).into_iter().flatten());
            for &img in cands {
                if live_video[img].is_none() {
                    live_video[img] = Some(v);
                    live_parent[v] = Some(img);
                    break;
                }
            }
        }
        for &e in idxs {
            if rows[e].kind != 0 {
                continue;
            }
            let stem = takeout::stem_of(&rows[e].name);
            if let Some(orig) = takeout::strip_edited(stem) {
                if let Some(list) = images_by_stem.get(&orig.to_lowercase()) {
                    let ext = takeout::ext_of(&rows[e].name);
                    let pick = list
                        .iter()
                        .copied()
                        .find(|&o| takeout::ext_of(&rows[o].name) == ext)
                        .or(list.first().copied());
                    if let Some(o) = pick {
                        if o != e && edited_of[o].is_none() {
                            edited_of[o] = Some(e);
                            original_of[e] = Some(o);
                        }
                    }
                }
            }
        }
    }

    // Duplicate groups (same content in year folder + album folders, or across archives)
    let mut groups: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        groups.entry((r.fp, r.size)).or_default().push(i);
    }
    // 0 visible, 1 duplicate, 2 live companion, 3 superseded by edit
    let mut hidden = vec![0i64; n];
    let mut canonical: Vec<usize> = (0..n).collect();
    for members in groups.values() {
        let companion = members.iter().find(|&&i| live_parent[i].is_some()).copied();
        let superseded = members.iter().find(|&&i| edited_of[i].is_some()).copied();
        let best = *members
            .iter()
            .min_by_key(|&&i| {
                let top = takeout::top_dir(&rows[i].dir_key);
                (!takeout::is_year_folder(top), rows[i].id)
            })
            .unwrap();
        for &i in members {
            canonical[i] = best;
            hidden[i] = if companion.is_some() {
                2
            } else if superseded.is_some() {
                3
            } else if i == best {
                0
            } else {
                1
            };
            if let Some(c) = companion {
                live_parent[i] = live_parent[i].or(live_parent[c]);
            }
            if let Some(s) = superseded {
                edited_of[i] = edited_of[i].or(edited_of[s]);
            }
        }
        // A visible copy inherits the Live video found next to any of its copies.
        if hidden[best] == 0 {
            if live_video[best].is_none() {
                live_video[best] = members.iter().find_map(|&i| live_video[i]);
            }
            if original_of[best].is_none() {
                original_of[best] = members.iter().find_map(|&i| original_of[i]);
            }
        }
    }

    // An edited Live Photo keeps the motion of its original.
    for i in 0..n {
        if hidden[i] == 0 && rows[i].kind == 0 && live_video[i].is_none() {
            if let Some(o) = original_of[i] {
                live_video[i] = live_video[o].or_else(|| live_video[canonical[o]]);
            }
        }
    }

    // Visible representative of every row.
    let rep = |mut i: usize| -> Option<usize> {
        for _ in 0..8 {
            match hidden[i] {
                0 => return Some(i),
                1 => i = canonical[i],
                2 => i = live_parent[i]?,
                3 => i = edited_of[i]?,
                _ => return None,
            }
        }
        None
    };
    let reps: Vec<Option<usize>> = (0..n).map(rep).collect();

    // Flags aggregated over every copy that maps onto a visible item.
    let mut fav = vec![false; n];
    let mut archived = vec![false; n];
    let mut trashed_all = vec![true; n];
    let mut people: Vec<HashSet<String>> = vec![HashSet::new(); n];
    for (i, r) in rows.iter().enumerate() {
        let Some(v) = reps[i] else { continue };
        let top = takeout::top_dir(&r.dir_key);
        fav[v] |= r.sc_fav;
        archived[v] |= r.sc_archived || takeout::is_archive_folder(top);
        trashed_all[v] &= r.sc_trashed || takeout::is_trash_folder(top);
        if let Some(p) = &r.people {
            if let Ok(names) = serde_json::from_str::<Vec<String>>(p) {
                people[v].extend(names);
            }
        }
    }

    let albums: HashMap<String, i64> = state.db.read(|c| {
        let mut st = c.prepare("SELECT dir_key, id FROM albums")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })?;

    state.db.write(|c| {
        let tx = c.transaction()?;
        {
            let mut up = tx.prepare_cached(
                "UPDATE items SET hidden=?2, canonical_id=?3, live_video_id=?4, live_parent_id=?5,
                   edited_id=?6, original_id=?7, taken_ts=?8, fav=?9, archived=?10, trashed=?11 WHERE id=?1",
            )?;
            for (i, r) in rows.iter().enumerate() {
                let id_of = |x: Option<usize>| x.map(|j| rows[j].id);
                let visible = hidden[i] == 0;
                let taken = match (hidden[i], live_parent[i]) {
                    (2, Some(p)) => rows[p].taken,
                    _ => r.taken,
                };
                let f = r.user_fav.unwrap_or(fav[i]);
                let new = (
                    hidden[i],
                    if hidden[i] == 1 { Some(rows[canonical[i]].id) } else { None },
                    if visible { id_of(live_video[i]) } else { None },
                    id_of(live_parent[i]),
                    id_of(edited_of[i]),
                    if visible { id_of(original_of[i]) } else { None },
                    taken,
                    visible && f,
                    visible && archived[i],
                    visible && trashed_all[i],
                );
                if new != r.old {
                    up.execute(params![
                        r.id, new.0, new.1, new.2, new.3, new.4, new.5, new.6, new.7, new.8, new.9
                    ])?;
                }
            }

            tx.execute("DELETE FROM album_items", [])?;
            let mut ai = tx.prepare_cached("INSERT OR IGNORE INTO album_items(album_id, item_id) VALUES(?1,?2)")?;
            for (i, r) in rows.iter().enumerate() {
                let top = takeout::top_dir(&r.dir_key);
                if let (Some(&aid), Some(v)) = (albums.get(top), reps[i]) {
                    ai.execute(params![aid, rows[v].id])?;
                }
            }
            tx.execute("DELETE FROM albums WHERE id NOT IN (SELECT DISTINCT album_id FROM album_items)", [])?;

            tx.execute("DELETE FROM item_people", [])?;
            let mut pid_cache: HashMap<String, i64> = HashMap::new();
            let mut ins_p = tx.prepare_cached("INSERT INTO people(name) VALUES(?1) ON CONFLICT(name) DO NOTHING")?;
            let mut get_p = tx.prepare_cached("SELECT id FROM people WHERE name=?1")?;
            let mut ip = tx.prepare_cached("INSERT OR IGNORE INTO item_people(item_id, person_id) VALUES(?1,?2)")?;
            for (i, names) in people.iter().enumerate() {
                for name in names {
                    let key = name.to_lowercase();
                    let pid = match pid_cache.get(&key) {
                        Some(&p) => p,
                        None => {
                            ins_p.execute([name])?;
                            let p: i64 = get_p.query_row([name], |r| r.get(0))?;
                            pid_cache.insert(key, p);
                            p
                        }
                    };
                    ip.execute(params![rows[i].id, pid])?;
                }
            }
            tx.execute("DELETE FROM people WHERE id NOT IN (SELECT DISTINCT person_id FROM item_people)", [])?;
        }
        tx.commit()
    })?;
    Ok(())
}
