//! Game Repo Server with the stand-in device client (docs/PLAN.md, "Phase 9a design",
//! "The stand-in client and the tests"): pairing, the catalogue, plans, downloads that
//! resume, games added to and removed from a device, and the library left as it was.

mod common;

use common::*;
use gamerepo_core::{hash, roots};
use gamerepo_core::library::Library;
use gamerepo_device::{self as device, AddOptions};
use gamerepo_serve::{Options, Server};
use gamerepo_wire::api::{self, Changes, Found, Identified, Page, PlanAsk, System};
use gamerepo_wire::Plan;
use serde_json::Value;
use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[test]
fn a_device_pairs_with_a_code_and_only_a_paired_device_is_answered() {
    let dir = temp("pairing");
    let lib = library(&dir);
    let data = dir.join("server");
    let s = start(&lib, &data);
    let addr = address(&s);
    // what the server is, without pairing
    let (info, fp) = device::client::info(&addr).unwrap();
    assert_eq!((info.library.as_str(), info.api), ("Test Library", api::API));
    assert_eq!(fp, s.fingerprint());
    // not paired: refused
    let stranger = device::Server { address: addr.clone(), fingerprint: fp.clone(), token: "0".repeat(64), ..Default::default() };
    assert_eq!(status_of(&stranger.get::<Vec<System>>("/v1/systems").unwrap_err()), 401);
    // no pairing going on, then wrong codes, then the fifth ends it
    assert!(device::client::pair(&addr, "123456", "Phone", "test").unwrap_err().to_string().contains("No device is being paired"));
    let (code, secs) = s.pair();
    assert_eq!(secs, 600);
    let wrong = if code == "000000" { "000001" } else { "000000" };
    for _ in 0..4 {
        assert!(device::client::pair(&addr, wrong, "Phone", "test").unwrap_err().to_string().contains("isn't the code"));
    }
    assert!(device::client::pair(&addr, wrong, "Phone", "test").unwrap_err().to_string().contains("fifth wrong code"));
    assert!(device::client::pair(&addr, &code, "Phone", "test").is_err(), "the right code is too late");
    // paired: answered, and listed with its name
    let phone = paired(&s);
    assert_eq!(phone.fingerprint, s.fingerprint());
    let systems: Vec<System> = phone.get("/v1/systems").unwrap();
    assert_eq!(systems.iter().map(|x| (x.id.as_str(), x.games)).collect::<Vec<_>>(), [("gb", 3), ("mame", 2), ("psx", 1)]);
    let devices = s.devices();
    assert_eq!((devices.len(), devices[0].name.as_str()), (1, "Test phone"));
    // removed: refused, and told to pair again
    s.remove_device(&phone.device).unwrap();
    let e = phone.get::<Vec<System>>("/v1/systems").unwrap_err();
    assert_eq!(status_of(&e), 401);
    assert!(e.to_string().contains("pair it again"), "{e}");
    // a device unpairs itself
    let tablet = paired(&s);
    tablet.call::<Value>("DELETE", "/v1/device", None::<&()>).unwrap();
    assert!(s.devices().is_empty());
    // a server with another certificate isn't talked to
    let phone = paired(&s);
    s.stop();
    std::fs::remove_file(data.join("cert.pem")).unwrap();
    std::fs::remove_file(data.join("key.pem")).unwrap();
    let s = Server::start(Options { library: lib.clone(), data: data.clone(), port: phone.address.rsplit(':').next().unwrap().parse().unwrap(), announce: false, watch: false, ..Default::default() }).unwrap();
    let e = phone.get::<Vec<System>>("/v1/systems").unwrap_err();
    assert!(e.to_string().contains("isn't the one this device paired with"), "{e}");
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn games_are_added_to_a_device_and_removed_and_the_library_is_left_as_it_was() {
    let dir = temp("add");
    let lib = library(&dir);
    let before = (snapshot(&lib), snapshot(&dir.join("Roms")));
    let s = start(&lib, &dir.join("server"));
    let phone = paired(&s);
    let d = a_device(&dir);

    // the catalogue: a game on two discs is one game, with both discs' size
    let page: Page = phone.get("/v1/games?system=psx").unwrap();
    assert_eq!(page.items.len(), 1);
    let quest = &page.items[0];
    assert_eq!((quest.key.as_str(), quest.title.as_str(), quest.discs), ("psx/Quest (USA) (Disc 1)", "Quest", 2));
    assert_eq!(quest.size, (2 * 2352 * 4 + cue("Quest (USA) (Disc 1)").len() + cue("Quest (USA) (Disc 2)").len()) as u64, "{quest:?}");
    let found: Page = phone.get("/v1/games?q=alph").unwrap();
    assert_eq!(found.items.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), ["gb/Alpha (Europe)"]);
    assert_eq!(found.items[0].picture.as_deref(), Some("cover"));
    let sorted: Page = phone.get("/v1/games?system=gb&sort=size&desc=1&limit=2").unwrap();
    assert_eq!((sorted.total, sorted.items[0].key.as_str(), sorted.items.len()), (3, "gb/Huge (World)", 2));
    let one: Value = phone.get("/v1/games/gb/Alpha%20(Europe)").unwrap();
    assert_eq!(one["details"]["developer"], "Dev Co");
    assert_eq!(one["files"][0]["name"], "Alpha (Europe).gb");

    // a plan: each media kind's size, those not asked for too
    let plan: Plan = phone.call("POST", "/v1/plans", Some(&PlanAsk { games: vec!["gb/Alpha (Europe)".into()], media: vec![], size: 320, ..Default::default() })).unwrap();
    let cover = plan.kinds.iter().find(|k| k.kind == "cover").unwrap();
    assert!(cover.estimated && cover.games == 1 && cover.size < png(800, 600).len() as u64 / 4, "{:?}", plan.kinds);
    assert!(plan.files.iter().all(|f| f.role != "cover"), "not asked for");

    // added: a Game Boy game with its cover (made smaller) and manual, and the game on two discs
    let o = AddOptions { media: vec!["cover".into()], manuals: true, size: 320, collection: "On the phone".into(), ..Default::default() };
    let added = device::add(&phone, &d, &["gb/Alpha (Europe)".into(), "psx/Quest (USA) (Disc 1)".into()], &o, &mut |_| {}).unwrap();
    assert_eq!(added.games, 2);
    let home = &d.home;
    let mut on = files_under(home);
    on.retain(|p| !p.starts_with("ES-DE/settings"));
    assert_eq!(on, [
        "ES-DE/collections/custom-On the phone.cfg",
        "ES-DE/downloaded_media/gb/covers/Alpha (Europe).png",
        "ES-DE/downloaded_media/gb/manuals/Alpha (Europe).pdf",
        "ES-DE/gamelists/gb/gamelist.xml",
        "ES-DE/gamelists/psx/gamelist.xml",
        "ES-DE/gamerepo/device.json",
        "ROMs/gb/Alpha (Europe).gb",
        "ROMs/psx/Quest (USA).m3u/Quest (USA) (Disc 1).bin",
        "ROMs/psx/Quest (USA).m3u/Quest (USA) (Disc 1).cue",
        "ROMs/psx/Quest (USA).m3u/Quest (USA) (Disc 2).bin",
        "ROMs/psx/Quest (USA).m3u/Quest (USA) (Disc 2).cue",
        "ROMs/psx/Quest (USA).m3u/Quest (USA).m3u",
    ]);
    assert_eq!(std::fs::read(home.join("ROMs/gb/Alpha (Europe).gb")).unwrap(), bytes(1, 32 * 1024));
    assert_eq!(std::fs::read_to_string(home.join("ROMs/psx/Quest (USA).m3u/Quest (USA).m3u")).unwrap(), "Quest (USA) (Disc 1).cue\nQuest (USA) (Disc 2).cue\n");
    let (w, h) = image::image_dimensions(home.join("ES-DE/downloaded_media/gb/covers/Alpha (Europe).png")).unwrap();
    assert_eq!(w.max(h), 320, "the cover made smaller for the device");
    let gl = std::fs::read_to_string(home.join("ES-DE/gamelists/gb/gamelist.xml")).unwrap();
    assert!(gl.contains("<playcount>4</playcount>") && gl.contains("<developer>Dev Co</developer>"), "the device's own entry kept, the game's added: {gl}");
    let psx_gl = std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap();
    assert_eq!(psx_gl.matches("<game>").count(), 1, "one entry for both discs: {psx_gl}");
    assert!(psx_gl.contains("<path>./Quest (USA).m3u</path>"), "{psx_gl}");
    let coll = std::fs::read_to_string(home.join("ES-DE/collections/custom-On the phone.cfg")).unwrap();
    assert!(coll.contains("%ROMPATH%/gb/Alpha (Europe).gb") && coll.contains("%ROMPATH%/psx/Quest (USA).m3u"), "{coll}");
    let rec = d.record();
    assert_eq!(rec.games.iter().map(|g| (g.key.as_str(), g.title.as_str(), g.files.len())).collect::<Vec<_>>(), [("gb/Alpha (Europe)", "Alpha", 3), ("psx/Quest (USA) (Disc 1)", "Quest", 5)]);
    // the server hears what the device holds
    assert_eq!(s.devices()[0].holding.games, 2);
    // added again: nothing fetched
    let again = device::add(&phone, &d, &["gb/Alpha (Europe)".into()], &o, &mut |_| {}).unwrap();
    assert_eq!((again.files, again.there), (0, 3));

    // removed: its files, entry and collection line; the device's own entry stays
    let r = device::remove(&d, &["psx/Quest (USA) (Disc 1)".into()]).unwrap();
    assert_eq!((r.games, r.files, r.kept), (1, 5, 0));
    assert!(!home.join("ROMs/psx").exists(), "its folders went with it");
    assert!(!std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap().contains("Quest"));
    let coll = std::fs::read_to_string(home.join("ES-DE/collections/custom-On the phone.cfg")).unwrap();
    assert!(!coll.contains("Quest") && coll.contains("Alpha"), "{coll}");
    assert!(device::remove(&d, &["psx/Quest (USA) (Disc 1)".into()]).is_err(), "not held now");

    // arcade games sharing a BIOS set: it stays while one of them is held
    let o = AddOptions { media: vec![], ..Default::default() };
    device::add(&phone, &d, &["mame/beta".into(), "mame/kappa".into()], &o, &mut |_| {}).unwrap();
    let bios = home.join("ROMs/mame/bios1.zip");
    assert!(bios.is_file() && home.join("ROMs/mame/beta.zip").is_file() && home.join("ROMs/mame/kappa.zip").is_file(), "{:?}", files_under(home));
    let r = device::remove(&d, &["mame/beta".into()]).unwrap();
    assert_eq!((r.files, r.kept), (1, 1));
    assert!(bios.is_file(), "kappa still needs it");
    device::remove(&d, &["mame/kappa".into()]).unwrap();
    assert!(!bios.exists() && !home.join("ROMs/mame").exists());

    // a device's files told apart (C18): by their CRC32s, a ZIP's members' too
    let alpha = hash::hash_reader(&mut &bytes(1, 32 * 1024)[..], 32 * 1024, "a", &AtomicBool::new(false)).unwrap();
    let kappa = hash::hash_reader(&mut &bytes(103, 64)[..], 64, "k", &AtomicBool::new(false)).unwrap();
    let ids: Vec<Identified> = phone
        .call("POST", "/v1/identify", Some(&vec![
            Found { name: "a.gb".into(), size: 32 * 1024, crcs: vec![(alpha.crc32.clone(), 32 * 1024)], chd_sha1: None },
            Found { name: "kappa.zip".into(), size: 200, crcs: vec![(kappa.crc32.clone(), 64)], chd_sha1: None },
            Found { name: "other.gb".into(), size: 3, crcs: vec![("01020304".into(), 3)], chd_sha1: None },
        ]))
        .unwrap();
    assert_eq!(ids.iter().map(|i| i.game.as_deref()).collect::<Vec<_>>(), [Some("gb/Alpha (Europe)"), Some("mame/kappa"), None]);

    // a picture as a thumbnail, asked again only when it changed
    let (head, _) = phone.open_request("GET", "/v1/pictures/gb/Alpha%20(Europe)/cover?size=128").unwrap();
    assert_eq!((head.status, head.header("content-type")), (200, Some("image/jpeg")));

    s.stop();
    // the library and the folders with games, byte for byte and time for time (D90)
    assert!(before == (snapshot(&lib), snapshot(&dir.join("Roms"))), "the server changed nothing in the library");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn downloads_resume_four_go_at_once_and_the_catalogue_follows_the_library() {
    let dir = temp("downloads");
    let lib = library(&dir);
    let s = start(&lib, &dir.join("server"));
    let phone = paired(&s);
    let plan: Plan = phone.call("POST", "/v1/plans", Some(&PlanAsk { games: vec!["gb/Big (World)".into(), "gb/Huge (World)".into()], ..Default::default() })).unwrap();
    let big = plan.files.iter().find(|f| f.path == "gb/Big (World).gb").unwrap();
    assert_eq!((big.size, big.made, big.sha1.is_some()), (BIG as u64, false, true), "a loose file's SHA-1 is the index's");

    // cut at 40%, then resumed: only the rest comes
    let part = dir.join("part/Big.gb.part");
    let first = phone.fetch(&big.url, &part, Some(BIG as u64 * 4 / 10)).unwrap();
    assert_eq!(first, BIG as u64 * 4 / 10);
    let rest = phone.fetch(&big.url, &part, None).unwrap();
    assert_eq!(rest, BIG as u64 - first);
    assert_eq!(device::sha1_file(&part).unwrap(), phone.sha1_of(&big.url, Duration::from_secs(30)).unwrap());

    // a file changed under a resume starts again
    std::fs::remove_file(&part).unwrap();
    phone.fetch(&big.url, &part, Some(1 << 20)).unwrap();
    let src = dir.join("Roms/GB/Big (World).gb");
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&src, bytes(99, BIG)).unwrap();
    let again = phone.fetch(&big.url, &part, None).unwrap();
    assert_eq!(again, BIG as u64, "the whole file again");
    assert_eq!(std::fs::read(&part).unwrap(), bytes(99, BIG));
    std::fs::write(&src, bytes(2, BIG)).unwrap();

    // four at once to a device; a fifth waits its turn
    let huge = plan.files.iter().find(|f| f.path == "gb/Huge (World).gb").unwrap();
    let held: Vec<_> = (0..4).map(|_| phone.open_request("GET", &huge.url).unwrap()).collect();
    assert!(held.iter().all(|(h, _)| h.status == 200));
    let (fifth, _) = phone.open_request("GET", &huge.url).unwrap();
    assert_eq!(fifth.status, 429);
    drop(held);
    let mut ok = false;
    for _ in 0..100 {
        if phone.open_request("GET", &huge.url).unwrap().0.status == 200 {
            ok = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(ok, "a place is free once one is done");

    // the catalogue follows the library: a game added, one gone, each with a new marker
    let page: Page = phone.get("/v1/games?system=gb").unwrap();
    let rev = page.rev;
    std::fs::write(dir.join("Roms/GB/Gamma (Japan).gb"), bytes(4, 16 * 1024)).unwrap();
    std::fs::remove_file(dir.join("Roms/GB/Huge (World).gb")).unwrap();
    s.changed(None);
    assert!(s.wait_ready(Duration::from_secs(60)));
    let c: Changes = phone.get(&format!("/v1/changes?since={rev}")).unwrap();
    assert_eq!(c.items.iter().map(|g| g.key.as_str()).collect::<Vec<_>>(), ["gb/Gamma (Japan)"]);
    assert_eq!(c.gone, ["gb/Huge (World)"]);
    assert!(c.rev > rev && !c.more);
    let none: Changes = phone.get(&format!("/v1/changes?since={}", c.rev)).unwrap();
    assert!(none.items.is_empty() && none.gone.is_empty());
    assert_eq!(status_of(&phone.get::<Changes>("/v1/changes?since=1").unwrap_err()), 410, "a marker from before this catalogue");
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

// The target of docs/PLAN.md ("Phase 9a design"): a download on this computer at 250 MB/s
// or more. Run with `cargo test --release -- --ignored timing` (CI does, on Windows).
#[test]
#[ignore]
fn timing_a_download_on_this_computer() {
    let dir = temp("timing-download");
    let lib_dir = dir.join("lib");
    let lib = Library::open(&lib_dir).unwrap();
    let games = dir.join("Roms");
    std::fs::create_dir_all(&games).unwrap();
    const SIZE: usize = 512 << 20;
    std::fs::write(games.join("big.bin"), bytes(5, 1 << 20).repeat(SIZE >> 20)).unwrap();
    let root = roots::add(&lib, &games, None).unwrap();
    drop(lib);
    let s = start(&lib_dir, &dir.join("server"));
    let phone = paired(&s);
    let url = format!("/v1/files/roots/{}/big.bin", root.id);
    let rate = |took: Duration| SIZE as f64 / took.as_secs_f64() / 1e6;
    // the server's part: sent over loopback HTTPS, the bytes read and let go, best of two
    let mut took = Duration::MAX;
    for _ in 0..2 {
        let t = std::time::Instant::now();
        let (head, body) = phone.open_request("GET", &url).unwrap();
        assert_eq!((head.status, head.length), (200, SIZE as u64));
        assert_eq!(std::io::copy(&mut body.take(head.length), &mut std::io::sink()).unwrap(), SIZE as u64);
        took = took.min(t.elapsed());
    }
    let served = rate(took);
    // and the device's as well: written to this computer's disk and synced, which is the
    // disk's speed more than the server's (84 MB/s on Windows CI), so only said
    let part = dir.join("device/big.bin.part");
    let t = std::time::Instant::now();
    assert_eq!(phone.fetch(&url, &part, None).unwrap(), SIZE as u64);
    eprintln!("512 MiB sent over HTTPS in {took:?}: {served:.0} MB/s; fetched and written to disk: {:.0} MB/s", rate(t.elapsed()));
    assert!(served >= 250.0, "{served:.0} MB/s (at least 250)");
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
