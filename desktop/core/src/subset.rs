//! Collections made from folders or from a DAT (docs/PLAN.md, "Collections rework
//! design", D49): the files of a few folders from anywhere, or the games of a DAT read in
//! place, matched by hash against each system's audit DAT, reviewed, then kept as DATs
//! of the collection's own (`_library/collections/<id>/`, one per system, with the full
//! set's names and hashes) that its rules take their games from.

use crate::archive;
use crate::audit::{self, GAMES};
use crate::collection::{self, Collection, Game, Rules};
use crate::dat;
use crate::hash::{self, Hashes};
use crate::index::Index;
use crate::library::{Library, APP_DIR};
use crate::organise::{self, key};
use crate::platforms;
use anyhow::{bail, Context, Result};
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

/// What a file holds: an archive's or disc image's members (a ZIP's by the CRCs it
/// records, D16), else the file itself.
fn read_one(path: &Path, size: u64, cancel: &AtomicBool) -> std::io::Result<Vec<Hashes>> {
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
    let mut f = std::fs::File::open(path)?;
    Ok(vec![hash::hash_reader(&mut f, size, &name, cancel)?])
}

/// Read the files under `paths`, several at a time.
pub fn read_folders(paths: &[PathBuf], cancel: &AtomicBool, progress: &mut (dyn FnMut(Value) + Send)) -> Result<Vec<Found>> {
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
                match read_one(path, *size, cancel) {
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

/// Each system's audit DAT (the BIOS files aside): DAT id → system.
fn audit_dats(conn: &Connection, lib: &Library) -> Result<HashMap<i64, String>> {
    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != ?1")?;
    let systems: Vec<String> = q.query_map([crate::bios::ID], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let mut out = HashMap::new();
    for p in systems {
        if let Some(id) = audit::primary_dat(conn, lib, &p)? {
            out.insert(id, p);
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
        for x in std::iter::once(h).chain(h.hl.as_deref()) {
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
            unknown.push(json!({ "path": f.shown, "why": "No system's audit DAT has it." }));
        }
        for id in ids {
            games.entry(id).or_default().push((f.shown.clone(), f.path.clone()));
        }
    }
    // each game's name, system and status in the full set
    let mut by_dat: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for id in games.keys() {
        let d: i64 = conn.query_row("SELECT dat_id FROM dat_games WHERE id = ?1", [id], |r| r.get(0))?;
        by_dat.entry(d).or_default().push(*id);
    }
    let mut systems = vec![];
    for (dat_id, ids) in by_dat {
        let platform = dats.get(&dat_id).cloned().unwrap_or_default();
        let wanted: HashSet<i64> = ids.into_iter().collect();
        let mut q = conn.prepare(&format!("WITH {GAMES} SELECT gs.id, gs.name, gs.status FROM gs ORDER BY gs.name"))?;
        let mut rows = q.query([dat_id])?;
        let mut list = vec![];
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            if wanted.contains(&id) {
                let from: Vec<&str> = games[&id].iter().map(|(s, _)| s.as_str()).collect();
                let files: Vec<&str> = games[&id].iter().map(|(_, p)| p.as_str()).collect();
                list.push(json!({ "id": id, "name": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?, "from": from, "files": files }));
            }
        }
        let have = list.iter().filter(|g| g["status"] == "have").count();
        systems.push(json!({ "platform": platform, "name": platforms::name_of(&platform), "dat": dat_id, "games": list, "have": have }));
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
    crate::library::valid_id(id)?;
    Ok(lib.root().join(APP_DIR).join("collections").join(id))
}

/// Save the games chosen (DAT game ids of the audit DATs) as a collection made from
/// folders or a DAT: a new one named `name`, or (`id`) the one read again, whose DATs are
/// replaced. `made_from` says what it was read from.
pub fn save(ix: &Index, lib: &Library, name: &str, id: Option<&str>, games: &[i64], made_from: Value) -> Result<Collection> {
    lib.writable()?;
    if games.is_empty() {
        bail!("None of the games was found in your audit DATs, so there's nothing to make a collection of.");
    }
    let conn = ix.conn();
    let mut c = match id {
        Some(id) => collection::load(lib, id)?,
        None => collection::save(lib, Collection {
            id: String::new(), name: name.into(), rules: Rules::default(), added: vec![], removed: vec![], size_limit: None,
            made_from: None, created: String::new(), modified: String::new(),
        })?,
    };
    // the games, by system
    let mut q = conn.prepare("SELECT g.id, g.name, COALESCE(g.title, ''), COALESCE(g.regions, ''), d.platform FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1")?;
    let mut by: BTreeMap<String, Vec<Game>> = BTreeMap::new();
    for id in games {
        let (gid, gname, title, regions, platform): (i64, String, String, String, String) = q.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).with_context(|| format!("There's no DAT game {id}."))?;
        by.entry(platform.clone()).or_default().push(Game {
            id: gid, platform, name: gname, title, regions, status: String::new(), manual: false, size: 0, dat_size: 0,
        });
    }
    let dir = dats_dir(lib, &c.id)?;
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
        let dname = format!("{} - {sys}", c.name);
        let refs: Vec<&Game> = list.iter().collect();
        let text = collection::dat_xml(conn, &dname, &format!("{} ({} games of {sys}, made by Game Repo)", c.name, list.len()), &refs)?;
        let p = dir.join(format!("{}.dat", collection::safe(&dname)));
        std::fs::write(&p, text).with_context(|| format!("couldn't write {}", p.display()))?;
        files.push(lib.relative(&p).unwrap_or_else(|| p.display().to_string()));
    }
    c.rules = Rules {
        platforms: by.keys().cloned().collect(),
        include: "all".into(),
        one_per_title: false,
        only_regions: false,
        languages: vec![],
        exclude: vec![],
        search: String::new(),
        from_dats: vec![],
        from_files: files,
        ..c.rules
    };
    c.made_from = Some(made_from);
    collection::save(lib, c)
}

/// Bring into the library the games of a subset its full set lacks (D53): each game's
/// files copied (hardlinked on the same drive) into the first folder with games for its
/// system, named as the audit DAT names it, as an Organise change with the preview and
/// Undo. `games`: DAT game ids with the files that were found to be them. The folders
/// read are never changed (D5); byte-identical copies of a file are brought in once.
pub fn bring_plan(ix: &Index, lib: &Library, games: &[(i64, Vec<PathBuf>)], cancel: &AtomicBool) -> Result<organise::Plan> {
    lib.writable()?;
    let conn = ix.conn();
    let mut q = conn.prepare("SELECT g.name, d.platform, (SELECT COUNT(*) FROM dat_roms r WHERE r.game_id = g.id AND COALESCE(r.status, '') <> 'nodump'), (SELECT r.name FROM dat_roms r WHERE r.game_id = g.id LIMIT 1) FROM dat_games g JOIN dats d ON d.id = g.dat_id WHERE g.id = ?1")?;
    let (mut ops, mut left) = (vec![], vec![]);
    let mut seen = HashSet::new();
    let mut outs: HashSet<String> = HashSet::new();
    let mut count = 0;
    for (id, files) in games {
        let (game, platform, roms, first_rom): (String, String, i64, Option<String>) = q.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).with_context(|| format!("There's no DAT game {id}."))?;
        let Some(place) = crate::addgame::places(lib, &platform).into_iter().find_map(|p| p["path"].as_str().map(PathBuf::from)) else {
            left.push(json!({ "path": format!("{} › {game}", platforms::name_of(&platform)), "why": format!("There's no folder with games for {} to bring it into: add one on the Audit page.", platforms::name_of(&platform)) }));
            continue;
        };
        // one file of a game of one ROM, or an archive holding it; every file of a game of several
        let whole = roms <= 1 || files.iter().any(|f| f.file_name().and_then(|n| n.to_str()).is_some_and(|n| archive::container_of(n).is_some()));
        let mut brought = false;
        for f in files {
            if brought && whole {
                break;
            }
            let name = f.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let ext = f.extension().map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase())).unwrap_or_default();
            let is_archive = archive::container_of(&name).is_some();
            if whole && roms > 1 && !is_archive {
                continue;
            }
            let mut r = std::fs::File::open(f).with_context(|| format!("couldn't read {}", f.display()))?;
            let size = r.metadata()?.len();
            let h = hash::hash_reader(&mut r, size, "", cancel)?;
            if !seen.insert(h.sha1.clone().unwrap_or_else(|| format!("{}:{size}", h.crc32))) {
                continue;
            }
            // named as the DAT names it: the game (an archive or image) or its one ROM
            let named = match (is_archive, roms, &first_rom) {
                (true, _, _) => format!("{game}{ext}"),
                (false, 1, Some(rom)) => rom.clone(),
                _ => name.clone(),
            };
            let to = place.join(organise::safe_name(&named));
            if to.exists() || !outs.insert(key(&to)) {
                left.push(json!({ "path": f.display().to_string(), "why": format!("There's a file called {} in {} already.", to.file_name().unwrap_or_default().to_string_lossy(), place.display()) }));
                continue;
            }
            let src = organise::Src {
                file_id: 0, root: format!("dir:{}", f.parent().unwrap_or(Path::new(".")).display()), rel: name.clone(), path: f.display().to_string(),
                member: None, size, crc32: h.crc32, sha1: h.sha1,
            };
            let op = if organise::same_drive(f, &place) { "link" } else { "copy" };
            ops.push(organise::Op { op: op.into(), game: game.clone(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size, note: None });
            brought = true;
        }
        if brought {
            count += 1;
        }
    }
    let places: HashSet<String> = ops.iter().filter_map(|o| Path::new(&o.to).parent().map(|p| p.display().to_string())).collect();
    Ok(organise::Plan {
        id: organise::new_id(), created: crate::library::now(), kind: "bring".into(),
        label: format!("Brought {count} game{} into the library", if count == 1 { "" } else { "s" }),
        title: format!("Bring {count} game{} into the library", if count == 1 { "" } else { "s" }),
        dat: Value::Null, platform: String::new(), mode: "copy".into(), format: "keep".into(),
        out: if places.len() == 1 { places.into_iter().next() } else { None },
        space: organise::space_needed(&ops), ops, games: count, in_place: 0, conflicts: vec![], duplicates: vec![], left, new_root: None, present: vec![], export: None,
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
        let found = read_folders(&[sub.clone()], &none, &mut |_| {}).unwrap();
        assert_eq!(found.len(), 4);
        let r = review(&ix, &lib, &found).unwrap();
        let gb = &r["systems"][0];
        let names: Vec<&str> = gb["games"].as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Alpha (USA)", "Delta (World)", "Zeta (Japan)"], "{r}");
        assert_eq!(gb["have"], 1);
        assert_eq!(r["unknown"][0]["path"], "Best of GB/notes.txt");
        // saved: its own DAT, which the collection takes its games from
        let ids: Vec<i64> = gb["games"].as_array().unwrap().iter().map(|g| g["id"].as_i64().unwrap()).collect();
        let c = save(&ix, &lib, "Best of GB", None, &ids, json!({ "folders": [sub.display().to_string()] })).unwrap();
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
        let c = save(&ix, &lib, "", Some(&c.id), &ids[..1], json!({ "dat": "mine.dat" })).unwrap();
        assert_eq!(collection::work_out(&ix, &lib, &c).unwrap().games().count(), 1);
        // a copy has its own DATs; deleting the first takes its DATs, not the copy's
        let copy = collection::save(&lib, Collection { id: String::new(), name: "Copy".into(), ..c.clone() }).unwrap();
        assert!(copy.rules.from_files[0].contains(&copy.id), "{:?}", copy.rules.from_files);
        collection::delete(&lib, &c.id).unwrap();
        assert!(!dats_dir(&lib, &c.id).unwrap().exists());
        assert_eq!(collection::work_out(&ix, &lib, &copy).unwrap().games().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
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
        let r = review(&ix, &lib, &read_folders(&[sub.clone()], &none, &mut |_| {}).unwrap()).unwrap();
        let lacking: Vec<(i64, Vec<PathBuf>)> = r["systems"][0]["games"].as_array().unwrap().iter()
            .filter(|g| g["status"] == "missing")
            .map(|g| (g["id"].as_i64().unwrap(), g["files"].as_array().unwrap().iter().map(|f| PathBuf::from(f.as_str().unwrap())).collect()))
            .collect();
        assert_eq!(lacking.len(), 2, "{r}");
        let plan = bring_plan(&ix, &lib, &lacking, &none).unwrap();
        let mut to: Vec<String> = plan.ops.iter().map(|o| Path::new(&o.to).file_name().unwrap().to_string_lossy().into_owned()).collect();
        to.sort();
        assert_eq!(to, ["Alpha (USA).gb", "Zeta (Japan).gb"]);
        assert_eq!(plan.games, 2);
        organise::run(&ix, &lib, &plan, &none, &mut |_| {}).unwrap();
        assert!(dir.join("Roms/Zeta (Japan).gb").is_file() && sub.join("zeta copy.gb").is_file(), "the folder read isn't changed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
