//! Starter DATs (D17): the No-Intro cartridge DATs and the Redump disc DATs as
//! libretro keeps them in its public database, downloaded and imported when asked
//! (*Get starter DATs*). They can be a little behind No-Intro's and Redump's own, and
//! libretro's Redump copies list only one track of each disc (the one it identifies
//! games by), so a disc's other tracks and its cue show as unknown until Redump's own
//! DAT is imported; a newer DAT imported by hand becomes the version in use. The
//! address is a setting on this computer (`dat_source`), in case libretro moves them;
//! the Redump copies are looked for in `redump/` beside its `no-intro/`, and the BIOS
//! DAT (libretro's `System.dat`, D41) in `dat/` beside `metadat/`.

use crate::index::Index;
use crate::library::Library;
use crate::platforms::{self, PLATFORMS};
use anyhow::Result;
use serde_json::{json, Map, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

/// Where libretro keeps its copies of the No-Intro DATs, one `<DAT name>.dat` each.
pub const SOURCE: &str = "https://raw.githubusercontent.com/libretro/libretro-database/master/metadat/no-intro/";

/// DATs are read up to this size (the largest cartridge DATs are a few MB, disc DATs tens).
const LIMIT: u64 = 200 << 20;

/// What the page says after disc DATs came from libretro.
pub const DISC_NOTE: &str = "libretro's copies of Redump's DATs list one track of each disc, so a disc's other tracks and its cue sheet show as unknown files. Import Redump's own DAT (redump.org, Downloads) for the full check.";

/// "Nintendo - Game Boy" -> "Nintendo%20-%20Game%20Boy" (a path segment).
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The file at `url`, or None when the server says it isn't there.
fn fetch(url: &str) -> Result<Option<Vec<u8>>> {
    match ureq::get(url).header("User-Agent", concat!("GameRepo/", env!("CARGO_PKG_VERSION"))).call() {
        Ok(mut r) => {
            let mut out = vec![];
            r.body_mut().with_config().limit(LIMIT).reader().read_to_end(&mut out)?;
            Ok(Some(out))
        }
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(anyhow::anyhow!("couldn't download {url}: {e}")),
    }
}

/// Download and import the starter DAT of each platform in `only` (all the platforms
/// the app knows when empty) that has no DAT yet, or of every one with `again`.
pub fn get(ix: &Index, lib: &Library, source: &str, only: &[String], again: bool, cancel: &AtomicBool, progress: &mut dyn FnMut(Value)) -> Result<Value> {
    lib.writable()?;
    crate::dat::sync_library(ix, lib)?;
    let base = if source.ends_with('/') { source.to_string() } else { format!("{source}/") };
    let redump = base.strip_suffix("no-intro/").map(|b| format!("{b}redump/")).unwrap_or_else(|| base.clone());
    // the BIOS DAT is libretro's dat/System.dat, beside metadat/ (bios.rs)
    let db = base.strip_suffix("metadat/no-intro/").or_else(|| base.strip_suffix("no-intro/")).unwrap_or(&base).to_string();
    let mut discs = false;
    let have: Vec<String> = ix.conn().prepare("SELECT DISTINCT platform FROM dats")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    // arcade DATs come with MAME's or FBNeo's releases, not libretro's No-Intro and Redump copies
    let wanted: Vec<_> = PLATFORMS.iter().filter(|p| if only.is_empty() { !platforms::is_arcade(p.id) } else { only.iter().any(|o| o == p.id) }).collect();
    let (mut imported, mut skipped, mut missing, mut failed) = (vec![], vec![], vec![], vec![]);
    for (i, p) in wanted.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        progress(json!({ "text": "Downloading", "item": i + 1, "items": wanted.len(), "name": p.name }));
        if !again && have.iter().any(|h| h == p.id) {
            skipped.push(json!(p.id));
            continue;
        }
        // the DAT's name has changed over the years: try each
        let mut found = false;
        let from = if platforms::is_disc(p.id) { &redump } else { &base };
        for name in p.dat_names {
            let url = if p.id == crate::bios::ID { format!("{db}dat/System.dat") } else { format!("{from}{}.dat", encode(name)) };
            match fetch(&url) {
                Ok(Some(bytes)) => {
                    let mut answers = Map::new();
                    if let Ok(d) = crate::dat::parse(&bytes) {
                        if platforms::from_dat_name(&d.name).map(|x| x.id) != Some(p.id) {
                            answers.insert(d.name.clone(), json!(p.id));
                        }
                    }
                    match crate::dat::import_bytes(ix, lib, &bytes, &format!("{name}.dat"), &answers) {
                        Ok(crate::dat::Imported::Loaded(v)) => {
                            discs |= platforms::is_disc(p.id);
                            imported.push(v)
                        }
                        Ok(_) => failed.push(json!({ "platform": p.id, "error": "its system couldn't be worked out" })),
                        Err(e) => failed.push(json!({ "platform": p.id, "error": format!("{e:#}") })),
                    }
                    found = true;
                    break;
                }
                Ok(None) => continue,
                Err(e) => {
                    failed.push(json!({ "platform": p.id, "error": format!("{e:#}") }));
                    found = true;
                    break;
                }
            }
        }
        if !found {
            missing.push(json!(p.id));
        }
    }
    if imported.is_empty() && !failed.is_empty() && failed.len() == wanted.len() - skipped.len() {
        anyhow::bail!("{}", failed[0]["error"].as_str().unwrap_or("the downloads failed"));
    }
    Ok(json!({
        "imported": imported, "skipped": skipped, "missing": missing, "failed": failed,
        "stopped": cancel.load(Ordering::Relaxed), "source": base,
        "note": discs.then_some(DISC_NOTE),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::{dat_xml, rom, rom_xml};
    use crate::library::tests::temp_dir;

    /// A web server on 127.0.0.1 answering from `files` (name -> body), 404 otherwise.
    fn serve(files: Vec<(String, String)>) -> String {
        use std::io::Write;
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for s in l.incoming().flatten() {
                let mut s = s;
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split(' ').nth(1).unwrap_or("").to_string();
                let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone());
                let (code, body) = match body { Some(b) => ("200 OK", b), None => ("404 Not Found", String::new()) };
                let _ = write!(s, "HTTP/1.1 {code}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            }
        });
        format!("http://{addr}/no-intro")
    }

    #[test]
    fn starter_dats_are_downloaded_for_platforms_without_one() {
        let dir = temp_dir("starter");
        let lib = Library::open(dir.join("Lib")).unwrap();
        let ix = Index::open(&dir.join("index.sqlite"), "lib-x").unwrap();
        let gb = dat_xml("Nintendo - Game Boy", "20261001", &[("A (USA)", None, rom_xml("A (USA).gb", &rom(1, 64)))]);
        // an older name for the platform's DAT is found too
        let a26 = dat_xml("Atari - 2600", "20260101", &[("B (USA)", None, rom_xml("B (USA).a26", &rom(2, 64)))]);
        let base = serve(vec![
            ("/no-intro/Nintendo%20-%20Game%20Boy.dat".into(), gb),
            ("/no-intro/Atari%20-%202600.dat".into(), a26),
            ("/redump/The%203DO%20Company%20-%203DO.dat".into(), dat_xml("The 3DO Company - 3DO", "2026.10.07", &[("C (USA)", None, rom_xml("C (USA).bin", &rom(3, 64)))])),
        ]);
        let only = vec!["gb".to_string(), "atari2600".to_string(), "vectrex".to_string(), "3do".to_string()];
        let r = get(&ix, &lib, &base, &only, false, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(r["imported"].as_array().unwrap().len(), 3, "{r}");
        assert_eq!(r["missing"], json!(["vectrex"]));
        assert_eq!(r["note"], json!(DISC_NOTE), "disc DATs come from redump/ beside no-intro/");
        assert!(lib.root().join("_library/dats/gb/Nintendo - Game Boy (20261001).dat").is_file());
        // a platform that has a DAT is left alone unless asked again
        let again = get(&ix, &lib, &base, &only, false, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(again["skipped"].as_array().unwrap().len(), 3);
        assert!(lib.meta()["dat_platforms"].is_null(), "nothing needed remembering");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
