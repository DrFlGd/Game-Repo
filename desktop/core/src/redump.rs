//! Redump's own DATs (D167): each disc system's, downloaded from redump.org as its
//! Downloads page gives them (a ZIP holding the DAT) and imported, when asked (*Get
//! Redump's DATs…*, or *Add a system…* › Download its DAT). They're the full check
//! libretro's copies can't give, as those list one track of each disc; one of Redump's is
//! used before libretro's copy of it, whatever the dates (dat::libretro_copy). Redump's
//! change most days, so each one asked for is downloaded again, and a version already
//! imported is just kept. Where each system's is comes from `platforms::redump_slug`;
//! the address is a setting on this computer (`redump_source`), in case Redump moves them.

use crate::index::Index;
use crate::library::Library;
use crate::platforms::{self, PLATFORMS};
use anyhow::{bail, Result};
use serde_json::{json, Map, Value};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicBool, Ordering};

/// Where redump.org keeps its DATs, one `<system>/` each.
pub const SOURCE: &str = "http://redump.org/datfile/";

/// What redump.org sent, as DATs (file name, text): the `.dat` and `.xml` files in a ZIP,
/// or a DAT as it is. Anything else (a page asking to sign in) isn't one.
fn dats_in(bytes: Vec<u8>, slug: &str) -> Result<Vec<(String, Vec<u8>)>> {
    if bytes.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| anyhow::anyhow!("redump.org's ZIP couldn't be read: {e}"))?;
        let mut out = vec![];
        for i in 0..zip.len() {
            let mut m = zip.by_index(i)?;
            let name = m.name()?.into_owned();
            let ext = platforms::ext_of(&name);
            if m.is_dir() || (ext != "dat" && ext != "xml") {
                continue;
            }
            let mut text = vec![];
            m.read_to_end(&mut text)?;
            out.push((name.rsplit('/').next().unwrap_or(&name).to_string(), text));
        }
        if out.is_empty() {
            bail!("redump.org's ZIP has no DAT in it");
        }
        return Ok(out);
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).trim_start_matches('\u{feff}').trim_start().to_ascii_lowercase();
    if ["<?xml", "<datafile", "<!doctype datafile", "clrmamepro"].iter().any(|s| head.starts_with(s)) {
        return Ok(vec![(format!("{slug}.dat"), bytes)]);
    }
    bail!("redump.org didn't send a DAT (it may give this one only to people signed in there): download it from redump.org and import it")
}

/// Download and import Redump's DAT of each system in `only` (every one Redump has when
/// empty).
pub fn get(ix: &Index, lib: &Library, source: &str, only: &[String], cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    crate::dat::sync_library(ix, lib)?;
    let base = if source.ends_with('/') { source.to_string() } else { format!("{source}/") };
    let wanted: Vec<_> = PLATFORMS
        .iter()
        .filter_map(|p| platforms::redump_slug(p.id).map(|slug| (p, slug)))
        .filter(|(p, _)| only.is_empty() || only.iter().any(|o| o == p.id))
        .collect();
    if wanted.is_empty() {
        bail!("Redump has no DATs for {}.", only.iter().map(|o| platforms::name_of(o)).collect::<Vec<_>>().join(", "));
    }
    let (mut imported, mut missing, mut failed) = (vec![], vec![], vec![]);
    for (i, (p, slug)) in wanted.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(json!({ "text": "Downloading", "item": i + 1, "items": wanted.len(), "name": p.name }));
        let got = crate::starter::fetch(&format!("{base}{}/", crate::starter::encode(slug))).and_then(|b| b.map(|b| dats_in(b, slug)).transpose());
        let dats = match got {
            Ok(Some(d)) => d,
            Ok(None) => {
                missing.push(json!(p.id));
                continue;
            }
            Err(e) => {
                failed.push(json!({ "platform": p.id, "error": format!("{e:#}") }));
                continue;
            }
        };
        for (file, bytes) in dats {
            // (it's for this system, whatever its name says)
            let mut answers = Map::new();
            if let Ok(d) = crate::dat::parse(&bytes) {
                if platforms::from_dat_name(&d.name).map(|x| x.id) != Some(p.id) {
                    answers.insert(d.name.clone(), json!(p.id));
                }
            }
            match crate::dat::import_bytes(ix, lib, &bytes, &file, &answers) {
                Ok(crate::dat::Imported::Loaded(v)) => imported.push(v),
                Ok(_) => failed.push(json!({ "platform": p.id, "error": "its system couldn't be worked out" })),
                Err(e) => failed.push(json!({ "platform": p.id, "error": format!("{e:#}") })),
            }
        }
    }
    if imported.is_empty() && !failed.is_empty() && missing.is_empty() && !cancel.load(Ordering::Relaxed) {
        bail!("{}", failed[0]["error"].as_str().unwrap_or("the downloads failed"));
    }
    Ok(json!({ "imported": imported, "missing": missing, "failed": failed, "stopped": cancel.load(Ordering::Relaxed), "source": base }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{dat_xml, rom, rom_xml};
    use crate::library::tests::temp_dir;
    use crate::starter::tests::serve;

    #[test]
    fn redumps_dats_are_downloaded_and_used_before_libretros_copies() {
        let dir = temp_dir("redump");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        // libretro's copy first, one track a disc, dated later than Redump's own
        let one_track = dat_xml("Sony - PlayStation", "2026.10.07", &[("A (USA)", None, rom_xml("A (USA) (Track 1).bin", &rom(1, 64)))]);
        let copy = dir.join("libretro.dat");
        std::fs::write(&copy, one_track).unwrap();
        crate::dat::import(&ix, &lib, &copy, &Map::new()).unwrap();
        // Redump's: a ZIP holding the DAT, as redump.org gives it
        let full = dat_xml(
            "Sony - PlayStation",
            "2026-10-01 01-02-03",
            &[("A (USA)", None, format!("{}{}", rom_xml("A (USA) (Track 1).bin", &rom(1, 64)), rom_xml("A (USA) (Track 2).bin", &rom(2, 64))))],
        );
        let zipped = dir.join("psx.zip");
        crate::audit::tests::zip(&zipped, &[("Sony - PlayStation - Datfile (1) (2026-10-01 01-02-03).dat", full.as_bytes())]);
        let ps5 = dat_xml("Sony - PlayStation 5", "2026-10-02 01-02-03", &[("B (Japan)", None, rom_xml("B (Japan).iso", &rom(3, 64)))]);
        let base = serve(vec![
            ("/datfile/psx/".into(), std::fs::read(&zipped).unwrap()),
            // a DAT as it is, and a page that isn't one
            ("/datfile/ps5/".into(), ps5.into_bytes()),
            ("/datfile/ps4/".into(), b"<!DOCTYPE html><html><body>Sign in</body></html>".to_vec()),
        ]);
        let source = format!("{base}/datfile");
        let only = ["psx", "ps5", "ps4", "saturn", "nes"].map(String::from);
        let r = get(&ix, &lib, &source, &only, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let got: Vec<_> = r["imported"].as_array().unwrap().iter().map(|d| (d["platform"].as_str().unwrap(), d["roms"].as_u64().unwrap())).collect();
        assert_eq!(got, [("psx", 2), ("ps5", 1)], "{r}");
        assert_eq!(r["missing"], json!(["saturn"]), "a system Redump has nothing at; NES isn't one of Redump's");
        assert!(r["failed"][0]["error"].as_str().unwrap().contains("signed in"), "{r}");
        // Redump's is the version in use, though libretro's copy is dated later
        let current: Vec<(String, i64)> = ix.conn().prepare("SELECT version, roms FROM dats WHERE platform = 'psx' AND current = 1").unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(current, [("2026-10-01 01-02-03".to_string(), 2)]);
        // and stays so when libretro's next copy comes
        std::fs::write(&copy, dat_xml("Sony - PlayStation", "2026.10.20", &[("A (USA)", None, rom_xml("A (USA) (Track 1).bin", &rom(1, 64)))])).unwrap();
        crate::dat::import(&ix, &lib, &copy, &Map::new()).unwrap();
        let v: String = ix.conn().query_row("SELECT version FROM dats WHERE platform = 'psx' AND current = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(v, "2026-10-01 01-02-03");
        assert!(crate::dat::libretro_copy("2026.10.20") && !crate::dat::libretro_copy("20261001-123456") && !crate::dat::libretro_copy("2026-10-01 01-02-03"));
        // asked again, the version already imported is kept as it is
        let again = get(&ix, &lib, &source, &["psx".to_string()], &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(again["imported"][0]["already"], json!(true), "{again}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
