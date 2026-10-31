//! ROM patches kept with their games (docs/PLAN.md, "Cheats and patches", D201–D203): each
//! game folder's `patches/` keeps its patch files as they came, with `patches.json` saying
//! each one's kind (*Translation*, *Hack*, *Fix*, *Restoration*, *Other*), title, author,
//! version, language, web address, notes, format and the file it's for (its readme is kept
//! beside it, `<patch>.txt`).
//!
//! *Apply* makes a patched copy and never changes the game's file: it's written beside the
//! game, named after it with the patch's tags, and added to the system's custom DAT as a
//! variant of the game (`cloneof`), so it's a game of its own in the Library under its
//! original. Its `game.json` says which patches made it, in order, and the original's SHA-1.
//! It's one Organise change (kind `patch`, op `patch`) with Undo: the copy goes and the
//! custom DAT drops it.
//!
//! Bringing patches in pairs each with a game: a BPS's or UPS's own source CRC32 (with or
//! without a copier header) against the library's files is *sure*; else the name compared,
//! *likely*. The patch formats themselves are `rompatch.rs`.

use crate::cheats::{self, game_of, had_games, shown, stamp, title_and_tags, walk, LibGame};
use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use crate::library::{now, Library};
use crate::meta::{self, DatGame};
use crate::organise::{self, safe_name, Op, Plan, Src};
use crate::rompatch;
use crate::variants::{self, split_name, words};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub const FOLDER: &str = "patches";
pub const MANIFEST: &str = "patches.json";

/// The kinds of ROM patch, and the tag each gives its patched copy (D121's).
pub const KINDS: &[(&str, &str, &str)] = &[
    ("translation", "Translation", "Translated"),
    ("hack", "Hack", "Hack"),
    ("fix", "Fix", "Bug Fix"),
    ("restoration", "Restoration", "Restoration"),
    ("other", "Other", "Hack"),
];

pub fn kind_name(id: &str) -> &'static str {
    KINDS.iter().find(|k| k.0 == id).map(|k| k.1).unwrap_or("Other")
}

/// The file names patches come in (`.dat` isn't one: too many other files are).
fn is_patch_name(name: &str) -> bool {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    rompatch::EXTENSIONS.contains(&ext.as_str()) && ext != "dat"
}

/// The patch files in a game's patches folder (not patches.json, the readmes or what's being written).
fn files_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| !n.starts_with('.') && is_patch_name(n))
        .collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

fn signature(dir: &Path) -> String {
    let mut s = String::new();
    for (n, p) in files_in(dir) {
        if let Some((a, b)) = stamp(&p) {
            s.push_str(&format!("{n}\t{a}\t{b}\n"));
        }
    }
    if let Some((a, b)) = stamp(&dir.join(MANIFEST)) {
        s.push_str(&format!("{MANIFEST}\t{a}\t{b}\n"));
    }
    s
}

/// A patch's details: patches.json's, else what its file says and its name suggests.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Details {
    pub kind: String,
    pub title: String,
    pub author: String,
    pub version: String,
    pub language: String,
    pub url: String,
    pub notes: String,
}

impl Details {
    pub fn of(e: &Value, name: &str) -> Details {
        let mut d: Details = serde_json::from_value(e.clone()).unwrap_or_default();
        if d.kind.is_empty() {
            d.kind = guess(name, "").kind;
        }
        if d.title.is_empty() {
            d.title = guess(name, "").title;
        }
        d
    }
}

/// Bring the index's patches up to date with the game folders'.
pub fn sync(ix: &Index, lib: &Library) -> Result<()> {
    let conn = ix.conn();
    let known: HashMap<String, String> = {
        let mut q = conn.prepare("SELECT folder, sig FROM patch_folders")?;
        let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut seen = HashSet::new();
    let tx = conn.unchecked_transaction()?;
    for sys in std::fs::read_dir(meta::games_root(lib)).into_iter().flatten().flatten() {
        if !sys.path().is_dir() || sys.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let platform = sys.file_name().to_string_lossy().into_owned();
        for g in std::fs::read_dir(sys.path()).into_iter().flatten().flatten() {
            let dir = g.path().join(FOLDER);
            if !dir.is_dir() {
                continue;
            }
            let Some(folder) = lib.relative(&g.path()) else { continue };
            seen.insert(folder.clone());
            let sig = signature(&dir);
            if known.get(&folder) == Some(&sig) {
                continue;
            }
            tx.execute("DELETE FROM patches WHERE folder = ?1", [&folder])?;
            let manifest = read_json_object(&dir.join(MANIFEST));
            for (name, p) in files_in(&dir) {
                let e = &manifest["files"][&name];
                let d = Details::of(e, &name);
                let head = read_head(&p);
                let format = rompatch::detect(&head).unwrap_or("").to_string();
                let applied: Vec<String> = e["applied"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
                let Some(rel) = lib.relative(&p) else { continue };
                tx.execute(
                    "INSERT OR REPLACE INTO patches (path, folder, platform, name, format, kind, title, author, version, language, applied) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![rel, folder, platform, name, format, d.kind, d.title, d.author, d.version, d.language, applied.join("\n")],
                )?;
            }
            tx.execute("INSERT OR REPLACE INTO patch_folders (folder, sig) VALUES (?1, ?2)", params![folder, sig])?;
        }
    }
    for f in known.keys().filter(|f| !seen.contains(*f)) {
        tx.execute("DELETE FROM patches WHERE folder = ?1", [f])?;
        tx.execute("DELETE FROM patch_folders WHERE folder = ?1", [f])?;
    }
    tx.commit()?;
    Ok(())
}

fn read_head(p: &Path) -> Vec<u8> {
    let mut b = vec![0u8; 8];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut b)).unwrap_or(0);
    b.truncate(n);
    b
}

/// How many patches a game folder has (a game's page).
pub fn count_of(conn: &Connection, folder: Option<&str>) -> i64 {
    folder.and_then(|f| conn.query_row("SELECT COUNT(*) FROM patches WHERE folder = ?1", [f], |r| r.get(0)).ok()).unwrap_or(0)
}

// ------------------------------------------------------------ guessing (D202)

/// What a patch's name and readme suggest: its kind, language, version, author and title.
pub fn guess(name: &str, readme: &str) -> Details {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let n = stem.to_lowercase().replace(['_', '-', '.'], " ");
    let head: String = readme.lines().take(40).collect::<Vec<_>>().join("\n");
    let r = head.to_lowercase();
    let has = |s: &str, words: &[&str]| words.iter().any(|w| s.contains(w));
    const TRANSLATION: &[&str] = &["t en", "t+eng", "t eng", "translation", "translated", "traducción", "traduccion", "traduction", "tradução", "traduzione", "übersetzung", "fan translation"];
    const RESTORATION: &[&str] = &["restoration", "restored", "uncensor", "unrestored"];
    const FIX: &[&str] = &["bug fix", "bugfix", "fix", "improvement", "improved", "patch fix", "msu"];
    let kind = if has(&n, TRANSLATION) {
        "translation"
    } else if has(&n, RESTORATION) {
        "restoration"
    } else if has(&n, FIX) {
        "fix"
    } else if has(&n, &["hack"]) {
        "hack"
    } else if has(&r, TRANSLATION) {
        "translation"
    } else if has(&r, RESTORATION) {
        "restoration"
    } else if has(&r, &["bug fix", "bugfix", "fixes"]) {
        "fix"
    } else {
        "hack"
    };
    const LANGS: &[(&[&str], &str)] = &[
        (&["english", "t en", "t+eng", "t eng", "inglés"], "English"),
        (&["spanish", "español", "espanol", "t es", "t+spa"], "Spanish"),
        (&["french", "français", "francais", "t fr", "t+fre"], "French"),
        (&["german", "deutsch", "t de", "t+ger"], "German"),
        (&["italian", "italiano", "t it", "t+ita"], "Italian"),
        (&["brazilian", "português", "portuguese", "t pt", "t+por", "t+bra"], "Portuguese"),
        (&["russian", "t ru", "t+rus"], "Russian"),
        (&["chinese", "t zh", "t+chi"], "Chinese"),
        (&["korean", "t ko", "t+kor"], "Korean"),
    ];
    let mut language = String::new();
    if kind == "translation" {
        for s in [&n, &r] {
            if let Some((_, l)) = LANGS.iter().find(|(ws, _)| has(s, ws)) {
                language = l.to_string();
                break;
            }
        }
    }
    let version = version_in(stem).or_else(|| head.lines().find_map(version_in)).unwrap_or_default();
    let author = head
        .lines()
        .find_map(|l| {
            let t = l.trim();
            let low = t.to_lowercase();
            ["translated by", "hacked by", "created by", "made by", "author:", "authors:", "by "].iter().find_map(|k| {
                let at = low.find(k)?;
                (at == 0 || *k != "by ").then(|| t[at + k.len()..].trim_matches(|c: char| c == ':' || c.is_whitespace()).to_string())
            })
        })
        .filter(|a| !a.is_empty() && a.len() < 80)
        .unwrap_or_default();
    // the title: the name without its version and the words saying what it is
    let mut title = stem.replace('_', " ");
    if !version.is_empty() {
        if let Some(at) = title.to_lowercase().rfind(&format!("v{}", version.to_lowercase())) {
            title.replace_range(at..at + version.len() + 1, "");
        }
    }
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(|c: char| c == '-' || c == ' ').to_string();
    Details { kind: kind.into(), title, author, version, language, ..Default::default() }
}

/// A version written "v1.2", "v1", "Version 1.2a" or "ver. 2".
fn version_in(s: &str) -> Option<String> {
    let low = s.to_lowercase();
    let b = low.as_bytes();
    for (i, w) in low.char_indices() {
        if w != 'v' || (i > 0 && b[i - 1].is_ascii_alphanumeric()) {
            continue;
        }
        let rest = &low[i..];
        let skip = ["version ", "version", "ver. ", "ver.", "ver ", "v. ", "v.", "v"].iter().find(|p| rest.starts_with(*p)).map(|p| p.len()).unwrap_or(1);
        let v: String = rest[skip..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
        let v = v.trim_end_matches('.');
        if v.starts_with(|c: char| c.is_ascii_digit()) && v.len() <= 12 && v.chars().filter(|c| c.is_ascii_alphabetic()).count() <= 2 {
            return Some(s[i + skip..i + skip + v.len()].to_string());
        }
    }
    None
}

// ------------------------------------------------------------ a game's ROM

/// A game's file to patch: the one matched to its DAT game (a disc's several files aren't yet).
fn rom_of(conn: &Connection, lib: &Library, g: &LibGame, want_crc: Option<u32>) -> Result<Src> {
    let id = g.dat_game.ok_or_else(|| anyhow!("{} isn't a game of a DAT, so there's no file to patch.", g.title))?;
    let mut q = conn.prepare(
        "SELECT f.id, b.crc32, COALESCE(b.hl_crc32, '') FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id JOIN blobs b ON b.id = f.blob_id
         WHERE r.game_id = ?1 AND f.missing_since IS NULL ORDER BY f.id",
    )?;
    let rows: Vec<(i64, String, String)> = q.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let roms: i64 = conn.query_row("SELECT COUNT(*) FROM dat_roms WHERE game_id = ?1", [id], |r| r.get(0))?;
    let pick = match want_crc.map(|c| format!("{c:08x}")) {
        Some(c) => rows.iter().find(|r| r.1.eq_ignore_ascii_case(&c) || r.2.eq_ignore_ascii_case(&c)).or(if roms == 1 { rows.first() } else { None }),
        None if roms == 1 => rows.first(),
        None => None,
    };
    let Some((fid, ..)) = pick else {
        if rows.is_empty() {
            bail!("You don't have {}'s file, so there's nothing to patch.", g.title);
        }
        bail!("{} is a game of several files (a disc's tracks): patching one of them comes later.", g.title);
    };
    let roots = crate::roots::list(lib);
    crate::convert::src_of(conn, &roots, *fid)?.ok_or_else(|| anyhow!("{}'s file isn't scanned yet.", g.title))
}

/// A file's bytes: the file, or its member of a ZIP or 7z.
pub(crate) fn read_src(s: &Src, cancel: &AtomicBool) -> Result<Vec<u8>> {
    match &s.member {
        None => Ok(std::fs::read(&s.path).with_context(|| format!("couldn't read {}", s.path))?),
        Some(m) => {
            let mut out = None;
            crate::archive::each_wanted(Path::new(&s.path), &|n| n == m, cancel, &mut |_, _, r| {
                let mut b = vec![];
                r.read_to_end(&mut b)?;
                out = Some(b);
                Ok(())
            })
            .with_context(|| format!("couldn't read {}", s.path))?;
            out.ok_or_else(|| anyhow!("{m} isn't in {} any more", s.path))
        }
    }
}

/// Apply patches in order. A patch made for the file without its copier header (an SNES
/// ROM's 512 bytes, an NES ROM's iNES 16) is applied to what follows it, and the header
/// put back.
pub fn apply_all(source: &[u8], patches: &[Vec<u8>]) -> Result<Vec<u8>> {
    let mut cur = source.to_vec();
    for p in patches {
        let i = rompatch::info(p)?;
        if let (Some(size), Some(crc)) = (i.source_size, i.source_crc) {
            let size = size as usize;
            if cur.len() > size && i.fits(cur.len() as u64, crc32fast::hash(&cur)) == Some(false) && crc32fast::hash(&cur[cur.len() - size..]) == crc {
                let head = cur.len() - size;
                let mut out = cur[..head].to_vec();
                out.extend(rompatch::apply(p, &cur[head..])?);
                cur = out;
                continue;
            }
        }
        cur = rompatch::apply(p, &cur)?;
    }
    Ok(cur)
}

// ------------------------------------------------------------ the Patches page

/// A game's folder in the library: the Library's, else its DAT game's (one made since the
/// Library's rows were last brought up to date).
pub fn game_dir(conn: &Connection, lib: &Library, g: &LibGame) -> Result<Option<PathBuf>> {
    if let Some(d) = g.folder.as_ref().and_then(|f| lib.resolve(f).ok()).filter(|d| d.join("game.json").is_file()) {
        return Ok(Some(d));
    }
    match g.dat_game {
        Some(id) => Ok(meta::folder_of(conn, &meta::dat_game(conn, id)?)?.and_then(|f| lib.resolve(&f).ok())),
        None => Ok(g.folder.as_ref().and_then(|f| lib.resolve(f).ok())),
    }
}

fn entry(manifest: &Value, name: &str) -> Value {
    manifest["files"].get(name).cloned().unwrap_or(json!({}))
}

/// What the Patches page lists: each patch kept, with its game.
pub fn list(ix: &Index, lib: &Library) -> Result<Value> {
    cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let mut q = conn.prepare("SELECT path, folder, name, format, kind, title, author, version, language, applied FROM patches ORDER BY folder, name")?;
    type Row = (String, String, String, String, String, String, String, String, String, String);
    let rows: Vec<Row> = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut g_of = conn.prepare(&format!("SELECT {} FROM library WHERE folder = ?1 ORDER BY addon != '', have DESC LIMIT 1", "key, kind, platform, dat_game, name, title, folder, picture"))?;
    let mut items = vec![];
    for (path, folder, name, format, kind, title, author, version, language, applied) in rows {
        let Some(g) = g_of.query_row([&folder], |r| Ok(LibGame { key: r.get(0)?, kind: r.get(1)?, platform: r.get(2)?, dat_game: r.get(3)?, name: r.get(4)?, title: r.get(5)?, folder: r.get(6)?, picture: r.get(7)? })).optional()? else {
            continue;
        };
        let mut o = shown(&g);
        o["id"] = json!(path);
        o["game_key"] = json!(g.key);
        o["game_title"] = json!(g.title);
        o["file"] = json!(name);
        o["title"] = json!(if title.is_empty() { name.clone() } else { title });
        o["format"] = json!(format);
        o["format_name"] = json!(rompatch::FORMATS.iter().find(|f| f.0 == format).map(|f| f.1).unwrap_or("?"));
        o["kind"] = json!(kind);
        o["kind_name"] = json!(kind_name(&kind));
        o["author"] = json!(author);
        o["version"] = json!(version);
        o["language"] = json!(language);
        let applied: Vec<&str> = applied.lines().filter(|l| !l.is_empty()).collect();
        o["applied"] = json!(applied.iter().map(|n| json!({ "name": n, "key": format!("{}/{n}", g.platform) })).collect::<Vec<_>>());
        items.push(o);
    }
    Ok(json!({ "items": items, "kinds": KINDS.iter().map(|k| json!({ "id": k.0, "name": k.1 })).collect::<Vec<_>>() }))
}

/// One game's patches, with what each says about itself and whether it fits the game's file.
pub fn game(ix: &Index, lib: &Library, key: &str) -> Result<Value> {
    cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let mut out = vec![];
    if let Some(dir) = game_dir(ix.conn(), lib, &g)?.map(|d| d.join(FOLDER)) {
        let manifest = read_json_object(&dir.join(MANIFEST));
        for (name, p) in files_in(&dir) {
            let e = entry(&manifest, &name);
            let d = Details::of(&e, &name);
            let bytes = std::fs::read(&p).unwrap_or_default();
            let info = rompatch::info(&bytes).ok();
            let readme = dir.join(format!("{name}.txt"));
            out.push(json!({
                "name": name, "details": d, "info": info, "applied": e["applied"], "added": e["added"], "from": e["from"],
                "readme": readme.is_file().then(|| cheats::read_text(&readme)),
                "path": lib.relative(&p), "size": bytes.len(),
            }));
        }
    }
    let mut v = shown(&g);
    v["patches"] = json!(out);
    v["kinds"] = json!(KINDS.iter().map(|k| json!({ "id": k.0, "name": k.1 })).collect::<Vec<_>>());
    // a variant names the patches it was made with
    if let Some(dir) = game_dir(ix.conn(), lib, &g)? {
        let gj = meta::read_game(&dir);
        if gj["patched"].is_object() {
            v["patched"] = gj["patched"].clone();
        }
    }
    Ok(v)
}

/// Change a patch's details (patches.json; the patch itself isn't touched).
pub fn edit(ix: &Index, lib: &Library, key: &str, name: &str, d: &Details) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let dir = game_dir(ix.conn(), lib, &g)?.map(|d| d.join(FOLDER)).ok_or_else(|| anyhow!("{} has no patches.", g.title))?;
    if !dir.join(name).is_file() || name.contains(['/', '\\']) {
        bail!("{} has no patch {name}.", g.title);
    }
    if !KINDS.iter().any(|k| k.0 == d.kind) {
        bail!("A patch's kind is one of Translation, Hack, Fix, Restoration or Other.");
    }
    let mut m = read_json_object(&dir.join(MANIFEST));
    let mut e = entry(&m, name);
    let fields = serde_json::to_value(d)?;
    for (k, v) in fields.as_object().into_iter().flatten() {
        e[k] = v.clone();
    }
    e["edited"] = json!(now());
    m["files"][name] = e;
    crate::config::write_json(&dir.join(MANIFEST), &sorted(&m))?;
    sync(ix, lib)?;
    game(ix, lib, key)
}

/// Take a patch away from its game: it and its readme moved to `patches/.removed/<when>/`
/// (a patched copy made with it stays: it's a game of its own).
pub fn remove(ix: &Index, lib: &Library, key: &str, name: &str) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let dir = game_dir(ix.conn(), lib, &g)?.map(|d| d.join(FOLDER)).ok_or_else(|| anyhow!("{} has no patches.", g.title))?;
    let p = dir.join(name);
    if name.contains(['/', '\\']) || !p.is_file() {
        bail!("{} has no patch {name}.", g.title);
    }
    let when = now().replace([':', 'T'], "-").trim_end_matches('Z').to_string();
    let to = dir.join(".removed").join(&when);
    std::fs::create_dir_all(&to)?;
    std::fs::rename(&p, to.join(name))?;
    let readme = dir.join(format!("{name}.txt"));
    if readme.is_file() {
        std::fs::rename(&readme, to.join(format!("{name}.txt")))?;
    }
    let mut m = read_json_object(&dir.join(MANIFEST));
    if let Some(files) = m["files"].as_object_mut() {
        if let Some(e) = files.remove(name) {
            crate::config::write_json(&to.join(MANIFEST), &json!({ "files": { name: e } }))?;
        }
    }
    crate::config::write_json(&dir.join(MANIFEST), &sorted(&m))?;
    sync(ix, lib)?;
    game(ix, lib, key)
}

// ------------------------------------------------------------ bringing patches in (D202)

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Found {
    /// The patch file (taken out of a ZIP into the data folder's `patch-import/`)
    pub path: String,
    /// The name it's kept under in the game's patches folder
    pub name: String,
    /// Its readme, kept beside it as `<name>.txt`
    pub readme: Option<String>,
    pub key: String,
    pub title: String,
    pub platform: String,
    /// sure (its source's CRC32 is a file you have) | likely (by name) | you | none
    pub pairing: String,
    pub format: String,
    pub details: Details,
    /// What *Test* said, when it was tried: fits, how, why
    pub test: Option<Value>,
    /// It can't be applied here (`xdelta3`)
    pub needs: String,
    pub include: bool,
    /// Apply it once it's in (ticked for sure pairs that fit)
    pub apply: bool,
    /// From a ZIP: the ZIP's name
    pub from: Option<String>,
}

/// The library's games whose file has this CRC32 and size (or without its copier header).
fn games_with(conn: &Connection, size: u64, crc: u32) -> Result<Vec<String>> {
    let c = format!("{crc:08x}");
    let mut q = conn.prepare(
        "SELECT DISTINCT l.key FROM blobs b JOIN files f ON f.blob_id = b.id JOIN matches m ON m.file_id = f.id JOIN dat_roms r ON r.id = m.rom_id
         JOIN library l ON l.dat_game = r.game_id
         WHERE ((lower(b.crc32) = ?1 AND b.size = ?2) OR (lower(b.hl_crc32) = ?1 AND b.hl_size = ?2)) AND f.missing_since IS NULL AND l.addon = ''",
    )?;
    let v = q.query_map(params![c, size as i64], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// Take a ZIP's patches and readmes out into `at` (each its own folder), as files to read.
fn unzip(zip: &Path, at: &Path) -> Result<Vec<PathBuf>> {
    let dir = at.join(organise::new_id());
    std::fs::create_dir_all(&dir)?;
    let mut out = vec![];
    let never = AtomicBool::new(false);
    crate::archive::each_wanted(zip, &|n| is_patch_name(n) || n.to_lowercase().ends_with(".txt") || n.to_lowercase().ends_with(".nfo"), &never, &mut |n, _, r| {
        let base = n.rsplit(['/', '\\']).next().unwrap_or(n);
        if base.is_empty() {
            return Ok(());
        }
        let p = dir.join(safe_name(base));
        let mut f = std::fs::File::create(&p)?;
        std::io::copy(r, &mut f)?;
        out.push(p);
        Ok(())
    })
    .with_context(|| format!("couldn't read {}", zip.display()))?;
    Ok(out)
}

/// Read patch files picked (or the patches in folders, and ZIPs of them, picked), each
/// paired with a game you have; or all with `key`'s game.
pub fn read_files(ix: &Index, lib: &Library, data_dir: &Path, paths: &[PathBuf], key: Option<&str>, platform: Option<&str>) -> Result<Vec<Found>> {
    cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let staging = data_dir.join("patch-import");
    let mut files: Vec<(PathBuf, Option<String>)> = vec![];
    let add = |p: PathBuf, files: &mut Vec<(PathBuf, Option<String>)>| -> Result<()> {
        let low = p.to_string_lossy().to_lowercase();
        if low.ends_with(".zip") || low.ends_with(".7z") {
            let zname = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            for x in unzip(&p, &staging)? {
                files.push((x, Some(zname.clone())));
            }
        } else {
            files.push((p, None));
        }
        Ok(())
    };
    for p in paths {
        if p.is_dir() {
            for e in walk(p) {
                add(e, &mut files)?;
            }
        } else if p.is_file() {
            add(p.clone(), &mut files)?;
        }
    }
    let given = key.map(|k| game_of(conn, k)).transpose()?;
    let platforms: Vec<String> = platform.map(|p| vec![p.to_string()]).unwrap_or_default();
    let games = if given.is_some() { vec![] } else { had_games(conn, &platforms)? };
    let mut by_title: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for (i, g) in games.iter().enumerate() {
        by_title.entry(words(split_name(&g.name).0)).or_default().push(i);
    }
    // readmes beside the patches: the one of the same name, else the only one in its folder
    let readme_for = |p: &Path, all: &[(PathBuf, Option<String>)]| -> Option<PathBuf> {
        let stem = p.file_stem()?.to_string_lossy().to_lowercase();
        let txts: Vec<&PathBuf> = all.iter().map(|x| &x.0).filter(|x| x.parent() == p.parent() && x.extension().is_some_and(|e| ["txt", "nfo"].contains(&e.to_string_lossy().to_lowercase().as_str()))).collect();
        txts.iter().find(|x| x.file_stem().is_some_and(|s| s.to_string_lossy().to_lowercase() == stem)).or(if txts.len() == 1 { txts.first() } else { None }).map(|x| (*x).clone())
    };
    let mut out = vec![];
    for (f, from) in &files {
        let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let bytes = std::fs::read(f).unwrap_or_default();
        let Some(format) = rompatch::detect(&bytes) else { continue };
        let info = rompatch::info(&bytes).ok();
        let readme = readme_for(f, &files);
        let text = readme.as_ref().map(|r| cheats::read_text(r)).unwrap_or_default();
        let details = guess(&name, &text);
        let sys = f.ancestors().skip(1).filter_map(|a| a.file_name()).find_map(|n| cheats::platform_of_folder(&n.to_string_lossy()));
        let (g, pairing): (Option<LibGame>, &str) = match &given {
            Some(g) => (Some(g.clone()), "you"),
            None => {
                let by_crc = match info.as_ref().and_then(|i| Some((i.source_size?, i.source_crc?))) {
                    Some((s, c)) => games_with(conn, s, c)?.into_iter().next().and_then(|k| game_of(conn, &k).ok()),
                    None => None,
                };
                match by_crc {
                    Some(g) => (Some(g), "sure"),
                    None => {
                        // the patch's name, else its ZIP's, by title (the most tags in common)
                        let fits = |i: &usize| sys.as_ref().is_none_or(|s| &games[*i].platform == s);
                        let names = [Some(name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(name.clone())), from.as_ref().map(|z| z.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(z.clone()))];
                        let mut best = None;
                        for n in names.into_iter().flatten() {
                            let clean = n.replace('_', " ");
                            let (_, tags) = title_and_tags(&clean);
                            let hit = by_title.get(&words(split_name(&clean).0)).and_then(|l| {
                                l.iter().filter(|i| fits(i)).max_by_key(|&&i| title_and_tags(&games[i].name).1.iter().filter(|t| tags.contains(t)).count())
                            });
                            if let Some(&i) = hit {
                                best = Some(games[i].clone());
                                break;
                            }
                        }
                        match best {
                            Some(g) => (Some(g), "likely"),
                            None => (None, "none"),
                        }
                    }
                }
            }
        };
        let needs = info.as_ref().map(|i| i.needs.clone()).unwrap_or_default();
        out.push(Found {
            path: f.display().to_string(), name: safe_name(&name), readme: readme.map(|r| r.display().to_string()),
            key: g.as_ref().map(|g| g.key.clone()).unwrap_or_default(), title: g.as_ref().map(|g| g.title.clone()).unwrap_or_default(),
            platform: g.as_ref().map(|g| g.platform.clone()).or(sys).unwrap_or_default(), pairing: pairing.into(), format: format.into(), details,
            test: None, needs: needs.clone(), include: g.is_some() && pairing != "likely", apply: pairing == "sure" && needs.is_empty(), from: from.clone(),
        });
    }
    Ok(out)
}

/// *Test*: the patch applied to the game's file in memory, and whether it fits.
pub fn test(ix: &Index, lib: &Library, key: &str, patch: &Path) -> Result<Value> {
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let bytes = std::fs::read(patch).with_context(|| format!("couldn't read {}", patch.display()))?;
    let info = rompatch::info(&bytes)?;
    let src = rom_of(conn, lib, &g, info.source_crc)?;
    let source = read_src(&src, &AtomicBool::new(false))?;
    let checks = info.target_crc.is_some() || info.block_at.is_some() || info.format == "xdelta";
    Ok(match apply_all(&source, &[bytes]) {
        Ok(out) => json!({ "fits": true, "how": if checks { "checked" } else { "applied" }, "size": out.len(), "crc": format!("{:08x}", crc32fast::hash(&out)) }),
        Err(e) => json!({ "fits": false, "why": e.to_string() }),
    })
}

/// A game's folder for its patches: its own, else one planned (its game.json written by the change).
fn folder_for(conn: &Connection, lib: &Library, g: &LibGame) -> Result<(PathBuf, Option<Value>)> {
    if let Some(d) = g.folder.as_ref().and_then(|x| lib.resolve(x).ok()).filter(|d| d.join("game.json").is_file()) {
        return Ok((d, None));
    }
    let id = g.dat_game.ok_or_else(|| anyhow!("{} has no folder in the library to keep its patches in.", g.title))?;
    let dg: DatGame = meta::dat_game(conn, id)?;
    if let Some(d) = meta::folder_of(conn, &dg)?.and_then(|x| lib.resolve(&x).ok()).filter(|d| d.join("game.json").is_file()) {
        return Ok((d, None));
    }
    let d = meta::games_root(lib).join(&dg.platform).join(safe_name(&dg.name));
    if d.join("game.json").is_file() {
        bail!("Another game already has {}'s folder in the library.", dg.name);
    }
    let gj = json!({
        "id": organise::new_id(), "platform": dg.platform, "name": dg.name, "dat": dg.dat, "title": dg.title, "created": now(),
        "roms": dg.roms.iter().map(|r| json!({ "name": r.name, "size": r.size, "crc32": r.crc32, "md5": r.md5, "sha1": r.sha1 })).collect::<Vec<_>>(),
    });
    Ok((d, Some(gj)))
}

fn empty_plan(kind: &str, label: String, title: String, lib: &Library, ops: Vec<Op>, games: usize, dat: Value, platform: &str) -> Plan {
    Plan {
        id: organise::new_id(),
        created: now(),
        kind: kind.into(),
        label,
        title,
        dat,
        platform: platform.into(),
        mode: "copy".into(),
        format: "keep".into(),
        out: Some(meta::games_root(lib).display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None,
        new_roots: vec![],
        present: vec![],
        export: None,
        brought: vec![],
    }
}

/// Plan bringing the patches found into their games' patches folders: one change, with Undo.
/// A patch already there (the same bytes) isn't brought again; a name taken gets a number.
pub fn plan(ix: &Index, lib: &Library, found: &[Found]) -> Result<Plan> {
    lib.writable()?;
    let conn = ix.conn();
    let items: Vec<&Found> = found.iter().filter(|f| f.include && !f.key.is_empty()).collect();
    if items.is_empty() {
        bail!("Choose at least one patch to bring in.");
    }
    struct Target {
        label: String,
        dir: PathBuf,
        manifest: Value,
        taken: HashSet<String>,
        sums: HashSet<String>,
        ops: Vec<Op>,
        new_game: Option<Value>,
    }
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    let mut applying = vec![];
    for f in &items {
        let g = game_of(conn, &f.key)?;
        let (dir, new_game) = folder_for(conn, lib, &g)?;
        let t = targets.entry(organise::key(&dir)).or_insert_with(|| {
            let pd = dir.join(FOLDER);
            let manifest = read_json_object(&pd.join(MANIFEST));
            let existing = files_in(&pd);
            let sums = existing.iter().filter_map(|(_, p)| std::fs::read(p).ok()).map(|b| cheats::sha1_hex(&b)).collect();
            let mut taken: HashSet<String> = existing.into_iter().map(|(n, _)| n.to_lowercase()).collect();
            for e in std::fs::read_dir(&pd).into_iter().flatten().flatten() {
                taken.insert(e.file_name().to_string_lossy().to_lowercase());
            }
            Target { label: g.name.clone(), dir: pd, manifest, taken, sums, ops: vec![], new_game }
        });
        let src = Path::new(&f.path);
        let bytes = std::fs::read(src).with_context(|| format!("couldn't read {}", f.path))?;
        let sum = cheats::sha1_hex(&bytes);
        if !t.sums.insert(sum.clone()) {
            continue;
        }
        let mut name = safe_name(&f.name);
        if t.taken.contains(&name.to_lowercase()) {
            let (s, x) = name.rsplit_once('.').map(|(s, x)| (s.to_string(), x.to_string())).unwrap_or((name.clone(), String::new()));
            let mut n = 2;
            while t.taken.contains(&name.to_lowercase()) {
                name = format!("{s} ({n}).{x}");
                n += 1;
            }
        }
        t.taken.insert(name.to_lowercase());
        let mut s = outside(src);
        s.size = bytes.len() as u64;
        t.ops.push(Op { op: "copy".into(), game: t.label.clone(), to: t.dir.join(&name).display().to_string(), from: vec![s], names: vec![], aside: vec![], size: bytes.len() as u64, note: Some("a patch".into()) });
        if let Some(r) = &f.readme {
            let text = cheats::read_text(Path::new(r));
            t.ops.push(text_op(&t.label, &t.dir.join(format!("{name}.txt")), text, "the patch's readme"));
        }
        let mut e = serde_json::to_value(&f.details)?;
        e["format"] = json!(f.format);
        e["added"] = json!(now());
        e["sha1"] = json!(sum);
        e["from"] = json!(f.from.clone().unwrap_or_else(|| src.file_name().unwrap_or_default().to_string_lossy().into_owned()));
        t.manifest["files"][&name] = e;
        if f.apply {
            applying.push(json!({ "key": f.key, "name": name }));
        }
    }
    let mut ops = vec![];
    let mut n = 0;
    for t in targets.into_values() {
        if t.ops.is_empty() {
            continue;
        }
        n += 1;
        if let Some(gj) = &t.new_game {
            let p = t.dir.parent().unwrap_or(&t.dir).join("game.json");
            ops.push(text_op(&t.label, &p, serde_json::to_string_pretty(&sorted(gj))? + "\n", "game.json"));
        }
        ops.extend(t.ops);
        ops.push(text_op(&t.label, &t.dir.join(MANIFEST), serde_json::to_string_pretty(&sorted(&t.manifest))? + "\n", "patches.json: the patches' kinds and details"));
    }
    if ops.is_empty() {
        bail!("Those patches are with their games already.");
    }
    let games = |n: usize| format!("{n} game{}", if n == 1 { "" } else { "s" });
    Ok(empty_plan("patches", format!("Brought in patches for {}", games(n)), format!("Bring in patches for {}", games(n)), lib, ops, n, json!({ "apply": applying }), ""))
}

// ------------------------------------------------------------ applying (D201)

/// The patched copy's name: the game's, with the patches' tags (*English Translated*, *Hack*,
/// *Bug Fix*, *Restoration*) and each patch's title and version.
pub fn variant_name(original: &str, patches: &[Details]) -> String {
    let mut tags: Vec<String> = variants::tags_of(original);
    for d in patches {
        let tag = match (d.kind.as_str(), d.language.as_str()) {
            ("translation", "English") => "English Translated".to_string(),
            ("translation", "") => "Translated".to_string(),
            ("translation", l) => format!("{l} Translated"),
            (k, _) => KINDS.iter().find(|x| x.0 == k).map(|x| x.2).unwrap_or("Hack").to_string(),
        };
        if !tags.contains(&tag) {
            tags.push(tag);
        }
        let mut t = d.title.trim().to_string();
        // the game's own title isn't the patch's name
        if words(&t) == words(split_name(original).0) || t.is_empty() {
            t = String::new();
        }
        let label = [t, if d.version.is_empty() { String::new() } else { format!("v{}", d.version.trim_start_matches(['v', 'V'])) }]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let label = label.replace(['(', ')', '[', ']'], "");
        if !label.is_empty() && !tags.contains(&label) {
            tags.push(label);
        }
    }
    safe_name(&variants::compose(split_name(original).0, &tags, ""))
}

/// Plan *Apply*: the game's file with these patches (in order) written beside it as a new
/// file (zipped when the game's is), added to the custom DAT as a variant once it's written.
pub fn apply_plan(ix: &Index, lib: &Library, key: &str, names: &[String], name: Option<&str>) -> Result<Plan> {
    lib.writable()?;
    cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    if names.is_empty() {
        bail!("Choose the patches to apply.");
    }
    if g.kind != "dat" || g.dat_game.is_none() {
        bail!("{} isn't a game of a DAT: a patched copy is a variant of one.", g.title);
    }
    if variants::is_custom(conn, conn.query_row("SELECT dat_id FROM dat_games WHERE id = ?1", [g.dat_game.unwrap_or(-1)], |r| r.get(0))?)? {
        bail!("{} is a variant already: apply the patches to its original.", g.title);
    }
    let dir = game_dir(ix.conn(), lib, &g)?.map(|d| d.join(FOLDER)).ok_or_else(|| anyhow!("{} has no patches.", g.title))?;
    let manifest = read_json_object(&dir.join(MANIFEST));
    let mut bytes = vec![];
    let mut details = vec![];
    let mut paths = vec![];
    for n in names {
        let p = dir.join(n);
        if n.contains(['/', '\\']) || !p.is_file() {
            bail!("{} has no patch {n}.", g.title);
        }
        let b = std::fs::read(&p)?;
        let i = rompatch::info(&b)?;
        if !i.needs.is_empty() {
            bail!("{n} was made with xdelta3's own compression, which Game Repo can't read yet.");
        }
        bytes.push(b);
        details.push(Details::of(&entry(&manifest, n), n));
        paths.push(p.display().to_string());
    }
    let first_crc = rompatch::info(&bytes[0]).ok().and_then(|i| i.source_crc);
    let src = rom_of(conn, lib, &g, first_crc)?;
    let source = read_src(&src, &AtomicBool::new(false))?;
    let out = apply_all(&source, &bytes).with_context(|| format!("{} doesn't fit {}", names.join(", "), g.title))?;
    let vname = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => safe_name(n),
        None => variant_name(&g.name, &details),
    };
    if vname == g.name {
        bail!("The patched copy needs a name of its own.");
    }
    // the ROM's own extension, in a ZIP when the game's file is one
    let rom_name = src.member.clone().unwrap_or_else(|| Path::new(&src.path).file_name().unwrap_or_default().to_string_lossy().into_owned());
    let ext = crate::platforms::ext_of(&rom_name);
    let inner = if ext.is_empty() { vname.clone() } else { format!("{vname}.{ext}") };
    let beside = Path::new(&src.path).parent().unwrap_or(Path::new(".")).to_path_buf();
    let zipped = src.member.is_some() && src.path.to_lowercase().ends_with(".zip");
    let to = beside.join(if zipped { format!("{vname}.zip") } else { inner.clone() });
    if to.exists() {
        bail!("There's a file called {} beside {} already.", to.file_name().unwrap_or_default().to_string_lossy(), g.title);
    }
    let mut names_op = vec![if zipped { inner.clone() } else { String::new() }];
    names_op.extend(paths.iter().cloned());
    let applied_names = names.to_vec();
    // the original's patches.json says each was applied, to this copy
    let mut m = manifest.clone();
    for n in &applied_names {
        let mut e = entry(&m, n);
        let mut a: Vec<Value> = e["applied"].as_array().cloned().unwrap_or_default();
        if !a.contains(&json!(vname)) {
            a.push(json!(vname));
        }
        e["applied"] = json!(a);
        m["files"][n] = e;
    }
    let ops = vec![
        Op {
            op: "patch".into(), game: vname.clone(), to: to.display().to_string(), from: vec![src.clone()], names: names_op, aside: vec![],
            size: out.len() as u64, note: Some(format!("{} patched with {}", g.name, applied_names.join(", "))),
        },
        text_op(&g.name, &dir.join(MANIFEST), serde_json::to_string_pretty(&sorted(&m))? + "\n", "patches.json: the patches applied"),
    ];
    let dat = json!({
        "variant": vname, "original": g.name, "key": g.key, "patches": applied_names, "details": details,
        "source_sha1": cheats::sha1_hex(&source), "sha1": cheats::sha1_hex(&out), "crc32": format!("{:08x}", crc32fast::hash(&out)), "size": out.len(),
    });
    Ok(empty_plan("patch", format!("Made {vname}"), format!("Make {vname}"), lib, ops, 1, dat, &g.platform))
}

/// Run a `patch` step: the source read (from its archive), the patches applied, the result
/// written (in a TorrentZip when `names[0]` names its member) and checked.
pub(crate) fn perform(op: &Op, cancel: &AtomicBool) -> Result<organise::Written> {
    let src = op.from.first().ok_or_else(|| anyhow!("a patch step needs its source"))?;
    let source = read_src(src, cancel)?;
    let patches: Vec<Vec<u8>> = op.names.iter().skip(1).map(|p| std::fs::read(p).with_context(|| format!("couldn't read the patch {p}"))).collect::<Result<_>>()?;
    let out = apply_all(&source, &patches)?;
    let to = Path::new(&op.to);
    if to.exists() {
        bail!("{} is there already", to.display());
    }
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = to.with_file_name(format!("{}.gamerepo-tmp", to.file_name().unwrap_or_default().to_string_lossy()));
    let member = op.names.first().cloned().unwrap_or_default();
    let written = if member.is_empty() {
        std::fs::write(&tmp, &out)?;
        let h = crate::hash::hash_reader(&mut out.as_slice(), out.len() as u64, &op.to, cancel)?;
        organise::Written { members: vec![], file: Some(h), kind: "file" }
    } else {
        let size = out.len() as u64;
        let data = std::sync::Arc::new(out);
        let d = data.clone();
        let entries = vec![crate::torrentzip::Entry { name: member, size, open: Box::new(move || Ok(Box::new(std::io::Cursor::new(d.to_vec())) as Box<dyn Read>)) }];
        let members = crate::torrentzip::write(&tmp, entries, cancel)?;
        organise::Written { members, file: None, kind: "zip" }
    };
    if to.exists() {
        let _ = std::fs::remove_file(&tmp);
        bail!("{} is there already", to.display());
    }
    std::fs::rename(&tmp, to)?;
    Ok(written)
}

/// After a `patch` change: the new file added to the custom DAT as a variant of its original,
/// its game.json saying how it was made; after its Undo, taken out again.
pub fn after(ix: &Index, lib: &Library, plan: &Plan, undone: bool) -> Result<()> {
    let d = &plan.dat;
    let (Some(vname), Some(original)) = (d["variant"].as_str(), d["original"].as_str()) else { return Ok(()) };
    let platform = plan.platform.as_str();
    let conn = ix.conn();
    if undone {
        if variants::read(lib, platform)?.games.iter().any(|g| g.name == vname) {
            variants::remove(ix, lib, platform, &[vname.to_string()])?;
        }
        return Ok(());
    }
    let Some(op) = plan.ops.iter().find(|o| o.op == "patch") else { return Ok(()) };
    let roots = crate::roots::list(lib);
    let to = Path::new(&op.to);
    let Some((root, rel)) = roots.iter().find_map(|r| to.strip_prefix(&r.dir).ok().map(|x| (r.id.clone(), x.to_string_lossy().replace('\\', "/")))) else {
        bail!("{} isn't in a folder with games, so it can't be added to the custom DAT.", op.to);
    };
    let member = op.names.first().filter(|m| !m.is_empty());
    let rel = match member {
        Some(m) => format!("{rel}/{m}"),
        None => rel,
    };
    let file: Option<i64> = conn.query_row("SELECT id FROM files WHERE root_id = ?1 AND rel_path = ?2", params![root, rel], |r| r.get(0)).optional()?;
    let file = file.ok_or_else(|| anyhow!("{} wasn't found in the index after it was written.", op.to))?;
    variants::add(ix, lib, platform, &[variants::Add { files: vec![file], name: vname.to_string(), original: Some(original.to_string()) }])?;
    // its folder says which patches made it, and from what
    if let Some(dg) = meta::dat_game_named(conn, lib, platform, vname)? {
        if variants::is_custom(conn, conn.query_row("SELECT dat_id FROM dat_games WHERE id = ?1", [dg.id], |r| r.get(0))?)? {
            let dir = meta::ensure_folder(ix, lib, &dg)?;
            let mut v = meta::read_game(&dir);
            let mut p = Map::new();
            p.insert("original".into(), json!(original));
            p.insert("original_sha1".into(), d["source_sha1"].clone());
            p.insert("patches".into(), json!(d["patches"].as_array().cloned().unwrap_or_default().iter().zip(d["details"].as_array().cloned().unwrap_or_default()).map(|(n, det)| {
                let mut x = det.clone();
                x["file"] = n.clone();
                x
            }).collect::<Vec<_>>()));
            p.insert("made".into(), json!(now()));
            v["patched"] = Value::Object(p);
            meta::write_game(ix, lib, &dir, &v)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patchs_kind_language_version_and_author_are_guessed() {
        let d = guess("FF5_RPGe_v1.1.ips", "Final Fantasy V\nTranslated by RPGe\n");
        assert_eq!((d.kind.as_str(), d.language.as_str(), d.version.as_str(), d.author.as_str()), ("translation", "", "1.1", "RPGe"), "{d:?}");
        let d = guess("Super Mario World - Kaizo (Hack) v2.bps", "");
        assert_eq!((d.kind.as_str(), d.version.as_str()), ("hack", "2"));
        let d = guess("Zelda3 Bug Fix.ips", "");
        assert_eq!(d.kind, "fix");
        let d = guess("mother3_traducción_español.xdelta", "");
        assert_eq!((d.kind.as_str(), d.language.as_str()), ("translation", "Spanish"));
        let d = guess("patch.ups", "Version 1.03\nThis restoration patch puts back the original sprites.\nAuthor: Someone\n");
        assert_eq!((d.kind.as_str(), d.version.as_str(), d.author.as_str()), ("restoration", "1.03", "Someone"));
        assert_eq!(version_in("Vagrant Story"), None);
    }

    #[test]
    fn a_patched_copy_is_named_after_its_game_with_the_patches_tags() {
        let rpge = Details { kind: "translation".into(), title: "RPGe".into(), version: "1.1".into(), language: "English".into(), ..Default::default() };
        assert_eq!(variant_name("Final Fantasy V (Japan)", &[rpge.clone()]), "Final Fantasy V (Japan) (English Translated) (RPGe v1.1)");
        let fix = Details { kind: "fix".into(), title: "Final Fantasy V".into(), ..Default::default() };
        assert_eq!(variant_name("Final Fantasy V (Japan)", &[rpge, fix]), "Final Fantasy V (Japan) (English Translated) (RPGe v1.1) (Bug Fix)");
    }

    #[test]
    fn a_patch_is_paired_by_its_crc_brought_in_applied_as_a_variant_and_undone() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("patches");
        let cancel = AtomicBool::new(false);
        let delta = crate::audit::tests::rom(8, 1024);
        let mut target = delta.clone();
        target[10..20].copy_from_slice(b"TRANSLATED");
        target.extend(b"MORE");
        let inbox = dir.join("downloads");
        std::fs::create_dir_all(&inbox).unwrap();
        std::fs::write(inbox.join("dlt_eng_v1.0.ups"), rompatch::tests::make_ups(&delta, &target)).unwrap();
        std::fs::write(inbox.join("readme.txt"), "Delta English translation\nTranslated by Someone\n").unwrap();
        // an IPS says nothing of its source: paired by name, or not at all
        std::fs::write(inbox.join("Delta (World) [Hack].ips"), b"PATCHEOF").unwrap();
        let found = read_files(&ix, &lib, &dir.join("data"), &[inbox.clone()], None, None).unwrap();
        let ups = found.iter().find(|f| f.format == "ups").unwrap();
        assert_eq!((ups.key.as_str(), ups.pairing.as_str(), ups.include, ups.apply), ("gb/Delta (World)", "sure", true, true), "{ups:?}");
        assert_eq!((ups.details.kind.as_str(), ups.details.language.as_str(), ups.details.version.as_str(), ups.details.author.as_str()), ("translation", "English", "1.0", "Someone"));
        assert!(ups.readme.is_some());
        let ips = found.iter().find(|f| f.format == "ips").unwrap();
        assert_eq!((ips.key.as_str(), ips.pairing.as_str(), ips.include), ("gb/Delta (World)", "likely", false), "{ips:?}");
        let t = test(&ix, &lib, "gb/Delta (World)", Path::new(&ups.path)).unwrap();
        assert_eq!(t["fits"], json!(true), "{t}");
        assert_eq!(t["how"], json!("checked"));
        // brought in: the patch and its readme in the game's folder
        let p = plan(&ix, &lib, &found).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        let listed = list(&ix, &lib).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{listed}");
        assert_eq!(items[0]["kind"], json!("translation"));
        assert_eq!(items[0]["game_key"], json!("gb/Delta (World)"));
        let g = game(&ix, &lib, "gb/Delta (World)").unwrap();
        assert_eq!(g["patches"][0]["name"], json!("dlt_eng_v1.0.ups"));
        assert!(g["patches"][0]["readme"].as_str().unwrap().contains("Someone"));
        assert_eq!(g["patches"][0]["info"]["source_size"], json!(1024));
        // brought in again: nothing new
        assert!(plan(&ix, &lib, &found).is_err());
        // applied: a copy beside the game, a variant of it in the custom DAT
        let d = Details { kind: "translation".into(), title: "DLT".into(), version: "1.0".into(), language: "English".into(), author: "Someone".into(), ..Default::default() };
        edit(&ix, &lib, "gb/Delta (World)", "dlt_eng_v1.0.ups", &d).unwrap();
        let a = apply_plan(&ix, &lib, "gb/Delta (World)", &["dlt_eng_v1.0.ups".into()], None).unwrap();
        let vname = "Delta (World) (English Translated) (DLT v1.0)";
        assert_eq!(a.dat["variant"], json!(vname));
        organise::run(&ix, &lib, &a, &cancel, &mut |_| {}).unwrap();
        let made = dir.join("Roms").join(format!("{vname}.gb"));
        assert_eq!(std::fs::read(&made).unwrap(), target);
        assert_eq!(std::fs::read(dir.join("Roms/Delta (World).gb")).unwrap(), delta, "the game's own file isn't changed");
        let custom = variants::read(&lib, "gb").unwrap();
        let v = custom.games.iter().find(|g| g.name == vname).expect("the variant in the custom DAT");
        assert_eq!(v.cloneof.as_deref(), Some("Delta (World)"));
        let dg = meta::dat_game_named(ix.conn(), &lib, "gb", vname).unwrap().unwrap();
        let vdir = lib.resolve(&meta::folder_of(ix.conn(), &dg).unwrap().unwrap()).unwrap();
        let gj = meta::read_game(&vdir);
        assert_eq!(gj["patched"]["original"], json!("Delta (World)"));
        assert_eq!(gj["patched"]["patches"][0]["file"], json!("dlt_eng_v1.0.ups"));
        assert_eq!(gj["patched"]["original_sha1"], json!(cheats::sha1_hex(&delta)));
        let listed = list(&ix, &lib).unwrap();
        assert_eq!(listed["items"][0]["applied"][0]["name"], json!(vname), "{listed}");
        // the file matched as the variant
        let matched: i64 = ix.conn().query_row(
            "SELECT COUNT(*) FROM files f JOIN matches m ON m.file_id = f.id JOIN dat_roms r ON r.id = m.rom_id JOIN dat_games g ON g.id = r.game_id WHERE f.name = ?1 AND g.name = ?2",
            params![format!("{vname}.gb"), vname], |r| r.get(0)).unwrap();
        assert_eq!(matched, 1);
        // a patch that doesn't fit says so
        let other = dir.join("other.ups");
        std::fs::write(&other, rompatch::tests::make_ups(&crate::audit::tests::rom(99, 1024), &target)).unwrap();
        assert_eq!(test(&ix, &lib, "gb/Delta (World)", &other).unwrap()["fits"], json!(false));
        // Undo: the copy goes, and the custom DAT drops it
        organise::undo(&ix, &lib, &a.id, &cancel, &mut |_| {}).unwrap();
        assert!(!made.exists());
        assert!(!variants::read(&lib, "gb").unwrap().games.iter().any(|g| g.name == vname));
        sync(&ix, &lib).unwrap();
        assert_eq!(list(&ix, &lib).unwrap()["items"][0]["applied"], json!([]));
        // taken away: kept aside, not listed
        remove(&ix, &lib, "gb/Delta (World)", "dlt_eng_v1.0.ups").unwrap();
        assert_eq!(list(&ix, &lib).unwrap()["items"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_patch_for_the_rom_without_its_header_is_applied_after_it() {
        let rom: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
        let mut target = rom.clone();
        target[100] = 0xFF;
        let p = rompatch::tests::make_ups(&rom, &target);
        let mut headered = vec![0u8; 512];
        headered.extend(&rom);
        let out = apply_all(&headered, &[p.clone()]).unwrap();
        assert_eq!(&out[..512], &[0u8; 512][..]);
        assert_eq!(&out[512..], &target[..]);
        assert_eq!(apply_all(&rom, &[p]).unwrap(), target);
    }
}
