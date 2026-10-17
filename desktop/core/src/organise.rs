//! Organising a platform's set by its DAT (docs/PLAN.md, "Phase 2 design", 2a): the
//! three modes of D5 (copy to a folder, rename where they are, move into the
//! library), each planned first from the index alone, previewed, then run as a job
//! with every written file read back and checked before its original is set aside.
//! Every change is journalled in `_library/journal/` (Model Library's pattern), so it
//! can be undone, and one interrupted by a crash can be finished or put back.
//!
//! Originals a change replaces are never deleted (D20): they're moved into
//! `.gamerepo-set-aside/<change>/` at the top of their folder with games (the same
//! drive, so it's a rename; hidden, so scans skip it) until emptied.

use crate::archive;
use crate::audit;
use crate::cue;
use crate::hash::{self, Hashes};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::platforms;
use crate::roots::{self, Root};
use crate::torrentzip;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The folder originals are set aside in, at the top of each folder with games.
pub const SET_ASIDE: &str = ".gamerepo-set-aside";
/// Where files are put together before they're moved into place.
pub(crate) const STAGING: &str = ".gamerepo-tmp";
/// Finished changes kept in the history (those still holding set-aside files are kept until emptied).
const KEEP: usize = 20;

// ------------------------------------------------------------ what a change is

/// Where an operation's bytes come from: a file in a folder with games, or a member of an archive there.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Src {
    /// The index row: the file, or the archive for a member
    pub file_id: i64,
    pub root: String,
    /// The file's (or archive's) path relative to its folder with games, with `/`
    pub rel: String,
    /// Its path on this computer when planned
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    pub size: u64,
    pub crc32: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
}

/// One step of a change.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Op {
    /// rename | move | copy | link | zip | extract | aside
    pub op: String,
    pub game: String,
    /// Where it goes (for `aside`, the source's own path)
    pub to: String,
    pub from: Vec<Src>,
    /// For `zip`: each source's name in the ZIP
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    /// Sources set aside once this step is done and checked
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aside: Vec<Src>,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A planned change: what the preview shows and the journal keeps.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub id: String,
    pub created: String,
    /// organise | aside (duplicates set aside)
    pub kind: String,
    pub label: String,
    pub dat: Value,
    pub platform: String,
    pub mode: String,
    pub format: String,
    #[serde(default)]
    pub out: Option<String>,
    pub ops: Vec<Op>,
    /// Games planned, games already in place, and why others were left
    pub games: usize,
    pub in_place: usize,
    pub conflicts: Vec<Value>,
    pub duplicates: Vec<Value>,
    pub left: Vec<Value>,
    pub space: Vec<Value>,
    /// A folder with games added for the output (mode 3), when it isn't in one
    #[serde(default)]
    pub new_root: Option<String>,
    /// Targets there already with the right contents: `{ game, to }` (an export keeps them)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub present: Vec<Value>,
    /// For an export (export.rs): which, and where to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export: Option<Value>,
}

impl Plan {
    /// What the preview and the history show (without the operations).
    pub fn summary(&self) -> Value {
        let mut by_op: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
        for o in &self.ops {
            let e = by_op.entry(o.op.as_str()).or_default();
            e.0 += 1;
            e.1 += o.size;
        }
        let set_aside: usize = self.ops.iter().map(|o| o.aside.len() + (o.op == "aside") as usize).sum();
        json!({
            "id": self.id, "created": self.created, "kind": self.kind, "label": self.label, "dat": self.dat,
            "platform": self.platform, "platform_name": platforms::name_of(&self.platform), "mode": self.mode,
            "format": self.format, "out": self.out, "games": self.games, "in_place": self.in_place,
            "ops": self.ops.len(), "by_op": by_op.iter().map(|(k, (n, b))| json!({ "op": k, "count": n, "bytes": b })).collect::<Vec<_>>(),
            "set_aside": set_aside, "conflicts": self.conflicts.len(), "duplicates": self.duplicates.len(), "left": self.left.len(),
            "space": self.space, "short": self.space.iter().any(|s| s["short"] == true), "new_root": self.new_root,
        })
    }
}

/// What the Organise dialog chose.
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub dat_id: i64,
    /// copy | rename | move
    pub mode: String,
    pub out: Option<PathBuf>,
    /// keep | zip | loose
    pub format: String,
    pub partial: bool,
    pub always_copy: bool,
    /// Only the misnamed files (rename mode, as they are)
    pub only_misnamed: bool,
    /// Only these games (by name), not the whole platform
    pub games: Option<Vec<String>>,
    /// Laid out for ES-DE (an export, Phase 3): a game of one file is the file; a game of
    /// several is a folder named as its sheet (or first file), which ES-DE shows as the game
    pub esde: bool,
    /// Every game in a folder of its own with its files named as the DAT says (the BIOS
    /// set: a system's folder, its files as the cores look for them)
    pub per_game_folder: bool,
    /// Files placed before (an export's), by path key: their size and modified time then.
    /// One still the same is there already, without reading it again.
    pub known: HashMap<String, (u64, i64)>,
}

// ------------------------------------------------------------ names

/// A name made safe for a file on Windows (and everywhere else).
pub fn safe_name(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    let s = s.trim_start().to_string();
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if s.is_empty() {
        "_".into()
    } else if reserved {
        format!("_{s}")
    } else {
        s
    }
}

/// A ROM's name from a DAT as a relative path (some DATs put ROMs in folders), each part made safe.
pub(crate) fn safe_rel(name: &str) -> PathBuf {
    name.split(['/', '\\']).filter(|p| !p.is_empty() && *p != "." && *p != "..").map(safe_name).collect()
}

/// How a platform's games are named (Settings › Naming), from library.json's `naming`.
#[derive(Clone, Debug)]
pub struct Naming {
    pub template: String,
    pub folder: bool,
}

pub const DEFAULT_TEMPLATE: &str = "{name}";

pub fn naming(lib: &Library, platform: &str) -> Naming {
    let n = &lib.meta()["naming"][platform];
    Naming {
        template: n["template"].as_str().filter(|t| !t.trim().is_empty()).unwrap_or(DEFAULT_TEMPLATE).to_string(),
        folder: n["folder"].as_bool().unwrap_or(false),
    }
}

pub fn set_naming(lib: &Library, platform: &str, template: &str, folder: bool) -> Result<()> {
    platforms::by_id(platform).ok_or_else(|| anyhow!("There's no platform {platform}."))?;
    check_template(template)?;
    let mut all = lib.meta()["naming"].as_object().cloned().unwrap_or_default();
    if template.trim() == DEFAULT_TEMPLATE && !folder {
        all.remove(platform);
    } else {
        all.insert(platform.into(), json!({ "template": template.trim(), "folder": folder }));
    }
    lib.update_meta(json!({ "naming": all }))?;
    Ok(())
}

const FIELDS: &[&str] = &["name", "title", "region", "languages", "revision"];

fn check_template(t: &str) -> Result<()> {
    if t.trim().is_empty() {
        bail!("A name needs at least one field, such as {{name}}.");
    }
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        let Some(j) = rest[i..].find('}') else { bail!("A {{ in the name isn't closed.") };
        let field = &rest[i + 1..i + j];
        if !FIELDS.contains(&field) {
            bail!("{{{field}}} isn't a field a name can use (they are {}).", FIELDS.iter().map(|f| format!("{{{f}}}")).collect::<Vec<_>>().join(", "));
        }
        rest = &rest[i + j + 1..];
    }
    Ok(())
}

pub(crate) struct GameFields {
    pub(crate) name: String,
    title: Option<String>,
    regions: Option<String>,
    languages: Option<String>,
    revision: Option<String>,
}

/// A game's name by the template; empty fields drop the brackets around them.
pub(crate) fn render(template: &str, g: &GameFields) -> String {
    if template == DEFAULT_TEMPLATE {
        return g.name.clone();
    }
    let value = |f: &str| -> String {
        match f {
            "name" => g.name.clone(),
            "title" => g.title.clone().unwrap_or_else(|| g.name.clone()),
            "region" => g.regions.clone().unwrap_or_default(),
            "languages" => g.languages.clone().unwrap_or_default(),
            "revision" => g.revision.clone().map(|r| format!("Rev {r}")).unwrap_or_default(),
            _ => String::new(),
        }
    };
    let mut out = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let j = rest[i..].find('}').map(|j| i + j).unwrap_or(rest.len() - 1);
        out.push_str(&value(&rest[i + 1..j]));
        rest = &rest[j + 1..];
    }
    out.push_str(rest);
    // brackets left empty by a missing field go, with the space before them
    for (a, b) in [("(", ")"), ("[", "]")] {
        out = out.replace(&format!(" {a}{b}"), "").replace(&format!("{a}{b}"), "");
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The fields a name can use, and a sample game named by `template` (Settings › Naming).
pub fn example(template: &str) -> Result<Value> {
    check_template(template)?;
    let g = GameFields {
        name: "Super Game (USA, Europe) (En,Fr) (Rev 1)".into(),
        title: Some("Super Game".into()),
        regions: Some("USA, Europe".into()),
        languages: Some("En,Fr".into()),
        revision: Some("1".into()),
    };
    let bare = GameFields { name: "Other Game (Japan)".into(), title: Some("Other Game".into()), regions: Some("Japan".into()), languages: None, revision: None };
    Ok(json!({ "fields": FIELDS, "examples": [[g.name.clone(), render(template, &g)], [bare.name.clone(), render(template, &bare)]] }))
}

fn ext_of_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.contains(['/', '\\']) => format!(".{ext}"),
        _ => String::new(),
    }
}

// ------------------------------------------------------------ files and drives

pub(crate) fn key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

/// The nearest folder above `p` that exists (for free space and the drive it's on).
pub(crate) fn existing(p: &Path) -> PathBuf {
    let mut at = p.to_path_buf();
    while !at.exists() {
        match at.parent() {
            Some(parent) if parent != at => at = parent.to_path_buf(),
            _ => break,
        }
    }
    at
}

/// Whether two paths are on the same drive (a rename or a hardlink works between them).
pub fn same_drive(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(existing(a)), std::fs::metadata(existing(b))) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let first = |p: &Path| p.components().next().map(|c| c.as_os_str().to_string_lossy().to_lowercase());
        first(a) == first(b)
    }
}

/// The drive a path is on, as the preview names it.
pub(crate) fn drive_name(p: &Path) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(existing(p)).map(|m| format!("dev{}", m.dev())).unwrap_or_default()
    }
    #[cfg(not(unix))]
    {
        p.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default()
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows
    matches!(e.raw_os_error(), Some(18) | Some(17)) || e.kind() == io::ErrorKind::CrossesDevices
}

/// The folder with games a path is in, and its path there with `/`.
pub(crate) fn in_root<'a>(roots: &'a [Root], p: &Path) -> Option<(&'a Root, String)> {
    let k = key(p);
    roots.iter().filter(|r| r.found()).find_map(|r| {
        let base = key(&r.dir);
        let base = base.trim_end_matches('/');
        k.strip_prefix(base).and_then(|rest| rest.strip_prefix('/')).map(|rel| {
            // the part of the original path after the root, with its own case
            let n = p.to_string_lossy().replace('\\', "/");
            (r, n[n.len() - rel.len()..].to_string())
        })
    })
}

pub(crate) fn root_path(roots: &[Root], root: &str, rel: &str) -> Option<PathBuf> {
    roots.iter().find(|r| r.id == root).map(|r| rel.split('/').fold(r.dir.clone(), |p, part| p.join(part)))
}

// ------------------------------------------------------------ planning

/// A matched file that could stand for a ROM.
#[derive(Clone, Debug)]
pub(crate) struct Cand {
    pub(crate) src: Src,
    /// For a member: its archive's (or disc image's) kind, name and how many members it has
    pub(crate) archive: Option<(String, String, i64)>,
    pub(crate) named: bool,
    /// How it matched (`cue`: a cue sheet matched by the files it names)
    pub(crate) method: String,
}

impl Cand {
    /// A track (or the ISO) in a disc image, which is only ever placed whole.
    pub(crate) fn in_image(&self) -> bool {
        self.archive.as_ref().is_some_and(|a| crate::image::KINDS.contains(&a.0.as_str()))
    }
}

pub(crate) struct RomRow {
    pub(crate) name: String,
    pub(crate) cands: Vec<Cand>,
}

pub(crate) struct GameRow {
    pub(crate) fields: GameFields,
    pub(crate) roms: Vec<RomRow>,
}

/// An id for a change, ordered by time to the millisecond (the newest sorts last).
pub(crate) fn new_id() -> String {
    use std::sync::atomic::AtomicU64;
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let mut prev = LAST.load(Ordering::SeqCst);
    let ms = loop {
        let next = now_ms.max(prev + 1);
        match LAST.compare_exchange(prev, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => break next,
            Err(p) => prev = p,
        }
    };
    let t = crate::library::iso_from_unix((ms / 1000) as i64).replace(['-', ':'], "").replace('T', "-").replace('Z', "");
    format!("{t}-{:03}", ms % 1000)
}

/// The games of the DAT with their ROMs and every file found for each.
pub(crate) fn load_games(conn: &Connection, roots: &[Root], dat_id: i64) -> Result<Vec<GameRow>> {
    let found: HashSet<&str> = roots.iter().filter(|r| r.found()).map(|r| r.id.as_str()).collect();
    let mut q = conn.prepare(
        "SELECT g.id, g.name, g.title, g.regions, g.languages, g.revision, r.id, r.name,
                f.id, f.root_id, f.rel_path, f.name, f.size, b.crc32, b.sha1, f.missing_since,
                p.id, p.root_id, p.rel_path, p.name, p.kind, (SELECT COUNT(*) FROM files x WHERE x.parent_id = p.id), m.method
         FROM dat_games g JOIN dat_roms r ON r.game_id = g.id
         LEFT JOIN matches m ON m.rom_id = r.id
         LEFT JOIN files f ON f.id = m.file_id
         LEFT JOIN blobs b ON b.id = f.blob_id
         LEFT JOIN files p ON p.id = f.parent_id
         WHERE g.dat_id = ?1 AND (r.status IS NULL OR r.status != 'nodump')
         ORDER BY g.name, g.id, r.name, r.id, f.root_id, f.rel_path",
    )?;
    let mut games: Vec<GameRow> = vec![];
    let mut last: (i64, i64) = (-1, -1);
    let mut rows = q.query([dat_id])?;
    while let Some(r) = rows.next()? {
        let (gid, rid): (i64, i64) = (r.get(0)?, r.get(6)?);
        if gid != last.0 {
            games.push(GameRow {
                fields: GameFields { name: r.get(1)?, title: r.get(2)?, regions: r.get(3)?, languages: r.get(4)?, revision: r.get(5)? },
                roms: vec![],
            });
        }
        let g = games.last_mut().unwrap();
        if (gid, rid) != last {
            g.roms.push(RomRow { name: r.get(7)?, cands: vec![] });
        }
        last = (gid, rid);
        let Some(fid) = r.get::<_, Option<i64>>(8)? else { continue };
        let missing: Option<String> = r.get(15)?;
        let froot: String = r.get(9)?;
        if missing.is_some() || !found.contains(froot.as_str()) {
            continue;
        }
        let (frel, fname, fsize): (String, String, i64) = (r.get(10)?, r.get(11)?, r.get(12)?);
        let (crc32, sha1): (Option<String>, Option<String>) = (r.get(13)?, r.get(14)?);
        let parent: Option<i64> = r.get(16)?;
        let method: String = r.get(22)?;
        let rom_name = g.roms.last().unwrap().name.clone();
        let cand = match parent {
            None => Cand {
                src: Src {
                    file_id: fid, path: root_path(roots, &froot, &frel).unwrap_or_default().display().to_string(), root: froot, rel: frel,
                    member: None, size: fsize as u64, crc32: crc32.unwrap_or_default(), sha1,
                },
                archive: None,
                named: fname == rom_name,
                method,
            },
            Some(pid) => {
                let (proot, prel, pname, pkind, count): (String, String, String, String, i64) = (r.get(17)?, r.get(18)?, r.get(19)?, r.get(20)?, r.get(21)?);
                let member = frel.strip_prefix(&format!("{prel}/")).unwrap_or(&fname).to_string();
                // a disc image's tracks have no names of their own: the image is named for the game
                let image = crate::image::KINDS.contains(&pkind.as_str());
                Cand {
                    named: (member == rom_name || image) && pname == format!("{}.{pkind}", g.fields.name),
                    method,
                    src: Src {
                        file_id: pid, path: root_path(roots, &proot, &prel).unwrap_or_default().display().to_string(), root: proot, rel: prel,
                        member: Some(member), size: fsize as u64, crc32: crc32.unwrap_or_default(), sha1,
                    },
                    archive: Some((pkind, pname, count)),
                }
            }
        };
        g.roms.last_mut().unwrap().cands.push(cand);
    }
    Ok(games)
}

/// Which of the files found for a ROM is used (the others are duplicates): one named
/// right, else one already in the format it's going to, else one on the output's drive,
/// else the first by folder and path.
fn pick<'a>(cands: &'a [Cand], format: &str, out: Option<&Path>) -> Option<&'a Cand> {
    cands.iter().min_by_key(|c| {
        let in_format = match format {
            "zip" => c.archive.as_ref().is_some_and(|a| a.0 == "zip"),
            "loose" => c.archive.is_none(),
            _ => true,
        };
        let on_drive = out.is_none_or(|o| same_drive(Path::new(&c.src.path), o));
        (!c.named, !in_format, !on_drive, c.src.root.clone(), c.src.rel.clone(), c.src.member.clone())
    })
}

/// The files a game is organised from (one per ROM, or an archive or disc image used
/// whole, whose id is returned too), the others being duplicates.
pub(crate) fn choose<'a>(g: &'a GameRow, format: &str, out: Option<&Path>) -> (Vec<(&'a RomRow, &'a Cand)>, Option<i64>) {
    let mut chosen: Vec<(&RomRow, &Cand)> = g.roms.iter().filter_map(|r| pick(&r.cands, format, out).map(|c| (r, c))).collect();
    // a disc image holding the game's tracks is used whole, and its cue sheet isn't needed
    if let Some(img) = chosen.iter().find(|(_, c)| c.in_image()).map(|(_, c)| c.src.file_id) {
        let chosen = g.roms.iter()
            .filter(|r| !cue::is_sheet(&r.name))
            .filter_map(|r| r.cands.iter().find(|c| c.src.file_id == img && c.in_image()).map(|c| (r, c)))
            .collect();
        return (chosen, Some(img));
    }
    // one archive holding every ROM of the game is used whole, if there is one
    let have = g.roms.iter().filter(|r| !r.cands.is_empty()).count();
    let mut whole: Option<i64> = None;
    let archives: HashSet<i64> = chosen.iter().filter(|(_, c)| c.archive.is_some()).map(|(_, c)| c.src.file_id).collect();
    for a in g.roms.iter().flat_map(|r| r.cands.iter()).filter(|c| c.archive.is_some() && !c.in_image()).map(|c| c.src.file_id).collect::<HashSet<_>>() {
        let all = g.roms.iter().filter(|r| !r.cands.is_empty()).all(|r| r.cands.iter().any(|c| c.src.file_id == a && c.archive.is_some()));
        let count = g.roms.iter().flat_map(|r| r.cands.iter()).find(|c| c.src.file_id == a).and_then(|c| c.archive.as_ref()).map(|x| x.2).unwrap_or(0);
        if all && count as usize == have && (whole.is_none() || archives.contains(&a)) {
            whole = Some(a);
        }
    }
    if let Some(a) = whole {
        chosen = g.roms.iter().filter_map(|r| r.cands.iter().find(|c| c.src.file_id == a && c.archive.is_some()).map(|c| (r, c))).collect();
    }
    (chosen, whole)
}

struct Planner<'a> {
    plan: Plan,
    taken: HashSet<String>,
    roots: &'a [Root],
    mode: String,
    always_copy: bool,
    /// archive file id → (the archive, its members, members used by this plan)
    consumed: HashMap<i64, (Src, i64, HashSet<String>)>,
    /// file id → where an earlier step of this plan moved it
    moved: HashMap<i64, String>,
    /// archive file id → how many members it has
    counts: HashMap<i64, i64>,
}

impl Planner<'_> {
    /// A target not already taken by another game of this plan: `name`, else `name (2)`…
    fn claim(&mut self, dir: &Path, name: &str) -> PathBuf {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
            _ => (name.to_string(), String::new()),
        };
        let mut n = 1;
        loop {
            let candidate = if n == 1 { dir.join(name) } else { dir.join(format!("{stem} ({n}){ext}")) };
            if self.taken.insert(key(&candidate)) {
                return candidate;
            }
            n += 1;
        }
    }

    /// What a file going to `to` from `src` takes: nothing when it's there already.
    fn place(&mut self, game: &str, src: &Src, to: PathBuf) -> Option<Op> {
        let src = self.redirect(src);
        let from = Path::new(&src.path);
        if key(from) == key(&to) && from.file_name() == to.file_name() {
            return None;
        }
        // a file an earlier game moved is copied from where it went
        let moved = self.moved.contains_key(&src.file_id);
        let op = match self.mode.as_str() {
            "rename" | "move" if moved => "copy",
            "rename" => "rename",
            "move" => "move",
            _ if self.always_copy => "copy",
            _ => "link",
        };
        Some(Op { op: op.into(), game: game.into(), to: to.display().to_string(), from: vec![src.clone()], names: vec![], aside: vec![], size: src.size, note: None })
    }

    /// Where a source is by the time this step runs (an earlier step may have moved it).
    fn redirect(&self, src: &Src) -> Src {
        let mut s = src.clone();
        if let Some(p) = self.moved.get(&src.file_id) {
            s.path = p.clone();
        }
        s
    }

    /// A member of an archive is used by a step (an archive all of whose members are
    /// used is set aside at the end, in modes 2 and 3).
    fn consume(&mut self, src: &Src) {
        let Some(m) = &src.member else { return };
        let count = self.counts.get(&src.file_id).copied().unwrap_or(i64::MAX);
        let e = self.consumed.entry(src.file_id).or_insert_with(|| (whole_archive(src), count, HashSet::new()));
        e.2.insert(m.clone());
    }
}

/// Plan organising a platform's set (nothing is written).
pub fn plan(ix: &Index, lib: &Library, o: &Options) -> Result<Plan> {
    let conn = ix.conn();
    let d = audit::dat_ref(conn, o.dat_id)?;
    if !["copy", "rename", "move"].contains(&o.mode.as_str()) {
        bail!("There's no way to organise called {}.", o.mode);
    }
    if !["keep", "zip", "loose"].contains(&o.format.as_str()) {
        bail!("There's no format called {}.", o.format);
    }
    let roots = roots::list(lib);
    let platform_folder = platforms::by_id(&d.platform).and_then(|p| p.dat_names.first().copied()).unwrap_or(&d.platform).to_string();
    let out: Option<PathBuf> = match o.mode.as_str() {
        "copy" => Some(o.out.clone().ok_or_else(|| anyhow!("Choose the folder the sorted set goes to."))?),
        "move" => Some(o.out.clone().unwrap_or_else(|| lib.root().join(safe_name(&platform_folder)))),
        _ => None,
    };
    if let Some(out) = &out {
        if key(out).starts_with(&key(&lib.root().join(APP_DIR))) {
            bail!("The library's own _library folder can't hold games.");
        }
    }
    let mut new_root = None;
    if o.mode == "move" {
        let out = out.as_ref().unwrap();
        if in_root(&roots, &out.join("x")).is_none() {
            // checked now, added when the change runs
            let probe = roots::check_new(lib, out);
            if let Err(e) = probe {
                bail!("The games can't go to {}: {e:#}", out.display());
            }
            new_root = Some(out.display().to_string());
        }
    }
    let naming = naming(lib, &d.platform);
    let format = if o.only_misnamed { "keep".to_string() } else { o.format.clone() };
    if platforms::is_disc(&d.platform) && format != "keep" {
        bail!("Disc games keep their format when they're organised: Convert changes it.");
    }
    let mode = if o.only_misnamed { "rename".to_string() } else { o.mode.clone() };
    let mut games = load_games(conn, &roots, o.dat_id)?;
    if let Some(only) = &o.games {
        let only: HashSet<&str> = only.iter().map(String::as_str).collect();
        games.retain(|g| only.contains(g.fields.name.as_str()));
    }
    let id = new_id();
    let label = match (o.only_misnamed, mode.as_str()) {
        (true, _) => format!("Renamed the misnamed {} files", platforms::name_of(&d.platform)),
        (_, "copy") => format!("Sorted {} into {}", platforms::name_of(&d.platform), out.as_ref().unwrap().display()),
        (_, "rename") => format!("Renamed {} where they are", platforms::name_of(&d.platform)),
        _ => format!("Moved {} into {}", platforms::name_of(&d.platform), out.as_ref().unwrap().display()),
    };
    let mut p = Planner {
        plan: Plan {
            id, created: now(), kind: "organise".into(), label,
            dat: json!({ "id": d.id, "name": d.name, "version": d.version }), platform: d.platform.clone(),
            mode: mode.clone(), format: format.clone(), out: out.as_ref().map(|o| o.display().to_string()),
            ops: vec![], games: 0, in_place: 0, conflicts: vec![], duplicates: vec![], left: vec![], space: vec![], new_root,
            present: vec![], export: None,
        },
        taken: HashSet::new(),
        roots: &roots,
        mode: mode.clone(),
        always_copy: o.always_copy,
        consumed: HashMap::new(),
        moved: HashMap::new(),
        counts: games.iter().flat_map(|g| g.roms.iter()).flat_map(|r| r.cands.iter()).filter_map(|c| c.archive.as_ref().map(|a| (c.src.file_id, a.2))).collect(),
    };
    for g in &games {
        // a game in a disc image has its cue sheet (the image needs none)
        let in_image = g.roms.iter().any(|r| r.cands.iter().any(Cand::in_image));
        let have = g.roms.iter().filter(|r| !r.cands.is_empty() || (in_image && cue::is_sheet(&r.name))).count();
        if have == 0 || (have < g.roms.len() && !o.partial) {
            continue;
        }
        if o.only_misnamed && g.roms.iter().all(|r| r.cands.is_empty() || r.cands.iter().any(|c| c.named)) {
            // a misnamed copy of a file that's named right elsewhere is a duplicate, not renamed
            for c in g.roms.iter().flat_map(|r| r.cands.iter()).filter(|c| !c.named) {
                let shown = match &c.src.member {
                    Some(m) => format!("{} › {m}", c.src.path),
                    None => c.src.path.clone(),
                };
                p.plan.left.push(json!({ "path": shown, "why": format!("{} is named right in another file, so this is a copy of it (Set aside duplicates deals with those).", g.fields.name) }));
            }
            continue;
        }
        let (chosen, whole) = choose(g, &format, out.as_deref());
        for r in &g.roms {
            for c in &r.cands {
                if !chosen.iter().any(|(_, k)| std::ptr::eq(*k, c)) {
                    let shown = match &c.src.member {
                        Some(m) => format!("{} › {m}", c.src.path),
                        None => c.src.path.clone(),
                    };
                    p.plan.duplicates.push(json!({ "game": g.fields.name, "rom": r.name, "path": shown }));
                }
            }
        }
        let any_member = chosen.iter().any(|(_, c)| c.archive.is_some());
        let target = match format.as_str() {
            "zip" => "zip",
            "loose" => "loose",
            _ if whole.is_some() => "archive",
            _ if any_member => "zip",
            _ => "loose",
        };
        let base = safe_name(&render(&naming.template, &g.fields));
        let src_dir = |c: &Cand| Path::new(&c.src.path).parent().map(Path::to_path_buf).unwrap_or_default();
        let first = chosen[0].1;
        let multi = g.roms.len() > 1;
        // for ES-DE, a game of several files is a folder named as the file it starts from
        let per_game = (naming.folder || o.per_game_folder) && !o.esde;
        let folder_name = if o.esde {
            let start = g.roms.iter().find(|r| cue::is_sheet(&r.name)).or(g.roms.first()).map(|r| r.name.rsplit(['/', '\\']).next().unwrap_or(&r.name).to_string());
            safe_name(&start.unwrap_or_else(|| base.clone()))
        } else {
            base.clone()
        };
        let dest = |folder: bool| -> PathBuf {
            match &out {
                Some(o) if folder => o.join(&folder_name),
                Some(o) => o.clone(),
                None => src_dir(first),
            }
        };
        let before = p.plan.ops.len();
        let mut conflict = None;
        match target {
            "archive" => {
                let c = first;
                let (kind, _, _) = c.archive.clone().unwrap();
                // (a disc image's tracks need no names)
                let names_right = c.in_image() || chosen.iter().all(|(r, c)| c.src.member.as_deref() == Some(r.name.as_str()));
                let dir = dest(per_game);
                if names_right {
                    let to = p.claim(&dir, &format!("{base}.{kind}"));
                    let src = whole_archive(&c.src);
                    if let Some(op) = p.place(&g.fields.name, &src, to) {
                        p.plan.ops.push(op);
                    }
                } else {
                    let to = p.claim(&dir, &format!("{base}.zip"));
                    let mut op = zip_op(&g.fields.name, &to, &chosen);
                    op.from = op.from.iter().map(|s| p.redirect(s)).collect();
                    if kind != "zip" {
                        op.note = Some(format!("The .{kind} is rebuilt as a ZIP: its files need renaming, and the app doesn't write .{kind} files."));
                    }
                    p.plan.ops.push(op);
                }
            }
            "zip" => {
                let dir = dest(per_game);
                let to_name = format!("{base}.zip");
                let already = whole.is_some()
                    && first.archive.as_ref().is_some_and(|a| a.0 == "zip")
                    && chosen.iter().all(|(r, c)| c.src.member.as_deref() == Some(r.name.as_str()))
                    && torrentzip::is_torrentzip(Path::new(&first.src.path));
                let to = p.claim(&dir, &to_name);
                if already {
                    let src = whole_archive(&first.src);
                    if let Some(op) = p.place(&g.fields.name, &src, to) {
                        p.plan.ops.push(op);
                    }
                } else {
                    let mut op = zip_op(&g.fields.name, &to, &chosen);
                    op.from = op.from.iter().map(|s| p.redirect(s)).collect();
                    for (_, c) in &chosen {
                        if c.archive.is_none() && mode != "copy" {
                            op.aside.push(p.redirect(&c.src));
                        }
                    }
                    p.plan.ops.push(op);
                }
            }
            _ => {
                let folder = per_game || multi;
                // a cue sheet that isn't the DAT's own names the tracks as they were: it's rewritten
                let mut renames: Vec<String> = vec![];
                let mut sheets: Vec<usize> = vec![];
                for (r, c) in &chosen {
                    let (dir, name) = match &out {
                        None => (src_dir(c), if multi { r.name.clone() } else { format!("{base}{}", ext_of_name(&r.name)) }),
                        Some(_) if folder => (dest(true), r.name.clone()),
                        Some(_) => (dest(false), format!("{base}{}", ext_of_name(&r.name))),
                    };
                    let rel = safe_rel(&name);
                    let to = p.claim(&dir.join(rel.parent().unwrap_or(Path::new(""))), &rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    if c.archive.is_none() && c.method == "cue" {
                        let src = p.redirect(&c.src);
                        let in_place = key(Path::new(&src.path)) == key(&to);
                        let aside = if mode != "copy" && !in_place { vec![src.clone()] } else { vec![] };
                        sheets.push(p.plan.ops.len());
                        p.plan.ops.push(Op {
                            op: "cue".into(), game: g.fields.name.clone(), to: to.display().to_string(), from: vec![src],
                            names: vec![], aside, size: c.src.size, note: Some("Its FILE lines are rewritten for the tracks' new names.".into()),
                        });
                        continue;
                    }
                    if c.archive.is_none() && !cue::is_sheet(&r.name) {
                        let old = Path::new(&c.src.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        renames.extend([old, to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()]);
                    }
                    if c.archive.is_some() {
                        p.plan.ops.push(Op {
                            op: "extract".into(), game: g.fields.name.clone(), to: to.display().to_string(), from: vec![p.redirect(&c.src)],
                            names: vec![], aside: vec![], size: c.src.size, note: None,
                        });
                    } else if let Some(op) = p.place(&g.fields.name, &c.src, to) {
                        p.plan.ops.push(op);
                    }
                }
                for i in sheets {
                    p.plan.ops[i].names = renames.clone();
                }
            }
        }
        // a target that's there already with the same contents needs nothing; with other
        // contents (and not the source itself) it's a conflict, and the game is left
        let ops: Vec<Op> = p.plan.ops.drain(before..).collect();
        let mut kept = vec![];
        let mut there = vec![];
        for op in ops {
            let to = Path::new(&op.to);
            if !to.exists() || op.from.iter().any(|s| s.member.is_none() && same_file(Path::new(&s.path), to)) {
                kept.push(op);
            } else if placed_before(&o.known, to) || already_there(conn, &roots, &op, cancel_never()) {
                there.push(json!({ "game": op.game, "to": op.to }));
            } else {
                conflict = Some(op.to.clone());
                break;
            }
        }
        if let Some(to) = conflict {
            p.plan.conflicts.push(json!({ "game": g.fields.name, "to": to }));
            continue;
        }
        p.plan.present.extend(there);
        for op in &kept {
            if matches!(op.op.as_str(), "zip" | "extract") {
                for s in &op.from {
                    p.consume(s);
                }
            }
            if matches!(op.op.as_str(), "rename" | "move") {
                p.moved.insert(op.from[0].file_id, op.to.clone());
            }
        }
        p.plan.ops.extend(kept);
        p.plan.games += 1;
        if p.plan.ops.len() == before {
            p.plan.in_place += 1;
        }
    }
    // archives every member of which went elsewhere are set aside after (modes 2 and 3)
    // a file used by more than one game isn't set aside (another step reads it)
    let mut uses: HashMap<(i64, Option<String>), usize> = HashMap::new();
    for op in &p.plan.ops {
        for s in &op.from {
            *uses.entry((s.file_id, s.member.clone())).or_default() += 1;
        }
    }
    for op in &mut p.plan.ops {
        op.aside.retain(|s| uses.get(&(s.file_id, s.member.clone())).copied().unwrap_or(0) <= 1);
    }
    let consumed: Vec<(Src, i64, HashSet<String>)> = p.consumed.values().cloned().collect();
    for (src, count, used) in consumed {
        let src = p.redirect(&src);
        if used.len() as i64 == count {
            if mode != "copy" {
                p.plan.ops.push(Op { op: "aside".into(), game: String::new(), to: src.path.clone(), size: 0, from: vec![src], names: vec![], aside: vec![], note: None });
            }
        } else if mode != "copy" {
            p.plan.left.push(json!({ "path": src.path, "why": "It holds other files too, so it stays where it is." }));
        }
    }
    p.plan.space = space_needed(&p.plan.ops);
    let _ = p.roots;
    Ok(p.plan)
}

/// The space a change's steps take on each drive they write to, and what's free there.
pub(crate) fn space_needed(ops: &[Op]) -> Vec<Value> {
    let mut need: BTreeMap<String, (u64, PathBuf)> = BTreeMap::new();
    for op in ops {
        let to = Path::new(&op.to);
        let from = op.from.first().map(|s| PathBuf::from(&s.path)).unwrap_or_default();
        let bytes = match op.op.as_str() {
            "rename" | "aside" => 0,
            "move" | "link" if same_drive(&from, to) => 0,
            _ => op.size,
        };
        if bytes > 0 {
            let e = need.entry(drive_name(to)).or_insert((0, to.to_path_buf()));
            e.0 += bytes;
        }
    }
    need.into_iter()
        .map(|(drive, (bytes, at))| {
            let free = fs4::available_space(existing(&at)).unwrap_or(u64::MAX);
            json!({ "drive": drive, "needed": bytes, "free": free, "short": bytes > free })
        })
        .collect()
}

fn cancel_never() -> &'static AtomicBool {
    static NEVER: AtomicBool = AtomicBool::new(false);
    &NEVER
}

/// Whether what a step would write is at its target already: a ZIP holding the same
/// members, or a file with the same contents (by the index when it's in a folder with
/// games, else by size and modified time, as copies keep it, or by reading it).
fn already_there(conn: &Connection, roots: &[Root], op: &Op, cancel: &AtomicBool) -> bool {
    let to = Path::new(&op.to);
    let Ok(meta) = std::fs::metadata(to) else { return false };
    if op.op == "zip" {
        let Ok(l) = archive::read(to, "zip", false, cancel) else { return false };
        let mut have: Vec<(String, u64, String)> = l.members.iter().map(|m| (m.name.clone(), m.hashes.size, m.hashes.crc32.clone())).collect();
        let mut want: Vec<(String, u64, String)> = op.names.iter().zip(&op.from).map(|(n, s)| (n.clone(), s.size, s.crc32.clone())).collect();
        have.sort();
        want.sort();
        return have == want;
    }
    let src = &op.from[0];
    if meta.len() != src.size {
        return false;
    }
    if let Some((root, rel)) = in_root(roots, to) {
        let sha1: Option<Option<String>> = conn
            .query_row(
                "SELECT b.sha1 FROM files f JOIN blobs b ON b.id = f.blob_id WHERE f.root_id = ?1 AND f.rel_path = ?2 AND f.parent_id IS NULL",
                params![root.id, rel],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        if let (Some(Some(have)), Some(want)) = (&sha1, &src.sha1) {
            return have == want;
        }
    }
    if src.member.is_none() {
        let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        if mtime(to).is_some() && mtime(to) == mtime(Path::new(&src.path)) {
            return true;
        }
        if src.sha1.is_none() && src.crc32.is_empty() {
            return false;
        }
    }
    check_file(to, src, cancel).is_ok()
}

/// Whether a file an export placed is still as it was then (its size and modified time).
fn placed_before(known: &HashMap<String, (u64, i64)>, to: &Path) -> bool {
    known.get(&key(to)).is_some_and(|&(size, mtime)| file_stamp(to) == Some((size, mtime)))
}

/// A file's size and modified time (ms), as an export remembers them.
pub(crate) fn file_stamp(p: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len(), t))
}

fn zip_op(game: &str, to: &Path, chosen: &[(&RomRow, &Cand)]) -> Op {
    Op {
        op: "zip".into(), game: game.into(), to: to.display().to_string(),
        from: chosen.iter().map(|(_, c)| c.src.clone()).collect(),
        names: chosen.iter().map(|(r, _)| r.name.replace('\\', "/")).collect(),
        aside: vec![], size: chosen.iter().map(|(_, c)| c.src.size).sum(), note: None,
    }
}

/// Plan setting aside the duplicates of a DAT's ROMs: every file but the one kept (as
/// organising would keep it). An archive is set aside only when everything in it is a duplicate.
pub fn plan_duplicates(ix: &Index, lib: &Library, dat_id: i64) -> Result<Plan> {
    let conn = ix.conn();
    let d = audit::dat_ref(conn, dat_id)?;
    let roots = roots::list(lib);
    let games = load_games(conn, &roots, dat_id)?;
    let mut plan = Plan {
        id: new_id(), created: now(), kind: "aside".into(),
        label: format!("Set aside {} duplicates", platforms::name_of(&d.platform)),
        dat: json!({ "id": d.id, "name": d.name, "version": d.version }), platform: d.platform.clone(),
        mode: "aside".into(), format: "keep".into(), out: None, ops: vec![], games: 0, in_place: 0,
        conflicts: vec![], duplicates: vec![], left: vec![], space: vec![], new_root: None, present: vec![], export: None,
    };
    let mut kept: HashSet<(i64, Option<String>)> = HashSet::new();
    let mut extra: Vec<&Cand> = vec![];
    for g in &games {
        let (chosen, _) = choose(g, "keep", None);
        for (_, c) in &chosen {
            kept.insert((c.src.file_id, c.src.member.clone()));
        }
        for c in g.roms.iter().flat_map(|r| r.cands.iter()) {
            if !chosen.iter().any(|(_, k)| std::ptr::eq(*k, c)) {
                extra.push(c);
            }
        }
    }
    let mut archives: HashMap<i64, (Src, i64, usize)> = HashMap::new();
    let mut seen: HashSet<(i64, Option<String>)> = HashSet::new();
    for c in extra {
        if kept.contains(&(c.src.file_id, c.src.member.clone())) || !seen.insert((c.src.file_id, c.src.member.clone())) {
            continue;
        }
        match &c.archive {
            None => plan.ops.push(Op { op: "aside".into(), game: String::new(), to: c.src.path.clone(), size: c.src.size, from: vec![c.src.clone()], names: vec![], aside: vec![], note: None }),
            Some((_, _, count)) => {
                let e = archives.entry(c.src.file_id).or_insert_with(|| {
                    let mut s = c.src.clone();
                    s.member = None;
                    (s, *count, 0)
                });
                e.2 += 1;
            }
        }
    }
    for (src, count, n) in archives.into_values() {
        if n as i64 == count && !kept.iter().any(|(id, _)| *id == src.file_id) {
            plan.ops.push(Op { op: "aside".into(), game: String::new(), to: src.path.clone(), size: 0, from: vec![src], names: vec![], aside: vec![], note: None });
        } else {
            plan.left.push(json!({ "path": src.path, "why": "It holds files that aren't duplicates too, so it stays." }));
        }
    }
    plan.ops.sort_by(|a, b| a.to.cmp(&b.to));
    plan.games = plan.ops.len();
    Ok(plan)
}

// ------------------------------------------------------------ the journal

fn journal_dir(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("journal")
}

fn plan_path(lib: &Library, id: &str) -> PathBuf {
    journal_dir(lib).join(format!("{id}.json"))
}

fn log_path(lib: &Library, id: &str) -> PathBuf {
    journal_dir(lib).join(format!("{id}.log"))
}

/// Plans waiting to be run (or dropped) are kept here, outside the library.
pub fn pending_path(data_dir: &Path, id: &str) -> PathBuf {
    data_dir.join("plans").join(format!("{id}.json"))
}

pub fn save_pending(data_dir: &Path, plan: &Plan) -> Result<()> {
    let dir = data_dir.join("plans");
    std::fs::create_dir_all(&dir)?;
    // old plans that were never run go
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let day = std::time::Duration::from_secs(24 * 3600);
        for e in rd.flatten() {
            if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().unwrap_or_default() > day) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    crate::config::write_atomic(&pending_path(data_dir, &plan.id), &serde_json::to_vec(plan)?)
}

pub fn load_pending(data_dir: &Path, id: &str) -> Result<Plan> {
    crate::library::valid_id(id).map_err(|_| anyhow!("There's no planned change {id}."))?;
    let bytes = std::fs::read(pending_path(data_dir, id)).map_err(|_| anyhow!("That preview has expired: plan it again."))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// One page of a plan's operations for the preview, optionally only one kind.
pub fn preview(plan: &Plan, what: &str, offset: usize, limit: usize) -> Value {
    let items: Vec<Value> = match what {
        "conflicts" => plan.conflicts.clone(),
        "duplicates" => plan.duplicates.clone(),
        "left" => plan.left.clone(),
        "present" => plan.present.clone(),
        op => plan
            .ops
            .iter()
            .filter(|o| op.is_empty() || op == "all" || o.op == op)
            .map(|o| {
                let full = |s: &Src| match &s.member {
                    Some(m) => format!("{} › {m}", s.path),
                    None => s.path.clone(),
                };
                // shown shorter: sources by their place in their folder with games, the
                // target in the output folder (or, renamed in its own folder, by its name)
                let from: Vec<String> = o.from.iter().map(|s| match &s.member {
                    Some(m) => format!("{} › {m}", s.rel),
                    None => s.rel.clone(),
                }).collect();
                let to = Path::new(&o.to);
                let same_dir = o.from.len() == 1 && o.from[0].member.is_none() && Path::new(&o.from[0].path).parent() == to.parent();
                let in_out = plan.out.as_deref().and_then(|out| to.strip_prefix(out).ok());
                let to_shown = if o.op == "aside" {
                    from.first().cloned().unwrap_or_default()
                } else if let Some(rel) = in_out {
                    rel.to_string_lossy().replace('\\', "/")
                } else if same_dir {
                    to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                } else {
                    o.to.clone()
                };
                json!({ "op": o.op, "game": o.game, "to": to_shown, "to_full": o.to, "from": from, "from_full": o.from.iter().map(full).collect::<Vec<_>>(), "size": o.size, "note": o.note, "aside": o.aside.len() })
            })
            .collect(),
    };
    let total = items.len();
    json!({ "total": total, "items": items.into_iter().skip(offset).take(limit.clamp(1, 1000)).collect::<Vec<_>>() })
}

/// What happened to a change so far, from its log.
#[derive(Default, Debug)]
struct State {
    done: HashMap<usize, Value>,
    finished: bool,
    undone: bool,
    emptied: bool,
}

fn read_state(lib: &Library, id: &str) -> State {
    let mut s = State::default();
    let text = std::fs::read_to_string(log_path(lib, id)).unwrap_or_default();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["state"].as_str() {
            Some("done") => s.finished = true,
            Some("undone") => s.undone = true,
            Some("emptied") => s.emptied = true,
            _ => {}
        }
        if let Some(i) = v["op"].as_u64() {
            s.done.insert(i as usize, v);
        }
    }
    s
}

fn log(lib: &Library, id: &str, v: &Value) -> Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log_path(lib, id))?;
    writeln!(f, "{v}")?;
    f.sync_data()?;
    Ok(())
}

/// A change's log, open while it runs: each step is written as it's done, and made
/// durable every so often and whenever a step set something aside (a step done but not
/// yet durable is safe to do again: a rename already done is recognised).
struct Log {
    f: std::fs::File,
    unsynced: usize,
}

impl Log {
    fn open(lib: &Library, id: &str) -> Result<Self> {
        Ok(Self { f: std::fs::OpenOptions::new().create(true).append(true).open(log_path(lib, id))?, unsynced: 0 })
    }
    fn write(&mut self, v: &Value, important: bool) -> Result<()> {
        writeln!(self.f, "{v}")?;
        self.unsynced += 1;
        if important || self.unsynced >= 64 {
            self.sync()?;
        }
        Ok(())
    }
    fn sync(&mut self) -> Result<()> {
        if self.unsynced > 0 {
            self.f.sync_data()?;
            self.unsynced = 0;
        }
        Ok(())
    }
}

fn load(lib: &Library, id: &str) -> Result<Plan> {
    crate::library::valid_id(id).map_err(|_| anyhow!("There's no change {id}."))?;
    let bytes = std::fs::read(plan_path(lib, id)).map_err(|_| anyhow!("There's no change {id} in the history."))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// The set-aside folders of a change (one per folder with games it set files aside in).
fn aside_dirs(lib: &Library, id: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = roots::list(lib).iter().map(|r| r.dir.join(SET_ASIDE).join(id)).filter(|p| p.is_dir()).collect();
    let in_lib = lib.root().join(APP_DIR).join("set-aside").join(id);
    if in_lib.is_dir() {
        v.push(in_lib);
    }
    // an export's: at the top of its folders
    if let Ok(plan) = load(lib, id) {
        let mut tops: Vec<PathBuf> = plan.ops.iter().flat_map(|o| o.from.iter().chain(o.aside.iter())).filter_map(|s| s.root.strip_prefix("dir:")).map(|d| Path::new(d).join(SET_ASIDE).join(id)).collect();
        tops.sort();
        tops.dedup();
        v.extend(tops.into_iter().filter(|p| p.is_dir()));
    }
    v
}

fn dir_size(p: &Path) -> u64 {
    let mut n = 0;
    let mut stack = vec![p.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => n += e.metadata().map(|m| m.len()).unwrap_or(0),
                _ => {}
            }
        }
    }
    n
}

/// The changes in the history, newest first, with what each can still do.
pub fn history(lib: &Library, running: &[String]) -> Result<Value> {
    let mut ids: Vec<String> = std::fs::read_dir(journal_dir(lib))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(String::from))
        .collect();
    ids.sort();
    ids.reverse();
    let mut out = vec![];
    let mut undo_next = true;
    for id in ids {
        let Ok(plan) = load(lib, &id) else { continue };
        let st = read_state(lib, &id);
        let held: u64 = aside_dirs(lib, &id).iter().map(|d| dir_size(d)).sum();
        let state = if running.contains(&id) {
            "running"
        } else if st.undone {
            "undone"
        } else if st.finished {
            "done"
        } else {
            "interrupted"
        };
        let had_aside = st.done.values().any(|v| v["aside"].as_array().is_some_and(|a| !a.is_empty()));
        let can_undo = undo_next && matches!(state, "done" | "interrupted") && !(st.emptied && had_aside);
        if matches!(state, "done" | "interrupted") {
            undo_next = false;
        }
        let mut s = plan.summary();
        s["state"] = json!(state);
        s["done_ops"] = json!(st.done.len());
        s["can_undo"] = json!(can_undo);
        s["held"] = json!(held);
        s["emptied"] = json!(st.emptied);
        out.push(s);
    }
    Ok(json!(out))
}

/// Keep the newest finished changes; older ones go unless they hold set-aside files.
fn prune(lib: &Library) {
    let Ok(rd) = std::fs::read_dir(journal_dir(lib)) else { return };
    let mut ids: Vec<String> = rd.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(String::from)).collect();
    ids.sort();
    let finished: Vec<&String> = ids.iter().filter(|id| {
        let st = read_state(lib, id);
        (st.finished || st.undone) && aside_dirs(lib, id).is_empty()
    }).collect();
    for id in finished.iter().rev().skip(KEEP) {
        let _ = std::fs::remove_file(plan_path(lib, id));
        let _ = std::fs::remove_file(log_path(lib, id));
    }
}

// ------------------------------------------------------------ running

/// Hash a file and check it's what was expected.
fn check_file(path: &Path, want: &Src, cancel: &AtomicBool) -> Result<Hashes> {
    let size = std::fs::metadata(path)?.len();
    let mut f = std::fs::File::open(path)?;
    let h = hash::hash_reader(&mut f, size, &path.to_string_lossy(), cancel)?;
    check_hashes(&h, want).with_context(|| format!("{} isn't what was expected after writing it", path.display()))?;
    Ok(h)
}

pub(crate) fn check_hashes(h: &Hashes, want: &Src) -> Result<()> {
    let ok = h.size == want.size
        && match &want.sha1 {
            Some(s) => h.sha1.as_deref() == Some(s.as_str()),
            None => h.crc32 == want.crc32,
        };
    if !ok {
        bail!("its contents don't match ({} bytes, CRC32 {}; expected {} bytes, CRC32 {})", h.size, h.crc32, want.size, want.crc32);
    }
    Ok(())
}

/// Members of archives this step reads, unpacked into a staging folder by `to`'s side.
fn stage(op: &Op, at: &Path, cancel: &AtomicBool) -> Result<HashMap<(String, String), PathBuf>> {
    let mut out = HashMap::new();
    let mut wanted: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    for s in &op.from {
        if let Some(m) = &s.member {
            wanted.entry(s.path.clone()).or_default().insert(m.clone());
        }
    }
    let mut n = 0;
    for (archive_path, members) in wanted {
        archive::each_wanted(Path::new(&archive_path), &|name| members.contains(name), cancel, &mut |name, _, r| {
            if members.contains(name) {
                n += 1;
                let p = at.join(format!("{n}"));
                let mut f = std::fs::File::create(&p)?;
                io::copy(r, &mut f)?;
                f.sync_all()?;
                out.insert((archive_path.clone(), name.to_string()), p);
            } else {
                io::copy(r, &mut io::sink())?;
            }
            Ok(())
        })
        .with_context(|| format!("couldn't read {archive_path}"))?;
    }
    for s in &op.from {
        if let Some(m) = &s.member {
            if !out.contains_key(&(s.path.clone(), m.clone())) {
                bail!("{m} isn't in {} any more", s.path);
            }
        }
    }
    Ok(out)
}

/// Move `path` into the change's set-aside folder; returns where it went.
fn set_aside(lib: &Library, roots: &[Root], id: &str, src: &Src) -> Result<String> {
    let from = Path::new(&src.path);
    let to = match (roots.iter().find(|r| r.id == src.root), src.root.strip_prefix("dir:")) {
        (Some(r), _) => src.rel.split('/').fold(r.dir.join(SET_ASIDE).join(id), |p, part| p.join(part)),
        // a file in an export's folder: set aside at the top of it
        (None, Some(dir)) => src.rel.split('/').fold(Path::new(dir).join(SET_ASIDE).join(id), |p, part| p.join(part)),
        (None, None) => lib.root().join(APP_DIR).join("set-aside").join(id).join(from.file_name().unwrap_or_default()),
    };
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::rename(from, &to).with_context(|| format!("couldn't set {} aside", from.display()))?;
    Ok(to.display().to_string())
}

pub(crate) fn make_parent(p: &Path) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("couldn't make the folder {}", dir.display()))?;
    }
    Ok(())
}

/// Rename `from` to `to`, by way of a temporary name when only the case differs.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if key(from) != key(to) && to.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} is there already", to.display())));
    }
    if key(from) == key(to) && from != to {
        let tmp = from.with_file_name(format!("{}.gamerepo-rename", from.file_name().unwrap_or_default().to_string_lossy()));
        std::fs::rename(from, &tmp)?;
        return std::fs::rename(&tmp, to);
    }
    std::fs::rename(from, to)
}

/// Copy `from` to `to` through a temporary file, checked before it takes its name.
fn copy_checked(from: &Path, to: &Path, want: &Src, cancel: &AtomicBool) -> Result<()> {
    let tmp = to.with_file_name(format!("{}.gamerepo-tmp", to.file_name().unwrap_or_default().to_string_lossy()));
    let r = (|| -> Result<()> {
        std::fs::copy(from, &tmp).with_context(|| format!("couldn't copy {}", from.display()))?;
        if let Ok(t) = std::fs::metadata(from).and_then(|m| m.modified()) {
            let _ = std::fs::File::options().write(true).open(&tmp).and_then(|f| f.set_modified(t));
        }
        if want.sha1.is_none() && want.crc32.is_empty() {
            check_archive_copy(from, &tmp, want, cancel)?;
        } else {
            check_file(&tmp, want, cancel)?;
        }
        std::fs::rename(&tmp, to)?;
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// A copy of a file with no hashes recorded: the same size, and for an archive every member reads back (unpacked, so
/// each one's CRC32 is checked) as the original records it.
fn check_archive_copy(from: &Path, copy: &Path, want: &Src, cancel: &AtomicBool) -> Result<()> {
    let size = std::fs::metadata(copy)?.len();
    if size != want.size {
        bail!("the copy of {} is {size} bytes, not {}", from.display(), want.size);
    }
    let kind = archive::container_of(&from.to_string_lossy());
    let Some(kind) = kind.filter(|k| !crate::image::KINDS.contains(k)) else {
        // a disc image, or a plain file (a document, a picture): the same bytes
        let hash = |p: &Path| -> Result<Option<String>> {
            Ok(hash::hash_reader(&mut std::fs::File::open(p)?, size, "", cancel)?.sha1)
        };
        if hash(from)? != hash(copy)? {
            bail!("the copy of {} isn't the same", from.display());
        }
        return Ok(());
    };
    let orig = archive::read(from, kind, false, cancel)?;
    let back = archive::read(copy, kind, true, cancel).with_context(|| format!("the copy of {} doesn't read back", from.display()))?;
    let set = |l: &archive::Listing| -> Vec<(String, u64, String)> {
        let mut v: Vec<_> = l.members.iter().map(|m| (m.name.clone(), m.hashes.size, m.hashes.crc32.clone())).collect();
        v.sort();
        v
    };
    if set(&orig) != set(&back) {
        bail!("the copy of {} doesn't hold the same files", from.display());
    }
    Ok(())
}

/// What a step wrote, for the index: a file's hashes, or an archive's (or disc
/// image's) kind and its members' hashes.
pub(crate) struct Written {
    pub(crate) members: Vec<(String, Hashes)>,
    pub(crate) file: Option<Hashes>,
    pub(crate) kind: &'static str,
}

/// Do one step. Returns what it wrote (for the index) and whether a move had to copy.
fn perform(op: &Op, cancel: &AtomicBool) -> Result<(Option<Written>, bool)> {
    let to = Path::new(&op.to);
    match op.op.as_str() {
        "rename" | "move" => {
            let src = &op.from[0];
            if !Path::new(&src.path).exists() && std::fs::metadata(to).is_ok_and(|m| m.len() == src.size) {
                // done already (by a run that stopped before its log was written)
                return Ok((None, false));
            }
            make_parent(to)?;
            match rename(Path::new(&src.path), to) {
                Ok(()) => Ok((None, false)),
                Err(e) if is_cross_device(&e) => {
                    copy_checked(Path::new(&src.path), to, src, cancel)?;
                    Ok((None, true))
                }
                Err(e) => Err(anyhow!("couldn't move {} to {}: {e}", src.path, to.display())),
            }
        }
        "link" | "copy" => {
            let src = &op.from[0];
            make_parent(to)?;
            if op.op == "link" && std::fs::hard_link(&src.path, to).is_ok() {
                return Ok((None, false));
            }
            copy_checked(Path::new(&src.path), to, src, cancel)?;
            Ok((None, true))
        }
        "zip" | "extract" => {
            make_parent(to)?;
            let staging = to.parent().unwrap_or(Path::new(".")).join(STAGING).join(format!("{}", std::process::id()));
            std::fs::create_dir_all(&staging)?;
            let r = (|| -> Result<(Option<Written>, bool)> {
                let staged = stage(op, &staging, cancel)?;
                let path_of = |s: &Src| -> PathBuf {
                    match &s.member {
                        Some(m) => staged[&(s.path.clone(), m.clone())].clone(),
                        None => PathBuf::from(&s.path),
                    }
                };
                if op.op == "extract" {
                    let src = &op.from[0];
                    let p = path_of(src);
                    let h = check_file(&p, src, cancel)?;
                    std::fs::rename(&p, to).or_else(|_| std::fs::copy(&p, to).map(|_| ()))?;
                    return Ok((Some(Written { members: vec![], file: Some(h), kind: "file" }), true));
                }
                let entries: Vec<torrentzip::Entry> = op
                    .from
                    .iter()
                    .zip(&op.names)
                    .map(|(s, n)| {
                        let p = path_of(s);
                        torrentzip::Entry { name: n.clone(), size: s.size, open: Box::new(move || Ok(Box::new(std::fs::File::open(p)?) as Box<dyn Read>)) }
                    })
                    .collect();
                let tmp = staging.join("game.zip");
                let written = torrentzip::write(&tmp, entries, cancel)?;
                // each member as expected, then the ZIP read back
                for (s, n) in op.from.iter().zip(&op.names) {
                    let h = written.iter().find(|(name, _)| name == n).map(|(_, h)| h).ok_or_else(|| anyhow!("{n} wasn't written"))?;
                    check_hashes(h, s).with_context(|| format!("{n} from {}", s.path))?;
                }
                let back = archive::members(&tmp, cancel)?;
                for (n, h) in &written {
                    if !back.iter().any(|(bn, bh)| bn == n && bh.sha1 == h.sha1) {
                        bail!("{n} didn't read back from the new ZIP as written");
                    }
                }
                if to.exists() {
                    bail!("{} is there already", to.display());
                }
                std::fs::rename(&tmp, to)?;
                Ok((Some(Written { members: written, file: None, kind: "zip" }), true))
            })();
            let _ = std::fs::remove_dir_all(&staging);
            let _ = std::fs::remove_dir(to.parent().unwrap_or(Path::new(".")).join(STAGING));
            r
        }
        "cue" => {
            make_parent(to)?;
            let src = &op.from[0];
            let h = write_sheet(Path::new(&src.path), to, &op.names, cancel)?;
            Ok((Some(Written { members: vec![], file: Some(h), kind: "file" }), true))
        }
        "convert" | "sheet" => {
            make_parent(to)?;
            Ok((Some(crate::convert::perform(op, cancel)?), true))
        }
        "text" => {
            // a file the change writes whole (an export's gamelist or custom collection)
            make_parent(to)?;
            let text = op.names.first().map(String::as_str).unwrap_or("");
            let tmp = to.with_file_name(format!("{}.gamerepo-tmp", to.file_name().unwrap_or_default().to_string_lossy()));
            std::fs::write(&tmp, text.as_bytes())?;
            if to.exists() {
                let _ = std::fs::remove_file(&tmp);
                bail!("{} is there already", to.display());
            }
            std::fs::rename(&tmp, to)?;
            let h = hash::hash_reader(&mut text.as_bytes(), text.len() as u64, &to.to_string_lossy(), cancel)?;
            Ok((Some(Written { members: vec![], file: Some(h), kind: "file" }), true))
        }
        "image" => {
            // a picture made smaller (an export's, D35)
            make_parent(to)?;
            let max: u32 = op.names.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let bytes = crate::export::shrink(Path::new(&op.from[0].path), max, to)?;
            let tmp = to.with_file_name(format!("{}.gamerepo-tmp", to.file_name().unwrap_or_default().to_string_lossy()));
            std::fs::write(&tmp, &bytes)?;
            if to.exists() {
                let _ = std::fs::remove_file(&tmp);
                bail!("{} is there already", to.display());
            }
            std::fs::rename(&tmp, to)?;
            let h = hash::hash_reader(&mut bytes.as_slice(), bytes.len() as u64, &to.to_string_lossy(), cancel)?;
            Ok((Some(Written { members: vec![], file: Some(h), kind: "file" }), true))
        }
        "aside" => Ok((None, false)),
        other => bail!("unknown step {other}"),
    }
}

/// Write the cue sheet (or GDI) at `from` to `to` with the files it names renamed
/// (`names`: old, new, old, new…), through a temporary file.
fn write_sheet(from: &Path, to: &Path, names: &[String], cancel: &AtomicBool) -> Result<Hashes> {
    let text = std::fs::read(from).with_context(|| format!("couldn't read {}", from.display()))?;
    let renames: HashMap<String, String> = names.chunks_exact(2).map(|p| (p[0].clone(), p[1].clone())).collect();
    let out = cue::rewrite(&String::from_utf8_lossy(&text), &renames, platforms::ext_of(&to.to_string_lossy()) == "gdi");
    let tmp = to.with_file_name(format!("{}.gamerepo-tmp", to.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, out.as_bytes())?;
    if to.exists() {
        let _ = std::fs::remove_file(&tmp);
        bail!("{} is there already", to.display());
    }
    std::fs::rename(&tmp, to)?;
    Ok(hash::hash_reader(&mut out.as_bytes(), out.len() as u64, &to.to_string_lossy(), cancel)?)
}

/// An archive as a whole file: its own hashes aren't known (only its members'), so a
/// copy of it is checked by reading every member back.
pub(crate) fn whole_archive(member: &Src) -> Src {
    let mut s = member.clone();
    s.member = None;
    s.sha1 = None;
    s.crc32 = String::new();
    s.size = std::fs::metadata(&s.path).map(|m| m.len()).unwrap_or(0);
    s
}

/// Keep the index in step with a step that's done (so no rescan is needed).
fn update_index(conn: &Connection, roots: &[Root], op: &Op, written: Option<&Written>, set_aside: &[Src]) -> Result<()> {
    let to = Path::new(&op.to);
    let placed = in_root(roots, to);
    let meta = std::fs::metadata(to).ok();
    let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
    let size = meta.as_ref().map(|m| m.len() as i64).unwrap_or(0);
    let name = to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match op.op.as_str() {
        "rename" | "move" => {
            let src = &op.from[0];
            match &placed {
                Some((root, rel)) => {
                    conn.execute(
                        "UPDATE files SET root_id = ?2, rel_path = ?3 || substr(rel_path, ?4), matched = 0 WHERE parent_id = ?1",
                        params![src.file_id, root.id, rel, src.rel.chars().count() as i64 + 1],
                    )?;
                    conn.execute(
                        "UPDATE files SET root_id = ?2, rel_path = ?3, name = ?4, mtime = ?5, size = ?6, missing_since = NULL, matched = 0 WHERE id = ?1",
                        params![src.file_id, root.id, rel, name, mtime, size],
                    )?;
                }
                None => {
                    conn.execute("DELETE FROM files WHERE id = ?1", [src.file_id])?;
                }
            }
        }
        "link" | "copy" => {
            if let Some((root, rel)) = &placed {
                let src = &op.from[0];
                conn.execute(
                    "INSERT OR REPLACE INTO files (root_id, parent_id, rel_path, name, kind, size, mtime, blob_id, hashed, matched, platform, locked)
                     SELECT ?2, NULL, ?3, ?4, kind, ?5, ?6, blob_id, hashed, 0, platform, locked FROM files WHERE id = ?1",
                    params![src.file_id, root.id, rel, name, size, mtime],
                )?;
                let new_id = conn.last_insert_rowid();
                conn.execute(
                    "INSERT OR REPLACE INTO files (root_id, parent_id, rel_path, name, kind, size, mtime, blob_id, hashed, matched, platform)
                     SELECT ?2, ?3, ?4 || substr(rel_path, ?5), name, kind, size, ?6, blob_id, hashed, 0, platform FROM files WHERE parent_id = ?1",
                    params![src.file_id, root.id, new_id, rel, src.rel.chars().count() as i64 + 1, mtime],
                )?;
            }
        }
        "zip" | "extract" | "cue" | "convert" | "sheet" => {
            if let (Some((root, rel)), Some(w)) = (&placed, written) {
                let platform: Option<String> = conn
                    .query_row("SELECT platform FROM files WHERE id = ?1", [op.from[0].file_id], |r| r.get(0))
                    .optional()?
                    .flatten();
                if let Some(h) = &w.file {
                    let blob = crate::scan::blob_id(conn, h)?;
                    conn.execute(
                        "INSERT OR REPLACE INTO files (root_id, rel_path, name, kind, size, mtime, blob_id, hashed, matched, platform)
                         VALUES (?1, ?2, ?3, 'file', ?4, ?5, ?6, 2, 0, ?7)",
                        params![root.id, rel, name, size, mtime, blob, platform],
                    )?;
                } else {
                    conn.execute(
                        "INSERT OR REPLACE INTO files (root_id, rel_path, name, kind, size, mtime, hashed, matched, platform)
                         VALUES (?1, ?2, ?3, ?7, ?4, ?5, 2, 1, ?6)",
                        params![root.id, rel, name, size, mtime, platform, w.kind],
                    )?;
                    let zid = conn.last_insert_rowid();
                    for (n, h) in &w.members {
                        let blob = crate::scan::blob_id(conn, h)?;
                        conn.execute(
                            "INSERT OR REPLACE INTO files (root_id, parent_id, rel_path, name, kind, size, mtime, blob_id, hashed, matched, platform)
                             VALUES (?1, ?2, ?3, ?4, 'member', ?5, ?6, ?7, 2, 0, ?8)",
                            params![root.id, zid, format!("{rel}/{n}"), n.rsplit('/').next().unwrap_or(n), h.size as i64, mtime, blob, platform],
                        )?;
                    }
                }
            }
        }
        "aside" => {
            conn.execute("DELETE FROM files WHERE id = ?1", [op.from[0].file_id])?;
        }
        _ => {}
    }
    for s in set_aside {
        conn.execute("DELETE FROM files WHERE id = ?1", [s.file_id])?;
    }
    Ok(())
}

/// Run a planned change (or finish an interrupted one): journal it, then each step in
/// turn, checked, with the index kept up to date. `progress` gets `{ item, items, name }`.
pub fn run(ix: &Index, lib: &Library, plan: &Plan, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    let id = plan.id.clone();
    let path = plan_path(lib, &id);
    if !path.exists() {
        if plan.ops.is_empty() {
            return Ok(json!({ "id": null, "done": 0, "failed": [], "stopped": false, "nothing": true }));
        }
        if plan.space.iter().any(|s| s["short"] == true) {
            bail!("There isn't enough free space for this change (see the preview).");
        }
        crate::config::write_json(&path, &serde_json::to_value(plan)?)?;
    }
    if let Some(dir) = &plan.new_root {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir)?;
        if in_root(&roots::list(lib), &dir.join("x")).is_none() {
            roots::add(lib, &dir, Some(&plan.platform))?;
            let conn = ix.conn();
            for r in roots::list(lib) {
                conn.execute("INSERT OR IGNORE INTO roots (id, path, platform) VALUES (?1, ?2, ?3)", params![r.id, r.path, r.platform])?;
            }
        }
    }
    let roots = roots::list(lib);
    let state = read_state(lib, &id);
    let conn = ix.conn();
    let (mut done, mut failed) = (state.done.len(), vec![]);
    let mut failed_games: HashSet<String> = HashSet::new();
    let mut stopped = false;
    let mut journal = Log::open(lib, &id)?;
    for (i, op) in plan.ops.iter().enumerate() {
        if state.done.contains_key(&i) {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        progress(json!({ "item": i + 1, "items": plan.ops.len(), "name": if op.game.is_empty() { op.to.clone() } else { op.game.clone() } }));
        if !op.game.is_empty() && failed_games.contains(&op.game) {
            continue;
        }
        // a cue sheet rewritten under its own name is set aside first, and read from there
        let mut early = vec![];
        let mut step = op.clone();
        let replaces = (op.op == "cue" || op.op == "text") && op.from.first().is_some_and(|f| key(Path::new(&f.path)) == key(Path::new(&op.to)));
        if replaces && Path::new(&op.to).exists() {
            match set_aside(lib, &roots, &id, &op.from[0]) {
                Ok(held) => {
                    early.push(json!([op.from[0].path, held]));
                    step.from[0].path = held;
                }
                Err(e) => {
                    failed.push(json!({ "game": op.game, "to": op.to, "error": format!("{e:#}") }));
                    continue;
                }
            }
        }
        let result = perform(&step, cancel).and_then(|(written, copied)| {
            // originals go only once what replaced them is checked
            let mut aside: Vec<Src> = op.aside.clone();
            if (op.op == "move" && copied) || op.op == "aside" {
                aside.push(op.from[0].clone());
            }
            let mut went = early.clone();
            for s in &aside {
                if Path::new(&s.path).exists() {
                    went.push(json!([s.path, set_aside(lib, &roots, &id, s)?]));
                }
            }
            let tx = conn.unchecked_transaction()?;
            update_index(&tx, &roots, op, written.as_ref(), &op.aside)?;
            tx.commit()?;
            let important = copied || !went.is_empty();
            journal.write(&json!({ "op": i, "copied": copied, "aside": went }), important)
        });
        if result.is_err() {
            // the sheet set aside early goes back
            for pair in &early {
                let _ = std::fs::rename(pair[1].as_str().unwrap_or(""), pair[0].as_str().unwrap_or(""));
            }
        }
        match result {
            Ok(()) => done += 1,
            Err(e) if cancel.load(Ordering::Relaxed) && format!("{e:#}").contains("stopped") => {
                stopped = true;
                break;
            }
            Err(e) => {
                if !op.game.is_empty() {
                    failed_games.insert(op.game.clone());
                }
                failed.push(json!({ "game": op.game, "to": op.to, "error": format!("{e:#}") }));
            }
        }
    }
    journal.sync()?;
    drop(journal);
    if !stopped {
        log(lib, &id, &json!({ "state": "done", "failed": failed.len() }))?;
        prune(lib);
    }
    if plan.export.is_some() {
        crate::export::record(lib, plan, false)?;
    }
    audit::match_files(conn)?;
    audit::changed(conn)?;
    Ok(json!({ "id": id, "label": plan.label, "done": done, "ops": plan.ops.len(), "failed": failed, "stopped": stopped }))
}

/// Finish a change that was interrupted.
pub fn finish(ix: &Index, lib: &Library, id: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    let plan = load(lib, id)?;
    let st = read_state(lib, id);
    if st.finished || st.undone {
        bail!("That change isn't unfinished.");
    }
    run(ix, lib, &plan, cancel, progress)
}

/// Undo a change: newest step first, renames renamed back, files it wrote removed,
/// originals it set aside put back. Only the newest change still done can be undone.
pub fn undo(ix: &Index, lib: &Library, id: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    let hist = history(lib, &[])?;
    let entry = hist.as_array().into_iter().flatten().find(|h| h["id"] == id).cloned().ok_or_else(|| anyhow!("There's no change {id} in the history."))?;
    if entry["can_undo"] != true {
        bail!("{} can't be undone: undo the changes after it first, or its set-aside files were emptied.", entry["label"].as_str().unwrap_or(id));
    }
    let plan = load(lib, id)?;
    let st = read_state(lib, id);
    let roots = roots::list(lib);
    let conn = ix.conn();
    let mut steps: Vec<usize> = st.done.keys().copied().collect();
    steps.sort_unstable();
    steps.reverse();
    let (mut undone, mut problems) = (0, vec![]);
    let mut made: HashSet<PathBuf> = HashSet::new();
    for (n, i) in steps.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let op = &plan.ops[*i];
        let rec = &st.done[i];
        progress(json!({ "item": n + 1, "items": steps.len(), "name": if op.game.is_empty() { op.to.clone() } else { op.game.clone() } }));
        let to = Path::new(&op.to);
        let r = (|| -> Result<()> {
            let copied = rec["copied"] == true;
            match op.op.as_str() {
                "rename" | "move" if !copied => {
                    let from = Path::new(&op.from[0].path);
                    if to.exists() && !from.exists() {
                        make_parent(from)?;
                        rename(to, from)?;
                        if let Some((root, rel)) = in_root(&roots, to) {
                            let back = in_root(&roots, from);
                            let row: Option<i64> = conn.query_row("SELECT id FROM files WHERE root_id = ?1 AND rel_path = ?2 AND parent_id IS NULL", params![root.id, rel], |r| r.get(0)).optional()?;
                            if let (Some(fid), Some((broot, brel))) = (row, back) {
                                let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                conn.execute("UPDATE files SET root_id = ?2, rel_path = ?3 || substr(rel_path, ?4) WHERE parent_id = ?1", params![fid, broot.id, brel, rel.chars().count() as i64 + 1])?;
                                conn.execute("UPDATE files SET root_id = ?2, rel_path = ?3, name = ?4 WHERE id = ?1", params![fid, broot.id, brel, name])?;
                            }
                        }
                    } else if !from.exists() {
                        bail!("{} isn't where the change put it", to.display());
                    }
                }
                "aside" => {}
                _ => {
                    // a file the change wrote
                    if to.exists() {
                        std::fs::remove_file(to).with_context(|| format!("couldn't remove {}", to.display()))?;
                        if let Some((root, rel)) = in_root(&roots, to) {
                            conn.execute("DELETE FROM files WHERE root_id = ?1 AND rel_path = ?2 AND parent_id IS NULL", params![root.id, rel])?;
                        }
                        if let Some(dir) = to.parent() {
                            made.insert(dir.to_path_buf());
                        }
                    }
                }
            }
            for pair in rec["aside"].as_array().into_iter().flatten() {
                let (orig, held) = (pair[0].as_str().unwrap_or(""), pair[1].as_str().unwrap_or(""));
                if Path::new(held).exists() && !Path::new(orig).exists() {
                    make_parent(Path::new(orig))?;
                    std::fs::rename(held, orig).with_context(|| format!("couldn't put {orig} back"))?;
                } else if !Path::new(orig).exists() {
                    bail!("{orig} can't be put back: its set-aside copy is gone");
                }
            }
            Ok(())
        })();
        match r {
            Ok(()) => undone += 1,
            Err(e) => problems.push(json!({ "game": op.game, "to": op.to, "error": format!("{e:#}") })),
        }
    }
    // folders the change made and left empty go, up to its output folder
    let top = plan.out.as_ref().map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = made.into_iter().collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let mut at = Some(d.as_path());
        while let Some(dir) = at {
            if top.as_ref().is_some_and(|t| !key(dir).starts_with(&key(t))) || std::fs::remove_dir(dir).is_err() {
                break;
            }
            at = dir.parent();
        }
    }
    // and the folder with games it added, if it's empty again
    if let Some(dir) = plan.new_root.as_ref().map(PathBuf::from) {
        if !dir.exists() || std::fs::read_dir(&dir).is_ok_and(|mut rd| rd.next().is_none()) {
            let _ = std::fs::remove_dir(&dir);
            if let Some(r) = roots::list(lib).into_iter().find(|r| key(&r.dir) == key(&dir)) {
                roots::remove(lib, &r.id)?;
                conn.execute("DELETE FROM files WHERE root_id = ?1", [&r.id])?;
                conn.execute("DELETE FROM roots WHERE id = ?1", [&r.id])?;
            }
        }
    }
    for d in aside_dirs(lib, id) {
        remove_empty_tree(&d);
    }
    if !cancel.load(Ordering::Relaxed) {
        log(lib, id, &json!({ "state": "undone", "problems": problems.len() }))?;
    }
    if plan.export.is_some() {
        crate::export::record(lib, &plan, true)?;
    }
    audit::match_files(conn)?;
    audit::changed(conn)?;
    Ok(json!({ "id": id, "label": plan.label, "undone": undone, "problems": problems }))
}

fn remove_empty_tree(d: &Path) {
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_tree(&e.path());
        }
    }
    let _ = std::fs::remove_dir(d);
    if let Some(parent) = d.parent().filter(|p| p.file_name().is_some_and(|n| n == SET_ASIDE)) {
        let _ = std::fs::remove_dir(parent);
    }
}

/// Delete what a change set aside (or every change's, with `None`). The only step that deletes.
pub fn empty_set_aside(lib: &Library, id: Option<&str>) -> Result<Value> {
    lib.writable()?;
    let ids: Vec<String> = match id {
        Some(i) => vec![load(lib, i)?.id],
        None => history(lib, &[])?.as_array().into_iter().flatten().filter(|h| h["held"].as_u64().unwrap_or(0) > 0).filter_map(|h| h["id"].as_str().map(String::from)).collect(),
    };
    let mut freed = 0;
    for i in &ids {
        for d in aside_dirs(lib, i) {
            freed += dir_size(&d);
            std::fs::remove_dir_all(&d).with_context(|| format!("couldn't empty {}", d.display()))?;
            if let Some(parent) = d.parent() {
                let _ = std::fs::remove_dir(parent);
            }
        }
        log(lib, i, &json!({ "state": "emptied" }))?;
    }
    prune(lib);
    Ok(json!({ "freed": freed, "changes": ids.len() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{fixture, import, names, run_scan, Fixture};

    /// Every file under `dir` with its SHA1 (hidden folders and files included).
    fn tree(dir: &Path) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let bytes = std::fs::read(&p).unwrap();
                    let rel = p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                    out.insert(rel, hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes)));
                }
            }
        }
        out
    }

    fn opts(dat: i64, mode: &str, format: &str) -> Options {
        Options { dat_id: dat, mode: mode.into(), out: None, format: format.into(), partial: false, always_copy: false, only_misnamed: false, games: None, ..Default::default() }
    }

    fn go(f: &Fixture, plan: &Plan) -> Value {
        run(&f.ix, &f.lib, plan, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    fn back(f: &Fixture, id: &str) -> Value {
        undo(&f.ix, &f.lib, id, &AtomicBool::new(false), &mut |_| {}).unwrap()
    }

    fn setup(name: &str) -> (Fixture, i64) {
        let f = fixture(name);
        run_scan(&f, false);
        let dat = import(&f, "gb.dat")[0]["id"].as_i64().unwrap();
        (f, dat)
    }

    #[test]
    fn names_are_safe_and_follow_the_template() {
        assert_eq!(safe_name("A: B? <C>."), "A_ B_ _C_");
        assert_eq!(safe_name("CON.gb"), "_CON.gb");
        assert_eq!(safe_name("Com1"), "_Com1");
        assert_eq!(safe_name("Comet"), "Comet");
        let g = GameFields { name: "Alpha (USA) (Rev 1)".into(), title: Some("Alpha".into()), regions: Some("USA".into()), languages: None, revision: Some("1".into()) };
        assert_eq!(render("{name}", &g), "Alpha (USA) (Rev 1)");
        assert_eq!(render("{title} ({region}) ({languages}) ({revision})", &g), "Alpha (USA) (Rev 1)");
        assert!(check_template("{title} [{nope}]").is_err());
        assert!(check_template("{title} ({region})").is_ok());
    }

    #[test]
    fn rename_where_they_are_then_undo() {
        let (f, dat) = setup("organise-rename");
        let before = tree(&f.dir.join("Roms"));
        let plan = plan(&f.ix, &f.lib, &opts(dat, "rename", "keep")).unwrap();
        // Alpha and Delta are named right already; beta.gb becomes Beta (Europe).gb
        assert_eq!((plan.games, plan.in_place, plan.ops.len()), (3, 2, 1), "{:#?}", plan.ops);
        assert_eq!(plan.ops[0].op, "rename");
        assert!(plan.ops[0].to.ends_with("Beta (Europe).gb"));
        assert_eq!(plan.duplicates.len(), 1, "Alpha copy.gb is a duplicate");
        let r = go(&f, &plan);
        assert_eq!(r["done"], 1, "{r}");
        assert!(f.gb.join("Beta (Europe).gb").is_file() && !f.gb.join("beta.gb").exists());
        // the index followed: no rescan needed, and a rescan reads nothing again
        assert_eq!(names(&f, dat, "misnamed"), ["Alpha copy.gb"]);
        let s = run_scan(&f, false);
        assert_eq!((s["added"].as_i64(), s["hashed"].as_i64(), s["removed"].as_i64()), (Some(0), Some(0), Some(0)), "{s}");
        // undo puts it back as it was
        let u = back(&f, &plan.id);
        assert_eq!(u["undone"], 1, "{u}");
        assert_eq!(tree(&f.dir.join("Roms")), before);
        assert_eq!(names(&f, dat, "misnamed"), ["Alpha copy.gb", "beta.gb"]);
        let s = run_scan(&f, false);
        assert_eq!(s["hashed"], 0, "{s}");
        let h = history(&f.lib, &[]).unwrap();
        assert_eq!(h[0]["state"], "undone");
        assert_eq!(h[0]["can_undo"], false);
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    /// The made-up CD as a renamed cue/bin set, and as a CHD named wrong, with a DAT for it.
    fn disc_setup(name: &str) -> (Fixture, i64, PathBuf) {
        let f = fixture(name);
        let src = crate::image::tests::sources(&format!("{name}-src"));
        let read = |n: &str| std::fs::read(src.join(n)).unwrap();
        let (cue_bytes, t1, t2) = (read("Disc Game (USA).cue"), read("Disc Game (USA) (Track 1).bin"), read("Disc Game (USA) (Track 2).bin"));
        let psx = f.dir.join("Roms/PSX");
        crate::audit::tests::write(&psx.join("Mine/t1.bin"), &t1);
        crate::audit::tests::write(&psx.join("Mine/t2.bin"), &t2);
        crate::audit::tests::write(&psx.join("Mine/mine.cue"), b"FILE \"t1.bin\" BINARY\r\n  TRACK 01 MODE1/2352\r\n    INDEX 01 00:00:00\r\nFILE \"t2.bin\" BINARY\r\n  TRACK 02 AUDIO\r\n    INDEX 00 00:00:00\r\n    INDEX 01 00:00:10\r\n");
        let x = crate::audit::tests::rom_xml;
        let dat = crate::audit::tests::dat_xml("Sony - PlayStation", "2026", &[
            ("Disc Game (USA)", None, x("Disc Game (USA).cue", &cue_bytes) + &x("Disc Game (USA) (Track 1).bin", &t1) + &x("Disc Game (USA) (Track 2).bin", &t2)),
        ]);
        crate::audit::tests::write(&f.dir.join("dats/psx.dat"), dat.as_bytes());
        run_scan(&f, false);
        let id = import(&f, "psx.dat")[0]["id"].as_i64().unwrap();
        let _ = std::fs::remove_dir_all(&src);
        (f, id, psx)
    }

    #[test]
    fn a_renamed_disc_set_and_its_cue_sheet() {
        let (f, dat, psx) = disc_setup("organise-disc");
        let before = tree(&f.dir.join("Roms"));
        assert!(plan(&f.ix, &f.lib, &opts(dat, "rename", "zip")).is_err(), "discs keep their format");
        let p = plan(&f.ix, &f.lib, &opts(dat, "rename", "keep")).unwrap();
        let mut ops: Vec<(&str, String)> = p.ops.iter().map(|o| (o.op.as_str(), Path::new(&o.to).file_name().unwrap().to_string_lossy().into_owned())).collect();
        ops.sort();
        assert_eq!(ops, [("cue", "Disc Game (USA).cue".into()), ("rename", "Disc Game (USA) (Track 1).bin".into()), ("rename", "Disc Game (USA) (Track 2).bin".into())]);
        let r = go(&f, &p);
        assert_eq!(r["done"], 3, "{r}");
        // the sheet names the tracks' new names, and is now Redump's own (an exact match)
        let made = std::fs::read_to_string(psx.join("Mine/Disc Game (USA).cue")).unwrap();
        assert!(made.contains("FILE \"Disc Game (USA) (Track 2).bin\" BINARY\r\n"), "{made}");
        assert!(!psx.join("Mine/mine.cue").exists());
        assert_eq!(names(&f, dat, "misnamed"), Vec::<String>::new());
        assert_eq!(names(&f, dat, "matched").len(), 3);
        let method: String = f.ix.conn().query_row("SELECT m.method FROM matches m JOIN files x ON x.id = m.file_id WHERE x.name = 'Disc Game (USA).cue'", [], |r| r.get(0)).unwrap();
        assert!(method.starts_with("sha1"), "{method}");
        // the same in place: a sheet named right whose FILE lines aren't
        back(&f, &p.id);
        assert_eq!(tree(&f.dir.join("Roms")), before);
        std::fs::rename(psx.join("Mine/mine.cue"), psx.join("Mine/Disc Game (USA).cue")).unwrap();
        run_scan(&f, false);
        let p = plan(&f.ix, &f.lib, &opts(dat, "rename", "keep")).unwrap();
        assert_eq!(p.ops.iter().filter(|o| o.op == "cue").count(), 1);
        let r = go(&f, &p);
        assert_eq!(r["done"], 3, "{r}");
        assert!(std::fs::read_to_string(psx.join("Mine/Disc Game (USA).cue")).unwrap().contains("Disc Game (USA) (Track 1).bin"));
        let u = back(&f, &p.id);
        assert_eq!(u["undone"], 3, "{u}");
        assert!(std::fs::read_to_string(psx.join("Mine/Disc Game (USA).cue")).unwrap().contains("\"t1.bin\""));
        // as a CHD named wrong: renamed whole, with no cue (the CHD needs none)
        std::fs::remove_dir_all(psx.join("Mine")).unwrap();
        std::fs::copy(crate::image::tests::fixtures().join("Disc Game (USA).chd"), psx.join("disc.chd")).unwrap();
        run_scan(&f, false);
        assert_eq!(names(&f, dat, "misnamed"), ["Track 01.bin", "Track 02.bin"]);
        let p = plan(&f.ix, &f.lib, &opts(dat, "rename", "keep")).unwrap();
        assert_eq!(p.ops.len(), 1, "{:#?}", p.ops);
        assert_eq!((p.ops[0].op.as_str(), Path::new(&p.ops[0].to).file_name().unwrap().to_str()), ("rename", Some("Disc Game (USA).chd")));
        go(&f, &p);
        assert_eq!(names(&f, dat, "matched"), ["Track 01.bin", "Track 02.bin"]);
        let s = audit::summary(f.ix.conn(), dat).unwrap();
        assert_eq!(s["have"], 1, "{s}");
        // and copied into a sorted folder
        let mut o = opts(dat, "copy", "keep");
        o.out = Some(f.dir.join("Sorted"));
        o.always_copy = true;
        let p = plan(&f.ix, &f.lib, &o).unwrap();
        go(&f, &p);
        assert!(f.dir.join("Sorted/Disc Game (USA).chd").is_file());
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn copy_into_torrentzips_leaves_the_originals() {
        let (f, dat) = setup("organise-copy");
        let before = tree(&f.dir.join("Roms"));
        let out = f.dir.join("Sorted");
        let mut o = opts(dat, "copy", "zip");
        o.out = Some(out.clone());
        let plan = plan(&f.ix, &f.lib, &o).unwrap();
        assert_eq!(plan.ops.iter().map(|o| o.op.as_str()).collect::<Vec<_>>(), ["zip", "zip", "zip"], "{:#?}", plan.ops);
        let r = go(&f, &plan);
        assert_eq!(r["done"], 3, "{r}");
        for g in ["Alpha (USA)", "Beta (Europe)", "Delta (World)"] {
            assert!(torrentzip::is_torrentzip(&out.join(format!("{g}.zip"))), "{g}");
        }
        assert_eq!(tree(&f.dir.join("Roms")), before, "nothing in the folders with games changed");
        // planned again: it's all there
        let again = super::plan(&f.ix, &f.lib, &o).unwrap();
        assert_eq!((again.ops.len(), again.in_place, again.conflicts.len()), (0, 3, 0), "{:#?}", again.ops);
        // a different file where a game would go is a conflict, and that game is left
        std::fs::write(out.join("Beta (Europe).zip"), b"not this").unwrap();
        let clash = super::plan(&f.ix, &f.lib, &o).unwrap();
        assert_eq!(clash.conflicts.len(), 1);
        std::fs::remove_file(out.join("Beta (Europe).zip")).unwrap();
        back(&f, &plan.id);
        assert!(!out.exists(), "undo removes what it wrote, and the folder it made");
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn move_into_the_library_loose_and_undo() {
        let (f, dat) = setup("organise-move");
        let before = tree(&f.dir.join("Roms"));
        let mut o = opts(dat, "move", "loose");
        o.partial = true;
        let plan = plan(&f.ix, &f.lib, &o).unwrap();
        let target = f.lib.root().join("Nintendo - Game Boy");
        assert_eq!(plan.new_root.as_deref(), Some(target.to_string_lossy().as_ref()));
        let ops: Vec<(&str, String)> = plan.ops.iter().map(|o| (o.op.as_str(), Path::new(&o.to).strip_prefix(&target).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or(o.to.clone()))).collect();
        assert!(ops.contains(&("move", "Alpha (USA).gb".into())), "{ops:?}");
        assert!(ops.contains(&("move", "Beta (Europe).gb".into())), "{ops:?}");
        assert!(ops.contains(&("extract", "Delta (World).gb".into())), "{ops:?}");
        // beta.gb is also Epsilon's second track: copied from where Beta went
        assert!(ops.contains(&("copy", "Epsilon (USA)/Epsilon (USA) (Track 2).gb".into())), "{ops:?}");
        assert!(plan.ops.iter().any(|o| o.op == "aside" && o.to.ends_with("Delta (World).zip")), "the emptied ZIP is set aside");
        let r = go(&f, &plan);
        assert_eq!(r["failed"].as_array().unwrap().len(), 0, "{r}");
        assert!(target.join("Epsilon (USA)/Epsilon (USA) (Track 2).gb").is_file());
        assert!(f.dir.join("Roms").join(SET_ASIDE).join(&plan.id).join("Game Boy/Delta (World).zip").is_file());
        assert!(roots::list(&f.lib).iter().any(|r| r.dir == target), "the library's platform folder is now a folder with games");
        assert_eq!(names(&f, dat, "have"), ["Alpha (USA)", "Beta (Europe)", "Delta (World)"]);
        assert_eq!(names(&f, dat, "misnamed"), ["Alpha copy.gb"]);
        let s = run_scan(&f, false);
        assert_eq!((s["added"].as_i64(), s["hashed"].as_i64()), (Some(0), Some(0)), "the index was kept up to date: {s}");
        let h = history(&f.lib, &[]).unwrap();
        assert!(h[0]["held"].as_u64().unwrap() > 0);
        back(&f, &plan.id);
        assert_eq!(tree(&f.dir.join("Roms")), before);
        assert!(!target.exists() && !roots::list(&f.lib).iter().any(|r| r.dir == target), "the folder it added goes again");
        run_scan(&f, false);
        assert_eq!(names(&f, dat, "misnamed"), ["Alpha copy.gb", "beta.gb"]);
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn duplicates_set_aside_emptied_and_an_interrupted_change() {
        let (f, dat) = setup("organise-dupes");
        let plan = plan_duplicates(&f.ix, &f.lib, dat).unwrap();
        assert_eq!(plan.ops.len(), 1);
        assert!(plan.ops[0].to.ends_with("Alpha copy.gb"));
        go(&f, &plan);
        assert!(!f.gb.join("Copies/Alpha copy.gb").exists());
        assert_eq!(names(&f, dat, "duplicates"), Vec::<String>::new());
        back(&f, &plan.id);
        assert!(f.gb.join("Copies/Alpha copy.gb").is_file());
        // set aside again, then emptied: the only step that deletes, and Undo is gone
        let again = plan_duplicates(&f.ix, &f.lib, dat).unwrap();
        run_scan(&f, false);
        let again = if again.ops.is_empty() { plan_duplicates(&f.ix, &f.lib, dat).unwrap() } else { again };
        go(&f, &again);
        let e = empty_set_aside(&f.lib, Some(&again.id)).unwrap();
        assert_eq!(e["freed"], 32768);
        let h = history(&f.lib, &[]).unwrap();
        assert_eq!((h[0]["held"].as_u64(), h[0]["can_undo"].as_bool()), (Some(0), Some(false)), "{h}");
        assert!(undo(&f.ix, &f.lib, &again.id, &AtomicBool::new(false), &mut |_| {}).is_err());

        // a change stopped half-way is interrupted, and can be finished
        let out = f.dir.join("Sorted");
        let mut o = opts(dat, "copy", "keep");
        o.out = Some(out.clone());
        o.always_copy = true;
        let p = super::plan(&f.ix, &f.lib, &o).unwrap();
        assert_eq!(p.ops.len(), 3);
        let stop = AtomicBool::new(false);
        let r = run(&f.ix, &f.lib, &p, &stop, &mut |v| {
            if v["item"] == 2 {
                stop.store(true, Ordering::Relaxed);
            }
        })
        .unwrap();
        assert_eq!(r["stopped"], true);
        let h = history(&f.lib, &[]).unwrap();
        assert_eq!((h[0]["state"].as_str(), h[0]["can_undo"].as_bool()), (Some("interrupted"), Some(true)));
        let r = finish(&f.ix, &f.lib, &p.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["done"], 3, "{r}");
        assert_eq!(history(&f.lib, &[]).unwrap()[0]["state"], "done");
        assert!(out.join("Delta (World).zip").is_file() && out.join("Beta (Europe).gb").is_file());
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    // The performance targets of the Phase 2 design, on generated data. Run with
    // `cargo test --release -- --ignored timing` (CI does, on Windows).
    #[test]
    #[ignore]
    fn timing_plan_of_50k_files_and_renaming_10k() {
        let dir = crate::library::tests::temp_dir("timing-organise");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let mut dat = String::from("<?xml version=\"1.0\"?>\n<datafile><header><name>Nintendo - Game Boy</name><version>1</version></header>\n");
        for d in 0..100u32 {
            let folder = dir.join(format!("Roms/Game Boy/Folder {d}"));
            std::fs::create_dir_all(&folder).unwrap();
            for i in 0..500u32 {
                let n = d * 500 + i;
                let bytes = (n + 1_000_000).to_le_bytes();
                std::fs::write(folder.join(format!("x{n}.gb")), bytes).unwrap();
                let h = hash::hash_reader(&mut &bytes[..], 4, "x.gb", &AtomicBool::new(false)).unwrap();
                dat.push_str(&format!(
                    "<game name=\"Game {n} (USA)\"><description>Game {n}</description><rom name=\"Game {n} (USA).gb\" size=\"4\" crc=\"{}\" sha1=\"{}\"/></game>\n",
                    h.crc32, h.sha1.unwrap()
                ));
            }
        }
        dat.push_str("</datafile>\n");
        std::fs::write(dir.join("big.dat"), &dat).unwrap();
        roots::add(&lib, &dir.join("Roms"), None).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options { full: false, readers: 4, only: None }, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let r = crate::dat::import(&ix, &lib, &dir.join("big.dat"), &serde_json::Map::new()).unwrap();
        let crate::dat::Imported::Loaded(v) = &r[0] else { panic!() };
        let dat_id = v["id"].as_i64().unwrap();
        let o = Options { dat_id, mode: "rename".into(), out: None, format: "keep".into(), partial: false, always_copy: false, only_misnamed: false, games: None, ..Default::default() };
        let t = std::time::Instant::now();
        let mut plan = plan(&ix, &lib, &o).unwrap();
        let planned = t.elapsed();
        assert_eq!(plan.ops.len(), 50_000);
        plan.ops.truncate(10_000);
        let t = std::time::Instant::now();
        let r = run(&ix, &lib, &plan, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let renamed = t.elapsed();
        eprintln!("plan of 50,000 files {planned:?}; renaming 10,000 {renamed:?}");
        assert_eq!(r["done"], 10_000);
        assert!(planned.as_secs_f64() < 2.0, "planning took {planned:?}");
        assert!(renamed.as_secs_f64() < 30.0, "renaming took {renamed:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
