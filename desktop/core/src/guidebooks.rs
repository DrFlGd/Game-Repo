//! Guidebooks (docs/PLAN.md, "Maps and Guides notes, part 10", D232–D240): published guides
//! you have as one file, a PDF, a CBZ or a CBR, kept in the library beside a game's packs, as
//! the Atlas pack contract (block A8) lays them out:
//!
//! ```text
//! Maps and Guides/
//!   <Game title>/
//!     <Game title> - <Source>.atlaspack
//!     Guidebooks/
//!       Official Guide.pdf
//!       .text/Official Guide.pdf.json     (its text, read page by page as magazines' is)
//!   guidebooks.json                       (Game Repo's record: titles, a folder's game)
//! ```
//!
//! A title folder's game is, in order: the game `guidebooks.json` gives it (an import, or
//! *Link to a game…*), the games of the packs `packs.json` keeps in that folder, else the
//! games you have whose title is the folder's word for word (D218's sure match). The
//! guidebooks themselves are plain files, one copy: imported (copied, linked or moved in,
//! with Undo), renamed (their title, in the record), set aside with Undo, and their text read
//! with the magazines' reader ([`crate::magtext::read`], kept in a `.text/` folder beside
//! them as an issue's is).
//!
//! They travel to Atlas in a pack of their own, one for each game: format 8, written by
//! Atlas's own crate (`write_guidebook_pack`, `rewrite_guidebook_pack`), packId
//! `guidebooks-<game id>` ([`pack_id_for`]). Game Repo Server makes it when a device asks for
//! it, into a cache of its own ([`Cache`], in the server's data folder, never the library),
//! keeps it, and makes it again only when the guidebooks, their titles or their text change,
//! or when another set of them is asked for; a CBR is turned into a CBZ on the way, since
//! Atlas never opens CBR.

use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use crate::library::{now, Library};
use crate::maps::{self, PackGame};
use crate::organise::{self, Op, Plan};
use anyhow::{anyhow, bail, Context, Result};
use atlas_pack::{GuidebookPack, NewGuidebook, MAX_GUIDEBOOKS, MAX_GUIDEBOOK_BYTES, MAX_GUIDEBOOK_PAGES, MAX_GUIDEBOOK_TEXT_BYTES, MAX_GUIDEBOOK_TITLE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

/// The files a guidebook can be (Atlas reads PDF and CBZ; a CBR is made a CBZ for it).
pub const EXTS: &[&str] = &["pdf", "cbz", "cbr"];
/// Game Repo's record of the guidebooks, in `Maps and Guides/`.
pub const RECORD: &str = "guidebooks.json";
/// The start of every guidebook pack's id (the contract: `guidebooks-<game id>`).
pub const PACK_PREFIX: &str = "guidebooks-";
/// What the packs Game Repo writes say wrote them (`pack.writtenBy`, `exportedBy`): Atlas
/// changes only packs whose writer is "Atlas", so it never changes these.
pub const WRITER: &str = "Game Repo";

// ------------------------------------------------------------ the record

/// What `guidebooks.json` keeps of a guidebook, by its path below `Maps and Guides/`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Kept {
    /// Its title as shown (absent: its file's name without the extension)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub added: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// What `guidebooks.json` keeps of a title folder: the games its guidebooks are for.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct FolderKept {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub games: Vec<PackGame>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `guidebooks.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Record {
    pub guidebooks: BTreeMap<String, Kept>,
    pub folders: BTreeMap<String, FolderKept>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

fn record_path(lib: &Library) -> PathBuf {
    maps::folder(lib).join(RECORD)
}

pub fn read_record(lib: &Library) -> Record {
    serde_json::from_value(read_json_object(&record_path(lib))).unwrap_or_default()
}

fn record_text(r: &Record) -> Result<String> {
    Ok(serde_json::to_string_pretty(&sorted(&serde_json::to_value(r)?))? + "\n")
}

fn write_record(lib: &Library, r: &Record) -> Result<()> {
    lib.writable()?;
    std::fs::create_dir_all(maps::folder(lib))?;
    crate::config::write_json(&record_path(lib), &serde_json::to_value(r)?)
}

// ------------------------------------------------------------ the guidebooks in the folder

/// A guidebook file in the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Guidebook {
    /// Its path below `Maps and Guides/`, with `/`: `<folder>/Guidebooks/<file>`
    pub rel: String,
    /// Its title folder's name
    pub folder: String,
    /// Its file's name, which is its id in a game's set (what a device unticks)
    pub file: String,
    pub title: String,
    /// pdf | cbz | cbr, by its name
    pub kind: String,
    pub size: u64,
    pub mtime: i64,
    pub path: PathBuf,
}

fn mtime_of(m: &std::fs::Metadata) -> i64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A file's name says it can be a guidebook: `.pdf`, `.cbz` or `.cbr`, not hidden.
pub fn is_guidebook_name(name: &str) -> bool {
    !name.starts_with('.') && EXTS.contains(&crate::platforms::ext_of(name).as_str())
}

/// Its title by its file's name: the name without its extension.
fn title_of_file(file: &str) -> String {
    match file.rsplit_once('.') {
        Some((stem, _)) if !stem.trim().is_empty() => stem.trim().to_string(),
        _ => file.to_string(),
    }
}

/// Every guidebook in the folder: the files in each title folder's `Guidebooks/` (not hidden,
/// not one still being written), in order of folder and file.
pub fn list(lib: &Library) -> Vec<Guidebook> {
    let base = maps::folder(lib);
    let rec = read_record(lib);
    let mut out = vec![];
    for t in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let folder = t.file_name().to_string_lossy().into_owned();
        if folder.starts_with('.') || !t.file_type().is_ok_and(|f| f.is_dir()) {
            continue;
        }
        let Some(gb) = std::fs::read_dir(t.path()).into_iter().flatten().flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(maps::GUIDEBOOKS) && e.file_type().is_ok_and(|f| f.is_dir())) else { continue };
        let gb_name = gb.file_name().to_string_lossy().into_owned();
        for e in std::fs::read_dir(gb.path()).into_iter().flatten().flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if !is_guidebook_name(&file) || !e.file_type().is_ok_and(|f| f.is_file()) {
                continue;
            }
            let Ok(m) = e.metadata() else { continue };
            let rel = format!("{folder}/{gb_name}/{file}");
            let title = rec.guidebooks.get(&rel).and_then(|k| k.title.clone()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| title_of_file(&file));
            out.push(Guidebook { kind: crate::platforms::ext_of(&file), size: m.len(), mtime: mtime_of(&m), path: e.path(), rel, folder: folder.clone(), file, title });
        }
    }
    out.sort_by_key(|g| (g.folder.to_lowercase(), g.file.to_lowercase()));
    out
}

/// A guidebook by its path below `Maps and Guides/`.
pub fn by_rel(lib: &Library, rel: &str) -> Result<Guidebook> {
    list(lib).into_iter().find(|g| g.rel == rel).ok_or_else(|| anyhow!("There's no guidebook {rel} in Maps and Guides."))
}

// ------------------------------------------------------------ pages

type Stamp = (PathBuf, u64, i64);

fn pages_cache() -> &'static Mutex<HashMap<Stamp, std::result::Result<u32, String>>> {
    static C: OnceLock<Mutex<HashMap<Stamp, std::result::Result<u32, String>>>> = OnceLock::new();
    C.get_or_init(Mutex::default)
}

/// How many pages a guidebook has: a PDF's page tree as Atlas's crate counts it (else as the
/// magazines' PDF reader does), a CBZ's or CBR's pictures. Remembered by the file's size and
/// time while the app runs. Err: a plain sentence.
pub fn pages(g: &Guidebook) -> std::result::Result<u32, String> {
    pages_of(&g.path, &g.kind, g.size, g.mtime)
}

/// How many pages a file of `kind` (pdf, cbz, cbr) has, as [`pages`] counts them, remembered by
/// the file's size and time while the app runs (a magazine's issue too, D245).
pub(crate) fn pages_of(path: &Path, kind: &str, size: u64, mtime: i64) -> std::result::Result<u32, String> {
    let key = (path.to_path_buf(), size, mtime);
    if let Some(r) = pages_cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return r.clone();
    }
    let r = count_pages(path, kind);
    pages_cache().lock().unwrap_or_else(|e| e.into_inner()).insert(key, r.clone());
    r
}

fn count_pages(path: &Path, kind: &str) -> std::result::Result<u32, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match kind {
        "pdf" => atlas_pack::guidebook_pages(path).map_err(String::from).or_else(|e| {
            let p = path.to_path_buf();
            std::panic::catch_unwind(move || lopdf::Document::load(&p).ok().map(|d| d.get_pages().len() as u32))
                .ok()
                .flatten()
                .filter(|n| *n > 0)
                .ok_or(e)
        }),
        _ => match crate::magazines::pages(path) {
            Ok(v) if v.is_empty() => Err(format!("{name} holds no pictures of pages.")),
            Ok(v) => Ok(v.len() as u32),
            Err(_) => Err(format!("{name} isn't a {} file that can be read.", kind.to_uppercase())),
        },
    }
}

// ------------------------------------------------------------ a folder's game

/// How a title folder's game was found: the record (an import or a link by hand), its packs,
/// or its title; none.
fn folder_games(ix: &Index, rec: &Record, packs: &maps::Record, folder: &str) -> Result<(Vec<PackGame>, &'static str)> {
    if let Some(g) = rec.folders.get(folder).filter(|f| !f.games.is_empty()) {
        return Ok((g.games.clone(), "record"));
    }
    let mut games: Vec<PackGame> = vec![];
    let prefix = format!("{}/", folder.to_lowercase());
    for k in packs.packs.values() {
        if k.file.as_deref().is_some_and(|f| f.to_lowercase().starts_with(&prefix)) {
            for g in &k.games {
                if !games.iter().any(|x| x.key == g.key) {
                    games.push(g.clone());
                }
            }
        }
    }
    if !games.is_empty() {
        return Ok((games, "packs"));
    }
    if folder.eq_ignore_ascii_case(maps::NO_GAME) {
        return Ok((vec![], ""));
    }
    let sure = maps::sure(&maps::matches(ix, folder, 8)?, None);
    let games: Vec<PackGame> = sure.into_iter().map(|m| PackGame { key: m.key, title: m.title, platform: m.platform }).collect();
    let how = if games.is_empty() { "" } else { "title" };
    Ok((games, how))
}

/// A game's guidebooks: a title folder, its games and its guidebooks.
#[derive(Clone, Debug)]
pub struct Folder {
    pub name: String,
    pub games: Vec<PackGame>,
    /// record | packs | title | "" (none)
    pub how: &'static str,
    pub guidebooks: Vec<Guidebook>,
}

impl Folder {
    /// The game the pack's id is made from: the first of its games (the one whose title names
    /// the folder, as a pack's first game does, D216).
    pub fn pack_id(&self) -> Option<String> {
        self.games.first().map(|g| pack_id_for(&g.key))
    }

    /// The game's title, as the pack gives it (`game.title`, which Atlas joins by).
    pub fn game_title(&self) -> String {
        self.games.first().map(|g| g.title.clone()).filter(|t| !t.trim().is_empty()).unwrap_or_else(|| self.name.clone())
    }
}

/// Every title folder with guidebooks, each with its games.
pub fn folders(ix: &Index, lib: &Library) -> Result<Vec<Folder>> {
    let rec = read_record(lib);
    let packs = maps::read_record(lib);
    let mut by: BTreeMap<String, Vec<Guidebook>> = BTreeMap::new();
    for g in list(lib) {
        by.entry(g.folder.clone()).or_default().push(g);
    }
    let mut out = vec![];
    for (name, guidebooks) in by {
        let (games, how) = folder_games(ix, &rec, &packs, &name)?;
        out.push(Folder { name, games, how, guidebooks });
    }
    Ok(out)
}

/// The folder whose guidebooks are a game's, by any of its Library keys (a game's discs).
pub fn folder_of(ix: &Index, lib: &Library, keys: &[String]) -> Result<Option<Folder>> {
    if list(lib).is_empty() {
        return Ok(None);
    }
    Ok(folders(ix, lib)?.into_iter().find(|f| f.games.iter().any(|g| keys.contains(&g.key))))
}

/// The pack id of a game's guidebooks (the contract: `guidebooks-<game id>`, the game id
/// Game Repo's own, stable for the game's life). A Library key (`<system>/<DAT name>`,
/// `pc/<id>`) has characters a pack id can't, so the id is the key's letters and digits in
/// lower case joined by `-`, cut to fit, and the first 8 hex digits of the key's SHA-1, so two
/// keys that read alike never share one: `guidebooks-snes-hollow-marsh-usa-1a2b3c4d`. Always
/// 1 to 64 characters that Atlas's `is_pack_id` takes.
pub fn pack_id_for(key: &str) -> String {
    slug_id(PACK_PREFIX, key)
}

/// `<prefix><slug>-<8 hex digits of the key's SHA-1>`: the key's letters and digits in lower
/// case joined by `-`, cut so the whole is at most 64 characters, as [`pack_id_for`] says.
pub(crate) fn slug_id(prefix: &str, key: &str) -> String {
    use sha1::Digest;
    let h = hex::encode(sha1::Sha1::digest(key.as_bytes()));
    let room = 64 - prefix.len() - 1 - 8;
    let mut slug = String::new();
    for w in key.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let w = w.to_ascii_lowercase();
        if slug.len() + w.len() + usize::from(!slug.is_empty()) > room {
            let left = room.saturating_sub(slug.len() + usize::from(!slug.is_empty()));
            if left >= 3 {
                if !slug.is_empty() {
                    slug.push('-');
                }
                slug.push_str(&w[..left]);
            }
            break;
        }
        if !slug.is_empty() {
            slug.push('-');
        }
        slug.push_str(&w);
    }
    if slug.is_empty() {
        format!("{prefix}{}", &h[..8])
    } else {
        format!("{prefix}{slug}-{}", &h[..8])
    }
}

/// Whether a pack id is one of Game Repo's guidebook packs.
pub fn is_pack_id(id: &str) -> bool {
    id.starts_with(PACK_PREFIX) && atlas_pack::is_pack_id(id)
}

// ------------------------------------------------------------ the page, and a game's

/// Why a guidebook can't go in a pack for Atlas (the contract's bounds), if it can't.
fn cannot_go(g: &Guidebook, pages: &std::result::Result<u32, String>) -> Option<String> {
    if g.size == 0 {
        return Some("It's empty, so it can't go to Atlas.".into());
    }
    if g.size > MAX_GUIDEBOOK_BYTES {
        return Some("It's larger than 16 GB, the most a guidebook for Atlas can be.".into());
    }
    match pages {
        Err(e) => Some(format!("{e} It can't go to Atlas.")),
        Ok(n) if *n > MAX_GUIDEBOOK_PAGES => Some("It has more than 100,000 pages, the most a guidebook for Atlas can have.".into()),
        _ => None,
    }
}

fn game_json(g: &PackGame) -> Value {
    json!({ "key": g.key, "title": g.title, "platform": g.platform, "platform_name": crate::browse::system_name(&g.platform) })
}

fn guidebook_json(lib: &Library, f: &Folder, g: &Guidebook) -> Value {
    let p = pages(g);
    let text = crate::magtext::kept(&g.path);
    json!({
        "id": format!("guidebook:{}", g.rel),
        "guidebook": true,
        "rel": g.rel,
        "path": lib.relative(&g.path),
        "folder": g.folder,
        "file": g.file,
        "title": g.title,
        "kind": g.kind,
        "size": g.size,
        "modified": g.mtime,
        "pages": p.as_ref().ok(),
        "pages_error": p.as_ref().err(),
        "cannot_go": cannot_go(g, &p),
        "text": text.map(|k| json!({ "read": k.read, "pages": k.pages.iter().filter(|t| !t.trim().is_empty()).count(), "recognised": k.recognised.len(), "unread": k.unread.len(), "note": k.note })),
        "games": f.games.iter().map(game_json).collect::<Vec<_>>(),
        "how": f.how,
        "pack_id": f.pack_id(),
        "game_title": f.game_title(),
    })
}

/// The guidebooks for the Maps and Guides page: every one with its pages, size, file type,
/// whether its text is read, its games, and why it can't go to Atlas if it can't.
pub fn overview(ix: &Index, lib: &Library) -> Result<Value> {
    let fs = folders(ix, lib)?;
    let items: Vec<Value> = fs.iter().flat_map(|f| f.guidebooks.iter().map(move |g| (f, g))).map(|(f, g)| guidebook_json(lib, f, g)).collect();
    Ok(json!({ "guidebooks": items }))
}

/// A game's guidebooks, for its page: by its key.
pub fn of_game(ix: &Index, lib: &Library, key: &str) -> Vec<Value> {
    match folder_of(ix, lib, &[key.to_string()]) {
        Ok(Some(f)) => f.guidebooks.iter().map(|g| guidebook_json(lib, &f, g)).collect(),
        _ => vec![],
    }
}

/// A guidebook's title changed (empty: its file's name again). Kept in the record.
pub fn set_title(lib: &Library, rel: &str, title: &str) -> Result<Value> {
    lib.writable()?;
    let g = by_rel(lib, rel)?;
    let t = title.trim();
    if t.chars().count() > MAX_GUIDEBOOK_TITLE {
        bail!("A guidebook's title can be at most {MAX_GUIDEBOOK_TITLE} characters.");
    }
    let mut r = read_record(lib);
    let k = r.guidebooks.entry(g.rel.clone()).or_insert_with(|| Kept { added: now(), ..Default::default() });
    k.title = (!t.is_empty() && t != title_of_file(&g.file)).then(|| t.to_string());
    write_record(lib, &r)?;
    Ok(json!({ "title": if t.is_empty() { title_of_file(&g.file) } else { t.to_string() } }))
}

/// A title folder's guidebooks linked to a game by hand (`key`), or (None) back to being found
/// by its packs or its title. Kept in the record; the folder isn't moved.
pub fn link(ix: &Index, lib: &Library, folder: &str, key: Option<&str>) -> Result<Value> {
    lib.writable()?;
    if folder.eq_ignore_ascii_case(maps::NO_GAME) {
        bail!("Guidebooks in No game yet can't be linked to a game: import them to their game.");
    }
    if !list(lib).iter().any(|g| g.folder == folder) {
        bail!("There are no guidebooks in {folder}.");
    }
    let mut r = read_record(lib);
    match key {
        Some(k) => r.folders.entry(folder.to_string()).or_default().games = vec![maps::game_of(ix, k)?],
        None => {
            r.folders.remove(folder);
        }
    }
    write_record(lib, &r)?;
    Ok(json!({ "ok": true }))
}

// ------------------------------------------------------------ importing and removing

/// The guidebook files in the paths picked: files themselves, and those in folders picked
/// (not inside them); files already in Maps and Guides are left out.
pub fn files_in(lib: &Library, paths: &[PathBuf]) -> Vec<PathBuf> {
    let base = organise::key(&maps::folder(lib));
    let mut out = vec![];
    for p in paths {
        let files: Vec<PathBuf> = if p.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).filter(|x| x.is_file()).collect();
            v.sort();
            v
        } else {
            vec![p.clone()]
        };
        for f in files {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if is_guidebook_name(&name) && !organise::key(&f).starts_with(&base) && !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// Plan bringing guidebook files into a game's folder (`key`, a Library key): into
/// `<its folder>/Guidebooks/` (the folder that has its guidebooks or packs already, else the
/// game's title as a pack's folder is named, D216), copied (hardlinked on the same drive
/// unless `always_copy`) or moved, each under its own name (a free one when taken), with
/// `guidebooks.json` written in the same change so Undo puts both back.
pub fn import_plan(ix: &Index, lib: &Library, paths: &[PathBuf], key: &str, mode: &str, always_copy: bool) -> Result<Plan> {
    lib.writable()?;
    let files = files_in(lib, paths);
    if files.is_empty() {
        bail!("Choose PDF, CBZ or CBR files to import as guidebooks.");
    }
    let game = maps::game_of(ix, key)?;
    let base = maps::folder(lib);
    let mut rec = read_record(lib);
    // the folder that's the game's already: its guidebooks', else its packs'
    let existing = folders(ix, lib)?.into_iter().find(|f| f.games.iter().any(|g| g.key == key)).map(|f| f.name);
    let packs = maps::read_record(lib);
    let of_packs = packs.packs.values().filter(|k| k.games.first().is_some_and(|g| g.key == key)).find_map(|k| k.file.as_deref().and_then(|f| f.split_once('/')).map(|(top, _)| top.to_string())).filter(|t| !t.eq_ignore_ascii_case(maps::NO_GAME));
    let folder = existing.or(of_packs).unwrap_or_else(|| maps::folder_for(std::slice::from_ref(&game)));
    let gb_dir = format!("{folder}/{}", maps::GUIDEBOOKS);
    let mut taken: HashSet<String> = HashSet::new();
    let mut ops = vec![];
    let added = now();
    for src in &files {
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name = maps::free_name(&base, &gb_dir, &name, &mut taken, None);
        let to = base.join(&gb_dir).join(&name);
        let size = std::fs::metadata(src).with_context(|| format!("There's no file {}.", src.display()))?.len();
        let op = match mode {
            "move" => "move",
            _ if !always_copy && organise::same_drive(src, &base) => "link",
            _ => "copy",
        };
        rec.guidebooks.insert(format!("{gb_dir}/{name}"), Kept { title: None, added: added.clone(), ..Default::default() });
        ops.push(Op { op: op.into(), game: game.title.clone(), to: to.display().to_string(), from: vec![organise::Src { size, ..outside(src) }], names: vec![], aside: vec![], size, note: None });
    }
    // the folder is the game's for sure once its guidebooks are imported to it
    let f = rec.folders.entry(folder.clone()).or_default();
    if !f.games.iter().any(|g| g.key == key) {
        f.games.insert(0, game.clone());
    }
    ops.push(text_op(&game.title, &record_path(lib), record_text(&rec)?, "the record of the guidebooks"));
    let n = files.len();
    let s = if n == 1 { "" } else { "s" };
    let mut plan = maps::plan_of(lib, "maps_guidebooks_import", if mode == "move" { "move" } else { "copy" }, format!("Import {n} guidebook{s} for {}", game.title), format!("Imported {n} guidebook{s} for {}", game.title), ops, n, json!({ "folder": folder, "key": key }));
    plan.space = organise::space_needed(&plan.ops);
    Ok(plan)
}

/// Plan setting guidebooks aside (by their paths below `Maps and Guides/`), with Undo. Their
/// text and record stay, so one put back is as it was.
pub fn remove_plan(lib: &Library, rels: &[String]) -> Result<Plan> {
    lib.writable()?;
    let all = list(lib);
    let mut ops = vec![];
    for rel in rels {
        crate::library::rel_inside(rel)?;
        let Some(g) = all.iter().find(|g| &g.rel == rel) else { bail!("There's no guidebook {rel} in Maps and Guides.") };
        let s = maps::folder_src(lib, rel);
        ops.push(Op { op: "aside".into(), game: g.title.clone(), to: s.path.clone(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: None });
    }
    if ops.is_empty() {
        bail!("Choose the guidebooks to remove.");
    }
    let (title, label) = match ops.as_slice() {
        [o] => (format!("Remove {}", o.game), format!("Removed {}", o.game)),
        _ => (format!("Remove {} guidebooks", ops.len()), format!("Removed {} guidebooks", ops.len())),
    };
    let n = ops.len();
    Ok(maps::plan_of(lib, "maps_guidebooks_remove", "aside", title, label, ops, n, Value::Null))
}

// ------------------------------------------------------------ their text

/// Read the guidebooks' text page by page, as the magazines' is ([`crate::magtext::read`]): a
/// PDF's own text, else text recognition; kept beside each in `.text/`. Only those not read
/// since their file changed (or every one, `again`), or those in `rels`.
pub fn read_texts(lib: &Library, rels: Option<&[String]>, again: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    let reader = crate::magtext::reader();
    let todo: Vec<Guidebook> = list(lib)
        .into_iter()
        .filter(|g| rels.is_none_or(|r| r.contains(&g.rel)))
        .filter(|g| {
            again
                || match crate::magtext::kept(&g.path) {
                    None => true,
                    Some(k) => !k.unread.is_empty() && k.reader.is_none() && reader.is_some(),
                }
        })
        .collect();
    let (mut read, mut pages, mut recognised, mut unread) = (0, 0, 0, 0);
    let mut notes: Vec<String> = vec![];
    for (n, g) in todo.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(json!({ "item": n + 1, "items": todo.len(), "name": g.title }));
        let k = match crate::magtext::read(&g.path, reader.as_ref(), cancel, &mut |page, of| progress(json!({ "item": n + 1, "items": todo.len(), "name": g.title, "page": page, "pages": of }))) {
            Ok(k) => k,
            Err(e) => {
                let e = format!("{e:#}");
                if !notes.contains(&e) {
                    notes.push(e);
                }
                continue;
            }
        };
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let to = crate::magtext::text_path(&g.path).ok_or_else(|| anyhow!("no folder"))?;
        std::fs::create_dir_all(to.parent().unwrap_or(Path::new(".")))?;
        crate::config::write_json(&to, &serde_json::to_value(&k)?)?;
        read += 1;
        pages += k.pages.len();
        recognised += k.recognised.len();
        unread += k.unread.len();
        if let Some(note) = k.note {
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
    }
    Ok(json!({ "guidebooks": read, "pages": pages, "recognised": recognised, "unread": unread, "notes": notes, "reader": reader.as_ref().map(crate::magtext::Reader::id) }))
}

/// A guidebook's text for the viewer (its path below `Maps and Guides/`): each page's, and
/// those not read; null when it hasn't been read.
pub fn text_of(lib: &Library, rel: &str) -> Result<Value> {
    let g = by_rel(lib, rel)?;
    Ok(match crate::magtext::kept(&g.path) {
        Some(k) => json!({ "pages": k.pages, "recognised": k.recognised, "unread": k.unread, "note": k.note, "read": k.read }),
        None => Value::Null,
    })
}

// ------------------------------------------------------------ the pack for Atlas

/// A guidebook to put in a game's pack: what it's made from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Input {
    /// Its file's name in `Guidebooks/` (its id in the set)
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub kind: String,
    pub size: u64,
    pub mtime: i64,
    /// Its kept text's file's size and time, when its text is read for the file as it is
    pub text: Option<(u64, i64)>,
}

/// What a game's guidebook pack is to hold: its id, the game's title, and the guidebooks
/// ticked (in order), with those that can't go and why.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wanted {
    pub pack_id: String,
    pub game_title: String,
    pub items: Vec<Input>,
    /// (title, why) of each left out by the contract's bounds
    pub left: Vec<(String, String)>,
}

impl Wanted {
    /// Its recipe: the pack id, the game's title and each guidebook's id, title, file's size
    /// and time and text's size and time, hashed. The same recipe is the same pack.
    pub fn recipe(&self) -> String {
        use sha2::Digest;
        let r = serde_json::to_string(&(MAKER, &self.pack_id, &self.game_title, self.items.iter().map(|i| (&i.id, &i.title, i.size, i.mtime, i.text)).collect::<Vec<_>>())).unwrap_or_default();
        hex::encode(sha2::Sha256::digest(r.as_bytes()))[..32].to_string()
    }

    /// The pack's size, reckoned before it's made: its guidebooks' and a little more.
    pub fn estimate(&self) -> u64 {
        self.items.iter().map(|i| i.size + 4096).sum::<u64>() + 16 * 1024
    }
}

/// The version of how Game Repo makes the pack: a change to it makes every pack again.
const MAKER: u32 = 1;

fn text_stamp(path: &Path) -> Option<(u64, i64)> {
    crate::magtext::kept(path)?;
    let t = crate::magtext::text_path(path)?;
    let m = std::fs::metadata(t).ok()?;
    Some((m.len(), mtime_of(&m)))
}

/// What a game's guidebook pack holds: the folder's guidebooks less those in `left_out` (by
/// their file's names), at most [`MAX_GUIDEBOOKS`], each that the contract's bounds allow. None:
/// the folder has no game, or nothing to send.
pub fn wanted(f: &Folder, left_out: &[String]) -> Option<Wanted> {
    let pack_id = f.pack_id()?;
    let mut w = Wanted { pack_id, game_title: f.game_title().chars().take(MAX_GUIDEBOOK_TITLE).collect(), ..Default::default() };
    for g in f.guidebooks.iter().filter(|g| !left_out.contains(&g.file)) {
        if let Some(why) = cannot_go(g, &pages(g)) {
            w.left.push((g.title.clone(), why));
            continue;
        }
        if w.items.len() >= MAX_GUIDEBOOKS {
            w.left.push((g.title.clone(), format!("A game's pack for Atlas holds at most {MAX_GUIDEBOOKS} guidebooks.")));
            continue;
        }
        w.items.push(Input { id: g.file.clone(), title: g.title.chars().take(MAX_GUIDEBOOK_TITLE).collect(), path: g.path.clone(), kind: g.kind.clone(), size: g.size, mtime: g.mtime, text: text_stamp(&g.path) });
    }
    (!w.items.is_empty()).then_some(w)
}

/// One guidebook in a made pack: what it was made from, and its entry in the pack.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MadeInput {
    pub id: String,
    pub title: String,
    pub size: u64,
    pub mtime: i64,
    pub text: Option<(u64, i64)>,
    /// Its entry in the pack: `guidebooks/<name>.pdf` or `.cbz`
    pub entry: String,
}

/// A pack made into the cache.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Made {
    /// Its recipe ([`Wanted::recipe`])
    pub id: String,
    /// Its file's name in the pack's folder of the cache
    pub file: String,
    pub size: u64,
    pub sha1: String,
    /// Its `pack.updated` and `exportedAt`
    pub updated: String,
    pub exported_at: String,
    pub made: String,
    pub inputs: Vec<MadeInput>,
    /// How many of its guidebooks were copied from the pack made before, not written again
    pub kept: usize,
    /// A pack of magazines (D245): how many issues it holds and their pages, and the issues
    /// left out as it was made, each (title, why)
    pub items: usize,
    pub pages: u64,
    pub left: Vec<(String, String)>,
}

/// `made.json` in a pack's folder of the cache: what's made, oldest first; the last is the
/// newest, and the only one made again from (or given out for a recipe, while it's the one).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Ledger {
    made: Vec<Made>,
    next: u64,
}

/// The guidebook packs made, in a folder of their own (Game Repo Server's data folder): a
/// folder for each pack id, its files and `made.json`. A pack is made once for a recipe and
/// kept; a recipe whose pack isn't the newest made of that id is made again (so a device that
/// has the newer one gets one newer still, as Go never puts an older pack over a newer one);
/// the newest two of each id are kept, so a download under way finishes.
pub struct Cache {
    pub dir: PathBuf,
}

/// How many made files of each pack id are kept.
const KEEP: usize = 2;

impl Cache {
    pub fn new(dir: PathBuf) -> Cache {
        Cache { dir }
    }

    fn pack_dir(&self, pack_id: &str) -> Result<PathBuf> {
        if !atlas_pack::is_pack_id(pack_id) {
            bail!("{pack_id} can't be a pack's id.");
        }
        Ok(self.dir.join(pack_id))
    }

    fn ledger(&self, pack_id: &str) -> Ledger {
        self.pack_dir(pack_id).ok().and_then(|d| std::fs::read(d.join("made.json")).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, pack_id: &str, l: &Ledger) -> Result<()> {
        let d = self.pack_dir(pack_id)?;
        std::fs::create_dir_all(&d)?;
        let tmp = d.join("made.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(l)?)?;
        std::fs::rename(tmp, d.join("made.json"))?;
        Ok(())
    }

    /// A made pack's file.
    pub fn path(&self, pack_id: &str, m: &Made) -> PathBuf {
        self.dir.join(pack_id).join(&m.file)
    }

    fn whole(&self, pack_id: &str, m: &Made) -> bool {
        std::fs::metadata(self.path(pack_id, m)).is_ok_and(|x| x.len() == m.size)
    }

    /// The pack made for `w`, when it's made and is the newest made of its id.
    pub fn current(&self, w: &Wanted) -> Option<Made> {
        self.current_of(&w.pack_id, &w.recipe())
    }

    /// The pack made of `recipe`, when it's made and is the newest made of `pack_id`.
    pub fn current_of(&self, pack_id: &str, recipe: &str) -> Option<Made> {
        let l = self.ledger(pack_id);
        let last = l.made.last()?;
        (last.id == recipe && self.whole(pack_id, last)).then(|| last.clone())
    }

    /// The newest pack made of a recipe, still in the cache (for a download asked for by it).
    pub fn by_id(&self, pack_id: &str, id: &str) -> Option<Made> {
        let l = self.ledger(pack_id);
        l.made.iter().rev().find(|m| m.id == id && self.whole(pack_id, m)).cloned()
    }

    /// The newest pack made of an id, whatever it holds.
    pub fn newest(&self, pack_id: &str) -> Option<Made> {
        let l = self.ledger(pack_id);
        l.made.last().filter(|m| self.whole(pack_id, m)).cloned()
    }

    /// The pack for `w`: the one made already when it's the newest of its id, else made now.
    /// Made from the newest one before it where it can be: the guidebooks it has unchanged are
    /// copied from it as they are (`rewrite_guidebook_pack`), and only the others written.
    pub fn make(&self, w: &Wanted, cancel: &AtomicBool) -> Result<Made> {
        self.make_with(&w.pack_id, &w.recipe(), cancel, &mut |target, before, work| self.make_in(w, before, target, work, cancel))
    }

    /// A pack of `pack_id` for `recipe`: the one made already when it's the newest of its id,
    /// else made now by `write` (into the target it's given, with the newest whole pack made
    /// before, and a work folder emptied after), which gives what it made of it (its dates,
    /// inputs and what was left out). It's dated after every pack made of this id before it,
    /// named, hashed, put in the ledger, and only the newest [`KEEP`] files of the id kept.
    /// Guidebooks' packs (D236) and magazines' (D245) are made through it.
    pub fn make_with(&self, pack_id: &str, recipe: &str, cancel: &AtomicBool, write: &mut dyn FnMut(&Path, Option<&Made>, &Path) -> Result<Made>) -> Result<Made> {
        if let Some(m) = self.current_of(pack_id, recipe) {
            return Ok(m);
        }
        let dir = self.pack_dir(pack_id)?;
        std::fs::create_dir_all(&dir)?;
        let mut l = self.ledger(pack_id);
        let work = dir.join(".work");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work)?;
        // dated after every pack made of this id before it (the dates are to the second), so
        // a device that has any of them takes this one in its place (D236)
        if let Some(t) = l.made.iter().filter_map(|m| gamerepo_wire::packs::instant(&m.updated)).map(|t| t.0).max() {
            let now = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while now() <= t && std::time::Instant::now() < until && !cancel.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
        let target = dir.join(format!("making.{}", crate::atlaspack::EXTENSION));
        let _ = std::fs::remove_file(&target);
        let before = l.made.last().filter(|m| self.whole(pack_id, m)).cloned();
        let r = write(&target, before.as_ref(), &work).and_then(|mut m| {
            m.id = recipe.to_string();
            m.size = std::fs::metadata(&target)?.len();
            m.sha1 = sha1_file(&target, cancel)?;
            m.made = now();
            Ok(m)
        });
        let _ = std::fs::remove_dir_all(&work);
        let mut m = match r {
            Ok(m) => m,
            Err(e) => {
                let _ = std::fs::remove_file(&target);
                return Err(e);
            }
        };
        l.next += 1;
        // the file it was made as, renamed to its own name
        let file = format!("{}-{}.{}", &m.id[..12.min(m.id.len())], l.next, crate::atlaspack::EXTENSION);
        std::fs::rename(&target, dir.join(&file))?;
        m.file = file;
        l.made.push(m.clone());
        while l.made.len() > KEEP {
            let old = l.made.remove(0);
            let _ = std::fs::remove_file(dir.join(&old.file));
        }
        self.save(pack_id, &l)?;
        Ok(m)
    }

    fn make_in(&self, w: &Wanted, before: Option<&Made>, target: &Path, work: &Path, cancel: &AtomicBool) -> Result<Made> {
        // the guidebooks the newest pack has as they are now
        let unchanged = |i: &Input| before.and_then(|b| b.inputs.iter().find(|x| x.id == i.id && x.title == i.title && x.size == i.size && x.mtime == i.mtime && x.text == i.text)).map(|x| x.entry.clone());
        let kept: Vec<(usize, String)> = w.items.iter().enumerate().filter_map(|(n, i)| unchanged(i).map(|e| (n, e))).collect();
        let keep_entries: HashSet<String> = kept.iter().map(|(_, e)| e.clone()).collect();
        let fresh: Vec<&Input> = w.items.iter().enumerate().filter(|(n, _)| !kept.iter().any(|(k, _)| k == n)).map(|(_, i)| i).collect();
        let mut new = vec![];
        for (n, i) in fresh.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                bail!("stopped");
            }
            new.push(new_guidebook(i, work, n)?);
        }
        let pack = GuidebookPack {
            pack_id: w.pack_id.clone(),
            game_title: w.game_title.clone(),
            written_by: written_by(),
            guidebooks: new,
        };
        let header = match before.filter(|_| !kept.is_empty()) {
            Some(b) => atlas_pack::rewrite_guidebook_pack(&self.path(&w.pack_id, b), &|f| keep_entries.contains(&f.file), &pack, target),
            None => atlas_pack::write_guidebook_pack(&pack, target),
        }
        .map_err(|e| anyhow!("{e}"))?;
        // each guidebook's entry: a kept one keeps its own; the new ones are written after the
        // kept ones, in their order
        let mut entries = header.guidebooks.iter().map(|g| g.file.clone()).filter(|f| !keep_entries.contains(f));
        let mut inputs: Vec<MadeInput> = kept.iter().map(|(n, e)| (&w.items[*n], e.clone())).map(|(i, entry)| MadeInput { id: i.id.clone(), title: i.title.clone(), size: i.size, mtime: i.mtime, text: i.text, entry }).collect();
        for i in &fresh {
            inputs.push(MadeInput { id: i.id.clone(), title: i.title.clone(), size: i.size, mtime: i.mtime, text: i.text, entry: entries.next().unwrap_or_default() });
        }
        Ok(Made {
            updated: header.pack.updated.clone().unwrap_or_default(),
            exported_at: header.exported_at.clone(),
            items: header.guidebooks.len(),
            pages: header.guidebooks.iter().map(|g| u64::from(g.pages)).sum(),
            inputs,
            kept: kept.len(),
            ..Default::default()
        })
    }
}

/// Who writes Game Repo's packs (`pack.writtenBy`, `exportedBy`): Game Repo, this version.
pub(crate) fn written_by() -> atlas_pack::AppInfo {
    atlas_pack::AppInfo { name: WRITER.into(), version: env!("CARGO_PKG_VERSION").into(), platform: std::env::consts::OS.into() }
}

fn sha1_file(p: &Path, cancel: &AtomicBool) -> Result<String> {
    use sha1::Digest;
    let mut f = std::fs::File::open(p)?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("stopped");
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// A guidebook as the crate takes it: a PDF or a CBZ as it is, any other (a CBR, or a `.cbz`
/// that isn't a ZIP) made a CBZ in `work` first; its pages counted; its text, when it's read,
/// cut to its pages and left out when it's over the contract's 64 MB.
fn new_guidebook(i: &Input, work: &Path, n: usize) -> Result<NewGuidebook> {
    let zip_already = i.kind == "cbz" && crate::magazines::comic_kind(&i.path) == Some("zip");
    let (file, pages) = if i.kind == "pdf" || zip_already {
        let g = Guidebook { rel: String::new(), folder: String::new(), file: i.id.clone(), title: i.title.clone(), kind: i.kind.clone(), size: i.size, mtime: i.mtime, path: i.path.clone() };
        (i.path.clone(), pages(&g).map_err(|e| anyhow!(e))?)
    } else {
        let to = work.join(format!("{n}.cbz"));
        let pages = to_cbz(&i.path, &to)?;
        (to, pages)
    };
    let text = crate::magtext::kept(&i.path).map(|k| k.pages).map(|mut p| {
        p.truncate(pages as usize);
        p
    });
    let text = text.filter(|p| {
        let t = atlas_pack::GuidebookText::of_pages(p);
        !t.pages.is_empty() && serde_json::to_vec(&t).is_ok_and(|b| (b.len() as u64) <= MAX_GUIDEBOOK_TEXT_BYTES)
    });
    Ok(NewGuidebook { file, title: i.title.clone(), pages: Some(pages), text })
}

/// A comic (a CBR, or any CBZ or CBR whatever it really is) made a CBZ for Atlas: its pictures
/// in the order of their names, stored as they are, named `0001.jpg`, `0002.png`… The number
/// of pages.
pub fn to_cbz(from: &Path, to: &Path) -> Result<u32> {
    let names = crate::magazines::pages(from)?;
    if names.is_empty() {
        bail!("{} holds no pictures of pages.", from.file_name().unwrap_or_default().to_string_lossy());
    }
    let at: HashMap<String, usize> = names.iter().enumerate().map(|(n, x)| (x.clone(), n)).collect();
    let kind = crate::magazines::comic_kind(from).unwrap_or("zip");
    let width = names.len().to_string().len().max(4);
    // read in the archive's order, written in the pages' order: each page kept in a file of
    // the work folder first (a RAR can only be read in its own order)
    let dir = to.with_extension("pages");
    std::fs::create_dir_all(&dir)?;
    let mut got = vec![None; names.len()];
    crate::archive::each_wanted_as(from, kind, &|x| at.contains_key(x), &AtomicBool::new(false), &mut |name, _, r| {
        let n = at[name];
        let p = dir.join(format!("{n}"));
        let mut f = std::fs::File::create(&p)?;
        std::io::copy(r, &mut f)?;
        got[n] = Some(p);
        Ok(())
    })?;
    let out = std::fs::File::create(to)?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(out));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
    for (n, p) in got.iter().enumerate() {
        let Some(p) = p else { bail!("A page of {} couldn't be read.", from.file_name().unwrap_or_default().to_string_lossy()) };
        let ext = crate::platforms::ext_of(&names[n]);
        z.start_file(format!("{:0width$}.{ext}", n + 1), opts)?;
        std::io::copy(&mut std::fs::File::open(p)?, &mut z)?;
    }
    z.finish()?.flush()?;
    let _ = std::fs::remove_dir_all(&dir);
    Ok(names.len() as u32)
}

/// A game's guidebook pack made into `cache` now (for the CLI and tests): its folder's
/// guidebooks less `left_out`.
pub fn make_for(ix: &Index, lib: &Library, cache: &Cache, key: &str, left_out: &[String]) -> Result<(Made, Wanted)> {
    let f = folder_of(ix, lib, &[key.to_string()])?.ok_or_else(|| anyhow!("{key} has no guidebooks."))?;
    let w = wanted(&f, left_out).ok_or_else(|| anyhow!("None of {}'s guidebooks can go to Atlas.", f.game_title()))?;
    Ok((cache.make(&w, &AtomicBool::new(false))?, w))
}

/// The title a file brings in (its name without the extension), for the import's review.
pub fn title_for(path: &Path) -> String {
    title_of_file(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

#[cfg(test)]
pub(crate) mod tests {
    //! Guidebooks made up here: PDFs written with lopdf, CBZs with the zip crate, and CBRs as
    //! RAR 4 archives of stored files written byte by byte (no free tool writes RAR, and
    //! unrar reads these as it reads any).
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// A PDF of `pages` pages, each a line of text (empty for a blank page).
    pub(crate) fn pdf(path: &Path, pages: &[&str]) {
        use lopdf::{dictionary, Object, Stream};
        let mut doc = lopdf::Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let mut kids = vec![];
        for t in pages {
            let body = if t.is_empty() { String::new() } else { format!("BT /F1 12 Tf 20 700 Td ({t}) Tj ET") };
            let content = doc.add_object(Stream::new(dictionary! {}, body.into_bytes()));
            let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()], "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } });
            kids.push(Object::Reference(page));
        }
        let n = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => n }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        doc.save(path).unwrap();
    }

    /// A small JPEG's first bytes and some more.
    pub(crate) fn jpeg(seed: u8) -> Vec<u8> {
        crate::atlaspack::tests::jpeg(seed)
    }

    pub(crate) fn cbz(path: &Path, pages: &[(&str, Vec<u8>)]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in pages {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    /// A RAR 4 archive of stored files (RAR's own format, as `rar a -m0` writes it).
    pub(crate) fn rar(path: &Path, files: &[(&str, Vec<u8>)]) {
        let crc16 = |b: &[u8]| (crc32fast::hash(b) & 0xFFFF) as u16;
        let mut out: Vec<u8> = b"Rar!\x1a\x07\x00".to_vec();
        // the archive's header: type 0x73, no flags, 13 bytes
        let main = [0x73u8, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0];
        out.extend(crc16(&main).to_le_bytes());
        out.extend(main);
        for (name, data) in files {
            let mut h: Vec<u8> = vec![0x74];
            h.extend(0x8000u16.to_le_bytes()); // its data follows
            let size = (32 + name.len()) as u16;
            h.extend(size.to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes()); // packed
            h.extend((data.len() as u32).to_le_bytes()); // unpacked
            h.push(2); // Windows
            h.extend(crc32fast::hash(data).to_le_bytes());
            h.extend(0x5A21_0000u32.to_le_bytes()); // a DOS time in 2025
            h.push(29);
            h.push(0x30); // stored
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0x20u32.to_le_bytes());
            h.extend(name.as_bytes());
            out.extend(crc16(&h).to_le_bytes());
            out.extend(&h);
            out.extend(data);
        }
        out.extend([0xC4, 0x3D, 0x7B, 0x00, 0x40, 0x07, 0x00]);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, out).unwrap();
    }

    fn run(ix: &Index, lib: &Library, plan: &Plan) {
        let r = organise::run(ix, lib, plan, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(r["failed"].as_array().unwrap().is_empty(), "{r}");
        maps::after(ix, lib, plan, false).unwrap();
    }

    fn library(name: &str) -> (PathBuf, Library, Index) {
        let (dir, lib, ix) = crate::collection::tests::fixture(name);
        crate::meta::sync(&ix, &lib).unwrap();
        crate::browse::refresh(&ix, &lib).unwrap();
        (dir, lib, ix)
    }

    /// A guidebook's text kept beside it, as the magazines' reader keeps an issue's.
    fn keep_text(path: &Path, pages: &[&str]) {
        let (size, modified) = {
            let m = std::fs::metadata(path).unwrap();
            (m.len(), mtime_of(&m))
        };
        let k = crate::magtext::Kept { size, modified, read: now(), pages: pages.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        let to = crate::magtext::text_path(path).unwrap();
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        crate::config::write_json(&to, &serde_json::to_value(&k).unwrap()).unwrap();
    }

    #[test]
    fn guidebooks_are_found_imported_renamed_and_removed_with_undo() {
        let (dir, lib, ix) = library("guidebooks-import");
        let base = maps::folder(&lib);
        // picked to import: a PDF, a CBZ, a CBR and a text file, which isn't one
        let picked = dir.join("picked");
        pdf(&picked.join("Delta Official Guide.pdf"), &["Delta", "", "The swamp"]);
        cbz(&picked.join("Delta Comic.cbz"), &[("p2.jpg", jpeg(2)), ("p1.jpg", jpeg(1)), ("notes.txt", b"x".to_vec())]);
        rar(&picked.join("Delta Art.cbr"), &[("b.jpg", jpeg(4)), ("a.jpg", jpeg(3))]);
        std::fs::write(picked.join("readme.txt"), b"not one").unwrap();
        assert_eq!(files_in(&lib, std::slice::from_ref(&picked)).len(), 3);
        let plan = import_plan(&ix, &lib, std::slice::from_ref(&picked), "gb/Delta (World)", "copy", true).unwrap();
        assert_eq!((plan.kind.as_str(), plan.games), ("maps_guidebooks_import", 3));
        run(&ix, &lib, &plan);
        for f in ["Delta Official Guide.pdf", "Delta Comic.cbz", "Delta Art.cbr"] {
            assert!(base.join("Delta/Guidebooks").join(f).is_file(), "{f}");
            assert!(picked.join(f).is_file(), "copied: the original stays");
        }
        // one put there by hand in a folder named for a game you have (Beta), and one being written
        pdf(&base.join("Beta/Guidebooks/Beta Strategy.pdf"), &["Beta"]);
        std::fs::write(base.join("Beta/Guidebooks/Beta Strategy 2.pdf.part"), b"%PDF-").unwrap();
        // a folder no game is named for
        pdf(&base.join("Omega/Guidebooks/Omega.pdf"), &["Omega"]);
        let v = overview(&ix, &lib).unwrap();
        let all = v["guidebooks"].as_array().unwrap();
        let by = |t: &str| all.iter().find(|g| g["title"] == t).unwrap_or_else(|| panic!("{t}: {v}")).clone();
        assert_eq!(all.len(), 5, "{v}");
        let guide = by("Delta Official Guide");
        assert_eq!((guide["kind"].as_str(), guide["pages"].as_u64(), guide["how"].as_str()), (Some("pdf"), Some(3), Some("record")));
        assert_eq!(guide["games"][0]["key"], "gb/Delta (World)");
        assert_eq!(guide["pack_id"], json!(pack_id_for("gb/Delta (World)")));
        assert_eq!(guide["text"], Value::Null, "not read yet");
        assert_eq!(by("Delta Comic")["pages"], 2, "its pictures, not its text file");
        assert_eq!((by("Delta Art")["kind"].as_str(), by("Delta Art")["pages"].as_u64()), (Some("cbr"), Some(2)));
        assert_eq!(by("Beta Strategy")["how"], "title");
        assert!(by("Beta Strategy")["games"][0]["key"].as_str().unwrap().starts_with("gb/Beta ("), "{}", by("Beta Strategy"));
        assert_eq!(by("Omega")["games"], json!([]));
        // packs keep passing over the folder
        assert!(maps::sync(&ix, &lib).unwrap().packs.is_empty());
        // a game's page lists its own
        assert_eq!(of_game(&ix, &lib, "gb/Delta (World)").len(), 3);
        assert_eq!(of_game(&ix, &lib, "gb/Alpha (USA)").len(), 0);

        // a title, kept in the record; empty is its file's name again
        set_title(&lib, "Delta/Guidebooks/Delta Comic.cbz", "The Delta Comic").unwrap();
        assert_eq!(by_rel(&lib, "Delta/Guidebooks/Delta Comic.cbz").unwrap().title, "The Delta Comic");
        set_title(&lib, "Delta/Guidebooks/Delta Comic.cbz", "").unwrap();
        assert_eq!(by_rel(&lib, "Delta/Guidebooks/Delta Comic.cbz").unwrap().title, "Delta Comic");
        // Omega linked to a game by hand, then found again
        link(&ix, &lib, "Omega", Some("gb/Gamma (USA) (Beta)")).unwrap();
        assert_eq!(of_game(&ix, &lib, "gb/Gamma (USA) (Beta)").len(), 1);
        link(&ix, &lib, "Omega", None).unwrap();
        assert!(of_game(&ix, &lib, "gb/Gamma (USA) (Beta)").is_empty());

        // removed: set aside, out of the folder Atlas Importer reads, then put back by Undo
        let plan = remove_plan(&lib, &["Delta/Guidebooks/Delta Art.cbr".to_string()]).unwrap();
        run(&ix, &lib, &plan);
        assert!(!base.join("Delta/Guidebooks/Delta Art.cbr").exists());
        assert_eq!(list(&lib).len(), 4);
        let r = organise::undo(&ix, &lib, &plan.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(r["problems"].as_array().unwrap().is_empty(), "{r}");
        assert!(base.join("Delta/Guidebooks/Delta Art.cbr").is_file());
        assert!(remove_plan(&lib, &["Delta/Guidebooks/nothing.pdf".to_string()]).is_err());

        // a second import to the game finds its folder, a name taken gets another
        let again = dir.join("again");
        pdf(&again.join("Delta Official Guide.pdf"), &["Delta, second printing"]);
        let plan = import_plan(&ix, &lib, &[again.join("Delta Official Guide.pdf")], "gb/Delta (World)", "move", false).unwrap();
        run(&ix, &lib, &plan);
        assert!(base.join("Delta/Guidebooks/Delta Official Guide (2).pdf").is_file());
        assert!(!again.join("Delta Official Guide.pdf").exists(), "moved");
        // the import's Undo takes the file and the record back
        let r = organise::undo(&ix, &lib, &plan.id, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(r["problems"].as_array().unwrap().is_empty(), "{r}");
        assert!(again.join("Delta Official Guide.pdf").is_file());
        assert!(!read_record(&lib).guidebooks.contains_key("Delta/Guidebooks/Delta Official Guide (2).pdf"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn their_text_is_read_page_by_page_as_the_magazines_is() {
        let (dir, lib, ix) = library("guidebooks-text");
        let base = maps::folder(&lib);
        pdf(&base.join("Delta/Guidebooks/Guide.pdf"), &["The first page of the Delta guide", "", "The third page says where the key is"]);
        let r = read_texts(&lib, None, false, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["guidebooks"], 1, "{r}");
        let t = text_of(&lib, "Delta/Guidebooks/Guide.pdf").unwrap();
        assert!(t["pages"][2].as_str().unwrap().contains("key"), "{t}");
        let v = overview(&ix, &lib).unwrap();
        assert_eq!(v["guidebooks"][0]["text"]["pages"], 2, "two pages with text: {v}");
        // read once
        assert_eq!(read_texts(&lib, None, false, &AtomicBool::new(false), &mut |_| {}).unwrap()["guidebooks"], 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cbr_is_made_a_cbz_with_its_pages_in_order() {
        let dir = crate::library::tests::temp_dir("guidebooks-cbr");
        let from = dir.join("Art.cbr");
        rar(&from, &[("page10.jpg", jpeg(10)), ("page2.jpg", jpeg(2)), ("page1.jpg", jpeg(1)), ("info.txt", b"x".to_vec())]);
        let to = dir.join("art.cbz");
        assert_eq!(to_cbz(&from, &to).unwrap(), 3);
        let mut z = zip::ZipArchive::new(std::fs::File::open(&to).unwrap()).unwrap();
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect();
        assert_eq!(names, ["0001.jpg", "0002.jpg", "0003.jpg"]);
        for (n, seed) in [(0, 1), (1, 2), (2, 10)] {
            let mut f = z.by_index(n).unwrap();
            assert_eq!(f.compression(), zip::CompressionMethod::Stored);
            let mut b = vec![];
            f.read_to_end(&mut b).unwrap();
            assert_eq!(b, jpeg(seed));
        }
        assert_eq!(atlas_pack::guidebook_pages(&to).unwrap(), 3, "Atlas's crate counts its pages too");
        // a CBR that's really a ZIP is made one as well
        cbz(&dir.join("Zip.cbr"), &[("b.png", jpeg(5)), ("a.png", jpeg(6))]);
        assert_eq!(to_cbz(&dir.join("Zip.cbr"), &dir.join("zip.cbz")).unwrap(), 2);
        // one with no pictures says so
        rar(&dir.join("Empty.cbr"), &[("info.txt", b"x".to_vec())]);
        assert!(to_cbz(&dir.join("Empty.cbr"), &dir.join("empty.cbz")).unwrap_err().to_string().contains("no pictures"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_games_pack_is_made_once_made_again_on_a_change_and_read_back_by_atlass_crate() {
        let (dir, lib, ix) = library("guidebooks-pack");
        let base = maps::folder(&lib);
        let gb = base.join("Delta/Guidebooks");
        pdf(&gb.join("Official Guide.pdf"), &["Delta", "", "The swamp"]);
        keep_text(&gb.join("Official Guide.pdf"), &["Delta", "", "The swamp", "a page too many"]);
        rar(&gb.join("Art Book.cbr"), &[("2.jpg", jpeg(2)), ("1.jpg", jpeg(1))]);
        cbz(&gb.join("Comic.cbz"), &[("1.jpg", jpeg(7))]);
        link(&ix, &lib, "Delta", Some("gb/Delta (World)")).unwrap();
        let cache = Cache::new(dir.join("cache"));
        let key = "gb/Delta (World)";
        let id = pack_id_for(key);

        let (m, w) = make_for(&ix, &lib, &cache, key, &[]).unwrap();
        assert_eq!((m.kept, w.items.len()), (0, 3));
        let file = cache.path(&id, &m);
        assert_eq!(std::fs::metadata(&file).unwrap().len(), m.size);
        // read back with Atlas's own crate
        let h = atlas_pack::read_header(&file).unwrap();
        assert_eq!((h.version, h.pack_id.as_str(), h.title.as_str()), (8, id.as_str(), "Delta"));
        assert_eq!(h.game.as_ref().map(|g| g.title.as_str()), Some("Delta"));
        assert_eq!(h.origin(), Some(atlas_pack::Origin::Guidebooks));
        assert_eq!(h.source.origin, Some(atlas_pack::Origin::Guidebooks));
        // and Game Repo's own reader takes it as the sort Guidebooks (D231), as any pack of guidebooks
        let d = crate::atlaspack::read_header(&file).unwrap();
        assert_eq!((d.sort(), d.from_words().as_deref()), (Some(crate::atlaspack::Sort::Guidebooks), Some("Guidebooks")));
        assert_eq!(h.exported_by.name, WRITER, "Atlas never changes it");
        let titles: Vec<(&str, &str, u32)> = h.guidebooks.iter().map(|g| (g.title.as_str(), g.file.as_str(), g.pages)).collect();
        assert_eq!(titles, [("Art Book", "guidebooks/art-book.cbz", 2), ("Comic", "guidebooks/comic.cbz", 1), ("Official Guide", "guidebooks/official-guide.pdf", 3)]);
        let guide = &h.guidebooks[2];
        assert_eq!(guide.bytes, std::fs::metadata(gb.join("Official Guide.pdf")).unwrap().len());
        assert_eq!(guide.text.as_deref(), Some("guidebooks/official-guide.text.json"));
        assert!(h.guidebooks[0].text.is_none(), "no text read for it");
        let mut z = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
        let mut b = vec![];
        z.by_name("guidebooks/official-guide.text.json").unwrap().read_to_end(&mut b).unwrap();
        let t = atlas_pack::parse_guidebook_text(&b, 3).unwrap();
        assert_eq!(t.pages.iter().map(|p| p.page).collect::<Vec<_>>(), [1, 3], "a blank page left out, the one past its last cut");
        assert_eq!(z.by_name("guidebooks/art-book.cbz").unwrap().compression(), zip::CompressionMethod::Stored);
        let mut b = vec![];
        z.by_name("guidebooks/official-guide.pdf").unwrap().read_to_end(&mut b).unwrap();
        assert_eq!(b, std::fs::read(gb.join("Official Guide.pdf")).unwrap(), "the file as it is");
        drop(z);

        // made once: the same again is the one made
        let (again, _) = make_for(&ix, &lib, &cache, key, &[]).unwrap();
        assert_eq!(again, m);
        assert_eq!(cache.current(&w), Some(m.clone()));
        // a title changed: made again, the others copied from the one before as they are
        set_title(&lib, "Delta/Guidebooks/Comic.cbz", "The Comic").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let (m2, w2) = make_for(&ix, &lib, &cache, key, &[]).unwrap();
        assert_ne!(m2.id, m.id);
        assert_eq!(m2.kept, 2);
        assert!(m2.updated > m.updated, "{} after {}", m2.updated, m.updated);
        let h2 = atlas_pack::read_header(&cache.path(&id, &m2)).unwrap();
        assert_eq!(h2.guidebooks.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), ["Art Book", "Official Guide", "The Comic"]);
        assert_eq!(h2.pack_version, "1.0.1");
        // one left out (a device unticked it): a pack of the others
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let (m3, _) = make_for(&ix, &lib, &cache, key, &["Art Book.cbr".to_string()]).unwrap();
        assert_eq!(atlas_pack::read_header(&cache.path(&id, &m3)).unwrap().guidebooks.len(), 2);
        // and every one again: made again (newer still), never the one made before, which a
        // device holding the smaller one would keep as newer
        assert_eq!(cache.current(&w2), None);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let (m4, _) = make_for(&ix, &lib, &cache, key, &[]).unwrap();
        assert_eq!(m4.id, m2.id);
        assert!(m4.updated > m3.updated);
        assert_eq!(cache.by_id(&id, &m2.id), Some(m4.clone()), "the newest of the recipe");
        // two kept of each pack
        let files = std::fs::read_dir(cache.dir.join(&id)).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".atlaspack")).count();
        assert_eq!(files, 2);
        // a file changed: made again
        std::thread::sleep(std::time::Duration::from_millis(20));
        cbz(&gb.join("Comic.cbz"), &[("1.jpg", jpeg(7)), ("2.jpg", jpeg(8))]);
        let (m5, _) = make_for(&ix, &lib, &cache, key, &[]).unwrap();
        assert_ne!(m5.id, m4.id);
        let h5 = atlas_pack::read_header(&cache.path(&id, &m5)).unwrap();
        assert_eq!(h5.guidebooks.len(), 3);
        // each guidebook's entry is known, whichever pack it was copied through
        for i in &m5.inputs {
            let g = h5.guidebooks.iter().find(|g| g.file == i.entry).unwrap();
            assert_eq!(g.title, i.title);
        }
        assert_eq!(atlas_pack::read_header(&cache.path(&id, &m5)).unwrap().guidebooks.iter().find(|g| g.title == "The Comic").unwrap().pages, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_contracts_bounds_leave_out_what_cant_go() {
        let (dir, lib, ix) = library("guidebooks-bounds");
        let gb = maps::folder(&lib).join("Delta/Guidebooks");
        pdf(&gb.join("Good.pdf"), &["Delta"]);
        std::fs::create_dir_all(&gb).unwrap();
        std::fs::write(gb.join("Empty.pdf"), b"").unwrap();
        std::fs::write(gb.join("Broken.pdf"), b"%PDF-1.4 nothing more").unwrap();
        std::fs::write(gb.join("Not a comic.cbz"), b"not a zip").unwrap();
        let long = "A".repeat(250);
        pdf(&gb.join("Long.pdf"), &["x"]);
        set_title(&lib, "Delta/Guidebooks/Long.pdf", &long).unwrap_err();
        let f = folder_of(&ix, &lib, &["gb/Delta (World)".to_string()]).unwrap().unwrap();
        let w = wanted(&f, &[]).unwrap();
        assert_eq!(w.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["Good.pdf", "Long.pdf"]);
        let left: Vec<&str> = w.left.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(left, ["Broken", "Empty", "Not a comic"]);
        assert!(w.left.iter().all(|(_, why)| why.contains("Atlas")), "{:?}", w.left);
        // the page says why
        let v = overview(&ix, &lib).unwrap();
        assert!(v["guidebooks"].as_array().unwrap().iter().find(|g| g["title"] == "Empty").unwrap()["cannot_go"].as_str().unwrap().contains("empty"));
        // more than a pack can hold: the first 100
        for n in 0..100 {
            pdf(&gb.join(format!("Extra {n:03}.pdf")), &["x"]);
        }
        let f = folder_of(&ix, &lib, &["gb/Delta (World)".to_string()]).unwrap().unwrap();
        let w = wanted(&f, &[]).unwrap();
        assert_eq!(w.items.len(), MAX_GUIDEBOOKS);
        assert_eq!(w.left.len(), 3 + 2);
        // none left: no pack
        let all: Vec<String> = f.guidebooks.iter().map(|g| g.file.clone()).collect();
        assert!(wanted(&f, &all).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_games_pack_id_is_safe_stable_and_its_own() {
        for key in ["gb/Delta (World)", "pc/12345", "snes/Legend of Zelda, The - A Link to the Past (USA) (Rev 1) [!] with a very long name indeed", "x/ÄÖÜ", "a/b"] {
            let id = pack_id_for(key);
            assert!(atlas_pack::is_pack_id(&id) && id.len() <= 64 && is_pack_id(&id), "{id}");
            assert_eq!(id, pack_id_for(key), "the same for the game's life");
        }
        assert_eq!(pack_id_for("gb/Delta (World)"), format!("guidebooks-gb-delta-world-{}", &hex::encode(<sha1::Sha1 as sha1::Digest>::digest(b"gb/Delta (World)"))[..8]));
        // keys that read alike are told apart
        assert_ne!(pack_id_for("gb/Delta (World)"), pack_id_for("gb/Delta World"));
        assert!(pack_id_for("x/ÄÖÜ").starts_with("guidebooks-x-"));
        assert!(!is_pack_id("delta-example-maps"));
    }
}
