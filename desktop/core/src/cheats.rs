//! Cheats (docs/PLAN.md, "Cheats and patches", D197–D199): each game's cheat files, kept in
//! its folder's `cheats/` (`_library/games/<system>/<Game>/cheats/`) with `cheats.json`
//! saying each one's format, source, when it was fetched and whether it's been edited here.
//! RetroArch's `.cht` is the usual format, read and written as a list of cheats; the
//! emulators' own (PCSX2's `.pnach`, PPSSPP's CWCheat `.ini`, Dolphin's game `.ini`,
//! DuckStation's `.cht`, Azahar's and Eden's `.txt`, melonDS's `.mch`) are kept and edited
//! as text, their cheats counted and named for the list.
//!
//! *Get cheats…* reads RetroArch's own cheat pack (libretro-database's `cht/`, CC BY-SA
//! 4.0): the `cheats.zip` its Online Updater fetches, kept a week in the data folder, or,
//! when that can't be reached, the same files from GitHub. Each file is matched to the
//! games you have by its name (the DAT game's name is *sure*, the same title *likely*), and
//! they come in as one change with Undo. A file edited here is never replaced by a newer
//! one; the newer one is offered beside it.
//!
//! Saving a file keeps the one before in `cheats/.versions/<when>/` (the last twenty of
//! each), so an edit can be gone back on.

use crate::config::{read_json_object, sorted, write_json};
use crate::docs::text_op;
use crate::index::Index;
use crate::library::{now, Library};
use crate::meta::{self, DatGame};
use crate::organise::{self, safe_name, Op, Plan};
use crate::variants::{split_name, tags_of, words};
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// A game folder's cheats folder, and the file listing them.
pub const FOLDER: &str = "cheats";
pub const MANIFEST: &str = "cheats.json";
/// Where an edited file's earlier versions are kept, in its cheats folder.
pub const VERSIONS: &str = ".versions";
/// How many versions of each file are kept.
pub const KEEP: usize = 20;
/// RetroArch's cheat pack, as its Online Updater fetches it.
pub const PACK_URL: &str = "https://buildbot.libretro.com/assets/frontend/cheats.zip";
/// The same files on GitHub: the folder's list, then each file.
pub const GITHUB_API: &str = "https://api.github.com/repos/libretro/libretro-database";
pub const GITHUB_RAW: &str = "https://raw.githubusercontent.com/libretro/libretro-database/master/cht/";
/// What the pack is, for the page to credit.
pub const CREDIT: &str = "Cheats from libretro-database (github.com/libretro/libretro-database), CC BY-SA 4.0.";
/// How long the pack is kept before it's fetched again (seconds).
const PACK_AGE: u64 = 7 * 24 * 3600;

/// The formats: id, name, the extension a new file gets, and the systems it's for (empty: any).
pub const FORMATS: &[(&str, &str, &str, &[&str])] = &[
    ("retroarch", "RetroArch", "cht", &[]),
    ("pcsx2", "PCSX2", "pnach", &["ps2"]),
    ("ppsspp", "PPSSPP (CWCheat)", "ini", &["psp"]),
    ("dolphin", "Dolphin", "ini", &["gc", "wii"]),
    ("duckstation", "DuckStation", "cht", &["psx"]),
    ("azahar", "Azahar", "txt", &["3ds"]),
    ("eden", "Eden and yuzu", "txt", &["switch"]),
    ("melonds", "melonDS", "mch", &["nds"]),
    ("notes", "Notes", "txt", &[]),
];

pub fn format_name(id: &str) -> &'static str {
    FORMATS.iter().find(|f| f.0 == id).map(|f| f.1).unwrap_or("Other")
}

/// The formats a system's cheats can be in, RetroArch's first (a PC game's are notes).
pub fn formats_for(platform: &str) -> Vec<&'static str> {
    if platform == "pc" {
        return vec!["notes"];
    }
    FORMATS.iter().filter(|f| f.3.is_empty() || f.3.contains(&platform)).map(|f| f.0).collect()
}

/// A file's format, from its name and what's in it.
pub fn guess_format(name: &str, text: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let has_line = |f: &dyn Fn(&str) -> bool| text.lines().map(str::trim).any(|l| f(l));
    match ext.as_str() {
        "pnach" => "pcsx2",
        "mch" => "melonds",
        "cht" if has_line(&|l| l.starts_with("cheats") && l.contains('=')) || text.trim().is_empty() => "retroarch",
        "cht" => "duckstation",
        "ini" if has_line(&|l| l.starts_with("_C") || l.starts_with("_S ")) => "ppsspp",
        "ini" => "dolphin",
        "txt" if has_line(&|l| l.starts_with('[') && l.ends_with(']')) && has_line(&|l| is_code_line(l)) => "azahar",
        "txt" | "md" => "notes",
        _ => "notes",
    }
}

/// A line of two words of eight hexadecimal digits ("0D1A2B3C 00000063").
fn is_code_line(l: &str) -> bool {
    let w: Vec<&str> = l.split_whitespace().collect();
    w.len() >= 2 && w.iter().take(2).all(|x| x.len() == 8 && x.chars().all(|c| c.is_ascii_hexdigit()))
}

// ------------------------------------------------------------ RetroArch's .cht

/// One cheat in a `.cht`: its description, code, whether it's on at start, and its other
/// keys (the memory search's, `cheatN_handler` and so on) kept as they were.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Cheat {
    pub desc: String,
    pub code: String,
    pub enable: bool,
    pub extra: BTreeMap<String, String>,
}

/// A `.cht`'s cheats (and the file's own keys besides `cheats`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Cht {
    pub cheats: Vec<Cheat>,
    pub head: BTreeMap<String, String>,
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    let v = v.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(v);
    v.replace("\\\"", "\"")
}

/// Read a RetroArch `.cht`. Lines that aren't `key = value` are passed over.
pub fn parse_cht(text: &str) -> Cht {
    let mut by: BTreeMap<usize, Cheat> = BTreeMap::new();
    let mut head = BTreeMap::new();
    let mut count = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), unquote(v));
        if k == "cheats" {
            count = v.parse::<usize>().ok();
            continue;
        }
        let Some(rest) = k.strip_prefix("cheat") else {
            head.insert(k.to_string(), v);
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let Ok(n) = digits.parse::<usize>() else {
            head.insert(k.to_string(), v);
            continue;
        };
        let key = rest[digits.len()..].trim_start_matches('_');
        let c = by.entry(n).or_default();
        match key {
            "desc" => c.desc = v,
            "code" => c.code = v,
            "enable" => c.enable = v.eq_ignore_ascii_case("true") || v == "1",
            _ => {
                c.extra.insert(key.to_string(), v);
            }
        }
    }
    let mut cheats: Vec<Cheat> = by.into_values().collect();
    if let Some(n) = count {
        cheats.truncate(n.max(cheats.iter().rposition(|c| !c.code.is_empty() || !c.desc.is_empty()).map(|i| i + 1).unwrap_or(0)));
    }
    Cht { cheats, head }
}

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('"', "\\\""))
}

/// Write a `.cht` as RetroArch does: `cheats = N`, then each cheat's keys, numbered again.
pub fn write_cht(c: &Cht) -> String {
    let mut s = String::new();
    for (k, v) in &c.head {
        s.push_str(&format!("{k} = {}\n", quote(v)));
    }
    s.push_str(&format!("cheats = {}\n", c.cheats.len()));
    for (i, ch) in c.cheats.iter().enumerate() {
        s.push('\n');
        s.push_str(&format!("cheat{i}_desc = {}\n", quote(&ch.desc)));
        s.push_str(&format!("cheat{i}_code = {}\n", quote(ch.code.trim())));
        s.push_str(&format!("cheat{i}_enable = {}\n", if ch.enable { "true" } else { "false" }));
        for (k, v) in &ch.extra {
            // numbers and booleans as RetroArch writes them, unquoted
            let plain = v.parse::<f64>().is_ok() || v == "true" || v == "false";
            s.push_str(&format!("cheat{i}_{k} = {}\n", if plain { v.clone() } else { quote(v) }));
        }
    }
    s
}

/// The names of the cheats in a file of any format.
pub fn names_in(format: &str, text: &str) -> Vec<String> {
    match format {
        "retroarch" => parse_cht(text).cheats.into_iter().map(|c| c.desc).collect(),
        "ppsspp" => text.lines().filter_map(|l| l.trim().strip_prefix("_C").map(|r| r.get(1..).unwrap_or("").trim().to_string())).collect(),
        "dolphin" => {
            // the `$name` lines of its code sections
            let mut on = false;
            let mut v = vec![];
            for l in text.lines().map(str::trim) {
                if l.starts_with('[') {
                    on = matches!(l, "[ActionReplay]" | "[Gecko]");
                } else if on {
                    if let Some(n) = l.strip_prefix('$') {
                        v.push(n.trim().to_string());
                    }
                }
            }
            v
        }
        "pcsx2" => {
            // `[name]` blocks (PCSX2 2.x), else its `comment=` lines
            let blocks: Vec<String> = text.lines().map(str::trim).filter_map(|l| l.strip_prefix('[').and_then(|r| r.strip_suffix(']')).map(|n| n.trim().to_string())).collect();
            if !blocks.is_empty() {
                blocks
            } else {
                text.lines().filter_map(|l| l.trim().strip_prefix("comment=").map(|n| n.trim().to_string())).collect()
            }
        }
        "duckstation" | "azahar" | "eden" | "melonds" => text
            .lines()
            .map(str::trim)
            .filter_map(|l| l.strip_prefix('[').and_then(|r| r.strip_suffix(']')))
            .map(|n| n.trim().trim_matches('*').trim().to_string())
            .filter(|n| !n.is_empty())
            .collect(),
        _ => vec![],
    }
}

// ------------------------------------------------------------ a game's cheats folder

/// A file of a game's cheats, as cheats.json says.
fn entry_of(manifest: &Value, name: &str, text: &str) -> (String, Option<String>, bool, Option<String>) {
    let e = &manifest["files"][name];
    let format = e["format"].as_str().filter(|f| FORMATS.iter().any(|x| x.0 == *f)).map(String::from).unwrap_or_else(|| guess_format(name, text).to_string());
    (format, e["source"].as_str().map(String::from), e["edited"].as_bool().unwrap_or(false), e["fetched"].as_str().map(String::from))
}

fn stamp(p: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len() as i64, t))
}

/// The cheat files in a cheats folder: not cheats.json, its versions, or what's being written.
fn files_in(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| n != MANIFEST && !n.starts_with('.') && !n.ends_with(".gamerepo-tmp"))
        .collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

/// What a cheats folder holds, as a stamp: each file's name, size and time, and cheats.json's.
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

/// Bring the index's cheats up to date with the game folders': a folder read again only when
/// its files or cheats.json changed.
pub fn sync(ix: &Index, lib: &Library) -> Result<()> {
    let conn = ix.conn();
    let known: HashMap<String, String> = {
        let mut q = conn.prepare("SELECT folder, sig FROM cheat_folders")?;
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
            index_one(&tx, lib, &folder, &platform, &dir, &sig)?;
        }
    }
    for f in known.keys().filter(|f| !seen.contains(*f)) {
        tx.execute("DELETE FROM cheats WHERE folder = ?1", [f])?;
        tx.execute("DELETE FROM cheat_folders WHERE folder = ?1", [f])?;
    }
    tx.commit()?;
    Ok(())
}

fn index_one(conn: &Connection, lib: &Library, folder: &str, platform: &str, dir: &Path, sig: &str) -> Result<()> {
    conn.execute("DELETE FROM cheats WHERE folder = ?1", [folder])?;
    let manifest = read_json_object(&dir.join(MANIFEST));
    for (name, p) in files_in(dir) {
        let text = read_text(&p);
        let (format, source, edited, _) = entry_of(&manifest, &name, &text);
        let names = names_in(&format, &text);
        let Some(rel) = lib.relative(&p) else { continue };
        let (size, mtime) = stamp(&p).unwrap_or((0, 0));
        conn.execute(
            "INSERT OR REPLACE INTO cheats (path, folder, platform, name, format, count, names, source, edited, size, mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![rel, folder, platform, name, format, names.len() as i64, names.join("\n"), source, edited as i64, size, mtime],
        )?;
    }
    conn.execute("INSERT OR REPLACE INTO cheat_folders (folder, sig) VALUES (?1, ?2)", params![folder, sig])?;
    Ok(())
}

/// A file's text, whatever its encoding (UTF-8, else Latin-1, as old cheat files are).
pub fn read_text(p: &Path) -> String {
    let b = std::fs::read(p).unwrap_or_default();
    let b = b.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(&b);
    match std::str::from_utf8(b) {
        Ok(s) => s.to_string(),
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

// ------------------------------------------------------------ the Cheats page

/// A game as the Library lists it: its key, names, system, picture and folder.
#[derive(Clone, Debug, Default)]
pub struct LibGame {
    pub key: String,
    pub kind: String,
    pub platform: String,
    pub dat_game: Option<i64>,
    pub name: String,
    pub title: String,
    pub folder: Option<String>,
    pub picture: Option<String>,
}

fn lib_game(r: &rusqlite::Row) -> rusqlite::Result<LibGame> {
    Ok(LibGame { key: r.get(0)?, kind: r.get(1)?, platform: r.get(2)?, dat_game: r.get(3)?, name: r.get(4)?, title: r.get(5)?, folder: r.get(6)?, picture: r.get(7)? })
}

const LIB_COLS: &str = "key, kind, platform, dat_game, name, title, folder, picture";

pub fn game_of(conn: &Connection, key: &str) -> Result<LibGame> {
    conn.query_row(&format!("SELECT {LIB_COLS} FROM library WHERE key = ?1"), [key], lib_game)
        .optional()?
        .ok_or_else(|| anyhow!("There's no game {key} in the library (it may have gone since the list was shown)."))
}

/// The games you have (not their updates, DLC or other discs), on `platforms` (all when empty).
fn had_games(conn: &Connection, platforms: &[String]) -> Result<Vec<LibGame>> {
    let mut q = conn.prepare(&format!("SELECT {LIB_COLS} FROM library WHERE have = 1 AND addon = '' ORDER BY platform, sort"))?;
    let all: Vec<LibGame> = q.query_map([], lib_game)?.collect::<rusqlite::Result<_>>()?;
    Ok(all.into_iter().filter(|g| platforms.is_empty() || platforms.contains(&g.platform)).collect())
}

fn shown(g: &LibGame) -> Value {
    json!({ "key": g.key, "kind": g.kind, "platform": g.platform, "platform_name": crate::browse::system_name(&g.platform), "title": g.title, "name": g.name, "picture": g.picture, "folder": g.folder })
}

/// The index's game folders, cheats and Library rows up to date (the Library's rows are
/// made when a page asks for them).
fn fresh(ix: &Index, lib: &Library) -> Result<()> {
    meta::sync(ix, lib)?;
    crate::browse::refresh(ix, lib)?;
    Ok(())
}

/// What the Cheats page lists: each game with cheats (`show` "with", the default, or
/// "edited"), or each game you have with none ("without").
pub fn list(ix: &Index, lib: &Library, show: &str) -> Result<Value> {
    fresh(ix, lib)?;
    let conn = ix.conn();
    let mut by_folder: BTreeMap<String, Vec<(String, String, i64, String, Option<String>, bool)>> = BTreeMap::new();
    {
        let mut q = conn.prepare("SELECT folder, name, format, count, names, source, edited FROM cheats ORDER BY folder, name")?;
        for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, i64>(6)? == 1))))? {
            let (f, x) = r?;
            by_folder.entry(f).or_default().push(x);
        }
    }
    let mut q = conn.prepare(&format!("SELECT {LIB_COLS} FROM library WHERE folder = ?1 ORDER BY addon != '', have DESC LIMIT 1"))?;
    let mut items = vec![];
    if show == "without" {
        for g in had_games(conn, &[])? {
            if g.folder.as_ref().is_some_and(|f| by_folder.contains_key(f)) {
                continue;
            }
            let mut o = shown(&g);
            o["id"] = json!(g.key);
            o["count"] = json!(0);
            o["files"] = json!([]);
            items.push(o);
        }
    } else {
        for (folder, files) in &by_folder {
            let Some(g) = q.query_row([folder], lib_game).optional()? else { continue };
            let edited = files.iter().any(|f| f.5);
            if show == "edited" && !edited {
                continue;
            }
            let mut o = shown(&g);
            o["id"] = json!(g.key);
            o["count"] = json!(files.iter().map(|f| f.2).sum::<i64>());
            o["edited"] = json!(edited);
            let mut formats: Vec<&str> = files.iter().map(|f| format_name(&f.1)).collect();
            formats.dedup();
            o["formats"] = json!(formats);
            let mut sources: Vec<&str> = files.iter().map(|f| source_name(f.4.as_deref())).collect();
            sources.sort();
            sources.dedup();
            o["sources"] = json!(sources);
            o["names"] = json!(files.iter().map(|f| f.3.as_str()).collect::<Vec<_>>().join("\n"));
            o["files"] = json!(files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>());
            items.push(o);
        }
    }
    let pack = pack_info(None);
    Ok(json!({ "items": items, "show": show, "formats": formats_json(), "credit": CREDIT, "pack": pack }))
}

fn formats_json() -> Vec<Value> {
    FORMATS.iter().map(|f| json!({ "id": f.0, "name": f.1, "ext": f.2, "systems": f.3 })).collect()
}

/// Where a file came from, as said.
pub fn source_name(s: Option<&str>) -> &'static str {
    match s {
        Some("libretro") => "libretro",
        Some("user") | None => "You",
        Some(_) => "Imported",
    }
}

/// A game's cheats folder, if it has a folder.
fn cheats_dir(lib: &Library, g: &LibGame) -> Result<Option<PathBuf>> {
    match &g.folder {
        Some(f) => Ok(Some(lib.resolve(f)?.join(FOLDER))),
        None => Ok(None),
    }
}

/// A game's folder, made for a DAT game that has none yet.
fn ensure_dir(ix: &Index, lib: &Library, g: &LibGame) -> Result<PathBuf> {
    if let Some(f) = &g.folder {
        let d = lib.resolve(f)?;
        if d.join("game.json").is_file() {
            return Ok(d);
        }
    }
    let id = g.dat_game.ok_or_else(|| anyhow!("{} has no folder in the library to keep its cheats in.", g.title))?;
    let dg = meta::dat_game(ix.conn(), id)?;
    let d = meta::ensure_folder(ix, lib, &dg)?;
    // the Library knows it by its folder from now on
    if let Some(rel) = lib.relative(&d) {
        ix.conn().execute("UPDATE library SET folder = ?2 WHERE key = ?1", params![g.key, rel])?;
    }
    Ok(d)
}

/// One game's cheat files, for the editor: each one's format, source, text, and a `.cht`'s
/// cheats as a list; and whether libretro has a newer copy of a file edited here.
pub fn game(ix: &Index, lib: &Library, data_dir: &Path, key: &str) -> Result<Value> {
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let mut files = vec![];
    if let Some(dir) = cheats_dir(lib, &g)? {
        let manifest = read_json_object(&dir.join(MANIFEST));
        let mut pack: Option<Option<Pack>> = None;
        for (name, p) in files_in(&dir) {
            let text = read_text(&p);
            let (format, source, edited, fetched) = entry_of(&manifest, &name, &text);
            let mut o = json!({
                "name": name, "format": format, "format_name": format_name(&format), "source": source, "source_name": source_name(source.as_deref()),
                "edited": edited, "fetched": fetched, "names": names_in(&format, &text), "path": lib.relative(&p), "full": p.display().to_string(),
                "versions": versions_of(&dir, &name).len(),
            });
            if format == "retroarch" {
                o["cht"] = json!(parse_cht(&text));
            }
            // a file of libretro's changed here, and libretro's own since
            if source.as_deref() == Some("libretro") && edited {
                let pk = pack.get_or_insert_with(|| Pack::open_cached(data_dir).ok());
                if let Some(pk) = pk {
                    let was = manifest["files"][&name]["sha1"].as_str().unwrap_or("");
                    if let Some(e) = pk.find(&g.platform, name.trim_end_matches(".cht")) {
                        if let Ok(t) = pk.read(e) {
                            o["newer"] = json!(!was.is_empty() && sha1_hex(t.as_bytes()) != was);
                        }
                    }
                }
            }
            o["text"] = json!(text);
            files.push(o);
        }
    }
    let mut v = shown(&g);
    v["files"] = json!(files);
    v["formats"] = json!(formats_for(&g.platform).iter().map(|f| json!({ "id": f, "name": format_name(f) })).collect::<Vec<_>>());
    Ok(v)
}

fn sha1_hex(b: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(b);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name != safe_name(name) || name == MANIFEST || name.starts_with('.') || name.contains(['/', '\\']) {
        bail!("{name} isn't a name a cheat file can have.");
    }
    Ok(())
}

/// The versions kept of a file: (when, its path), newest first.
fn versions_of(dir: &Path, name: &str) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir.join(VERSIONS))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().join(name)))
        .filter(|(_, p)| p.is_file())
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Keep a file's version before it changes, and only the last [`KEEP`].
fn keep_version(dir: &Path, name: &str) -> Result<()> {
    let p = dir.join(name);
    if !p.is_file() {
        return Ok(());
    }
    let when = chrono_stamp();
    let to = dir.join(VERSIONS).join(&when);
    std::fs::create_dir_all(&to)?;
    std::fs::copy(&p, to.join(name))?;
    for (_, old) in versions_of(dir, name).into_iter().skip(KEEP) {
        let _ = std::fs::remove_file(&old);
        if let Some(d) = old.parent() {
            let _ = std::fs::remove_dir(d);
        }
    }
    Ok(())
}

/// Now, as a folder's name that sorts by time ("2026-10-31 120102.123").
fn chrono_stamp() -> String {
    let n = now();
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_millis()).unwrap_or(0);
    format!("{}.{ms:03}", n.replace('T', " ").replace([':', 'Z'], ""))
}

fn write_file(p: &Path, text: &str) -> Result<()> {
    let tmp = p.with_file_name(format!("{}.gamerepo-tmp", p.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

fn set_entry(dir: &Path, name: &str, f: impl FnOnce(&mut Map<String, Value>)) -> Result<()> {
    let mut m = read_json_object(&dir.join(MANIFEST));
    if !m["files"].is_object() {
        m["files"] = json!({});
    }
    let files = m["files"].as_object_mut().expect("an object");
    let e = files.entry(name.to_string()).or_insert_with(|| json!({}));
    if !e.is_object() {
        *e = json!({});
    }
    f(e.as_object_mut().expect("an object"));
    write_json(&dir.join(MANIFEST), &m)
}

/// Save a file of a game's cheats, edited here: `cht` (a `.cht` as a list) or `text`. The
/// version before is kept.
pub fn save(ix: &Index, lib: &Library, data_dir: &Path, key: &str, name: &str, cht: Option<&Cht>, text: Option<&str>) -> Result<Value> {
    lib.writable()?;
    check_name(name)?;
    let g = game_of(ix.conn(), key)?;
    let dir = cheats_dir(lib, &g)?.ok_or_else(|| anyhow!("{} has no cheats yet.", g.title))?;
    let p = dir.join(name);
    if !p.is_file() {
        bail!("{name} isn't one of {}'s cheat files.", g.title);
    }
    let text = match (cht, text) {
        (Some(c), _) => write_cht(c),
        (None, Some(t)) => t.to_string(),
        _ => bail!("Nothing to save."),
    };
    if read_text(&p) != text {
        keep_version(&dir, name)?;
        write_file(&p, &text)?;
        set_entry(&dir, name, |e| {
            e.insert("edited".into(), json!(true));
            e.insert("changed".into(), json!(now()));
        })?;
    }
    sync(ix, lib)?;
    game(ix, lib, data_dir, key)
}

/// A new cheat file for a game, in `format` (its folder made if it has none).
pub fn create(ix: &Index, lib: &Library, data_dir: &Path, key: &str, format: &str, name: Option<&str>) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let f = FORMATS.iter().find(|f| f.0 == format).ok_or_else(|| anyhow!("There's no format {format}."))?;
    let dir = ensure_dir(ix, lib, &g)?.join(FOLDER);
    std::fs::create_dir_all(&dir)?;
    let base = match name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => safe_name(n),
        None => safe_name(&format!("{}.{}", g.name, f.2)),
    };
    let base = if base.contains('.') { base } else { format!("{base}.{}", f.2) };
    let taken: HashSet<String> = files_in(&dir).into_iter().map(|(n, _)| n.to_lowercase()).collect();
    let mut name = base.clone();
    let mut n = 2;
    while taken.contains(&name.to_lowercase()) {
        let (s, x) = base.rsplit_once('.').unwrap_or((&base, ""));
        name = format!("{s} ({n}).{x}");
        n += 1;
    }
    let start = match format {
        "retroarch" => write_cht(&Cht::default()),
        "pcsx2" => format!("gametitle={}\n", g.title),
        _ => String::new(),
    };
    write_file(&dir.join(&name), &start)?;
    set_entry(&dir, &name, |e| {
        e.insert("format".into(), json!(format));
        e.insert("source".into(), json!("user"));
        e.insert("added".into(), json!(now()));
    })?;
    sync(ix, lib)?;
    let mut v = game(ix, lib, data_dir, key)?;
    v["made"] = json!(name);
    Ok(v)
}

/// Take a file away from a game's cheats; it's kept with its versions, so it can come back.
pub fn remove(ix: &Index, lib: &Library, data_dir: &Path, key: &str, name: &str) -> Result<Value> {
    lib.writable()?;
    check_name(name)?;
    let g = game_of(ix.conn(), key)?;
    let dir = cheats_dir(lib, &g)?.ok_or_else(|| anyhow!("{} has no cheats.", g.title))?;
    if !dir.join(name).is_file() {
        bail!("{name} isn't one of {}'s cheat files.", g.title);
    }
    keep_version(&dir, name)?;
    std::fs::remove_file(dir.join(name))?;
    let mut m = read_json_object(&dir.join(MANIFEST));
    if let Some(f) = m["files"].as_object_mut() {
        f.remove(name);
    }
    m["removed"][name] = json!(now());
    write_json(&dir.join(MANIFEST), &m)?;
    sync(ix, lib)?;
    game(ix, lib, data_dir, key)
}

/// A file's versions: when each was kept, its cheats' names, and its text.
pub fn versions(ix: &Index, lib: &Library, key: &str, name: &str) -> Result<Value> {
    check_name(name)?;
    let g = game_of(ix.conn(), key)?;
    let Some(dir) = cheats_dir(lib, &g)? else { return Ok(json!({ "versions": [] })) };
    let format = read_json_object(&dir.join(MANIFEST))["files"][name]["format"].as_str().map(String::from);
    Ok(json!({ "versions": versions_of(&dir, name).into_iter().map(|(when, p)| {
        let text = read_text(&p);
        let f = format.clone().unwrap_or_else(|| guess_format(name, &text).to_string());
        json!({ "when": when, "count": names_in(&f, &text).len(), "text": text })
    }).collect::<Vec<_>>() }))
}

/// Go back to a version of a file (the one there now kept as a version too); a file taken
/// away comes back.
pub fn restore(ix: &Index, lib: &Library, data_dir: &Path, key: &str, name: &str, when: &str) -> Result<Value> {
    lib.writable()?;
    check_name(name)?;
    if when.contains(['/', '\\']) || when.starts_with('.') {
        bail!("There's no version {when}.");
    }
    let g = game_of(ix.conn(), key)?;
    let dir = cheats_dir(lib, &g)?.ok_or_else(|| anyhow!("{} has no cheats.", g.title))?;
    let from = dir.join(VERSIONS).join(when).join(name);
    if !from.is_file() {
        bail!("There's no version {when} of {name}.");
    }
    let text = read_text(&from);
    keep_version(&dir, name)?;
    write_file(&dir.join(name), &text)?;
    set_entry(&dir, name, |e| {
        e.insert("edited".into(), json!(true));
        e.insert("changed".into(), json!(now()));
    })?;
    let mut m = read_json_object(&dir.join(MANIFEST));
    if let Some(r) = m["removed"].as_object_mut() {
        if r.remove(name).is_some() {
            write_json(&dir.join(MANIFEST), &m)?;
        }
    }
    sync(ix, lib)?;
    game(ix, lib, data_dir, key)
}

/// The files taken away from a game's cheats, to bring back.
pub fn removed(ix: &Index, lib: &Library, key: &str) -> Result<Vec<String>> {
    let g = game_of(ix.conn(), key)?;
    let Some(dir) = cheats_dir(lib, &g)? else { return Ok(vec![]) };
    let m = read_json_object(&dir.join(MANIFEST));
    Ok(m["removed"].as_object().map(|r| r.keys().filter(|n| !dir.join(n).is_file()).cloned().collect()).unwrap_or_default())
}

// ------------------------------------------------------------ libretro's pack

/// Where a file of the pack is: a member of the cached zip, or a file fetched from GitHub.
#[derive(Clone, Debug)]
pub enum At {
    Zip(String),
    File(PathBuf),
    Url(String),
}

/// One file of the pack: its system's folder, its name without `.cht`, and where it is.
#[derive(Clone, Debug)]
pub struct PackFile {
    pub platform: String,
    pub stem: String,
    pub at: At,
}

/// libretro's cheat pack, read: its files by system.
pub struct Pack {
    pub zip: Option<PathBuf>,
    pub files: Vec<PackFile>,
    /// (platform, stem lower case) → the file
    by_name: HashMap<(String, String), usize>,
    pub date: Option<String>,
    pub from: String,
}

fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("cheats-cache")
}

/// The system a folder of the pack is for ("Nintendo - Super Nintendo Entertainment System").
fn platform_of_folder(name: &str) -> Option<String> {
    crate::platforms::from_dat_name(name).or_else(|| crate::platforms::from_folder(name)).map(|p| p.id.to_string())
}

/// What's known of the pack kept: its date and where from.
pub fn pack_info(data_dir: Option<&Path>) -> Value {
    let Some(d) = data_dir else { return Value::Null };
    let info = read_json_object(&cache_dir(d).join("pack.json"));
    if info.as_object().is_some_and(|o| o.is_empty()) {
        Value::Null
    } else {
        info
    }
}

impl Pack {
    fn index(&mut self) {
        self.by_name = self.files.iter().enumerate().map(|(i, f)| ((f.platform.clone(), f.stem.to_lowercase()), i)).collect();
    }

    /// The pack kept on this computer, if there is one.
    pub fn open_cached(data_dir: &Path) -> Result<Pack> {
        let dir = cache_dir(data_dir);
        let info = read_json_object(&dir.join("pack.json"));
        let zip = dir.join("cheats.zip");
        if zip.is_file() {
            return Self::from_zip(&zip, info["date"].as_str().map(String::from), info["from"].as_str().unwrap_or(PACK_URL));
        }
        // the files fetched from GitHub
        let files_dir = dir.join("files");
        if files_dir.is_dir() {
            let mut files = vec![];
            for sys in std::fs::read_dir(&files_dir)?.flatten() {
                let Some(platform) = platform_of_folder(&sys.file_name().to_string_lossy()) else { continue };
                for f in std::fs::read_dir(sys.path())?.flatten() {
                    let n = f.file_name().to_string_lossy().into_owned();
                    if let Some(stem) = n.strip_suffix(".cht") {
                        files.push(PackFile { platform: platform.clone(), stem: stem.to_string(), at: At::File(f.path()) });
                    }
                }
            }
            let mut p = Pack { zip: None, files, by_name: HashMap::new(), date: info["date"].as_str().map(String::from), from: "GitHub".into() };
            p.index();
            return Ok(p);
        }
        bail!("libretro's cheats haven't been fetched yet.")
    }

    pub fn from_zip(zip: &Path, date: Option<String>, from: &str) -> Result<Pack> {
        let f = std::fs::File::open(zip)?;
        let z = zip::ZipArchive::new(f).with_context(|| format!("{} isn't a zip that opens", zip.display()))?;
        let mut files = vec![];
        for name in z.file_names().flatten() {
            let n = name.replace('\\', "/");
            let parts: Vec<&str> = n.split('/').filter(|p| !p.is_empty()).collect();
            // `<system>/<game>.cht`, perhaps under a top folder (`cht/`)
            if parts.len() < 2 {
                continue;
            }
            let (sys, file) = (parts[parts.len() - 2], parts[parts.len() - 1]);
            let Some(stem) = file.strip_suffix(".cht") else { continue };
            let Some(platform) = platform_of_folder(sys) else { continue };
            files.push(PackFile { platform, stem: stem.to_string(), at: At::Zip(name.to_string()) });
        }
        let mut p = Pack { zip: Some(zip.to_path_buf()), files, by_name: HashMap::new(), date, from: from.to_string() };
        p.index();
        Ok(p)
    }

    /// The file named after a game (by its DAT name), on its system.
    pub fn find(&self, platform: &str, stem: &str) -> Option<&PackFile> {
        self.by_name.get(&(platform.to_string(), stem.to_lowercase())).map(|&i| &self.files[i])
    }

    /// A file's text.
    pub fn read(&self, f: &PackFile) -> Result<String> {
        match &f.at {
            At::Zip(name) => {
                let zip = self.zip.as_ref().ok_or_else(|| anyhow!("the pack has no zip"))?;
                let mut z = zip::ZipArchive::new(std::fs::File::open(zip)?)?;
                let mut m = z.by_name(name)?;
                let mut b = vec![];
                m.read_to_end(&mut b)?;
                let b = b.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(&b);
                Ok(match std::str::from_utf8(b) {
                    Ok(s) => s.to_string(),
                    Err(_) => b.iter().map(|&c| c as char).collect(),
                })
            }
            At::File(p) => Ok(read_text(p)),
            At::Url(u) => {
                let b = crate::starter::fetch(u)?.ok_or_else(|| anyhow!("{u} isn't there"))?;
                Ok(String::from_utf8_lossy(&b).into_owned())
            }
        }
    }
}

/// Where the pack is fetched from (a test's stand-in server can be given instead).
#[derive(Clone, Debug)]
pub struct Sources {
    pub zip: String,
    pub api: String,
    pub raw: String,
}

impl Default for Sources {
    fn default() -> Self {
        Sources { zip: PACK_URL.into(), api: GITHUB_API.into(), raw: GITHUB_RAW.into() }
    }
}

fn age_of(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok()?.modified().ok()?.elapsed().ok().map(|d| d.as_secs())
}

/// The pack, fetched when there's none kept, it's more than a week old, or `again`. When the
/// buildbot can't be reached, the files of `platforms` named as `wanted` are fetched from
/// GitHub; when nothing can, the one kept is used.
pub fn get_pack(data_dir: &Path, src: &Sources, again: bool, wanted: &HashMap<String, Vec<String>>, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Pack> {
    let dir = cache_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let zip = dir.join("cheats.zip");
    let fresh = age_of(&zip).is_some_and(|a| a < PACK_AGE);
    if fresh && !again {
        if let Ok(p) = Pack::open_cached(data_dir) {
            return Ok(p);
        }
    }
    progress(json!({ "text": "Downloading libretro's cheats" }));
    let failed = match crate::starter::fetch(&src.zip) {
        Ok(Some(bytes)) => {
            let tmp = dir.join("cheats.zip.gamerepo-tmp");
            std::fs::write(&tmp, &bytes)?;
            match Pack::from_zip(&tmp, Some(now()), &src.zip) {
                Ok(_) => {
                    std::fs::rename(&tmp, &zip)?;
                    write_json(&dir.join("pack.json"), &json!({ "date": now(), "from": src.zip }))?;
                    return Pack::open_cached(data_dir);
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    format!("{e:#}")
                }
            }
        }
        Ok(None) => format!("{} isn't there", src.zip),
        Err(e) => format!("{e:#}"),
    };
    // the same files from GitHub: each system's folder listed, then the files wanted fetched
    match from_github(data_dir, src, wanted, cancel, progress) {
        Ok(p) => Ok(p),
        Err(e) => match Pack::open_cached(data_dir) {
            Ok(p) => Ok(p),
            Err(_) => bail!("libretro's cheats couldn't be downloaded ({failed}; from GitHub: {e:#})."),
        },
    }
}

/// The pack's files for the games wanted (system → their DAT names), from GitHub.
fn from_github(data_dir: &Path, src: &Sources, wanted: &HashMap<String, Vec<String>>, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Pack> {
    let get_json = |url: &str| -> Result<Value> {
        let b = crate::starter::fetch(url)?.ok_or_else(|| anyhow!("{url} isn't there"))?;
        Ok(serde_json::from_slice(&b)?)
    };
    let top = get_json(&format!("{}/contents/cht", src.api))?;
    let dirs: Vec<(String, String)> = top
        .as_array()
        .ok_or_else(|| anyhow!("GitHub's answer wasn't a list"))?
        .iter()
        .filter(|e| e["type"] == "dir")
        .filter_map(|e| Some((e["name"].as_str()?.to_string(), e["sha"].as_str()?.to_string())))
        .collect();
    let files_dir = cache_dir(data_dir).join("files");
    let mut files = vec![];
    let mut n = 0;
    for (folder, sha) in dirs {
        let Some(platform) = platform_of_folder(&folder) else { continue };
        let Some(names) = wanted.get(&platform) else { continue };
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let tree = get_json(&format!("{}/git/trees/{sha}", src.api))?;
        let listed: HashMap<String, String> = tree["tree"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|e| e["path"].as_str())
            .filter_map(|p| p.strip_suffix(".cht").map(|s| (s.to_lowercase(), p.to_string())))
            .collect();
        // by name, and by title as matching does: a file for each game's title
        let mut titles: HashMap<Vec<String>, Vec<&String>> = HashMap::new();
        for (stem, _) in &listed {
            titles.entry(words(split_name(stem).0)).or_default().push(stem);
        }
        let mut take: Vec<String> = vec![];
        for want in names {
            if let Some(p) = listed.get(&want.to_lowercase()) {
                take.push(p.clone());
            } else if let Some(s) = titles.get(&words(split_name(&want.to_lowercase()).0)) {
                take.extend(s.iter().filter_map(|s| listed.get(*s)).cloned());
            }
        }
        take.sort();
        take.dedup();
        for p in take {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            n += 1;
            progress(json!({ "text": "Downloading cheats from GitHub", "item": n, "name": p }));
            let url = format!("{}{}/{}", src.raw, crate::starter::encode(&folder), crate::starter::encode(&p));
            if let Some(b) = crate::starter::fetch(&url)? {
                let to = files_dir.join(safe_name(&folder)).join(safe_name(&p));
                std::fs::create_dir_all(to.parent().unwrap_or(&files_dir))?;
                std::fs::write(&to, b)?;
                files.push(PackFile { platform: platform.clone(), stem: p.trim_end_matches(".cht").to_string(), at: At::File(to) });
            }
        }
    }
    write_json(&cache_dir(data_dir).join("pack.json"), &json!({ "date": now(), "from": "GitHub" }))?;
    let mut p = Pack { zip: None, files, by_name: HashMap::new(), date: Some(now()), from: "GitHub".into() };
    p.index();
    // the whole of what's been fetched from GitHub before, too
    if let Ok(all) = Pack::open_cached(data_dir) {
        if all.zip.is_none() && all.files.len() > p.files.len() {
            return Ok(all);
        }
    }
    Ok(p)
}

// ------------------------------------------------------------ matching and bringing in

/// A cheat file found for a game: from the pack (`stem`) or a file picked (`path`), with how
/// sure the pairing is, and whether it's new, a newer copy, or an edited file's newer copy.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Found {
    pub key: String,
    pub title: String,
    pub platform: String,
    /// The pack's file (its name without `.cht`), or a file's path
    pub stem: Option<String>,
    pub path: Option<String>,
    /// The name it's kept under in the game's cheats folder
    pub name: String,
    pub format: String,
    /// sure | likely
    pub pairing: String,
    /// new | newer (replaces libretro's copy, not edited) | edited (kept: the newer is offered)
    pub state: String,
    pub count: usize,
    pub include: bool,
}

/// A title as compared, and the tags after it.
fn title_and_tags(name: &str) -> (Vec<String>, Vec<String>) {
    (words(split_name(name).0), tags_of(name).into_iter().map(|t| t.to_lowercase()).collect())
}

/// Match the pack's files to the games you have on `platforms` (or the `keys` given): each
/// game's DAT name is sure; else a file of the same title (the most tags in common) is likely.
pub fn find(ix: &Index, lib: &Library, pack: &Pack, platforms: &[String], keys: &[String]) -> Result<Value> {
    fresh(ix, lib)?;
    let conn = ix.conn();
    let games: Vec<LibGame> = if keys.is_empty() {
        had_games(conn, platforms)?
    } else {
        keys.iter().filter_map(|k| game_of(conn, k).ok()).collect()
    };
    // the pack's files by system and title
    let mut by_title: HashMap<(String, Vec<String>), Vec<usize>> = HashMap::new();
    for (i, f) in pack.files.iter().enumerate() {
        by_title.entry((f.platform.clone(), words(split_name(&f.stem).0))).or_default().push(i);
    }
    let (mut found, mut same, mut kept) = (vec![], 0, 0);
    let mut none = 0;
    for g in &games {
        if g.kind != "dat" {
            continue;
        }
        let (file, pairing) = match pack.find(&g.platform, &g.name) {
            Some(f) => (f, "sure"),
            None => {
                let (t, tags) = title_and_tags(&g.name);
                let Some(list) = by_title.get(&(g.platform.clone(), t)) else {
                    none += 1;
                    continue;
                };
                let best = list.iter().max_by_key(|&&i| {
                    let (_, ft) = title_and_tags(&pack.files[i].stem);
                    (ft.iter().filter(|x| tags.contains(x)).count(), std::cmp::Reverse(ft.len()))
                });
                (&pack.files[*best.expect("a file")], "likely")
            }
        };
        let name = format!("{}.cht", safe_name(&file.stem));
        // what the game has already of libretro's
        let mut state = "new";
        if let Some(dir) = cheats_dir(lib, g)? {
            let p = dir.join(&name);
            if p.is_file() {
                let m = read_json_object(&dir.join(MANIFEST));
                let e = &m["files"][&name];
                if e["source"].as_str() != Some("libretro") {
                    // a file of the user's of that name: theirs stays
                    kept += 1;
                    continue;
                }
                let text = pack.read(file)?;
                let was = e["sha1"].as_str().unwrap_or("");
                if sha1_hex(text.as_bytes()) == was || read_text(&p) == text {
                    same += 1;
                    continue;
                }
                state = if e["edited"].as_bool().unwrap_or(false) { "edited" } else { "newer" };
            }
        }
        if state == "edited" {
            kept += 1;
            continue;
        }
        let count = pack.read(file).map(|t| parse_cht(&t).cheats.len()).unwrap_or(0);
        found.push(Found {
            key: g.key.clone(), title: g.title.clone(), platform: g.platform.clone(), stem: Some(file.stem.clone()), path: None, name, format: "retroarch".into(),
            pairing: pairing.into(), state: state.into(), count, include: pairing == "sure",
        });
    }
    Ok(json!({
        "found": found, "same": same, "kept": kept, "none": none, "games": games.len(),
        "pack": { "date": pack.date, "from": pack.from, "files": pack.files.len() }, "credit": CREDIT,
    }))
}

/// Read cheat files picked (or the cheat files in folders picked), each paired with a game
/// you have by its name, as the pack's are; or all with `key`'s game.
pub fn read_files(ix: &Index, lib: &Library, paths: &[PathBuf], key: Option<&str>, platform: Option<&str>) -> Result<Vec<Found>> {
    fresh(ix, lib)?;
    let conn = ix.conn();
    let exts = ["cht", "pnach", "ini", "txt", "mch", "md"];
    let mut files = vec![];
    for p in paths {
        if p.is_dir() {
            for e in walk(p) {
                if e.extension().map(|x| x.to_string_lossy().to_lowercase()).is_some_and(|x| exts.contains(&x.as_str())) {
                    files.push(e);
                }
            }
        } else if p.is_file() {
            files.push(p.clone());
        }
    }
    let given = key.map(|k| game_of(conn, k)).transpose()?;
    let platforms: Vec<String> = platform.map(|p| vec![p.to_string()]).unwrap_or_default();
    let games = if given.is_some() { vec![] } else { had_games(conn, &platforms)? };
    let mut by_name: HashMap<String, usize> = HashMap::new();
    let mut by_title: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
    for (i, g) in games.iter().enumerate() {
        by_name.insert(g.name.to_lowercase(), i);
        by_title.entry(words(split_name(&g.name).0)).or_default().push(i);
    }
    let mut out = vec![];
    for f in files {
        let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let stem = name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(name.clone());
        let text = read_text(&f);
        let format = guess_format(&name, &text).to_string();
        let count = names_in(&format, &text).len();
        // the system from the folders' names, when the file's in one of a system
        let sys = f.ancestors().skip(1).filter_map(|a| a.file_name()).find_map(|n| platform_of_folder(&n.to_string_lossy()));
        let (g, pairing) = match &given {
            Some(g) => (Some(g.clone()), "you"),
            None => {
                let fits = |i: &usize| sys.as_ref().is_none_or(|s| &games[*i].platform == s);
                match by_name.get(&stem.to_lowercase()).filter(|i| fits(i)) {
                    Some(&i) => (Some(games[i].clone()), "sure"),
                    None => {
                        let (_, tags) = title_and_tags(&stem);
                        let best = by_title.get(&words(split_name(&stem).0)).and_then(|l| {
                            l.iter().filter(|i| fits(i)).max_by_key(|&&i| title_and_tags(&games[i].name).1.iter().filter(|t| tags.contains(t)).count())
                        });
                        match best {
                            Some(&i) => (Some(games[i].clone()), "likely"),
                            None => (None, "none"),
                        }
                    }
                }
            }
        };
        out.push(Found {
            key: g.as_ref().map(|g| g.key.clone()).unwrap_or_default(), title: g.as_ref().map(|g| g.title.clone()).unwrap_or_default(),
            platform: g.as_ref().map(|g| g.platform.clone()).or(sys).unwrap_or_default(), stem: None, path: Some(f.display().to_string()),
            name: safe_name(&name), format, pairing: pairing.into(), state: "new".into(), count, include: g.is_some() && pairing != "likely",
        });
    }
    Ok(out)
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Plan bringing the files found into their games' cheats folders: one change, with Undo.
/// A game with no folder yet gets one. A file the user has of the same name is never
/// replaced; one of the pack's not edited here is, by its newer copy.
pub fn plan(ix: &Index, lib: &Library, pack: Option<&Pack>, found: &[Found]) -> Result<Plan> {
    lib.writable()?;
    let conn = ix.conn();
    let items: Vec<&Found> = found.iter().filter(|f| f.include && !f.key.is_empty()).collect();
    if items.is_empty() {
        bail!("Choose at least one game's cheats to bring in.");
    }
    struct Target {
        label: String,
        dir: PathBuf,
        manifest: Value,
        taken: HashSet<String>,
        ops: Vec<Op>,
        new_game: Option<Value>,
    }
    let mut targets: BTreeMap<String, Target> = BTreeMap::new();
    for f in &items {
        let g = game_of(conn, &f.key)?;
        let (dir, new_game) = match g.folder.as_ref().and_then(|x| lib.resolve(x).ok()).filter(|d| d.join("game.json").is_file()) {
            Some(d) => (d, None),
            None => {
                let id = g.dat_game.ok_or_else(|| anyhow!("{} has no folder in the library to keep its cheats in.", g.title))?;
                let dg: DatGame = meta::dat_game(conn, id)?;
                match meta::folder_of(conn, &dg)?.and_then(|x| lib.resolve(&x).ok()).filter(|d| d.join("game.json").is_file()) {
                    Some(d) => (d, None),
                    None => {
                        let d = meta::games_root(lib).join(&dg.platform).join(safe_name(&dg.name));
                        if d.join("game.json").is_file() {
                            bail!("Another game already has {}'s folder in the library.", dg.name);
                        }
                        let gj = json!({
                            "id": organise::new_id(), "platform": dg.platform, "name": dg.name, "dat": dg.dat, "title": dg.title, "created": now(),
                            "roms": dg.roms.iter().map(|r| json!({ "name": r.name, "size": r.size, "crc32": r.crc32, "md5": r.md5, "sha1": r.sha1 })).collect::<Vec<_>>(),
                        });
                        (d, Some(gj))
                    }
                }
            }
        };
        let tkey = organise::key(&dir);
        let t = targets.entry(tkey).or_insert_with(|| {
            let cd = dir.join(FOLDER);
            let manifest = read_json_object(&cd.join(MANIFEST));
            let taken = files_in(&cd).into_iter().map(|(n, _)| n.to_lowercase()).collect();
            Target { label: g.name.clone(), dir: cd, manifest, taken, ops: vec![], new_game }
        });
        let text = match (&f.stem, &f.path, pack) {
            (Some(stem), _, Some(pk)) => {
                let file = pk.find(&f.platform, stem).or_else(|| pk.files.iter().find(|x| &x.stem == stem)).ok_or_else(|| anyhow!("libretro's cheats have no {stem}."))?;
                pk.read(file)?
            }
            (_, Some(p), _) => read_text(Path::new(p)),
            _ => bail!("Where {}'s cheats come from isn't known.", f.title),
        };
        let mut name = safe_name(&f.name);
        let there = t.dir.join(&name);
        let from_pack = f.stem.is_some();
        if there.is_file() {
            let e = &t.manifest["files"][&name];
            let replace = from_pack && e["source"].as_str() == Some("libretro") && !e["edited"].as_bool().unwrap_or(false);
            if !replace {
                // a file of that name the user has or edited: this one beside it
                let (s, x) = name.rsplit_once('.').map(|(s, x)| (s.to_string(), x.to_string())).unwrap_or((name.clone(), String::new()));
                let mut n = 2;
                while t.taken.contains(&name.to_lowercase()) {
                    name = format!("{s} ({n}).{x}");
                    n += 1;
                }
            }
        }
        t.taken.insert(name.to_lowercase());
        t.ops.push(text_op(&t.label, &t.dir.join(&name), text.clone(), "a cheat file"));
        let mut e = json!({ "format": f.format, "added": now() });
        if from_pack {
            e["source"] = json!("libretro");
            e["fetched"] = json!(now());
            e["sha1"] = json!(sha1_hex(text.as_bytes()));
        } else {
            e["source"] = json!(f.path.as_deref().map(|p| Path::new(p).file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_else(|| "user".into()));
        }
        t.manifest["files"][&name] = e;
    }
    let mut ops = vec![];
    let n = targets.len();
    for t in targets.into_values() {
        if let Some(gj) = &t.new_game {
            let p = t.dir.parent().unwrap_or(&t.dir).join("game.json");
            ops.push(text_op(&t.label, &p, serde_json::to_string_pretty(&sorted(gj))? + "\n", "game.json"));
        }
        ops.extend(t.ops);
        ops.push(text_op(&t.label, &t.dir.join(MANIFEST), serde_json::to_string_pretty(&sorted(&t.manifest))? + "\n", "cheats.json: the cheat files' formats and sources"));
    }
    let games = |n: usize| format!("{n} game{}", if n == 1 { "" } else { "s" });
    Ok(Plan {
        id: organise::new_id(),
        created: now(),
        kind: "cheats".into(),
        label: format!("Brought in cheats for {}", games(n)),
        title: format!("Bring in cheats for {}", games(n)),
        dat: Value::Null,
        platform: String::new(),
        mode: "copy".into(),
        format: "keep".into(),
        out: Some(meta::games_root(lib).display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games: n,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None,
        new_roots: vec![],
        present: vec![],
        export: None,
        brought: vec![],
    })
}

/// Take libretro's newer copy of a file edited here (the edited one kept as a version).
pub fn take_newer(ix: &Index, lib: &Library, data_dir: &Path, key: &str, name: &str) -> Result<Value> {
    lib.writable()?;
    check_name(name)?;
    let g = game_of(ix.conn(), key)?;
    let dir = cheats_dir(lib, &g)?.ok_or_else(|| anyhow!("{} has no cheats.", g.title))?;
    let pack = Pack::open_cached(data_dir)?;
    let f = pack.find(&g.platform, name.trim_end_matches(".cht")).ok_or_else(|| anyhow!("libretro's cheats have no {name} for {}.", g.title))?;
    let text = pack.read(f)?;
    keep_version(&dir, name)?;
    write_file(&dir.join(name), &text)?;
    let sha = sha1_hex(text.as_bytes());
    set_entry(&dir, name, |e| {
        e.insert("edited".into(), json!(false));
        e.insert("source".into(), json!("libretro"));
        e.insert("fetched".into(), json!(now()));
        e.insert("sha1".into(), json!(sha));
    })?;
    sync(ix, lib)?;
    game(ix, lib, data_dir, key)
}

/// The `.cht` of a game's folder an export or a device takes (D200): one edited here first,
/// else libretro's, else any.
pub fn retroarch_file(game_dir: &Path) -> Option<PathBuf> {
    let dir = game_dir.join(FOLDER);
    let m = read_json_object(&dir.join(MANIFEST));
    let mut best: Option<(u8, PathBuf)> = None;
    for (name, p) in files_in(&dir) {
        let e = &m["files"][&name];
        let format = e["format"].as_str().map(String::from).unwrap_or_else(|| guess_format(&name, &read_text(&p)).to_string());
        if format != "retroarch" {
            continue;
        }
        let rank = if e["edited"].as_bool().unwrap_or(false) { 0 } else if e["source"] == "libretro" { 1 } else { 2 };
        if best.as_ref().is_none_or(|b| rank < b.0) {
            best = Some((rank, p));
        }
    }
    best.map(|b| b.1)
}

/// The RetroArch core that runs a system, as its folder of cheats is named (the RetroArch
/// save profile's `cores`, D192).
pub fn core_of(system: &str) -> Option<String> {
    gamerepo_wire::saves::built_in().into_iter().find(|p| p.id == "retroarch")?.cores.get(system).cloned()
}

/// How many cheats a game has (for its page's tab).
pub fn count_of(conn: &Connection, folder: Option<&str>) -> i64 {
    let Some(f) = folder else { return 0 };
    conn.query_row("SELECT COALESCE(SUM(count), 0) + COUNT(*) * 0 FROM cheats WHERE folder = ?1", [f], |r| r.get(0)).unwrap_or(0)
}

/// The DAT names of the games you have on `platforms`, by system (what GitHub's files are
/// fetched for).
pub fn wanted(ix: &Index, lib: &Library, platforms: &[String], keys: &[String]) -> Result<HashMap<String, Vec<String>>> {
    fresh(ix, lib)?;
    let conn = ix.conn();
    let games: Vec<LibGame> = if keys.is_empty() { had_games(conn, platforms)? } else { keys.iter().filter_map(|k| game_of(conn, k).ok()).collect() };
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for g in games.into_iter().filter(|g| g.kind == "dat") {
        out.entry(g.platform).or_default().push(g.name);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cht_is_read_and_written_again() {
        let text = "cheats = 3\n\ncheat0_desc = \"Infinite Lives\"\ncheat0_code = \"7E0DBE63\"\ncheat0_enable = false\n\n\
                    cheat1_desc = \"Moon \\\"jump\\\"\"\ncheat1_code = \"7E0019:01+7E001A:02\"\ncheat1_enable = true\ncheat1_handler = 1\ncheat1_address = 1234\n\n\
                    cheat2_desc = \"Third\"\ncheat2_code = \"C0DE\"\n";
        let c = parse_cht(text);
        assert_eq!(c.cheats.len(), 3);
        assert_eq!(c.cheats[0], Cheat { desc: "Infinite Lives".into(), code: "7E0DBE63".into(), enable: false, extra: BTreeMap::new() });
        assert_eq!(c.cheats[1].desc, "Moon \"jump\"");
        assert!(c.cheats[1].enable);
        assert_eq!(c.cheats[1].extra.get("handler").map(String::as_str), Some("1"));
        let again = parse_cht(&write_cht(&c));
        assert_eq!(again, c);
        assert_eq!(guess_format("x.cht", text), "retroarch");
        assert_eq!(names_in("retroarch", text), vec!["Infinite Lives", "Moon \"jump\"", "Third"]);
    }

    #[test]
    fn the_other_formats_are_named_and_counted() {
        let pnach = "gametitle=Okami\n[Infinite Health]\npatch=1,EE,0012,extended,0\n[Max Yen]\npatch=1,EE,0034,extended,1\n";
        assert_eq!((guess_format("ABCD1234.pnach", pnach), names_in("pcsx2", pnach)), ("pcsx2", vec!["Infinite Health".to_string(), "Max Yen".into()]));
        let cw = "_S ULUS-10041\n_G Lumines\n_C0 Max Score\n_L 0x20001234 0x0098967F\n_C1 Level 99\n_L 0x2000 0x63\n";
        assert_eq!((guess_format("ULUS10041.ini", cw), names_in("ppsspp", cw).len()), ("ppsspp", 2));
        let dol = "[Core]\nCPUThread = True\n[ActionReplay]\n$Infinite Health\n0429D5D8 00000064\n$Max Coins\n0429D5DC 000003E7\n[ActionReplay_Enabled]\n$Max Coins\n";
        assert_eq!((guess_format("GALE01.ini", dol), names_in("dolphin", dol)), ("dolphin", vec!["Infinite Health".to_string(), "Max Coins".into()]));
        let az = "[Infinite HP]\n0D1A2B3C 00000063\n[*Max Money]\n0D1A2B40 0000270F\n";
        assert_eq!((guess_format("0004000000033500.txt", az), names_in("azahar", az).len()), ("azahar", 2));
        let duck = "[Infinite Health]\nType = Gameshark\nActivation = EndFrame\n80012345 0064\n";
        assert_eq!(guess_format("SLUS-00067.cht", duck), "duckstation");
        assert_eq!(guess_format("notes.txt", "Type IDKFA at the menu."), "notes");
        assert_eq!(formats_for("ps2"), vec!["retroarch", "pcsx2", "notes"]);
        assert_eq!(formats_for("pc"), vec!["notes"]);
    }

    /// A zip of the pack's shape: `cht/<system>/<game>.cht` (or without `cht/`).
    pub(crate) fn pack_zip(files: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(vec![]);
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            for (n, t) in files {
                z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
                z.write_all(t.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    fn cht(names: &[&str]) -> String {
        write_cht(&Cht { cheats: names.iter().map(|n| Cheat { desc: n.to_string(), code: "00000000".into(), ..Default::default() }).collect(), head: BTreeMap::new() })
    }

    #[test]
    fn the_pack_is_fetched_matched_brought_in_edited_and_kept() {
        let (dir, lib, ix) = crate::collection::tests::fixture("cheats");
        crate::browse::refresh(&ix, &lib).unwrap();
        let data = dir.join("data");
        let zip = pack_zip(&[
            ("Nintendo - Game Boy/Alpha (Europe).cht", &cht(&["Infinite Lives", "Max Score"])),
            ("Nintendo - Game Boy/Beta (USA).cht", &cht(&["Level Select"])),
            ("Nintendo - Game Boy/Omega (USA).cht", &cht(&["Nothing"])),
            ("Nintendo - Super Nintendo Entertainment System/Alpha (Europe).cht", &cht(&["Other system"])),
            ("readme.txt", "not a cheat"),
        ]);
        let url = crate::starter::tests::serve(vec![("/cheats.zip".to_string(), zip)]);
        let src = Sources { zip: format!("{url}/cheats.zip"), api: format!("{url}/none"), raw: format!("{url}/none/") };
        let cancel = AtomicBool::new(false);
        let pack = get_pack(&data, &src, false, &HashMap::new(), &cancel, &mut |_| {}).unwrap();
        assert_eq!(pack.files.len(), 4, "{:?}", pack.files);
        assert!(pack.find("gb", "alpha (europe)").is_some());
        let r = find(&ix, &lib, &pack, &["gb".to_string()], &[]).unwrap();
        let found: Vec<Found> = serde_json::from_value(r["found"].clone()).unwrap();
        let by = |k: &str| found.iter().find(|f| f.key == k).cloned();
        let a = by("gb/Alpha (Europe)").expect("Alpha's");
        assert_eq!((a.pairing.as_str(), a.count, a.include), ("sure", 2, true));
        // a Beta of another region's is likely, and waits for a yes
        let b = by("gb/Beta (Europe)").or_else(|| by("gb/Beta (Japan)")).or_else(|| by("gb/Beta (Europe) (Rev 1)")).expect("a Beta");
        assert_eq!((b.pairing.as_str(), b.include), ("likely", false));
        // brought in as a change, and undone
        let plan = plan(&ix, &lib, Some(&pack), &found).unwrap();
        organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        crate::meta::sync(&ix, &lib).unwrap();
        crate::browse::refresh(&ix, &lib).unwrap();
        let l = list(&ix, &lib, "with").unwrap();
        let items = l["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{l}");
        assert_eq!((items[0]["key"].as_str(), items[0]["count"].as_i64()), (Some("gb/Alpha (Europe)"), Some(2)));
        assert!(l["items"][0]["names"].as_str().unwrap().contains("Max Score"));
        let without = list(&ix, &lib, "without").unwrap();
        assert!(without["items"].as_array().unwrap().iter().all(|i| i["key"] != "gb/Alpha (Europe)"));
        // edited: a cheat added, its version before kept
        let g = game(&ix, &lib, &data, "gb/Alpha (Europe)").unwrap();
        let mut c: Cht = serde_json::from_value(g["files"][0]["cht"].clone()).unwrap();
        c.cheats.push(Cheat { desc: "Moon Jump".into(), code: "C0DE0001".into(), enable: true, ..Default::default() });
        let g = save(&ix, &lib, &data, "gb/Alpha (Europe)", "Alpha (Europe).cht", Some(&c), None).unwrap();
        assert_eq!(g["files"][0]["names"].as_array().unwrap().len(), 3);
        assert_eq!(g["files"][0]["edited"], json!(true));
        assert_eq!(g["files"][0]["versions"], json!(1));
        // libretro's pack changes: the edited file is kept, its newer copy offered
        let zip2 = pack_zip(&[("Nintendo - Game Boy/Alpha (Europe).cht", &cht(&["Infinite Lives", "Max Score", "New one"]))]);
        let p2 = dir.join("pack2.zip");
        std::fs::write(&p2, zip2).unwrap();
        std::fs::copy(&p2, data.join("cheats-cache").join("cheats.zip")).unwrap();
        let pack = Pack::open_cached(&data).unwrap();
        let r = find(&ix, &lib, &pack, &["gb".to_string()], &[]).unwrap();
        assert_eq!(r["kept"], json!(1), "{r}");
        assert!(r["found"].as_array().unwrap().iter().all(|f| f["key"] != "gb/Alpha (Europe)"));
        let g = game(&ix, &lib, &data, "gb/Alpha (Europe)").unwrap();
        assert_eq!(g["files"][0]["newer"], json!(true));
        let g = take_newer(&ix, &lib, &data, "gb/Alpha (Europe)", "Alpha (Europe).cht").unwrap();
        assert_eq!((g["files"][0]["names"].as_array().unwrap().len(), &g["files"][0]["edited"]), (3, &json!(false)));
        // and back to the edited one
        let v = versions(&ix, &lib, "gb/Alpha (Europe)", "Alpha (Europe).cht").unwrap();
        let when = v["versions"][0]["when"].as_str().unwrap().to_string();
        assert!(v["versions"][0]["text"].as_str().unwrap().contains("Moon Jump"));
        let g = restore(&ix, &lib, &data, "gb/Alpha (Europe)", "Alpha (Europe).cht", &when).unwrap();
        assert!(g["files"][0]["names"].as_array().unwrap().iter().any(|n| n == "Moon Jump"));
        // a new file, then taken away and brought back
        let g = create(&ix, &lib, &data, "gb/Delta (World)", "notes", None).unwrap();
        let made = g["made"].as_str().unwrap().to_string();
        assert_eq!(made, "Delta (World).txt");
        let g = save(&ix, &lib, &data, "gb/Delta (World)", &made, None, Some("Hold Select at the title.")).unwrap();
        assert_eq!(g["files"][0]["text"], json!("Hold Select at the title."));
        remove(&ix, &lib, &data, "gb/Delta (World)", &made).unwrap();
        assert_eq!(removed(&ix, &lib, "gb/Delta (World)").unwrap(), vec![made.clone()]);
        let v = versions(&ix, &lib, "gb/Delta (World)", &made).unwrap();
        let when = v["versions"][0]["when"].as_str().unwrap().to_string();
        let g = restore(&ix, &lib, &data, "gb/Delta (World)", &made, &when).unwrap();
        assert_eq!(g["files"][0]["text"], json!("Hold Select at the title."));
        assert!(save(&ix, &lib, &data, "gb/Delta (World)", "../x", None, Some("")).is_err());
        // the first change undone: Alpha's cheats folder's files go
        let _ = std::fs::remove_dir_all(dir.join("lib").join("_never"));
    }

    #[test]
    fn files_picked_are_paired_by_name_and_brought_in_with_undo() {
        let (dir, lib, ix) = crate::collection::tests::fixture("cheatfiles");
        crate::browse::refresh(&ix, &lib).unwrap();
        let pick = dir.join("Downloads").join("Nintendo - Game Boy");
        std::fs::create_dir_all(&pick).unwrap();
        std::fs::write(pick.join("Delta (World).cht"), cht(&["A", "B"])).unwrap();
        std::fs::write(pick.join("Epsilon (USA).cht"), cht(&["C"])).unwrap();
        std::fs::write(pick.join("Unknown Game.cht"), cht(&["D"])).unwrap();
        std::fs::write(pick.join("picture.png"), b"no").unwrap();
        let found = read_files(&ix, &lib, &[pick.clone()], None, None).unwrap();
        assert_eq!(found.len(), 3, "{found:?}");
        let by = |n: &str| found.iter().find(|f| f.name == n).unwrap().clone();
        assert_eq!((by("Delta (World).cht").pairing.as_str(), by("Delta (World).cht").key.as_str()), ("sure", "gb/Delta (World)"));
        assert_eq!((by("Epsilon (USA).cht").pairing.as_str(), by("Epsilon (USA).cht").key.as_str()), ("likely", "gb/Epsilon (Japan) (En,Ja)"));
        assert_eq!(by("Unknown Game.cht").pairing, "none");
        let mut found = found;
        for f in found.iter_mut() {
            f.include = !f.key.is_empty();
        }
        let p = plan(&ix, &lib, None, &found).unwrap();
        let cancel = AtomicBool::new(false);
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        crate::meta::sync(&ix, &lib).unwrap();
        crate::browse::refresh(&ix, &lib).unwrap();
        let l = list(&ix, &lib, "with").unwrap();
        assert_eq!(l["items"].as_array().unwrap().len(), 2, "{l}");
        let delta = l["items"].as_array().unwrap().iter().find(|i| i["key"] == "gb/Delta (World)").unwrap();
        assert_eq!(delta["sources"], json!(["Imported"]));
        // undone: they go, and the list is empty again
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(list(&ix, &lib, "with").unwrap()["items"].as_array().unwrap().len(), 0);
    }
}
