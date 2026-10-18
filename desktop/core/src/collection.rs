//! Collections (docs/PLAN.md, "Phase 3 design", D21): saved queries over the games of
//! one or more systems, kept in the library as `_library/collections/<id>.json` and
//! worked out from the index when they're shown or used: rules, one game per title
//! (1G1R), other DATs as sources, and games added or left out by hand. A collection
//! can be saved as DATs, one per system (D23). It holds groups (D75), each with its own
//! way of choosing games: its own fields are the first, `groups` the others, and a game
//! an earlier group has isn't repeated in a later one.

use crate::audit::{self, xml, GAMES};
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::platforms;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
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
    /// The systems it covers; none: every system with a DAT (or, with `dats`, theirs)
    pub platforms: Vec<String>,
    /// have (the games you have) | all (every game in the DAT) | none (only the games added by hand)
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
    /// Only games in these DAT files of its own (library paths; a collection made from
    /// folders or a DAT keeps them in `_library/collections/<id>/`, D49), the same way
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub from_files: Vec<String>,
    /// Its games are those of these DATs (by DAT name) in place of their system's audit
    /// DAT: a group of translations from a DAT of them, say (D75)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dats: Vec<String>,
    /// Only games with these details (the Library page's filters, Phase 6)
    #[serde(skip_serializing_if = "Details::is_empty")]
    pub details: Details,
    /// Its games' updates, DLC and patches you have go with them (D66)
    pub with_addons: bool,
}

/// A collection's rules by the games' details, as the Library page filters them: values
/// of one OR'd, the rules AND'd.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Details {
    /// Words in the title, name, description, developer or publisher
    pub search: String,
    pub genre: Vec<String>,
    pub decade: Vec<i64>,
    pub developer: Vec<String>,
    pub publisher: Vec<String>,
    /// At least this many players
    pub players: i64,
    /// At least this rating (0 to 1)
    pub rating: f64,
    /// pictures | description | manual | video | documents
    pub has: Vec<String>,
    /// Only games needing a look
    pub look: bool,
}

impl Details {
    pub fn is_empty(&self) -> bool {
        *self == Details::default()
    }
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
            from_files: vec![],
            dats: vec![],
            details: Details::default(),
            with_addons: true,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Collection {
    /// Empty for a new one, which `save` gives an id
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub rules: Rules,
    /// Games added and left out by hand, as `system/game name`
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
    /// What it should fit in, in bytes (an SD card's size, say); none: no limit
    #[serde(default)]
    pub size_limit: Option<u64>,
    /// What a collection made from folders or a DAT was read from (`folders`, or `dat`),
    /// for *Read them again…* (D49)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_from: Option<Value>,
    /// The first group's name (D75); empty: the collection's own
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group_name: String,
    /// Its groups after the first, whose fields are the collection's own above (D75), so
    /// an app from before groups reads it as its first group
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub modified: String,
}

/// A group of a collection (D75): a name and its own way of choosing games, as a
/// collection has them (rules, games added and left out by hand, what it was read from).
/// One can span systems, and a system can have several.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Group {
    /// Its id within the collection (the first group, the collection's own fields, is "")
    pub id: String,
    pub name: String,
    pub rules: Rules,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub made_from: Option<Value>,
    /// Keys a newer app added
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Collection {
    /// The first group's name: its own, else the collection's.
    pub fn first_group_name(&self) -> String {
        if self.group_name.trim().is_empty() { self.name.clone() } else { self.group_name.clone() }
    }

    /// Its groups in order: the first, made of its own fields, then the others.
    pub fn all_groups(&self) -> Vec<Group> {
        let first = Group {
            id: String::new(), name: self.first_group_name(), rules: self.rules.clone(), added: self.added.clone(), removed: self.removed.clone(),
            made_from: self.made_from.clone(), extra: Map::new(),
        };
        std::iter::once(first).chain(self.groups.iter().cloned()).collect()
    }

    /// One of its groups, by id ("" is the first).
    pub fn group(&self, id: &str) -> Option<Group> {
        self.all_groups().into_iter().find(|g| g.id == id)
    }

    /// Change one of its groups, by id ("" is the first: the collection's own fields).
    pub fn change_group(&mut self, id: &str, f: impl FnOnce(&mut Group)) -> Result<()> {
        if !id.is_empty() {
            let g = self.groups.iter_mut().find(|g| g.id == id).ok_or_else(|| anyhow!("{} has no group {id}.", self.name))?;
            f(g);
            return Ok(());
        }
        let mut g = self.all_groups().remove(0);
        f(&mut g);
        self.rules = g.rules;
        self.added = g.added;
        self.removed = g.removed;
        self.made_from = g.made_from;
        self.group_name = if g.name.trim() == self.name.trim() { String::new() } else { g.name.trim().to_string() };
        Ok(())
    }

    /// A new group by rules, after the others; returns its id.
    pub fn add_group(&mut self, name: &str, rules: Rules) -> String {
        let id = new_id(name);
        self.groups.push(Group { id: id.clone(), name: name.trim().to_string(), rules, ..Default::default() });
        id
    }

    /// Games out of it by hand: off the added list of the group that has them if they were
    /// added by hand there, else on that group's left-out list.
    pub fn leave_out(&mut self, w: &Worked, keys: &[String]) -> Result<()> {
        for k in keys {
            let id = w.group_of(k).map(|i| w.groups[i].id.clone()).unwrap_or_default();
            self.change_group(&id, |g| {
                if g.added.contains(k) {
                    g.added.retain(|x| x != k);
                } else if !g.removed.contains(k) {
                    g.removed.push(k.clone());
                }
            })?;
        }
        Ok(())
    }

    /// Games put back: off every group's left-out list.
    pub fn put_back(&mut self, keys: &[String]) {
        let set: HashSet<&String> = keys.iter().collect();
        self.removed.retain(|k| !set.contains(k));
        for g in &mut self.groups {
            g.removed.retain(|k| !set.contains(k));
        }
    }

    /// Games added by hand to a group (and no longer left out of it).
    pub fn add_games(&mut self, group: &str, keys: &[String]) -> Result<()> {
        self.change_group(group, |g| {
            g.removed.retain(|k| !keys.contains(k));
            for k in keys {
                if !g.added.contains(k) {
                    g.added.push(k.clone());
                }
            }
        })
    }
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

/// Where a collection's own DATs are kept (`_library/collections/<id>/`), and a group's
/// after the first (`<id>/<group>/`).
pub fn own_dir(lib: &Library, id: &str, group: &str) -> Result<PathBuf> {
    crate::library::valid_id(id)?;
    let d = dir(lib).join(id);
    if group.is_empty() {
        return Ok(d);
    }
    crate::library::valid_id(group)?;
    Ok(d.join(group))
}

/// Save a collection (a new one when it has no id), keeping keys a newer app added.
pub fn save(lib: &Library, mut c: Collection) -> Result<Collection> {
    lib.writable()?;
    c.name = c.name.trim().to_string();
    if c.name.is_empty() {
        bail!("Give the collection a name.");
    }
    if list(lib).iter().any(|o| o.id != c.id && o.name.eq_ignore_ascii_case(&c.name)) {
        bail!("There's a collection called {} already.", c.name);
    }
    // its groups: each named, once, with an id of its own
    c.group_name = c.group_name.trim().to_string();
    if c.group_name == c.name {
        c.group_name.clear();
    }
    let mut ids: HashSet<String> = HashSet::new();
    let mut names: HashSet<String> = [c.first_group_name().to_lowercase()].into();
    for g in &mut c.groups {
        g.name = g.name.trim().to_string();
        if g.name.is_empty() {
            bail!("Give the group a name.");
        }
        if !names.insert(g.name.to_lowercase()) {
            bail!("{} has a group called {} already.", c.name, g.name);
        }
        if g.id.is_empty() || crate::library::valid_id(&g.id).is_err() || ids.contains(&g.id) {
            g.id = new_id(&g.name);
        }
        ids.insert(g.id.clone());
    }
    if c.all_groups().iter().any(|g| !["have", "all", "none"].contains(&g.rules.include.as_str())) {
        bail!("A collection has the games you have, every game in the DAT, or only the games added to it.");
    }
    c.size_limit = c.size_limit.filter(|&n| n > 0);
    if c.id.is_empty() {
        c.id = new_id(&c.name);
        c.created = now();
        // a copy of one made from folders or a DAT gets copies of its DATs (and its
        // groups'): each collection's own DATs go when it's deleted
        let own = dir(lib).join(&c.id);
        let copy = |f: &mut String| -> Result<()> {
            let Ok(from) = lib.resolve(f) else { return Ok(()) };
            let Ok(rest) = from.strip_prefix(dir(lib)) else { return Ok(()) };
            // (its parts after the collection's id: the group's folder, the file)
            let parts: Vec<_> = rest.components().skip(1).collect();
            if parts.is_empty() || !from.is_file() {
                return Ok(());
            }
            let to = parts.iter().fold(own.clone(), |p, x| p.join(x));
            std::fs::create_dir_all(to.parent().unwrap_or(&own))?;
            std::fs::copy(&from, &to).with_context(|| format!("couldn't copy {}", from.display()))?;
            *f = lib.relative(&to).unwrap_or_else(|| to.display().to_string());
            Ok(())
        };
        for f in c.rules.from_files.iter_mut() {
            copy(f)?;
        }
        for g in c.groups.iter_mut() {
            for f in g.rules.from_files.iter_mut() {
                copy(f)?;
            }
        }
    }
    c.modified = now();
    let p = path(lib, &c.id)?;
    let mut v = std::fs::read(&p).ok().and_then(|t| serde_json::from_slice::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    for (k, x) in serde_json::to_value(&c)?.as_object().unwrap() {
        v[k] = x.clone();
    }
    // (what it no longer has isn't kept from before)
    if let Some(o) = v.as_object_mut() {
        for (k, gone) in [("made_from", c.made_from.is_none()), ("group_name", c.group_name.is_empty()), ("groups", c.groups.is_empty())] {
            if gone {
                o.remove(k);
            }
        }
    }
    std::fs::create_dir_all(dir(lib))?;
    crate::config::write_json(&p, &v)?;
    Ok(c)
}

pub fn delete(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    std::fs::remove_file(path(lib, id)?).map_err(|_| anyhow!("There's no collection {id}."))?;
    // and its own DATs (its groups' too), if it was made from folders or a DAT
    let own = dir(lib).join(id);
    if own.is_dir() {
        std::fs::remove_dir_all(own)?;
    }
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

/// The names of the DATs the collections' groups take their games from in place of their
/// systems' audit DATs (`Rules.dats`).
pub fn group_dat_names(lib: &Library) -> Vec<String> {
    let mut names: Vec<String> = list(lib).iter().flat_map(|c| c.all_groups()).flat_map(|g| g.rules.dats).collect();
    names.sort();
    names.dedup();
    names
}

/// The DATs of a system (by those names) a collection's group takes its games from in
/// place of its audit DAT, so the Library has rows for their games (browse.rs).
pub fn group_dats(conn: &Connection, lib: &Library, platform: &str, names: &[String]) -> Result<Vec<i64>> {
    let primary = audit::primary_dat(conn, lib, platform)?;
    let mut v = vec![];
    let mut q = conn.prepare("SELECT id FROM dats WHERE name = ?1 AND platform = ?2 AND current = 1")?;
    for n in names {
        for id in q.query_map(params![n, platform], |r| r.get::<_, i64>(0))? {
            let id = id?;
            if Some(id) != primary && !v.contains(&id) {
                v.push(id);
            }
        }
    }
    Ok(v)
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
    /// The bytes its files take on disk (its archives, images or files; 0 when there are none)
    pub size: u64,
    /// The bytes of its ROMs as the DAT lists them (unpacked)
    pub dat_size: u64,
}

impl Game {
    pub fn key(&self) -> String {
        format!("{}/{}", self.platform, self.name)
    }
}

/// A collection's games on one system, of one of its groups, from one DAT.
#[derive(Clone, Debug)]
pub struct System {
    pub platform: String,
    pub dat_id: i64,
    /// The group they're in (its place in `Worked::groups`)
    pub group: usize,
    pub games: Vec<Game>,
    /// Left out by hand (they'd be in otherwise)
    pub removed: Vec<Game>,
    /// Its games' updates, DLC and patches that you have (all or some of their files),
    /// with the rules' *With their updates and DLC* (D66)
    pub addons: Vec<Addon>,
}

/// An update, DLC or patch going with one of a collection's games.
#[derive(Clone, Debug)]
pub struct Addon {
    /// The DAT it's in (its own, usually: "(Updates and DLC)")
    pub dat_id: i64,
    /// The game it belongs to (its name)
    pub base: String,
    pub game: Game,
}

/// A group as the worked-out collection names it.
#[derive(Clone, Debug, Default)]
pub struct GroupRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Default)]
pub struct Worked {
    /// Each group's games by system, the groups in order
    pub systems: Vec<System>,
    /// Its groups, in order (one for a collection without others)
    pub groups: Vec<GroupRef>,
    /// Systems asked for that have no DAT
    pub no_dat: Vec<String>,
    /// DATs named as sources that aren't in the library
    pub missing_dats: Vec<String>,
}

impl Worked {
    pub fn games(&self) -> impl Iterator<Item = &Game> {
        self.systems.iter().flat_map(|s| s.games.iter())
    }

    /// The group a game (by key) is in, or left out of by hand.
    pub fn group_of(&self, key: &str) -> Option<usize> {
        self.systems.iter().find(|s| s.games.iter().chain(&s.removed).any(|g| g.key() == key)).map(|s| s.group)
    }

    pub fn summary(&self) -> Value {
        // on disk: the files of the games there (all or in part); the rest by the DAT's sizes
        // with the add-ons going with them
        let size = |s: &System| s.games.iter().map(|g| g.size).sum::<u64>() + s.addons.iter().map(|a| a.game.size).sum::<u64>();
        let wanting = |g: &[Game]| g.iter().filter(|g| g.status != "have").map(|g| g.dat_size.saturating_sub(g.size)).sum::<u64>();
        // the systems, each group's together
        let by_system = |of: &[&System]| -> Vec<Value> {
            let mut order: Vec<&str> = vec![];
            for s in of {
                if !order.contains(&s.platform.as_str()) {
                    order.push(&s.platform);
                }
            }
            order
                .iter()
                .map(|p| {
                    let these: Vec<&&System> = of.iter().filter(|s| s.platform == *p).collect();
                    let sum = |f: &dyn Fn(&System) -> u64| these.iter().map(|s| f(s)).sum::<u64>();
                    json!({
                        "platform": p, "name": platforms::name_of(p), "dat": these[0].dat_id, "dats": these.iter().map(|s| s.dat_id).collect::<Vec<_>>(),
                        "games": sum(&|s| s.games.len() as u64), "have": sum(&|s| s.games.iter().filter(|g| g.status == "have").count() as u64),
                        "removed": sum(&|s| s.removed.len() as u64), "addons": sum(&|s| s.addons.len() as u64),
                        "size": sum(&|s| size(s)), "missing_size": sum(&|s| wanting(&s.games)),
                    })
                })
                .collect()
        };
        let all: Vec<&System> = self.systems.iter().collect();
        let groups: Vec<Value> = self
            .groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let of: Vec<&System> = self.systems.iter().filter(|s| s.group == i).collect();
                json!({
                    "id": g.id, "name": g.name, "games": of.iter().map(|s| s.games.len()).sum::<usize>(),
                    "have": of.iter().map(|s| s.games.iter().filter(|g| g.status == "have").count()).sum::<usize>(),
                    "removed": of.iter().map(|s| s.removed.len()).sum::<usize>(), "size": of.iter().map(|s| size(s)).sum::<u64>(),
                    "systems": by_system(&of),
                })
            })
            .collect();
        json!({
            "games": self.systems.iter().map(|s| s.games.len()).sum::<usize>(),
            "have": self.games().filter(|g| g.status == "have").count(),
            "removed": self.systems.iter().map(|s| s.removed.len()).sum::<usize>(),
            "addons": self.systems.iter().map(|s| s.addons.len()).sum::<usize>(),
            "size": self.systems.iter().map(size).sum::<u64>(),
            "missing_size": self.systems.iter().map(|s| wanting(&s.games)).sum::<u64>(),
            "systems": by_system(&all),
            "groups": groups,
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

/// The systems a group's rules cover: those ticked, else every system with a DAT.
fn systems_of(conn: &Connection, r: &Rules) -> Result<Vec<String>> {
    if !r.platforms.is_empty() {
        return Ok(r.platforms.clone());
    }
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != 'bios' ORDER BY platform")?;
    let v = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

/// The systems a group's games are of, each with the DAT they're from: its audit DAT, or
/// (`Rules.dats`) the DATs named for it. With DATs named and no systems ticked, their
/// systems. DATs named that aren't in the library are added to `missing`.
fn dats_of(conn: &Connection, lib: &Library, r: &Rules, missing: &mut Vec<String>) -> Result<Vec<(String, Option<i64>)>> {
    if r.dats.is_empty() {
        let mut v = vec![];
        for p in systems_of(conn, r)? {
            let d = audit::primary_dat(conn, lib, &p)?;
            v.push((p, d));
        }
        return Ok(v);
    }
    let mut named: Vec<(String, i64)> = vec![];
    let mut q = conn.prepare("SELECT platform, id FROM dats WHERE name = ?1 AND current = 1 ORDER BY platform")?;
    for n in &r.dats {
        let rows: Vec<(String, i64)> = q.query_map([n], |x| Ok((x.get(0)?, x.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        if rows.is_empty() && !missing.contains(n) {
            missing.push(n.clone());
        }
        named.extend(rows);
    }
    if r.platforms.is_empty() {
        return Ok(named.into_iter().map(|(p, id)| (p, Some(id))).collect());
    }
    let mut v = vec![];
    for p in &r.platforms {
        let mine: Vec<i64> = named.iter().filter(|(x, _)| x == p).map(|(_, id)| *id).collect();
        if mine.is_empty() {
            v.push((p.clone(), audit::primary_dat(conn, lib, p)?));
        } else {
            v.extend(mine.into_iter().map(|id| (p.clone(), Some(id))));
        }
    }
    Ok(v)
}

/// What the collection holds now.
pub fn work_out(ix: &Index, lib: &Library, c: &Collection) -> Result<Arc<Worked>> {
    let conn = ix.conn();
    let groups = c.all_groups();
    let mut missing = vec![];
    let mut dats = vec![];
    for g in &groups {
        dats.push(dats_of(conn, lib, &g.rules, &mut missing)?);
    }
    // rules by details read the library's rows (browse.rs), brought up to date first
    let library = if groups.iter().all(|g| g.rules.details.is_empty()) {
        String::new()
    } else {
        crate::browse::refresh(ix, lib)?;
        conn.query_row("SELECT COALESCE(group_concat(value, ','), '') FROM index_meta WHERE key LIKE 'library:%'", [], |r| r.get(0))?
    };
    // its own DAT files, by size and time
    let files: Vec<String> = groups.iter().flat_map(|g| g.rules.from_files.iter()).map(|f| {
        let m = lib.resolve(f).ok().and_then(|p| std::fs::metadata(p).ok());
        format!("{f}:{:?}:{:?}", m.as_ref().map(|m| m.len()), m.and_then(|m| m.modified().ok()))
    }).collect();
    // (its name and size limit don't change its games)
    // and the add-ons linked by hand
    let hand = if groups.iter().any(|g| g.rules.with_addons) { lib.meta()["addons"].to_string() } else { String::new() };
    let ways: Vec<Value> = groups.iter().map(|g| json!([g.id, g.name, g.rules, g.added, g.removed])).collect();
    let key = format!("{}|{}|{dats:?}|{library}|{files:?}|{hand}|{}", ix.path().display(), serde_json::to_string(&ways)?, audit::generation(conn)?);
    if let Some((k, w)) = LAST.lock().unwrap().as_ref() {
        if *k == key {
            return Ok(w.clone());
        }
    }
    let mut w = Worked { missing_dats: missing, ..Default::default() };
    // a game an earlier group has (or left out by hand) isn't repeated in a later one,
    // by its key: the same game of another DAT has the same name
    let (mut inside, mut out): (HashSet<String>, HashSet<String>) = (HashSet::new(), HashSet::new());
    for (i, g) in groups.iter().enumerate() {
        w.groups.push(GroupRef { id: g.id.clone(), name: g.name.clone() });
        for mut s in group_systems(conn, lib, g, &dats[i], &mut w)? {
            s.group = i;
            s.games.retain(|x| !inside.contains(&x.key()) && (x.manual || g.added.contains(&x.key()) || !out.contains(&x.key())));
            s.removed.retain(|x| !inside.contains(&x.key()) && !out.contains(&x.key()));
            inside.extend(s.games.iter().map(Game::key));
            out.extend(s.removed.iter().map(Game::key));
            if g.rules.with_addons {
                s.addons = addons_of(conn, lib, &s.platform, &s.games)?;
            }
            w.systems.push(s);
        }
    }
    let w = Arc::new(w);
    *LAST.lock().unwrap() = Some((key, w.clone()));
    Ok(w)
}

/// The hashes of the ROMs, and the names of the games, of the DATs a collection takes its games from.
struct Sources {
    names: HashSet<String>,
    sha1: HashSet<String>,
    crc: HashSet<(String, i64)>,
    /// The collection's own DATs' (`from_files`): a game is theirs only when they hold
    /// every ROM of it, so a ROM it shares with another game doesn't bring that one in
    own_sha1: HashSet<String>,
    own_crc: HashSet<(String, i64)>,
}

fn sources(conn: &Connection, lib: &Library, names: &[String], files: &[String], missing: &mut Vec<String>) -> Result<Option<Sources>> {
    if names.is_empty() && files.is_empty() {
        return Ok(None);
    }
    let mut s = Sources { names: HashSet::new(), sha1: HashSet::new(), crc: HashSet::new(), own_sha1: HashSet::new(), own_crc: HashSet::new() };
    for f in files {
        let read = lib.resolve(f).ok().filter(|p| p.is_file()).and_then(|p| crate::dat::read_file(&p).ok());
        let Some(read) = read else {
            missing.push(f.rsplit('/').next().unwrap_or(f).to_string());
            continue;
        };
        for g in read.into_iter().flat_map(|d| d.games) {
            for rom in &g.roms {
                if let Some(h) = &rom.sha1 {
                    s.own_sha1.insert(h.clone());
                }
                if let (Some(c), Some(z)) = (&rom.crc32, rom.size) {
                    s.own_crc.insert((c.clone(), z as i64));
                }
            }
            s.names.insert(g.name);
        }
    }
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
pub(crate) fn revision_key(s: &str) -> Vec<u64> {
    s.split(['.', ' '])
        .map(|p| p.parse::<u64>().unwrap_or_else(|_| p.chars().next().map(|c| c.to_ascii_uppercase() as u64).unwrap_or(0)))
        .collect()
}

/// A group's games, by system (each from its DAT), as its rules and hands pick them.
fn group_systems(conn: &Connection, lib: &Library, g: &Group, dats: &[(String, Option<i64>)], w: &mut Worked) -> Result<Vec<System>> {
    let r = &g.rules;
    let mut out = vec![];
    let src = sources(conn, lib, &r.from_dats, &r.from_files, &mut w.missing_dats)?;
    let exclude: HashSet<&str> = r.exclude.iter().map(String::as_str).collect();
    let langs: HashSet<String> = r.languages.iter().map(|l| l.to_ascii_lowercase()).collect();
    let search = r.search.trim().to_lowercase();
    let added: HashSet<&str> = g.added.iter().map(String::as_str).collect();
    let removed: HashSet<&str> = g.removed.iter().map(String::as_str).collect();
    let rank = |regions: &str| -> usize {
        split(regions).filter_map(|g| r.regions.iter().position(|x| x.eq_ignore_ascii_case(g))).min().unwrap_or(r.regions.len())
    };
    for (platform, dat) in dats {
        let Some(dat_id) = *dat else {
            if !w.no_dat.contains(platform) {
                w.no_dat.push(platform.clone());
            }
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
        let sizes = sizes(conn, dat_id)?;
        // a source DAT's games by their ROMs' hashes
        let by_hash: HashSet<i64> = match &src {
            Some(s) => {
                let mut ids = HashSet::new();
                // each game's ROMs (nodumps aside), and how many its own DATs hold
                let mut own: HashMap<i64, (usize, usize)> = HashMap::new();
                let mut q = conn.prepare("SELECT game_id, sha1, crc32, size FROM dat_roms WHERE dat_id = ?1 AND COALESCE(status, '') <> 'nodump'")?;
                let mut rows = q.query([dat_id])?;
                while let Some(x) = rows.next()? {
                    let game: i64 = x.get(0)?;
                    let sha1: Option<String> = x.get(1)?;
                    let crc: Option<(String, i64)> = match (x.get::<_, Option<String>>(2)?, x.get::<_, Option<i64>>(3)?) {
                        (Some(c), Some(z)) => Some((c, z)),
                        _ => None,
                    };
                    if sha1.as_ref().is_some_and(|h| s.sha1.contains(h)) || crc.as_ref().is_some_and(|k| s.crc.contains(k)) {
                        ids.insert(game);
                    }
                    let e = own.entry(game).or_default();
                    e.0 += 1;
                    if sha1.as_ref().is_some_and(|h| s.own_sha1.contains(h)) || crc.as_ref().is_some_and(|k| s.own_crc.contains(k)) {
                        e.1 += 1;
                    }
                }
                ids.extend(own.into_iter().filter(|(_, (all, held))| *held > 0 && held == all).map(|(g, _)| g));
                ids
            }
            None => HashSet::new(),
        };
        let with_details = if r.details.is_empty() { None } else { Some(crate::browse::matching(conn, platform, &r.details)?) };
        let passes = |g: &Row| -> bool {
            r.include != "none"
                && (r.include == "all" || g.status == "have")
                && with_details.as_ref().is_none_or(|d| d.contains(&g.id))
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
        let game = |g: &Row, manual: bool| {
            let (size, dat_size) = sizes.get(&g.id).copied().unwrap_or_default();
            Game {
                id: g.id, platform: platform.clone(), name: g.name.clone(), title: g.title.clone(), regions: g.regions.clone(), status: g.status.clone(), manual,
                size, dat_size,
            }
        };
        let mut sys = System { platform: platform.clone(), dat_id, group: 0, games: vec![], removed: vec![], addons: vec![] };
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
        out.push(sys);
    }
    Ok(out)
}

/// The updates, DLC and patches you have (all or some of their files) of these games.
fn addons_of(conn: &Connection, lib: &Library, platform: &str, games: &[Game]) -> Result<Vec<Addon>> {
    let mut by = crate::addons::by_base(conn, lib, platform)?;
    let links: Vec<crate::addons::Link> = games.iter().flat_map(|g| by.remove(&g.name).unwrap_or_default()).collect();
    if links.is_empty() {
        return Ok(vec![]);
    }
    let dats: std::collections::BTreeSet<i64> = links.iter().map(|l| l.dat_id).collect();
    let status = crate::addons::statuses(conn, dats.iter().copied())?;
    let mut size = HashMap::new();
    for d in &dats {
        size.extend(sizes(conn, *d)?);
    }
    let mut v = vec![];
    for l in links {
        let st = status.get(&l.id).cloned().unwrap_or_else(|| "missing".into());
        if st == "missing" {
            continue;
        }
        let (size, dat_size) = size.get(&l.id).copied().unwrap_or_default();
        v.push(Addon {
            dat_id: l.dat_id,
            base: l.base.clone().unwrap_or_default(),
            game: Game { id: l.id, platform: platform.to_string(), name: l.name, title: String::new(), regions: String::new(), status: st, manual: false, size, dat_size },
        });
    }
    Ok(v)
}

/// Each game of a DAT with the bytes its files take on disk and the bytes of its ROMs as
/// the DAT lists them. On disk, a ROM counts once, by one of the files found for it (the
/// archive or image it's in, else the file itself), and each file counts once per game.
pub(crate) fn sizes(conn: &Connection, dat_id: i64) -> Result<HashMap<i64, (u64, u64)>> {
    let mut out: HashMap<i64, (u64, u64)> = HashMap::new();
    let mut q = conn.prepare("SELECT game_id, SUM(COALESCE(size, 0)) FROM dat_roms WHERE dat_id = ?1 AND (status IS NULL OR status != 'nodump') GROUP BY game_id")?;
    for r in q.query_map([dat_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (g, z) = r?;
        out.entry(g).or_default().1 = z.max(0) as u64;
    }
    let mut q = conn.prepare(
        "WITH pick AS (SELECT r.game_id, MIN(COALESCE(f.parent_id, f.id)) AS file_id
                       FROM dat_roms r JOIN matches m ON m.rom_id = r.id JOIN files f ON f.id = m.file_id
                       WHERE r.dat_id = ?1 AND f.missing_since IS NULL GROUP BY r.id)
         SELECT p.game_id, SUM(x.size) FROM (SELECT DISTINCT game_id, file_id FROM pick) p JOIN files x ON x.id = p.file_id GROUP BY p.game_id",
    )?;
    for r in q.query_map([dat_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
        let (g, z) = r?;
        out.entry(g).or_default().0 = z.max(0) as u64;
    }
    Ok(out)
}

/// A page of a collection's games (or of those left out by hand), of a system or a group
/// (by id), with a search, by name or (`by_size`) the biggest first.
#[allow(clippy::too_many_arguments)]
pub fn page(w: &Worked, platform: Option<&str>, group: Option<&str>, removed: bool, search: &str, by_size: bool, offset: usize, limit: usize) -> Value {
    let s = search.trim().to_lowercase();
    let mut items: Vec<(&Game, usize)> = w
        .systems
        .iter()
        .filter(|x| platform.is_none_or(|p| p == x.platform))
        .filter(|x| group.is_none_or(|g| w.groups.get(x.group).is_some_and(|r| r.id == g)))
        .flat_map(|x| if removed { x.removed.iter() } else { x.games.iter() }.map(move |g| (g, x.group)))
        .filter(|(g, _)| s.is_empty() || g.name.to_lowercase().contains(&s))
        .collect();
    if by_size {
        // a game that isn't there by its DAT size
        let bytes = |g: &Game| if g.size > 0 { g.size } else { g.dat_size };
        items.sort_by(|(a, _), (b, _)| bytes(b).cmp(&bytes(a)).then(a.name.cmp(&b.name)));
    }
    let none = GroupRef::default();
    json!({
        "total": items.len(),
        "items": items.iter().skip(offset).take(limit.clamp(1, 1000)).map(|(g, i)| {
            let r = w.groups.get(*i).unwrap_or(&none);
            json!({ "key": g.key(), "platform": g.platform, "platform_name": platforms::name_of(&g.platform), "name": g.name, "title": g.title,
                    "regions": g.regions, "status": g.status, "manual": g.manual, "size": g.size, "dat_size": g.dat_size,
                    "group": r.id, "group_name": r.name })
        }).collect::<Vec<_>>(),
    })
}

/// Games of a system's DAT that aren't in the collection, to add by hand to one of its
/// groups (by id; "" is the first): of the DAT that group takes the system's games from.
pub fn candidates(ix: &Index, lib: &Library, c: &Collection, group: &str, platform: &str, search: &str, limit: usize) -> Result<Value> {
    let conn = ix.conn();
    let g = c.group(group).ok_or_else(|| anyhow!("{} has no group {group}.", c.name))?;
    let mine = Rules { platforms: vec![platform.to_string()], ..g.rules.clone() };
    let Some(dat) = dats_of(conn, lib, &mine, &mut vec![])?.into_iter().find_map(|(_, d)| d) else {
        bail!("{} has no DAT.", platforms::name_of(platform))
    };
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
pub(crate) fn safe(s: &str) -> String {
    let s: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    s.trim().trim_end_matches('.').chars().take(150).collect()
}

/// Save the collection as DATs, one per system of each group ("Name - System", or "Name -
/// Group - System" for a group not named as the collection), into `to` (by default
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
    // each group's games of a system together (a group can take them from more than one DAT)
    let mut done: Vec<(usize, &str)> = vec![];
    for s in w.systems.iter().filter(|s| !s.games.is_empty()) {
        if done.contains(&(s.group, s.platform.as_str())) {
            continue;
        }
        done.push((s.group, &s.platform));
        let sys = platforms::name_of(&s.platform);
        let group = w.groups.get(s.group).cloned().unwrap_or_default();
        let name = if group.name.is_empty() || group.name == c.name { format!("{} - {sys}", c.name) } else { format!("{} - {} - {sys}", c.name, group.name) };
        let games: Vec<&Game> = w.systems.iter().filter(|x| x.group == s.group && x.platform == s.platform).flat_map(|x| x.games.iter()).collect();
        let text = dat_xml(ix.conn(), &name, &format!("{} ({} games of {sys}, made by Game Repo)", c.name, games.len()), &games)?;
        let p = dir.join(format!("{}.dat", safe(&name)));
        std::fs::write(&p, text).with_context(|| format!("couldn't write {}", p.display()))?;
        files.push(json!({ "platform": s.platform, "group": group.id, "group_name": group.name, "path": p.display().to_string(), "rel": lib.relative(&p), "games": games.len() }));
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
        let mut c = Collection { name: "Test".into(), ..Default::default() };
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
        // only the games added by hand, saved from the page without an id
        let c: Collection = serde_json::from_value(json!({ "name": "Picked", "rules": { "platforms": ["gb"], "include": "none" }, "added": ["gb/Delta (World)"] })).unwrap();
        let c = save(&lib, c).unwrap();
        assert!(!c.id.is_empty());
        assert_eq!(names(&work_out(&ix, &lib, &c).unwrap()), ["Delta (World)"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_collection_knows_its_size_on_disk_and_what_it_lacks() {
        let (dir, lib, ix) = fixture("collection-size");
        let w = work_out(&ix, &lib, &with(|_| {})).unwrap();
        let sum = w.summary();
        assert_eq!((sum["size"].as_u64(), sum["missing_size"].as_u64()), (Some(4 * 1024), Some(0)));
        assert_eq!(sum["systems"][0]["size"].as_u64(), Some(4 * 1024));
        // every game: Alpha (USA) and Zeta aren't there, so they count by the DAT's sizes
        let w = work_out(&ix, &lib, &with(|r| r.include = "all".into())).unwrap();
        let sum = w.summary();
        assert_eq!((sum["size"].as_u64(), sum["missing_size"].as_u64()), (Some(3 * 1024), Some(2 * 1024)));
        let p = page(&w, None, None, false, "", true, 0, 10);
        assert!(p["items"].as_array().unwrap().iter().all(|g| g["size"].as_u64().unwrap() + g["dat_size"].as_u64().unwrap() >= 1024));
        // a size limit is kept with the collection and doesn't change its games
        let mut c = with(|_| {});
        c.size_limit = Some(32_000_000_000);
        let c = save(&lib, c).unwrap();
        assert_eq!(load(&lib, &c.id).unwrap().size_limit, Some(32_000_000_000));
        assert_eq!(work_out(&ix, &lib, &c).unwrap().games().count(), 4);
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

    /// The Game Boy collection with a second group, "Favs", of every game of the
    /// Favourites DAT in place of the Game Boy's audit DAT.
    fn with_favs() -> Collection {
        let mut c = with(|_| {});
        c.add_group("Favs", Rules { platforms: vec!["gb".into()], include: "all".into(), dats: vec!["Favourites".into()], ..Rules::default() });
        c
    }

    fn group_names(w: &Worked, i: usize) -> Vec<String> {
        w.systems.iter().filter(|s| s.group == i).flat_map(|s| s.games.iter().map(|g| g.name.clone())).collect()
    }

    #[test]
    fn each_group_picks_its_own_games_from_its_own_dat_and_none_twice() {
        let (dir, lib, ix) = fixture("collection-groups");
        let fav: i64 = ix.conn().query_row("SELECT id FROM dats WHERE name = 'Favourites'", [], |r| r.get(0)).unwrap();
        // the first group as before; the second from the Favourites DAT, not the Game Boy's
        let w = work_out(&ix, &lib, &with_favs()).unwrap();
        assert_eq!(w.groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["Test", "Favs"]);
        assert_eq!(group_names(&w, 0), ["Alpha (Europe)", "Beta (Europe) (Rev 1)", "Delta (World)", "Epsilon (Japan) (En,Ja)"]);
        assert_eq!(group_names(&w, 1), ["My Delta", "Zeta (Japan)"]);
        assert!(w.systems.iter().filter(|s| s.group == 1).all(|s| s.dat_id == fav));
        // My Delta is Delta's ROM, so you have it; Zeta isn't on disk
        let have: Vec<&str> = w.systems[1].games.iter().map(|g| g.status.as_str()).collect();
        assert_eq!(have, ["have", "missing"]);
        assert_eq!(w.group_of("gb/My Delta"), Some(1));
        // a game an earlier group has isn't repeated in a later one: every game of the Game
        // Boy's DAT first, so Zeta's in the first group only
        let mut c = with_favs();
        c.rules.include = "all".into();
        let w = work_out(&ix, &lib, &c).unwrap();
        assert!(group_names(&w, 0).contains(&"Zeta (Japan)".to_string()));
        assert_eq!(group_names(&w, 1), ["My Delta"]);
        assert_eq!(w.games().filter(|g| g.name == "Zeta (Japan)").count(), 1);
        // nor one an earlier group left out by hand, unless it's added to the later one by hand
        c.removed = vec!["gb/Zeta (Japan)".into()];
        let w = work_out(&ix, &lib, &c).unwrap();
        assert_eq!(group_names(&w, 1), ["My Delta"]);
        let favs = c.groups[0].id.clone();
        c.add_games(&favs, &["gb/Zeta (Japan)".into()]).unwrap();
        let w = work_out(&ix, &lib, &c).unwrap();
        assert_eq!(group_names(&w, 1), ["My Delta", "Zeta (Japan)"]);
        // left out by hand: on the list of the group that has it; put back from every group
        let mut c = with_favs();
        let w = work_out(&ix, &lib, &c).unwrap();
        c.leave_out(&w, &["gb/My Delta".into(), "gb/Alpha (Europe)".into()]).unwrap();
        assert_eq!((c.removed.clone(), c.groups[0].removed.clone()), (vec!["gb/Alpha (Europe)".to_string()], vec!["gb/My Delta".to_string()]));
        let w = work_out(&ix, &lib, &c).unwrap();
        assert_eq!(group_names(&w, 1), ["Zeta (Japan)"]);
        assert_eq!(w.summary()["groups"][1]["removed"], 1);
        c.put_back(&["gb/My Delta".into(), "gb/Alpha (Europe)".into()]);
        assert!(c.removed.is_empty() && c.groups[0].removed.is_empty());
        // the summary: each group with its games and systems, the systems together
        let sum = work_out(&ix, &lib, &c).unwrap().summary();
        assert_eq!((sum["games"].as_u64(), sum["groups"][0]["games"].as_u64(), sum["groups"][1]["games"].as_u64()), (Some(6), Some(4), Some(2)));
        assert_eq!(sum["systems"].as_array().unwrap().len(), 1, "{sum}");
        assert_eq!(sum["systems"][0]["dats"].as_array().unwrap().len(), 2);
        // a page of one group's games says which group each is in
        let w = work_out(&ix, &lib, &c).unwrap();
        let p = page(&w, None, Some(c.groups[0].id.as_str()), false, "", false, 0, 10);
        assert_eq!((p["total"].as_u64(), p["items"][0]["group_name"].as_str()), (Some(2), Some("Favs")));
        // games to add by hand to the group: of its DAT
        let saved = save(&lib, c).unwrap();
        let cand = candidates(&ix, &lib, &saved, &saved.groups[0].id, "gb", "", 10).unwrap();
        assert!(cand.as_array().unwrap().is_empty(), "both of the Favourites DAT's games are in it: {cand}");
        let cand = candidates(&ix, &lib, &saved, "", "gb", "", 10).unwrap();
        assert!(cand.as_array().unwrap().iter().any(|g| g["name"] == "Gamma (USA) (Beta)"), "{cand}");
        // the Library has rows for the Favourites DAT's games the Game Boy's lacks by name
        crate::browse::refresh(&ix, &lib).unwrap();
        let rows: Vec<String> = ix.conn().prepare("SELECT key FROM library WHERE platform = 'gb' ORDER BY key").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert!(rows.contains(&"gb/My Delta".to_string()) && rows.iter().filter(|k| k.starts_with("gb/Zeta")).count() == 1, "{rows:?}");
        // its games as the Library lists them: a section for each group, with its name and count
        let q = crate::browse::Query { collection: saved.id.clone(), all: true, group: "collection".into(), limit: 50, ..Default::default() };
        let v = crate::browse::query(&ix, &lib, &q).unwrap();
        let groups: Vec<(String, i64)> = v["groups"].as_array().unwrap().iter().map(|g| (g["name"].as_str().unwrap().to_string(), g["count"].as_i64().unwrap())).collect();
        assert_eq!(groups, [("Test".to_string(), 4), ("Favs".to_string(), 2)], "{v}");
        assert_eq!(v["items"].as_array().unwrap().last().unwrap()["group_name"], "Favs");
        // and one group's, and those left out by hand
        let q1 = crate::browse::Query { collection_group: Some(saved.groups[0].id.clone()), ..q.clone() };
        assert_eq!(crate::browse::query(&ix, &lib, &q1).unwrap()["total"], 2);
        let mut out = saved.clone();
        out.leave_out(&w, &["gb/Delta (World)".into()]).unwrap();
        let out = save(&lib, out).unwrap();
        let q2 = crate::browse::Query { collection: out.id.clone(), left_out: true, ..q.clone() };
        let v = crate::browse::query(&ix, &lib, &q2).unwrap();
        assert_eq!((v["total"].as_i64(), v["items"][0]["key"].as_str()), (Some(1), Some("gb/Delta (World)")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_collection_from_before_groups_is_its_first_group_and_keeps_its_fields() {
        let (dir, lib, ix) = fixture("collection-old");
        // a collection as an app from before groups wrote it
        let old = json!({ "id": "old-1", "name": "Old", "rules": { "platforms": ["gb"], "include": "have" }, "added": ["gb/Gamma (USA) (Beta)"], "removed": [],
                          "size_limit": null, "created": "2026-01-01", "modified": "2026-01-01", "colour": "blue" });
        std::fs::create_dir_all(super::dir(&lib)).unwrap();
        std::fs::write(path(&lib, "old-1").unwrap(), serde_json::to_vec(&old).unwrap()).unwrap();
        let c = load(&lib, "old-1").unwrap();
        let groups = c.all_groups();
        assert_eq!((groups.len(), groups[0].name.as_str(), groups[0].id.as_str()), (1, "Old", ""));
        assert_eq!(groups[0].added, ["gb/Gamma (USA) (Beta)"]);
        let w = work_out(&ix, &lib, &c).unwrap();
        assert_eq!((w.groups.len(), w.games().count()), (1, 5));
        // a group added: its own fields are still the first group's, so an older app reads it as that
        let mut c = c;
        c.add_group("More", Rules { platforms: vec!["gb".into()], include: "all".into(), search: "zeta".into(), ..Rules::default() });
        c.change_group("", |g| g.name = "The first".into()).unwrap();
        let c = save(&lib, c).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(path(&lib, "old-1").unwrap()).unwrap()).unwrap();
        assert_eq!((v["name"].as_str(), v["group_name"].as_str(), v["colour"].as_str()), (Some("Old"), Some("The first"), Some("blue")));
        assert_eq!((v["rules"]["include"].as_str(), v["added"][0].as_str()), (Some("have"), Some("gb/Gamma (USA) (Beta)")));
        assert_eq!(v["groups"][0]["name"], "More");
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct Before {
            name: String,
            rules: Value,
            added: Vec<String>,
            removed: Vec<String>,
        }
        assert_eq!(serde_json::from_value::<Before>(v.clone()).unwrap().added, ["gb/Gamma (USA) (Beta)"]);
        let w = work_out(&ix, &lib, &c).unwrap();
        assert_eq!(group_names(&w, 1), ["Zeta (Japan)"]);
        // the groups taken away: none of their keys is left behind
        let mut c = load(&lib, "old-1").unwrap();
        c.groups.clear();
        c.change_group("", |g| g.name = "Old".into()).unwrap();
        save(&lib, c).unwrap();
        let v: Value = serde_json::from_slice(&std::fs::read(path(&lib, "old-1").unwrap()).unwrap()).unwrap();
        assert!(v.get("groups").is_none() && v.get("group_name").is_none(), "{v}");
        // two groups of one name are refused, as is a group without one
        let mut c = load(&lib, "old-1").unwrap();
        c.add_group("old", Rules::default());
        assert!(save(&lib, c.clone()).is_err());
        c.groups[0].name = " ".into();
        assert!(save(&lib, c).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_collection_of_groups_is_saved_as_a_dat_for_each_groups_system() {
        let (dir, lib, ix) = fixture("collection-group-dats");
        let c = save(&lib, with_favs()).unwrap();
        let r = save_dats(&ix, &lib, &c, Some(&dir.join("out"))).unwrap();
        let names: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f["path"].as_str().unwrap()).file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Test - Game Boy.dat", "Test - Favs - Game Boy.dat"]);
        assert_eq!(r["files"][1]["group_name"], "Favs");
        let d = crate::dat::parse(&std::fs::read(r["files"][1]["path"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(d.games.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["My Delta", "Zeta (Japan)"]);
        assert_eq!(d.games[0].roms[0].name, "delta.gb", "as the group's own DAT has it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
