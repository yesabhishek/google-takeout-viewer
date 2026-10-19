//! Everything specific to the Google Takeout layout: JSON sidecars, their
//! (many) naming schemes, album metadata, and folder classification.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "heic", "heif", "hif", "avif", "gif", "webp", "tif", "tiff",
    "bmp", "dng", "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "orf", "rw2", "raf", "pef",
    "srw", "raw", "ico",
];
pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "qt", "3gp", "3g2", "avi", "mkv", "webm", "wmv", "mpg", "mpeg", "mts",
    "m2ts", "ts", "flv", "mp",
];

pub fn media_kind(ext: &str) -> Option<i64> {
    if IMAGE_EXTS.contains(&ext) {
        Some(0)
    } else if VIDEO_EXTS.contains(&ext) {
        Some(1)
    } else {
        None
    }
}

/// Lowercased extension (without dot).
pub fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

pub fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

// ---------------------------------------------------------------------------
// JSON parsing

#[derive(Deserialize, Default)]
struct TimeField {
    timestamp: Option<Value>,
}

impl TimeField {
    fn secs(&self) -> Option<i64> {
        let v = match &self.timestamp {
            Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
            Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            _ => None,
        }?;
        // 0 and negative values show up for "unknown"
        if v > 0 {
            Some(v)
        } else {
            None
        }
    }
}

#[derive(Deserialize, Default)]
struct Geo {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

impl Geo {
    fn point(&self) -> Option<(f64, f64)> {
        let (la, lo) = (self.latitude?, self.longitude?);
        if la.abs() < 1e-7 && lo.abs() < 1e-7 {
            None
        } else {
            Some((la, lo))
        }
    }
}

#[derive(Deserialize)]
struct PersonField {
    name: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawJson {
    title: Option<String>,
    description: Option<String>,
    #[serde(rename = "photoTakenTime")]
    photo_taken_time: Option<TimeField>,
    #[serde(rename = "creationTime")]
    creation_time: Option<TimeField>,
    #[serde(rename = "geoData")]
    geo_data: Option<Geo>,
    #[serde(rename = "geoDataExif")]
    geo_data_exif: Option<Geo>,
    people: Option<Vec<PersonField>>,
    favorited: Option<bool>,
    archived: Option<bool>,
    trashed: Option<bool>,
    date: Option<TimeField>,
    #[serde(rename = "albumData")]
    album_data: Option<AlbumData>,
}

#[derive(Deserialize, Default)]
struct AlbumData {
    title: Option<String>,
    description: Option<String>,
    date: Option<TimeField>,
}

#[derive(Debug, Clone, Default)]
pub struct Sidecar {
    pub title: Option<String>,
    pub description: Option<String>,
    pub taken: Option<i64>,
    pub created: Option<i64>,
    pub geo: Option<(f64, f64)>,
    pub people: Vec<String>,
    pub favorite: bool,
    pub archived: bool,
    pub trashed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct AlbumMeta {
    pub title: String,
    pub description: Option<String>,
    pub ts: Option<i64>,
}

pub enum JsonKind {
    Sidecar(Sidecar),
    Album(AlbumMeta),
    Other,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn parse_json(bytes: &[u8]) -> JsonKind {
    let raw: RawJson = match serde_json::from_slice(bytes) {
        Ok(r) => r,
        Err(_) => return JsonKind::Other,
    };
    if let Some(ad) = raw.album_data {
        return JsonKind::Album(AlbumMeta {
            title: non_empty(ad.title).unwrap_or_default(),
            description: non_empty(ad.description),
            ts: ad.date.and_then(|d| d.secs()),
        });
    }
    let is_sidecar =
        raw.photo_taken_time.is_some() || (raw.creation_time.is_some() && raw.date.is_none());
    if is_sidecar {
        let geo = raw
            .geo_data
            .as_ref()
            .and_then(|g| g.point())
            .or_else(|| raw.geo_data_exif.as_ref().and_then(|g| g.point()));
        let people = raw
            .people
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| non_empty(p.name))
            .collect();
        return JsonKind::Sidecar(Sidecar {
            title: non_empty(raw.title),
            description: non_empty(raw.description),
            taken: raw.photo_taken_time.and_then(|t| t.secs()),
            created: raw.creation_time.and_then(|t| t.secs()),
            geo,
            people,
            favorite: raw.favorited.unwrap_or(false),
            archived: raw.archived.unwrap_or(false),
            trashed: raw.trashed.unwrap_or(false),
        });
    }
    if raw.title.is_some() {
        return JsonKind::Album(AlbumMeta {
            title: non_empty(raw.title).unwrap_or_default(),
            description: non_empty(raw.description),
            ts: raw.date.and_then(|d| d.secs()),
        });
    }
    JsonKind::Other
}

// ---------------------------------------------------------------------------
// Sidecar name matching
//
// Takeout names sidecars in several ways, all subject to a 51 character limit:
//   IMG_1234.JPG.json
//   IMG_1234.JPG.supplemental-metadata.json   (2024+), truncated: .supplemental-metad.json, .suppl.json, .s.json …
//   IMG_1234.JPG(1).json / IMG_1234.JPG.supplemental-metadata(1).json  for the media file IMG_1234(1).JPG
//   IMG_1234.json                              (older exports, extension dropped)
//   very_long_file_name_that_got_truncated_at.json

const SUPPLEMENTAL: &str = "supplemental-metadata";

/// Split a trailing "(n)" off a string.
fn split_dup(s: &str) -> (&str, u32) {
    if let Some(stripped) = s.strip_suffix(')') {
        if let Some(open) = stripped.rfind('(') {
            let digits = &stripped[open + 1..];
            if !digits.is_empty() && digits.len() <= 4 && digits.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(n) = digits.parse::<u32>() {
                    return (&s[..open], n);
                }
            }
        }
    }
    (s, 0)
}

/// From a sidecar file name, return (lowercased media-name prefix, duplicate index).
pub fn sidecar_key(json_name: &str) -> Option<(String, u32)> {
    let lower = json_name.to_lowercase();
    let base = lower.strip_suffix(".json")?;
    let (base, dup) = split_dup(base);
    let mut base = base.to_string();
    if let Some(dot) = base.rfind('.') {
        let tail = &base[dot + 1..];
        // ".supplemental-metadata" or any truncation of it (including a lone ".")
        if SUPPLEMENTAL.starts_with(tail) && media_kind(tail).is_none() {
            base.truncate(dot);
        }
    }
    if base.is_empty() {
        return None;
    }
    Some((base, dup))
}

pub const EDITED_SUFFIXES: &[&str] = &[
    "-edited", "-bearbeitet", "-modifié", "-modifie", "-editado", "-modificado", "-modificato",
    "-bewerkt", "-redigeret", "-redigert", "-redigerad", "-muokattu", "-edytowane", "-upraveno",
    "-upravené", "-szerkesztett", "-изменено", "-düzenlendi", "-編集済み", "-편집됨", "-已编辑",
    "-已編輯", "-ditambahkan", "-diedit", "-đã chỉnh sửa", "-แก้ไขแล้ว",
];

/// If `stem` ends with an "edited" marker, return the original stem.
pub fn strip_edited(stem: &str) -> Option<&str> {
    let lower = stem.to_lowercase();
    for suf in EDITED_SUFFIXES {
        if lower.ends_with(suf) {
            // suffix byte length is the same in the lowercase and original
            // for all entries above except case-insensitive ASCII, which is fine.
            let cut = stem.len().checked_sub(suf.len())?;
            if stem.is_char_boundary(cut) {
                return Some(&stem[..cut]);
            }
        }
    }
    None
}

/// Sidecars of one logical directory (merged across split Takeout archives).
#[derive(Default)]
pub struct DirSidecars {
    exact: HashMap<(String, u32), usize>,
    all: Vec<(String, u32, usize)>,
    by_title: HashMap<String, Vec<(u32, usize)>>,
}

impl DirSidecars {
    pub fn add(&mut self, json_name: &str, title: Option<&str>, idx: usize) {
        if let Some((prefix, dup)) = sidecar_key(json_name) {
            self.exact.entry((prefix.clone(), dup)).or_insert(idx);
            self.all.push((prefix, dup, idx));
        }
        if let Some(t) = title {
            let (_, dup) = sidecar_key(json_name).unwrap_or_default();
            self.by_title.entry(t.to_lowercase()).or_default().push((dup, idx));
        }
    }

    /// Find the sidecar for a media file name in this directory.
    pub fn find(&self, media_name: &str) -> Option<usize> {
        let lname = media_name.to_lowercase();
        let mut cands: Vec<(String, u32)> = vec![(lname.clone(), 0)];

        // IMG_1234(1).JPG -> (IMG_1234.JPG, 1)
        let stem = stem_of(&lname);
        let ext = &lname[stem.len()..]; // includes the dot, may be empty
        let (dstem, dup) = split_dup(stem);
        if dup > 0 {
            cands.push((format!("{dstem}{ext}"), dup));
        }
        // IMG_1234-edited.JPG, IMG_1234(1)-edited.JPG, IMG_1234-edited(1).JPG -> original
        for (s, d) in [(stem, 0u32), (dstem, dup)] {
            if let Some(orig) = strip_edited(s) {
                let (ostem, odup) = split_dup(orig);
                let odup = if odup > 0 { odup } else { d };
                cands.push((format!("{ostem}{ext}"), odup));
                if odup > 0 {
                    cands.push((format!("{ostem}{ext}"), 0));
                }
            }
        }

        for c in &cands {
            if let Some(&i) = self.exact.get(c) {
                return Some(i);
            }
        }
        // Extension-less sidecar (IMG_1234.json)
        for (name, dup) in &cands {
            let s = stem_of(name).to_string();
            if let Some(&i) = self.exact.get(&(s, *dup)) {
                return Some(i);
            }
        }
        // Truncated sidecar names: longest prefix wins.
        for (name, dup) in &cands {
            let mut best: Option<(usize, usize)> = None;
            for (prefix, pdup, idx) in &self.all {
                if *pdup == *dup && prefix.chars().count() >= 30 && name.starts_with(prefix.as_str())
                    && best.is_none_or(|(l, _)| prefix.len() > l) {
                        best = Some((prefix.len(), *idx));
                    }
            }
            if let Some((_, i)) = best {
                return Some(i);
            }
        }
        // Last resort: the sidecar's "title" holds the original file name.
        for (name, dup) in &cands {
            if let Some(list) = self.by_title.get(name) {
                if let Some((_, i)) = list.iter().find(|(d, _)| d == dup).or(list.first()) {
                    return Some(*i);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Folders

/// Is this a folder of the Google Photos part of a Takeout archive?
pub fn is_photos_folder(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("photo")
        || l.contains("foto")
        || l.contains("фото")
        || l.contains("相簿")
        || l.contains("相册")
        || l.contains("フォト")
        || l.contains("포토")
}

/// "Takeout", "Takeout 2", "Takeout (1)" — but not the archive-named parent
/// folder "takeout-20240101T000000Z-001" that unzipping creates.
pub fn is_takeout_folder(name: &str) -> bool {
    let l = name.to_lowercase();
    match l.strip_prefix("takeout") {
        Some(rest) => rest.chars().all(|c| c.is_ascii_digit() || matches!(c, ' ' | '-' | '_' | '(' | ')')),
        None => false,
    }
}

/// Directory key: the path of the containing folder relative to the
/// "Google Photos" folder, so that split archives merge into one tree.
pub fn dir_key(root: &Path, dir: &Path) -> String {
    let rel = dir.strip_prefix(root).unwrap_or(dir);
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    // Find the last "<Takeout*>/<Google Photos>" pair, or a lone Google Photos folder.
    let mut cut: Option<usize> = None;
    for i in 0..comps.len() {
        let gp = is_photos_folder(&comps[i]) && comps[i].to_lowercase().starts_with("google");
        let after_takeout = i > 0 && is_takeout_folder(&comps[i - 1]) && is_photos_folder(&comps[i]);
        if gp || after_takeout {
            cut = Some(i + 1);
        }
    }
    let start = cut.unwrap_or(0);
    comps[start.min(comps.len())..].join("/")
}

pub fn top_dir(dir_key: &str) -> &str {
    dir_key.split('/').next().unwrap_or("")
}

/// "Photos from 2019", "Fotos von 2019", "2019" …
pub fn is_year_folder(name: &str) -> bool {
    let n = name.trim();
    if n.len() < 4 {
        return false;
    }
    let (head, year) = n.split_at(n.len() - 4);
    let year_ok = year.bytes().all(|b| b.is_ascii_digit())
        && matches!(year.parse::<u32>(), Ok(y) if (1800..=2100).contains(&y));
    year_ok && !head.chars().any(|c| c.is_ascii_digit())
}

const ARCHIVE_NAMES: &[&str] = &[
    "archive", "archiv", "archivo", "archivio", "archief", "archiwum", "arkiv", "arquivo", "архив",
    "archives",
];
const TRASH_NAMES: &[&str] = &[
    "trash", "bin", "papierkorb", "corbeille", "papelera", "cestino", "prullenbak", "kosz",
    "lixeira", "papperskorg", "papirkurv", "roskakori", "корзина", "ゴミ箱", "휴지통",
];
const SKIP_ALBUM_NAMES: &[&str] = &["locked folder", "failed videos", "untitled"];

pub fn is_archive_folder(name: &str) -> bool {
    ARCHIVE_NAMES.contains(&name.to_lowercase().as_str())
}
pub fn is_trash_folder(name: &str) -> bool {
    TRASH_NAMES.contains(&name.to_lowercase().as_str())
}
pub fn is_special_folder(name: &str) -> bool {
    is_archive_folder(name)
        || is_trash_folder(name)
        || SKIP_ALBUM_NAMES.contains(&name.to_lowercase().as_str())
}

/// Folders we never descend into.
pub fn skip_dir(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('@') // Synology @eaDir
        || name.starts_with('#')
        || name.eq_ignore_ascii_case("$RECYCLE.BIN")
        || name.eq_ignore_ascii_case("System Volume Information")
        || name.ends_with(".photoslibrary")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(names: &[&str]) -> DirSidecars {
        let mut d = DirSidecars::default();
        for (i, n) in names.iter().enumerate() {
            d.add(n, None, i);
        }
        d
    }

    #[test]
    fn sidecar_keys() {
        assert_eq!(sidecar_key("IMG_1234.JPG.json"), Some(("img_1234.jpg".into(), 0)));
        assert_eq!(
            sidecar_key("IMG_1234.JPG.supplemental-metadata.json"),
            Some(("img_1234.jpg".into(), 0))
        );
        assert_eq!(sidecar_key("IMG_1234.JPG.supplemental-me.json"), Some(("img_1234.jpg".into(), 0)));
        assert_eq!(sidecar_key("IMG_1234.JPG.s.json"), Some(("img_1234.jpg".into(), 0)));
        assert_eq!(sidecar_key("IMG_1234.JPG..json"), Some(("img_1234.jpg".into(), 0)));
        assert_eq!(sidecar_key("IMG_1234.JPG(2).json"), Some(("img_1234.jpg".into(), 2)));
        assert_eq!(
            sidecar_key("IMG_1234.JPG.supplemental-metadata(1).json"),
            Some(("img_1234.jpg".into(), 1))
        );
        assert_eq!(sidecar_key("metadata.json"), Some(("metadata".into(), 0)));
    }

    #[test]
    fn matching() {
        let d = dir(&[
            "IMG_1234.JPG.supplemental-metadata.json",
            "IMG_1234.JPG.supplemental-metadata(1).json",
            "PXL_20230101_123456789.MP.jpg.supplemental-m.json",
            "IMG_5555.json",
            "a_really_long_file_name_from_whatsapp_and_frie.json",
            "IMG_7777.HEIC.json",
        ]);
        assert_eq!(d.find("IMG_1234.JPG"), Some(0));
        assert_eq!(d.find("IMG_1234(1).JPG"), Some(1));
        assert_eq!(d.find("IMG_1234-edited.JPG"), Some(0));
        assert_eq!(d.find("IMG_1234(1)-edited.JPG"), Some(1));
        assert_eq!(d.find("PXL_20230101_123456789.MP.jpg"), Some(2));
        assert_eq!(d.find("IMG_5555.PNG"), Some(3));
        assert_eq!(d.find("a_really_long_file_name_from_whatsapp_and_frie.jpg"), Some(4));
        assert_eq!(d.find("a_really_long_file_name_from_whatsapp_and_friends.jpg"), Some(4));
        assert_eq!(d.find("IMG_7777.HEIC"), Some(5));
        assert_eq!(d.find("IMG_9999.JPG"), None);
    }

    #[test]
    fn edited() {
        assert_eq!(strip_edited("IMG_1-edited"), Some("IMG_1"));
        assert_eq!(strip_edited("IMG_1-bearbeitet"), Some("IMG_1"));
        assert_eq!(strip_edited("IMG_1"), None);
    }

    #[test]
    fn dir_keys() {
        let root = PathBuf::from("/Volumes/X");
        assert_eq!(
            dir_key(&root, Path::new("/Volumes/X/takeout-001/Takeout/Google Photos/Trip")),
            "Trip"
        );
        assert_eq!(
            dir_key(&root, Path::new("/Volumes/X/Takeout 2/Google Fotos/Fotos von 2020")),
            "Fotos von 2020"
        );
        assert_eq!(dir_key(&root, Path::new("/Volumes/X/Takeout/Google Photos")), "");
        assert_eq!(dir_key(&root, Path::new("/Volumes/X/Holiday/Day 1")), "Holiday/Day 1");
    }

    #[test]
    fn takeout_folders() {
        assert!(is_takeout_folder("Takeout"));
        assert!(is_takeout_folder("Takeout 2"));
        assert!(is_takeout_folder("Takeout (1)"));
        assert!(!is_takeout_folder("takeout-20240101T000000Z-001"));
        assert!(!is_takeout_folder("Takeout photos backup"));
    }

    #[test]
    fn year_folders() {
        assert!(is_year_folder("Photos from 2019"));
        assert!(is_year_folder("Fotos von 2020"));
        assert!(is_year_folder("2021"));
        assert!(!is_year_folder("Trip 2 2019"));
        assert!(!is_year_folder("Summer"));
    }

    #[test]
    fn parse() {
        let s = br#"{"title":"IMG_1.JPG","description":"","photoTakenTime":{"timestamp":"1600000000"},
            "creationTime":{"timestamp":"1600000100"},"geoData":{"latitude":0.0,"longitude":0.0},
            "geoDataExif":{"latitude":48.8,"longitude":2.3},"people":[{"name":"Ada"}],"favorited":true}"#;
        match parse_json(s) {
            JsonKind::Sidecar(sc) => {
                assert_eq!(sc.taken, Some(1600000000));
                assert_eq!(sc.geo, Some((48.8, 2.3)));
                assert_eq!(sc.people, vec!["Ada".to_string()]);
                assert!(sc.favorite);
            }
            _ => panic!("not a sidecar"),
        }
        let a = br#"{"title":"Trip","description":"","access":"protected","date":{"timestamp":"1500000000"}}"#;
        assert!(matches!(parse_json(a), JsonKind::Album(m) if m.title == "Trip" && m.ts == Some(1500000000)));
    }
}
