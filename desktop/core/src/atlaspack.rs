//! Atlas pack files (`.atlaspack`, docs/PLAN.md, D215): what Game Repo reads of a pack
//! Atlas Importer wrote, by the *Atlas pack contract for Game Repo*.
//!
//! The reading is Atlas's own `atlas-pack` crate (the contract's block A3), a git
//! dependency pinned to the commit of Atlas's main branch the contract names, until Atlas
//! tags a release with it; its `docs/pack-file-format.md` is the reference for the format.
//! [`read_header`] reads the description from the front of the file only, and
//! [`read_preview`] one small picture the description lists, from the entries right behind
//! it. A file that's wrong gets the crate's own sentence (not a pack file, a newer format
//! naming the app that wrote it, a damaged description), shown as it is: Game Repo doesn't
//! guess. Nothing in a pack is ever unpacked to disk by the names inside it, and no address
//! in it is ever fetched.
//!
//! What's Game Repo's own here: [`Description`], the crate's `Header` as a card reads it,
//! every field optional (a format 6 pack has no `game`, `maps`, `guides` or `cover`), so it
//! can also be kept in the index and read back from there; which of two files of a pack is
//! the newer ([`Description::newer`]); the name the contract gives a pack's file; and
//! whether a file's name says it's a pack.

use serde::Serialize;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::path::Path;

pub use atlas_pack::{EXTENSION, FORMAT, VERSION};

/// A pack's description, read from the front of its file by Atlas's crate: refused, with
/// the crate's sentence, when it isn't a pack file, needs a newer format than
/// [`VERSION`], or is damaged.
pub fn read_header(path: &Path) -> Result<Description, String> {
    let h = atlas_pack::read_header(path)?;
    Ok(Description::from_value(serde_json::to_value(&h).map_err(|e| format!("This pack file's description is damaged: {e}."))?))
}

/// One small picture of a pack (its cover or a map's) by the name its description gives,
/// read by Atlas's crate: a safe `preview/<name>.jpg` name the description lists, right
/// behind the description, a JPEG of at most 512 KB.
pub fn read_preview(path: &Path, name: &str) -> Result<Vec<u8>, String> {
    atlas_pack::read_preview(path, name)
}

/// The sentence for a description that has no `packId` (the crate takes an empty one).
pub const NO_PACK_ID: &str = "This pack file's description is damaged: it has no packId.";

// ------------------------------------------------------------ the description

/// The app that wrote a pack (`exportedBy`).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Exporter {
    pub name: Option<String>,
    pub version: Option<String>,
    pub platform: Option<String>,
}

/// Where a pack was read from (`source`). The addresses it read (`source.pages`) are
/// counted, never kept here and never fetched.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Source {
    pub kind: Option<String>,
    /// The site's name where the pack records it, else its host name (block A3; a field of
    /// format 7's, in a file of any version)
    pub name: Option<String>,
    pub importer_id: Option<String>,
    pub importer_title: Option<String>,
    pub newest_page_at: Option<String>,
    pub oldest_page_at: Option<String>,
    pub pages: usize,
}

/// The game a pack is for (`game`, block A3): `title` is the title the packs were put under
/// in Atlas Importer.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Game {
    pub title: Option<String>,
    /// As the site named it: a hint for matching, never proof
    pub system: Option<String>,
}

/// A map in a pack (`maps[]`, block A3).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MapInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub locations: Option<u64>,
    /// Its small picture's name in the pack (`preview/<name>.jpg`); none for a map with no
    /// image, and none in a file below version 7
    pub preview: Option<String>,
}

/// A pack's pages, as a guide or a wiki (`guides[]`: one entry at most, as Atlas writes it).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GuideInfo {
    pub title: Option<String>,
    /// guide (pages read in order) | wiki; any other is a type a newer Atlas knows ("other"
    /// as the crate reads it), shown as a guide
    pub kind: Option<String>,
    pub pages: Option<u64>,
}

/// A pack's description (`atlaspack.json`), as a card reads it: every field optional, the
/// whole of it kept in `raw` (the crate's `Header` as JSON, which is what the index keeps).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Description {
    /// The lowest format needed to read it (absent: 1)
    pub version: u64,
    /// The pack's identity: the same every time it's written again
    pub pack_id: Option<String>,
    pub title: Option<String>,
    pub pack_version: Option<String>,
    /// overview | standard | full
    pub detail: Option<String>,
    pub exported_at: Option<String>,
    pub exported_by: Exporter,
    pub created: Option<String>,
    /// `pack.updated`: when its content last changed (what two files of a pack compare by)
    pub updated: Option<String>,
    pub source: Source,
    /// `contents`: maps, layers, locations, categories, regions, checklists, guides…
    pub contents: BTreeMap<String, u64>,
    pub files: Option<u64>,
    pub bytes: Option<u64>,
    pub game: Game,
    pub personal_use: bool,
    pub cover: Option<String>,
    pub maps: Vec<MapInfo>,
    pub guides: Vec<GuideInfo>,
    /// The whole description as it was read
    #[serde(skip)]
    pub raw: Value,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn number(v: &Value, k: &str) -> Option<u64> {
    num_of(v.get(k)?)
}

fn num_of(x: &Value) -> Option<u64> {
    x.as_u64().or_else(|| x.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)).or_else(|| x.as_str()?.trim().parse().ok())
}

impl Description {
    /// A description already checked by [`read_header`] (the crate's `Header` as JSON, or as
    /// the index kept it): what isn't there, or isn't what it should be, is left out.
    pub fn from_value(raw: Value) -> Description {
        let v = &raw;
        let by = v.get("exportedBy").cloned().unwrap_or(Value::Null);
        let pack = v.get("pack").cloned().unwrap_or(Value::Null);
        let src = v.get("source").cloned().unwrap_or(Value::Null);
        let importer = src.get("importer").cloned().unwrap_or(Value::Null);
        let game = v.get("game").cloned().unwrap_or(Value::Null);
        let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        Description {
            version: number(v, "version").unwrap_or(1),
            pack_id: text(v, "packId").or_else(|| text(&pack, "id")),
            title: text(v, "title").or_else(|| text(&pack, "title")),
            pack_version: text(v, "packVersion").or_else(|| text(&pack, "packVersion")),
            detail: text(v, "detail").or_else(|| v.get("images").and_then(|i| text(i, "detail"))),
            exported_at: text(v, "exportedAt"),
            exported_by: Exporter { name: text(&by, "name"), version: text(&by, "version"), platform: text(&by, "platform") },
            created: text(&pack, "created"),
            updated: text(&pack, "updated"),
            source: Source {
                kind: text(&src, "kind"),
                name: text(&src, "name"),
                importer_id: text(&importer, "id"),
                importer_title: text(&importer, "title"),
                newest_page_at: text(&src, "newestPageAt"),
                oldest_page_at: text(&src, "oldestPageAt"),
                pages: src.get("pages").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            },
            contents: v.get("contents").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, x)| Some((k.clone(), num_of(x)?))).collect()).unwrap_or_default(),
            files: number(v, "files"),
            bytes: number(v, "bytes"),
            game: Game { title: text(&game, "title"), system: text(&game, "system") },
            personal_use: v.get("personalUse").and_then(Value::as_bool).unwrap_or(false),
            cover: text(v, "cover"),
            maps: list("maps")
                .iter()
                .filter(|m| m.is_object())
                .map(|m| MapInfo { id: text(m, "id"), name: text(m, "name"), width: number(m, "width"), height: number(m, "height"), locations: number(m, "locations"), preview: text(m, "preview") })
                .collect(),
            guides: list("guides")
                .iter()
                .filter(|g| g.is_object())
                .map(|g| GuideInfo { title: text(g, "title"), kind: text(g, "type"), pages: number(g, "pages") })
                .collect(),
            raw,
        }
    }

    /// The title to match a game by and name its folder by: `game.title`, else `title`.
    pub fn game_title(&self) -> Option<&str> {
        self.game.title.as_deref().or(self.title.as_deref())
    }

    /// The site's name: `source.name`, else `source.importer.title`.
    pub fn source_name(&self) -> Option<&str> {
        self.source.name.as_deref().or(self.source.importer_title.as_deref())
    }

    /// The name the contract gives a pack: `<game title> - <source name>.atlaspack`.
    pub fn file_name(&self) -> Option<String> {
        let t = self.game_title()?;
        Some(match self.source_name() {
            Some(s) => format!("{t} - {s}.{EXTENSION}"),
            None => format!("{t}.{EXTENSION}"),
        })
    }

    /// The small pictures it names (its cover and its maps'), each only once, and only safe
    /// `preview/<name>.jpg` names: the only names [`read_preview`] reads.
    pub fn previews(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for n in self.cover.iter().chain(self.maps.iter().filter_map(|m| m.preview.as_ref())) {
            if atlas_pack::is_preview_name(n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
        out
    }

    /// Which of two descriptions of a pack is the newer: the later `pack.updated`, then
    /// the later `exportedAt` (Equal: neither is).
    pub fn newer(&self, other: &Description) -> Ordering {
        later(self.updated.as_deref(), other.updated.as_deref()).then_with(|| later(self.exported_at.as_deref(), other.exported_at.as_deref()))
    }

    /// Who wrote it, for a sentence: "Atlas Importer 0.22.0".
    pub fn written_by(&self) -> Option<String> {
        let n = self.exported_by.name.as_deref()?;
        Some(match &self.exported_by.version {
            Some(v) => format!("{n} {v}"),
            None => n.to_string(),
        })
    }
}

/// An ISO 8601 date and time as seconds and nanoseconds since 1970 (UTC), for comparing.
fn instant(s: &str) -> Option<(i64, u32)> {
    let s = s.trim();
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse().ok()) };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return None;
    }
    let (mut h, mut mi, mut sec, mut nanos, mut at) = (0, 0, 0, 0u32, 10);
    if matches!(b.get(10), Some(b'T') | Some(b't') | Some(b' ')) {
        h = num(11..13)?;
        mi = num(14..16)?;
        at = 16;
        if b.get(16) == Some(&b':') {
            sec = num(17..19)?;
            at = 19;
        }
        if b.get(at) == Some(&b'.') {
            let digits: String = s[at + 1..].chars().take_while(char::is_ascii_digit).collect();
            at += 1 + digits.len();
            nanos = format!("{:0<9}", &digits[..digits.len().min(9)]).parse().unwrap_or(0);
        }
    }
    let offset = match s.get(at..) {
        None | Some("") | Some("Z") | Some("z") => 0,
        Some(o) if (o.starts_with('+') || o.starts_with('-')) && o.len() >= 3 => {
            let oh: i64 = o.get(1..3)?.parse().ok()?;
            let om: i64 = o.get(o.len() - 2..).filter(|_| o.len() >= 5)?.parse().unwrap_or(0);
            (oh * 3600 + om * 60) * if o.starts_with('-') { -1 } else { 1 }
        }
        _ => return None,
    };
    // days from the civil date (Howard Hinnant's days_from_civil)
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some((days * 86_400 + h * 3600 + mi * 60 + sec - offset, nanos))
}

/// Which of two dates is later; one that can't be read counts as earlier than one that can.
fn later(a: Option<&str>, b: Option<&str>) -> Ordering {
    match (a.and_then(instant), b.and_then(instant)) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => a.unwrap_or("").cmp(b.unwrap_or("")),
    }
}

/// Whether a file's name says it's a pack: `<name>.atlaspack`, not one still being
/// written (`.part`).
pub fn is_pack_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(&format!(".{EXTENSION}")) && !lower.starts_with('.')
}

/// What a card shows of a description, as JSON for the page.
pub fn card(d: &Description) -> Value {
    let mut v = serde_json::to_value(d).unwrap_or_default();
    if let Some(o) = v.as_object_mut() {
        o.insert("game_title".into(), d.game_title().map(Value::from).unwrap_or(Value::Null));
        o.insert("source_name".into(), d.source_name().map(Value::from).unwrap_or(Value::Null));
        o.insert("written_by".into(), d.written_by().map(Value::from).unwrap_or(Value::Null));
    }
    v
}

#[cfg(test)]
pub(crate) mod tests {
    //! Game Repo's side of reading packs: what a card gets from a description, the crate's
    //! sentences passed on as they are, and the newer of two files. Packs are made here as
    //! `docs/pack-file-format.md` in Atlas's repository says Atlas writes them (the
    //! description deflated first, the small pictures stored right behind it, then the
    //! rest); made-up games and sites only.
    use super::*;
    use serde_json::json;
    use std::fs::File;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    /// A small JPEG's first bytes and some more (enough to be told a JPEG).
    pub(crate) fn jpeg(seed: u8) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0];
        b.extend((0..200u32).map(|i| (i as u8).wrapping_mul(seed)));
        b.extend([0xFF, 0xD9]);
        b
    }

    /// A description of a made-up game and site with only what a pack from before block A3
    /// has (and a field from later, which a reader ignores).
    pub(crate) fn format6(id: &str, title: &str, updated: &str) -> Value {
        json!({
            "format": "atlas-pack-file", "version": 1, "packId": id, "title": title, "packVersion": "1.0.0", "detail": "full",
            "exportedAt": "2026-11-18T21:04:11Z",
            "exportedBy": { "name": "Atlas Importer", "version": "0.22.0", "platform": "windows" },
            "pack": { "id": id, "title": title, "packVersion": "1.0.0", "schemaVersion": 2, "attribution": [], "created": "2026-11-02T10:00:00Z", "updated": updated },
            "source": { "kind": "web-pages", "importer": { "id": "example-maps", "title": "Example Maps" },
                "pages": [{ "url": "https://maps.example/a-page", "mapId": "map:one", "importedAt": updated }],
                "newestPageAt": updated, "oldestPageAt": "2026-11-02T10:00:00Z" },
            "contents": { "maps": 2, "layers": 2, "locations": 1840, "categories": 46, "regions": 12, "checklists": 3, "guides": 210 },
            "images": { "detail": "full", "files": 21844, "bytes": 913000000, "layers": [] },
            "files": 21861, "bytes": 931000000,
            "aFieldFromLater": { "kept": true }
        })
    }

    /// Block A3's fields over [`format6`], as built: the game, the site's name, the maps with
    /// their small pictures (a third map with no image has none), one entry of pages (a
    /// wiki), and the cover, the first map's picture; version 7, as it carries pictures.
    pub(crate) fn format7(id: &str, game: &str, updated: &str) -> Value {
        let mut d = format6(id, game, updated);
        let o = d.as_object_mut().unwrap();
        o.insert("version".into(), json!(7));
        o.insert("game".into(), json!({ "title": game, "system": "Saltbox 64" }));
        o.insert("source".into(), json!({ "kind": "web-pages", "name": "Example Maps", "importer": { "id": "example-maps", "title": "Example Maps" }, "newestPageAt": updated }));
        o.insert("personalUse".into(), json!(true));
        o.insert("cover".into(), json!("preview/map-one.jpg"));
        o.insert("maps".into(), json!([
            { "id": "map:one", "name": "The Marsh", "width": 16384, "height": 16384, "locations": 1210, "preview": "preview/map-one.jpg" },
            { "id": "map:two", "name": "The Caves", "width": 8192, "height": 8192, "locations": 630, "preview": "preview/map-two.jpg" },
            { "id": "map:three", "name": "The Attic", "width": 1024, "height": 1024, "locations": 0 }
        ]));
        o.insert("guides".into(), json!([{ "title": "The Marsh Wiki", "type": "wiki", "pages": 210 }]));
        d
    }

    /// A pack as Atlas writes one: the description deflated first, the small pictures stored
    /// right behind it, then a JSON file and an image.
    pub(crate) fn write_pack(path: &Path, description: &Value, previews: &[(&str, Vec<u8>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        z.start_file("atlaspack.json", deflated).unwrap();
        z.write_all(serde_json::to_string_pretty(description).unwrap().as_bytes()).unwrap();
        for (n, b) in previews {
            z.start_file(*n, stored).unwrap();
            z.write_all(b).unwrap();
        }
        z.start_file("manifest.json", deflated).unwrap();
        z.write_all(b"{}").unwrap();
        z.start_file("images/tile-0-0.png", stored).unwrap();
        z.write_all(&[0u8; 300]).unwrap();
        z.finish().unwrap();
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("gamerepo-test-atlaspack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_pack_from_before_block_a3_falls_back_to_its_title_site_and_counts() {
        let d = dir("six");
        let p = d.join("Hollow Bog - Example Maps.atlaspack");
        write_pack(&p, &format6("hollow-bog-example-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[]);
        let x = read_header(&p).unwrap();
        assert_eq!((x.version, x.pack_id.as_deref(), x.title.as_deref()), (1, Some("hollow-bog-example-maps"), Some("Hollow Bog")));
        assert_eq!((x.game_title(), x.source_name(), x.detail.as_deref()), (Some("Hollow Bog"), Some("Example Maps"), Some("full")));
        assert_eq!((x.contents.get("maps"), x.contents.get("guides"), x.source.pages), (Some(&2), Some(&210), 1));
        assert_eq!((x.updated.as_deref(), x.written_by().as_deref()), (Some("2026-11-17T08:30:00Z"), Some("Atlas Importer 0.22.0")));
        assert!(x.maps.is_empty() && x.cover.is_none() && !x.personal_use && x.previews().is_empty());
        assert_eq!(x.file_name().as_deref(), Some("Hollow Bog - Example Maps.atlaspack"));
        assert!(read_preview(&p, "preview/map-one.jpg").is_err(), "no picture it doesn't list");
        // what the index keeps reads back the same
        assert_eq!(Description::from_value(x.raw.clone()), x);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn block_a3s_names_and_pictures_and_a_lower_version_without_pictures() {
        let d = dir("seven");
        let p = d.join("pack.atlaspack");
        let (one, two) = (jpeg(3), jpeg(5));
        write_pack(&p, &format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", one.clone()), ("preview/map-two.jpg", two.clone())]);
        let x = read_header(&p).unwrap();
        assert_eq!((x.version, x.game.system.as_deref(), x.source_name(), x.personal_use), (7, Some("Saltbox 64"), Some("Example Maps"), true));
        assert_eq!(x.maps.iter().map(|m| (m.name.clone().unwrap(), m.locations.unwrap())).collect::<Vec<_>>(), [("The Marsh".to_string(), 1210), ("The Caves".to_string(), 630), ("The Attic".to_string(), 0)]);
        assert_eq!(x.guides.iter().map(|g| (g.kind.clone().unwrap(), g.pages.unwrap())).collect::<Vec<_>>(), [("wiki".to_string(), 210)]);
        assert_eq!(x.previews(), ["preview/map-one.jpg", "preview/map-two.jpg"]);
        assert_eq!(read_preview(&p, "preview/map-one.jpg").unwrap(), one);
        assert_eq!(read_preview(&p, "preview/map-two.jpg").unwrap(), two);
        // only the names the description lists, and only safe ones
        assert!(read_preview(&p, "manifest.json").is_err());
        assert!(read_preview(&p, "preview/../manifest.json").is_err());
        // a picture that isn't a JPEG, or is over 512 KB, isn't read: the crate's sentence
        let q = d.join("png.atlaspack");
        let mut big = jpeg(7);
        big.resize(513 << 10, 0);
        write_pack(&q, &format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", b"\x89PNG not a jpeg".to_vec()), ("preview/map-two.jpg", big)]);
        assert_eq!(read_preview(&q, "preview/map-one.jpg").unwrap_err(), "The small picture in this pack file is not a JPEG picture.");
        assert!(read_preview(&q, "preview/map-two.jpg").is_err());
        // a pack whose maps have no picture keeps a lower version, with the other new fields
        let mut six = format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z");
        six["version"] = json!(6);
        six.as_object_mut().unwrap().remove("cover");
        for m in six["maps"].as_array_mut().unwrap() {
            m.as_object_mut().unwrap().remove("preview");
        }
        write_pack(&q, &six, &[]);
        let y = read_header(&q).unwrap();
        assert_eq!((y.version, y.game_title(), y.source_name(), y.maps.len(), y.guides.len()), (6, Some("Hollow Bog"), Some("Example Maps"), 3, 1));
        assert!(y.previews().is_empty() && y.cover.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_guide_of_a_type_not_known_and_a_name_that_isnt_safe_are_taken_as_the_crate_takes_them() {
        let d = dir("unknown");
        let p = d.join("pack.atlaspack");
        let mut v = format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z");
        v["guides"] = json!([{ "title": "The Marsh Atlas", "type": "atlas-of-the-future", "pages": 12 }]);
        v["maps"][1]["preview"] = json!("preview/../../evil.jpg");
        write_pack(&p, &v, &[("preview/map-one.jpg", jpeg(3))]);
        let x = read_header(&p).unwrap();
        assert_eq!((x.guides[0].kind.as_deref(), x.guides[0].pages), (Some("other"), Some(12)), "a type it doesn't know is accepted");
        assert_eq!((x.maps[1].preview.as_deref(), x.previews()), (None, vec!["preview/map-one.jpg".to_string()]), "an unsafe name is left out");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_isnt_a_pack_or_is_too_new_gets_the_crates_sentence() {
        let d = dir("wrong");
        let text = d.join("notes.atlaspack");
        std::fs::write(&text, b"just some text").unwrap();
        assert_eq!(read_header(&text).unwrap_err(), "This file is not an Atlas pack file.");
        std::fs::write(&text, b"").unwrap();
        assert_eq!(read_header(&text).unwrap_err(), "This file is not an Atlas pack file.");
        // another format
        let other = d.join("other.atlaspack");
        let mut wrong = format6("x", "X", "2026-01-01T00:00:00Z");
        wrong["format"] = json!("something-else");
        write_pack(&other, &wrong, &[]);
        assert_eq!(read_header(&other).unwrap_err(), "This file is not an Atlas pack file.");
        // version 8 is newer than the crate reads: the sentence names what wrote it
        let mut eight = format7("x", "X", "2026-01-01T00:00:00Z");
        eight["version"] = json!(8);
        write_pack(&other, &eight, &[]);
        assert_eq!(
            read_header(&other).unwrap_err(),
            "This pack file was made by Atlas Importer 0.22.0 in a newer format (version 8). Install a newer version of Atlas to open it."
        );
        // a description that's missing what every pack has
        let mut damaged = format6("x", "X", "2026-01-01T00:00:00Z");
        damaged.as_object_mut().unwrap().remove("images");
        write_pack(&other, &damaged, &[]);
        assert!(read_header(&other).unwrap_err().starts_with("This pack file's description is damaged:"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_newer_of_two_files_of_a_pack_is_told_by_its_dates() {
        let a = Description::from_value(format6("p", "P", "2026-11-17T08:30:00Z"));
        let mut b = Description::from_value(format6("p", "P", "2026-11-17T09:30:00+02:00"));
        assert_eq!(a.newer(&b), Ordering::Greater, "08:30 UTC is after 07:30 UTC");
        b.updated = a.updated.clone();
        b.exported_at = Some("2026-11-19T00:00:00.5Z".into());
        assert_eq!(b.newer(&a), Ordering::Greater, "exportedAt decides a tie");
        b.exported_at = a.exported_at.clone();
        assert_eq!(a.newer(&b), Ordering::Equal);
        assert!(is_pack_name("A - B.atlaspack") && is_pack_name("x.ATLASPACK") && !is_pack_name("A - B.atlaspack.part") && !is_pack_name("x.part"));
    }
}
