//! What the tests with devices share: a synthetic library, a device's ES-DE folders, a
//! server started on it, and a snapshot of a folder to compare.
#![allow(dead_code)]

use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::{dat, hash, meta, roots, scan, torrentzip};
use gamerepo_device::{self as device, Device};
use gamerepo_serve::{Options, Server};
use serde_json::json;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("gamerepo-serve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn bytes(seed: u32, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(12345);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

pub fn rom_xml(name: &str, b: &[u8]) -> String {
    let h = hash::hash_reader(&mut &b[..], b.len() as u64, name, &AtomicBool::new(false)).unwrap();
    format!("<rom name=\"{name}\" size=\"{}\" crc=\"{}\" sha1=\"{}\"/>", b.len(), h.crc32, h.sha1.unwrap())
}

pub fn zip(path: &Path, members: &[(&str, Vec<u8>)]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let entries = members
        .iter()
        .map(|(n, b)| {
            let b = b.clone();
            torrentzip::Entry { name: n.to_string(), size: b.len() as u64, open: Box::new(move || Ok(Box::new(std::io::Cursor::new(b)) as Box<dyn Read>)) }
        })
        .collect();
    torrentzip::write(path, entries, &AtomicBool::new(false)).unwrap();
}

pub fn cue(disc: &str) -> String {
    format!("FILE \"{disc}.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n")
}

pub fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8])).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// The sizes of the Game Boy games' files.
pub const BIG: usize = 6 << 20;
pub const HUGE: usize = 40 << 20;

/// A library: Game Boy games (Alpha with details, a cover and a manual; Big and Huge for
/// the downloads; Gamma in the DAT but not yet had), a PlayStation game on two discs, and
/// MAME games sharing a BIOS set. Its DATs are imported with an index of Game Repo's own;
/// the server makes its own.
pub fn library(dir: &Path) -> PathBuf {
    let lib_dir = dir.join("lib");
    let lib = Library::open(&lib_dir).unwrap();
    lib.update_meta(json!({ "name": "Test Library" })).unwrap();
    let ix = Index::open(&dir.join("game-repo-index.sqlite"), &lib.id().unwrap()).unwrap();
    // Game Boy
    let gb = dir.join("Roms/GB");
    std::fs::create_dir_all(&gb).unwrap();
    let games = [("Alpha (Europe)", bytes(1, 32 * 1024)), ("Big (World)", bytes(2, BIG)), ("Huge (World)", bytes(3, HUGE))];
    let mut xml = String::new();
    for (n, b) in &games {
        std::fs::write(gb.join(format!("{n}.gb")), b).unwrap();
        xml.push_str(&format!("<game name=\"{n}\"><description>{n}</description>{}</game>\n", rom_xml(&format!("{n}.gb"), b)));
    }
    xml.push_str(&format!("<game name=\"Gamma (Japan)\"><description>Gamma (Japan)</description>{}</game>\n", rom_xml("Gamma (Japan).gb", &bytes(4, 16 * 1024))));
    std::fs::write(dir.join("gb.dat"), format!("<?xml version=\"1.0\"?>\n<datafile><header><name>Nintendo - Game Boy</name><version>1</version></header>\n{xml}</datafile>\n")).unwrap();
    // PlayStation, a game on two discs
    let psx = dir.join("Roms/PSX");
    std::fs::create_dir_all(&psx).unwrap();
    let mut xml = String::new();
    for (i, n) in ["Quest (USA) (Disc 1)", "Quest (USA) (Disc 2)"].iter().enumerate() {
        let track = bytes(10 + i as u32, 2352 * 4);
        let cue = cue(n);
        std::fs::write(psx.join(format!("{n}.bin")), &track).unwrap();
        std::fs::write(psx.join(format!("{n}.cue")), &cue).unwrap();
        xml.push_str(&format!("<game name=\"{n}\"><description>{n}</description>{}{}</game>\n", rom_xml(&format!("{n}.cue"), cue.as_bytes()), rom_xml(&format!("{n}.bin"), &track)));
    }
    std::fs::write(dir.join("psx.dat"), format!("<?xml version=\"1.0\"?>\n<datafile><header><name>Sony - PlayStation</name><version>1</version></header>\n{xml}</datafile>\n")).unwrap();
    // MAME: two games needing one BIOS set
    let mame = dir.join("Roms/MAME");
    let b = |n: u32| bytes(100 + n, 64);
    zip(&mame.join("bios1.zip"), &[("bios.bin", b(1))]);
    zip(&mame.join("beta.zip"), &[("b.1", b(2))]);
    zip(&mame.join("kappa.zip"), &[("k.1", b(3))]);
    std::fs::write(
        dir.join("mame.xml"),
        format!(
            r#"<?xml version="1.0"?><mame build="0.271 (mame0271)">
<machine name="bios1" isbios="yes"><description>Test BIOS</description>{}</machine>
<machine name="beta" romof="bios1"><description>Beta Quest</description>{}{}</machine>
<machine name="kappa" romof="bios1"><description>Kappa Kart</description>{}{}</machine>
</mame>"#,
            rom_xml("bios.bin", &b(1)),
            rom_xml("bios.bin", &b(1)).replace("<rom ", "<rom merge=\"bios.bin\" "),
            rom_xml("b.1", &b(2)),
            rom_xml("bios.bin", &b(1)).replace("<rom ", "<rom merge=\"bios.bin\" "),
            rom_xml("k.1", &b(3)),
        ),
    )
    .unwrap();
    roots::add(&lib, &gb, Some("gb")).unwrap();
    roots::add(&lib, &psx, Some("psx")).unwrap();
    roots::add(&lib, &mame, None).unwrap();
    scan::scan(&ix, &lib, &scan::Options::default(), &AtomicBool::new(false), &mut |_| {}).unwrap();
    for d in ["gb.dat", "psx.dat", "mame.xml"] {
        dat::import(&ix, &lib, &dir.join(d), &serde_json::Map::new()).unwrap();
    }
    // Alpha's details, cover and manual
    let g = meta::dat_game_named(ix.conn(), &lib, "gb", "Alpha (Europe)").unwrap().unwrap();
    let gdir = meta::ensure_folder(&ix, &lib, &g).unwrap();
    let mut gj = meta::read_game(&gdir);
    gj["details"] = json!({ "description": "A & B", "developer": "Dev Co" });
    meta::write_game(&ix, &lib, &gdir, &gj).unwrap();
    std::fs::create_dir_all(gdir.join("media")).unwrap();
    std::fs::write(gdir.join("media").join("cover.png"), png(800, 600)).unwrap();
    std::fs::create_dir_all(gdir.join("docs")).unwrap();
    std::fs::write(gdir.join("docs").join("Alpha Manual.pdf"), b"%PDF-1.4\n%%EOF\n").unwrap();
    meta::sync(&ix, &lib).unwrap();
    lib_dir
}

/// A device: ES-DE's folder with its settings, and a gamelist with an entry of its own.
pub fn a_device(dir: &Path) -> Device {
    let home = dir.join("device");
    std::fs::create_dir_all(home.join("ES-DE/settings")).unwrap();
    std::fs::write(home.join("ES-DE/settings/es_settings.xml"), "<?xml version=\"1.0\"?>\n<string name=\"ROMDirectory\" value=\"\" />\n").unwrap();
    std::fs::create_dir_all(home.join("ES-DE/gamelists/gb")).unwrap();
    std::fs::write(home.join("ES-DE/gamelists/gb/gamelist.xml"), "<?xml version=\"1.0\"?>\n<gameList>\n\t<game>\n\t\t<path>./Mine.gb</path>\n\t\t<name>Mine</name>\n\t\t<playcount>4</playcount>\n\t</game>\n</gameList>\n").unwrap();
    Device::open(&home).unwrap()
}

pub fn start(lib: &Path, data: &Path) -> Server {
    let s = Server::start(Options { library: lib.to_path_buf(), data: data.to_path_buf(), port: 0, announce: false, watch: false, ..Default::default() }).unwrap();
    assert!(s.wait_ready(Duration::from_secs(120)), "the server read the library: {:?}", s.status());
    s
}

pub fn address(s: &Server) -> String {
    format!("127.0.0.1:{}", s.port())
}

pub fn paired(s: &Server) -> device::Server {
    let (code, _) = s.pair();
    device::client::pair(&address(s), &code, "Test phone", "test").unwrap()
}

/// Every file under `dir` with its bytes and modified time.
pub fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>, Option<std::time::SystemTime>)> {
    let mut v = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let m = std::fs::metadata(&p).unwrap();
                v.push((p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap(), m.modified().ok()));
            }
        }
    }
    v.sort();
    v
}

pub fn files_under(dir: &Path) -> Vec<String> {
    snapshot(dir).into_iter().map(|(p, _, _)| p).collect()
}

pub fn status_of(e: &anyhow::Error) -> u16 {
    e.downcast_ref::<device::client::Refused>().map(|r| r.status).unwrap_or(0)
}

