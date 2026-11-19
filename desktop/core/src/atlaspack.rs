//! Atlas pack files (`.atlaspack`, docs/PLAN.md, D215): what Game Repo reads of a pack
//! Atlas Importer wrote, by the *Atlas pack contract for Game Repo*.
//!
//! This is a stand-in for Atlas's own `atlas-pack` crate (the contract's block A3), with
//! the surface the crate will have: [`FORMAT`], [`NEWEST`], [`EXTENSION`],
//! [`read_description`] and [`read_preview`], and a plain sentence for each way a file is
//! wrong ([`PackError`]). It's to be replaced by that crate, as a git dependency pinned to
//! an Atlas release tag, once Atlas tags one; the crate's `docs/pack-file-format.md` is the
//! reference for names then.
//!
//! A pack is a ZIP (ZIP64 when it needs to be) whose first entry is always `atlaspack.json`,
//! the description; format 7 puts a small JPEG for each map right behind it, under
//! `preview/`, stored as it is. Only the front of the file is read: the description from
//! the first local header (stored or deflated, at most 4 MB unpacked), a picture by
//! walking the local headers behind it, and only when a header can't say how long its
//! entry is, the archive's central directory. Nothing in a pack is ever unpacked to disk by
//! the names inside it, and no address in it is ever fetched. Every field is optional and
//! unknown fields are kept (`raw`), so a renamed field costs one line here.

use flate2::bufread::DeflateDecoder;
use serde::Serialize;
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// What `format` always says in a pack's description.
pub const FORMAT: &str = "atlas-pack-file";
/// The newest format this reader knows; a pack that needs a newer one is refused (D215).
pub const NEWEST: u64 = 7;
/// A pack file's extension.
pub const EXTENSION: &str = "atlaspack";
/// The first entry of every pack.
pub const DESCRIPTION: &str = "atlaspack.json";
/// A description larger than this, unpacked, is refused.
pub const MAX_DESCRIPTION: u64 = 4 << 20;
/// A small picture larger than this is refused.
pub const MAX_PREVIEW: u64 = 4 << 20;
/// Where the small pictures are in a pack.
pub const PREVIEWS: &str = "preview/";

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;

/// Why a file can't be read as a pack: a plain sentence each, shown as it is (D215).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackError {
    /// Not a ZIP, its first entry isn't `atlaspack.json`, or `format` isn't `atlas-pack-file`.
    NotAPack,
    /// A format newer than [`NEWEST`]: the version, and the app (with its version) that wrote it.
    Newer { version: u64, by: Option<String> },
    /// The description can't be read: what's wrong.
    Damaged(String),
}

impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::NotAPack => write!(f, "This isn't a pack file."),
            PackError::Newer { version, by: Some(by) } => write!(f, "This pack needs a newer Game Repo: it's format {version}, written by {by}; this Game Repo reads up to format {NEWEST}."),
            PackError::Newer { version, by: None } => write!(f, "This pack needs a newer Game Repo: it's format {version}; this Game Repo reads up to format {NEWEST}."),
            PackError::Damaged(what) => write!(f, "This pack's description is damaged: {what}."),
        }
    }
}

impl std::error::Error for PackError {}

fn damaged(what: impl Into<String>) -> PackError {
    PackError::Damaged(what.into())
}

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
    /// The site's name as Atlas Importer shows it (format 7)
    pub name: Option<String>,
    pub importer_id: Option<String>,
    pub importer_title: Option<String>,
    pub newest_page_at: Option<String>,
    pub oldest_page_at: Option<String>,
    pub pages: usize,
}

/// The game a pack is for (`game`, format 7).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Game {
    pub title: Option<String>,
    /// As the site named it: a hint for matching, never proof
    pub system: Option<String>,
}

/// A map in a pack (`maps[]`, format 7).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MapInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub locations: Option<u64>,
    /// Its small picture's name in the pack, under `preview/`
    pub preview: Option<String>,
}

/// A guide or a wiki in a pack (`guides[]`, format 7).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GuideInfo {
    pub title: Option<String>,
    /// guide | wiki
    pub kind: Option<String>,
    pub pages: Option<u64>,
}

/// A pack's description (`atlaspack.json`): every field optional, every field kept in `raw`.
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
    /// The description in a pack's `atlaspack.json` (already checked by [`read_description`],
    /// or kept from it): what isn't there, or isn't what it should be, is left out.
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

    /// The small pictures it names (its cover and its maps'), each only once, and only those
    /// under `preview/` with a plain name: the only names [`read_preview`] reads.
    pub fn previews(&self) -> Vec<String> {
        let mut out: Vec<String> = vec![];
        for n in self.cover.iter().chain(self.maps.iter().filter_map(|m| m.preview.as_ref())) {
            if preview_name_ok(n) && !out.contains(n) {
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

/// A picture's name as a description gives it: under `preview/`, nothing but plain parts.
fn preview_name_ok(n: &str) -> bool {
    n.starts_with(PREVIEWS) && n.len() > PREVIEWS.len() && !n.contains('\\') && n.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
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

// ------------------------------------------------------------ reading the front of a ZIP

/// A local file header, as far as it says.
struct Local {
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    /// Its sizes; none when a data descriptor after the entry gives them
    compressed: Option<u64>,
    size: Option<u64>,
    /// Where its data starts
    data: u64,
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or([0; 8]))
}

/// The local header at `at`, or None when there's none there (the central directory, the end).
fn local_at(f: &mut BufReader<File>, at: u64) -> std::io::Result<Option<Local>> {
    f.seek(SeekFrom::Start(at))?;
    let mut h = [0u8; 30];
    match f.read_exact(&mut h) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if u32_at(&h, 0) != LOCAL {
        return Ok(None);
    }
    let (flags, method, crc) = (u16_at(&h, 6), u16_at(&h, 8), u32_at(&h, 14));
    let (mut compressed, mut size) = (u32_at(&h, 18) as u64, u32_at(&h, 22) as u64);
    let (name_len, extra_len) = (u16_at(&h, 26) as usize, u16_at(&h, 28) as usize);
    let mut name = vec![0u8; name_len];
    f.read_exact(&mut name)?;
    let mut extra = vec![0u8; extra_len];
    f.read_exact(&mut extra)?;
    // ZIP64: the sizes that don't fit in the header are in its extra field 0x0001, in order
    if size == 0xFFFF_FFFF || compressed == 0xFFFF_FFFF {
        let mut i = 0;
        while i + 4 <= extra.len() {
            let (id, len) = (u16_at(&extra, i), u16_at(&extra, i + 2) as usize);
            let body = &extra[(i + 4).min(extra.len())..(i + 4 + len).min(extra.len())];
            if id == 0x0001 {
                let mut j = 0;
                if size == 0xFFFF_FFFF && body.len() >= j + 8 {
                    size = u64_at(body, j);
                    j += 8;
                }
                if compressed == 0xFFFF_FFFF && body.len() >= j + 8 {
                    compressed = u64_at(body, j);
                }
                break;
            }
            i += 4 + len;
        }
    }
    let described = flags & 0x0008 != 0 && compressed == 0;
    Ok(Some(Local {
        name: String::from_utf8_lossy(&name).into_owned(),
        flags,
        method,
        crc,
        compressed: (!described && compressed != 0xFFFF_FFFF).then_some(compressed),
        size: (!described && size != 0xFFFF_FFFF).then_some(size),
        data: at + 30 + name_len as u64 + extra_len as u64,
    }))
}

/// An entry's bytes read from where its data starts: at most `max` unpacked (more is
/// `Err(None)`), checked by its CRC32. Also returns where the entry ends (after its data
/// descriptor, if it has one). `Ok(None)`: its length can't be told from here (a stored
/// entry with a data descriptor), so the central directory is needed.
#[allow(clippy::type_complexity)]
fn entry_bytes(f: &mut BufReader<File>, e: &Local, max: u64) -> Result<Option<(Vec<u8>, u64)>, Option<String>> {
    let io = |x: std::io::Error| Some(format!("it couldn't be read ({x})"));
    if e.flags & 0x0001 != 0 {
        return Err(Some("it's encrypted".into()));
    }
    if e.size.is_some_and(|n| n > max) {
        return Err(None);
    }
    f.seek(SeekFrom::Start(e.data)).map_err(io)?;
    let (bytes, end, crc) = match e.method {
        0 => {
            let (Some(n), Some(c)) = (e.size, e.compressed) else { return Ok(None) };
            if n != c {
                return Err(Some("a stored entry's sizes disagree".into()));
            }
            let mut b = vec![0u8; n as usize];
            f.read_exact(&mut b).map_err(io)?;
            (b, e.data + n, Some(e.crc))
        }
        8 => {
            let mut b = vec![];
            let mut d = DeflateDecoder::new(f.by_ref().take(e.compressed.unwrap_or(u64::MAX)));
            d.by_ref().take(max + 1).read_to_end(&mut b).map_err(|x| Some(format!("its compressed data is broken ({x})")))?;
            if b.len() as u64 > max {
                return Err(None);
            }
            let used = d.total_in();
            let mut end = e.data + used;
            let mut crc = Some(e.crc);
            if e.compressed.is_none() {
                // a data descriptor right behind it: its CRC32 (with or without its signature)
                f.seek(SeekFrom::Start(end)).map_err(io)?;
                let mut x = [0u8; 8];
                if f.read_exact(&mut x).is_ok() {
                    let (c, skip) = if u32_at(&x, 0) == DESCRIPTOR { (u32_at(&x, 4), 4) } else { (u32_at(&x, 0), 0) };
                    crc = Some(c);
                    // the sizes after it: 8 bytes, or 16 for ZIP64; told by what follows
                    let base = end + skip + 4;
                    end = [base + 8, base + 16]
                        .into_iter()
                        .find(|&at| {
                            let mut s = [0u8; 4];
                            f.seek(SeekFrom::Start(at)).is_ok() && f.read_exact(&mut s).is_ok() && matches!(u32_at(&s, 0), LOCAL | CENTRAL)
                        })
                        .unwrap_or(base + 8);
                } else {
                    crc = None;
                }
            } else if let Some(c) = e.compressed {
                end = e.data + c;
            }
            if let Some(n) = e.size {
                if n != b.len() as u64 {
                    return Err(Some("its size isn't what its header says".into()));
                }
            }
            (b, end, crc)
        }
        m => return Err(Some(format!("it's packed a way packs never are (method {m})"))),
    };
    if let Some(c) = crc {
        if crc32fast::hash(&bytes) != c {
            return Err(Some("its check (CRC32) doesn't match".into()));
        }
    }
    Ok(Some((bytes, end)))
}

/// An entry read through the archive's central directory (`first`: it must be the first
/// entry, by its place in the archive), at most `max` unpacked.
fn by_directory(path: &Path, name: &str, first: bool, max: u64) -> Result<Vec<u8>, PackError> {
    let f = File::open(path).map_err(|e| damaged(format!("it couldn't be read ({e})")))?;
    let mut z = zip::ZipArchive::new(BufReader::new(f)).map_err(|_| damaged("its list of entries can't be read"))?;
    let at = z.index_for_name(name).ok_or_else(|| if first { PackError::NotAPack } else { damaged(format!("it has no picture {name}")) })?;
    if first {
        let offset = z.by_index_raw(at).map(|e| e.header_start()).map_err(|_| damaged("its first entry can't be read"))?;
        let earliest = (0..z.len()).filter_map(|i| z.by_index_raw(i).ok().map(|e| e.header_start())).min().unwrap_or(0);
        if offset != earliest {
            return Err(PackError::NotAPack);
        }
    }
    let mut e = z.by_index(at).map_err(|_| damaged(format!("{name} can't be read")))?;
    if e.size() > max {
        return Err(too_big(name));
    }
    let mut b = vec![];
    e.by_ref().take(max + 1).read_to_end(&mut b).map_err(|x| damaged(format!("{name} can't be read ({x})")))?;
    if b.len() as u64 > max {
        return Err(too_big(name));
    }
    Ok(b)
}

fn too_big(name: &str) -> PackError {
    if name == DESCRIPTION {
        damaged("it's larger than 4 MB")
    } else {
        damaged(format!("its picture {name} is larger than 4 MB"))
    }
}

/// The description, and where the entries behind it start (none: not known from the front).
fn front(path: &Path) -> Result<(Description, Option<u64>), PackError> {
    let file = File::open(path).map_err(|e| damaged(format!("it couldn't be read ({e})")))?;
    let mut f = BufReader::with_capacity(64 << 10, file);
    let first = match local_at(&mut f, 0) {
        Ok(Some(l)) => l,
        // not a ZIP, or an empty one
        Ok(None) => return Err(PackError::NotAPack),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(PackError::NotAPack),
        Err(e) => return Err(damaged(format!("it couldn't be read ({e})"))),
    };
    if first.name != DESCRIPTION {
        return Err(PackError::NotAPack);
    }
    let (bytes, next) = match entry_bytes(&mut f, &first, MAX_DESCRIPTION) {
        Ok(Some((b, end))) => (b, Some(end)),
        Ok(None) => (by_directory(path, DESCRIPTION, true, MAX_DESCRIPTION)?, None),
        Err(None) => return Err(too_big(DESCRIPTION)),
        Err(Some(what)) => return Err(damaged(what)),
    };
    let raw: Value = serde_json::from_slice(&bytes).map_err(|e| damaged(format!("it isn't JSON that can be read ({e})")))?;
    if !raw.is_object() {
        return Err(damaged("it isn't a JSON object"));
    }
    if raw.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(PackError::NotAPack);
    }
    if raw.get("version").is_some_and(|v| !v.is_null() && number(&raw, "version").is_none()) {
        return Err(damaged("its version isn't a number"));
    }
    let d = Description::from_value(raw);
    if d.version > NEWEST {
        return Err(PackError::Newer { version: d.version, by: d.written_by() });
    }
    Ok((d, next))
}

/// Read a pack's description from the front of its file only: the first entry must be
/// `atlaspack.json` (stored or deflated, at most 4 MB unpacked), `format` must be
/// `atlas-pack-file`, and `version` (absent: 1) at most [`NEWEST`].
pub fn read_description(path: &Path) -> Result<Description, PackError> {
    front(path).map(|(d, _)| d)
}

/// One small picture of a pack (its cover or a map's), by its name in the description:
/// only names the description gives, under `preview/`, at most 4 MB, and a JPEG. Found
/// by walking the entries right behind the description, else through the archive's
/// central directory.
pub fn read_preview(path: &Path, name: &str) -> Result<Vec<u8>, PackError> {
    let (d, next) = front(path)?;
    if !d.previews().iter().any(|p| p == name) {
        return Err(damaged(format!("it names no picture {name}")));
    }
    let jpeg = |b: Vec<u8>| -> Result<Vec<u8>, PackError> {
        if b.len() >= 3 && b[..3] == [0xFF, 0xD8, 0xFF] {
            Ok(b)
        } else {
            Err(damaged(format!("its picture {name} isn't a JPEG")))
        }
    };
    if let Some(mut at) = next {
        let file = File::open(path).map_err(|e| damaged(format!("it couldn't be read ({e})")))?;
        let mut f = BufReader::with_capacity(64 << 10, file);
        // the pictures are right behind the description: walk them, and no further
        for _ in 0..4096 {
            let Ok(Some(e)) = local_at(&mut f, at) else { break };
            if !e.name.starts_with(PREVIEWS) {
                break;
            }
            if e.name == name {
                return match entry_bytes(&mut f, &e, MAX_PREVIEW) {
                    Ok(Some((b, _))) => jpeg(b),
                    Ok(None) => jpeg(by_directory(path, name, false, MAX_PREVIEW)?),
                    Err(None) => Err(too_big(name)),
                    Err(Some(what)) => Err(damaged(format!("its picture {name} can't be read: {what}"))),
                };
            }
            match e.compressed {
                Some(c) => at = e.data + c,
                None => break,
            }
        }
    }
    jpeg(by_directory(path, name, false, MAX_PREVIEW)?)
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

/// The fields of a description kept beside a card, by name (what isn't a known field).
pub fn unknown_fields(d: &Description) -> Map<String, Value> {
    const KNOWN: &[&str] = &[
        "format", "version", "packId", "title", "packVersion", "detail", "exportedAt", "exportedBy", "pack", "source", "contents", "images", "files", "bytes", "game",
        "personalUse", "cover", "maps", "guides",
    ];
    d.raw.as_object().map(|m| m.iter().filter(|(k, _)| !KNOWN.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
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

    /// A format 6 description of a made-up game and site.
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

    /// Format 7's additions over a format 6 description: the game, the site's name, the maps
    /// with their pictures, the guides.
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
            { "id": "map:two", "name": "The Caves", "width": 8192, "height": 8192, "locations": 630, "preview": "preview/map-two.jpg" }
        ]));
        o.insert("guides".into(), json!([{ "title": "Walkthrough", "type": "guide", "pages": 64 }, { "title": "The Wiki", "type": "wiki", "pages": 146 }]));
        d
    }

    /// A pack as Atlas Importer writes one: the description deflated first, then the
    /// pictures stored, then a JSON file and an image.
    pub(crate) fn write_pack(path: &Path, description: &Value, previews: &[(&str, Vec<u8>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(File::create(path).unwrap());
        let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        z.start_file(DESCRIPTION, deflated).unwrap();
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

    /// A ZIP written by hand with its first entry deflated and followed by a data
    /// descriptor (its sizes zero in the header), as a writer that can't seek leaves one.
    fn streamed(path: &Path, name: &str, body: &[u8], then: &[(&str, &[u8])]) {
        let mut enc = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        enc.write_all(body).unwrap();
        let packed = enc.finish().unwrap();
        let mut out = vec![];
        let mut central = vec![];
        let mut entry = |out: &mut Vec<u8>, name: &str, data: &[u8], method: u16, flags: u16, crc: u32, size: u32| {
            let at = out.len() as u32;
            out.extend(LOCAL.to_le_bytes());
            out.extend(20u16.to_le_bytes());
            out.extend(flags.to_le_bytes());
            out.extend(method.to_le_bytes());
            out.extend([0u8; 4]);
            let (c, cs, us) = if flags & 8 != 0 { (0, 0, 0) } else { (crc, data.len() as u32, size) };
            out.extend(c.to_le_bytes());
            out.extend(cs.to_le_bytes());
            out.extend(us.to_le_bytes());
            out.extend((name.len() as u16).to_le_bytes());
            out.extend(0u16.to_le_bytes());
            out.extend(name.as_bytes());
            out.extend(data);
            if flags & 8 != 0 {
                out.extend(DESCRIPTOR.to_le_bytes());
                out.extend(crc.to_le_bytes());
                out.extend((data.len() as u32).to_le_bytes());
                out.extend(size.to_le_bytes());
            }
            central.extend(CENTRAL.to_le_bytes());
            central.extend(20u16.to_le_bytes());
            central.extend(20u16.to_le_bytes());
            central.extend(flags.to_le_bytes());
            central.extend(method.to_le_bytes());
            central.extend([0u8; 4]);
            central.extend(crc.to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend(size.to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(at.to_le_bytes());
            central.extend(name.as_bytes());
        };
        entry(&mut out, name, &packed, 8, 8, crc32fast::hash(body), body.len() as u32);
        for (n, b) in then {
            entry(&mut out, n, b, 0, 0, crc32fast::hash(b), b.len() as u32);
        }
        let n = 1 + then.len() as u16;
        let cd_at = out.len() as u32;
        out.extend(&central);
        out.extend(0x0605_4b50u32.to_le_bytes());
        out.extend([0u8; 4]);
        out.extend(n.to_le_bytes());
        out.extend(n.to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_at.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn a_format_6_pack_is_read_from_its_front() {
        let d = dir("six");
        let p = d.join("Hollow Bog - Example Maps.atlaspack");
        write_pack(&p, &format6("hollow-bog-example-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[]);
        let x = read_description(&p).unwrap();
        assert_eq!((x.version, x.pack_id.as_deref(), x.title.as_deref()), (1, Some("hollow-bog-example-maps"), Some("Hollow Bog")));
        assert_eq!((x.game_title(), x.source_name(), x.detail.as_deref()), (Some("Hollow Bog"), Some("Example Maps"), Some("full")));
        assert_eq!((x.contents.get("maps"), x.contents.get("guides"), x.source.pages), (Some(&2), Some(&210), 1));
        assert_eq!((x.updated.as_deref(), x.written_by().as_deref()), (Some("2026-11-17T08:30:00Z"), Some("Atlas Importer 0.22.0")));
        assert!(x.maps.is_empty() && x.cover.is_none() && !x.personal_use && x.previews().is_empty());
        assert_eq!(x.file_name().as_deref(), Some("Hollow Bog - Example Maps.atlaspack"));
        assert_eq!(unknown_fields(&x).get("aFieldFromLater"), Some(&json!({ "kept": true })), "a field it doesn't know is kept");
        assert!(read_preview(&p, "preview/map-one.jpg").is_err(), "no picture it doesn't name");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_format_7_pack_gives_its_names_and_pictures() {
        let d = dir("seven");
        let p = d.join("pack.atlaspack");
        let (one, two) = (jpeg(3), jpeg(5));
        write_pack(&p, &format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", one.clone()), ("preview/map-two.jpg", two.clone())]);
        let x = read_description(&p).unwrap();
        assert_eq!((x.version, x.game.system.as_deref(), x.source_name(), x.personal_use), (7, Some("Saltbox 64"), Some("Example Maps"), true));
        assert_eq!(x.maps.iter().map(|m| (m.name.clone().unwrap(), m.locations.unwrap())).collect::<Vec<_>>(), [("The Marsh".to_string(), 1210), ("The Caves".to_string(), 630)]);
        assert_eq!(x.guides.iter().map(|g| (g.kind.clone().unwrap(), g.pages.unwrap())).collect::<Vec<_>>(), [("guide".to_string(), 64), ("wiki".to_string(), 146)]);
        assert_eq!(x.previews(), ["preview/map-one.jpg", "preview/map-two.jpg"]);
        assert_eq!(read_preview(&p, "preview/map-one.jpg").unwrap(), one);
        assert_eq!(read_preview(&p, "preview/map-two.jpg").unwrap(), two);
        // only the names the description gives
        assert!(read_preview(&p, "manifest.json").is_err());
        assert!(read_preview(&p, "preview/../manifest.json").is_err());
        // a picture that isn't a JPEG is refused
        let q = d.join("png.atlaspack");
        write_pack(&q, &format7("marsh-maps", "Hollow Bog", "2026-11-17T08:30:00Z"), &[("preview/map-one.jpg", b"\x89PNG not a jpeg".to_vec())]);
        assert!(read_preview(&q, "preview/map-one.jpg").unwrap_err().to_string().contains("isn't a JPEG"));
        // named but not there
        assert!(read_preview(&q, "preview/map-two.jpg").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_isnt_a_pack_or_is_too_new_gets_its_sentence() {
        let d = dir("wrong");
        let text = d.join("notes.atlaspack");
        std::fs::write(&text, b"just some text").unwrap();
        assert_eq!(read_description(&text).unwrap_err().to_string(), "This isn't a pack file.");
        std::fs::write(&text, b"").unwrap();
        assert_eq!(read_description(&text).unwrap_err(), PackError::NotAPack);
        // a ZIP whose first entry isn't the description
        let other = d.join("other.atlaspack");
        let mut z = zip::ZipWriter::new(File::create(&other).unwrap());
        z.start_file("readme.txt", SimpleFileOptions::default()).unwrap();
        z.write_all(b"hello").unwrap();
        z.start_file(DESCRIPTION, SimpleFileOptions::default()).unwrap();
        z.write_all(serde_json::to_string(&format6("x", "X", "2026-01-01T00:00:00Z")).unwrap().as_bytes()).unwrap();
        z.finish().unwrap();
        assert_eq!(read_description(&other).unwrap_err(), PackError::NotAPack);
        // another format
        let mut wrong = format6("x", "X", "2026-01-01T00:00:00Z");
        wrong["format"] = json!("something-else");
        write_pack(&other, &wrong, &[]);
        assert_eq!(read_description(&other).unwrap_err(), PackError::NotAPack);
        // version 8 is newer than this reader
        let mut eight = format7("x", "X", "2026-01-01T00:00:00Z");
        eight["version"] = json!(8);
        write_pack(&other, &eight, &[]);
        assert_eq!(
            read_description(&other).unwrap_err().to_string(),
            "This pack needs a newer Game Repo: it's format 8, written by Atlas Importer 0.22.0; this Game Repo reads up to format 7."
        );
        // a description that isn't JSON
        let mut z = zip::ZipWriter::new(File::create(&other).unwrap());
        z.start_file(DESCRIPTION, SimpleFileOptions::default()).unwrap();
        z.write_all(b"{ not json").unwrap();
        z.finish().unwrap();
        assert!(read_description(&other).unwrap_err().to_string().starts_with("This pack's description is damaged: it isn't JSON"));
        // version absent reads as 1
        let mut none = format6("y", "Y", "2026-01-01T00:00:00Z");
        none.as_object_mut().unwrap().remove("version");
        write_pack(&other, &none, &[]);
        assert_eq!(read_description(&other).unwrap().version, 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_description_over_4_mb_is_refused() {
        let d = dir("big");
        let p = d.join("big.atlaspack");
        let mut v = format6("big", "Big", "2026-01-01T00:00:00Z");
        v["padding"] = json!("x".repeat((MAX_DESCRIPTION + 10) as usize));
        write_pack(&p, &v, &[]);
        assert_eq!(read_description(&p).unwrap_err().to_string(), "This pack's description is damaged: it's larger than 4 MB.");
        // stored the same, too
        let mut z = zip::ZipWriter::new(File::create(&p).unwrap());
        z.start_file(DESCRIPTION, SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        z.write_all(serde_json::to_string(&v).unwrap().as_bytes()).unwrap();
        z.finish().unwrap();
        assert_eq!(read_description(&p).unwrap_err(), damaged("it's larger than 4 MB"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_first_entry_with_a_data_descriptor_is_read() {
        let d = dir("described");
        let p = d.join("streamed.atlaspack");
        let body = serde_json::to_vec(&format7("streamed", "Hollow Bog", "2026-11-17T08:30:00Z")).unwrap();
        let pic = jpeg(9);
        streamed(&p, DESCRIPTION, &body, &[("preview/map-one.jpg", &pic), ("manifest.json", b"{}")]);
        let x = read_description(&p).unwrap();
        assert_eq!(x.pack_id.as_deref(), Some("streamed"));
        assert_eq!(read_preview(&p, "preview/map-one.jpg").unwrap(), pic, "found behind the descriptor");
        // the CRC32 is checked: one byte of the description changed
        let mut bad = std::fs::read(&p).unwrap();
        let mut enc = flate2::write::DeflateEncoder::new(vec![], flate2::Compression::default());
        enc.write_all(&body).unwrap();
        let packed = enc.finish().unwrap();
        let at = 30 + DESCRIPTION.len() + packed.len() + 4;
        bad[at] ^= 0xFF;
        std::fs::write(&p, bad).unwrap();
        assert!(read_description(&p).unwrap_err().to_string().contains("CRC32"));
        // the zip crate's own streamed writer (data descriptors for every entry)
        let q = d.join("zipstream.atlaspack");
        let mut z = zip::ZipWriter::new_stream(File::create(&q).unwrap());
        z.start_file(DESCRIPTION, SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)).unwrap();
        z.write_all(&body).unwrap();
        z.start_file("preview/map-one.jpg", SimpleFileOptions::default().compression_method(CompressionMethod::Stored)).unwrap();
        z.write_all(&pic).unwrap();
        z.finish().unwrap();
        assert_eq!(read_description(&q).unwrap().pack_id.as_deref(), Some("streamed"));
        assert_eq!(read_preview(&q, "preview/map-one.jpg").unwrap(), pic);
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
