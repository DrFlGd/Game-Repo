//! Texture packs (docs/PLAN.md, "Cheats and patches", D204): kept in a game's
//! `patches/textures/` as they came (an archive, or a folder of the pack's files), each
//! with its title, author, version and the emulator it's for in `patches.json`'s
//! `textures`. They're never copied twice: brought in hardlinked on the same drive, or
//! moved in when asked.
//!
//! The emulator installs them: each save profile's `textures` says where it loads a game's
//! pack from in its folder (Dolphin `Load/Textures/<game ID>/`, PCSX2
//! `textures/<serial>/replacements/`…). *Install into an emulator…* on this computer
//! unpacks the pack there (an archive once, into the data folder) and puts its files in as
//! an Organise change, hardlinked where it can, with Undo; the emulator's folder is kept in
//! `config.json`'s `emulator_dirs`.

use crate::cheats::{game_of, LibGame};
use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use rusqlite::Connection;
use crate::library::{now, Library};
use crate::organise::{self, safe_name, Op, Plan};
use crate::patches;
use anyhow::{anyhow, bail, Context, Result};
use gamerepo_wire::saves::Profile;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub const FOLDER: &str = "textures";

/// The emulators that load texture packs for `platform`: (profile, its folder for them).
pub fn emulators_for(platform: &str) -> Vec<(Profile, String)> {
    gamerepo_wire::saves::built_in()
        .into_iter()
        .filter(|p| p.systems.iter().any(|s| s == platform) || p.kind == "retroarch")
        .filter_map(|p| {
            let path = p.textures.get(platform).or_else(|| p.textures.get("*")).cloned()?;
            Some((p, path))
        })
        .collect()
}

/// The emulators for a system, as the page lists them.
pub fn emulators_json(platform: &str) -> Vec<Value> {
    emulators_for(platform).iter().map(|(p, path)| json!({ "id": p.id, "name": p.name, "path": path, "setting": p.textures_setting })).collect()
}

/// Where in the emulator's folder a game's pack goes: its `textures` with the game's tokens.
pub fn expand(path: &str, id: Option<&str>, stem: &str, pack: &str) -> Result<String> {
    if (path.contains("{id}") || path.contains("{id_nodash}")) && id.is_none() {
        bail!("The emulator finds a pack by the game's ID, and Game Repo couldn't read this game's.");
    }
    let id = id.unwrap_or("");
    Ok(path.replace("{id_nodash}", &id.replace('-', "")).replace("{id}", id).replace("{stem}", stem).replace("{pack}", pack))
}

/// A game's texture packs folder.
fn dir_of(conn: &rusqlite::Connection, lib: &Library, g: &LibGame) -> Result<Option<PathBuf>> {
    Ok(patches::game_dir(conn, lib, g)?.map(|d| d.join(patches::FOLDER).join(FOLDER)))
}

pub(crate) fn is_archive(name: &str) -> bool {
    let n = name.to_lowercase();
    [".zip", ".7z", ".rar"].iter().any(|x| n.ends_with(x))
}

pub(crate) fn size_of(p: &Path) -> u64 {
    if p.is_dir() {
        crate::cheats::walk(p).iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()
    } else {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
}

/// A game's texture packs, with their details.
pub fn packs(ix: &Index, lib: &Library, key: &str) -> Result<Value> {
    crate::cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let mut out = vec![];
    if let Some(dir) = dir_of(conn, lib, &g)? {
        let m = read_json_object(&dir.parent().unwrap_or(&dir).join(patches::MANIFEST));
        let mut names: Vec<(String, PathBuf)> = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).filter(|(n, _)| !n.starts_with('.')).collect();
        names.sort();
        for (n, p) in names {
            let e = m["textures"].get(&n).cloned().unwrap_or(json!({}));
            out.push(json!({
                "name": n, "title": e["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&n), "author": e["author"], "version": e["version"], "emulator": e["emulator"],
                "game_id": e["game_id"], "size": size_of(&p), "archive": p.is_file(), "added": e["added"], "installed": e["installed"],
            }));
        }
    }
    let id = crate::gameid::of_key(conn, lib, key);
    Ok(json!({ "key": key, "title": g.title, "platform": g.platform, "game_id": id, "packs": out, "emulators": emulators_json(&g.platform) }))
}

/// Every game's texture packs, for the page's tab.
pub fn list(ix: &Index, lib: &Library) -> Result<Value> {
    crate::cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let mut items = vec![];
    for sys in std::fs::read_dir(crate::meta::games_root(lib)).into_iter().flatten().flatten() {
        for g in std::fs::read_dir(sys.path()).into_iter().flatten().flatten() {
            let dir = g.path().join(patches::FOLDER).join(FOLDER);
            if !dir.is_dir() {
                continue;
            }
            let Some(folder) = lib.relative(&g.path()) else { continue };
            let row: Option<LibGame> = conn
                .query_row("SELECT key, kind, platform, dat_game, name, title, folder, picture FROM library WHERE folder = ?1 ORDER BY addon != '', have DESC LIMIT 1", [&folder], |r| {
                    Ok(LibGame { key: r.get(0)?, kind: r.get(1)?, platform: r.get(2)?, dat_game: r.get(3)?, name: r.get(4)?, title: r.get(5)?, folder: r.get(6)?, picture: r.get(7)? })
                })
                .ok();
            let Some(lg) = row else { continue };
            let m = read_json_object(&g.path().join(patches::FOLDER).join(patches::MANIFEST));
            for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with('.') {
                    continue;
                }
                let d = m["textures"].get(&n).cloned().unwrap_or(json!({}));
                let mut o = crate::cheats::shown(&lg);
                o["id"] = json!(format!("{}/{n}", lg.key));
                o["game_key"] = json!(lg.key);
                o["game_title"] = json!(lg.title);
                o["file"] = json!(n);
                o["title"] = json!(d["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(&n));
                o["author"] = d["author"].clone();
                o["version"] = d["version"].clone();
                o["emulator"] = d["emulator"].clone();
                o["size"] = json!(size_of(&e.path()));
                o["installed"] = d["installed"].clone();
                items.push(o);
            }
        }
    }
    Ok(json!({ "items": items }))
}

/// Plan bringing packs (archives or folders) in to a game: hardlinked or copied (`how`
/// "copy"), always copied ("always") or moved ("move"), each with its details guessed.
pub fn import_plan(ix: &Index, lib: &Library, key: &str, paths: &[PathBuf], how: &str) -> Result<Plan> {
    lib.writable()?;
    crate::cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let (game_dir, new_game) = patches::folder_for(conn, lib, &g)?;
    let pdir = game_dir.join(patches::FOLDER);
    let tdir = pdir.join(FOLDER);
    let mut m = read_json_object(&pdir.join(patches::MANIFEST));
    let emulator = emulators_for(&g.platform).first().map(|(p, _)| p.id.clone());
    let game_id = crate::gameid::of_key(conn, lib, key);
    let op_of = |from: &Path, to: &Path| -> Op {
        let s = outside(from);
        let kind = match how {
            "move" => "move",
            "always" => "copy",
            _ => "link",
        };
        Op { op: kind.into(), game: g.name.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some("a texture pack".into()) }
    };
    let mut ops = vec![];
    if let Some(gj) = &new_game {
        ops.push(text_op(&g.name, &game_dir.join("game.json"), serde_json::to_string_pretty(&sorted(gj))? + "\n", "game.json"));
    }
    for p in paths {
        let name = safe_name(&p.file_name().unwrap_or_default().to_string_lossy());
        if name.is_empty() {
            continue;
        }
        if tdir.join(&name).exists() {
            bail!("{} has a texture pack called {name} already.", g.title);
        }
        if p.is_dir() {
            for f in crate::cheats::walk(p) {
                let rel = f.strip_prefix(p).unwrap_or(&f);
                ops.push(op_of(&f, &tdir.join(&name).join(rel)));
            }
        } else if p.is_file() && is_archive(&name) {
            ops.push(op_of(p, &tdir.join(&name)));
        } else {
            bail!("{} isn't a texture pack: choose its folder, or the ZIP or 7z it came in.", p.display());
        }
        let stem = if p.is_dir() { name.clone() } else { name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(name.clone()) };
        let d = patches::guess(&format!("{stem}.x"), "");
        m["textures"][&name] = json!({ "title": d.title, "version": d.version, "emulator": emulator, "game_id": game_id, "added": now() });
    }
    if ops.len() <= new_game.is_some() as usize {
        bail!("Choose a texture pack: its folder, or the ZIP or 7z it came in.");
    }
    ops.push(text_op(&g.name, &pdir.join(patches::MANIFEST), serde_json::to_string_pretty(&sorted(&m))? + "\n", "patches.json: the texture packs' details"));
    let n = paths.len();
    let label = if n == 1 { "a texture pack".to_string() } else { format!("{n} texture packs") };
    Ok(plan_of("textures", format!("Brought in {label} for {}", g.title), format!("Bring in {label} for {}", g.title), lib, ops, &g.platform, json!({ "key": key })))
}

pub(crate) fn plan_of(kind: &str, label: String, title: String, lib: &Library, ops: Vec<Op>, platform: &str, dat: Value) -> Plan {
    Plan {
        id: organise::new_id(),
        created: now(),
        kind: kind.into(),
        label,
        title,
        dat,
        platform: platform.into(),
        mode: "copy".into(),
        format: "keep".into(),
        out: Some(crate::meta::games_root(lib).display().to_string()),
        space: organise::space_needed(&ops),
        ops,
        games: 1,
        in_place: 0,
        conflicts: vec![],
        duplicates: vec![],
        left: vec![],
        new_root: None,
        new_roots: vec![],
        present: vec![],
        export: None,
        brought: vec![],
    }
}

/// Unpack an archive into `to` (each member a file under it).
pub(crate) fn unpack(archive: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    let never = AtomicBool::new(false);
    crate::archive::each(archive, &never, &mut |name, _, r| {
        let rel: PathBuf = name.split(['/', '\\']).filter(|x| !x.is_empty() && *x != "." && *x != "..").collect();
        if rel.as_os_str().is_empty() || name.ends_with('/') {
            let mut sink = std::io::sink();
            std::io::copy(r, &mut sink)?;
            return Ok(());
        }
        let p = to.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::File::create(&p)?;
        std::io::copy(r, &mut f)?;
        Ok(())
    })
    .with_context(|| format!("couldn't unpack {}", archive.display()))?;
    Ok(())
}

/// The pack's files, by their paths inside it: a lone top folder (a ZIP of `GALE01/…`, or
/// of the pack's own name) isn't part of where they go.
pub(crate) fn pack_files(root: &Path) -> Vec<(PathBuf, PathBuf)> {
    let files: Vec<PathBuf> = crate::cheats::walk(root);
    let rels: Vec<PathBuf> = files.iter().map(|f| f.strip_prefix(root).unwrap_or(f).to_path_buf()).collect();
    let tops: std::collections::HashSet<_> = rels.iter().filter_map(|r| r.components().next()).collect();
    let strip = tops.len() == 1 && rels.iter().all(|r| r.components().count() > 1);
    files
        .into_iter()
        .zip(rels)
        .map(|(f, r)| {
            let r = if strip { r.components().skip(1).collect() } else { r };
            (f, r)
        })
        .collect()
}

/// Plan *Install into an emulator…*: the pack's files put where `emulator` (a save profile's
/// id) loads them in its folder `folder`, hardlinked on the same drive; an archive is
/// unpacked first, into `<data dir>/textures-staging/`. The folder is remembered.
pub fn install_plan(ix: &Index, lib: &Library, data_dir: &Path, key: &str, pack: &str, emulator: &str, folder: &Path) -> Result<Plan> {
    crate::cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let tdir = dir_of(conn, lib, &g)?.ok_or_else(|| anyhow!("{} has no texture packs.", g.title))?;
    let src = tdir.join(pack);
    if pack.contains(['/', '\\']) || !src.exists() {
        bail!("{} has no texture pack {pack}.", g.title);
    }
    let (_, path) = emulators_for(&g.platform).into_iter().find(|(p, _)| p.id == emulator).ok_or_else(|| anyhow!("That emulator doesn't load texture packs for {}.", crate::browse::system_name(&g.platform)))?;
    install_at(conn, lib, data_dir, &g, key, pack, emulator, &path, folder)
}

/// `install_plan` once the emulator's path for the pack is known.
#[allow(clippy::too_many_arguments)]
fn install_at(conn: &Connection, lib: &Library, data_dir: &Path, g: &LibGame, key: &str, pack: &str, emulator: &str, path: &str, folder: &Path) -> Result<Plan> {
    let tdir = dir_of(conn, lib, g)?.ok_or_else(|| anyhow!("{} has no texture packs.", g.title))?;
    let src = tdir.join(pack);
    if !folder.is_dir() {
        bail!("{} isn't a folder.", folder.display());
    }
    let id = crate::gameid::of_key(conn, lib, key);
    let stem = crate::browse::files_of(conn, lib, g.dat_game.unwrap_or(-1))
        .ok()
        .and_then(|f| f.first().and_then(|x| x["path"].as_str().map(|p| Path::new(p).file_stem().unwrap_or_default().to_string_lossy().into_owned())))
        .unwrap_or_else(|| g.name.clone());
    let pack_name = if src.is_file() { pack.rsplit_once('.').map(|(s, _)| s).unwrap_or(pack) } else { pack };
    let rel = expand(path, id.as_deref(), &stem, pack_name)?;
    let target = rel.split('/').filter(|x| !x.is_empty()).fold(folder.to_path_buf(), |p, x| p.join(x));
    let root = if src.is_file() {
        let staging = data_dir.join("textures-staging");
        // packs unpacked for plans a day old or more are done with
        for e in std::fs::read_dir(&staging).into_iter().flatten().flatten() {
            if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 86_400) {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
        let at = staging.join(organise::new_id());
        unpack(&src, &at)?;
        at
    } else {
        src.clone()
    };
    let mut ops = vec![];
    for (f, r) in pack_files(&root) {
        let s = outside(&f);
        ops.push(Op { op: "link".into(), game: g.name.clone(), to: target.join(&r).display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some("a texture".into()) });
    }
    if ops.is_empty() {
        bail!("The pack {pack} has no files in it.");
    }
    // the game's patches.json says where it went
    let pdir = tdir.parent().unwrap_or(&tdir).to_path_buf();
    let mut m = read_json_object(&pdir.join(patches::MANIFEST));
    let mut e = m["textures"].get(pack).cloned().unwrap_or(json!({}));
    e["installed"] = json!({ "emulator": emulator, "to": target.display().to_string(), "when": now() });
    m["textures"][pack] = e;
    ops.push(text_op(&g.name, &pdir.join(patches::MANIFEST), serde_json::to_string_pretty(&sorted(&m))? + "\n", "patches.json: where the pack went"));
    Ok(plan_of(
        "textures_install",
        format!("Installed {pack} for {}", g.title),
        format!("Install {pack} for {}", g.title),
        lib,
        ops,
        &g.platform,
        json!({ "key": key, "pack": pack, "emulator": emulator, "to": target.display().to_string() }),
    ))
}

/// Remove a pack from the library: moved to `patches/textures/.removed/<when>/`.
pub fn remove(ix: &Index, lib: &Library, key: &str, pack: &str) -> Result<Value> {
    lib.writable()?;
    let conn = ix.conn();
    let g = game_of(conn, key)?;
    let tdir = dir_of(conn, lib, &g)?.ok_or_else(|| anyhow!("{} has no texture packs.", g.title))?;
    let src = tdir.join(pack);
    if pack.contains(['/', '\\']) || !src.exists() {
        bail!("{} has no texture pack {pack}.", g.title);
    }
    let to = tdir.join(".removed").join(now().replace([':', 'T'], "-").trim_end_matches('Z'));
    std::fs::create_dir_all(&to)?;
    std::fs::rename(&src, to.join(pack))?;
    packs(ix, lib, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_emulator_says_where_a_games_pack_goes() {
        let gc: Vec<String> = emulators_for("gc").iter().map(|(p, _)| p.id.clone()).collect();
        assert!(gc.contains(&"dolphin".to_string()), "{gc:?}");
        assert_eq!(expand("Load/Textures/{id}/", Some("GALE01"), "x", "p").unwrap(), "Load/Textures/GALE01/");
        assert_eq!(expand("PSP/TEXTURES/{id_nodash}/", Some("ULUS-10041"), "x", "p").unwrap(), "PSP/TEXTURES/ULUS10041/");
        assert!(expand("textures/{id}/", None, "x", "p").is_err());
        let nes: Vec<(String, String)> = emulators_for("nes").into_iter().map(|(p, path)| (p.id, path)).collect();
        assert_eq!(nes, vec![("retroarch".to_string(), "system/HdPacks/{stem}/".to_string())]);
    }

    #[test]
    fn a_pack_is_brought_in_installed_into_an_emulators_folder_and_undone() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("textures");
        let cancel = AtomicBool::new(false);
        // a NES-style pack for a Game Boy game won't do: give the Game Boy a stand-in path via RetroArch? No:
        // RetroArch loads packs for nes and n64 only, so the fixture's Game Boy games have no emulator
        assert!(emulators_for("gb").is_empty());
        // the pack: a ZIP with a lone top folder
        let zip = dir.join("Delta HD.zip");
        {
            let f = std::fs::File::create(&zip).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let o: zip::write::SimpleFileOptions = Default::default();
            for (n, b) in [("Delta HD/hires.txt", &b"<ver>100"[..]), ("Delta HD/tiles/a.png", b"PNG1")] {
                w.start_file(n, o).unwrap();
                std::io::Write::write_all(&mut w, b).unwrap();
            }
            w.finish().unwrap();
        }
        let p = import_plan(&ix, &lib, "gb/Delta (World)", std::slice::from_ref(&zip), "copy").unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        let v = packs(&ix, &lib, "gb/Delta (World)").unwrap();
        assert_eq!(v["packs"][0]["name"], json!("Delta HD.zip"));
        assert_eq!(v["packs"][0]["title"], json!("Delta HD"));
        assert_eq!(v["packs"][0]["archive"], json!(true));
        assert!(zip.is_file(), "brought in as a copy (or hardlink), the download stays");
        let listed = list(&ix, &lib).unwrap();
        assert_eq!(listed["items"].as_array().unwrap().len(), 1);
        // no emulator loads Game Boy packs, so installing says so
        let emu = dir.join("emu");
        std::fs::create_dir_all(&emu).unwrap();
        assert!(install_plan(&ix, &lib, &dir.join("data"), "gb/Delta (World)", "Delta HD.zip", "retroarch", &emu).is_err());
        // installed where a path says (as RetroArch's Mesen would for a NES game): unpacked, its lone top folder dropped
        let g = game_of(ix.conn(), "gb/Delta (World)").unwrap();
        let p = install_at(ix.conn(), &lib, &dir.join("data"), &g, "gb/Delta (World)", "Delta HD.zip", "retroarch", "system/HdPacks/{stem}/", &emu).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        let at = emu.join("system/HdPacks/Delta (World)");
        assert_eq!(std::fs::read(at.join("hires.txt")).unwrap(), b"<ver>100");
        assert_eq!(std::fs::read(at.join("tiles/a.png")).unwrap(), b"PNG1");
        let v = packs(&ix, &lib, "gb/Delta (World)").unwrap();
        assert_eq!(v["packs"][0]["installed"]["emulator"], json!("retroarch"), "{v}");
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert!(!at.join("hires.txt").exists(), "undone: the emulator's folder is as it was");
        // removed: kept aside
        remove(&ix, &lib, "gb/Delta (World)", "Delta HD.zip").unwrap();
        assert_eq!(packs(&ix, &lib, "gb/Delta (World)").unwrap()["packs"], json!([]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
