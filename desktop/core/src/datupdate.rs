//! Update DATs (D172): the newest of every system's DATs in one go, from Settings ›
//! DAT files or Import › DAT files. Redump's own for the disc systems Redump has
//! (redump.rs), and libretro's copies (starter.rs) for the rest and for any disc system
//! whose Redump DAT couldn't be had. The systems are those with a DAT, or every system
//! the app knows. Each version already imported is kept (D173): a new one is used unless
//! an older one was chosen, and *Use this version* goes back to any of them.

use crate::index::Index;
use crate::library::{now, Library};
use crate::platforms::{self, PLATFORMS};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};

/// What to update, and where from.
pub struct Ask {
    /// libretro's copies (`dat_source`) and redump.org (`redump_source`)
    pub libretro: Option<String>,
    pub redump: Option<String>,
    /// every system the app knows, not only those with a DAT
    pub every: bool,
}

/// The systems an update covers: those with a DAT (their custom DATs aside), or every
/// one the app knows; arcade systems apart, as their DATs come with MAME's or FBNeo's
/// releases.
pub fn systems(ix: &Index, every: bool) -> Result<(Vec<String>, Vec<String>)> {
    let have: BTreeSet<String> =
        ix.conn().prepare("SELECT DISTINCT platform FROM dats WHERE version != 'custom'")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    let all: Vec<String> = if every { PLATFORMS.iter().map(|p| p.id.to_string()).chain(have.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect() } else { have.into_iter().collect() };
    Ok(all.into_iter().partition(|p| !platforms::is_arcade(p)))
}

/// Download the newest DATs and import them; what came, by system.
pub fn update(ix: &Index, lib: &Library, ask: &Ask, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    crate::dat::sync_library(ix, lib)?;
    let (wanted, arcade) = systems(ix, ask.every)?;
    let (mut imported, mut failed, mut redump_failed): (Vec<(Value, &str)>, HashMap<String, String>, Vec<Value>) = (vec![], HashMap::new(), vec![]);
    let mut note = None;
    // Redump's first: the disc systems it has
    let discs: Vec<String> = wanted.iter().filter(|p| platforms::redump_slug(p).is_some()).cloned().collect();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    if let (Some(src), false) = (&ask.redump, discs.is_empty()) {
        let mut told = |mut p: Value| {
            p["text"] = json!("Downloading Redump's DATs");
            progress(p)
        };
        match crate::redump::get(ix, lib, src, &discs, cancel, &mut told) {
            Ok(r) => {
                for d in r["imported"].as_array().into_iter().flatten() {
                    covered.insert(d["platform"].as_str().unwrap_or("").to_string());
                    imported.push((d.clone(), "redump"));
                }
                redump_failed = r["failed"].as_array().cloned().unwrap_or_default();
            }
            Err(e) => redump_failed = discs.iter().map(|p| json!({ "platform": p, "error": format!("{e:#}") })).collect(),
        }
    }
    // then libretro's copies, for the rest
    let rest: Vec<String> = wanted.iter().filter(|p| !covered.contains(*p)).cloned().collect();
    if let (Some(src), false, false) = (&ask.libretro, rest.is_empty(), cancel.load(Ordering::Relaxed)) {
        let mut told = |mut p: Value| {
            p["text"] = json!("Downloading libretro's copies");
            progress(p)
        };
        match crate::starter::get(ix, lib, src, &rest, true, cancel, &mut told) {
            Ok(r) => {
                for d in r["imported"].as_array().into_iter().flatten() {
                    covered.insert(d["platform"].as_str().unwrap_or("").to_string());
                    imported.push((d.clone(), "libretro"));
                }
                for f in r["failed"].as_array().into_iter().flatten() {
                    failed.insert(f["platform"].as_str().unwrap_or("").to_string(), f["error"].as_str().unwrap_or("").to_string());
                }
                note = r["note"].as_str().map(String::from);
            }
            Err(e) => {
                for p in &rest {
                    failed.insert(p.clone(), format!("{e:#}"));
                }
            }
        }
    }
    // Redump's couldn't be had: said, unless libretro's copy came instead
    let mut instead = vec![];
    for f in redump_failed {
        let p = f["platform"].as_str().unwrap_or("").to_string();
        if covered.contains(&p) {
            instead.push(f);
        } else {
            failed.entry(p).or_insert_with(|| f["error"].as_str().unwrap_or("").to_string());
        }
    }
    let stopped = cancel.load(Ordering::Relaxed);
    if !stopped && imported.is_empty() && !failed.is_empty() && failed.len() == wanted.len() {
        let first = failed.values().next().cloned().unwrap_or_default();
        anyhow::bail!("{first}");
    }
    // what each new version is now: used, or kept beside the version chosen
    let current: HashMap<i64, bool> = ix.conn().prepare("SELECT id, current FROM dats")?.query_map([], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? == 1)))?.collect::<rusqlite::Result<_>>()?;
    let in_use = |platform: &str, name: &str| -> Option<String> {
        ix.conn().query_row("SELECT version FROM dats WHERE platform = ?1 AND name = ?2 AND current = 1", [platform, name], |r| r.get(0)).ok()
    };
    let chosen = lib.meta()["dat_current"].as_object().cloned().unwrap_or_default();
    let (mut new, mut kept) = (vec![], vec![]);
    let mut same: BTreeSet<String> = BTreeSet::new();
    for (d, from) in &imported {
        let platform = d["platform"].as_str().unwrap_or("");
        if d["already"].as_bool() == Some(true) {
            same.insert(platform.to_string());
            continue;
        }
        let used = d["id"].as_i64().and_then(|id| current.get(&id)).copied().unwrap_or(false);
        let name = d["name"].as_str().unwrap_or("");
        let mut v = json!({ "platform": platform, "platform_name": d["platform_name"], "name": name, "version": d["version"], "from": from, "used": used });
        if !used {
            v["in_use"] = json!(in_use(platform, name));
            // an older version chosen by hand stays (otherwise it's libretro's copy, which
            // Redump's own or No-Intro's is used before)
            if chosen.contains_key(&format!("{platform}/{name}")) {
                kept.push(v.clone());
            }
        }
        new.push(v);
    }
    new.sort_by(|a, b| a["platform_name"].as_str().cmp(&b["platform_name"].as_str()));
    let fresh: BTreeSet<&str> = new.iter().filter_map(|v| v["platform"].as_str()).collect();
    same.retain(|p| !fresh.contains(p.as_str()));
    let missing: Vec<&String> = wanted.iter().filter(|p| !covered.contains(*p) && !failed.contains_key(*p)).collect();
    let failed: Vec<Value> = {
        let mut f: Vec<_> = failed.into_iter().collect();
        f.sort();
        f.into_iter().map(|(p, e)| json!({ "platform": p, "error": e })).collect()
    };
    if !stopped {
        lib.update_meta(json!({ "dats_updated": now() }))?;
    }
    Ok(json!({
        "systems": wanted.len(), "new": new, "same": same, "kept": kept, "missing": missing, "failed": failed,
        "instead": instead, "arcade": arcade, "stopped": stopped, "note": note,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{dat_xml, rom, rom_xml};
    use crate::library::tests::temp_dir;
    use crate::starter::tests::serve;
    use serde_json::Map;

    #[test]
    fn every_systems_dats_are_updated_and_older_versions_kept() {
        let dir = temp_dir("datupdate");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let file = dir.join("in.dat");
        let import = |xml: String| {
            std::fs::write(&file, xml).unwrap();
            crate::dat::import(&ix, &lib, &file, &Map::new()).unwrap();
        };
        // what the library has: Game Boy's and NES's from libretro, PlayStation's libretro copy,
        // and an arcade DAT
        import(dat_xml("Nintendo - Game Boy", "20260101-000000", &[("A (USA)", None, rom_xml("A (USA).gb", &rom(1, 64)))]));
        import(dat_xml("Nintendo - Nintendo Entertainment System", "20260101-000000", &[("N (USA)", None, rom_xml("N (USA).nes", &rom(5, 64)))]));
        import(dat_xml("Sony - PlayStation", "2026.01.01", &[("P (USA)", None, rom_xml("P (USA) (Track 1).bin", &rom(2, 64)))]));
        import(dat_xml("Sony - PlayStation 2", "2026.01.01", &[("Q (USA)", None, rom_xml("Q (USA).iso", &rom(6, 64)))]));
        let mut answers = Map::new();
        answers.insert("MAME".into(), json!("mame"));
        std::fs::write(&file, dat_xml("MAME", "0.270", &[("sf2", None, rom_xml("sf2.bin", &rom(3, 64)))])).unwrap();
        crate::dat::import(&ix, &lib, &file, &answers).unwrap();
        // Game Boy's older version is chosen by hand
        let gb_old: i64 = ix.conn().query_row("SELECT id FROM dats WHERE platform = 'gb'", [], |r| r.get(0)).unwrap();
        import(dat_xml("Nintendo - Game Boy", "20260601-000000", &[("A (USA)", None, rom_xml("A (USA).gb", &rom(1, 64)))]));
        crate::dat::use_version(&ix, &lib, gb_old).unwrap();
        let full = dat_xml(
            "Sony - PlayStation",
            "2026-10-01 01-02-03",
            &[("P (USA)", None, format!("{}{}", rom_xml("P (USA) (Track 1).bin", &rom(2, 64)), rom_xml("P (USA) (Track 2).bin", &rom(4, 64))))],
        );
        let base = serve(vec![
            ("/no-intro/Nintendo%20-%20Game%20Boy.dat".into(), dat_xml("Nintendo - Game Boy", "20261001-000000", &[("A (USA)", None, rom_xml("A (USA).gb", &rom(1, 64)))]).into_bytes()),
            ("/no-intro/Nintendo%20-%20Nintendo%20Entertainment%20System.dat".into(), dat_xml("Nintendo - Nintendo Entertainment System", "20260101-000000", &[("N (USA)", None, rom_xml("N (USA).nes", &rom(5, 64)))]).into_bytes()),
            ("/redump/Sony%20-%20PlayStation%202.dat".into(), dat_xml("Sony - PlayStation 2", "2026.10.07", &[("Q (USA)", None, rom_xml("Q (USA).iso", &rom(6, 64)))]).into_bytes()),
            ("/datfile/psx/".into(), full.into_bytes()),
            // Redump gives PlayStation 2's only to people signed in: libretro's copy instead
            ("/datfile/ps2/".into(), b"<!DOCTYPE html><html><body>Sign in</body></html>".to_vec()),
        ]);
        let ask = Ask { libretro: Some(format!("{base}/no-intro/")), redump: Some(format!("{base}/datfile/")), every: false };
        let r = update(&ix, &lib, &ask, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let new: Vec<_> = r["new"].as_array().unwrap().iter().map(|v| (v["platform"].as_str().unwrap(), v["from"].as_str().unwrap(), v["used"].as_bool().unwrap())).collect();
        assert_eq!(new, [("gb", "libretro", false), ("psx", "redump", true), ("ps2", "libretro", true)], "{r}");
        assert_eq!(r["same"], json!(["nes"]), "NES's is the newest already");
        assert_eq!(r["kept"].as_array().unwrap().len(), 1, "Game Boy keeps the version chosen: {r}");
        assert_eq!(r["kept"][0]["in_use"], json!("20260101-000000"));
        assert_eq!(r["instead"][0]["platform"], json!("ps2"), "{r}");
        assert_eq!(r["arcade"], json!(["mame"]));
        assert!(r["failed"].as_array().unwrap().is_empty(), "{r}");
        assert!(lib.meta()["dats_updated"].is_string());
        // every version is kept, as a file and in the list
        let gb: Vec<String> = std::fs::read_dir(lib.root().join("_library/dats/gb")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(gb.len(), 3, "{gb:?}");
        let list = crate::dat::list(&ix).unwrap();
        let gbs: Vec<_> = list.as_array().unwrap().iter().filter(|d| d["platform"] == "gb").map(|d| (d["version"].as_str().unwrap(), d["current"].as_bool().unwrap(), d["newest"].as_bool().unwrap())).collect();
        assert_eq!(gbs, [("20261001-000000", false, true), ("20260601-000000", false, false), ("20260101-000000", true, false)]);
        // choosing the newest goes back to the newest as they come (D173)
        let newest = list.as_array().unwrap().iter().find(|d| d["platform"] == "gb" && d["newest"] == true).unwrap()["id"].as_i64().unwrap();
        crate::dat::use_version(&ix, &lib, newest).unwrap();
        assert!(lib.meta()["dat_current"].as_object().unwrap().get("gb/Nintendo - Game Boy").is_none());
        import(dat_xml("Nintendo - Game Boy", "20261101-000000", &[("A (USA)", None, rom_xml("A (USA).gb", &rom(1, 64)))]));
        let v: String = ix.conn().query_row("SELECT version FROM dats WHERE platform = 'gb' AND current = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "20261101-000000");
        // every system: those without a DAT are asked for too, arcade systems apart
        let (all, _) = systems(&ix, true).unwrap();
        assert!(all.len() > 50 && all.contains(&"snes".to_string()) && !all.contains(&"mame".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
