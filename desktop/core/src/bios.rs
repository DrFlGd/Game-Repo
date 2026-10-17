//! BIOS files (docs/PLAN.md, "Phase 5b design", BIOS files, D40, D41): each system's
//! BIOS and firmware files as a set of their own. What a system needs comes from a BIOS
//! DAT (libretro's `System.dat` by default), kept as a DAT of the system `bios` whose
//! games are the systems; the files are found by hash anywhere in the folders with
//! games, audited, gathered into the library's `BIOS/<system>/` (as an Organise copy),
//! and exported with a collection into the layout's BIOS folder.

use crate::dat::{Dat, Game};
use crate::index::Index;
use crate::library::Library;
use crate::organise::{self, Plan};
use crate::platforms;
use crate::roots;
use anyhow::{anyhow, bail, Result};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// The system id BIOS DATs are kept under.
pub const ID: &str = "bios";

/// Where libretro keeps its BIOS DAT (Get starter DATs offers it beside the game DATs).
pub const SOURCE: &str = "https://raw.githubusercontent.com/libretro/libretro-database/master/dat/System.dat";

/// The library's folder for the BIOS set.
pub const FOLDER: &str = "BIOS";

/// libretro's names for systems whose DAT names differ from No-Intro's and Redump's, and
/// the systems that share a BIOS (a PC Engine's System Card is the CD's).
const ALIASES: &[(&str, &[&str])] = &[
    ("Nintendo - Gameboy", &["gb"]),
    ("Nintendo - Gameboy Color", &["gbc"]),
    ("Nintendo - Super Game Boy", &["snes"]),
    ("Nintendo - Nintendo 64DD", &["n64"]),
    ("Nintendo - Famicom Disk System", &["fds"]),
    ("Nintendo - GameCube", &["gc"]),
    ("NEC - PC Engine - TurboGrafx 16 - SuperGrafx", &["pcengine", "pcenginecd"]),
    ("NEC - PC-FX", &["pcfx"]),
    ("Sega - Mega CD - Sega CD", &["segacd"]),
    ("Sega - Mega Drive - Genesis", &["genesis"]),
    ("Sega - Master System - Mark III", &["mastersystem"]),
    ("Sega - Saturn", &["saturn"]),
    ("Sega - Dreamcast", &["dreamcast"]),
    ("SNK - NeoGeo CD", &["neogeocd"]),
    ("3DO Company, The - 3DO", &["3do"]),
    ("Philips - CD-i", &["cdimono1"]),
    ("Atari - 5200", &["atari5200"]),
    ("Atari - 7800", &["atari7800"]),
    ("Atari - Lynx", &["atarilynx"]),
    ("Sony - PlayStation", &["psx"]),
    ("Sony - PlayStation 2", &["ps2"]),
    ("Sony - PlayStation Portable", &["psp"]),
];

/// The systems a BIOS DAT's game (a system's name) is for: libretro's names, then the
/// DAT names and folder names systems are known by.
pub fn systems_of(name: &str) -> Vec<String> {
    if let Some((_, ids)) = ALIASES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return ids.iter().map(|s| s.to_string()).collect();
    }
    if let Some(p) = platforms::from_dat_name(name).or_else(|| platforms::from_folder(name)) {
        if p.id != ID {
            return vec![p.id.to_string()];
        }
    }
    if platforms::by_id(name).is_some_and(|p| p.id != ID) {
        return vec![name.to_string()];
    }
    vec![]
}

/// libretro's `System.dat` is one game ("System") with a comment before each system's
/// files: it's read as a game per system. Any other DAT is as it is.
pub fn by_system(text: &str, dat: Dat) -> Dat {
    if dat.games.len() != 1 {
        return dat;
    }
    // each ROM line's system, in order
    let mut section = String::new();
    let mut sections: Vec<String> = vec![];
    let mut in_game = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("game (") || line == "game" {
            in_game = true;
            continue;
        }
        if !in_game {
            continue;
        }
        if let Some(c) = line.strip_prefix("comment ") {
            section = c.trim().trim_matches('"').to_string();
        } else if line.starts_with("rom (") || line.starts_with("rom(") {
            sections.push(section.clone());
        }
    }
    let game = &dat.games[0];
    if sections.len() != game.roms.len() || sections.iter().all(|s| s == &game.name) {
        return dat;
    }
    let mut games: Vec<Game> = vec![];
    for (r, s) in game.roms.iter().zip(&sections) {
        let name = if s.is_empty() { game.name.clone() } else { s.clone() };
        match games.iter_mut().find(|g| g.name == name) {
            Some(g) => g.roms.push(r.clone()),
            None => games.push(Game { name: name.clone(), description: name, roms: vec![r.clone()], ..Default::default() }),
        }
    }
    Dat { games, ..dat }
}

/// The current BIOS DATs.
fn dats(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut q = conn.prepare("SELECT id, name FROM dats WHERE platform = ?1 AND current = 1 ORDER BY name")?;
    let v = q.query_map([ID], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    Ok(v)
}

struct Found {
    path: PathBuf,
    shown: String,
    loose: bool,
}

/// Where each of a BIOS DAT's files is: its ROM id, and the files that match it.
fn found(conn: &Connection, lib: &Library, dat_id: i64) -> Result<HashMap<i64, Vec<Found>>> {
    let dirs: HashMap<String, PathBuf> = roots::list(lib).into_iter().map(|r| (r.id.clone(), r.dir.clone())).collect();
    let mut q = conn.prepare(
        "SELECT m.rom_id, f.root_id, f.rel_path, f.parent_id IS NULL, f.name FROM matches m
           JOIN dat_roms r ON r.id = m.rom_id JOIN files f ON f.id = m.file_id
          WHERE r.dat_id = ?1 AND f.missing_since IS NULL ORDER BY f.parent_id IS NOT NULL, f.rel_path",
    )?;
    let mut out: HashMap<i64, Vec<Found>> = HashMap::new();
    let rows: Vec<(i64, String, String, bool, String)> = q.query_map([dat_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<rusqlite::Result<_>>()?;
    for (rom, root, rel, loose, name) in rows {
        let Some(base) = dirs.get(&root) else { continue };
        let member = (!loose).then_some(name.as_str());
        let rel_fs = match member {
            Some(m) => rel.strip_suffix(&format!("/{m}")).unwrap_or(&rel).to_string(),
            None => rel.clone(),
        };
        let path = rel_fs.split('/').fold(base.clone(), |p, part| p.join(part));
        let shown = match member {
            Some(m) => format!("{} › {m}", path.display()),
            None => path.display().to_string(),
        };
        out.entry(rom).or_default().push(Found { path, shown, loose });
    }
    Ok(out)
}

/// The BIOS page: each system of each BIOS DAT with its files as *have*, *missing* or
/// *wrong* (a file of its name that isn't it), the systems the library has games for
/// first, and those missing files first among them.
pub fn summary(ix: &Index, lib: &Library) -> Result<Value> {
    let conn = ix.conn();
    let with_games: Vec<String> = {
        let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != ?1")?;
        let v = q.query_map([ID], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        v
    };
    let mut systems: Vec<Value> = vec![];
    let list = dats(conn)?;
    for (dat_id, dat_name) in &list {
        let have = found(conn, lib, *dat_id)?;
        let mut q = conn.prepare("SELECT g.name, r.id, r.name, r.size FROM dat_games g JOIN dat_roms r ON r.game_id = g.id WHERE g.dat_id = ?1 ORDER BY g.name, r.name")?;
        let rows: Vec<(String, i64, String, Option<i64>)> = q.query_map([dat_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        let mut by_game: BTreeMap<String, Vec<(i64, String, Option<i64>)>> = BTreeMap::new();
        for (g, id, name, size) in rows {
            by_game.entry(g).or_default().push((id, name, size));
        }
        let mut named = conn.prepare("SELECT root_id, rel_path FROM files WHERE name = ?1 COLLATE NOCASE AND missing_since IS NULL AND parent_id IS NULL LIMIT 1")?;
        let dirs: HashMap<String, PathBuf> = roots::list(lib).into_iter().map(|r| (r.id.clone(), r.dir.clone())).collect();
        for (game, roms) in by_game {
            let ids = systems_of(&game);
            let mut files = vec![];
            let (mut n_have, mut n_missing, mut n_wrong) = (0, 0, 0);
            for (id, name, size) in &roms {
                let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
                let f = match have.get(id).and_then(|v| v.first()) {
                    Some(f) => {
                        n_have += 1;
                        json!({ "name": name, "size": size, "status": "have", "at": f.shown, "loose": f.loose })
                    }
                    None => {
                        let wrong: Option<(String, String)> = named.query_row([base], |r| Ok((r.get(0)?, r.get(1)?))).ok();
                        match wrong {
                            Some((root, rel)) => {
                                n_wrong += 1;
                                let at = dirs.get(&root).map(|d| rel.split('/').fold(d.clone(), |p, x| p.join(x)).display().to_string()).unwrap_or(rel);
                                json!({ "name": name, "size": size, "status": "wrong", "at": at })
                            }
                            None => {
                                n_missing += 1;
                                json!({ "name": name, "size": size, "status": "missing" })
                            }
                        }
                    }
                };
                files.push(f);
            }
            let yours = ids.iter().any(|s| with_games.contains(s));
            systems.push(json!({
                "name": game, "dat": dat_id, "dat_name": dat_name,
                "systems": ids.iter().map(|s| json!({ "id": s, "name": platforms::name_of(s) })).collect::<Vec<_>>(),
                "files": files, "have": n_have, "missing": n_missing, "wrong": n_wrong, "yours": yours,
            }));
        }
    }
    systems.sort_by(|a, b| {
        let k = |v: &Value| (!v["yours"].as_bool().unwrap_or(false), v["have"].as_i64().unwrap_or(0) == 0 && v["missing"].as_i64().unwrap_or(0) == 0, v["missing"].as_i64().unwrap_or(0) == 0 && v["wrong"].as_i64().unwrap_or(0) == 0, v["name"].as_str().unwrap_or("").to_lowercase());
        k(a).cmp(&k(b))
    });
    let yours: Vec<&Value> = systems.iter().filter(|s| s["yours"] == true).collect();
    let short = yours.iter().filter(|s| s["have"] == 0).count();
    Ok(json!({
        "dats": list.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "systems": systems,
        "yours": yours.len(), "none": short,
        "folder": lib.root().join(FOLDER).display().to_string(),
    }))
}

/// A system's BIOS files the library has as files of their own (not inside a ZIP):
/// (its name in the DAT, with its folder, the file). Gathering puts a copy of those in
/// ZIPs in the BIOS folder.
pub fn found_for(conn: &Connection, lib: &Library, system: &str) -> Result<Vec<(String, PathBuf)>> {
    let mut out: Vec<(String, PathBuf)> = vec![];
    for (dat_id, _) in dats(conn)? {
        let have = found(conn, lib, dat_id)?;
        let mut q = conn.prepare("SELECT g.name, r.id, r.name FROM dat_games g JOIN dat_roms r ON r.game_id = g.id WHERE g.dat_id = ?1 ORDER BY r.name")?;
        let rows: Vec<(String, i64, String)> = q.query_map([dat_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        for (game, id, name) in rows {
            if !systems_of(&game).iter().any(|s| s == system) || out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
                continue;
            }
            if let Some(f) = have.get(&id).and_then(|v| v.iter().find(|f| f.loose)) {
                out.push((name, f.path.clone()));
            }
        }
    }
    Ok(out)
}

/// Plan *Gather the BIOS files*: a copy of each one found into the library's
/// `BIOS/<system>/` (hardlinked on the same drive), nested as its DAT says, as an
/// Organise change. The folder becomes a folder with games when it isn't in one.
pub fn gather_plan(ix: &Index, lib: &Library) -> Result<Plan> {
    lib.writable()?;
    let list = dats(ix.conn())?;
    let Some((dat_id, _)) = list.first() else { bail!("Import a BIOS DAT first (Get starter DATs offers libretro's).") };
    let out = lib.root().join(FOLDER);
    let mut plan = organise::plan(ix, lib, &organise::Options {
        dat_id: *dat_id, mode: "copy".into(), out: Some(out.clone()), format: "loose".into(), partial: true, per_game_folder: true,
        ..Default::default()
    })?;
    plan.label = "Gathered the BIOS files into the library's BIOS folder".into();
    plan.title = "Gather the BIOS files into the library's BIOS folder".into();
    if organise::in_root(&roots::list(lib), &out.join("x")).is_none() {
        roots::check_new(lib, &out).map_err(|e| anyhow!("The BIOS files can't go to {}: {e:#}", out.display()))?;
        plan.new_root = Some(out.display().to_string());
    }
    Ok(plan)
}

/// What a report of the missing BIOS files says (text, one system a section).
pub fn report(ix: &Index, lib: &Library) -> Result<String> {
    let s = summary(ix, lib)?;
    let mut out = String::from("BIOS files missing\n\n");
    for sys in s["systems"].as_array().into_iter().flatten() {
        let missing: Vec<&Value> = sys["files"].as_array().into_iter().flatten().filter(|f| f["status"] != "have").collect();
        if missing.is_empty() {
            continue;
        }
        out.push_str(&format!("{}{}\n", sys["name"].as_str().unwrap_or(""), if sys["yours"] == true { "" } else { " (no games of it in the library)" }));
        for f in missing {
            let why = if f["status"] == "wrong" { format!("  (a file of its name isn't it: {})", f["at"].as_str().unwrap_or("")) } else { String::new() };
            out.push_str(&format!("  {}{why}\n", f["name"].as_str().unwrap_or("")));
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libretros_system_dat_is_read_as_a_game_per_system() {
        let text = "clrmamepro (\n\tname \"System\"\n\tversion v1\n)\n\ngame (\n\tname \"System\"\n\tcomment \"System\"\n\n\tcomment \"Sony - PlayStation\"\n\trom ( name scph5500.bin size 4 crc 00000001 )\n\trom ( name scph5501.bin size 4 crc 00000002 )\n\n\tcomment \"Sega - Dreamcast\"\n\trom ( name dc/dc_boot.bin size 4 crc 00000003 )\n)\n";
        let dat = crate::dat::parse(text.as_bytes()).unwrap();
        assert_eq!(dat.games.len(), 1);
        let d = by_system(text, dat);
        assert_eq!(d.games.iter().map(|g| (g.name.as_str(), g.roms.len())).collect::<Vec<_>>(), [("Sony - PlayStation", 2), ("Sega - Dreamcast", 1)]);
        assert_eq!(d.games[1].roms[0].name, "dc/dc_boot.bin");
        assert_eq!(systems_of("Sony - PlayStation"), ["psx"]);
        assert_eq!(systems_of("NEC - PC Engine - TurboGrafx 16 - SuperGrafx"), ["pcengine", "pcenginecd"]);
        assert_eq!(systems_of("Nintendo - Gameboy"), ["gb"]);
        assert!(systems_of("ScummVM").is_empty());
    }
}
