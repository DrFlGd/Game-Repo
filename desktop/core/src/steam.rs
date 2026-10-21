//! Steam (docs/PLAN.md 3.4, "Phase 4 design", D28): PC games with a program to start
//! become non-Steam shortcuts, with their artwork in Steam's grid folder. Steam keeps its
//! shortcuts in `userdata/<id>/config/shortcuts.vdf`, a binary VDF file it rewrites
//! itself, so it's written only while Steam is closed, backed up first, and only the
//! shortcuts tagged "Game Repo" are the app's: every other is kept as it is.

use crate::library::{now, Library};
use crate::pcgames;
use anyhow::{bail, Context, Result};
pub use gamerepo_steam::{account_id, find, parse, running, shortcut_id, users, write, Shortcut, Vdf};
use gamerepo_steam::{config_dir, is_tagged, shortcuts_of};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// The tag that marks the app's shortcuts.
pub const TAG: &str = "Game Repo";

/// Whether a shortcut in the file is one of the app's (tagged "Game Repo").
fn is_ours(v: &Vdf) -> bool {
    is_tagged(v, TAG)
}

fn quoted(p: &str) -> String {
    gamerepo_steam::quoted(p)
}

/// The app's shortcut for each PC game with a program, its artwork beside it.
pub fn shortcuts(lib: &Library) -> (Vec<(String, Shortcut, BTreeMap<String, PathBuf>)>, Vec<String>) {
    let mut out = vec![];
    let mut without = vec![];
    for e in pcgames::list(lib) {
        let Some(l) = &e.game.launch else {
            without.push(e.game.title.clone());
            continue;
        };
        let prog = pcgames::program_path(&e.dir, l);
        let start = if l.dir.is_empty() { prog.parent().map(Path::to_path_buf).unwrap_or_default() } else { pcgames::program_path(&e.dir, &pcgames::Launch { program: l.dir.clone(), ..Default::default() }) };
        let exe = quoted(&prog.display().to_string());
        let name = e.game.title.clone();
        let media: BTreeMap<String, PathBuf> = pcgames::media(lib, &e.game.id)
            .into_iter()
            .filter_map(|(k, v)| Some((k, lib.resolve(v.as_str()?).ok()?)))
            .collect();
        let s = Shortcut {
            appid: shortcut_id(&exe, &name),
            icon: media.get("icon").map(|p| p.display().to_string()).unwrap_or_else(|| prog.display().to_string()),
            name,
            exe,
            start_dir: quoted(&start.display().to_string()),
            args: l.args.clone(),
        };
        out.push((e.game.id.clone(), s, media));
    }
    (out, without)
}

/// What the app last wrote for a user (kept on this computer): the shortcuts' ids and the
/// grid files.
fn record_path(data_dir: &Path, user: &str) -> PathBuf {
    data_dir.join("steam").join(format!("{user}.json"))
}

/// What writing the shortcuts would do: those added, changed and removed, and the others kept.
pub fn preview(lib: &Library, steam: &Path, user: &str) -> Result<Value> {
    let dir = config_dir(steam, user)?;
    let current = match std::fs::read(dir.join("shortcuts.vdf")) {
        Ok(b) => shortcuts_of(&parse(&b).context("shortcuts.vdf can't be read")?),
        Err(_) => vec![],
    };
    let (mine, without) = shortcuts(lib);
    let ours: Vec<&Vdf> = current.iter().filter(|v| is_ours(v)).collect();
    let others = current.len() - ours.len();
    let ids_now: HashSet<u32> = ours.iter().filter_map(|v| v.int("appid")).collect();
    let mut add = vec![];
    let mut same = vec![];
    for (game, s, media) in &mine {
        let row = json!({ "game": game, "name": s.name, "exe": s.exe.trim_matches('"'), "appid": s.appid, "art": media.keys().collect::<Vec<_>>() });
        if ids_now.contains(&s.appid) {
            same.push(row);
        } else {
            add.push(row);
        }
    }
    let keep: HashSet<u32> = mine.iter().map(|(_, s, _)| s.appid).collect();
    let remove: Vec<Value> = ours.iter().filter(|v| v.int("appid").is_some_and(|a| !keep.contains(&a))).map(|v| json!({ "name": v.str("AppName"), "appid": v.int("appid") })).collect();
    Ok(json!({
        "steam": steam.display().to_string(), "user": user, "add": add, "update": same, "remove": remove, "others": others,
        "without_program": without, "running": running(),
    }))
}

/// Write the app's shortcuts for a user (D28): only while Steam is closed (`steam_running`
/// says), shortcuts.vdf backed up first, every shortcut that isn't the app's kept, the
/// artwork copied to the grid folder.
pub fn write_shortcuts(lib: &Library, data_dir: &Path, steam: &Path, user: &str, steam_running: bool) -> Result<Value> {
    if steam_running {
        bail!("Steam is running: close it (Steam › Exit), then add the shortcuts.");
    }
    let (mine, _) = shortcuts(lib);
    let list: Vec<Shortcut> = mine.iter().map(|(_, s, _)| s.clone()).collect();
    let (others, backup) = gamerepo_steam::put_shortcuts(steam, user, TAG, &list, "gamerepo", &stamp())?;
    let dir = config_dir(steam, user)?;
    // the artwork: what the app wrote before goes, then this time's
    let grid = dir.join("grid");
    let rec_path = record_path(data_dir, user);
    let rec = crate::config::read_json_object(&rec_path);
    for f in rec["grid"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let _ = std::fs::remove_file(grid.join(f));
    }
    let mut written = vec![];
    for (_, s, media) in &mine {
        for (kind, suffix) in [("cover", "p"), ("banner", ""), ("hero", "_hero"), ("logo", "_logo"), ("icon", "_icon")] {
            let Some(src) = media.get(kind) else { continue };
            let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
            let name = format!("{}{suffix}.{ext}", s.appid);
            std::fs::create_dir_all(&grid)?;
            if std::fs::copy(src, grid.join(&name)).is_ok() {
                written.push(name);
            }
        }
    }
    crate::config::write_json(
        &rec_path,
        &json!({ "steam": steam.display().to_string(), "user": user, "written": now(), "backup": backup.display().to_string(),
                 "appids": mine.iter().map(|(_, s, _)| s.appid).collect::<Vec<_>>(), "grid": written }),
    )?;
    Ok(json!({ "shortcuts": mine.len(), "others": others, "art": written.len(), "backup": backup.display().to_string() }))
}

/// A backup's time: `20261020-120000`.
fn stamp() -> String {
    now().replace([':', '-'], "").replace('T', "-").replace('Z', "")
}

/// Whether the app's shortcut for a PC game is in what it last wrote for any Steam user
/// (Remove from the library… says it stays until Add to Steam… is run again, D129).
pub fn added(lib: &Library, data_dir: &Path, id: &str) -> bool {
    let Some(appid) = shortcuts(lib).0.into_iter().find(|(g, _, _)| g == id).map(|(_, s, _)| s.appid) else { return false };
    std::fs::read_dir(data_dir.join("steam")).into_iter().flatten().flatten().any(|e| {
        e.path().extension().is_some_and(|x| x == "json")
            && crate::config::read_json_object(&e.path())["appids"].as_array().is_some_and(|a| a.iter().any(|x| x.as_u64() == Some(u64::from(appid))))
    })
}

/// When the app last wrote a user's shortcuts.
pub fn last_written(data_dir: &Path, user: &str) -> Value {
    let rec = crate::config::read_json_object(&record_path(data_dir, user));
    if rec.get("written").is_none() {
        return Value::Null;
    }
    json!({ "written": rec["written"], "shortcuts": rec["appids"].as_array().map_or(0, Vec::len), "backup": rec["backup"] })
}

/// Take the app's shortcuts (and the artwork it added) out of Steam, keeping every
/// other shortcut as it is now; shortcuts.vdf is backed up first, as for a write.
pub fn remove_shortcuts(data_dir: &Path, steam: &Path, user: &str, steam_running: bool) -> Result<Value> {
    if steam_running {
        bail!("Steam is running: close it (Steam › Exit) first.");
    }
    let dir = config_dir(steam, user)?;
    let mut removed = 0;
    if dir.join("shortcuts.vdf").is_file() {
        removed = gamerepo_steam::tagged_ids(steam, user, TAG).len();
        gamerepo_steam::put_shortcuts(steam, user, TAG, &[], "gamerepo", &stamp())?;
    }
    let rec_path = record_path(data_dir, user);
    let rec = crate::config::read_json_object(&rec_path);
    let grid = dir.join("grid");
    let mut art = 0;
    for f in rec["grid"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if std::fs::remove_file(grid.join(f)).is_ok() {
            art += 1;
        }
    }
    let _ = std::fs::remove_file(&rec_path);
    Ok(json!({ "removed": removed, "art_removed": art }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn a_vdf_file_reads_and_writes_back_the_same() {
        let other = Vdf::Map(vec![
            (b"appid".to_vec(), Vdf::Int(0x9234_5678)),
            (b"AppName".to_vec(), Vdf::Str("Someone else's é".as_bytes().to_vec())),
            (b"Exe".to_vec(), Vdf::Str(b"\"C:\\x.exe\"".to_vec())),
            (b"LastPlayTime".to_vec(), Vdf::Int(1_700_000_000)),
            (b"weird".to_vec(), Vdf::Raw(0x07, vec![1, 2, 3, 4, 5, 6, 7, 8])),
            (b"tags".to_vec(), Vdf::Map(vec![(b"0".to_vec(), Vdf::Str(b"Favorites".to_vec()))])),
        ]);
        let top = vec![(b"shortcuts".to_vec(), Vdf::Map(vec![(b"0".to_vec(), other.clone())]))];
        let bytes = write(&top);
        assert_eq!(bytes[0], 0x00);
        assert_eq!(&bytes[1..11], b"shortcuts\0");
        assert_eq!(&bytes[bytes.len() - 2..], &[0x08, 0x08]);
        assert_eq!(parse(&bytes).unwrap(), top);
        assert_eq!(other.str("appname").as_deref(), Some("Someone else's é"));
        assert!(!is_ours(&other));
        // Steam's rule for a shortcut's id
        assert_eq!(shortcut_id("\"C:\\Games\\game.exe\"", "Game") & 0x8000_0000, 0x8000_0000);
        assert_eq!(account_id("76561197960287930"), Some(22202));
    }

    #[test]
    fn shortcuts_are_written_beside_the_others_backed_up_and_put_back() {
        let dir = temp_dir("steam-write");
        let lib = Library::open(dir.join("lib")).unwrap();
        // a PC game with a program and a cover
        let game_dir = pcgames::folder(&lib).join("Zeta Quest (1999)");
        std::fs::create_dir_all(game_dir.join("extracted")).unwrap();
        std::fs::write(game_dir.join("extracted/zeta.exe"), b"exe").unwrap();
        let g = pcgames::PcGame {
            id: "pc-1".into(), title: "Zeta Quest".into(), year: Some(1999),
            launch: Some(pcgames::Launch { program: "extracted/zeta.exe".into(), ..Default::default() }), ..Default::default()
        };
        crate::config::write_json(&game_dir.join("game.json"), &serde_json::to_value(&g).unwrap()).unwrap();
        pcgames::save_media(&lib, "pc-1", "cover", "jpg", b"jpeg").unwrap();
        // Steam with one user and a shortcut of their own
        let steam = dir.join("Steam");
        let cfg = steam.join("userdata/22202/config");
        std::fs::create_dir_all(&cfg).unwrap();
        std::fs::create_dir_all(steam.join("config")).unwrap();
        std::fs::write(steam.join("config/loginusers.vdf"), "\"users\"\n{\n\t\"76561197960287930\"\n\t{\n\t\t\"AccountName\"\t\t\"jerred\"\n\t\t\"PersonaName\"\t\t\"Jerred\"\n\t\t\"MostRecent\"\t\t\"1\"\n\t}\n}\n").unwrap();
        let theirs = Vdf::Map(vec![(b"appid".to_vec(), Vdf::Int(0x9000_0001)), (b"AppName".to_vec(), Vdf::Str(b"Theirs".to_vec()))]);
        let before = write(&[(b"shortcuts".to_vec(), Vdf::Map(vec![(b"0".to_vec(), theirs.clone())]))]);
        std::fs::write(cfg.join("shortcuts.vdf"), &before).unwrap();
        let u = users(&steam);
        assert_eq!((u[0]["id"].as_str(), u[0]["name"].as_str(), u[0]["recent"].as_bool()), (Some("22202"), Some("Jerred"), Some(true)));
        let p = preview(&lib, &steam, "22202").unwrap();
        assert_eq!((p["add"].as_array().unwrap().len(), p["others"].as_u64()), (1, Some(1)));
        assert!(write_shortcuts(&lib, &dir.join("data"), &steam, "22202", true).is_err(), "not while Steam runs");
        let r = write_shortcuts(&lib, &dir.join("data"), &steam, "22202", false).unwrap();
        assert_eq!((r["shortcuts"].as_u64(), r["others"].as_u64(), r["art"].as_u64()), (Some(1), Some(1), Some(1)));
        let list = shortcuts_of(&parse(&std::fs::read(cfg.join("shortcuts.vdf")).unwrap()).unwrap());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], theirs, "the user's own shortcut is kept as it was");
        let mine = &list[1];
        assert!(is_ours(mine));
        let exe = mine.str("Exe").unwrap();
        assert!(exe.starts_with('"') && exe.ends_with("zeta.exe\""));
        let appid = mine.int("appid").unwrap();
        assert_eq!(appid, shortcut_id(&exe, "Zeta Quest"));
        assert!(cfg.join(format!("grid/{appid}p.jpg")).is_file());
        // again: the same shortcut, not a second one
        write_shortcuts(&lib, &dir.join("data"), &steam, "22202", false).unwrap();
        let p = preview(&lib, &steam, "22202").unwrap();
        assert_eq!((p["add"].as_array().unwrap().len(), p["update"].as_array().unwrap().len()), (0, 1));
        // taken out: the user's own shortcut stays, the artwork goes
        let r = remove_shortcuts(&dir.join("data"), &steam, "22202", false).unwrap();
        assert_eq!(r["removed"].as_u64(), Some(1));
        let back = shortcuts_of(&parse(&std::fs::read(cfg.join("shortcuts.vdf")).unwrap()).unwrap());
        assert_eq!(back, [theirs]);
        assert!(!cfg.join(format!("grid/{appid}p.jpg")).exists());
        let backups = std::fs::read_dir(&cfg).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("shortcuts.vdf.gamerepo-")).count();
        assert!(backups >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
