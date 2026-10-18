//! Add a game… (docs/PLAN.md, "Phase 4 design", D24) for any system but PC: a ROM, an
//! archive, or a disc (its sheet brings its tracks) is copied or moved into a folder with
//! games for its system, as an Organise change with the preview and Undo. Once it's
//! there the app scans it: a DAT match can be named as the DAT does (Organise for that
//! game); a file no DAT knows can be identified, and what Identify finds is kept in
//! `_library/games/<system>/<SHA1>/game.json`, by the file's contents (D13).

use crate::audit;
use crate::cue;
use crate::hash;
use crate::identify;
use crate::index::Index;
use crate::library::{now, Library, APP_DIR};
use crate::organise::{self, key, Op, Plan, Src};
use crate::pcgames::Match;
use crate::platforms;
use crate::roots;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Where a system's games can be imported to, the best first: a folder with games for
/// it; the folders its games are in already; then, in a folder with games for every
/// system that holds several systems' games (or none yet), the system's own folder in
/// it. Each is `{ dir, why, platform }` (`platform`: the folder with games' own system, if
/// it has one). A folder with games for every system that holds only another system's
/// games isn't offered: games put there would be read as that system's. Without the
/// index (`conn`), only the folders with games are looked at.
pub fn places(conn: Option<&rusqlite::Connection>, lib: &Library, platform: &str) -> Vec<Value> {
    let name = platforms::name_of(platform);
    let rs: Vec<roots::Root> = roots::list(lib).into_iter().filter(|r| r.found()).collect();
    let mut out: Vec<Value> = vec![];
    let mut seen = HashSet::new();
    let mut push = |out: &mut Vec<Value>, dir: PathBuf, why: String, own: Option<&str>| {
        if seen.insert(key(&dir)) {
            out.push(json!({ "dir": dir.display().to_string(), "why": why, "platform": own }));
        }
    };
    for r in rs.iter().filter(|r| r.platform.as_deref() == Some(platform)) {
        push(&mut out, r.dir.clone(), format!("A folder with games for {name}"), Some(platform));
    }
    // the folders this system's games are in, the fullest first
    let mut held: Vec<(PathBuf, i64)> = vec![];
    let mut systems_in: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    if let Some(conn) = conn {
        let by_id: std::collections::HashMap<&str, &roots::Root> = rs.iter().map(|r| (r.id.as_str(), r)).collect();
        if let Ok(mut q) = conn.prepare("SELECT root_id, rel_path FROM files WHERE platform = ?1 AND parent_id IS NULL AND missing_since IS NULL") {
            let mut count: std::collections::HashMap<String, (PathBuf, i64)> = std::collections::HashMap::new();
            if let Ok(rows) = q.query_map([platform], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))) {
                for (root, rel) in rows.flatten() {
                    let Some(r) = by_id.get(root.as_str()) else { continue };
                    let dir = r.dir.join(&rel).parent().map(Path::to_path_buf).unwrap_or_else(|| r.dir.clone());
                    count.entry(key(&dir)).or_insert((dir, 0)).1 += 1;
                }
            }
            held = count.into_values().collect();
            held.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        if let Ok(mut q) = conn.prepare("SELECT root_id, COUNT(DISTINCT platform) FROM files WHERE platform IS NOT NULL AND missing_since IS NULL GROUP BY root_id") {
            if let Ok(rows) = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                systems_in = rows.flatten().collect();
            }
        }
    }
    for (dir, n) in held.into_iter().take(3) {
        let own = rs.iter().find(|r| dir.starts_with(&r.dir)).and_then(|r| r.platform.clone());
        push(&mut out, dir, format!("Where {n} of your {name} games are"), own.as_deref());
    }
    for r in rs.iter().filter(|r| r.platform.is_none()) {
        let systems = systems_in.get(&r.id).copied().unwrap_or(0);
        let mine = conn.is_some_and(|c| c.query_row("SELECT 1 FROM files WHERE root_id = ?1 AND platform = ?2 LIMIT 1", rusqlite::params![r.id, platform], |_| Ok(())).is_ok());
        if conn.is_some() && systems == 1 && !mine {
            continue;
        }
        // a folder named for a system is that system's, not every system's
        match r.dir.file_name().and_then(|n| platforms::from_folder(&n.to_string_lossy())) {
            Some(p) if p.id == platform => {
                push(&mut out, r.dir.clone(), format!("A folder with games for {name}"), None);
                continue;
            }
            Some(_) => continue,
            None => {}
        }
        // a folder there named for the system already, else the system's usual folder name
        let named = std::fs::read_dir(&r.dir).into_iter().flatten().flatten().filter(|e| e.path().is_dir())
            .find(|e| platforms::from_folder(&e.file_name().to_string_lossy()).is_some_and(|p| p.id == platform))
            .map(|e| e.path());
        let dir = named.unwrap_or_else(|| r.dir.join(organise::safe_name(&name)));
        push(&mut out, dir, format!("{name}'s folder in {}, a folder with games for every system", r.dir.display()), None);
    }
    if out.is_empty() {
        // a system with no folder yet gets one named for it, beside the other systems' folders
        let base = systems_parent(&rs).unwrap_or_else(|| lib.root().to_path_buf());
        let why = format!("A new folder for {name} in {}", base.display());
        push(&mut out, base.join(organise::safe_name(&name)), why, Some(platform));
    }
    out
}

/// The folder the systems' own folders are in, the usual one: Roms for Roms/GBA and
/// Roms/GBA/Champion. A folder is a system's when it's named for one, or is set as one's.
fn systems_parent(rs: &[roots::Root]) -> Option<PathBuf> {
    let mut count: Vec<(PathBuf, usize)> = vec![];
    for r in rs {
        let top = r.dir.ancestors().filter(|a| a.file_name().is_some_and(|n| platforms::from_folder(&n.to_string_lossy()).is_some())).last();
        let base = match top {
            Some(t) => t.parent(),
            None if r.platform.is_some() => r.dir.parent(),
            None => None,
        };
        let Some(base) = base else { continue };
        match count.iter_mut().find(|(d, _)| key(d) == key(base)) {
            Some(c) => c.1 += 1,
            None => count.push((base.to_path_buf(), 1)),
        }
    }
    // the most, the first found on a tie
    count.into_iter().rev().max_by_key(|c| c.1).map(|c| c.0)
}

/// The files a chosen file brings: a sheet its tracks (beside it).
pub(crate) fn with_tracks(p: &Path) -> Vec<PathBuf> {
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut v = vec![p.to_path_buf()];
    if cue::is_sheet(&name) {
        if let Ok(text) = std::fs::read_to_string(p) {
            let dir = p.parent().unwrap_or(Path::new("."));
            for f in cue::file_names(&text, name.ends_with(".gdi")) {
                let t = dir.join(&f);
                if t.is_file() && !v.contains(&t) {
                    v.push(t);
                }
            }
        }
    }
    v
}

/// Plan adding these files to `platform`'s games in `into` (a folder with games, or any
/// folder, which becomes one for the system). `mode`: copy (hardlinked on the same drive
/// unless `always_copy`) or move.
pub fn plan(lib: &Library, files: &[PathBuf], platform: &str, into: &Path, mode: &str, always_copy: bool, cancel: &AtomicBool) -> Result<Plan> {
    lib.writable()?;
    if platform == "pc" {
        bail!("PC games are added with Import… on the PC games page.");
    }
    if platforms::by_id(platform).is_none() {
        bail!("There's no system {platform}.");
    }
    if files.is_empty() {
        bail!("Choose a file to add.");
    }
    let rs = roots::list(lib);
    let new_root = match organise::in_root(&rs, &into.join("x")) {
        Some(_) => None,
        None => {
            if into.starts_with(lib.root().join(APP_DIR)) {
                bail!("Games can't go in the library's own _library folder.");
            }
            Some(into.display().to_string())
        }
    };
    let mut ops = vec![];
    let mut seen = HashSet::new();
    let mut games = 0;
    for f in files {
        if !f.is_file() {
            bail!("There's no file {}.", f.display());
        }
        games += 1;
        let game = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        for p in with_tracks(f) {
            if !seen.insert(key(&p)) {
                continue;
            }
            let name = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let to = into.join(organise::safe_name(&name));
            if key(&to) == key(&p) {
                bail!("{} is in that folder already.", name);
            }
            if to.exists() {
                bail!("There's a file called {name} in {} already.", into.display());
            }
            let mut r = std::fs::File::open(&p).with_context(|| format!("couldn't read {}", p.display()))?;
            let size = r.metadata()?.len();
            let h = hash::hash_reader(&mut r, size, "", cancel)?;
            let src = Src {
                file_id: 0, root: format!("dir:{}", p.parent().unwrap_or(Path::new(".")).display()), rel: name.clone(), path: p.display().to_string(),
                member: None, size, crc32: h.crc32, sha1: h.sha1,
            };
            let op = match mode {
                "move" => "move",
                _ if !always_copy && organise::same_drive(&p, into) => "link",
                _ => "copy",
            };
            ops.push(Op { op: op.into(), game: game.clone(), to: to.display().to_string(), from: vec![src], names: vec![], aside: vec![], size, note: None });
        }
    }
    let name = platforms::name_of(platform);
    Ok(Plan {
        id: organise::new_id(),
        created: now(),
        kind: "add".into(),
        label: format!("Add {games} game{} to {name}", if games == 1 { "" } else { "s" }),
        title: String::new(),
        dat: Value::Null,
        platform: platform.into(),
        mode: mode.into(),
        format: "keep".into(),
        out: Some(into.display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root, new_roots: vec![],
        present: vec![],
        export: None,
    })
}

/// What the files an Add put in place turned out to be, once scanned: the DAT game each
/// matched (and whether it's named as the DAT says), or that no DAT knows it.
pub fn results(ix: &Index, lib: &Library, plan: &Plan) -> Result<Value> {
    let conn = ix.conn();
    let rs = roots::list(lib);
    let dat = audit::primary_dat(conn, lib, &plan.platform)?;
    let mut out = vec![];
    let mut shown = HashSet::new();
    for op in &plan.ops {
        if !shown.insert(op.game.clone()) {
            continue;
        }
        let to = Path::new(&op.to);
        let Some((root, rel)) = organise::in_root(&rs, to) else { continue };
        let row: Option<(i64, Option<String>, String, i64)> = conn
            .query_row(
                "SELECT f.id, b.sha1, b.crc32, f.size FROM files f LEFT JOIN blobs b ON b.id = f.blob_id WHERE f.root_id = ?1 AND f.rel_path = ?2",
                params![root.id, rel],
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default(), r.get(3)?)),
            )
            .ok();
        let file_id = row.as_ref().map(|r| r.0);
        // the DAT game of the file, or of a member of it
        let matched: Option<(String, String)> = match (file_id, dat) {
            (Some(fid), Some(d)) => conn
                .query_row(
                    "SELECT g.name, r.name FROM matches m JOIN dat_roms r ON r.id = m.rom_id JOIN dat_games g ON g.id = r.game_id
                     JOIN files f ON f.id = m.file_id WHERE r.dat_id = ?2 AND (f.id = ?1 OR f.parent_id = ?1) LIMIT 1",
                    params![fid, d],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok(),
            _ => None,
        };
        let name = to.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let identity = row.as_ref().and_then(|r| r.1.clone()).and_then(|s| rom_identity(lib, &plan.platform, &s));
        out.push(json!({
            "file": name, "path": op.to, "game": matched.as_ref().map(|m| m.0.clone()), "dat_rom": matched.as_ref().map(|m| m.1.clone()),
            "named": matched.as_ref().is_some_and(|(g, r)| name == *r || to.file_stem().is_some_and(|s| s.to_string_lossy() == *g)),
            "sha1": row.as_ref().and_then(|r| r.1.clone()), "crc32": row.as_ref().map(|r| r.2.clone()), "size": row.as_ref().map(|r| r.3),
            "dat": dat, "identity": identity,
        }));
    }
    Ok(json!({ "platform": plan.platform, "platform_name": platforms::name_of(&plan.platform), "files": out }))
}

/// Where what Identify found for a file no DAT knows is kept: `_library/games/<system>/<SHA1>/`.
pub fn rom_dir(lib: &Library, platform: &str, sha1: &str) -> Result<PathBuf> {
    if sha1.len() != 40 || !sha1.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("That isn't a SHA1.");
    }
    platforms::valid_id(platform)?;
    Ok(lib.root().join(APP_DIR).join("games").join(platform).join(sha1.to_lowercase()))
}

/// What's kept for a file no DAT knows, if anything.
pub fn rom_identity(lib: &Library, platform: &str, sha1: &str) -> Option<Value> {
    let d = rom_dir(lib, platform, sha1).ok()?;
    let v: Value = serde_json::from_slice(&std::fs::read(d.join("game.json")).ok()?).ok()?;
    let mut v = v;
    v["media"] = json!(crate::identify::media_of(lib, &d.join("media")));
    Some(v)
}

/// Keep what a source said about a file no DAT knows, with its artwork.
pub fn identify_rom(lib: &Library, platform: &str, file: &Value, m: &Match) -> Result<Value> {
    lib.writable()?;
    let sha1 = file["sha1"].as_str().ok_or_else(|| anyhow!("The file hasn't been read yet: scan it first."))?;
    let d = rom_dir(lib, platform, sha1)?;
    std::fs::create_dir_all(&d)?;
    let p = d.join("game.json");
    let mut v = crate::config::read_json_object(&p);
    v["id"] = json!(format!("sha1:{}", sha1.to_lowercase()));
    v["platform"] = json!(platform);
    v["files"] = json!([{ "name": file["file"], "size": file["size"], "crc32": file["crc32"], "sha1": sha1 }]);
    v["title"] = json!(m.title);
    v["year"] = json!(m.year);
    v["ids"][m.source.as_str()] = json!(m.id);
    v["details"] = json!(m.details);
    v["identified"] = json!(now());
    // kept as the source's answer too (D46)
    let fields: Vec<String> = crate::meta::FIELDS.iter().map(|(k, _)| k.to_string()).collect();
    crate::sources::keep(&d, m, &fields, true)?;
    crate::sources::make(&mut v, &d, &crate::sources::Rules::of(lib));
    crate::config::write_json(&p, &v)?;
    identify::fetch_art(&d.join("media"), &m.art, true);
    Ok(rom_identity(lib, platform, sha1).unwrap_or(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn a_system_with_no_folder_gets_one_named_for_it_beside_the_others() {
        let dir = temp_dir("add-game-new-system");
        let lib = Library::open(dir.join("lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), &lib.id().unwrap()).unwrap();
        let roms = dir.join("Roms");
        let champion = roms.join("GB/Champion Collection");
        for (d, platform) in [(roms.join("GBA"), Some("gba")), (champion.clone(), None), (roms.join("SNES"), None)] {
            std::fs::create_dir_all(&d).unwrap();
            roots::add(&lib, &d, platform).unwrap();
        }
        std::fs::write(champion.join("Delta (World).gb"), crate::audit::tests::rom(8, 1024)).unwrap();
        crate::scan::scan(&ix, &lib, &crate::scan::Options::default(), &AtomicBool::new(false), &mut |_| {}).unwrap();
        // the Game Boy's collection and the Super Nintendo's folder aren't the Game Gear's
        let dirs = |p: &str| places(Some(ix.conn()), &lib, p).iter().map(|x| PathBuf::from(x["dir"].as_str().unwrap())).collect::<Vec<_>>();
        assert_eq!(dirs("gamegear"), [roms.join("Sega Game Gear")]);
        assert_eq!(dirs("snes"), [roms.join("SNES")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rom_is_added_scanned_and_found_in_its_dat_or_identified() {
        let (dir, lib, ix) = crate::collection::tests::fixture("add-game");
        // Zeta (Japan) isn't in the set yet; another file no DAT knows
        let elsewhere = dir.join("Downloads");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("zeta.gb"), crate::audit::tests::rom(10, 1024)).unwrap();
        std::fs::write(elsewhere.join("homebrew.gb"), b"a homebrew game nobody listed").unwrap();
        let roms = dir.join("Roms");
        assert_eq!(places(Some(ix.conn()), &lib, "gb")[0]["dir"].as_str(), Some(roms.to_str().unwrap()));
        let cancel = AtomicBool::new(false);
        let plan = plan(&lib, &[elsewhere.join("zeta.gb"), elsewhere.join("homebrew.gb")], "gb", &roms, "copy", true, &cancel).unwrap();
        assert_eq!((plan.games, plan.ops.len(), plan.kind.as_str()), (2, 2, "add"));
        organise::run(&ix, &lib, &plan, &cancel, &mut |_| {}).unwrap();
        assert!(roms.join("zeta.gb").is_file() && elsewhere.join("zeta.gb").is_file());
        let opts = crate::scan::Options { only: Some(plan.ops.iter().map(|o| PathBuf::from(&o.to)).collect()), ..Default::default() };
        crate::scan::scan(&ix, &lib, &opts, &cancel, &mut |_| {}).unwrap();
        let r = results(&ix, &lib, &plan).unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!((files[0]["game"].as_str(), files[0]["named"].as_bool()), (Some("Zeta (Japan)"), Some(false)));
        assert!(files[1]["game"].is_null() && files[1]["sha1"].is_string());
        // identified: kept by its SHA1 in the library
        let m = Match { source: "launchbox".into(), id: "77".into(), title: "Home Brew".into(), year: Some(2020), ..Default::default() };
        let v = identify_rom(&lib, "gb", &files[1], &m).unwrap();
        assert_eq!((v["title"].as_str(), v["ids"]["launchbox"].as_str()), (Some("Home Brew"), Some("77")));
        let again = results(&ix, &lib, &plan).unwrap();
        assert_eq!(again["files"][1]["identity"]["title"], "Home Brew");
        // a file can't go where one of its name is
        assert!(super::plan(&lib, &[elsewhere.join("zeta.gb")], "gb", &roms, "copy", true, &cancel).is_err());
        let _ = temp_dir;
        let _ = std::fs::remove_dir_all(&dir);
    }
}
