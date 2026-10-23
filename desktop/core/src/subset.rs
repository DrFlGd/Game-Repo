//! Collections made from folders or from a DAT (docs/PLAN.md, "Collections rework
//! design", D49): the files of a few folders from anywhere, or the games of a DAT read in
//! place, matched by hash against each system's audit DAT, reviewed, then kept as DATs
//! of the collection's own (`_library/collections/<id>/`, one per system, with the full
//! set's names and hashes) that its rules take their games from. Games imported from
//! folders can go into one too (D140), with the files no DAT knew waiting on it.

use crate::archive;
use crate::audit::{self, GAMES};
use crate::collection::{self, Collection, Game, Rules, Waiting, WaitingRom};
use crate::dat;
use crate::hash::{self, Hashes};
use crate::index::Index;
use crate::library::Library;
use crate::organise::{self, key};
use crate::platforms;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

/// A file read (or a game of a DAT): what it's shown as, and the hashes of what it holds.
#[derive(Clone, Debug, Default)]
pub struct Found {
    /// Its path (a file) or name (a DAT's game)
    pub path: String,
    /// Shown as: its path from the folder picked, or the game's name
    pub shown: String,
    pub size: u64,
    pub roms: Vec<Hashes>,
    /// Why it couldn't be read
    pub error: Option<String>,
}

/// Hidden and system entries, as a scan leaves them.
fn skipped(name: &str, ft: &std::fs::FileType) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.') || ft.is_symlink() || ["thumbs.db", "desktop.ini"].contains(&lower.as_str())
}

/// Every file under the folders (or the files themselves), with the path it's shown by.
fn walk(paths: &[PathBuf], cancel: &AtomicBool) -> Vec<(PathBuf, String, u64)> {
    let mut out = vec![];
    for top in paths {
        let base = top.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut stack = vec![top.clone()];
        while let Some(p) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                return out;
            }
            let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if p != *top && skipped(&name, &meta.file_type()) {
                continue;
            }
            if meta.is_dir() {
                let mut kids: Vec<PathBuf> = std::fs::read_dir(&p).into_iter().flatten().flatten().map(|e| e.path()).collect();
                kids.sort();
                stack.extend(kids.into_iter().rev());
            } else if meta.is_file() {
                let shown = p.strip_prefix(&base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
                out.push((p, shown, meta.len()));
            }
        }
    }
    out
}

/// The sizes of the audit DATs' ROMs: a file of none of them (nor of one with a header
/// taken off) can't be a game of theirs, so it isn't read (D133).
pub fn dat_sizes(conn: &Connection, lib: &Library) -> Result<HashSet<u64>> {
    let mut q = conn.prepare("SELECT DISTINCT size FROM dat_roms WHERE dat_id = ?1 AND size IS NOT NULL")?;
    let mut out = HashSet::new();
    for id in audit_dats(conn, lib)?.keys() {
        for s in q.query_map([id], |r| r.get::<_, i64>(0))? {
            out.insert(s? as u64);
        }
    }
    Ok(out)
}

/// Whether a file of `size` bytes may be a ROM of these sizes: as it is, or with a header
/// the DATs leave out (`hash::detect`'s) taken off.
fn may_be(sizes: &HashSet<u64>, size: u64) -> bool {
    [0, 16, 64, 128, 512].iter().any(|h| size >= *h && sizes.contains(&(size - h)))
}

/// What a file holds: an archive's or disc image's members (a ZIP's by the CRCs it
/// records, D16), else the file itself; a file no ROM of `sizes` is as big as, only its
/// size (D133).
fn read_one(path: &Path, size: u64, sizes: Option<&HashSet<u64>>, cancel: &AtomicBool) -> std::io::Result<Vec<Hashes>> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(kind) = archive::container_of(&name) {
        match archive::read(path, kind, false, cancel) {
            Ok(l) => return Ok(l.members.into_iter().map(|m| m.hashes).collect()),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => return Err(e),
            Err(e) if kind == "rvz" && e.kind() == std::io::ErrorKind::NotFound => {
                return Err(std::io::Error::other("RVZ images need dolphin-tool (Settings › Tools)"));
            }
            // not an archive after all: an ordinary file
            Err(_) => {}
        }
    }
    if sizes.is_some_and(|s| !may_be(s, size)) {
        return Ok(vec![Hashes { size, ..Default::default() }]);
    }
    let mut f = std::fs::File::open(path)?;
    Ok(vec![hash::hash_reader(&mut f, size, &name, cancel)?])
}

/// Read the files under `paths`, several at a time: those that may be ROMs of `sizes` (the
/// audit DATs', `dat_sizes`; None for every file) for their hashes.
pub fn read_folders(paths: &[PathBuf], sizes: Option<&HashSet<u64>>, cancel: &AtomicBool, progress: &mut (dyn FnMut(Value) + Send)) -> Result<Vec<Found>> {
    if paths.is_empty() {
        bail!("Choose a folder (or a few) first.");
    }
    for p in paths {
        if !p.exists() {
            bail!("{} isn't there.", p.display());
        }
    }
    progress(json!({ "text": "Listing files" }));
    let files = walk(paths, cancel);
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<Found>>> = Mutex::new(vec![None; files.len()]);
    let progress = Mutex::new(progress);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 8);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= files.len() || cancel.load(Ordering::Relaxed) {
                    break;
                }
                let (path, shown, size) = &files[i];
                let mut f = Found { path: path.display().to_string(), shown: shown.clone(), size: *size, ..Default::default() };
                match read_one(path, *size, sizes, cancel) {
                    Ok(roms) => f.roms = roms,
                    Err(e) => f.error = Some(e.to_string()),
                }
                out.lock().unwrap()[i] = Some(f);
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                (progress.lock().unwrap())(json!({ "text": "Reading", "item": n - 1, "items": files.len(), "name": shown }));
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        bail!("Stopped.");
    }
    Ok(out.into_inner().unwrap().into_iter().flatten().collect())
}

/// The games of the DAT (or DATs in a ZIP) at `path`, each as what it holds.
pub fn read_dat(path: &Path) -> Result<Vec<Found>> {
    let mut out = vec![];
    for d in dat::read_file(path)? {
        for g in d.games {
            let roms: Vec<Hashes> = g
                .roms
                .iter()
                .filter(|r| r.status.as_deref() != Some("nodump"))
                .filter_map(|r| Some(Hashes { size: r.size?, crc32: r.crc32.clone()?, md5: r.md5.clone(), sha1: r.sha1.clone(), ..Default::default() }))
                .collect();
            out.push(Found { path: g.name.clone(), shown: g.name, size: roms.iter().map(|h| h.size).sum(), roms, error: None });
        }
    }
    Ok(out)
}

/// Each system's audit DATs (the BIOS files aside): its main DAT, the others included in
/// its audit (D118) and its custom DAT of variants (D119), as Audit has them (D140).
/// DAT id → system.
fn audit_dats(conn: &Connection, lib: &Library) -> Result<HashMap<i64, String>> {
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != ?1")?;
    let systems: Vec<String> = q.query_map([crate::bios::ID], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut out = HashMap::new();
    for p in systems {
        if let Some(id) = audit::primary_dat(conn, lib, &p)? {
            out.insert(id, p.clone());
            for a in audit::also_dats(conn, lib, &p)? {
                out.insert(a, p.clone());
            }
        }
        if let Some(c) = crate::variants::custom_dat(conn, &p)? {
            out.insert(c, p);
        }
    }
    Ok(out)
}

/// The games (DAT game ids) whose ROMs a file's hashes are, in the audit DATs: the
/// games most of its ROMs belong to (of those, the ones it holds all of). A SHA1 that
/// differs from the DAT's rules a CRC out.
fn games_of(conn: &Connection, dats: &HashMap<i64, String>, roms: &[Hashes]) -> Result<Vec<i64>> {
    let mut q = conn.prepare_cached("SELECT game_id, dat_id, sha1 FROM dat_roms WHERE crc32 = ?1 AND size = ?2")?;
    let mut count: HashMap<i64, usize> = HashMap::new();
    for h in roms {
        let mut hit: HashSet<i64> = HashSet::new();
        for x in std::iter::once(h).chain(h.hl.as_deref()).filter(|x| !x.crc32.is_empty()) {
            let mut rows = q.query(rusqlite::params![x.crc32.to_ascii_lowercase(), x.size as i64])?;
            while let Some(r) = rows.next()? {
                let (game, dat_id, sha1): (i64, i64, Option<String>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                if !dats.contains_key(&dat_id) {
                    continue;
                }
                if let (Some(a), Some(b)) = (&x.sha1, &sha1) {
                    if !a.eq_ignore_ascii_case(b) {
                        continue;
                    }
                }
                hit.insert(game);
            }
        }
        for g in hit {
            *count.entry(g).or_default() += 1;
        }
    }
    let best = count.values().copied().max().unwrap_or(0);
    let mut v: Vec<i64> = count.into_iter().filter(|(_, n)| *n == best).map(|(g, _)| g).collect();
    // a ROM two games share (a game and a set holding it too): the games it makes whole
    if v.len() > 1 {
        let mut roms_of = conn.prepare_cached("SELECT COUNT(*) FROM dat_roms WHERE game_id = ?1 AND COALESCE(status, '') <> 'nodump'")?;
        let mut whole = vec![];
        for g in &v {
            let n: i64 = roms_of.query_row([g], |r| r.get(0))?;
            if n as usize <= best {
                whole.push(*g);
            }
        }
        if !whole.is_empty() {
            v = whole;
        }
    }
    v.sort();
    Ok(v)
}

/// What was read, matched against the audit DATs: the games found (by system, with what
/// the full set has of each) and what matched nothing.
pub fn review(ix: &Index, lib: &Library, found: &[Found]) -> Result<Value> {
    let conn = ix.conn();
    let dats = audit_dats(conn, lib)?;
    // game id → the files (or DAT games) that are it
    let mut games: BTreeMap<i64, Vec<(String, String)>> = BTreeMap::new();
    let mut unknown = vec![];
    for f in found {
        if let Some(e) = &f.error {
            unknown.push(json!({ "path": f.shown, "why": format!("It couldn't be read: {e}") }));
            continue;
        }
        if f.roms.is_empty() {
            unknown.push(json!({ "path": f.shown, "why": "There's nothing in it." }));
            continue;
        }
        let ids = games_of(conn, &dats, &f.roms)?;
        if ids.is_empty() {
            // the system it may be, from the folders it's in and its extension
            let dirs: Vec<&str> = f.shown.split('/').collect();
            let system = platforms::guess(None, &dirs[..dirs.len().saturating_sub(1)], &f.shown).filter(|p| p != "pc");
            // (its hashes: imported into a collection, it waits on it, D140)
            let roms: Vec<WaitingRom> = f.roms.iter().map(|h| WaitingRom { size: h.size, crc32: h.crc32.clone(), sha1: h.sha1.clone() }).collect();
            unknown.push(json!({ "path": f.shown, "file": f.path, "system": system, "roms": roms, "why": "None of the DATs Audit uses knows it." }));
        }
        for id in ids {
            games.entry(id).or_default().push((f.shown.clone(), f.path.clone()));
        }
    }
    // review H5: a sheet no DAT knows (its tracks renamed) goes with the game whose tracks
    // its FILE lines name, as a scan pairs them
    let game_of: HashMap<String, i64> = games.iter().flat_map(|(id, fs)| fs.iter().map(move |(_, p)| (key(Path::new(p)), *id))).collect();
    unknown.retain(|u| {
        let (Some(path), Some(shown)) = (u["file"].as_str(), u["path"].as_str()) else { return true };
        if !crate::cue::is_sheet(path) {
            return true;
        }
        let Ok(text) = std::fs::read(path) else { return true };
        let dir = Path::new(path).parent().unwrap_or(Path::new(""));
        let gdi = platforms::ext_of(path) == "gdi";
        let named = crate::cue::file_names(&String::from_utf8_lossy(&text), gdi).into_iter().find_map(|n| game_of.get(&key(&dir.join(n))).copied());
        match named {
            Some(id) => {
                games.entry(id).or_default().push((shown.to_string(), path.to_string()));
                false
            }
            None => true,
        }
    });
    // each game's name, system and status in the full set: a system's DATs together
    let mut by_system: BTreeMap<String, BTreeMap<i64, HashSet<i64>>> = BTreeMap::new();
    for id in games.keys() {
        let d: i64 = conn.query_row("SELECT dat_id FROM dat_games WHERE id = ?1", [id], |r| r.get(0))?;
        by_system.entry(dats.get(&d).cloned().unwrap_or_default()).or_default().entry(d).or_default().insert(*id);
    }
    let mut systems = vec![];
    for (platform, by_dat) in by_system {
        let mut list = vec![];
        for (dat_id, wanted) in &by_dat {
            let mut q = conn.prepare_cached(&format!("WITH {GAMES} SELECT gs.id, gs.name, gs.status FROM gs ORDER BY gs.name"))?;
            let mut rows = q.query([dat_id])?;
            while let Some(r) = rows.next()? {
                let id: i64 = r.get(0)?;
                if wanted.contains(&id) {
                    let from: Vec<&str> = games[&id].iter().map(|(s, _)| s.as_str()).collect();
                    let files: Vec<&str> = games[&id].iter().map(|(_, p)| p.as_str()).collect();
                    list.push(json!({ "id": id, "name": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?, "from": from, "files": files, "dat": dat_id }));
                }
            }
        }
        list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        let have = list.iter().filter(|g| g["status"] == "have").count();
        let places = crate::addgame::places(Some(conn), lib, &platform);
        let main = audit::primary_dat(conn, lib, &platform)?.or(by_dat.keys().next().copied());
        systems.push(json!({ "platform": platform, "name": platforms::name_of(&platform), "dat": main, "games": list, "have": have, "places": places }));
    }
    systems.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(json!({
        "read": found.len(),
        "games": systems.iter().map(|s| s["games"].as_array().map(Vec::len).unwrap_or(0)).sum::<usize>(),
        "systems": systems,
        "unknown": unknown,
    }))
}

/// Where a collection's own DATs are kept.
pub fn dats_dir(lib: &Library, id: &str) -> Result<PathBuf> {
    collection::own_dir(lib, id, "")
}

/// What the games read from folders or a DAT are saved as.
pub enum Target<'a> {
    /// A new collection, by name
    New(&'a str),
    /// A collection's group (by collection id and group id; "" is the first) read again,
    /// whose DATs are replaced
    Again(&'a str, &'a str),
    /// A new group of a collection (by collection id and the group's name, D75)
    Group(&'a str, &'a str),
}

/// Save the games chosen (DAT game ids of the audit DATs) as a collection, or a group of
/// one, made from folders or a DAT, kept as DATs of its own. `made_from` says what it was
/// read from; `waiting`, the files imported into it that no DAT knows yet (D140).
pub fn save(ix: &Index, lib: &Library, into: Target, games: &[i64], made_from: Value, waiting: Vec<Waiting>) -> Result<Collection> {
    lib.writable()?;
    if games.is_empty() && waiting.is_empty() {
        bail!("None of the games was found in your audited games, so there's nothing to make a collection of.");
    }
    let conn = ix.conn();
    let (mut c, group) = match into {
        Target::Again(id, g) => (collection::load(lib, id)?, g.to_string()),
        Target::New(name) => (collection::save(lib, Collection { name: name.into(), ..Default::default() })?, String::new()),
        Target::Group(id, name) => {
            let mut c = collection::load(lib, id)?;
            let g = c.add_group(name, Rules::default());
            // (named and given an id that's its own)
            let c = collection::save(lib, c)?;
            let g = c.groups.iter().find(|x| x.id == g).or(c.groups.last()).map(|x| x.id.clone()).unwrap_or_default();
            (c, g)
        }
    };
    let group_name = c.group(&group).map(|g| g.name).unwrap_or_default();
    // the games, by system
    let mut q = conn.prepare("SELECT g.id, g.name, COALESCE(g.title, ''), COALESCE(g.regions, ''), d.platform FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1")?;
    let mut by: BTreeMap<String, Vec<Game>> = BTreeMap::new();
    for id in games {
        let (gid, gname, title, regions, platform): (i64, String, String, String, String) = q.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).with_context(|| format!("There's no DAT game {id}."))?;
        by.entry(platform.clone()).or_default().push(Game {
            id: gid, platform, name: gname, title, regions, status: String::new(), manual: false, size: 0, dat_size: 0, set: None,
        });
    }
    let dir = collection::own_dir(lib, &c.id, &group)?;
    // the DATs it had are replaced
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if e.path().extension().is_some_and(|x| x == "dat") {
            std::fs::remove_file(e.path())?;
        }
    }
    std::fs::create_dir_all(&dir)?;
    let mut files = vec![];
    for (platform, list) in &by {
        let sys = platforms::name_of(platform);
        let dname = if group_name.is_empty() || group_name == c.name { format!("{} - {sys}", c.name) } else { format!("{} - {group_name} - {sys}", c.name) };
        let refs: Vec<&Game> = list.iter().collect();
        let text = collection::dat_xml(conn, &dname, &format!("{} ({} games of {sys}, made by Game Repo)", c.name, list.len()), &refs)?;
        let p = dir.join(format!("{}.dat", collection::safe(&dname)));
        std::fs::write(&p, text).with_context(|| format!("couldn't write {}", p.display()))?;
        files.push(lib.relative(&p).unwrap_or_else(|| p.display().to_string()));
    }
    // the systems of the files waiting too; with none of its games known yet, only those
    // that join it later (added by hand)
    let mut systems: Vec<String> = by.keys().cloned().collect();
    for w in &waiting {
        if !systems.contains(&w.platform) {
            systems.push(w.platform.clone());
        }
    }
    systems.sort();
    let include = if by.is_empty() { "none" } else { "all" };
    c.change_group(&group, |g| {
        if !waiting.is_empty() {
            g.waiting = waiting;
        }
        g.rules = Rules {
            platforms: systems,
            include: include.into(),
            one_per_title: false,
            only_regions: false,
            languages: vec![],
            exclude: vec![],
            search: String::new(),
            from_dats: vec![],
            from_files: files,
            ..g.rules.clone()
        };
        g.made_from = Some(made_from);
    })?;
    collection::save(lib, c)
}

/// The files waiting on a collection (D140) that a DAT knows now (added to their system's
/// custom DAT, say) join it: each game is added to the group it waited on, as a step of the
/// collection's history. Returns how many joined.
pub fn settle(ix: &Index, lib: &Library, id: &str) -> Result<usize> {
    let c = collection::load(lib, id)?;
    if c.all_groups().iter().all(|g| g.waiting.is_empty()) || lib.writable().is_err() {
        return Ok(0);
    }
    let conn = ix.conn();
    let dats = audit_dats(conn, lib)?;
    let mut name_of = conn.prepare("SELECT d.platform, g.name FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1")?;
    // (group, the file, the games it is)
    let mut joined: Vec<(String, Waiting, Vec<(String, String)>)> = vec![];
    for g in c.all_groups() {
        for w in &g.waiting {
            let roms: Vec<Hashes> = w.roms.iter().map(|r| Hashes { size: r.size, crc32: r.crc32.clone(), sha1: r.sha1.clone(), ..Default::default() }).collect();
            let ids = games_of(conn, &dats, &roms)?;
            if ids.is_empty() {
                continue;
            }
            let games = ids.iter().map(|i| name_of.query_row([i], |r| Ok((r.get(0)?, r.get(1)?)))).collect::<rusqlite::Result<_>>()?;
            joined.push((g.id.clone(), w.clone(), games));
        }
    }
    if joined.is_empty() {
        return Ok(0);
    }
    let before = crate::history::before(ix, lib, id, "", "")?;
    let mut c = c;
    for (group, w, games) in &joined {
        c.change_group(group, |g| {
            g.waiting.retain(|x| x != w);
            for (platform, name) in games {
                let k = format!("{platform}/{name}");
                g.removed.retain(|x| *x != k);
                if !g.added.contains(&k) {
                    g.added.push(k);
                }
                if !g.rules.platforms.is_empty() && !g.rules.platforms.contains(platform) {
                    g.rules.platforms.push(platform.clone());
                }
            }
        })?;
    }
    collection::save(lib, c)?;
    crate::history::record(ix, lib, before, "hand", None)?;
    Ok(joined.len())
}

/// A file waiting on a collection's group (by its system and name) taken off: it stays in
/// the library, and the collection no longer waits for it (D140).
pub fn leave_waiting(lib: &Library, id: &str, group: &str, platform: &str, name: &str) -> Result<Collection> {
    lib.writable()?;
    let mut c = collection::load(lib, id)?;
    c.change_group(group, |g| g.waiting.retain(|w| !(w.platform == platform && w.name == name)))?;
    collection::save(lib, c)
}

/// One of a DAT game's ROMs.
struct DatRom {
    name: String,
    crc32: String,
    size: u64,
    sha1: Option<String>,
}

impl DatRom {
    fn is(&self, s: &organise::Src) -> bool {
        self.crc32.eq_ignore_ascii_case(&s.crc32) && self.size == s.size && !matches!((&self.sha1, &s.sha1), (Some(a), Some(b)) if !a.eq_ignore_ascii_case(b))
    }
}

/// Whether the file at `p` is a good copy of the game: its one ROM, or an archive holding all of them.
fn holds(p: &Path, roms: &[DatRom], cancel: &AtomicBool) -> bool {
    let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let have: Vec<Hashes> = if archive::container_of(&name).is_some() {
        archive::members(p, cancel).map(|m| m.into_iter().map(|(_, h)| h).collect()).unwrap_or_default()
    } else {
        std::fs::File::open(p).and_then(|mut f| hash::read_hashed(&mut f, false, cancel, &mut |_| Ok(()))).into_iter().collect()
    };
    !roms.is_empty() && roms.iter().all(|r| have.iter().any(|h| r.crc32.eq_ignore_ascii_case(&h.crc32) && r.size == h.size))
}

/// A game of several files (a disc's tracks and sheet, say) into a folder of its own named
/// for it, each named as its ROM (review M8): every step, or why none (a file in the way).
/// A sheet the DAT doesn't know (its tracks renamed) is written again for their new names
/// under the DAT's name for it (review H5).
#[allow(clippy::type_complexity)]
fn several(
    game: &str, place: &Path, files: &[PathBuf], roms: &[DatRom], outs: &mut HashSet<String>,
    src_of: &mut dyn FnMut(&Path) -> Result<organise::Src>, how: &dyn Fn(&Path, &Path) -> &'static str,
) -> Result<std::result::Result<Vec<organise::Op>, Value>> {
    let folder = place.join(organise::safe_name(game));
    let mut ops = vec![];
    let mut sheets = vec![];
    let mut renames: Vec<String> = vec![];
    let mut taken: Vec<String> = vec![];
    let in_way = |to: &Path, taken: &mut Vec<String>, outs: &HashSet<String>| -> Option<Value> {
        let k = key(to);
        if to.exists() || outs.contains(&k) || taken.contains(&k) {
            return Some(json!({ "path": format!("{game} ({} files)", files.len()), "why": format!("There's a file called {} in {} already, so none of the game's files are imported.", to.file_name().unwrap_or_default().to_string_lossy(), to.parent().unwrap_or(place).display()) }));
        }
        taken.push(k);
        None
    };
    for f in files {
        let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let src = src_of(f)?;
        let rom = roms.iter().find(|r| r.is(&src));
        if rom.is_none() && crate::cue::is_sheet(&name) {
            sheets.push(src);
            continue;
        }
        let named = rom.map(|r| r.name.rsplit(['/', '\\']).next().unwrap_or(&r.name).to_string()).unwrap_or_else(|| name.clone());
        let to = folder.join(organise::safe_name(&named));
        if let Some(why) = in_way(&to, &mut taken, outs) {
            return Ok(Err(why));
        }
        renames.extend([name, to.file_name().unwrap_or_default().to_string_lossy().into_owned()]);
        let size = src.size;
        ops.push(organise::Op { op: how(f, place).into(), game: game.into(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size, note: None });
    }
    for src in sheets {
        let own = Path::new(&src.path).file_name().unwrap_or_default().to_string_lossy().into_owned();
        let named = roms.iter().find(|r| crate::cue::is_sheet(&r.name)).map(|r| r.name.rsplit(['/', '\\']).next().unwrap_or(&r.name).to_string()).unwrap_or(own);
        let to = folder.join(organise::safe_name(&named));
        if let Some(why) = in_way(&to, &mut taken, outs) {
            return Ok(Err(why));
        }
        let aside = if how(Path::new(&src.path), place) == "move" { vec![src.clone()] } else { vec![] };
        let size = src.size;
        ops.push(organise::Op { op: "cue".into(), game: game.into(), to: to.display().to_string(), from: vec![src], names: renames.clone(), aside, size, note: Some("Its FILE lines are rewritten for the tracks' new names.".into()) });
    }
    outs.extend(taken);
    Ok(Ok(ops))
}

/// The files an import brings, by size: a file as big as one of them is read to see if
/// it's the same, so byte-identical copies are imported once; the rest aren't read (D133).
#[derive(Default)]
struct Seen {
    by_size: HashMap<u64, Vec<(PathBuf, Option<String>)>>,
}

impl Seen {
    /// Whether `p` is the first of its bytes.
    fn first(&mut self, p: &Path, size: u64, cancel: &AtomicBool) -> Result<bool> {
        let sha1 = |p: &Path| -> Result<Option<String>> { Ok(hash::identity(&mut std::fs::File::open(p)?, cancel).with_context(|| format!("couldn't read {}", p.display()))?.sha1) };
        let them = self.by_size.entry(size).or_default();
        if them.is_empty() {
            them.push((p.to_path_buf(), None));
            return Ok(true);
        }
        let mine = sha1(p)?;
        for (q, theirs) in them.iter_mut() {
            if theirs.is_none() {
                *theirs = sha1(q)?;
            }
            if *theirs == mine {
                return Ok(false);
            }
        }
        them.push((p.to_path_buf(), mine));
        Ok(true)
    }
}

/// What to import (D53, D73): the games found, the files no DAT knows that were given a
/// system, where each system's games go, and how.
#[derive(Debug, Default)]
pub struct Import {
    /// DAT game ids, with the files that were found to be them
    pub games: Vec<(i64, Vec<PathBuf>)>,
    /// Files no DAT knows, each with the system it was given: kept as they're named
    pub files: Vec<(String, Vec<PathBuf>)>,
    /// The folder each system's games go to; a system not named goes to the first of
    /// `addgame::places`
    pub places: HashMap<String, PathBuf>,
    /// copy (hardlinked on the same drive unless `always_copy`) or move
    pub mode: String,
    pub always_copy: bool,
}

/// Import games into the library (D53, D73): each game's files copied (hardlinked on the
/// same drive), or moved, into the folder chosen for its system, named as its audit DAT
/// names it, as an Organise change with the preview and Undo; a file no DAT knows keeps
/// its name. A folder chosen outside every folder with games becomes one for its system.
/// Copying never changes the folders read (D5); byte-identical copies of a file are
/// imported once.
pub fn bring_plan(ix: &Index, lib: &Library, imp: &Import, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<organise::Plan> {
    lib.writable()?;
    let conn = ix.conn();
    let mut q = conn.prepare("SELECT g.name, d.platform, (SELECT COUNT(*) FROM dat_roms r WHERE r.game_id = g.id AND COALESCE(r.status, '') <> 'nodump'), (SELECT r.name FROM dat_roms r WHERE r.game_id = g.id LIMIT 1) FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1")?;
    let (mut ops, mut left) = (vec![], vec![]);
    let mut seen = Seen::default();
    let mut outs: HashSet<String> = HashSet::new();
    let mut count = 0;
    let mut chosen: HashMap<String, Option<PathBuf>> = HashMap::new();
    let mut place_of = |platform: &str| -> Option<PathBuf> {
        chosen
            .entry(platform.to_string())
            .or_insert_with(|| {
                imp.places.get(platform).cloned().filter(|p| !p.as_os_str().is_empty()).or_else(|| {
                    crate::addgame::places(Some(conn), lib, platform).into_iter().find_map(|p| p["dir"].as_str().map(PathBuf::from))
                })
            })
            .clone()
    };
    let how = |from: &Path, place: &Path| match imp.mode.as_str() {
        "move" => "move",
        _ if !imp.always_copy && organise::same_drive(from, place) => "link",
        _ => "copy",
    };
    let nowhere = |platform: &str, game: &str| {
        let name = platforms::name_of(platform);
        json!({ "path": format!("{name} › {game}"), "why": format!("Choose where {name}'s games go.") })
    };
    let mut used: Vec<(PathBuf, String)> = vec![];
    // a file is read as it's copied, and checked by what was read then (D133); it's read
    // now (`hashed`) only to name a game's files as its ROMs
    let src_of = |f: &Path, hashed: bool, cancel: &AtomicBool| -> Result<organise::Src> {
        let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let size = std::fs::metadata(f).with_context(|| format!("couldn't read {}", f.display()))?.len();
        let h = match hashed {
            true => Some(hash::identity(&mut std::fs::File::open(f)?, cancel).with_context(|| format!("couldn't read {}", f.display()))?),
            false => None,
        };
        Ok(organise::Src {
            file_id: 0, root: format!("dir:{}", f.parent().unwrap_or(Path::new(".")).display()), rel: name, path: f.display().to_string(),
            member: None, size, crc32: h.as_ref().map(|h| h.crc32.clone()).unwrap_or_default(), sha1: h.and_then(|h| h.sha1),
        })
    };
    let items = imp.games.len() + imp.files.iter().map(|(_, f)| f.len()).sum::<usize>();
    let mut item = 0;
    let mut step = |progress: &mut dyn FnMut(Value)| -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            bail!("Stopped.");
        }
        progress(json!({ "text": "Checking the files", "item": item, "items": items }));
        item += 1;
        Ok(())
    };
    let mut roms_of = conn.prepare("SELECT name, crc32, size, sha1 FROM dat_roms WHERE game_id = ?1 AND COALESCE(status, '') <> 'nodump'")?;
    for (id, files) in &imp.games {
        step(progress)?;
        let (game, platform, roms, first_rom): (String, String, i64, Option<String>) = q.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).with_context(|| format!("There's no DAT game {id}."))?;
        let Some(place) = place_of(&platform) else {
            left.push(nowhere(&platform, &game));
            continue;
        };
        let dat_roms: Vec<DatRom> = roms_of
            .query_map([id], |r| Ok(DatRom { name: r.get(0)?, crc32: r.get::<_, Option<String>>(1)?.unwrap_or_default(), size: r.get::<_, Option<i64>>(2)?.unwrap_or(0) as u64, sha1: r.get(3)? }))?
            .collect::<rusqlite::Result<_>>()?;
        // one file of a game of one ROM, or an archive holding it; every file of a game of several
        let whole = roms <= 1 || files.iter().any(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| archive::container_of(n).is_some()));
        if !whole {
            // review M8: a folder of its own, and all of it or nothing
            match several(&game, &place, files, &dat_roms, &mut outs, &mut |f| src_of(f, true, cancel), &|f, p| how(f, p))? {
                Ok(game_ops) => {
                    ops.extend(game_ops);
                    used.push((place.clone(), platform.clone()));
                    count += 1;
                }
                Err(why) => left.push(why),
            }
            continue;
        }
        let mut brought = false;
        for f in files {
            if brought {
                break;
            }
            let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let ext = f.extension().map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase())).unwrap_or_default();
            let is_archive = archive::container_of(&name).is_some();
            if roms > 1 && !is_archive {
                continue;
            }
            let src = src_of(f, false, cancel)?;
            if !seen.first(f, src.size, cancel)? {
                continue;
            }
            // named as the DAT names it: the game (an archive or image) or its one ROM
            let named = match (is_archive, roms, &first_rom) {
                (true, _, _) => format!("{game}{ext}"),
                (false, 1, Some(rom)) => rom.clone(),
                _ => name.clone(),
            };
            let to = place.join(organise::safe_name(&named));
            let mut note = None;
            if !outs.insert(key(&to)) {
                left.push(json!({ "path": f.display().to_string(), "why": format!("Another file of these goes to {} too.", to.display()) }));
                continue;
            }
            if to.exists() {
                // review H6: a file there that isn't the DAT's (a bad dump) is set aside for it
                if holds(&to, &dat_roms, cancel) {
                    left.push(json!({ "path": f.display().to_string(), "why": format!("{} in {} is a good copy already.", to.file_name().unwrap_or_default().to_string_lossy(), place.display()) }));
                    continue;
                }
                let there = src_of(&to, false, cancel)?;
                ops.push(organise::Op { op: "aside".into(), game: game.clone(), to: to.display().to_string(), from: vec![there.clone()], names: vec![], aside: vec![], size: there.size, note: Some("It doesn't match the DAT: set aside for the good copy.".into()) });
                note = Some(format!("Replaces the {} there, which doesn't match the DAT.", to.file_name().unwrap_or_default().to_string_lossy()));
            }
            let size = src.size;
            ops.push(organise::Op { op: how(f, &place).into(), game: game.clone(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size, note });
            used.push((place.clone(), platform.clone()));
            brought = true;
        }
        if brought {
            count += 1;
        }
    }
    // files no DAT knows, in the system they were given (a sheet brings its tracks)
    for (platform, files) in &imp.files {
        if platforms::by_id(platform).is_none() || platform == "pc" {
            bail!("There's no system {platform} to import them into.");
        }
        for f in files {
            step(progress)?;
            let game = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let Some(place) = place_of(platform) else {
                left.push(nowhere(platform, &game));
                continue;
            };
            let mut brought = false;
            for p in crate::addgame::with_tracks(f) {
                let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let to = place.join(organise::safe_name(&name));
                if !p.is_file() {
                    continue;
                }
                if to.exists() || !outs.insert(key(&to)) {
                    left.push(json!({ "path": p.display().to_string(), "why": format!("There's a file called {name} in {} already.", place.display()) }));
                    continue;
                }
                let src = src_of(&p, false, cancel)?;
                if !seen.first(&p, src.size, cancel)? {
                    continue;
                }
                let size = src.size;
                ops.push(organise::Op { op: how(&p, &place).into(), game: game.clone(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size, note: None });
                used.push((place.clone(), platform.clone()));
                brought = true;
            }
            if brought {
                count += 1;
            }
        }
    }
    // a folder chosen outside the folders with games becomes one, for its system (or for
    // every system, when several systems' games go there)
    let rs = crate::roots::list(lib);
    let mut new_roots: Vec<[String; 2]> = vec![];
    for (dir, platform) in &used {
        if organise::in_root(&rs, &dir.join("x")).is_some() {
            continue;
        }
        if dir.starts_with(lib.root().join(crate::library::APP_DIR)) {
            bail!("Games can't go in the library's own _library folder.");
        }
        let d = dir.display().to_string();
        match new_roots.iter_mut().find(|[x, _]| *x == d) {
            Some(r) if r[1] != *platform => r[1] = String::new(),
            Some(_) => {}
            None => {
                // checked now, so the change can't stop part way when it adds the folder
                crate::roots::check_new(lib, dir).map_err(|e| anyhow!("{}'s games can't go to {}: {e:#}", platforms::name_of(platform), dir.display()))?;
                new_roots.push([d, platform.clone()]);
            }
        }
    }
    let places: HashSet<String> = ops.iter().filter_map(|o| Path::new(&o.to).parent().map(|p| p.display().to_string())).collect();
    let systems: HashSet<&str> = used.iter().map(|(_, p)| p.as_str()).collect();
    let s = if count == 1 { "" } else { "s" };
    Ok(organise::Plan {
        id: organise::new_id(), created: crate::library::now(), kind: "bring".into(),
        label: format!("Imported {count} game{s} into the library"),
        title: format!("Import {count} game{s} into the library"),
        dat: Value::Null, platform: if systems.len() == 1 { systems.into_iter().next().unwrap_or_default().to_string() } else { String::new() },
        mode: if imp.mode == "move" { "move".into() } else { "copy".into() }, format: "keep".into(),
        out: if places.len() == 1 { places.into_iter().next() } else { None },
        space: organise::space_needed(&ops), ops, games: count, in_place: 0, conflicts: vec![], duplicates: vec![], left, new_root: None, new_roots, present: vec![], export: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::rom;
    use crate::collection::tests::fixture;

    #[test]
    fn a_collection_is_made_from_a_folder_of_a_subset_and_from_a_dat_by_hash() {
        let (dir, lib, ix) = fixture("subset");
        // a subset folder from elsewhere: Alpha renamed (which the full set lacks), Delta
        // in a ZIP (which it has), Zeta (which it lacks), and a stranger
        let sub = dir.join("Elsewhere/Best of GB");
        std::fs::create_dir_all(sub.join("more")).unwrap();
        std::fs::write(sub.join("my alpha.gb"), rom(1, 1024)).unwrap();
        let z = std::fs::File::create(sub.join("more/Delta.zip")).unwrap();
        let mut zw = zip::ZipWriter::new(z);
        zw.start_file("Delta (World).gb", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut zw, &rom(8, 1024)).unwrap();
        zw.finish().unwrap();
        std::fs::write(sub.join("zeta.gb"), rom(10, 1024)).unwrap();
        std::fs::write(sub.join("notes.txt"), b"mine").unwrap();
        let none = AtomicBool::new(false);
        let sizes = dat_sizes(ix.conn(), &lib).unwrap();
        let found = read_folders(&[sub.clone()], Some(&sizes), &none, &mut |_| {}).unwrap();
        assert_eq!(found.len(), 4);
        // a file as big as no ROM of the DATs isn't read (D133)
        let notes = found.iter().find(|f| f.shown.ends_with("notes.txt")).unwrap();
        assert!(notes.roms.len() == 1 && notes.roms[0].crc32.is_empty() && notes.roms[0].size == 4, "{notes:?}");
        assert!(found.iter().filter(|f| f.shown.ends_with(".gb")).all(|f| !f.roms[0].crc32.is_empty()));
        let r = review(&ix, &lib, &found).unwrap();
        let gb = &r["systems"][0];
        let names: Vec<&str> = gb["games"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Alpha (USA)", "Delta (World)", "Zeta (Japan)"], "{r}");
        assert_eq!(gb["have"], 1);
        assert_eq!(r["unknown"][0]["path"], "Best of GB/notes.txt");
        // saved: its own DAT, which the collection takes its games from
        let ids: Vec<i64> = gb["games"].as_array().unwrap().iter().map(|g| g["id"].as_i64().unwrap()).collect();
        let c = save(&ix, &lib, Target::New("Best of GB"), &ids, json!({ "folders": [sub.display().to_string()] }), vec![]).unwrap();
        assert_eq!(c.rules.from_files.len(), 1);
        assert!(lib.root().join(&c.rules.from_files[0]).is_file());
        let w = collection::work_out(&ix, &lib, &c).unwrap();
        let mut got: Vec<String> = w.games().map(|g| g.name.clone()).collect();
        got.sort();
        assert_eq!(got, ["Alpha (USA)", "Delta (World)", "Zeta (Japan)"]);
        // a DAT from another tool, with its own names, finds the same games by hash
        let other = crate::audit::tests::dat_xml("My list", "1", &[
            ("alpha!", None, crate::audit::tests::rom_xml("a.gb", &rom(1, 1024))),
            ("mystery", None, crate::audit::tests::rom_xml("m.gb", &rom(77, 1024))),
        ]);
        std::fs::write(dir.join("mine.dat"), other).unwrap();
        let r = review(&ix, &lib, &read_dat(&dir.join("mine.dat")).unwrap()).unwrap();
        assert_eq!((r["games"].as_u64(), r["unknown"][0]["path"].as_str()), (Some(1), Some("mystery")), "{r}");
        // read again with fewer games: its DAT is replaced
        let c = save(&ix, &lib, Target::Again(&c.id, ""), &ids[..1], json!({ "dat": "mine.dat" }), vec![]).unwrap();
        assert_eq!(collection::work_out(&ix, &lib, &c).unwrap().games().count(), 1);
        // a group of it made from the folder: its own DATs in its own folder, named for it,
        // and the game the first group has isn't repeated in it
        let c = save(&ix, &lib, Target::Group(&c.id, "From the folder"), &ids, json!({ "folders": [sub.display().to_string()] }), vec![]).unwrap();
        let g = &c.groups[0];
        assert_eq!((g.name.as_str(), g.made_from.is_some()), ("From the folder", true));
        assert!(g.rules.from_files[0].contains(&format!("{}/{}/", c.id, g.id)) && g.rules.from_files[0].ends_with("Best of GB - From the folder - Game Boy.dat"), "{:?}", g.rules.from_files);
        assert_eq!(c.rules.from_files.len(), 1, "the first group's DATs as they were");
        let w = collection::work_out(&ix, &lib, &c).unwrap();
        assert_eq!((w.systems[0].games.len(), w.systems[1].games.len(), w.games().count()), (1, 2, 3));
        // a copy has its own DATs; deleting the first takes its DATs, not the copy's
        let copy = collection::save(&lib, Collection { id: String::new(), name: "Copy".into(), ..c.clone() }).unwrap();
        assert!(copy.rules.from_files[0].contains(&copy.id), "{:?}", copy.rules.from_files);
        assert!(copy.groups[0].rules.from_files[0].contains(&format!("{}/{}/", copy.id, copy.groups[0].id)), "{:?}", copy.groups[0].rules.from_files);
        assert!(lib.root().join(&copy.groups[0].rules.from_files[0]).is_file());
        collection::delete(&lib, &c.id).unwrap();
        assert!(!dats_dir(&lib, &c.id).unwrap().exists());
        assert_eq!(collection::work_out(&ix, &lib, &copy).unwrap().games().count(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_imported_into_a_collection_keeps_its_unknown_files_waiting_until_a_dat_knows_them() {
        use crate::audit::tests::{dat_xml, import, rom_xml, run_scan, write};
        let f = crate::audit::tests::fixture("subset-waiting");
        let gba = f.dir.join("Roms/GBA");
        write(&gba.join("Tetris Worlds (USA).gba"), &rom(44, 1024));
        write(&gba.join("Dark Eden (Prototype).gba"), &rom(42, 4096));
        let dat = dat_xml("Nintendo - Game Boy Advance", "20261001", &[
            ("Tetris Worlds (USA)", None, rom_xml("Tetris Worlds (USA).gba", &rom(44, 1024))),
            ("Zeta (Japan)", None, rom_xml("Zeta (Japan).gba", &rom(45, 2048))),
        ]);
        write(&f.dir.join("dats/gba.dat"), dat.as_bytes());
        run_scan(&f, false);
        let main = import(&f, "gba.dat")[0]["id"].as_i64().unwrap();
        // a folder of picks: one the library has, one it lacks, and one no DAT knows
        let picks = f.dir.join("Elsewhere/Picks");
        write(&picks.join("tetris.gba"), &rom(44, 1024));
        write(&picks.join("zeta.gba"), &rom(45, 2048));
        write(&picks.join("Dark Eden (Prototype).gba"), &rom(42, 4096));
        let none = AtomicBool::new(false);
        let found = read_folders(&[picks.clone()], None, &none, &mut |_| {}).unwrap();
        let r = review(&f.ix, &f.lib, &found).unwrap();
        let games: Vec<(&str, &str)> = r["systems"][0]["games"].as_array().unwrap().iter().map(|g| (g["name"].as_str().unwrap(), g["status"].as_str().unwrap())).collect();
        assert_eq!(games, [("Tetris Worlds (USA)", "have"), ("Zeta (Japan)", "missing")], "{r}");
        let eden = &r["unknown"][0];
        assert_eq!((eden["path"].as_str(), eden["roms"][0]["size"].as_u64()), (Some("Picks/Dark Eden (Prototype).gba"), Some(4096)), "{r}");
        // imported into a new collection: the games known, and the file no DAT knows waiting
        let ids: Vec<i64> = r["systems"][0]["games"].as_array().unwrap().iter().map(|g| g["id"].as_i64().unwrap()).collect();
        let waiting = vec![Waiting { platform: "gba".into(), name: "Dark Eden (Prototype).gba".into(), roms: serde_json::from_value(eden["roms"].clone()).unwrap() }];
        let c = save(&f.ix, &f.lib, Target::New("Picks"), &ids, json!({ "kind": "import", "paths": [picks] }), waiting.clone()).unwrap();
        crate::history::record(&f.ix, &f.lib, crate::history::new(&c), "made", None).unwrap();
        assert_eq!((collection::work_out(&f.ix, &f.lib, &c).unwrap().games().count(), c.waiting.len()), (2, 1));
        assert_eq!(settle(&f.ix, &f.lib, &c.id).unwrap(), 0, "no DAT knows it yet");
        // added to the custom DAT (Audit › Unknown), it joins the collection, as a step of its history
        let l = audit::list(&f.ix, &f.lib, main, "unknown", "", 0, 100).unwrap();
        let file = l["items"][0]["id"].as_i64().unwrap();
        crate::variants::add(&f.ix, &f.lib, "gba", &[crate::variants::Add { files: vec![file], name: "Dark Eden (Prototype)".into(), original: None }]).unwrap();
        assert_eq!(settle(&f.ix, &f.lib, &c.id).unwrap(), 1);
        let c = collection::load(&f.lib, &c.id).unwrap();
        let mut got: Vec<String> = collection::work_out(&f.ix, &f.lib, &c).unwrap().games().map(|g| g.name.clone()).collect();
        got.sort();
        assert_eq!((got.as_slice(), c.waiting.len()), (["Dark Eden (Prototype)", "Tetris Worlds (USA)", "Zeta (Japan)"].map(String::from).as_slice(), 0));
        let steps = crate::history::steps(&f.lib, &c.id).unwrap();
        assert_eq!((steps[0]["how"].as_str(), steps[0]["added"][0].as_str()), (Some("hand"), Some("gba/Dark Eden (Prototype)")), "{steps:?}");
        // From folders matches the custom DAT too now
        let r = review(&f.ix, &f.lib, &found).unwrap();
        assert_eq!((r["games"].as_u64(), r["unknown"].as_array().map(Vec::len)), (Some(3), Some(0)), "{r}");
        // a collection of files no DAT knows: none of its games yet, only those that join it
        let stranger = Waiting { platform: "gba".into(), name: "Stranger.gba".into(), roms: vec![WaitingRom { size: 512, crc32: "0badf00d".into(), sha1: None }] };
        let s = save(&f.ix, &f.lib, Target::New("Strangers"), &[], json!({ "kind": "import", "paths": [] }), vec![stranger]).unwrap();
        assert_eq!((s.rules.include.as_str(), s.rules.platforms.as_slice(), collection::work_out(&f.ix, &f.lib, &s).unwrap().games().count()), ("none", ["gba".to_string()].as_slice(), 0));
        // left: the collection no longer waits for it
        let s = leave_waiting(&f.lib, &s.id, "", "gba", "Stranger.gba").unwrap();
        assert!(s.waiting.is_empty());
        assert!(save(&f.ix, &f.lib, Target::New("Nothing"), &[], json!({}), vec![]).is_err());
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn the_games_the_full_set_lacks_are_brought_in_named_as_the_dat_says_once() {
        let (dir, lib, ix) = fixture("subset-bring");
        let sub = dir.join("Elsewhere/Picks");
        std::fs::create_dir_all(&sub).unwrap();
        // Alpha (USA) and Zeta aren't in the full set; Zeta is here twice
        std::fs::write(sub.join("alpha.gb"), rom(1, 1024)).unwrap();
        std::fs::write(sub.join("zeta.gb"), rom(10, 1024)).unwrap();
        std::fs::write(sub.join("zeta copy.gb"), rom(10, 1024)).unwrap();
        std::fs::write(sub.join("delta.gb"), rom(8, 1024)).unwrap();
        let none = AtomicBool::new(false);
        let r = review(&ix, &lib, &read_folders(&[sub.clone()], Some(&dat_sizes(ix.conn(), &lib).unwrap()), &none, &mut |_| {}).unwrap()).unwrap();
        let lacking: Vec<(i64, Vec<PathBuf>)> = r["systems"][0]["games"].as_array().unwrap().iter()
            .filter(|g| g["status"] == "missing")
            .map(|g| (g["id"].as_i64().unwrap(), g["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect()))
            .collect();
        assert_eq!(lacking.len(), 2, "{r}");
        let plan = bring_plan(&ix, &lib, &Import { games: lacking, ..Default::default() }, &none, &mut |_| {}).unwrap();
        let mut to: Vec<String> = plan.ops.iter().map(|o| Path::new(&o.to).file_name().unwrap().to_string_lossy().into_owned()).collect();
        to.sort();
        assert_eq!(to, ["Alpha (USA).gb", "Zeta (Japan).gb"]);
        assert_eq!(plan.games, 2);
        organise::run(&ix, &lib, &plan, &none, &mut |_| {}).unwrap();
        assert!(dir.join("Roms/Zeta (Japan).gb").is_file() && sub.join("zeta copy.gb").is_file(), "the folder read isn't changed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_systems_games_are_imported_into_its_own_folder_copied_or_moved() {
        let (dir, lib, ix) = fixture("subset-import-systems");
        let none = AtomicBool::new(false);
        // a Game Gear DAT, and a folder with games for every system that holds only Game Boy games
        let gg = vec![("Sonic (World)", None, crate::audit::tests::rom_xml("Sonic (World).gg", &rom(50, 1024)))];
        std::fs::write(dir.join("gg.dat"), crate::audit::tests::dat_xml("Sega - Game Gear", "1", &gg)).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("gg.dat"), &serde_json::Map::new()).unwrap();
        let champion = dir.join("Champion");
        std::fs::create_dir_all(&champion).unwrap();
        std::fs::write(champion.join("Delta (World).gb"), rom(8, 1024)).unwrap();
        crate::roots::add(&lib, &champion, None).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options::default(), &none, &mut |_| {}).unwrap();
        // a folder of both systems' games, and a file no DAT knows
        let src = dir.join("Downloads");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("sonic.gg"), rom(50, 1024)).unwrap();
        std::fs::write(src.join("zeta.gb"), rom(10, 1024)).unwrap();
        std::fs::write(src.join("homebrew.gg"), b"a game nobody listed").unwrap();
        let r = review(&ix, &lib, &read_folders(&[src.clone()], Some(&dat_sizes(ix.conn(), &lib).unwrap()), &none, &mut |_| {}).unwrap()).unwrap();
        let systems: Vec<&str> = r["systems"].as_array().unwrap().iter().map(|x| x["platform"].as_str().unwrap()).collect();
        assert_eq!(systems, ["gb", "gamegear"], "{r}");
        let unknown = &r["unknown"][0];
        assert_eq!((unknown["system"].as_str(), unknown["file"].as_str()), (Some("gamegear"), src.join("homebrew.gg").to_str()));
        // the Game Boy's games go to its folder; the Game Gear has none, and the folder for
        // every system holds only the Game Boy's games, so it isn't offered: a new folder
        // named for it is, beside the Game Boy's
        let gb = r["systems"].as_array().unwrap().iter().find(|x| x["platform"] == "gb").unwrap();
        let gg = r["systems"].as_array().unwrap().iter().find(|x| x["platform"] == "gamegear").unwrap();
        assert_eq!(gb["places"][0]["dir"].as_str(), dir.join("Roms").to_str());
        let new_gg = dir.join("Sega Game Gear");
        assert_eq!(gg["places"].as_array().unwrap().iter().map(|p| p["dir"].as_str().unwrap()).collect::<Vec<_>>(), [new_gg.to_str().unwrap()]);
        let games: Vec<(i64, Vec<PathBuf>)> = [gb, gg].iter().flat_map(|x| x["games"].as_array().unwrap().iter())
            .filter(|g| g["status"] == "missing")
            .map(|g| (g["id"].as_i64().unwrap(), g["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect()))
            .collect();
        let files = vec![("gamegear".to_string(), vec![src.join("homebrew.gg")])];
        let made = bring_plan(&ix, &lib, &Import { games: games.clone(), files: files.clone(), ..Default::default() }, &none, &mut |_| {}).unwrap();
        assert!(made.left.is_empty(), "{:?}", made.left);
        assert!(made.ops.iter().all(|o| !o.to.contains("Champion")));
        assert!(made.ops.iter().any(|o| Path::new(&o.to) == new_gg.join("Sonic (World).gg")), "{:?}", made.ops);
        assert_eq!(made.new_roots, [[new_gg.display().to_string(), "gamegear".to_string()]]);
        // a folder chosen for it becomes a folder with games for the Game Gear; moved, not copied
        let gg_dir = dir.join("Games/Game Gear");
        let imp = Import { games, files, places: [("gamegear".to_string(), gg_dir.clone())].into(), mode: "move".into(), ..Default::default() };
        let plan = bring_plan(&ix, &lib, &imp, &none, &mut |_| {}).unwrap();
        let mut to: Vec<String> = plan.ops.iter().map(|o| o.to.replace('\\', "/")).collect();
        to.sort();
        let at = |p: &Path| p.to_string_lossy().replace('\\', "/");
        assert_eq!(to, [at(&gg_dir.join("Sonic (World).gg")), at(&gg_dir.join("homebrew.gg")), at(&dir.join("Roms/Zeta (Japan).gb"))]);
        assert!(plan.ops.iter().all(|o| o.op == "move") && plan.games == 3);
        assert_eq!(plan.new_roots, [[gg_dir.display().to_string(), "gamegear".to_string()]]);
        organise::run(&ix, &lib, &plan, &none, &mut |_| {}).unwrap();
        assert!(gg_dir.join("Sonic (World).gg").is_file() && !src.join("sonic.gg").exists());
        let added = crate::roots::list(&lib).into_iter().find(|r| r.dir == gg_dir).expect("a folder with games for the Game Gear");
        assert_eq!(added.platform.as_deref(), Some("gamegear"));
        // Undo puts them back and takes the new folder away
        organise::undo(&ix, &lib, &plan.id, &none, &mut |_| {}).unwrap();
        assert!(src.join("sonic.gg").is_file() && !gg_dir.exists());
        assert!(!crate::roots::list(&lib).iter().any(|r| r.dir == gg_dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // review H5, M8 and H6
    #[test]
    fn a_disc_comes_in_whole_in_its_own_folder_with_its_sheet_and_a_bad_dump_is_replaced() {
        let (dir, lib, ix) = fixture("subset-disc");
        let none = AtomicBool::new(false);
        let (t1, t2) = (rom(60, 2352 * 4), rom(61, 2352 * 2));
        let sheet = |a: &str, b: &str| format!("FILE \"{a}\" BINARY\r\n  TRACK 01 MODE1/2352\r\n    INDEX 01 00:00:00\r\nFILE \"{b}\" BINARY\r\n  TRACK 02 AUDIO\r\n    INDEX 01 00:00:00\r\n");
        let dat_cue = sheet("Disc (USA) (Track 1).bin", "Disc (USA) (Track 2).bin");
        let x = crate::audit::tests::rom_xml;
        let games = vec![("Disc (USA)", None, x("Disc (USA).cue", dat_cue.as_bytes()) + &x("Disc (USA) (Track 1).bin", &t1) + &x("Disc (USA) (Track 2).bin", &t2))];
        std::fs::write(dir.join("psx.dat"), crate::audit::tests::dat_xml("Sony - PlayStation", "1", &games)).unwrap();
        crate::dat::import(&ix, &lib, &dir.join("psx.dat"), &serde_json::Map::new()).unwrap();
        // the disc renamed: its tracks match the DAT, its sheet (naming them as they are now) doesn't
        let src = dir.join("Downloads/My Disc");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("t1.bin"), &t1).unwrap();
        std::fs::write(src.join("t2.bin"), &t2).unwrap();
        std::fs::write(src.join("mine.cue"), sheet("t1.bin", "t2.bin")).unwrap();
        let r = review(&ix, &lib, &read_folders(&[src.clone()], Some(&dat_sizes(ix.conn(), &lib).unwrap()), &none, &mut |_| {}).unwrap()).unwrap();
        assert_eq!(r["unknown"], json!([]), "the sheet goes with its tracks: {r}");
        let g = &r["systems"][0]["games"][0];
        assert_eq!(g["files"].as_array().map(Vec::len), Some(3), "{r}");
        let games: Vec<(i64, Vec<PathBuf>)> = vec![(g["id"].as_i64().unwrap(), g["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect())];
        let place = dir.join("Games/PSX");
        let imp = Import { games: games.clone(), places: [("psx".to_string(), place.clone())].into(), ..Default::default() };
        // a file in the way: none of the disc comes in
        let folder = place.join("Disc (USA)");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Disc (USA) (Track 2).bin"), b"in the way").unwrap();
        let blocked = bring_plan(&ix, &lib, &imp, &none, &mut |_| {}).unwrap();
        assert_eq!((blocked.ops.len(), blocked.games, blocked.left.len()), (0, 0, 1), "{:?}", blocked.ops);
        std::fs::remove_file(folder.join("Disc (USA) (Track 2).bin")).unwrap();
        let plan = bring_plan(&ix, &lib, &imp, &none, &mut |_| {}).unwrap();
        assert_eq!(plan.games, 1);
        organise::run(&ix, &lib, &plan, &none, &mut |_| {}).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&folder).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["Disc (USA) (Track 1).bin", "Disc (USA) (Track 2).bin", "Disc (USA).cue"]);
        assert_eq!(std::fs::read_to_string(folder.join("Disc (USA).cue")).unwrap(), dat_cue, "the sheet is the DAT's again");
        // a bad dump where a game goes is set aside for the good copy
        let roms = dir.join("Roms");
        std::fs::write(roms.join("Zeta (Japan).gb"), b"a bad dump").unwrap();
        let picks = dir.join("Picks");
        std::fs::create_dir_all(&picks).unwrap();
        std::fs::write(picks.join("zeta.gb"), rom(10, 1024)).unwrap();
        let r = review(&ix, &lib, &read_folders(&[picks.clone()], None, &none, &mut |_| {}).unwrap()).unwrap();
        let z = &r["systems"][0]["games"][0];
        let imp = Import { games: vec![(z["id"].as_i64().unwrap(), vec![picks.join("zeta.gb")])], ..Default::default() };
        let plan = bring_plan(&ix, &lib, &imp, &none, &mut |_| {}).unwrap();
        assert_eq!(plan.ops.iter().map(|o| o.op.as_str()).collect::<Vec<_>>(), ["aside", "link"], "{:?}", plan.left);
        organise::run(&ix, &lib, &plan, &none, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(roms.join("Zeta (Japan).gb")).unwrap(), rom(10, 1024));
        // a good copy there is left alone
        assert!(bring_plan(&ix, &lib, &imp, &none, &mut |_| {}).unwrap().ops.is_empty());
        organise::undo(&ix, &lib, &plan.id, &none, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(roms.join("Zeta (Japan).gb")).unwrap(), b"a bad dump");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
