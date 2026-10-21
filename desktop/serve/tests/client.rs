//! Game Repo Go's backend with Game Repo Server (docs/PLAN.md, "Phase 9b design", the
//! tests): the catalogue kept on the device, read whole and then by its changes, and again
//! after a 410; the queue, with games written whole, paused, kept across a restart and
//! waiting while the library's away; *On this device* with games from the library and from
//! elsewhere; removals finished and put back after being cut short at each step; media on
//! their own; and nothing outside ES-DE's folders touched.

mod common;

use common::*;
use gamerepo_device::app::{App, FileSecrets};
use gamerepo_device::removing;
use gamerepo_serve::Server;
use gamerepo_wire::api::Page;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn open_app(dir: &Path, home: &Path) -> Arc<App> {
    let data = dir.join("go");
    App::open(&data, vec![home.to_path_buf()], "test", Box::new(FileSecrets(data.join("secrets.json"))), Box::new(|_| {})).unwrap()
}

fn call(app: &App, cmd: &str, args: Value) -> Value {
    app.call(cmd, &args).unwrap_or_else(|e| panic!("{cmd}: {e:#}"))
}

/// Waits for `ok`, failing after `secs` with what's there.
fn until(what: &str, secs: u64, ok: impl Fn() -> bool, show: impl Fn() -> String) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(Instant::now() < end, "{what}: {}", show());
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn synced(app: &App) {
    until("the catalogue read", 120, || !call(app, "status", json!({}))["sync"]["running"].as_bool().unwrap(), || call(app, "status", json!({}))["sync"].to_string());
    let st = call(app, "status", json!({}));
    assert_eq!(st["sync"]["error"], "", "{st}");
}

fn queue_done(app: &App) {
    until("the queue done", 120, || !app.queue.busy(), || call(app, "queue", json!({})).to_string());
}

fn item<'a>(q: &'a Value, key: &str) -> &'a Value {
    q["items"].as_array().unwrap().iter().find(|i| i["key"] == key).unwrap_or_else(|| panic!("{key} in {q}"))
}

fn keys(page: &Value) -> Vec<String> {
    page["items"].as_array().unwrap().iter().map(|g| g["key"].as_str().unwrap().to_string()).collect()
}

fn pair(app: &App, s: &Server) {
    let (code, _) = s.pair();
    let v = call(app, "pair", json!({ "address": address(s), "code": code }));
    assert_eq!(v["library"], "Test Library");
    synced(app);
}

const ALPHA: &str = "gb/Alpha (Europe)";
const QUEST: &str = "psx/Quest (USA) (Disc 1)";

#[test]
fn the_library_is_kept_on_the_device_read_whole_then_by_its_changes() {
    let dir = temp("go-catalogue");
    let lib = library(&dir);
    let s = start(&lib, &dir.join("server"));
    let d = a_device(&dir);
    for f in ["gb", "psx"] {
        std::fs::create_dir_all(d.home.join("ROMs").join(f)).unwrap();
    }
    let app = open_app(&dir, &d.home);
    let st = call(&app, "status", json!({}));
    assert_eq!((st["paired"].as_bool(), st["device"]["roms"].as_str()), (Some(false), d.home.join("ROMs").to_str()), "{st}");

    pair(&app, &s);
    let server = app.server().unwrap();
    let all: Page = server.get("/v1/games?limit=1").unwrap();
    let st = call(&app, "status", json!({}));
    assert_eq!(st["catalogue"]["games"].as_u64(), Some(all.total), "every game read: {st}");
    assert_eq!(st["sync"]["synced"]["whole"], true);

    // the systems ES-DE has a folder for, unless every one's asked for (C6)
    let ids = |v: &Value| v.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&call(&app, "systems", json!({}))), ["gb", "psx"]);
    call(&app, "set_settings", json!({ "show_every_system": true }));
    assert_eq!(ids(&call(&app, "systems", json!({}))).len(), 3, "MAME too");
    call(&app, "set_settings", json!({ "show_every_system": false }));

    // pages and search from the device's own copy
    let page = call(&app, "games", json!({}));
    assert!(keys(&page).iter().all(|k| k.starts_with("gb/") || k.starts_with("psx/")), "{page}");
    assert_eq!(keys(&call(&app, "games", json!({ "q": "alph" }))), [ALPHA]);
    assert_eq!(keys(&call(&app, "games", json!({ "system": "gb", "sort": "size", "desc": true, "limit": 1 }))), ["gb/Huge (World)"]);
    let game = call(&app, "game", json!({ "key": ALPHA }));
    assert_eq!(game["details"]["details"]["developer"], "Dev Co", "{game}");
    let pic = call(&app, "picture", json!({ "key": ALPHA, "kind": "cover", "size": 128 }));
    assert!(pic.as_str().unwrap().starts_with("data:image/jpeg;base64,"));
    assert_eq!(call(&app, "status", json!({}))["pictures"]["kept"], 1);

    // the library changes: only what changed is read
    std::fs::write(dir.join("Roms/GB/Gamma (Japan).gb"), bytes(4, 16 * 1024)).unwrap();
    std::fs::remove_file(dir.join("Roms/GB/Huge (World).gb")).unwrap();
    s.changed(None);
    assert!(s.wait_ready(Duration::from_secs(60)));
    call(&app, "sync", json!({}));
    synced(&app);
    let st = call(&app, "status", json!({}));
    assert_eq!((st["sync"]["synced"]["whole"].as_bool(), st["sync"]["synced"]["changed"].as_u64()), (Some(false), Some(2)), "{st}");
    assert_eq!(keys(&call(&app, "games", json!({ "q": "gamma" }))), ["gb/Gamma (Japan)"]);
    assert!(keys(&call(&app, "games", json!({ "q": "huge" }))).is_empty());

    // a marker the library no longer follows (410): read whole again
    app.catalogue.set_meta("rev", "1").unwrap();
    call(&app, "sync", json!({}));
    synced(&app);
    let st = call(&app, "status", json!({}));
    assert_eq!(st["sync"]["synced"]["whole"], true, "{st}");
    assert_eq!(st["catalogue"]["games"].as_u64(), Some(all.total));

    // away from home: the catalogue, the details and the picture kept are still there
    s.stop();
    assert_eq!(keys(&call(&app, "games", json!({ "q": "alph" }))), [ALPHA]);
    assert_eq!(call(&app, "game", json!({ "key": ALPHA }))["details"]["details"]["developer"], "Dev Co");
    assert!(call(&app, "picture", json!({ "key": ALPHA, "kind": "cover", "size": 128 })).is_string());
    assert!(call(&app, "picture", json!({ "key": ALPHA, "kind": "cover", "size": 64 })).is_null(), "not kept, and the library's away");

    call(&app, "unpair", json!({}));
    let st = call(&app, "status", json!({}));
    assert_eq!((st["paired"].as_bool(), st["catalogue"]["games"].as_u64()), (Some(false), Some(0)));
    app.queue.quit();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_queue_writes_games_whole_goes_on_after_a_restart_and_waits_for_the_library() {
    let dir = temp("go-queue");
    let lib = library(&dir);
    let s = start(&lib, &dir.join("server"));
    let d = a_device(&dir);
    let home = d.home.clone();
    let app = open_app(&dir, &home);
    pair(&app, &s);
    call(&app, "set_settings", json!({ "media": ["cover"], "manuals": true }));

    // before adding: the games' size, the media kinds with theirs, the free space (C9, C22)
    let p = call(&app, "prepare", json!({ "keys": [ALPHA, QUEST] }));
    assert!(p["size"].as_u64().unwrap() > 32 * 1024 && p["free"].as_u64().unwrap() > 0, "{p}");
    let kinds: Vec<(&str, bool)> = p["kinds"].as_array().unwrap().iter().map(|k| (k["kind"].as_str().unwrap(), k["ticked"].as_bool().unwrap())).collect();
    assert_eq!(kinds, [("cover", true), ("manual", true)], "{p}");

    // a game on two discs paused part way: nothing of it where ES-DE looks
    app.queue.pause_at(Some(4000));
    call(&app, "add", json!({ "keys": [QUEST] }));
    until("Quest paused", 60, || item(&call(&app, "queue", json!({})), QUEST)["state"] == "paused", || call(&app, "queue", json!({})).to_string());
    let q = call(&app, "queue", json!({}));
    let done = item(&q, QUEST)["done"].as_u64().unwrap();
    assert!(done >= 4000 && done < item(&q, QUEST)["size"].as_u64().unwrap(), "{q}");
    assert!(!home.join("ROMs/psx").exists(), "no half a game for ES-DE to list");
    assert!(home.join("ROMs/.gamerepo-coming/psx").is_dir());
    assert!(!home.join("ES-DE/gamelists/psx/gamelist.xml").exists() && d.record().games.is_empty());

    // the client closed and opened again: the queue's as it was, and goes on
    app.queue.quit();
    drop(app);
    let app = open_app(&dir, &home);
    let q = call(&app, "queue", json!({}));
    assert_eq!((item(&q, QUEST)["state"].as_str(), item(&q, QUEST)["done"].as_u64()), (Some("paused"), Some(done)), "{q}");
    app.queue.pause_at(None);
    call(&app, "resume", json!({}));
    queue_done(&app);
    let q = call(&app, "queue", json!({}));
    assert_eq!(item(&q, QUEST)["state"], "done", "{q}");
    assert!(home.join("ROMs/psx/Quest (USA).m3u/Quest (USA) (Disc 2).bin").is_file());
    assert!(!home.join("ROMs/.gamerepo-coming").exists(), "nothing left coming");
    assert!(std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap().contains("./Quest (USA).m3u"));
    assert_eq!(q["added"], 1, "Restart ES-DE to see the new games");
    call(&app, "clear_done", json!({}));
    assert_eq!(call(&app, "queue", json!({}))["added"], 0);

    // taken off the queue part way: what came of it goes
    app.queue.pause_at(Some(1 << 20));
    call(&app, "add", json!({ "keys": ["gb/Big (World)"] }));
    until("Big paused", 60, || item(&call(&app, "queue", json!({})), "gb/Big (World)")["state"] == "paused", || call(&app, "queue", json!({})).to_string());
    assert!(home.join("ROMs/.gamerepo-coming/gb/Big (World).gb").is_file());
    let id = item(&call(&app, "queue", json!({})), "gb/Big (World)")["id"].clone();
    call(&app, "cancel", json!({ "ids": [id] }));
    assert!(call(&app, "queue", json!({}))["items"].as_array().unwrap().is_empty());
    assert!(!home.join("ROMs/.gamerepo-coming").exists());
    app.queue.pause_at(None);

    // the library away (its address answers nothing): the queue waits for it, and goes on
    // by itself once it's back
    app.queue.set_retry(Duration::from_secs(2));
    let secrets = dir.join("go/secrets.json");
    let kept = std::fs::read_to_string(&secrets).unwrap();
    let mut away: Value = serde_json::from_str(&kept).unwrap();
    let mut server: Value = serde_json::from_str(away["server"].as_str().unwrap()).unwrap();
    server["address"] = json!("127.0.0.1:9");
    away["server"] = json!(server.to_string());
    std::fs::write(&secrets, away.to_string()).unwrap();
    call(&app, "add", json!({ "keys": [ALPHA] }));
    until("waiting for the library", 60, || item(&call(&app, "queue", json!({})), ALPHA)["state"] == "offline", || call(&app, "queue", json!({})).to_string());
    let q = call(&app, "queue", json!({}));
    assert_eq!((q["offline"].as_bool(), item(&q, ALPHA)["error"].as_str()), (Some(true), Some("Waiting for the library")));
    std::fs::write(&secrets, &kept).unwrap();
    queue_done(&app);
    assert_eq!(item(&call(&app, "queue", json!({})), ALPHA)["state"], "done");
    assert!(home.join("ROMs/gb/Alpha (Europe).gb").is_file() && home.join("ES-DE/downloaded_media/gb/covers/Alpha (Europe).png").is_file() && home.join("ES-DE/downloaded_media/gb/manuals/Alpha (Europe).pdf").is_file());

    // a file gone from the library: the plan's asked for again, then it's said what's wrong;
    // once it's back, the game comes
    let big = dir.join("Roms/GB/Big (World).gb");
    std::fs::rename(&big, dir.join("Big.away")).unwrap();
    call(&app, "add", json!({ "keys": ["gb/Big (World)"] }));
    queue_done(&app);
    let q = call(&app, "queue", json!({}));
    assert!(item(&q, "gb/Big (World)")["state"] == "failed" && !item(&q, "gb/Big (World)")["error"].as_str().unwrap().is_empty(), "{q}");
    std::fs::rename(dir.join("Big.away"), &big).unwrap();
    call(&app, "resume", json!({}));
    queue_done(&app);
    assert_eq!(item(&call(&app, "queue", json!({})), "gb/Big (World)")["state"], "done");
    assert_eq!(std::fs::read(home.join("ROMs/gb/Big (World).gb")).unwrap(), bytes(2, BIG));
    assert_eq!(d.record().games.len(), 3);

    app.queue.quit();
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

fn put(home: &Path, rel: &str, b: &[u8]) -> PathBuf {
    let p = home.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, b).unwrap();
    p
}

fn row<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["games"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap_or_else(|| panic!("{id} in {v}"))
}

#[test]
fn on_this_device_every_game_is_listed_and_removed_safely() {
    let dir = temp("go-device");
    let lib = library(&dir);
    let s = start(&lib, &dir.join("server"));
    let d = a_device(&dir);
    let home = d.home.clone();
    // games from elsewhere: Alpha under another name (the library knows it), one it
    // doesn't with its cover, and a game on two discs of cue sheets with their tracks
    put(&home, "ROMs/gb/My Alpha.gb", &bytes(1, 32 * 1024));
    put(&home, "ROMs/gb/My Alpha.srm", b"a save, never touched");
    put(&home, "ROMs/gb/Mine.gb", &bytes(77, 1000));
    put(&home, "ES-DE/downloaded_media/gb/covers/Mine.png", &png(16, 16));
    put(&home, "ROMs/psx/Saga.m3u", b"Saga (Disc 1).cue\nSaga (Disc 2).cue\n");
    for n in [1, 2] {
        put(&home, &format!("ROMs/psx/Saga (Disc {n}).cue"), cue(&format!("Saga (Disc {n})")).as_bytes());
        put(&home, &format!("ROMs/psx/Saga (Disc {n}).bin"), &bytes(200 + n, 2352));
    }
    put(&home, "ES-DE/gamelists/psx/gamelist.xml", b"<?xml version=\"1.0\"?>\n<gameList>\n\t<game>\n\t\t<path>./Saga.m3u</path>\n\t\t<name>Saga</name>\n\t\t<lastplayed>20240101T120000</lastplayed>\n\t</game>\n</gameList>\n");
    put(&home, "Saves/gb/Mine.srm", b"another save");
    let outside = || (snapshot(&lib), snapshot(&dir.join("Roms")), snapshot(&home.join("Saves")), std::fs::read(home.join("ROMs/gb/My Alpha.srm")).unwrap());
    let before = outside();

    let app = open_app(&dir, &home);
    pair(&app, &s);
    call(&app, "set_settings", json!({ "media": [] }));
    call(&app, "add", json!({ "keys": ["mame/beta", "mame/kappa", QUEST] }));
    queue_done(&app);

    // every game ES-DE lists, from the library or from elsewhere, those not known yet
    // checked against the library in the background
    let v = call(&app, "on_device", json!({}));
    assert_eq!(row(&v, "gb/My Alpha.gb")["from"], "unchecked", "{v}");
    until("checked", 60, || !call(&app, "status", json!({}))["checking"]["running"].as_bool().unwrap(), || call(&app, "status", json!({}))["checking"].to_string());
    let v = call(&app, "on_device", json!({}));
    let from = |id: &str| (row(&v, id)["from"].as_str().unwrap().to_string(), row(&v, id)["key"].as_str().unwrap_or("").to_string());
    assert_eq!(from("gb/My Alpha.gb"), ("known".into(), ALPHA.into()));
    assert_eq!(from("gb/Mine.gb"), ("elsewhere".into(), String::new()));
    assert_eq!(from("psx/Saga.m3u"), ("elsewhere".into(), String::new()));
    assert_eq!(from("mame/kappa.zip"), ("library".into(), "mame/kappa".into()));
    assert_eq!(from("psx/Quest (USA).m3u"), ("library".into(), QUEST.into()));
    assert_eq!(row(&v, "psx/Saga.m3u")["size"].as_u64(), Some((2 * 2352 + cue("Saga (Disc 1)").len() + cue("Saga (Disc 2)").len() + 36) as u64));
    assert_eq!(row(&v, "gb/Mine.gb")["media"][0][0], "cover");
    assert_eq!(v["games"].as_array().unwrap().len(), 6, "the BIOS set's beta's, not a game of its own: {v}");
    // the Library hides what's on the device (C6), Alpha too, unless it's asked to show it
    assert!(keys(&call(&app, "games", json!({ "system": "gb" }))).iter().all(|k| k != ALPHA));
    call(&app, "set_settings", json!({ "show_on_device": true }));
    let page = call(&app, "games", json!({ "q": "alph" }));
    assert_eq!(page["items"][0]["on_device"], true, "{page}");
    // Make room (C20): the one played longest ago first, then the oldest never played
    let room = call(&app, "make_room", json!({ "need": 1 }));
    assert_eq!(room, json!(["psx/Saga.m3u"]));

    // what goes is shown first; a BIOS set another game needs stays
    let r = call(&app, "removal", json!({ "asks": [{ "id": "mame/kappa.zip" }] }));
    let paths = |v: &Value, k: &str| v[k].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!((paths(&r, "files"), paths(&r, "kept")), (vec!["mame/kappa.zip".to_string()], vec!["mame/bios1.zip".to_string()]), "{r}");
    call(&app, "remove", json!({ "asks": [{ "id": "mame/kappa.zip" }] }));
    assert!(!home.join("ROMs/mame/kappa.zip").exists() && home.join("ROMs/mame/bios1.zip").is_file());
    assert!(d.record().games.iter().all(|g| g.key != "mame/kappa"));

    // a game from elsewhere goes with its media and entry; the device's other entries stay
    call(&app, "remove", json!({ "asks": [{ "id": "gb/Mine.gb" }] }));
    assert!(!home.join("ROMs/gb/Mine.gb").exists() && !home.join("ES-DE/downloaded_media/gb/covers/Mine.png").exists());
    let gl = std::fs::read_to_string(home.join("ES-DE/gamelists/gb/gamelist.xml")).unwrap();
    assert!(!gl.contains("Mine.gb"), "{gl}");

    // cut short after its files were set aside: put back as it was
    let saga_files = ["ROMs/psx/Saga.m3u", "ROMs/psx/Saga (Disc 1).cue", "ROMs/psx/Saga (Disc 1).bin", "ROMs/psx/Saga (Disc 2).cue", "ROMs/psx/Saga (Disc 2).bin"];
    let saga_was: Vec<Vec<u8>> = saga_files.iter().map(|f| std::fs::read(home.join(f)).unwrap()).collect();
    let psx_gl = std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap();
    let saga = [removing::Ask { id: "psx/Saga.m3u".into(), ..Default::default() }];
    let found = || gamerepo_device::scan::scan(&d, &app.catalogue.systems().unwrap());
    removing::cut_short(&d, &found(), &saga, "aside").unwrap();
    assert!(!home.join("ROMs/psx/Saga.m3u").exists(), "set aside");
    let pending = call(&app, "pending_removal", json!({}));
    assert_eq!(pending["can_put_back"], true, "{pending}");
    assert!(app.call("remove", &json!({ "asks": [{ "id": "psx/Quest (USA).m3u" }] })).is_err(), "not while one's cut short");
    call(&app, "put_back", json!({}));
    let saga_now = || saga_files.iter().map(|f| std::fs::read(home.join(f)).unwrap_or_default()).collect::<Vec<_>>();
    assert_eq!(saga_now(), saga_was);
    assert!(call(&app, "pending_removal", json!({})).is_null());

    // cut short after the lists changed: its entry is put back too
    removing::cut_short(&d, &found(), &saga, "lists").unwrap();
    assert!(!std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap().contains("Saga"));
    call(&app, "put_back", json!({}));
    assert_eq!(saga_now(), saga_was);
    assert_eq!(std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap().matches("<lastplayed>20240101T120000</lastplayed>").count(), 1);
    assert_eq!(gamerepo_wire::gamelist::blocks(&std::fs::read_to_string(home.join("ES-DE/gamelists/psx/gamelist.xml")).unwrap()).len(), gamerepo_wire::gamelist::blocks(&psx_gl).len());

    // a game from the library cut short after the record changed: the record's put back
    let sorted = |mut r: gamerepo_wire::api::Record| {
        r.games.sort_by(|a, b| a.key.cmp(&b.key));
        r
    };
    let rec = sorted(d.record());
    removing::cut_short(&d, &found(), &[removing::Ask { key: "mame/beta".into(), ..Default::default() }], "lists").unwrap();
    assert!(d.record().games.iter().all(|g| g.key != "mame/beta"));
    call(&app, "put_back", json!({}));
    assert_eq!(sorted(d.record()), rec);
    assert!(home.join("ROMs/mame/beta.zip").is_file() && home.join("ROMs/mame/bios1.zip").is_file());

    // cut short while deleting: it can only be finished
    removing::cut_short(&d, &found(), &saga, "deleting").unwrap();
    assert_eq!(call(&app, "pending_removal", json!({}))["can_put_back"], false);
    assert!(app.call("put_back", &json!({})).is_err());
    call(&app, "finish_removal", json!({}));
    assert!(saga_files.iter().all(|f| !home.join(f).exists()));
    assert!(!home.join("ROMs/.gamerepo-removing").exists() && !home.join("ES-DE/gamerepo/removing.json").exists());

    // media on their own: added to a game from elsewhere the library knows, which then
    // shows as from the library; removed, keeping the game; and added again
    call(&app, "add_media", json!({ "id": "gb/My Alpha.gb", "kinds": ["cover"] }));
    queue_done(&app);
    let cover = home.join("ES-DE/downloaded_media/gb/covers/My Alpha.png");
    assert!(cover.is_file(), "{:?}", files_under(&home));
    let gl = std::fs::read_to_string(home.join("ES-DE/gamelists/gb/gamelist.xml")).unwrap();
    assert!(gl.contains("<path>./My Alpha.gb</path>") && gl.contains("Dev Co"), "{gl}");
    let v = call(&app, "on_device", json!({}));
    assert_eq!((row(&v, "gb/My Alpha.gb")["from"].as_str(), row(&v, "gb/My Alpha.gb")["key"].as_str()), (Some("library"), Some(ALPHA)));
    assert_eq!(std::fs::read(home.join("ROMs/gb/My Alpha.gb")).unwrap(), bytes(1, 32 * 1024), "the game itself isn't fetched again");
    call(&app, "remove_media", json!({ "asks": [{ "id": "gb/My Alpha.gb", "kinds": ["cover"] }] }));
    assert!(!cover.exists() && home.join("ROMs/gb/My Alpha.gb").is_file());
    let held = d.record().games.into_iter().find(|g| g.key == ALPHA).unwrap();
    assert!(held.files.iter().all(|f| f.role != "cover") && held.files.iter().any(|f| f.path == "gb/My Alpha.gb"), "{held:?}");
    call(&app, "add_media", json!({ "key": ALPHA, "kinds": ["cover"] }));
    queue_done(&app);
    assert!(cover.is_file());

    // removed whole: its save beside it stays, as everything outside ES-DE's folders does
    call(&app, "remove", json!({ "asks": [{ "key": ALPHA }] }));
    assert!(!home.join("ROMs/gb/My Alpha.gb").exists() && !cover.exists());
    #[cfg(unix)]
    {
        let away = dir.join("away");
        std::fs::create_dir_all(&away).unwrap();
        std::fs::write(away.join("Far.gb"), b"far").unwrap();
        std::os::unix::fs::symlink(&away, home.join("ROMs/gb/Linked")).unwrap();
        assert!(removing::inside(&d, "roms", "gb/Linked/Far.gb").is_err(), "a link leading out of ES-DE's folders");
        assert!(removing::inside(&d, "roms", "gb/Fine.gb").is_ok());
        std::fs::remove_file(home.join("ROMs/gb/Linked")).unwrap();
    }
    app.queue.quit();
    s.stop();
    assert!(before == outside(), "the library, the saves and every file outside ES-DE's folders as they were");
    let _ = std::fs::remove_dir_all(&dir);
}

/// As on Jerred's Steam Deck (2026-10-20): ES-DE's gamelists folder a link to elsewhere, as
/// RetroDECK makes it, and the ROMs folder changed in Settings after games were added. The
/// games left in the old folder aren't on the device, so they can be added again or taken
/// off the record, and removing follows ES-DE's own linked folders.
#[test]
fn games_left_in_an_old_roms_folder_can_be_added_again_and_removed_through_linked_folders() {
    let dir = temp("go-moved");
    let lib = library(&dir);
    let s = start(&lib, &dir.join("server"));
    let d = a_device(&dir);
    let home = d.home.clone();
    let gamelists = if cfg!(unix) { dir.join("retrodeck/gamelists") } else { home.join("ES-DE/gamelists") };
    #[cfg(unix)]
    {
        std::fs::create_dir_all(dir.join("retrodeck")).unwrap();
        std::fs::rename(home.join("ES-DE/gamelists"), &gamelists).unwrap();
        std::os::unix::fs::symlink(&gamelists, home.join("ES-DE/gamelists")).unwrap();
    }
    let app = open_app(&dir, &home);
    pair(&app, &s);
    call(&app, "set_settings", json!({ "media": ["cover"] }));
    call(&app, "add", json!({ "keys": [ALPHA, QUEST] }));
    queue_done(&app);
    let old_alpha = home.join("ROMs/gb/Alpha (Europe).gb");
    assert!(old_alpha.is_file(), "{:?}", files_under(&home));
    assert!(std::fs::read_to_string(gamelists.join("gb/gamelist.xml")).unwrap().contains("Alpha (Europe).gb"));

    let roms = dir.join("new-roms");
    std::fs::create_dir_all(&roms).unwrap();
    call(&app, "set_settings", json!({ "roms": roms.to_string_lossy() }));
    let v = call(&app, "on_device", json!({}));
    let of = |v: &Value, key: &str| v["games"].as_array().unwrap().iter().find(|r| r["key"] == key).cloned().unwrap_or_else(|| panic!("{key} in {v}"));
    assert_eq!((of(&v, ALPHA)["missing"].as_bool(), of(&v, ALPHA)["size"].as_u64()), (Some(true), Some(0)), "{v}");
    assert!(call(&app, "make_room", json!({ "need": 1 })).as_array().unwrap().is_empty(), "nothing to free");
    let page = call(&app, "games", json!({ "q": "alph", "system": "gb" }));
    assert_eq!((keys(&page), page["items"][0]["on_device"].as_bool()), (vec![ALPHA.to_string()], Some(false)), "{page}");

    // taken off the record with its cover and gamelist entry; the old folder's copy stays
    let r = call(&app, "removal", json!({ "asks": [{ "key": ALPHA }] }));
    let paths = |k: &str| r[k].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!((paths("files"), paths("gone")), (vec!["gb/covers/Alpha (Europe).png".to_string()], vec!["gb/Alpha (Europe).gb".to_string()]), "{r}");
    assert_eq!(r["frees"].as_u64(), std::fs::metadata(home.join("ES-DE/downloaded_media/gb/covers/Alpha (Europe).png")).ok().map(|m| m.len()));
    call(&app, "remove", json!({ "asks": [{ "key": ALPHA }] }));
    assert!(d.record().games.iter().all(|g| g.key != ALPHA));
    assert!(!std::fs::read_to_string(gamelists.join("gb/gamelist.xml")).unwrap().contains("Alpha (Europe).gb"));
    assert!(old_alpha.is_file() && !home.join("ES-DE/downloaded_media/gb/covers/Alpha (Europe).png").exists());

    // added again, into the new folder, and removed from it
    call(&app, "add", json!({ "keys": [QUEST] }));
    queue_done(&app);
    assert!(roms.join("psx/Quest (USA).m3u").is_dir(), "{:?}", files_under(&roms));
    assert_eq!(of(&call(&app, "on_device", json!({})), QUEST)["missing"], Value::Null);

    // a disc deleted by hand: marked, offered again, and added again only it comes back
    let m3u = roms.join("psx/Quest (USA).m3u");
    let mut discs: Vec<PathBuf> = std::fs::read_dir(&m3u).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x != "m3u")).collect();
    discs.sort();
    assert!(discs.len() >= 2, "{:?}", files_under(&m3u));
    let (kept, gone) = (discs[0].clone(), discs.last().unwrap().clone());
    let kept_at = std::fs::metadata(&kept).unwrap().modified().unwrap();
    std::fs::remove_file(&gone).unwrap();
    let r = of(&call(&app, "on_device", json!({})), QUEST);
    assert_eq!((r["partial"].as_bool(), r["missing"].as_bool()), (Some(true), None), "{r}");
    let page = call(&app, "games", json!({ "q": "quest", "system": "psx" }));
    assert_eq!(page["items"][0]["on_device"].as_bool(), Some(false), "{page}");
    call(&app, "add", json!({ "keys": [QUEST] }));
    queue_done(&app);
    assert!(gone.is_file() && std::fs::metadata(&kept).unwrap().modified().unwrap() == kept_at, "{:?}", files_under(&m3u));
    let r = of(&call(&app, "on_device", json!({})), QUEST);
    assert_eq!(r["partial"], Value::Null, "{r}");
    assert_eq!(d.record().games.iter().filter(|g| g.key == QUEST).count(), 1);
    call(&app, "remove", json!({ "asks": [{ "key": QUEST }] }));
    assert!(!roms.join("psx/Quest (USA).m3u").exists() && home.join("ROMs/psx/Quest (USA).m3u").is_dir());
    assert!(!std::fs::read_to_string(gamelists.join("psx/gamelist.xml")).unwrap().contains("Quest"));
    assert!(d.record().games.is_empty(), "{:?}", d.record());
    app.queue.quit();
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

/// D114 and D116: games put in the queue not started wait for Start downloads, which says
/// what they take first; Download… on one of them starts it alone. ES-DE as RetroDECK and
/// EmuDeck set it up is found, and when it's set up more than once Game Repo Go asks which.
#[test]
fn queued_games_wait_to_be_started_and_retrodeck_and_emudeck_are_found() {
    let dir = temp("go-queued");
    let lib = library(&dir);
    let s = start(&lib, &dir.join("server"));
    let d = a_device(&dir);
    let home = d.home.clone();
    put(&home, ".config/EmuDeck/settings.sh", b"emulationPath=/home/deck/Emulation\n");
    let app = open_app(&dir, &home);
    pair(&app, &s);
    call(&app, "set_settings", json!({ "media": ["cover"] }));
    call(&app, "add", json!({ "keys": [ALPHA, QUEST], "start": false }));
    std::thread::sleep(Duration::from_millis(300));
    let q = call(&app, "queue", json!({}));
    assert_eq!((item(&q, ALPHA)["state"].as_str(), item(&q, QUEST)["state"].as_str(), q["queued"].as_u64(), q["left"].as_u64()), (Some("queued"), Some("queued"), Some(2), Some(0)), "{q}");
    assert!(!app.queue.busy() && !home.join("ROMs/gb/Alpha (Europe).gb").exists());
    assert_eq!(call(&app, "status", json!({}))["queue"]["queued"], 2);

    // what starting them takes: each one's size on disc, the whole, the free space
    let sum = call(&app, "start_summary", json!({}));
    let size_of = |key: &str| sum["games"].as_array().unwrap().iter().find(|g| g["key"] == key).and_then(|g| g["size"].as_u64()).unwrap_or_else(|| panic!("{key} in {sum}"));
    assert!(size_of(ALPHA) > 32 * 1024 && size_of(QUEST) > 2 * 2352 * 4, "{sum}");
    assert!(sum["size"].as_u64().unwrap() >= size_of(ALPHA) + size_of(QUEST), "{sum}");
    assert_eq!((sum["places"].as_array().unwrap().len(), sum["short"].as_u64()), (1, Some(0)), "{sum}");
    assert!(sum["places"][0]["free"].as_u64().unwrap() > 0);

    // Download… on a queued game starts it alone; Start downloads the rest
    call(&app, "add", json!({ "keys": [ALPHA] }));
    queue_done(&app);
    let q = call(&app, "queue", json!({}));
    assert_eq!((item(&q, ALPHA)["state"].as_str(), item(&q, QUEST)["state"].as_str()), (Some("done"), Some("queued")), "{q}");
    assert!(home.join("ROMs/gb/Alpha (Europe).gb").is_file() && !home.join("ROMs/psx/Quest (USA).m3u").exists());
    assert_eq!(call(&app, "start_queue", json!({}))["started"], 1);
    queue_done(&app);
    assert!(home.join("ROMs/psx/Quest (USA).m3u").is_dir(), "{:?}", files_under(&home.join("ROMs")));
    assert_eq!(call(&app, "start_summary", json!({}))["games"], json!([]));

    // ES-DE as EmuDeck set it up, found as before, and named
    let st = call(&app, "status", json!({}));
    assert_eq!(st["device"]["by"], if cfg!(windows) { "ES-DE" } else { "EmuDeck" }, "{st}");
    // RetroDECK too: set up twice, so Game Repo Go asks which rather than guessing
    let rd = home.join(".var/app/net.retrodeck.retrodeck/config/ES-DE");
    let rd_roms = dir.join("retrodeck/roms");
    std::fs::create_dir_all(&rd_roms).unwrap();
    put(&rd, "settings/es_settings.xml", format!("<?xml version=\"1.0\"?>\n<string name=\"ROMDirectory\" value=\"{}\" />\n<string name=\"MediaDirectory\" value=\"{}\" />\n", rd_roms.display(), dir.join("retrodeck/ES-DE/downloaded_media").display()).as_bytes());
    let st = call(&app, "status", json!({}));
    assert!(st["device"]["error"].as_str().unwrap_or("").contains("set up more than once"), "{st}");
    let installs = st["device"]["installs"].as_array().unwrap();
    assert_eq!(installs.iter().map(|i| i["by"].as_str().unwrap()).collect::<Vec<_>>(), [if cfg!(windows) { "ES-DE" } else { "EmuDeck" }, "RetroDECK"], "{st}");
    assert_eq!(installs[1]["roms"], json!(rd_roms));
    call(&app, "set_settings", json!({ "esde": rd.to_string_lossy() }));
    let st = call(&app, "status", json!({}));
    assert_eq!((st["device"]["by"].as_str(), st["device"]["roms"].clone()), (Some("RetroDECK"), json!(rd_roms)), "{st}");
    // a ROMs folder chosen by hand that isn't ES-DE's own is warned of
    let other = dir.join("other-roms");
    std::fs::create_dir_all(&other).unwrap();
    call(&app, "set_settings", json!({ "roms": other.to_string_lossy() }));
    assert_eq!(call(&app, "status", json!({}))["device"]["esde_roms"], json!(rd_roms));
    app.queue.quit();
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}

const ZETA: &str = "pcgames/zeta";
const OMEGA: &str = "pcgames/omega";

#[test]
fn pc_games_are_unpacked_by_the_library_written_whole_given_a_shortcut_and_removed_keeping_saves() {
    let dir = temp("go-pc");
    let lib = library(&dir);
    pc_games(&lib);
    let s = start(&lib, &dir.join("server"));
    let d = a_device(&dir);
    // Windows' Start menu for this test only
    #[cfg(windows)]
    std::env::set_var("APPDATA", dir.join("appdata"));
    #[cfg(windows)]
    let lnk = |name: &str| dir.join("appdata/Microsoft/Windows/Start Menu/Programs/Game Repo Go").join(format!("{name}.lnk"));
    let app = open_app(&dir, &d.home);
    let games = dir.join("Games");
    call(&app, "set_settings", json!({ "pc_games": games }));
    pair(&app, &s);
    let before = snapshot(&lib);

    // PC games are a system of their own for a device that can have them (D110); Lambda,
    // with only its installer, isn't listed
    let systems = call(&app, "systems", json!({}));
    let pc = systems.as_array().unwrap().iter().find(|x| x["id"] == "pcgames").unwrap_or_else(|| panic!("{systems}"));
    assert_eq!((pc["name"].as_str(), pc["games"].as_u64()), (Some("PC games"), Some(2)), "{systems}");
    assert_eq!(keys(&call(&app, "games", json!({ "system": "pcgames" }))), [OMEGA, ZETA]);
    let zeta_size = call(&app, "game", json!({ "key": ZETA }))["game"]["size"].as_u64().unwrap();
    assert_eq!(zeta_size, 7 + 64 * 1024, "Zeta's size is its unpacked files'");

    // before adding: Zeta's archive starts being unpacked on the library (D111), its size
    // counted as the list has it; a game with only an installer can't go, nor a patch yet
    let p = call(&app, "prepare", json!({ "keys": [ZETA, OMEGA, "pcgames/lambda"] }));
    assert_eq!(p["unpacking"], 1, "{p}");
    assert!(p["size"].as_u64().unwrap() >= zeta_size + 8 + 16 * 1024, "{p}");
    let left: Vec<&str> = p["left"].as_array().unwrap().iter().map(|l| l["what"].as_str().unwrap()).collect();
    assert_eq!(left, ["Omega › its patches (1)", "Lambda"], "{p}");
    assert!(p["left"][1]["why"].as_str().unwrap().contains("only its installer"), "{p}");

    // added: each game whole in its own `Title (Year)` folder of the PC games folder, the
    // one folder its files were all in (Omega's copy, Zeta's archive) left out (D124)
    call(&app, "add", json!({ "keys": [ZETA, OMEGA] }));
    queue_done(&app);
    let q = call(&app, "queue", json!({}));
    for k in [ZETA, OMEGA] {
        assert_eq!(item(&q, k)["state"], "done", "{q}");
    }
    assert_eq!(files_under(&games), ["Omega (2001)/omega.dat", "Omega (2001)/omega.exe", "Zeta (1999)/data/level1.dat", "Zeta (1999)/zeta.exe"]);
    assert_eq!(std::fs::read(games.join("Zeta (1999)/zeta.exe")).unwrap(), b"MZ zeta");
    // and a shortcut (D112): in Steam on Linux, through Proton, with its cover
    #[cfg(target_os = "linux")]
    {
        let wanted = gamerepo_device::steam::state(&dir.join("go")).wanted;
        let w = |k: &str| wanted.iter().find(|w| w.key == k).unwrap_or_else(|| panic!("{k} in {wanted:?}")).clone();
        let (omega, zeta) = (w(OMEGA), w(ZETA));
        assert_eq!((omega.name.as_str(), omega.args.as_str(), omega.proton), ("Omega", "-windowed", true));
        assert_eq!((omega.program, omega.start_dir), (games.join("Omega (2001)/omega.exe").display().to_string(), games.join("Omega (2001)").display().to_string()));
        assert!(omega.art.get("cover").is_some_and(|p| Path::new(p).is_file()), "{:?}", omega.art);
        assert_eq!(zeta.program, games.join("Zeta (1999)/zeta.exe").display().to_string());
    }
    // in the Start menu on Windows
    #[cfg(windows)]
    assert!(lnk("Omega").is_file() && lnk("Zeta").is_file());

    // On this device lists them by their folders
    let v = call(&app, "on_device", json!({}));
    let r = row(&v, "pc:Omega (2001)");
    assert_eq!((r["from"].as_str(), r["key"].as_str(), r["name"].as_str()), (Some("library"), Some(OMEGA), Some("Omega")), "{v}");

    // removed: what came from the library and its shortcut go; a save the game made stays,
    // with its folder, and is told of
    std::fs::create_dir_all(games.join("Omega (2001)/saves")).unwrap();
    std::fs::write(games.join("Omega (2001)/saves/slot1.sav"), b"a save").unwrap();
    let gone = call(&app, "remove", json!({ "asks": [{ "key": OMEGA }] }));
    assert_eq!(gone["kept_made"], json!([["Omega (2001)", 1]]), "{gone}");
    assert_eq!(files_under(&games), ["Omega (2001)/saves/slot1.sav", "Zeta (1999)/data/level1.dat", "Zeta (1999)/zeta.exe"]);
    #[cfg(target_os = "linux")]
    assert!(gamerepo_device::steam::state(&dir.join("go")).wanted.iter().all(|w| w.key != OMEGA));
    #[cfg(windows)]
    assert!(!lnk("Omega").exists() && lnk("Zeta").is_file());
    // Zeta removed too: its folder goes, as nothing's left in it
    let gone = call(&app, "remove", json!({ "asks": [{ "key": ZETA }] }));
    assert_eq!(gone["kept_made"], json!([]), "{gone}");
    assert!(!games.join("Zeta (1999)").exists());

    app.queue.quit();
    s.stop();
    assert!(before == snapshot(&lib), "the library as it was");
    let _ = std::fs::remove_dir_all(&dir);
}

// The targets of docs/PLAN.md ("Phase 9b design"): the whole catalogue of 100,000 games
// read over loopback in under 20 s; a page and a search of the device's copy under 50 ms.
// Run with `cargo test --release -- --ignored timing` (CI does, on Windows).
#[test]
#[ignore]
fn timing_game_repo_go_reading_100k_games() {
    use gamerepo_core::{index::Index, library::Library};
    let dir = temp("go-timing");
    let lib = library(&dir);
    let data = dir.join("server");
    let s = start(&lib, &data);
    // the server's index given 100,000 more games, as its catalogue reads them
    {
        let id = Library::open(&lib).unwrap().id().unwrap();
        let ix = Index::open(&gamerepo_core::index::path_for(&data, &id), &id).unwrap();
        let syl = ["ka", "zo", "ri", "mu", "te", "lo", "shi", "an", "bel", "dor"];
        let word = |n: usize| format!("{}{}{}", syl[n % 10], syl[n / 10 % 10], syl[n / 100 % 10]);
        let tx = ix.conn().unchecked_transaction().unwrap();
        {
            let mut put = tx
                .prepare(
                    "INSERT INTO library (id, key, kind, platform, name, title, sort, year, have, media, picture, size, year_sort, rating_sort, size_sort, date_sort)
                     VALUES (?1, ?2, 'dat', ?3, ?4, ?5, ?6, ?7, 1, 'cover', NULL, ?8, 0, 0, 0, 0)",
                )
                .unwrap();
            let mut text = tx.prepare("INSERT INTO library_text (rowid, title, name, body) VALUES (?1, ?2, ?3, '')").unwrap();
            for i in 0..100_000usize {
                let system = ["gb", "psx", "nes", "snes", "n64"][i % 5];
                let title = format!("{} {} {}", ["The", "Super", "Mega", "Hyper"][i % 4], word(i * 7919 % 1000), word(i / 3 % 1000));
                let name = format!("{title} (Game {i})");
                let id = 1_000_000 + i as i64;
                put.execute(rusqlite::params![id, format!("{system}/{name}"), system, name, title, title.to_lowercase(), (i % 7 != 0).then_some(1980 + (i % 40) as i64), (i * 1000) as i64]).unwrap();
                text.execute(rusqlite::params![id, title, name]).unwrap();
            }
        }
        tx.commit().unwrap();
        gamerepo_serve::catalogue::refresh(&ix).unwrap();
    }
    let d = a_device(&dir);
    for f in ["gb", "psx", "nes", "snes", "n64"] {
        std::fs::create_dir_all(d.home.join("ROMs").join(f)).unwrap();
    }
    let app = open_app(&dir, &d.home);
    let t = Instant::now();
    pair(&app, &s);
    let read = t.elapsed();
    let games = call(&app, "status", json!({}))["catalogue"]["games"].as_u64().unwrap();
    assert!(games > 100_000, "{games}");
    let best = |f: &dyn Fn() -> Value| (0..5).map(|_| {
        let t = Instant::now();
        f();
        t.elapsed()
    }).min().unwrap();
    let deep = best(&|| call(&app, "games", json!({ "offset": 90_000, "limit": 100 })));
    let by_size = best(&|| call(&app, "games", json!({ "system": "snes", "sort": "size", "desc": true, "offset": 15_000, "limit": 100 })));
    let search = best(&|| call(&app, "games", json!({ "q": "mega kari", "limit": 100 })));
    assert!(!keys(&call(&app, "games", json!({ "q": "mega kari", "limit": 100 }))).is_empty());
    eprintln!("Game Repo Go read {games} games in {read:?}; a page at 90,000 {deep:?}, a system's by size {by_size:?}, a search {search:?}");
    assert!(read < Duration::from_secs(20), "the catalogue read in {read:?}");
    for (what, took) in [("a page", deep), ("a system's page by size", by_size), ("a search", search)] {
        assert!(took < Duration::from_millis(50), "{what} took {took:?}");
    }
    app.queue.quit();
    s.stop();
    let _ = std::fs::remove_dir_all(&dir);
}
