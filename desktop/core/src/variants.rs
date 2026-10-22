//! Variants (docs/PLAN.md, "Phase 10 design", D119–D123): the translations, hacks,
//! prototypes and mods no DAT knows, kept in a custom DAT for their system that Game Repo
//! writes (`_library/dats/<system>/Custom - <System> (custom).dat`), each a variant of its
//! original in the system's main DAT (`cloneof`), or a game of its own. Its tags are the
//! parentheticals after its title, kept in its name; its original is suggested by comparing
//! names; its details and pictures are taken from the original's game folder into its own.

use crate::audit::{self, GAMES};
use crate::dat::{self, Dat, Game, Rom};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::{meta, platforms, sources};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The version every custom DAT has, which tells it from the DATs imported.
pub const VERSION: &str = "custom";
/// How many earlier versions of a custom DAT are kept, for Undo.
const KEEP: usize = 20;
/// The tags offered before any are added (D121).
pub const TAGS: &[&str] = &[
    "Prototype", "Beta", "Demo", "English Translated", "Translated", "Hack", "Mod", "Homebrew", "Unlicensed", "Pirate", "Bug Fix",
    "Restoration", "Colorized", "Enhanced",
];

/// A system's custom DAT's name.
pub fn dat_name(platform: &str) -> String {
    format!("Custom - {}", platforms::name_of(platform))
}

/// Where a system's custom DAT is kept, relative to the library.
fn rel(platform: &str) -> String {
    format!("{APP_DIR}/dats/{platform}/{} ({VERSION}).dat", dat::safe_name(&dat_name(platform)))
}

fn versions_dir(lib: &Library, platform: &str) -> PathBuf {
    lib.root().join(APP_DIR).join("dats").join(platform).join("custom-versions")
}

/// A system's custom DAT in the index, if it has one.
pub fn custom_dat(conn: &Connection, platform: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM dats WHERE platform = ?1 AND version = ?2 ORDER BY id DESC LIMIT 1", params![platform, VERSION], |r| r.get(0))
        .optional()?)
}

/// Whether a DAT (by id) is a custom DAT.
pub fn is_custom(conn: &Connection, dat_id: i64) -> Result<bool> {
    Ok(conn.query_row("SELECT version = ?2 FROM dats WHERE id = ?1", params![dat_id, VERSION], |r| r.get(0)).optional()?.unwrap_or(false))
}

// ------------------------------------------------------------ names and tags

/// A name's title and what follows it: its parentheticals and brackets, as written.
pub fn split_name(name: &str) -> (&str, &str) {
    let cut = name.find(['(', '[']).unwrap_or(name.len());
    (name[..cut].trim_end(), name[cut..].trim())
}

/// GoodTools' languages in its translation codes ("[T+Eng]").
const LANGUAGES: &[(&str, &str)] = &[
    ("eng", "English"), ("fre", "French"), ("ger", "German"), ("spa", "Spanish"), ("ita", "Italian"), ("por", "Portuguese"),
    ("bra", "Brazilian Portuguese"), ("rus", "Russian"), ("chi", "Chinese"), ("kor", "Korean"), ("dut", "Dutch"), ("swe", "Swedish"),
    ("pol", "Polish"), ("gre", "Greek"), ("tur", "Turkish"), ("ara", "Arabic"), ("heb", "Hebrew"), ("cat", "Catalan"),
];

/// GoodTools' one-letter regions ("(U)", "(JU)").
const REGION_CODES: &[(char, &str)] = &[
    ('U', "USA"), ('E', "Europe"), ('J', "Japan"), ('W', "World"), ('F', "France"), ('G', "Germany"), ('S', "Spain"), ('I', "Italy"),
    ('K', "Korea"), ('C', "China"), ('A', "Australia"), ('B', "Brazil"), ('H', "Netherlands"),
];

/// A GoodTools code as words: `[T+Eng]` English Translated, `[h1C]` Hack, `(U)` USA; `[!]`
/// (a good dump) is nothing; anything else is itself.
fn goodtools(inner: &str, bracket: bool) -> Vec<String> {
    let s = inner.trim();
    if bracket {
        if s == "!" {
            return vec![];
        }
        if let Some(rest) = s.strip_prefix("T+").or_else(|| s.strip_prefix("T-")) {
            let code: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect::<String>().to_ascii_lowercase();
            let lang = LANGUAGES.iter().find(|(c, _)| *c == code).map(|(_, l)| *l);
            return vec![lang.map(|l| format!("{l} Translated")).unwrap_or_else(|| "Translated".into())];
        }
        let mut cs = s.chars();
        if let Some(first) = cs.next() {
            let rest: String = cs.collect();
            if s.len() <= 4 && rest.chars().all(|c| c.is_ascii_alphanumeric()) && (rest.is_empty() || rest.starts_with(|c: char| c.is_ascii_digit() || c.is_ascii_uppercase())) {
                let word = match first {
                    'h' => Some("Hack"),
                    'p' => Some("Pirate"),
                    'b' => Some("Bad Dump"),
                    'a' => Some("Alternate"),
                    'f' => Some("Fixed"),
                    'o' => Some("Overdump"),
                    't' => Some("Trainer"),
                    _ => None,
                };
                if let Some(w) = word {
                    return vec![w.to_string()];
                }
            }
        }
    } else if !s.is_empty() && s.len() <= 3 && s.chars().all(|c| REGION_CODES.iter().any(|(k, _)| *k == c)) {
        return s.chars().filter_map(|c| REGION_CODES.iter().find(|(k, _)| *k == c).map(|(_, r)| r.to_string())).collect();
    }
    vec![s.to_string()]
}

/// A name's tags: each part of the parentheticals and brackets after its title, split at
/// commas, GoodTools' codes as words (D121).
pub fn tags_of(name: &str) -> Vec<String> {
    let (_, mut rest) = split_name(name);
    let mut out: Vec<String> = vec![];
    while let Some(open) = rest.find(['(', '[']) {
        let bracket = rest.as_bytes()[open] == b'[';
        let close = if bracket { ']' } else { ')' };
        let Some(len) = rest[open + 1..].find(close) else { break };
        let inner = &rest[open + 1..open + 1 + len];
        rest = &rest[open + 1 + len + 1..];
        for part in inner.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            for t in goodtools(part, bracket) {
                if !t.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                    out.push(t);
                }
            }
        }
    }
    out
}

/// A variant's name from its title and tags: what it was (`was`) keeps how its tags were
/// written while they're the same; changed, each tag is in parentheses of its own.
pub fn compose(title: &str, tags: &[String], was: &str) -> String {
    let title = title.trim();
    if !was.is_empty() && split_name(was).0 == title && tags_of(was) == tags {
        return was.to_string();
    }
    let mut s = title.to_string();
    for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        s.push_str(&format!(" ({t})"));
    }
    s
}

fn check_name(name: &str) -> Result<()> {
    let n = name.trim();
    if n.is_empty() {
        bail!("A variant needs a name.");
    }
    if n.len() > 200 || n.chars().any(|c| "<>:\"/\\|?*".contains(c) || c.is_control()) {
        bail!("A name can't have any of < > : \" / \\ | ? * in it.");
    }
    Ok(())
}

// ------------------------------------------------------------ the original, by name (D120)

/// Common letters with accents, without them.
fn fold(c: char) -> &'static str {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => "a",
        'ç' => "c",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => "o",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'ý' | 'ÿ' => "y",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        _ => "",
    }
}

/// A Roman numeral of two letters or more (II to XXXIX) as a number; "V", "X" and "I" alone
/// are left, as they're as often a letter ("Mega Man X").
fn roman(w: &str) -> Option<u32> {
    if w.len() < 2 || !w.chars().all(|c| "ivx".contains(c)) {
        return None;
    }
    let val = |c| match c {
        'i' => 1,
        'v' => 5,
        _ => 10,
    };
    let cs: Vec<u32> = w.chars().map(val).collect();
    let mut n = 0;
    for (i, x) in cs.iter().enumerate() {
        if cs.get(i + 1).is_some_and(|y| y > x) { n -= *x as i32 } else { n += *x as i32 }
    }
    let n = u32::try_from(n).ok().filter(|n| (2..40).contains(n))?;
    // only the usual way of writing it ("IIII" isn't 4)
    let mut s = "x".repeat((n / 10) as usize);
    s.push_str(["", "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix"][(n % 10) as usize]);
    (s == w).then_some(n)
}

/// A title's words as compared: lower case, without accents and punctuation, "&" as "and",
/// Roman numerals as numbers, "The" at either end dropped.
pub fn words(title: &str) -> Vec<String> {
    let lower = title.to_lowercase().replace('&', " and ");
    let mut s = String::with_capacity(lower.len());
    for c in lower.chars() {
        let f = fold(c);
        if !f.is_empty() {
            s.push_str(f);
        } else if c.is_alphanumeric() {
            s.push(c);
        } else if c != '\'' {
            s.push(' ');
        }
    }
    let mut w: Vec<String> = s.split_whitespace().map(|x| roman(x).map(|n| n.to_string()).unwrap_or_else(|| x.to_string())).collect();
    while w.first().is_some_and(|x| x == "the") {
        w.remove(0);
    }
    while w.last().is_some_and(|x| x == "the") {
        w.pop();
    }
    w
}

/// Words too common to find a title by.
pub(crate) const COMMON: &[&str] = &["the", "of", "and", "a", "an", "no", "de", "la", "le", "to", "in", "on", "for", "game", "edition", "version"];

fn bigrams(s: &str) -> HashSet<(char, char)> {
    let c: Vec<char> = s.chars().collect();
    c.windows(2).map(|w| (w[0], w[1])).collect()
}

fn dice<T: std::hash::Hash + Eq>(a: &HashSet<T>, b: &HashSet<T>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    2.0 * a.intersection(b).count() as f64 / (a.len() + b.len()) as f64
}

/// How alike two titles' words are, 0 to 1: 1 the same title (its words run together, so
/// "Fire Red" is "FireRed"); a DAT title the file's starts with ("Super Mario Bros. 3 -
/// Mario Adventure") 0.8; else by the words, and the pairs of letters, they share.
fn alike(file: &[String], dat: &[String]) -> f64 {
    let (a, b) = (file.concat(), dat.concat());
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let prefix = (dat.len() >= 2 && file.starts_with(dat)) as u8 as f64 * 0.8;
    let wa: HashSet<&String> = file.iter().collect();
    let wb: HashSet<&String> = dat.iter().collect();
    prefix.max(dice(&wa, &wb)).max(dice(&bigrams(&a), &bigrams(&b)) * 0.95)
}

/// How close a suggestion has to be to be offered.
const CLOSE: f64 = 0.6;

/// The main DAT's games, ready to be compared with names.
pub struct Titles {
    games: Vec<TitleGame>,
    exact: HashMap<String, Vec<usize>>,
    by_word: HashMap<String, Vec<usize>>,
    region_order: Vec<String>,
}

struct TitleGame {
    id: i64,
    name: String,
    words: Vec<String>,
    regions: Vec<String>,
    flagged: bool,
    sizes: Vec<i64>,
}

impl Titles {
    pub fn of(conn: &Connection, dat_id: i64) -> Result<Titles> {
        let mut q = conn.prepare(
            "SELECT g.id, g.name, COALESCE(g.title, ''), COALESCE(g.regions, ''), COALESCE(g.flags, ''), COALESCE(group_concat(r.size), '')
             FROM dat_games g LEFT JOIN dat_roms r ON r.game_id = g.id WHERE g.dat_id = ?1 AND (g.kind IS NULL OR g.kind NOT IN ('bios', 'device'))
             GROUP BY g.id ORDER BY g.name",
        )?;
        let mut t = Titles { games: vec![], exact: HashMap::new(), by_word: HashMap::new(), region_order: crate::collection::Rules::default().regions };
        let mut rows = q.query([dat_id])?;
        while let Some(r) = rows.next()? {
            let name: String = r.get(1)?;
            if crate::addons::kind_of(&name, false).is_some() {
                continue;
            }
            let title: String = r.get(2)?;
            let w = words(if title.is_empty() { split_name(&name).0 } else { &title });
            let i = t.games.len();
            t.exact.entry(w.concat()).or_default().push(i);
            for x in w.iter().filter(|x| !COMMON.contains(&x.as_str())) {
                let list = t.by_word.entry(x.clone()).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
            t.games.push(TitleGame {
                id: r.get(0)?,
                name,
                words: w,
                regions: r.get::<_, String>(3)?.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect(),
                flagged: !r.get::<_, String>(4)?.is_empty(),
                sizes: r.get::<_, String>(5)?.split(',').filter_map(|x| x.parse().ok()).collect(),
            });
        }
        Ok(t)
    }

    /// Up to three games for a file (its name without the extension, and its size): the
    /// best of each title, the sure ones first.
    pub fn suggest(&self, name: &str, size: Option<i64>) -> Vec<Value> {
        let (title, _) = split_name(name);
        let w = words(title);
        if w.is_empty() {
            return vec![];
        }
        let tags = tags_of(name);
        let mut scored: Vec<(f64, usize)> = vec![];
        if let Some(same) = self.exact.get(&w.concat()) {
            scored.extend(same.iter().map(|&i| (1.0, i)));
        } else {
            let mut seen: HashSet<usize> = HashSet::new();
            for x in w.iter().filter(|x| !COMMON.contains(&x.as_str())) {
                for &i in self.by_word.get(x).into_iter().flatten() {
                    if seen.insert(i) {
                        let s = alike(&w, &self.games[i].words);
                        if s >= CLOSE {
                            scored.push((s, i));
                        }
                    }
                }
            }
        }
        let rank = |g: &TitleGame| {
            let size_fits = size.is_some_and(|z| g.sizes.contains(&z));
            let region_named = g.regions.iter().any(|r| tags.iter().any(|t| t.eq_ignore_ascii_case(r)));
            let order = g.regions.iter().filter_map(|r| self.region_order.iter().position(|x| x.eq_ignore_ascii_case(r))).min().unwrap_or(self.region_order.len());
            (!size_fits, !region_named, order, g.flagged, g.name.len())
        };
        // the best release of each title
        let mut best: HashMap<String, (f64, usize)> = HashMap::new();
        for (s, i) in scored {
            let key = self.games[i].words.concat();
            match best.get(&key) {
                Some(&(bs, bi)) if (bs, std::cmp::Reverse(rank(&self.games[bi]))) >= (s, std::cmp::Reverse(rank(&self.games[i]))) => {}
                _ => {
                    best.insert(key, (s, i));
                }
            }
        }
        let mut v: Vec<(f64, usize)> = best.into_values().collect();
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then_with(|| self.games[a.1].name.cmp(&self.games[b.1].name)));
        v.into_iter()
            .take(3)
            .map(|(s, i)| json!({ "id": self.games[i].id, "name": self.games[i].name, "how": if s >= 1.0 { "same" } else { "close" } }))
            .collect()
    }
}

/// The main DAT's games whose names have these words, for *Choose the game…*.
pub fn search(conn: &Connection, lib: &Library, platform: &str, text: &str, limit: i64) -> Result<Value> {
    let Some(main) = audit::primary_dat(conn, lib, platform)? else {
        bail!("{} has no DAT to choose a game from.", platforms::name_of(platform));
    };
    let mut sql = String::from("SELECT name FROM dat_games WHERE dat_id = ?1");
    let ws: Vec<String> = text.split_whitespace().take(6).map(|w| format!("%{}%", w.replace(['%', '_'], ""))).collect();
    for i in 0..ws.len() {
        sql.push_str(&format!(" AND name LIKE ?{}", i + 2));
    }
    sql.push_str(&format!(" ORDER BY name LIMIT {}", limit.clamp(1, 200)));
    let mut q = conn.prepare(&sql)?;
    let mut args: Vec<rusqlite::types::Value> = vec![main.into()];
    args.extend(ws.into_iter().map(rusqlite::types::Value::from));
    let names: Vec<String> = q.query_map(rusqlite::params_from_iter(args), |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(json!(names))
}

/// Audit's *Unknown* list for a main DAT, each file with its suggested originals and its
/// tags (D120, D123).
pub fn annotate(conn: &Connection, dat_id: i64, items: &mut [Value]) -> Result<()> {
    if items.is_empty() || is_custom(conn, dat_id)? {
        return Ok(());
    }
    let t = Titles::of(conn, dat_id)?;
    for it in items.iter_mut() {
        let file = it["name"].as_str().unwrap_or("").to_string();
        let stem = stem(&file);
        it["suggested"] = json!(t.suggest(&stem, it["size"].as_i64()));
        it["tags"] = json!(tags_of(&stem));
        it["title"] = json!(split_name(&stem).0);
    }
    Ok(())
}

/// A file's name without its extension.
pub fn stem(file: &str) -> String {
    match file.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && e.len() <= 4 && e.chars().all(|c| c.is_ascii_alphanumeric()) => s.to_string(),
        _ => file.to_string(),
    }
}

// ------------------------------------------------------------ the tag list (D121)

/// The tags offered: the list kept in library.json (`variant_tags`; the starting ones until
/// it's changed) and every tag the custom DATs' games have.
pub fn tag_list(conn: &Connection, lib: &Library) -> Result<Vec<String>> {
    let mut v: Vec<String> = match lib.meta()["variant_tags"].as_array() {
        Some(a) => a.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        None => TAGS.iter().map(|s| s.to_string()).collect(),
    };
    let mut q = conn.prepare("SELECT g.name FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE d.version = ?1")?;
    for n in q.query_map([VERSION], |r| r.get::<_, String>(0))? {
        for t in tags_of(&n?) {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                v.push(t);
            }
        }
    }
    Ok(v)
}

/// Set the tag list (Settings, or *New tag…*).
pub fn set_tags(lib: &Library, tags: &[String]) -> Result<()> {
    lib.writable()?;
    let mut v: Vec<String> = vec![];
    for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if t.contains(['(', ')', '[', ']', ',']) || t.len() > 60 {
            bail!("A tag can't have brackets or a comma in it.");
        }
        if !v.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            v.push(t.to_string());
        }
    }
    lib.update_meta(json!({ "variant_tags": v }))?;
    Ok(())
}

// ------------------------------------------------------------ the custom DAT (D119)

/// A system's custom DAT as its file has it (an empty one when there's none yet).
pub fn read(lib: &Library, platform: &str) -> Result<Dat> {
    let p = lib.resolve(&rel(platform))?;
    if p.is_file() {
        return dat::parse(&std::fs::read(&p)?);
    }
    Ok(Dat { name: dat_name(platform), description: dat_name(platform), version: VERSION.into(), author: "Game Repo".into(), games: vec![] })
}

fn rom_xml(r: &Rom) -> String {
    let x = audit::xml;
    let mut s = format!("\t\t<rom name=\"{}\"", x(&r.name));
    if let Some(z) = r.size {
        s.push_str(&format!(" size=\"{z}\""));
    }
    for (k, v) in [("crc", &r.crc32), ("md5", &r.md5), ("sha1", &r.sha1)] {
        if let Some(v) = v {
            s.push_str(&format!(" {k}=\"{v}\""));
        }
    }
    s.push_str("/>\n");
    s
}

fn to_xml(platform: &str, d: &Dat) -> String {
    let x = audit::xml;
    let mut s = format!(
        "<?xml version=\"1.0\"?>\n<datafile>\n\t<header>\n\t\t<name>{}</name>\n\t\t<description>{}</description>\n\t\t<version>{VERSION}</version>\n\t\t<author>Game Repo</author>\n\t\t<comment>{}</comment>\n\t</header>\n",
        x(&dat_name(platform)),
        x(&format!("The variants Game Repo keeps for {}", platforms::name_of(platform))),
        x("Translations, hacks, prototypes and mods. A game's cloneof names its original in the system's main DAT, not in this one."),
    );
    let mut games: Vec<&Game> = d.games.iter().collect();
    games.sort_by(|a, b| a.name.cmp(&b.name));
    for g in games {
        let clone = g.cloneof.as_ref().map(|c| format!(" cloneof=\"{}\"", x(c))).unwrap_or_default();
        s.push_str(&format!("\t<game name=\"{}\"{clone}>\n\t\t<description>{}</description>\n", x(&g.name), x(&g.name)));
        for r in &g.roms {
            s.push_str(&rom_xml(r));
        }
        s.push_str("\t</game>\n");
    }
    s.push_str("</datafile>\n");
    s
}

/// Write a system's custom DAT (the one before kept for Undo), and load and match it again.
fn write(ix: &Index, lib: &Library, platform: &str, d: &Dat, keep_before: bool) -> Result<i64> {
    lib.writable()?;
    let rel = rel(platform);
    let path = lib.resolve(&rel)?;
    if keep_before && path.is_file() {
        let dir = versions_dir(lib, platform);
        std::fs::create_dir_all(&dir)?;
        // named so they sort oldest first: the time, then a count within it
        let stamp: String = now().chars().filter(|c| c.is_ascii_digit()).collect();
        let mut n = 1;
        let mut to = dir.join(format!("{stamp}-{n:03}.dat"));
        while to.exists() {
            n += 1;
            to = dir.join(format!("{stamp}-{n:03}.dat"));
        }
        std::fs::copy(&path, &to)?;
        let mut kept = version_files(lib, platform);
        while kept.len() > KEEP {
            let _ = std::fs::remove_file(kept.remove(0));
        }
    }
    crate::config::write_atomic(&path, to_xml(platform, d).as_bytes())?;
    let parsed = dat::parse(&std::fs::read(&path)?)?;
    let conn = ix.conn();
    let id = dat::load(conn, &rel, platform, &parsed)?;
    dat::keep_stamp(conn, &rel, &path)?;
    dat::update_current(conn, lib)?;
    audit::match_dat(conn, id)?;
    Ok(id)
}

/// The versions kept, oldest first.
fn version_files(lib: &Library, platform: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(versions_dir(lib, platform)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "dat")).collect();
    v.sort();
    v
}

/// What's added: the files (by id) as one game, under `name`, a variant of `original` (a
/// game of the main DAT) or, without one, a game of its own.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct Add {
    pub files: Vec<i64>,
    pub name: String,
    pub original: Option<String>,
}

/// A file's ROM as the custom DAT lists it.
fn rom_of(conn: &Connection, platform: &str, file: i64) -> Result<Rom> {
    conn.query_row(
        "SELECT f.name, b.size, b.crc32, b.md5, b.sha1, f.platform FROM files f JOIN blobs b ON b.id = f.blob_id WHERE f.id = ?1 AND f.hashed > 0",
        [file],
        |r| {
            let p: Option<String> = r.get(5)?;
            Ok((Rom { name: r.get(0)?, size: Some(r.get::<_, i64>(1)? as u64), crc32: Some(r.get(2)?), md5: r.get(3)?, sha1: r.get(4)?, ..Default::default() }, p))
        },
    )
    .optional()?
    .filter(|(_, p)| p.as_deref() == Some(platform))
    .map(|(r, _)| r)
    .ok_or_else(|| anyhow!("File {file} isn't a scanned file of {}.", platforms::name_of(platform)))
}

/// A one-file game's ROM is named for the game, so *Rename to the DAT's names…* names its
/// file for it; a game of several files keeps their names.
fn name_roms(g: &mut Game) {
    if let [r] = g.roms.as_mut_slice() {
        let ext = platforms::ext_of(&r.name);
        r.name = if ext.is_empty() { g.name.clone() } else { format!("{}.{ext}", g.name) };
    }
}

fn main_dat(conn: &Connection, lib: &Library, platform: &str) -> Result<i64> {
    if platforms::is_arcade(platform) {
        bail!("Arcade systems don't have variants: their hacks and bootlegs are sets MAME's own DATs list.");
    }
    audit::primary_dat(conn, lib, platform)?.ok_or_else(|| anyhow!("{} has no DAT yet: import one first.", platforms::name_of(platform)))
}

fn check_original(conn: &Connection, main: i64, original: &Option<String>) -> Result<()> {
    if let Some(o) = original {
        let there: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM dat_games WHERE dat_id = ?1 AND name = ?2)", params![main, o], |r| r.get(0))?;
        if !there {
            bail!("There's no game called {o} in the system's DAT.");
        }
    }
    Ok(())
}

fn check_free(conn: &Connection, main: i64, name: &str) -> Result<()> {
    let taken: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM dat_games WHERE dat_id = ?1 AND name = ?2)", params![main, name], |r| r.get(0))?;
    if taken {
        bail!("{name} is a game of the system's DAT already: give the variant its tags, such as (Hack) or (English Translated).");
    }
    Ok(())
}

/// Add files to a system's custom DAT (D123): each `Add` a game, or more files of one
/// already there by that name. Variants then take their originals' details and pictures.
pub fn add(ix: &Index, lib: &Library, platform: &str, adds: &[Add]) -> Result<Value> {
    lib.writable()?;
    let conn = ix.conn();
    let main = main_dat(conn, lib, platform)?;
    let mut d = read(lib, platform)?;
    let mut names = vec![];
    for a in adds {
        let name = a.name.trim().to_string();
        check_name(&name)?;
        check_free(conn, main, &name)?;
        check_original(conn, main, &a.original)?;
        if a.files.is_empty() {
            bail!("Choose the files for {name}.");
        }
        let mut roms = vec![];
        for f in &a.files {
            let r = rom_of(conn, platform, *f)?;
            let known = d.games.iter().flat_map(|g| &g.roms).any(|x| x.crc32 == r.crc32 && x.size == r.size && (x.sha1.is_none() || r.sha1.is_none() || x.sha1 == r.sha1));
            if !known && !roms.iter().any(|x: &Rom| x.crc32 == r.crc32 && x.size == r.size) {
                roms.push(r);
            }
        }
        if roms.is_empty() {
            continue;
        }
        match d.games.iter_mut().find(|g| g.name == name) {
            Some(g) => {
                g.roms.extend(roms);
                if a.original.is_some() {
                    g.cloneof = a.original.clone();
                }
                name_roms(g);
            }
            None => {
                let mut g = Game { name: name.clone(), description: name.clone(), cloneof: a.original.clone(), roms, ..Default::default() };
                name_roms(&mut g);
                d.games.push(g);
            }
        }
        if !names.contains(&name) {
            names.push(name);
        }
    }
    if names.is_empty() {
        bail!("Those files are in the custom DAT already.");
    }
    let id = write(ix, lib, platform, &d, true)?;
    let taken = take_named(ix, lib, id, &names)?;
    Ok(json!({ "dat": id, "added": names, "taken": taken, "name": dat_name(platform) }))
}

/// Change a variant: its name (and so its tags) and its original (`None`: a game of its own).
pub fn edit(ix: &Index, lib: &Library, platform: &str, name: &str, new_name: &str, original: Option<String>) -> Result<Value> {
    lib.writable()?;
    let conn = ix.conn();
    let main = main_dat(conn, lib, platform)?;
    let new_name = new_name.trim().to_string();
    check_name(&new_name)?;
    check_original(conn, main, &original)?;
    let mut d = read(lib, platform)?;
    if new_name != name {
        check_free(conn, main, &new_name)?;
        if d.games.iter().any(|g| g.name == new_name) {
            bail!("There's a variant called {new_name} already.");
        }
    }
    let g = d.games.iter_mut().find(|g| g.name == name).ok_or_else(|| anyhow!("There's no variant called {name}."))?;
    g.name = new_name.clone();
    g.description = new_name.clone();
    g.cloneof = original;
    name_roms(g);
    // its game folder keeps its name for it, and takes the original's again
    let folder = old_folder(conn, platform, name)?;
    let id = write(ix, lib, platform, &d, true)?;
    if let Some(f) = folder {
        let dir = lib.resolve(&f)?;
        let mut v = meta::read_game(&dir);
        if v.is_object() {
            v["name"] = json!(new_name);
            if !edited(&v, "title") {
                v["title"] = json!(new_name);
            }
            meta::write_game(ix, lib, &dir, &v)?;
        }
    }
    let taken = take_named(ix, lib, id, std::slice::from_ref(&new_name))?;
    Ok(json!({ "dat": id, "name": new_name, "taken": taken }))
}

fn edited(v: &Value, field: &str) -> bool {
    v["edited"].as_array().is_some_and(|a| a.iter().any(|e| e == field))
}

/// The game folder a custom DAT's game has, by its name, before the DAT changes.
fn old_folder(conn: &Connection, platform: &str, name: &str) -> Result<Option<String>> {
    let Some(c) = custom_dat(conn, platform)? else { return Ok(None) };
    let id: Option<i64> = conn.query_row("SELECT id FROM dat_games WHERE dat_id = ?1 AND name = ?2", params![c, name], |r| r.get(0)).optional()?;
    match id {
        Some(id) => meta::folder_of(conn, &meta::dat_game(conn, id)?),
        None => Ok(None),
    }
}

/// Take variants out of the custom DAT: their files are unknown again.
pub fn remove(ix: &Index, lib: &Library, platform: &str, names: &[String]) -> Result<Value> {
    lib.writable()?;
    let mut d = read(lib, platform)?;
    let before = d.games.len();
    d.games.retain(|g| !names.contains(&g.name));
    if d.games.len() == before {
        bail!("Those aren't in the custom DAT.");
    }
    let id = write(ix, lib, platform, &d, true)?;
    Ok(json!({ "dat": id, "removed": before - d.games.len() }))
}

/// Put back the custom DAT as it was before its last change.
pub fn undo(ix: &Index, lib: &Library, platform: &str) -> Result<Value> {
    lib.writable()?;
    let Some(last) = version_files(lib, platform).pop() else {
        bail!("There's no earlier version of the custom DAT to go back to.");
    };
    let d = dat::parse(&std::fs::read(&last)?)?;
    let id = write(ix, lib, platform, &d, false)?;
    std::fs::remove_file(&last)?;
    Ok(json!({ "dat": id, "games": d.games.len() }))
}

/// A system's variants as the Audit page's *Variants* list shows them.
pub fn list(ix: &Index, lib: &Library, platform: &str) -> Result<Value> {
    let conn = ix.conn();
    let Some(c) = custom_dat(conn, platform)? else {
        return Ok(json!({ "dat": null, "name": dat_name(platform), "items": [], "can_undo": false }));
    };
    let dirs: HashMap<String, PathBuf> = crate::roots::list(lib).into_iter().map(|r| (r.id, r.dir)).collect();
    let mut q = conn.prepare(&format!(
        "WITH {GAMES}
         SELECT gs.id, gs.name, dg.cloneof, gs.status,
                (SELECT f.root_id || char(9) || f.rel_path || char(9) || COALESCE(p.name, '') || char(9) || f.name || char(9) || (f.name = r.name)
                 FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id LEFT JOIN files p ON p.id = f.parent_id
                 WHERE r.game_id = gs.id LIMIT 1)
         FROM gs JOIN dat_games dg ON dg.id = gs.id ORDER BY gs.name LIMIT 5000"
    ))?;
    let items: Vec<Value> = q
        .query_map([c], |r| {
            let name: String = r.get(1)?;
            let file: Option<String> = r.get(4)?;
            let (path, shown, named) = match file {
                Some(s) => {
                    let p: Vec<&str> = s.split('\t').collect();
                    let base = dirs.get(p[0]).cloned().unwrap_or_default();
                    let rel = if p[2].is_empty() { p[1].to_string() } else { p[1].strip_suffix(&format!("/{}", p[3])).unwrap_or(p[1]).to_string() };
                    let at = rel.split('/').fold(base, |a, b| a.join(b));
                    let shown = if p[2].is_empty() { at.display().to_string() } else { format!("{} › {}", at.display(), p[3]) };
                    (Some(at.display().to_string()), Some(shown), p[4] == "1")
                }
                None => (None, None, true),
            };
            Ok(json!({
                "id": r.get::<_, i64>(0)?, "tags": tags_of(&name), "title": split_name(&name).0, "name": name,
                "original": r.get::<_, Option<String>>(2)?, "status": r.get::<_, String>(3)?, "path": path, "shown": shown, "misnamed": !named,
            }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(json!({ "dat": c, "name": dat_name(platform), "items": items, "can_undo": !version_files(lib, platform).is_empty() }))
}

// ------------------------------------------------------------ the original's details (D122)

/// The custom DAT's games by these names that are variants: each takes its original's.
fn take_named(ix: &Index, lib: &Library, dat_id: i64, names: &[String]) -> Result<usize> {
    let mut n = 0;
    let mut q = ix.conn().prepare("SELECT id FROM dat_games WHERE dat_id = ?1 AND name = ?2 AND cloneof IS NOT NULL")?;
    let ids: Vec<i64> = names.iter().filter_map(|name| q.query_row(params![dat_id, name], |r| r.get(0)).optional().transpose()).collect::<rusqlite::Result<_>>()?;
    drop(q);
    for id in ids {
        n += take(ix, lib, id)? as usize;
    }
    Ok(n)
}

/// A variant's original (its DAT game), if it has one in the system's main DAT.
pub fn original_of(conn: &Connection, lib: &Library, variant: i64) -> Result<Option<i64>> {
    let row: Option<(String, Option<String>)> = conn
        .query_row("SELECT d.platform, g.cloneof FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1 AND d.version = ?2", params![variant, VERSION], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((platform, Some(original))) = row else { return Ok(None) };
    let Some(main) = audit::primary_dat(conn, lib, &platform)? else { return Ok(None) };
    Ok(conn.query_row("SELECT id FROM dat_games WHERE dat_id = ?1 AND name = ?2 ORDER BY id LIMIT 1", params![main, original], |r| r.get(0)).optional()?)
}

/// The variants of a main DAT's game (their DAT games' ids and names).
pub fn variants_of(conn: &Connection, platform: &str, original: &str) -> Result<Vec<(i64, String)>> {
    let Some(c) = custom_dat(conn, platform)? else { return Ok(vec![]) };
    let mut q = conn.prepare("SELECT id, name FROM dat_games WHERE dat_id = ?1 AND cloneof = ?2 ORDER BY name")?;
    let v = q.query_map(params![c, original], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// The title a variant shows: `title` with its name's tags after it, unless it has them.
pub fn titled(title: &str, name: &str) -> String {
    let (_, suffix) = split_name(name);
    let t = title.trim();
    if suffix.is_empty() || t.ends_with(suffix) {
        t.to_string()
    } else {
        format!("{t} {suffix}")
    }
}

/// Link a file where it can be (the pictures take no more room), else copy it.
fn link_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    if to.exists() {
        std::fs::remove_file(to)?;
    }
    std::fs::hard_link(from, to).or_else(|_| std::fs::copy(from, to).map(|_| ()))
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else if p.is_file() && !p.to_string_lossy().ends_with(".gamerepo-tmp") {
            out.push(p);
        }
    }
}

/// A variant takes its original's details and pictures (D122): the sources' answers are
/// copied into its own game folder with its tags after their titles, and the pictures
/// linked; what was changed by hand on the variant stays. False when the original has
/// nothing yet (or the game isn't a variant).
pub fn take(ix: &Index, lib: &Library, variant: i64) -> Result<bool> {
    let conn = ix.conn();
    let Some(orig) = original_of(conn, lib, variant)? else { return Ok(false) };
    let o = meta::dat_game(conn, orig)?;
    let Some(of) = meta::folder_of(conn, &o)? else { return Ok(false) };
    let odir = lib.resolve(&of)?;
    let ov = meta::read_game(&odir);
    if !ov.is_object() {
        return Ok(false);
    }
    sources::prepare(&odir, &ov)?;
    let answers = sources::answers(&odir);
    let mut pictures = vec![];
    files_under(&odir.join("media"), &mut pictures);
    if answers.is_empty() && pictures.is_empty() {
        return Ok(false);
    }
    let g = meta::dat_game(conn, variant)?;
    let vdir = meta::ensure_folder(ix, lib, &g)?;
    for (s, mut a) in answers {
        if let Some(t) = a["title"].as_str().filter(|t| !t.is_empty()) {
            a["title"] = json!(titled(t, &g.name));
        }
        let d = vdir.join("sources");
        std::fs::create_dir_all(&d)?;
        crate::config::write_atomic(&d.join(format!("{s}.json")), serde_json::to_string_pretty(&a)?.as_bytes())?;
    }
    for p in &pictures {
        let rel = p.strip_prefix(&odir).unwrap_or(p);
        link_or_copy(p, &vdir.join(rel))?;
    }
    let mut v = meta::read_game(&vdir);
    for k in ["chosen", "pinned", "pinned_media", "identified_by"] {
        if !ov[k].is_null() {
            v[k] = ov[k].clone();
        }
    }
    if !ov["scraped"]["media"].is_null() {
        if !v["scraped"].is_object() {
            v["scraped"] = json!({});
        }
        v["scraped"]["media"] = ov["scraped"]["media"].clone();
    }
    v["variant_of"] = json!(o.name);
    sources::make(&mut v, &vdir, &sources::Rules::of(lib));
    if !edited(&v, "title") {
        let t = v["title"].as_str().unwrap_or(&g.name).to_string();
        v["title"] = json!(titled(&t, &g.name));
    }
    meta::write_game(ix, lib, &vdir, &v)?;
    Ok(true)
}

/// After a scrape of these DAT games: each one's variants take what it now has.
pub fn after_scrape(ix: &Index, lib: &Library, games: &[i64]) -> Result<usize> {
    let mut n = 0;
    for id in games {
        let g = meta::dat_game(ix.conn(), *id)?;
        if is_custom(ix.conn(), dat_of(ix.conn(), *id)?)? {
            continue;
        }
        for (v, _) in variants_of(ix.conn(), &g.platform, &g.name)? {
            n += take(ix, lib, v)? as usize;
        }
    }
    Ok(n)
}

fn dat_of(conn: &Connection, game: i64) -> Result<i64> {
    Ok(conn.query_row("SELECT dat_id FROM dat_games WHERE id = ?1", [game], |r| r.get(0))?)
}

/// What a scrape asks the sources about: a variant's original in its place (it takes the
/// original's after), and the rest as they are.
pub fn scrape_targets(conn: &Connection, lib: &Library, games: Vec<i64>) -> Result<Vec<i64>> {
    let mut out: Vec<i64> = vec![];
    let mut seen: HashSet<i64> = HashSet::new();
    for id in games {
        let target = original_of(conn, lib, id)?.unwrap_or(id);
        if seen.insert(target) {
            out.push(target);
        }
    }
    Ok(out)
}

/// The originals of the variants of a system that you have, for a scrape of its games.
pub fn originals_had(conn: &Connection, lib: &Library, platform: &str) -> Result<Vec<i64>> {
    let Some(c) = custom_dat(conn, platform)? else { return Ok(vec![]) };
    let mut q = conn.prepare(&format!("WITH {GAMES} SELECT id FROM gs WHERE status != 'missing'"))?;
    let had: Vec<i64> = q.query_map([c], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut v = vec![];
    for id in had {
        if let Some(o) = original_of(conn, lib, id)? {
            if !v.contains(&o) {
                v.push(o);
            }
        }
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{fixture, import, names, rom, rom_xml, run_scan, write};

    #[test]
    fn tags_are_the_parentheticals_split_at_commas() {
        assert_eq!(tags_of("Donkey Kong 3 (NA, Animal Crossing GameCube Link)"), ["NA", "Animal Crossing GameCube Link"]);
        assert_eq!(tags_of("Dragon Quest Monsters - Caravan Heart (English Translated)"), ["English Translated"]);
        assert_eq!(tags_of("Pokemon - Fire Red Version (U) (V1.1) [T+Fre] [h1C] [!]"), ["USA", "V1.1", "French Translated", "Hack"]);
        assert_eq!(tags_of("Dark Eden"), Vec::<String>::new());
        let tags = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // the way they were written is kept while they're the same
        let was = "Donkey Kong 3 (NA, Animal Crossing GameCube Link)";
        assert_eq!(compose("Donkey Kong 3", &tags(&["NA", "Animal Crossing GameCube Link"]), was), was);
        assert_eq!(compose("Donkey Kong 3", &tags(&["NA", "Hack"]), was), "Donkey Kong 3 (NA) (Hack)");
        assert_eq!(titled("Dragon Quest Monsters: Caravan Heart", "Dragon Quest Monsters - Caravan Heart (English Translated)"), "Dragon Quest Monsters: Caravan Heart (English Translated)");
    }

    #[test]
    fn names_are_compared_without_their_punctuation_accents_and_numerals() {
        assert_eq!(words("The Legend of Zelda - A Link to the Past"), ["legend", "of", "zelda", "a", "link", "to", "the", "past"]);
        assert_eq!(words("Final Fantasy VI"), ["final", "fantasy", "6"]);
        assert_eq!(words("Mega Man X"), ["mega", "man", "x"]);
        assert_eq!(words("Pokémon Pinball"), ["pokemon", "pinball"]);
        assert_eq!(words("Sonic & Knuckles"), ["sonic", "and", "knuckles"]);
        assert_eq!(roman("iiii"), None);
        assert!(alike(&words("Pokemon - Fire Red Version"), &words("Pokemon - FireRed Version")) >= 1.0);
        assert!(alike(&words("Super Mario Bros. 3 - Mario Adventure"), &words("Super Mario Bros. 3")) >= 0.8);
        assert!(alike(&words("Dark Eden"), &words("Tetris")) < CLOSE);
    }

    #[test]
    fn an_unknown_file_gets_its_original_suggested_and_goes_in_the_custom_dat_as_its_variant() {
        let f = fixture("variants-add");
        // two releases of one title: the one the file is the size of comes first
        let dqm = rom(40, 65536);
        let gba = f.dir.join("Roms/GBA");
        write(&gba.join("Dragon Quest Monsters - Caravan Heart (English Translated).gba"), &rom(41, 65536));
        write(&gba.join("Dark Eden (Prototype).gba"), &rom(42, 4096));
        let dat = crate::audit::tests::dat_xml("Nintendo - Game Boy Advance", "20261001", &[
            ("Dragon Quest Monsters - Caravan Heart (Japan)", None, rom_xml("Dragon Quest Monsters - Caravan Heart (Japan).gba", &dqm)),
            ("Dragon Quest Monsters - Caravan Heart (Japan) (Rev 1)", Some("Dragon Quest Monsters - Caravan Heart (Japan)"), rom_xml("x.gba", &rom(43, 131072))),
            ("Tetris Worlds (USA)", None, rom_xml("Tetris Worlds (USA).gba", &rom(44, 1024))),
        ]);
        write(&f.dir.join("dats/gba.dat"), dat.as_bytes());
        run_scan(&f, false);
        let main = import(&f, "gba.dat")[0]["id"].as_i64().unwrap();
        assert_eq!(names(&f, main, "unknown"), ["Dark Eden (Prototype).gba", "Dragon Quest Monsters - Caravan Heart (English Translated).gba"]);
        let l = audit::list(&f.ix, &f.lib, main, "unknown", "", 0, 100).unwrap();
        let dq = l["items"].as_array().unwrap().iter().find(|i| i["name"].as_str().unwrap().starts_with("Dragon")).unwrap().clone();
        assert_eq!(dq["suggested"][0]["name"], "Dragon Quest Monsters - Caravan Heart (Japan)", "{dq}");
        assert_eq!(dq["suggested"][0]["how"], "same");
        assert_eq!(dq["tags"], json!(["English Translated"]));
        let eden = l["items"].as_array().unwrap().iter().find(|i| i["name"].as_str().unwrap().starts_with("Dark")).unwrap().clone();
        assert_eq!(eden["suggested"], json!([]), "{eden}");

        // the original has details and a picture: the variant takes them, keeping its name
        let o = meta::dat_game(f.ix.conn(), dq["suggested"][0]["id"].as_i64().unwrap()).unwrap();
        let odir = meta::ensure_folder(&f.ix, &f.lib, &o).unwrap();
        std::fs::create_dir_all(odir.join("sources")).unwrap();
        std::fs::write(odir.join("sources/igdb.json"), json!({ "source": "igdb", "id": "7", "title": "Dragon Quest Monsters: Caravan Heart", "details": { "developer": "Tose" } }).to_string()).unwrap();
        std::fs::create_dir_all(odir.join("media")).unwrap();
        std::fs::write(odir.join("media/cover.png"), b"png").unwrap();
        sources::remake(&f.ix, &f.lib, &odir, &sources::Rules::of(&f.lib)).unwrap();

        let name = "Dragon Quest Monsters - Caravan Heart (English Translated)";
        let r = add(&f.ix, &f.lib, "gba", &[
            Add { files: vec![dq["id"].as_i64().unwrap()], name: name.into(), original: Some("Dragon Quest Monsters - Caravan Heart (Japan)".into()) },
            Add { files: vec![eden["id"].as_i64().unwrap()], name: "Dark Eden (Prototype)".into(), original: None },
        ]).unwrap();
        assert_eq!(r["taken"], 1, "{r}");
        assert_eq!(names(&f, main, "unknown"), Vec::<String>::new(), "added files aren't unknown");
        let c = custom_dat(f.ix.conn(), "gba").unwrap().unwrap();
        assert_eq!(names(&f, c, "have"), ["Dark Eden (Prototype)", name]);
        assert_eq!(audit::primary_dat(f.ix.conn(), &f.lib, "gba").unwrap(), Some(main), "the custom DAT is never the main one");
        let text = std::fs::read_to_string(f.lib.resolve(&rel("gba")).unwrap()).unwrap();
        assert!(text.contains("cloneof=\"Dragon Quest Monsters - Caravan Heart (Japan)\"") && text.contains(&format!("<rom name=\"{name}.gba\"")), "{text}");
        let id = list(&f.ix, &f.lib, "gba").unwrap()["items"].as_array().unwrap().iter().find(|i| i["name"] == name).unwrap()["id"].as_i64().unwrap();
        let vdir = f.lib.resolve(&meta::folder_of(f.ix.conn(), &meta::dat_game(f.ix.conn(), id).unwrap()).unwrap().unwrap()).unwrap();
        let v = meta::read_game(&vdir);
        assert_eq!(v["title"], "Dragon Quest Monsters: Caravan Heart (English Translated)", "{v}");
        assert_eq!(v["details"]["developer"], "Tose");
        assert_eq!(std::fs::read(vdir.join("media/cover.png")).unwrap(), b"png");

        // the Library shows it beside its original, under its own name, with its tag
        crate::browse::refresh(&f.ix, &f.lib).unwrap();
        let (title, tags): (String, String) = f.ix.conn().query_row("SELECT title, tags FROM library WHERE key = ?1", [format!("gba/{name}")], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((title.as_str(), tags.as_str()), ("Dragon Quest Monsters: Caravan Heart (English Translated)", "English Translated"));

        // renamed with another tag; removed, it's unknown again; Undo brings it back
        edit(&f.ix, &f.lib, "gba", "Dark Eden (Prototype)", "Dark Eden (Prototype) (Unreleased)", None).unwrap();
        assert_eq!(names(&f, custom_dat(f.ix.conn(), "gba").unwrap().unwrap(), "misnamed"), ["Dark Eden (Prototype).gba"]);
        remove(&f.ix, &f.lib, "gba", &["Dark Eden (Prototype) (Unreleased)".into()]).unwrap();
        assert_eq!(names(&f, main, "unknown"), ["Dark Eden (Prototype).gba"]);
        undo(&f.ix, &f.lib, "gba").unwrap();
        assert_eq!(names(&f, main, "unknown"), Vec::<String>::new());
        assert!(tag_list(f.ix.conn(), &f.lib).unwrap().iter().any(|t| t == "Unreleased"), "tags used join the list");
        assert!(add(&f.ix, &f.lib, "gba", &[Add { files: vec![eden["id"].as_i64().unwrap()], name: "Tetris Worlds (USA)".into(), original: None }]).is_err(), "a main DAT game's name is taken");
    }

    #[test]
    fn a_custom_dat_changed_in_its_place_is_loaded_again() {
        let f = fixture("variants-sync");
        run_scan(&f, false);
        import(&f, "gb.dat");
        let hack = f.ix.conn().query_row("SELECT id FROM files WHERE name = 'My Hack.gb'", [], |r| r.get::<_, i64>(0)).unwrap();
        add(&f.ix, &f.lib, "gb", &[Add { files: vec![hack], name: "Alpha (USA) (Hack)".into(), original: Some("Alpha (USA)".into()) }]).unwrap();
        // another computer's change to the file (here, by hand)
        let p = f.lib.resolve(&rel("gb")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&p, std::fs::read_to_string(&p).unwrap().replace("Alpha (USA) (Hack)", "Alpha (USA) (Mod)")).unwrap();
        dat::sync_library(&f.ix, &f.lib).unwrap();
        let c = custom_dat(f.ix.conn(), "gb").unwrap().unwrap();
        assert_eq!(names(&f, c, "have"), ["Alpha (USA) (Mod)"]);
    }
}
