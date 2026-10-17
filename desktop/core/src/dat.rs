//! DAT files (docs/dat-formats.md): reading Logiqx XML and ClrMamePro DATs, and
//! importing them. An imported DAT is copied to
//! `_library/dats/<platform>/<name> (<version>).dat` (the library is the source of
//! truth: [`sync_library`] loads the index from these files when it's rebuilt) and
//! loaded into the index, where its ROMs are matched with the scanned files.

use crate::index::Index;
use crate::library::{now, valid_id, Library, APP_DIR};
use crate::platforms;
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dat {
    pub name: String,
    pub description: String,
    pub version: String,
    /// The group: header homepage, else author
    pub author: String,
    pub games: Vec<Game>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Game {
    pub name: String,
    pub description: String,
    /// No-Intro's numeric ids (`id`, `cloneofid`)
    pub id: Option<String>,
    pub cloneofid: Option<String>,
    /// The parent's name (`cloneof`)
    pub cloneof: Option<String>,
    pub roms: Vec<Rom>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rom {
    pub name: String,
    pub size: Option<u64>,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    /// "baddump", "nodump", "verified" or none
    pub status: Option<String>,
}

fn hex_field(v: &str) -> Option<String> {
    let v = v.trim().to_ascii_lowercase();
    (!v.is_empty() && v != "-" && v.chars().all(|c| c.is_ascii_hexdigit())).then_some(v)
}

fn size_field(v: &str) -> Option<u64> {
    let v = v.trim();
    match v.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => v.parse().ok(),
    }
}

impl Rom {
    fn set(&mut self, k: &str, v: &str) {
        match k {
            "name" => self.name = v.to_string(),
            "size" => self.size = size_field(v),
            "crc" | "crc32" => self.crc32 = hex_field(v).map(|c| format!("{c:0>8}")),
            "md5" => self.md5 = hex_field(v),
            "sha1" => self.sha1 = hex_field(v),
            "status" | "flags" => self.status = Some(v.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()),
            _ => {}
        }
    }
}

/// Read a DAT of either format.
pub fn parse(bytes: &[u8]) -> Result<Dat> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace() && *b != 0xEF && *b != 0xBB && *b != 0xBF);
    let mut dat = match start.map(|i| bytes[i]) {
        Some(b'<') => parse_logiqx(bytes)?,
        Some(_) => parse_clrmamepro(&String::from_utf8_lossy(bytes))?,
        None => bail!("the file is empty"),
    };
    if dat.name.is_empty() {
        bail!("this DAT has no name in its header");
    }
    // a game listed again with the same files (libretro's Redump copies list a disc once
    // per serial) is the same game
    let mut seen: HashMap<String, Vec<Rom>> = HashMap::new();
    dat.games.retain(|g| match seen.get(&g.name) {
        Some(roms) if *roms == g.roms => false,
        _ => {
            seen.insert(g.name.clone(), g.roms.clone());
            true
        }
    });
    Ok(dat)
}

// ------------------------------------------------------------ Logiqx XML

fn attrs(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .flatten()
        .map(|a| {
            let k = a.key.as_ref().to_string();
            let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.into_owned()).unwrap_or_else(|_| a.value.to_string());
            (k, v)
        })
        .collect()
}

fn parse_logiqx(bytes: &[u8]) -> Result<Dat> {
    let mut r = quick_xml::Reader::from_reader(bytes);
    let mut buf = vec![];
    let mut dat = Dat::default();
    let mut stack: Vec<String> = vec![];
    let mut text = String::new();
    let mut game: Option<Game> = None;
    let (mut version, mut date, mut homepage) = (String::new(), String::new(), String::new());
    loop {
        let ev = r.read_event_into(&mut buf).map_err(|e| anyhow!("the DAT's XML can't be read at byte {}: {e}", r.buffer_position()))?;
        match ev {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(ev, Event::Empty(_));
                let tag = e.local_name().as_ref().to_string();
                let in_game = game.is_some();
                match tag.as_str() {
                    "game" | "machine" | "software" if !in_game => {
                        let mut g = Game::default();
                        for (k, v) in attrs(e) {
                            match k.as_str() {
                                "name" => g.name = v,
                                "id" => g.id = Some(v),
                                "cloneofid" => g.cloneofid = Some(v).filter(|v| !v.is_empty()),
                                "cloneof" => g.cloneof = Some(v).filter(|v| !v.is_empty()),
                                _ => {}
                            }
                        }
                        if empty {
                            dat.games.push(g);
                        } else {
                            game = Some(g);
                        }
                    }
                    "rom" if in_game => {
                        let mut rom = Rom::default();
                        for (k, v) in attrs(e) {
                            rom.set(&k, &v);
                        }
                        game.as_mut().unwrap().roms.push(rom);
                    }
                    _ => {}
                }
                if !empty {
                    stack.push(tag);
                    text.clear();
                }
            }
            Event::Text(t) => text.push_str(&t.xml10_content()),
            Event::CData(t) => text.push_str(&quick_xml::escape::escape(t.into_inner().as_ref())),
            Event::GeneralRef(r) => {
                text.push('&');
                text.push_str(&r.xml10_content());
                text.push(';');
            }
            Event::End(_) => {
                let tag = stack.pop().unwrap_or_default();
                let parent = stack.last().map(String::as_str).unwrap_or("");
                let value = || quick_xml::escape::unescape(text.trim()).map(|v| v.into_owned()).unwrap_or_else(|_| text.trim().to_string());
                match (parent, tag.as_str()) {
                    ("header", "name") => dat.name = value(),
                    ("header", "description") => dat.description = value(),
                    ("header", "version") => version = value(),
                    ("header", "date") => date = value(),
                    ("header", "homepage") => homepage = value(),
                    ("header", "author") => dat.author = value(),
                    ("game" | "machine" | "software", "description") => {
                        if let Some(g) = game.as_mut() {
                            g.description = value();
                        }
                    }
                    (_, "game" | "machine" | "software") => {
                        if let Some(g) = game.take() {
                            dat.games.push(g);
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    dat.version = if version.is_empty() { date } else { version };
    if !homepage.is_empty() {
        dat.author = homepage;
    }
    Ok(dat)
}

// ------------------------------------------------------------ ClrMamePro

#[derive(Debug, PartialEq)]
enum Tok {
    Open,
    Close,
    Word(String),
}

fn tokens(s: &str) -> Vec<Tok> {
    let mut out = vec![];
    let mut it = s.chars().peekable();
    while let Some(&c) = it.peek() {
        match c {
            '(' => {
                out.push(Tok::Open);
                it.next();
            }
            ')' => {
                out.push(Tok::Close);
                it.next();
            }
            '"' => {
                it.next();
                let mut w = String::new();
                while let Some(c) = it.next() {
                    match c {
                        '"' => break,
                        '\\' if it.peek() == Some(&'"') => w.push(it.next().unwrap()),
                        c => w.push(c),
                    }
                }
                out.push(Tok::Word(w));
            }
            c if c.is_whitespace() => {
                it.next();
            }
            _ => {
                let mut w = String::new();
                while let Some(&c) = it.peek() {
                    if c.is_whitespace() || c == '(' || c == ')' || c == '"' {
                        break;
                    }
                    w.push(c);
                    it.next();
                }
                out.push(Tok::Word(w));
            }
        }
    }
    out
}

/// key value pairs of a ( … ) block, with nested blocks as (key, tokens).
fn parse_clrmamepro(s: &str) -> Result<Dat> {
    let toks = tokens(s);
    let mut dat = Dat::default();
    let mut i = 0;
    let word = |i: usize| match toks.get(i) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    while i < toks.len() {
        let Some(kind) = word(i) else {
            i += 1;
            continue;
        };
        if toks.get(i + 1) != Some(&Tok::Open) {
            i += 1;
            continue;
        }
        i += 2;
        let mut game = Game::default();
        let mut header = Map::new();
        while i < toks.len() && toks[i] != Tok::Close {
            let Some(k) = word(i) else {
                i += 1;
                continue;
            };
            if toks.get(i + 1) == Some(&Tok::Open) {
                // a nested block: rom ( name … size … )
                i += 2;
                let mut rom = Rom::default();
                while i < toks.len() && toks[i] != Tok::Close {
                    if let (Some(rk), Some(rv)) = (word(i), word(i + 1)) {
                        rom.set(rk, rv);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                i += 1;
                if k == "rom" {
                    game.roms.push(rom);
                }
                continue;
            }
            let v = word(i + 1).unwrap_or("").to_string();
            header.insert(k.to_string(), json!(v));
            match k {
                "name" => game.name = v,
                "description" => game.description = v,
                "cloneof" => game.cloneof = Some(v).filter(|v| !v.is_empty()),
                _ => {}
            }
            i += 2;
        }
        i += 1;
        match kind {
            "clrmamepro" => {
                let h = |k: &str| header.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                dat.name = h("name");
                dat.description = h("description");
                dat.version = if h("version").is_empty() { h("date") } else { h("version") };
                dat.author = if h("homepage").is_empty() { h("author") } else { h("homepage") };
            }
            "game" | "resource" | "machine" => dat.games.push(game),
            _ => {}
        }
    }
    Ok(dat)
}

// ------------------------------------------------------------ names

/// A game's name split by the No-Intro naming convention.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameFields {
    pub title: String,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub revision: Option<String>,
    pub version: Option<String>,
    pub flags: Vec<String>,
}

const REGIONS: &[&str] = &[
    "World", "USA", "Europe", "Japan", "Asia", "Australia", "Brazil", "Canada", "China", "France", "Germany",
    "Hong Kong", "Italy", "Korea", "Netherlands", "Spain", "Sweden", "Taiwan", "UK", "United Kingdom", "Russia",
    "Scandinavia", "Greece", "Portugal", "Denmark", "Finland", "Norway", "Poland", "Latin America", "India",
    "Mexico", "Argentina", "New Zealand", "South Africa", "Belgium", "Austria", "Switzerland", "Ireland",
    "Israel", "Turkey", "Croatia", "Czech", "Hungary", "Unknown",
];

const FLAGS: &[(&str, &str)] = &[
    ("beta", "beta"), ("proto", "proto"), ("prototype", "proto"), ("demo", "demo"), ("sample", "sample"),
    ("unl", "unl"), ("pirate", "pirate"), ("aftermarket", "aftermarket"), ("bios", "bios"), ("kiosk", "kiosk"),
    ("virtual console", "virtual console"), ("program", "program"), ("debug", "debug"), ("alt", "alt"),
    ("homebrew", "homebrew"), ("hack", "hack"), ("enhancement chip", "enhancement chip"),
];

fn is_language(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_uppercase() && b[1].is_ascii_lowercase()
        && (b.len() == 2 || (b[2] == b'-' && s[3..].chars().all(|c| c.is_ascii_alphabetic()) && b.len() > 3))
}

pub fn parse_name(name: &str) -> NameFields {
    let mut f = NameFields::default();
    // a leading tag: "[BIOS] Game Boy Boot ROM (World)"
    let mut name = name.trim_start();
    while let Some(after) = name.strip_prefix('[') {
        let Some(end) = after.find(']') else { break };
        let tag = after[..end].trim().to_ascii_lowercase();
        if let Some((_, flag)) = FLAGS.iter().find(|(k, _)| tag == *k) {
            f.flags.push(flag.to_string());
        }
        name = after[end + 1..].trim_start();
    }
    let cut = name.find(['(', '[']).unwrap_or(name.len());
    f.title = name[..cut].trim().to_string();
    let mut rest = &name[cut..];
    while let Some(open) = rest.find(['(', '[']) {
        let close_ch = if rest.as_bytes()[open] == b'(' { ')' } else { ']' };
        let Some(len) = rest[open + 1..].find(close_ch) else { break };
        let inner = rest[open + 1..open + 1 + len].trim();
        rest = &rest[open + 1 + len + 1..];
        let parts: Vec<&str> = inner.split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
        let lower = inner.to_ascii_lowercase();
        if f.regions.is_empty() && !parts.is_empty() && parts.iter().all(|p| REGIONS.iter().any(|r| r.eq_ignore_ascii_case(p))) {
            f.regions = parts.iter().map(|p| p.to_string()).collect();
        } else if f.languages.is_empty() && !parts.is_empty() && parts.iter().all(|p| is_language(p)) && parts.iter().any(|p| p.len() >= 2) && f.title != inner {
            f.languages = parts.iter().map(|p| p.to_string()).collect();
        } else if let Some(r) = lower.strip_prefix("rev ") {
            f.revision = Some(r.trim().to_uppercase());
        } else if lower.starts_with('v') && lower[1..].starts_with(|c: char| c.is_ascii_digit()) {
            f.version = Some(inner[1..].to_string());
        } else if let Some((_, flag)) = FLAGS.iter().find(|(k, _)| lower == *k || lower.starts_with(&format!("{k} "))) {
            if !f.flags.iter().any(|x| x == flag) {
                f.flags.push(flag.to_string());
            }
        }
    }
    f
}

// ------------------------------------------------------------ importing

/// File names the DATs are stored under: no characters Windows refuses.
fn safe_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    s.trim().trim_end_matches('.').chars().take(150).collect()
}

/// Remembered answers to "which platform is this DAT for?" (in library.json, by the DAT's base name).
fn remembered_platform(lib: &Library, name: &str) -> Option<String> {
    lib.meta()["dat_platforms"][platforms::dat_base_name(name)].as_str().map(String::from)
}

/// What importing one DAT came to.
pub enum Imported {
    Loaded(Value),
    /// The platform couldn't be worked out: the page asks, and imports again with the answer.
    NeedsPlatform { name: String, file: String },
}

/// Import the DAT (or the DATs in a ZIP) at `path`. `answers` maps a DAT's name to its
/// platform, for those whose platform couldn't be worked out.
pub fn import(ix: &Index, lib: &Library, path: &Path, answers: &Map<String, Value>) -> Result<Vec<Imported>> {
    lib.writable()?;
    let bytes = std::fs::read(path).with_context(|| format!("couldn't read {}", path.display()))?;
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if bytes.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(|e| anyhow!("{file} isn't a ZIP that can be read: {e}"))?;
        let mut out = vec![];
        for i in 0..zip.len() {
            let mut m = zip.by_index(i)?;
            let name = m.name()?.into_owned();
            let ext = platforms::ext_of(&name);
            if m.is_dir() || !(ext == "dat" || ext == "xml") {
                continue;
            }
            let mut inner = vec![];
            std::io::Read::read_to_end(&mut m, &mut inner)?;
            drop(m);
            out.push(import_bytes(ix, lib, &inner, &format!("{file} › {name}"), answers)?);
        }
        if out.is_empty() {
            bail!("{file} has no .dat or .xml file inside.");
        }
        return Ok(out);
    }
    Ok(vec![import_bytes(ix, lib, &bytes, &file, answers)?])
}

pub fn import_bytes(ix: &Index, lib: &Library, bytes: &[u8], file: &str, answers: &Map<String, Value>) -> Result<Imported> {
    let dat = parse(bytes).with_context(|| format!("{file} isn't a DAT that can be read"))?;
    // "*": every DAT in this import is for that platform (the BIOS page's Import a BIOS DAT…)
    let answered = answers.get(&dat.name).or_else(|| answers.get("*")).and_then(Value::as_str).filter(|p| !p.is_empty());
    let platform = match answered {
        Some(p) => {
            valid_id(p)?;
            let mut known = lib.meta()["dat_platforms"].as_object().cloned().unwrap_or_default();
            known.insert(platforms::dat_base_name(&dat.name), json!(p));
            lib.update_meta(json!({ "dat_platforms": known }))?;
            p.to_string()
        }
        None => match remembered_platform(lib, &dat.name).or_else(|| platforms::from_dat_name(&dat.name).map(|p| p.id.to_string())) {
            Some(p) => p,
            None => return Ok(Imported::NeedsPlatform { name: dat.name, file: file.to_string() }),
        },
    };
    // a BIOS DAT's games are the systems (libretro's System.dat names them in comments)
    let dat = if platform == crate::bios::ID { crate::bios::by_system(&String::from_utf8_lossy(bytes), dat) } else { dat };
    let version = if dat.version.is_empty() { "undated".to_string() } else { dat.version.clone() };
    let dir = lib.root().join(APP_DIR).join("dats").join(&platform);
    let base = safe_name(&format!("{} ({})", dat.name, version));
    let mut n = 1;
    let rel = loop {
        let fname = if n == 1 { format!("{base}.dat") } else { format!("{base} {n}.dat") };
        let p = dir.join(&fname);
        if !p.exists() || std::fs::read(&p).ok().as_deref() == Some(bytes) {
            break format!("{APP_DIR}/dats/{platform}/{fname}");
        }
        n += 1;
    };
    let dest = lib.resolve(&rel)?;
    let existing: Option<i64> = ix.conn().query_row("SELECT id FROM dats WHERE source_path = ?1", [&rel], |r| r.get(0)).ok();
    let already = dest.exists() && existing.is_some();
    if !dest.exists() {
        crate::config::write_atomic(&dest, bytes)?;
    }
    let id = match existing {
        Some(id) if already => id,
        _ => load(ix.conn(), &rel, &platform, &dat)?,
    };
    update_current(ix.conn(), lib)?;
    let matched = crate::audit::match_dat(ix.conn(), id)?;
    Ok(Imported::Loaded(json!({
        "id": id, "platform": platform, "platform_name": platforms::name_of(&platform), "name": dat.name, "version": version,
        "games": dat.games.len(), "roms": dat.games.iter().map(|g| g.roms.len()).sum::<usize>(), "matched": matched,
        "already": already, "file": file, "path": rel,
    })))
}

/// Load a parsed DAT into the index (one transaction).
fn load(conn: &Connection, rel: &str, platform: &str, dat: &Dat) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM dats WHERE source_path = ?1", [rel])?;
    let roms: usize = dat.games.iter().map(|g| g.roms.len()).sum();
    tx.execute(
        "INSERT INTO dats (platform, name, version, description, author, source_path, imported, games, roms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![platform, dat.name, if dat.version.is_empty() { "undated" } else { &dat.version }, dat.description, dat.author, rel, now(), dat.games.len() as i64, roms as i64],
    )?;
    let dat_id = tx.last_insert_rowid();
    let mut by_name: HashMap<&str, i64> = HashMap::new();
    let mut by_nid: HashMap<&str, i64> = HashMap::new();
    let mut ids = Vec::with_capacity(dat.games.len());
    {
        let mut g_ins = tx.prepare(
            "INSERT INTO dat_games (dat_id, name, description, title, regions, languages, revision, version, flags) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        let mut r_ins = tx.prepare("INSERT INTO dat_roms (game_id, dat_id, name, size, crc32, md5, sha1, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
        let join = |v: &[String]| (!v.is_empty()).then(|| v.join(", "));
        for g in &dat.games {
            let f = parse_name(&g.name);
            g_ins.execute(params![dat_id, g.name, g.description, f.title, join(&f.regions), join(&f.languages), f.revision, f.version, join(&f.flags)])?;
            let gid = tx.last_insert_rowid();
            ids.push(gid);
            by_name.insert(&g.name, gid);
            if let Some(n) = &g.id {
                by_nid.insert(n, gid);
            }
            for r in &g.roms {
                r_ins.execute(params![gid, dat_id, r.name, r.size.map(|s| s as i64), r.crc32, r.md5, r.sha1, r.status])?;
            }
        }
        let mut p_upd = tx.prepare("UPDATE dat_games SET parent_id = ?2 WHERE id = ?1")?;
        for (g, gid) in dat.games.iter().zip(&ids) {
            let parent = g.cloneofid.as_deref().and_then(|n| by_nid.get(n)).or_else(|| g.cloneof.as_deref().and_then(|n| by_name.get(n)));
            if let Some(p) = parent.filter(|p| *p != gid) {
                p_upd.execute(params![gid, p])?;
            }
        }
    }
    tx.commit()?;
    Ok(dat_id)
}

/// Make each DAT's newest version current, unless another was chosen (library.json `dat_current`).
pub fn update_current(conn: &Connection, lib: &Library) -> Result<()> {
    let chosen = lib.meta()["dat_current"].as_object().cloned().unwrap_or_default();
    let mut q = conn.prepare("SELECT id, platform, name, version, imported, source_path FROM dats ORDER BY platform, name, version, imported")?;
    let rows: Vec<(i64, String, String, String)> = q
        .query_map([], |r| Ok((r.get(0)?, format!("{}/{}", r.get::<_, String>(1)?, r.get::<_, String>(2)?), r.get(5)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // rows are in version order, so the newest is last; a chosen version wins while it's there
    let mut current: HashMap<&str, (i64, bool)> = HashMap::new();
    for (id, lineage, src, _) in &rows {
        let pinned = chosen.get(lineage).and_then(Value::as_str) == Some(src.as_str());
        let keep = current.get(lineage.as_str()).is_some_and(|(_, p)| *p);
        if !keep {
            current.insert(lineage, (*id, pinned));
        }
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE dats SET current = 0", [])?;
    for (id, _) in current.values() {
        tx.execute("UPDATE dats SET current = 1 WHERE id = ?1", [id])?;
    }
    tx.commit()?;
    Ok(())
}

/// Make DAT `id` its platform's current version of that DAT.
pub fn use_version(ix: &Index, lib: &Library, id: i64) -> Result<()> {
    let (platform, name, src): (String, String, String) = ix
        .conn()
        .query_row("SELECT platform, name, source_path FROM dats WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(|_| anyhow!("There's no DAT {id}."))?;
    let mut chosen = lib.meta()["dat_current"].as_object().cloned().unwrap_or_default();
    chosen.insert(format!("{platform}/{name}"), json!(src));
    lib.update_meta(json!({ "dat_current": chosen }))?;
    update_current(ix.conn(), lib)?;
    crate::audit::changed(ix.conn())
}

/// Remove an imported DAT: its copy in the library and everything matched to it.
pub fn remove(ix: &Index, lib: &Library, id: i64) -> Result<()> {
    lib.writable()?;
    let src: String = ix
        .conn()
        .query_row("SELECT source_path FROM dats WHERE id = ?1", [id], |r| r.get(0))
        .map_err(|_| anyhow!("There's no DAT {id}."))?;
    let path = lib.resolve(&src)?;
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("couldn't remove {}", path.display()))?;
    }
    ix.conn().execute("DELETE FROM dats WHERE id = ?1", [id])?;
    update_current(ix.conn(), lib)?;
    crate::audit::changed(ix.conn())
}

/// Bring the index's DATs in line with `_library/dats/` (the library is the source of
/// truth): load files the index doesn't have, drop those that are gone.
pub fn sync_library(ix: &Index, lib: &Library) -> Result<()> {
    let base = lib.root().join(APP_DIR).join("dats");
    let mut on_disk: Vec<(String, String)> = vec![];
    for pdir in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        if !pdir.path().is_dir() {
            continue;
        }
        let platform = pdir.file_name().to_string_lossy().into_owned();
        if valid_id(&platform).is_err() {
            continue;
        }
        for f in std::fs::read_dir(pdir.path()).into_iter().flatten().flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let ext = platforms::ext_of(&name);
            if f.path().is_file() && (ext == "dat" || ext == "xml") {
                on_disk.push((format!("{APP_DIR}/dats/{platform}/{name}"), platform.clone()));
            }
        }
    }
    let conn = ix.conn();
    let known: Vec<String> = conn.prepare("SELECT source_path FROM dats")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut changed = false;
    for k in &known {
        if !on_disk.iter().any(|(rel, _)| rel == k) {
            conn.execute("DELETE FROM dats WHERE source_path = ?1", [k])?;
            changed = true;
        }
    }
    for (rel, platform) in &on_disk {
        if known.contains(rel) {
            continue;
        }
        let Ok(bytes) = std::fs::read(lib.resolve(rel)?) else { continue };
        let Ok(dat) = parse(&bytes) else { continue };
        let dat = if platform == crate::bios::ID { crate::bios::by_system(&String::from_utf8_lossy(&bytes), dat) } else { dat };
        let id = load(conn, rel, platform, &dat)?;
        crate::audit::match_dat(conn, id)?;
        changed = true;
    }
    if changed {
        update_current(conn, lib)?;
        crate::audit::changed(conn)?;
    }
    Ok(())
}

/// The imported DATs, grouped as the page lists them.
pub fn list(ix: &Index) -> Result<Value> {
    let mut q = ix.conn().prepare(
        "SELECT id, platform, name, version, description, author, source_path, imported, current, games, roms FROM dats ORDER BY platform, name, version DESC",
    )?;
    let rows: Vec<Value> = q
        .query_map([], |r| {
            let platform: String = r.get(1)?;
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "platform_name": platforms::name_of(&platform), "platform": platform,
                "name": r.get::<_, String>(2)?, "version": r.get::<_, String>(3)?, "description": r.get::<_, Option<String>>(4)?,
                "author": r.get::<_, Option<String>>(5)?, "path": r.get::<_, String>(6)?, "imported": r.get::<_, String>(7)?,
                "current": r.get::<_, i64>(8)? == 1, "games": r.get::<_, i64>(9)?, "roms": r.get::<_, i64>(10)?,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(json!(rows))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub const LOGIQX: &str = r#"<?xml version="1.0"?>
<!DOCTYPE datafile PUBLIC "-//Logiqx//DTD ROM Management Datafile//EN" "http://www.logiqx.com/dtds/datafile.dtd">
<datafile>
	<header>
		<name>Nintendo - Game Boy (Parent-Clone)</name>
		<description>Nintendo - Game Boy</description>
		<version>20261001-000000</version>
		<homepage>No-Intro</homepage>
	</header>
	<game name="Mario &amp; Yoshi (Europe)" id="0001">
		<description>Mario &amp; Yoshi (Europe)</description>
		<rom name="Mario &amp; Yoshi (Europe).gb" size="32768" crc="AB12CD34" md5="00112233445566778899aabbccddeeff" sha1="00112233445566778899AABBCCDDEEFF00112233" status="verified"/>
	</game>
	<game name="Yoshi (USA)" id="0002" cloneofid="0001">
		<description>Yoshi (USA)</description>
		<rom name="Yoshi (USA).gb" size="32768" crc="1234"/>
	</game>
	<game name="[BIOS] Boot ROM (World)" id="0003"><description>x</description><rom name="boot.bin" size="256" status="nodump"/></game>
</datafile>
"#;

    #[test]
    fn reads_logiqx() {
        let d = parse(LOGIQX.as_bytes()).unwrap();
        assert_eq!(d.name, "Nintendo - Game Boy (Parent-Clone)");
        assert_eq!(d.version, "20261001-000000");
        assert_eq!(d.author, "No-Intro");
        assert_eq!(d.games.len(), 3);
        assert_eq!(d.games[0].name, "Mario & Yoshi (Europe)");
        assert_eq!(d.games[0].description, "Mario & Yoshi (Europe)");
        let r = &d.games[0].roms[0];
        assert_eq!((r.name.as_str(), r.size, r.crc32.as_deref()), ("Mario & Yoshi (Europe).gb", Some(32768), Some("ab12cd34")));
        assert_eq!(r.sha1.as_deref(), Some("00112233445566778899aabbccddeeff00112233"));
        assert_eq!(d.games[1].cloneofid.as_deref(), Some("0001"));
        assert_eq!(d.games[1].roms[0].crc32.as_deref(), Some("00001234"));
        assert_eq!(d.games[2].roms[0].status.as_deref(), Some("nodump"));
    }

    #[test]
    fn reads_clrmamepro() {
        let s = r#"clrmamepro (
	name "Nintendo - Game Boy"
	description "Nintendo - Game Boy"
	version 20261001
)

game (
	name "Tetris (World) (Rev 1)"
	description "Tetris (World) (Rev 1)"
	cloneof "Tetris (World)"
	rom ( name "Tetris (World) (Rev 1).gb" size 32768 crc 46DF91AD md5 982ed5d2b12a0377eb14bcdc4123744e sha1 74591cc9501af93873f9a5d3eb12da12c0723bbc )
)

game (
	name "Tetris (World)"
	rom ( name "Tetris (World).gb" size 32768 crc 63f9407d )
)

game (
	name "Tetris (World)"
	serial "DMG-TRA"
	rom ( name "Tetris (World).gb" size 32768 crc 63f9407d serial "DMG-TRA" )
)
"#;
        let d = parse(s.as_bytes()).unwrap();
        assert_eq!((d.name.as_str(), d.version.as_str()), ("Nintendo - Game Boy", "20261001"));
        assert_eq!(d.games.len(), 2);
        assert_eq!(d.games[0].cloneof.as_deref(), Some("Tetris (World)"));
        assert_eq!(d.games[0].roms[0].crc32.as_deref(), Some("46df91ad"));
        assert_eq!(d.games[0].roms[0].size, Some(32768));
        assert_eq!(d.games[1].roms[0].name, "Tetris (World).gb");
    }

    #[test]
    fn names_are_split_into_fields() {
        let f = parse_name("Legend of Zelda, The - A Link to the Past (USA, Europe) (En,Fr,De) (Rev 1) (Beta 2)");
        assert_eq!(f.title, "Legend of Zelda, The - A Link to the Past");
        assert_eq!(f.regions, ["USA", "Europe"]);
        assert_eq!(f.languages, ["En", "Fr", "De"]);
        assert_eq!(f.revision.as_deref(), Some("1"));
        assert_eq!(f.flags, ["beta"]);
        let g = parse_name("[BIOS] Game Boy Boot ROM (Japan) (v1.1) (Unl)");
        assert_eq!(g.title, "Game Boy Boot ROM");
        assert_eq!(g.regions, ["Japan"]);
        assert_eq!(g.version.as_deref(), Some("1.1"));
        assert_eq!(g.flags, ["bios", "unl"]);
        assert_eq!(parse_name("Tetris").title, "Tetris");
        assert_eq!(parse_name("Pokemon (Zh-Hans) (Proto)").languages, ["Zh-Hans"]);
    }
}
