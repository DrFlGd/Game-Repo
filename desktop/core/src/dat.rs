//! DAT files (docs/dat-formats.md): reading Logiqx XML and ClrMamePro DATs, and
//! importing them. An imported DAT is copied to
//! `_library/dats/<platform>/<name> (<version>).dat` (the library is the source of
//! truth: [`sync_library`] loads the index from these files when it's rebuilt) and
//! loaded into the index, where its ROMs are matched with the scanned files. A DAT is read
//! as it's parsed (D149), from the file or a ZIP's member as a stream, never whole into
//! memory: a 300 MB MAME listxml takes what its games take, with progress and Stop.

use crate::index::Index;
use crate::library::{now, valid_id, Library, APP_DIR};
use crate::platforms;
use anyhow::{anyhow, bail, Context, Result};
use quick_xml::events::{BytesStart, Event};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Arcade DATs (Phase 7, D58): the set its ROMs lean on (its parent or BIOS set), its
    /// samples' set and samples, the devices it uses, "bios", "device" or "mechanical",
    /// and the DAT's year and manufacturer
    pub romof: Option<String>,
    pub sampleof: Option<String>,
    pub samples: Vec<String>,
    pub devices: Vec<String>,
    pub kind: Option<String>,
    pub year: Option<String>,
    pub manufacturer: Option<String>,
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
    /// The parent's or BIOS set's ROM this one is (arcade DATs' `merge`)
    pub merge: Option<String>,
    /// A disk (`<disk>`): named `<disk>.chd`, matched by the CHD's own SHA1
    pub disk: bool,
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
            "merge" => self.merge = Some(v.to_string()).filter(|v| !v.is_empty()),
            _ => {}
        }
    }
}

impl Rom {
    /// A disk is the game's `<name>.chd`.
    fn disk_named(mut self) -> Self {
        if self.disk {
            self.name = format!("{}.chd", self.name);
            self.merge = self.merge.map(|m| format!("{m}.chd"));
            self.size = None;
            self.crc32 = None;
        }
        self
    }
}

/// Read a DAT of either format.
pub fn parse(bytes: &[u8]) -> Result<Dat> {
    parse_from(bytes)
}

/// Read a DAT of either format as it comes: Logiqx XML a piece at a time, a ClrMamePro DAT
/// (always small) whole.
pub fn parse_from<R: BufRead>(mut r: R) -> Result<Dat> {
    // past any byte order mark and white space, to the first thing in it
    let first = loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            bail!("the file is empty");
        }
        match buf.iter().position(|b| !b.is_ascii_whitespace() && *b != 0xEF && *b != 0xBB && *b != 0xBF) {
            Some(i) => {
                let b = buf[i];
                r.consume(i);
                break b;
            }
            None => {
                let n = buf.len();
                r.consume(n);
            }
        }
    };
    let mut dat = if first == b'<' {
        parse_logiqx(r)?
    } else {
        let mut bytes = vec![];
        r.read_to_end(&mut bytes)?;
        parse_clrmamepro(&String::from_utf8_lossy(&bytes))?
    };
    if dat.name.is_empty() {
        bail!("this DAT has no name in its header");
    }
    // a game listed again with the same files (libretro's Redump copies list a disc once
    // per serial) is the same game
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut kept: Vec<Game> = Vec::with_capacity(dat.games.len());
    for g in std::mem::take(&mut dat.games) {
        if seen.get(&g.name).is_some_and(|&i| kept[i].roms == g.roms) {
            continue;
        }
        seen.insert(g.name.clone(), kept.len());
        kept.push(g);
    }
    dat.games = kept;
    Ok(dat)
}

/// A file being read for a DAT: how far it's got (for progress), and Stop.
struct Reading<'a, R> {
    inner: R,
    read: u64,
    cancel: &'a AtomicBool,
    progress: &'a mut dyn FnMut(u64),
    told: u64,
}

impl<R: Read> Read for Reading<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("stopped"));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        // a few times a megabyte at most
        if self.read - self.told >= 1 << 20 || n == 0 {
            self.told = self.read;
            (self.progress)(self.read);
        }
        Ok(n)
    }
}

/// Parse what `r` gives (a file, a ZIP's member), with progress in bytes read and Stop.
fn parse_reading<R: Read>(r: R, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<Dat> {
    let reading = Reading { inner: r, read: 0, cancel, progress, told: 0 };
    let dat = parse_from(BufReader::with_capacity(1 << 20, reading));
    if cancel.load(Ordering::Relaxed) {
        bail!("stopped");
    }
    dat
}

/// The DAT in a file, read as it's parsed.
pub fn parse_file(path: &Path) -> Result<Dat> {
    let f = std::fs::File::open(path).with_context(|| format!("couldn't read {}", path.display()))?;
    parse_reading(f, &AtomicBool::new(false), &mut |_| {})
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

fn parse_logiqx<R: BufRead>(input: R) -> Result<Dat> {
    let mut r = quick_xml::Reader::from_reader(input);
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
                                "romof" => g.romof = Some(v).filter(|v| !v.is_empty()),
                                "sampleof" => g.sampleof = Some(v).filter(|v| !v.is_empty()),
                                "isbios" if v == "yes" => g.kind = Some("bios".into()),
                                "isdevice" if v == "yes" => g.kind = Some("device".into()),
                                "ismechanical" if v == "yes" && g.kind.is_none() => g.kind = Some("mechanical".into()),
                                _ => {}
                            }
                        }
                        if empty {
                            dat.games.push(g);
                        } else {
                            game = Some(g);
                        }
                    }
                    "rom" | "disk" if in_game => {
                        let mut rom = Rom { disk: tag == "disk", ..Default::default() };
                        for (k, v) in attrs(e) {
                            rom.set(&k, &v);
                        }
                        game.as_mut().unwrap().roms.push(rom.disk_named());
                    }
                    "sample" if in_game => game.as_mut().unwrap().samples.extend(attrs(e).into_iter().find(|(k, _)| k == "name").map(|(_, v)| v)),
                    "device_ref" if in_game => {
                        let g = game.as_mut().unwrap();
                        if let Some((_, v)) = attrs(e).into_iter().find(|(k, _)| k == "name") {
                            if !g.devices.contains(&v) {
                                g.devices.push(v);
                            }
                        }
                    }
                    // MAME's -listxml has no header: it's "MAME" at its build
                    "mame" if stack.is_empty() => {
                        dat.name = "MAME".into();
                        if let Some((_, b)) = attrs(e).into_iter().find(|(k, _)| k == "build") {
                            version = b.split_whitespace().next().unwrap_or(&b).to_string();
                            dat.description = format!("MAME {version}");
                        }
                        dat.author = "MAME".into();
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
                    ("game" | "machine", "year") => {
                        if let Some(g) = game.as_mut() {
                            g.year = Some(value()).filter(|v| !v.is_empty());
                        }
                    }
                    ("game" | "machine", "manufacturer") => {
                        if let Some(g) = game.as_mut() {
                            g.manufacturer = Some(value()).filter(|v| !v.is_empty());
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
                match k {
                    "rom" => game.roms.push(rom),
                    "disk" => game.roms.push(Rom { disk: true, ..rom }.disk_named()),
                    "device_ref" if !rom.name.is_empty() => game.devices.push(rom.name),
                    _ => {}
                }
                continue;
            }
            let v = word(i + 1).unwrap_or("").to_string();
            header.insert(k.to_string(), json!(v));
            match k {
                "name" => game.name = v,
                "description" => game.description = v,
                "cloneof" => game.cloneof = Some(v).filter(|v| !v.is_empty()),
                "romof" => game.romof = Some(v).filter(|v| !v.is_empty()),
                "sampleof" => game.sampleof = Some(v).filter(|v| !v.is_empty()),
                "sample" if !v.is_empty() => game.samples.push(v),
                "year" => game.year = Some(v).filter(|v| !v.is_empty()),
                "manufacturer" => game.manufacturer = Some(v).filter(|v| !v.is_empty()),
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
            // a resource is a BIOS set
            "resource" => dat.games.push(Game { kind: Some("bios".into()), ..game }),
            "game" | "machine" => dat.games.push(game),
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
pub(crate) fn safe_name(s: &str) -> String {
    let s: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    s.trim().trim_end_matches('.').chars().take(150).collect()
}

/// Remembered answers to "which platform is this DAT for?" (in library.json, by the DAT's base name).
fn remembered_platform(lib: &Library, name: &str) -> Option<String> {
    lib.meta()["dat_platforms"][platforms::dat_base_name(name)].as_str().map(String::from)
}

/// What importing one DAT came to.
#[derive(Debug)]
pub enum Imported {
    Loaded(Value),
    /// The platform couldn't be worked out: the page asks, and imports again with the answer.
    NeedsPlatform { name: String, file: String },
}

fn is_zip(path: &Path) -> Result<bool> {
    let mut head = [0u8; 4];
    let mut f = std::fs::File::open(path).with_context(|| format!("couldn't read {}", path.display()))?;
    let n = f.read(&mut head)?;
    Ok(n == 4 && &head == b"PK\x03\x04")
}

/// A ZIP's members that are DATs: (index, name, size unpacked).
fn zip_dats(zip: &mut zip::ZipArchive<std::fs::File>) -> Result<Vec<(usize, String, u64)>> {
    let mut v = vec![];
    for i in 0..zip.len() {
        let m = zip.by_index(i)?;
        let name = m.name()?.into_owned();
        let ext = platforms::ext_of(&name);
        if !m.is_dir() && (ext == "dat" || ext == "xml") {
            v.push((i, name, m.size()));
        }
    }
    Ok(v)
}

fn open_zip(path: &Path, file: &str) -> Result<zip::ZipArchive<std::fs::File>> {
    let f = std::fs::File::open(path).with_context(|| format!("couldn't read {}", path.display()))?;
    zip::ZipArchive::new(f).map_err(|e| anyhow!("{file} isn't a ZIP that can be read: {e}"))
}

/// The DAT (or the DATs in a ZIP) at `path`, read without importing them (a collection
/// made from a DAT, D49).
pub fn read_file(path: &Path) -> Result<Vec<Dat>> {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !is_zip(path)? {
        return Ok(vec![parse_file(path).with_context(|| format!("{file} isn't a DAT that can be read"))?]);
    }
    let mut zip = open_zip(path, &file)?;
    let mut out = vec![];
    let never = AtomicBool::new(false);
    for (i, name, _) in zip_dats(&mut zip)? {
        let m = zip.by_index(i)?;
        out.push(parse_reading(m, &never, &mut |_| {}).with_context(|| format!("{file} › {name} isn't a DAT that can be read"))?);
    }
    if out.is_empty() {
        bail!("{file} has no .dat or .xml file inside.");
    }
    Ok(out)
}

/// Import the DAT (or the DATs in a ZIP) at `path`. `answers` maps a DAT's name to its
/// platform, for those whose platform couldn't be worked out.
pub fn import(ix: &Index, lib: &Library, path: &Path, answers: &Map<String, Value>) -> Result<Vec<Imported>> {
    import_with(ix, lib, path, answers, &AtomicBool::new(false), &mut |_, _| {})
}

/// `import`, with Stop and `progress(read, total)` in bytes (a ZIP's DATs' as unpacked).
pub fn import_with(ix: &Index, lib: &Library, path: &Path, answers: &Map<String, Value>, cancel: &AtomicBool, progress: &mut dyn FnMut(u64, u64)) -> Result<Vec<Imported>> {
    lib.writable()?;
    let _lock = crate::lock::take(lib, "importing DATs")?;
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !is_zip(path)? {
        let total = std::fs::metadata(path)?.len();
        let f = std::fs::File::open(path).with_context(|| format!("couldn't read {}", path.display()))?;
        let dat = parse_reading(f, cancel, &mut |n| progress(n, total)).with_context(|| format!("{file} isn't a DAT that can be read"))?;
        return Ok(vec![import_parsed(ix, lib, dat, Kept::File(path), &file, answers)?]);
    }
    let mut zip = open_zip(path, &file)?;
    let members = zip_dats(&mut zip)?;
    if members.is_empty() {
        bail!("{file} has no .dat or .xml file inside.");
    }
    let total: u64 = members.iter().map(|m| m.2).sum();
    let dir = lib.root().join(APP_DIR).join("dats");
    std::fs::create_dir_all(&dir)?;
    let mut out = vec![];
    let mut before = 0;
    for (i, name, size) in members {
        let label = format!("{file} › {name}");
        // unpacked beside the DATs as it's read, for the library to keep
        let tmp = dir.join(format!(".importing-{}-{i}.tmp", std::process::id()));
        let read = (|| -> Result<Dat> {
            let mut copy = std::fs::File::create(&tmp).with_context(|| format!("couldn't write {}", tmp.display()))?;
            let m = zip.by_index(i)?;
            let tee = Tee { inner: m, copy: &mut copy };
            parse_reading(tee, cancel, &mut |n| progress(before + n, total)).with_context(|| format!("{label} isn't a DAT that can be read"))
        })();
        let done = read.and_then(|dat| import_parsed(ix, lib, dat, Kept::Unpacked(&tmp), &label, answers));
        let _ = std::fs::remove_file(&tmp);
        out.push(done?);
        before += size;
    }
    Ok(out)
}

/// What's read is also written to `copy`.
struct Tee<'a, R> {
    inner: R,
    copy: &'a mut std::fs::File,
}

impl<R: Read> Read for Tee<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        std::io::Write::write_all(self.copy, &buf[..n])?;
        Ok(n)
    }
}

/// Where an imported DAT's text is, for the library to keep a copy.
enum Kept<'a> {
    Bytes(&'a [u8]),
    /// The file picked (copied in)
    File(&'a Path),
    /// A ZIP's member unpacked beside the DATs (moved in)
    Unpacked(&'a Path),
}

impl Kept<'_> {
    /// Whether the file at `p` holds the same.
    fn same_as(&self, p: &Path) -> bool {
        match self {
            Kept::Bytes(b) => std::fs::metadata(p).is_ok_and(|m| m.len() == b.len() as u64) && std::fs::read(p).ok().as_deref() == Some(*b),
            Kept::File(f) | Kept::Unpacked(f) => same_contents(f, p),
        }
    }

    fn text(&self) -> String {
        match self {
            Kept::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
            Kept::File(f) | Kept::Unpacked(f) => String::from_utf8_lossy(&std::fs::read(f).unwrap_or_default()).into_owned(),
        }
    }

    fn keep_at(&self, dest: &Path) -> Result<()> {
        match self {
            Kept::Bytes(b) => crate::config::write_atomic(dest, b),
            Kept::File(f) => {
                if let Some(d) = dest.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let tmp = dest.with_extension(format!("tmp-{}", std::process::id()));
                std::fs::copy(f, &tmp).with_context(|| format!("couldn't write {}", tmp.display()))?;
                std::fs::rename(&tmp, dest).with_context(|| format!("couldn't write {}", dest.display()))?;
                Ok(())
            }
            Kept::Unpacked(f) => {
                if let Some(d) = dest.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::rename(f, dest).with_context(|| format!("couldn't write {}", dest.display()))
            }
        }
    }
}

/// Two files with the same bytes, read a piece at a time.
fn same_contents(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else { return false };
    if ma.len() != mb.len() {
        return false;
    }
    let (Ok(fa), Ok(fb)) = (std::fs::File::open(a), std::fs::File::open(b)) else { return false };
    let (mut ra, mut rb) = (BufReader::with_capacity(1 << 20, fa), BufReader::with_capacity(1 << 20, fb));
    loop {
        let (Ok(x), Ok(y)) = (ra.fill_buf(), rb.fill_buf()) else { return false };
        if x.is_empty() || y.is_empty() {
            return x.is_empty() && y.is_empty();
        }
        let n = x.len().min(y.len());
        if x[..n] != y[..n] {
            return false;
        }
        ra.consume(n);
        rb.consume(n);
    }
}

pub fn import_bytes(ix: &Index, lib: &Library, bytes: &[u8], file: &str, answers: &Map<String, Value>) -> Result<Imported> {
    let dat = parse(bytes).with_context(|| format!("{file} isn't a DAT that can be read"))?;
    import_parsed(ix, lib, dat, Kept::Bytes(bytes), file, answers)
}

fn import_parsed(ix: &Index, lib: &Library, dat: Dat, kept: Kept, file: &str, answers: &Map<String, Value>) -> Result<Imported> {
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
    let dat = if platform == crate::bios::ID { crate::bios::by_system(&kept.text(), dat) } else { dat };
    let version = if dat.version.is_empty() { "undated".to_string() } else { dat.version.clone() };
    let dir = lib.root().join(APP_DIR).join("dats").join(&platform);
    let base = safe_name(&format!("{} ({})", dat.name, version));
    let mut n = 1;
    let rel = loop {
        let fname = if n == 1 { format!("{base}.dat") } else { format!("{base} {n}.dat") };
        let p = dir.join(&fname);
        if !p.exists() || kept.same_as(&p) {
            break format!("{APP_DIR}/dats/{platform}/{fname}");
        }
        n += 1;
    };
    let dest = lib.resolve(&rel)?;
    let existing: Option<i64> = ix.conn().query_row("SELECT id FROM dats WHERE source_path = ?1", [&rel], |r| r.get(0)).ok();
    let already = dest.exists() && existing.is_some();
    if !dest.exists() {
        kept.keep_at(&dest)?;
    }
    let id = match existing {
        Some(id) if already => id,
        _ => {
            let id = load(ix.conn(), &rel, &platform, &dat)?;
            crate::arcade::loaded(ix.conn(), lib, id, &platform)?;
            keep_stamp(ix.conn(), &rel, &dest)?;
            id
        }
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
pub(crate) fn load(conn: &Connection, rel: &str, platform: &str, dat: &Dat) -> Result<i64> {
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
            "INSERT INTO dat_games (dat_id, name, description, title, regions, languages, revision, version, flags, romof, sampleof, kind, year, manufacturer, cloneof)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?;
        let mut r_ins = tx.prepare("INSERT INTO dat_roms (game_id, dat_id, name, size, crc32, md5, sha1, status, merge, disk) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)")?;
        let mut l_ins = tx.prepare("INSERT INTO dat_links (game_id, kind, name) VALUES (?1, ?2, ?3)")?;
        let join = |v: &[String]| (!v.is_empty()).then(|| v.join(", "));
        // an arcade machine's name is a short name ("sf2"): its title is its description's
        let arcade = platforms::is_arcade(platform);
        // a variant's title is its whole name: its tags are part of it (D121)
        let custom = dat.version == crate::variants::VERSION;
        for g in &dat.games {
            let f = parse_name(if arcade && !g.description.is_empty() { &g.description } else { &g.name });
            let title = if custom { g.name.clone() } else { f.title };
            g_ins.execute(params![
                dat_id, g.name, g.description, title, join(&f.regions), join(&f.languages), f.revision, f.version, join(&f.flags),
                g.romof, g.sampleof, g.kind, g.year, g.manufacturer, g.cloneof
            ])?;
            let gid = tx.last_insert_rowid();
            ids.push(gid);
            by_name.insert(&g.name, gid);
            if let Some(n) = &g.id {
                by_nid.insert(n, gid);
            }
            for r in &g.roms {
                r_ins.execute(params![gid, dat_id, r.name, r.size.map(|s| s as i64), r.crc32, r.md5, r.sha1, r.status, r.merge, r.disk as i64])?;
            }
            for d in &g.devices {
                l_ins.execute(params![gid, "device", d])?;
            }
            for s in &g.samples {
                l_ins.execute(params![gid, "sample", s])?;
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
    let mut q = conn.prepare("SELECT id, platform, name, version, imported, source_path FROM dats ORDER BY platform, name, version != 'undated', version, imported")?;
    let rows: Vec<(i64, String, String, String)> = q
        .query_map([], |r| Ok((r.get(0)?, format!("{}/{}", r.get::<_, String>(1)?, r.get::<_, String>(2)?), r.get(5)?, r.get(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    // rows are in version order, so the newest is last (an undated DAT counts as the oldest,
    // review M14); a chosen version wins while it's there
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
    // and the DATs each system is audited with too, by name (D118)
    tx.execute("UPDATE dats SET also = 0", [])?;
    for (platform, names) in lib.meta()["also_dats"].as_object().cloned().unwrap_or_default() {
        for n in names.as_array().into_iter().flatten().filter_map(Value::as_str) {
            tx.execute("UPDATE dats SET also = 1 WHERE platform = ?1 AND name = ?2 AND version != 'custom'", params![platform, n])?;
        }
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

/// A DAT file's size and modified time, which say when it changed in its place.
fn file_stamp(p: &Path) -> Option<String> {
    let m = std::fs::metadata(p).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some(format!("{}:{t}", m.len()))
}

/// Keep what a DAT file was when it was loaded (`sync_library` loads it again when it changes).
pub(crate) fn keep_stamp(conn: &Connection, rel: &str, p: &Path) -> Result<()> {
    if let Some(st) = file_stamp(p) {
        conn.execute(
            "INSERT INTO index_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("dat_file:{rel}"), st],
        )?;
    }
    Ok(())
}

/// Bring the index's DATs in line with `_library/dats/` (the library is the source of
/// truth): load files the index doesn't have or that changed, drop those that are gone.
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
            conn.execute("DELETE FROM index_meta WHERE key = ?1", [format!("dat_file:{k}")])?;
            changed = true;
        }
    }
    for (rel, platform) in &on_disk {
        let path = lib.resolve(rel)?;
        let now_stamp = file_stamp(&path);
        if known.contains(rel) {
            // a DAT changed in its place (a custom DAT, written on another computer or by
            // Game Repo Server's library) is loaded again; one never stamped is stamped now
            let kept: Option<String> = conn.query_row("SELECT value FROM index_meta WHERE key = ?1", [format!("dat_file:{rel}")], |r| r.get(0)).optional()?;
            match kept {
                Some(k) if Some(&k) != now_stamp.as_ref() => {}
                Some(_) => continue,
                None => {
                    keep_stamp(conn, rel, &path)?;
                    continue;
                }
            }
        }
        let Ok(dat) = parse_file(&path) else { continue };
        let dat = if platform == crate::bios::ID { crate::bios::by_system(&Kept::File(&path).text(), dat) } else { dat };
        let id = load(conn, rel, platform, &dat)?;
        crate::arcade::loaded(conn, lib, id, platform)?;
        crate::audit::match_dat(conn, id)?;
        keep_stamp(conn, rel, &path)?;
        changed = true;
    }
    // a system made arcade (or not) since, or a set type changed on another computer (M11)
    changed |= crate::arcade::refresh_places(conn, lib)?;
    if changed {
        update_current(conn, lib)?;
        crate::audit::changed(conn)?;
    }
    Ok(())
}

/// The imported DATs, grouped as the page lists them.
pub fn list(ix: &Index) -> Result<Value> {
    let mut q = ix.conn().prepare(
        "SELECT id, platform, name, version, description, author, source_path, imported, current, games, roms FROM dats ORDER BY platform, name, version != 'undated' DESC, version DESC",
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

    /// A parsed DAT loaded into an index for `platform` (its id).
    pub fn load_for_test(conn: &Connection, platform: &str, dat: &Dat) -> i64 {
        load(conn, &format!("{APP_DIR}/dats/{platform}/{}.dat", dat.name), platform, dat).unwrap()
    }

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

    #[test]
    fn a_big_dat_is_read_as_its_parsed_with_progress_and_stop_and_a_zips_dats_kept_as_they_were() {
        // a byte order mark and white space before it, read one byte at a time
        let slow = format!("\u{feff}\n  {LOGIQX}");
        assert_eq!(parse_from(BufReader::with_capacity(1, slow.as_bytes())).unwrap(), parse(LOGIQX.as_bytes()).unwrap());
        let (dir, lib, ix) = crate::collection::tests::fixture("bigdat");
        let mut big = String::from("<?xml version=\"1.0\"?>\n<datafile><header><name>Nintendo - Game Boy</name><description>x</description><version>20261022</version></header>\n");
        for i in 0..30_000 {
            big.push_str(&format!("<game name=\"Game {i:05} (World)\"><description>Game {i:05}</description><rom name=\"Game {i:05} (World).gb\" size=\"32768\" crc=\"{i:08x}\" sha1=\"{i:040x}\"/></game>\n"));
        }
        big.push_str("</datafile>\n");
        let path = dir.join("big.dat");
        std::fs::write(&path, &big).unwrap();
        let mut seen = vec![];
        let r = import_with(&ix, &lib, &path, &Map::new(), &AtomicBool::new(false), &mut |n, t| seen.push((n, t))).unwrap();
        let total = big.len() as u64;
        assert!(seen.len() >= 4 && seen.windows(2).all(|w| w[0].0 <= w[1].0) && seen.last() == Some(&(total, total)), "{seen:?}");
        let Imported::Loaded(v) = &r[0] else { panic!("{r:?}") };
        assert_eq!(v["games"], 30_000);
        let kept = lib.resolve(v["path"].as_str().unwrap()).unwrap();
        assert!(same_contents(&kept, &path));
        let Imported::Loaded(again) = import(&ix, &lib, &path, &Map::new()).unwrap().remove(0) else { panic!() };
        assert_eq!((&again["path"], &again["already"]), (&v["path"], &json!(true)));

        // stopped part way: nothing imported, nothing left behind
        let path2 = dir.join("big2.dat");
        std::fs::write(&path2, big.replace("20261022", "20261023")).unwrap();
        let stop = AtomicBool::new(false);
        let e = import_with(&ix, &lib, &path2, &Map::new(), &stop, &mut |n, _| {
            if n > 1 << 20 {
                stop.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err();
        assert!(format!("{e:#}").contains("stopped"), "{e:#}");
        let dats = lib.root().join(APP_DIR).join("dats");
        fn names(d: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                out.push(e.file_name().to_string_lossy().into_owned());
                if e.path().is_dir() {
                    names(&e.path(), out);
                }
            }
        }
        let mut all = vec![];
        names(&dats, &mut all);
        assert!(!all.iter().any(|n| n.contains("20261023")), "{all:?}");

        // a ZIP's DATs, each read from the ZIP as it's parsed and kept in the library as it was
        let gba = LOGIQX.replace("Nintendo - Game Boy (Parent-Clone)", "Nintendo - Game Boy Advance");
        crate::audit::tests::zip(&dir.join("two.zip"), &[("a.dat", LOGIQX.as_bytes()), ("readme.txt", b"x"), ("b.xml", gba.as_bytes())]);
        let r = import(&ix, &lib, &dir.join("two.zip"), &Map::new()).unwrap();
        assert_eq!(r.len(), 2);
        for (imp, text) in r.iter().zip([LOGIQX.to_string(), gba]) {
            let Imported::Loaded(v) = imp else { panic!("{imp:?}") };
            assert_eq!(std::fs::read_to_string(lib.resolve(v["path"].as_str().unwrap()).unwrap()).unwrap(), text);
        }
        let mut all = vec![];
        names(&dats, &mut all);
        assert!(!all.iter().any(|n| n.starts_with(".importing") || n.contains(".tmp")), "{all:?}");
        assert_eq!(read_file(&dir.join("two.zip")).unwrap().iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["Nintendo - Game Boy (Parent-Clone)", "Nintendo - Game Boy Advance"]);
    }
}
