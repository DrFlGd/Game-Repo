//! Collections (docs/PLAN.md, "Phase 3 design", D21): saved queries over the games of
//! one or more systems, kept in the library as `_library/collections/<id>.json` and
//! worked out from the index when they're shown or used: rules, one game per title
//! (1G1R), other DATs as sources, and games added or left out by hand. A collection
//! can be saved as DATs, one per system (D23).

use crate::audit::{self, xml, GAMES};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::platforms;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The flags a collection leaves out unless told otherwise.
pub const DEFAULT_EXCLUDE: &[&str] = &["beta", "proto", "demo", "sample", "pirate", "bios", "program", "kiosk", "debug"];
/// The flags the rules can leave out (dat.rs's names for them).
pub const FLAGS: &[&str] = &[
    "beta", "proto", "demo", "sample", "unl", "pirate", "aftermarket", "bios", "program", "kiosk", "debug",
    "virtual console", "homebrew", "hack", "alt",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Rules {
    /// The systems it covers; none: every system with a DAT
    pub platforms: Vec<String>,
    /// have (the games you have) | all (every game in the DAT)
    pub include: String,
    /// One game per parent/clone group
    pub one_per_title: bool,
    /// Region order for 1G1R (and, with `only_regions`, the regions it keeps)
    pub regions: Vec<String>,
    pub only_regions: bool,
    /// Keep games listing one of these languages (games listing none are kept)
    pub languages: Vec<String>,
    /// Flags whose games are left out
    pub exclude: Vec<String>,
    /// A name search
    pub search: String,
    /// Only games in these DATs (by DAT name), by name or by a ROM's hashes
    pub from_dats: Vec<String>,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            platforms: vec![],
            include: "have".into(),
            one_per_title: true,
            regions: ["USA", "World", "Europe", "Japan"].map(String::from).to_vec(),
            only_regions: false,
            languages: vec![],
            exclude: DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect(),
            search: String::new(),
            from_dats: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Collection {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub rules: Rules,
    /// Games added and left out by hand, as `system/game name`
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub modified: String,
}

// ------------------------------------------------------------ kept in the library

fn dir(lib: &Library) -> PathBuf {
    lib.root().join(APP_DIR).join("collections")
}

fn path(lib: &Library, id: &str) -> Result<PathBuf> {
    crate::library::valid_id(id)?;
    Ok(dir(lib).join(format!("{id}.json")))
}

pub fn load(lib: &Library, id: &str) -> Result<Collection> {
    let p = path(lib, id)?;
    let text = std::fs::read(&p).map_err(|_| anyhow!("There's no collection {id}."))?;
    serde_json::from_slice(&text).with_context(|| format!("{} can't be read", p.display()))
}

/// Every collection, by name.
pub fn list(lib: &Library) -> Vec<Collection> {
    let mut v: Vec<Collection> = std::fs::read_dir(dir(lib))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    v.sort_by(|a: &Collection, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    v
}

/// Save a collection (a new one when it has no id), keeping keys a newer app added.
pub fn save(lib: &Library, mut c: Collection) -> Result<Collection> {
    lib.writable()?;
    c.name = c.name.trim().to_string();
    if c.name.is_empty() {
        bail!("Give the collection a name.");
    }
    if !["have", "all"].contains(&c.rules.include.as_str()) {
        bail!("A collection has the games you have or every game in the DAT.");
    }
    if list(lib).iter().any(|o| o.id != c.id && o.name.eq_ignore_ascii_case(&c.name)) {
        bail!("There's a collection called {} already.", c.name);
    }
    if c.id.is_empty() {
        c.id = new_id(&c.name);
        c.created = now();
    }
    c.modified = now();
    let p = path(lib, &c.id)?;
    let mut v = std::fs::read(&p).ok().and_then(|t| serde_json::from_slice::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    for (k, x) in serde_json::to_value(&c)?.as_object().unwrap() {
        v[k] = x.clone();
    }
    std::fs::create_dir_all(dir(lib))?;
    crate::config::write_json(&p, &v)?;
    Ok(c)
}

pub fn delete(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    std::fs::remove_file(path(lib, id)?).map_err(|_| anyhow!("There's no collection {id}."))?;
    Ok(())
}

/// An id from the name and the time: "best-of-snes-1f3a".
fn new_id(name: &str) -> String {
    let mut slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug: String = slug.trim_matches('-').chars().take(40).collect();
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}{:x}", if slug.is_empty() { String::new() } else { format!("{slug}-") }, (n as u64) & 0xffff_ffff)
}

// ------------------------------------------------------------ working it out

/// A game of a collection.
#[derive(Clone, Debug, Serialize)]
pub struct Game {
    pub id: i64,
    pub platform: String,
    pub name: String,
    pub title: String,
    pub regions: String,
    pub status: String,
    /// Added by hand (not by the rules)
    pub manual: bool,
}

impl Game {
    pub fn key(&self) -> String {
        format!("{}/{}", self.platform, self.name)
    }
}

/// A collection's games on one system.
#[derive(Clone, Debug)]
pub struct System {
    pub platform: String,
    pub dat_id: i64,
    pub games: Vec<Game>,
    /// Left out by hand (they'd be in otherwise)
    pub removed: Vec<Game>,
}

#[derive(Debug, Default)]
pub struct Worked {
    pub systems: Vec<System>,
    /// Systems asked for that have no DAT
    pub no_dat: Vec<String>,
    /// DATs named as sources that aren't in the library
    pub missing_dats: Vec<String>,
}

impl Worked {
    pub fn games(&self) -> impl Iterator<Item = &Game> {
        self.systems.iter().flat_map(|s| s.games.iter())
    }

    pub fn summary(&self) -> Value {
        json!({
            "games": self.systems.iter().map(|s| s.games.len()).sum::<usize>(),
            "have": self.games().filter(|g| g.status == "have").count(),
            "removed": self.systems.iter().map(|s| s.removed.len()).sum::<usize>(),
            "systems": self.systems.iter().map(|s| json!({
                "platform": s.platform, "name": platforms::name_of(&s.platform), "dat": s.dat_id, "games": s.games.len(),
                "have": s.games.iter().filter(|g| g.status == "have").count(), "removed": s.removed.len(),
            })).collect::<Vec<_>>(),
            "no_dat": self.no_dat.iter().map(|p| json!({ "platform": p, "name": platforms::name_of(p) })).collect::<Vec<_>>(),
            "missing_dats": self.missing_dats,
        })
    }
}

/// The last collection worked out, by what it depended on (the index changes rarely
/// between the pages of a list).
static LAST: Mutex<Option<(String, Arc<Worked>)>> = Mutex::new(None);

struct Row {
    id: i64,
    name: String,
    title: String,
    regions: String,
    languages: String,
    revision: String,
    version: String,
    flags: String,
    parent: Option<i64>,
    status: String,
}

fn split(s: &str) -> impl Iterator<Item = &str> {
    s.split(',').map(str::trim).filter(|x| !x.is_empty())
}

/// The systems a collection covers: those ticked, else every system with a DAT.
fn systems_of(conn: &Connection, c: &Collection) -> Result<Vec<String>> {
    if !c.rules.platforms.is_empty() {
        return Ok(c.rules.platforms.clone());
    }
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 ORDER BY platform")?;
    let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// What the collection holds now.
pub fn work_out(ix: &Index, lib: &Library, c: &Collection) -> Result<Arc<Worked>> {
    let conn = ix.conn();
    let systems = systems_of(conn, c)?;
    let mut dats = vec![];
    for p in &systems {
        dats.push((p.clone(), audit::primary_dat(conn, lib, p)?));
    }
    let key = format!("{}|{}|{}|{dats:?}", ix.path().display(), serde_json::to_string(c)?, audit::generation(conn)?);
    if let Some((k, w)) = LAST.lock().unwrap().as_ref() {
        if *k == key {
            return Ok(w.clone());
        }
    }
    let w = Arc::new(work_out_fresh(conn, c, &dats)?);
    *LAST.lock().unwrap() = Some((key, w.clone()));
    Ok(w)
}

/// The hashes of the ROMs, and the names of the games, of the DATs a collection takes its games from.
struct Sources {
    names: HashSet<String>,
    sha1: HashSet<String>,
    crc: HashSet<(String, i64)>,
}

fn sources(conn: &Connection, names: &[String], missing: &mut Vec<String>) -> Result<Option<Sources>> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut s = Sources { names: HashSet::new(), sha1: HashSet::new(), crc: HashSet::new() };
    for n in names {
        let ids: Vec<i64> = conn
            .prepare("SELECT id FROM dats WHERE name = ?1 AND current = 1")?
            .query_map([n], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if ids.is_empty() {
            missing.push(n.clone());
        }
        for id in ids {
            let mut q = conn.prepare("SELECT name FROM dat_games WHERE dat_id = ?1")?;
            for g in q.query_map([id], |r| r.get::<_, String>(0))? {
                s.names.insert(g?);
            }
            let mut q = conn.prepare("SELECT sha1, crc32, size FROM dat_roms WHERE dat_id = ?1")?;
            let mut rows = q.query([id])?;
            while let Some(r) = rows.next()? {
                if let Some(h) = r.get::<_, Option<String>>(0)? {
                    s.sha1.insert(h);
                }
                if let (Some(c), Some(z)) = (r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?) {
                    s.crc.insert((c, z));
                }
            }
        }
    }
    Ok(Some(s))
}

/// "1", "A" or "1.1" as something that sorts: higher is newer.
fn revision_key(s: &str) -> Vec<u64> {
    s.split(['.', ' '])
        .map(|p| p.parse::<u64>().unwrap_or_else(|_| p.chars().next().map(|c| c.to_ascii_uppercase() as u64).unwrap_or(0)))
        .collect()
}

fn work_out_fresh(conn: &Connection, c: &Collection, dats: &[(String, Option<i64>)]) -> Result<Worked> {
    let r = &c.rules;
    let mut w = Worked::default();
    let src = sources(conn, &r.from_dats, &mut w.missing_dats)?;
    let exclude: HashSet<&str> = r.exclude.iter().map(String::as_str).collect();
    let langs: HashSet<String> = r.languages.iter().map(|l| l.to_ascii_lowercase()).collect();
    let search = r.search.trim().to_lowercase();
    let added: HashSet<&str> = c.added.iter().map(String::as_str).collect();
    let removed: HashSet<&str> = c.removed.iter().map(String::as_str).collect();
    let rank = |regions: &str| -> usize {
        split(regions).filter_map(|g| r.regions.iter().position(|x| x.eq_ignore_ascii_case(g))).min().unwrap_or(r.regions.len())
    };
    for (platform, dat) in dats {
        let Some(dat_id) = *dat else {
            w.no_dat.push(platform.clone());
            continue;
        };
        let mut q = conn.prepare(&format!(
            "WITH {GAMES}
             SELECT gs.id, gs.name, COALESCE(gs.title, ''), COALESCE(gs.regions, ''), COALESCE(g.languages, ''), COALESCE(g.revision, ''),
                    COALESCE(g.version, ''), COALESCE(gs.flags, ''), gs.parent_id, gs.status
             FROM gs JOIN dat_games g ON g.id = gs.id ORDER BY gs.name"
        ))?;
        let rows: Vec<Row> = q
            .query_map([dat_id], |x| {
                Ok(Row {
                    id: x.get(0)?, name: x.get(1)?, title: x.get(2)?, regions: x.get(3)?, languages: x.get(4)?, revision: x.get(5)?,
                    version: x.get(6)?, flags: x.get(7)?, parent: x.get(8)?, status: x.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        // a source DAT's games by their ROMs' hashes
        let by_hash: HashSet<i64> = match &src {
            Some(s) => {
                let mut ids = HashSet::new();
                let mut q = conn.prepare("SELECT game_id, sha1, crc32, size FROM dat_roms WHERE dat_id = ?1")?;
                let mut rows = q.query([dat_id])?;
                while let Some(x) = rows.next()? {
                    let sha1: Option<String> = x.get(1)?;
                    let crc: Option<(String, i64)> = match (x.get::<_, Option<String>>(2)?, x.get::<_, Option<i64>>(3)?) {
                        (Some(c), Some(z)) => Some((c, z)),
                        _ => None,
                    };
                    if sha1.is_some_and(|h| s.sha1.contains(&h)) || crc.is_some_and(|k| s.crc.contains(&k)) {
                        ids.insert(x.get(0)?);
                    }
                }
                ids
            }
            None => HashSet::new(),
        };
        let passes = |g: &Row| -> bool {
            (r.include == "all" || g.status == "have")
                && !split(&g.flags).any(|f| exclude.contains(f))
                && (langs.is_empty() || g.languages.is_empty() || split(&g.languages).any(|l| langs.contains(&l.to_ascii_lowercase())))
                && (search.is_empty() || g.name.to_lowercase().contains(&search))
                && (!r.only_regions || rank(&g.regions) < r.regions.len())
                && src.as_ref().is_none_or(|s| s.names.contains(&g.name) || by_hash.contains(&g.id))
        };
        let mut picked: Vec<&Row> = rows.iter().filter(|g| passes(g)).collect();
        if r.one_per_title {
            let mut best: HashMap<i64, &Row> = HashMap::new();
            for g in picked {
                let group = g.parent.unwrap_or(g.id);
                let better = |a: &Row, b: &Row| {
                    // earlier region, no flags, newer revision and version, the parent, then by name
                    let k = |x: &Row| (rank(&x.regions), !x.flags.is_empty(), std::cmp::Reverse(revision_key(&x.revision)), std::cmp::Reverse(revision_key(&x.version)), x.parent.is_some());
                    (k(a), &a.name) < (k(b), &b.name)
                };
                match best.get(&group) {
                    Some(b) if !better(g, b) => {}
                    _ => {
                        best.insert(group, g);
                    }
                }
            }
            picked = best.into_values().collect();
            picked.sort_by(|a, b| a.name.cmp(&b.name));
        }
        let game = |g: &Row, manual: bool| Game {
            id: g.id, platform: platform.clone(), name: g.name.clone(), title: g.title.clone(), regions: g.regions.clone(), status: g.status.clone(), manual,
        };
        let mut sys = System { platform: platform.clone(), dat_id, games: vec![], removed: vec![] };
        let mut inside: HashSet<i64> = HashSet::new();
        for g in picked {
            let x = game(g, false);
            if removed.contains(x.key().as_str()) {
                sys.removed.push(x);
            } else {
                inside.insert(g.id);
                sys.games.push(x);
            }
        }
        for g in rows.iter().filter(|g| !inside.contains(&g.id) && added.contains(format!("{platform}/{}", g.name).as_str())) {
            sys.games.push(game(g, true));
        }
        sys.games.sort_by(|a, b| a.name.cmp(&b.name));
        w.systems.push(sys);
    }
    Ok(w)
}

/// A page of a collection's games (or of those left out by hand), with a search.
pub fn page(w: &Worked, platform: Option<&str>, removed: bool, search: &str, offset: usize, limit: usize) -> Value {
    let s = search.trim().to_lowercase();
    let items: Vec<&Game> = w
        .systems
        .iter()
        .filter(|x| platform.is_none_or(|p| p == x.platform))
        .flat_map(|x| if removed { x.removed.iter() } else { x.games.iter() })
        .filter(|g| s.is_empty() || g.name.to_lowercase().contains(&s))
        .collect();
    json!({
        "total": items.len(),
        "items": items.iter().skip(offset).take(limit.clamp(1, 1000)).map(|g| {
            json!({ "key": g.key(), "platform": g.platform, "platform_name": platforms::name_of(&g.platform), "name": g.name, "title": g.title,
                    "regions": g.regions, "status": g.status, "manual": g.manual })
        }).collect::<Vec<_>>(),
    })
}

/// Games of a system's DAT that aren't in the collection, to add by hand.
pub fn candidates(ix: &Index, lib: &Library, c: &Collection, platform: &str, search: &str, limit: usize) -> Result<Value> {
    let conn = ix.conn();
    let Some(dat) = audit::primary_dat(conn, lib, platform)? else { bail!("{} has no DAT.", platforms::name_of(platform)) };
    let w = work_out(ix, lib, c)?;
    let inside: HashSet<String> = w.games().map(Game::key).collect();
    let mut q = conn.prepare(&format!(
        "WITH {GAMES} SELECT name, COALESCE(regions, ''), status FROM gs WHERE name LIKE ?2 ESCAPE '\\' ORDER BY status = 'have' DESC, name"
    ))?;
    let like = format!("%{}%", search.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let mut out = vec![];
    let mut rows = q.query(params![dat, like])?;
    while let Some(r) = rows.next()? {
        let name: String = r.get(0)?;
        if inside.contains(&format!("{platform}/{name}")) {
            continue;
        }
        out.push(json!({ "key": format!("{platform}/{name}"), "name": name, "regions": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)? }));
        if out.len() >= limit {
            break;
        }
    }
    Ok(json!(out))
}

// ------------------------------------------------------------ saved as DATs (D23)

/// A system's games of a collection as a Logiqx DAT: each game and its ROMs as its own DAT lists them.
pub fn dat_xml(conn: &Connection, name: &str, description: &str, games: &[&Game]) -> Result<String> {
    let date = &now()[..10];
    let mut out = format!(
        "<?xml version=\"1.0\"?>\n<datafile>\n\t<header>\n\t\t<name>{}</name>\n\t\t<description>{}</description>\n\t\t<version>{}</version>\n\t\t<author>Game Repo</author>\n\t</header>\n",
        xml(name), xml(description), xml(date)
    );
    let inside: HashSet<i64> = games.iter().map(|g| g.id).collect();
    let mut gq = conn.prepare("SELECT g.description, p.id, p.name FROM dat_games g LEFT JOIN dat_games p ON p.id = g.parent_id WHERE g.id = ?1")?;
    let mut rq = conn.prepare("SELECT name, size, crc32, md5, sha1, status FROM dat_roms WHERE game_id = ?1 ORDER BY id")?;
    for g in games {
        let (desc, pid, pname): (Option<String>, Option<i64>, Option<String>) = gq.query_row([g.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        let clone = match (pid, pname) {
            (Some(p), Some(n)) if inside.contains(&p) => format!(" cloneof=\"{}\"", xml(&n)),
            _ => String::new(),
        };
        out.push_str(&format!("\t<game name=\"{}\"{clone}>\n\t\t<description>{}</description>\n", xml(&g.name), xml(desc.as_deref().unwrap_or(&g.name))));
        let mut rows = rq.query([g.id])?;
        while let Some(r) = rows.next()? {
            let mut rom = format!("\t\t<rom name=\"{}\"", xml(&r.get::<_, String>(0)?));
            if let Some(s) = r.get::<_, Option<i64>>(1)? {
                rom.push_str(&format!(" size=\"{s}\""));
            }
            for (k, i) in [("crc", 2), ("md5", 3), ("sha1", 4), ("status", 5)] {
                if let Some(v) = r.get::<_, Option<String>>(i)? {
                    rom.push_str(&format!(" {k}=\"{}\"", xml(&v)));
                }
            }
            out.push_str(&rom);
            out.push_str("/>\n");
        }
        out.push_str("\t</game>\n");
    }
    out.push_str("</datafile>\n");
    Ok(out)
}

/// File names without the characters Windows refuses.
fn safe(s: &str) -> String {
    let s: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    s.trim().trim_end_matches('.').chars().take(150).collect()
}

/// Save the collection as DATs, one per system, into `to` (by default
/// `_library/collection-dats/<name>/`). Returns where and what was written.
pub fn save_dats(ix: &Index, lib: &Library, c: &Collection, to: Option<&Path>) -> Result<Value> {
    let w = work_out(ix, lib, c)?;
    let dir = match to {
        Some(p) => p.to_path_buf(),
        None => {
            lib.writable()?;
            lib.root().join(APP_DIR).join("collection-dats").join(safe(&c.name))
        }
    };
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    let mut files = vec![];
    for s in w.systems.iter().filter(|s| !s.games.is_empty()) {
        let sys = platforms::name_of(&s.platform);
        let name = format!("{} - {sys}", c.name);
        let games: Vec<&Game> = s.games.iter().collect();
        let text = dat_xml(ix.conn(), &name, &format!("{} ({} games of {sys}, made by Game Repo)", c.name, games.len()), &games)?;
        let p = dir.join(format!("{}.dat", safe(&name)));
        std::fs::write(&p, text).with_context(|| format!("couldn't write {}", p.display()))?;
        files.push(json!({ "platform": s.platform, "path": p.display().to_string(), "rel": lib.relative(&p), "games": games.len() }));
    }
    if files.is_empty() {
        bail!("The collection has no games yet.");
    }
    Ok(json!({ "dir": dir.display().to_string(), "files": files }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::audit::tests::{dat_xml as fixture_dat, rom, rom_xml};
    use crate::library::tests::temp_dir;

    /// A library with a Game Boy DAT of parent/clone groups, some of its games on disk, and a second DAT of "favourites".
    pub(crate) fn fixture(name: &str) -> (PathBuf, Library, Index) {
        let dir = temp_dir(name);
        let lib = Library::open(dir.join("lib")).unwrap();
        let roms = dir.join("Roms");
        std::fs::create_dir_all(&roms).unwrap();
        let r = |n: u32| rom(n, 1024);
        let games: Vec<(&str, Option<&str>, String)> = vec![
            ("Alpha (USA)", None, rom_xml("Alpha (USA).gb", &r(1))),
            ("Alpha (Europe)", Some("Alpha (USA)"), rom_xml("Alpha (Europe).gb", &r(2))),
            ("Alpha (Japan)", Some("Alpha (USA)"), rom_xml("Alpha (Japan).gb", &r(3))),
            ("Beta (Japan)", None, rom_xml("Beta (Japan).gb", &r(4))),
            ("Beta (Europe) (Rev 1)", Some("Beta (Japan)"), rom_xml("Beta (Europe) (Rev 1).gb", &r(5))),
            ("Beta (Europe)", Some("Beta (Japan)"), rom_xml("Beta (Europe).gb", &r(6))),
            ("Gamma (USA) (Beta)", None, rom_xml("Gamma (USA) (Beta).gb", &r(7))),
            ("Delta (World)", None, rom_xml("Delta (World).gb", &r(8))),
            ("Epsilon (Japan) (En,Ja)", None, rom_xml("Epsilon (Japan) (En,Ja).gb", &r(9))),
            ("Zeta (Japan)", None, rom_xml("Zeta (Japan).gb", &r(10))),
        ];
        std::fs::write(dir.join("gb.dat"), fixture_dat("Nintendo - Game Boy", "20261001", &games)).unwrap();
        // on disk: all but Alpha (USA) and Zeta
        for (n, seed) in [("Alpha (Europe).gb", 2), ("Alpha (Japan).gb", 3), ("Beta (Japan).gb", 4), ("Beta (Europe) (Rev 1).gb", 5), ("Beta (Europe).gb", 6),
                          ("Gamma (USA) (Beta).gb", 7), ("Delta (World).gb", 8), ("Epsilon (Japan) (En,Ja).gb", 9)] {
            std::fs::write(roms.join(n), r(seed)).unwrap();
        }
        // favourites: Zeta by name, Delta by its ROM under another name
        let fav = vec![
            ("Zeta (Japan)", None, rom_xml("Zeta (Japan).gb", &r(10))),
            ("My Delta", None, rom_xml("delta.gb", &r(8))),
        ];
        std::fs::write(dir.join("fav.dat"), fixture_dat("Favourites", "1", &fav)).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        crate::roots::add(&lib, &roms, Some("gb")).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options::default(), &std::sync::atomic::AtomicBool::new(false), &mut |_| {}).unwrap();
        let none = serde_json::Map::new();
        crate::dat::import(&ix, &lib, &dir.join("gb.dat"), &none).unwrap();
        let answers: serde_json::Map<String, Value> = [("Favourites".to_string(), json!("gb"))].into_iter().collect();
        crate::dat::import(&ix, &lib, &dir.join("fav.dat"), &answers).unwrap();
        // the Game Boy DAT is the one audited
        let gb: i64 = ix.conn().query_row("SELECT id FROM dats WHERE name = 'Nintendo - Game Boy'", [], |r| r.get(0)).unwrap();
        audit::choose_dat(ix.conn(), &lib, gb).unwrap();
        (dir, lib, ix)
    }

    fn names(w: &Worked) -> Vec<String> {
        w.games().map(|g| g.name.clone()).collect()
    }

    fn with(f: impl FnOnce(&mut Rules)) -> Collection {
        let mut c = Collection { id: String::new(), name: "Test".into(), rules: Rules::default(), added: vec![], removed: vec![], created: String::new(), modified: String::new() };
        c.rules.platforms = vec!["gb".into()];
        f(&mut c.rules);
        c
    }

    #[test]
    fn rules_and_1g1r_pick_the_games() {
        let (dir, lib, ix) = fixture("collection-rules");
        // the defaults: games you have, 1G1R with USA, World, Europe, Japan, no betas
        let w = work_out(&ix, &lib, &with(|_| {})).unwrap();
        assert_eq!(names(&w), ["Alpha (Europe)", "Beta (Europe) (Rev 1)", "Delta (World)", "Epsilon (Japan) (En,Ja)"]);
        // every game in the DAT: Alpha (USA) and Zeta too
        let w = work_out(&ix, &lib, &with(|r| r.include = "all".into())).unwrap();
        assert_eq!(names(&w), ["Alpha (USA)", "Beta (Europe) (Rev 1)", "Delta (World)", "Epsilon (Japan) (En,Ja)", "Zeta (Japan)"]);
        // Japan first, betas kept, no 1G1R
        let w = work_out(&ix, &lib, &with(|r| { r.regions = vec!["Japan".into()]; r.exclude.clear(); r.one_per_title = false; })).unwrap();
        assert_eq!(w.games().count(), 8);
        let w = work_out(&ix, &lib, &with(|r| { r.regions = vec!["Japan".into()]; r.only_regions = true; })).unwrap();
        assert_eq!(names(&w), ["Alpha (Japan)", "Beta (Japan)", "Epsilon (Japan) (En,Ja)"]);
        // languages: a game listing others is left out, one listing none kept
        let w = work_out(&ix, &lib, &with(|r| { r.languages = vec!["Fr".into()]; })).unwrap();
        assert!(!names(&w).contains(&"Epsilon (Japan) (En,Ja)".to_string()) && names(&w).contains(&"Delta (World)".to_string()));
        // another DAT as the source: Zeta by name, Delta by its ROM's hashes
        let w = work_out(&ix, &lib, &with(|r| { r.include = "all".into(); r.from_dats = vec!["Favourites".into()]; })).unwrap();
        assert_eq!(names(&w), ["Delta (World)", "Zeta (Japan)"]);
        // by hand: one left out, one added
        let mut c = with(|_| {});
        c.removed = vec!["gb/Delta (World)".into()];
        c.added = vec!["gb/Gamma (USA) (Beta)".into()];
        let w = work_out(&ix, &lib, &c).unwrap();
        assert_eq!(names(&w), ["Alpha (Europe)", "Beta (Europe) (Rev 1)", "Epsilon (Japan) (En,Ja)", "Gamma (USA) (Beta)"]);
        assert_eq!(w.systems[0].removed.len(), 1);
        assert!(w.games().find(|g| g.name.starts_with("Gamma")).unwrap().manual);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_collection_is_saved_in_the_library_and_as_dats() {
        let (dir, lib, ix) = fixture("collection-save");
        let c = save(&lib, with(|_| {})).unwrap();
        assert!(c.id.starts_with("test-"));
        assert_eq!(load(&lib, &c.id).unwrap().rules, c.rules);
        assert!(save(&lib, Collection { id: String::new(), ..c.clone() }).is_err(), "the same name twice");
        let r = save_dats(&ix, &lib, &c, Some(&dir.join("out"))).unwrap();
        assert_eq!(r["files"].as_array().unwrap().len(), 1);
        let p = PathBuf::from(r["files"][0]["path"].as_str().unwrap());
        assert_eq!(p.file_name().unwrap(), "Test - Game Boy.dat");
        // read again, it's the same games with the same ROMs
        let d = crate::dat::parse(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(d.games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Alpha (Europe)", "Beta (Europe) (Rev 1)", "Delta (World)", "Epsilon (Japan) (En,Ja)"]);
        assert_eq!(d.games[2].roms[0].sha1.as_deref(), Some(crate::hash::hash_reader(&mut &rom(8, 1024)[..], 1024, "", &std::sync::atomic::AtomicBool::new(false)).unwrap().sha1.as_deref().unwrap()));
        assert!(d.games.iter().all(|g| g.cloneof.is_none()), "no clone links to games left out");
        // a collection of every game keeps the clone links between them
        let all = with(|r| { r.include = "all".into(); r.one_per_title = false; r.exclude.clear(); });
        let r = save_dats(&ix, &lib, &all, Some(&dir.join("all"))).unwrap();
        let d = crate::dat::parse(&std::fs::read(r["files"][0]["path"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(d.games.iter().find(|g| g.name == "Alpha (Japan)").unwrap().cloneof.as_deref(), Some("Alpha (USA)"));
        delete(&lib, &c.id).unwrap();
        assert!(list(&lib).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
