//! A game's guidebooks for Atlas on devices (docs/PLAN.md, "Maps and Guides notes, part 10",
//! D236–D237): the guidebooks the library keeps in `Maps and Guides/<title>/Guidebooks/`
//! travel in a pack of their own for each game, made by Game Repo Server with Atlas's crate
//! (`gamerepo_core::guidebooks::Cache`, in the server's data folder under `guidebooks/`) when
//! a device asks for it, and kept, so it isn't made twice.
//!
//! A game's details (`GET /v1/games/<key>`) list it with its packs (sort `guidebooks`), each
//! guidebook with its size, for a tick of its own; a plan adds it as a pack file (role `pack`,
//! place `atlas`) made of the guidebooks the device left ticked (`PlanAsk::guidebooks_left`).
//! A pack not made yet is made in the background: the plan isn't `ready`, gives a size
//! reckoned from the guidebooks and no SHA-1, and its address answers 202 until it's made, as
//! a made file's does. A set of guidebooks whose pack isn't the newest made of the game's is
//! made again, newer, so a device holding the newer one takes it (Go never puts an older pack
//! over a newer one). `GET /v1/packs` lists every game's pack of guidebooks too, grouped under
//! the game, and `GET /v1/packs/<packId>/file` sends the whole set's.

use crate::catalogue;
use anyhow::Result;
use gamerepo_core::guidebooks::{self as gb, Cache, Folder, Made, Wanted};
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::organise::safe_name;
use gamerepo_wire::api::{Guidebook, LibraryPack, Pack, PackGame, PlanAsk};
use gamerepo_wire::{Left, Plan};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

/// Where the packs are made, in the server's data folder.
pub const DIR: &str = "guidebooks";

#[derive(Clone)]
enum Job {
    Waiting(Box<Wanted>),
    Making,
    Failed(String),
}

/// Makes the games' packs of guidebooks, one at a time, in a thread of its own.
pub struct Maker {
    cache: Cache,
    jobs: Mutex<HashMap<String, Job>>,
    queue: Mutex<VecDeque<String>>,
    cv: Condvar,
}

/// What's made of a pack.
pub enum Status {
    Ready(PathBuf, Made),
    /// Being made, or waiting its turn
    Pending,
    Failed(String),
    /// Not asked for: ask for the plan again
    Unknown,
}

/// The maker for a data folder (one for each server in the process).
pub fn maker(data: &Path) -> Arc<Maker> {
    static ALL: OnceLock<Mutex<HashMap<PathBuf, Arc<Maker>>>> = OnceLock::new();
    let mut all = ALL.get_or_init(Mutex::default).lock().unwrap_or_else(|e| e.into_inner());
    all.entry(data.to_path_buf())
        .or_insert_with(|| {
            let m = Arc::new(Maker { cache: Cache::new(data.join(DIR)), jobs: Mutex::default(), queue: Mutex::default(), cv: Condvar::new() });
            let w = m.clone();
            std::thread::Builder::new().name("guidebooks".into()).spawn(move || w.work()).expect("a thread");
            m
        })
        .clone()
}

impl Maker {
    /// The pack for `w`: ready (made, and the newest of its game's), else asked for.
    pub fn status(&self, w: &Wanted) -> Status {
        let id = w.recipe();
        let mut jobs = self.jobs.lock().unwrap();
        match jobs.get(&id) {
            Some(Job::Waiting(_) | Job::Making) => return Status::Pending,
            Some(Job::Failed(e)) => return Status::Failed(e.clone()),
            None => {}
        }
        if let Some(m) = self.cache.current(w) {
            return Status::Ready(self.cache.path(&w.pack_id, &m), m);
        }
        jobs.insert(id.clone(), Job::Waiting(Box::new(w.clone())));
        self.queue.lock().unwrap().push_back(id);
        self.cv.notify_all();
        Status::Pending
    }

    /// A pack made of a recipe, for its address: being made, made, or not asked for.
    pub fn file(&self, pack_id: &str, id: &str) -> Status {
        match self.jobs.lock().unwrap().get(id) {
            Some(Job::Waiting(_) | Job::Making) => return Status::Pending,
            Some(Job::Failed(e)) => return Status::Failed(e.clone()),
            None => {}
        }
        match self.cache.by_id(pack_id, id) {
            Some(m) => Status::Ready(self.cache.path(pack_id, &m), m),
            None => Status::Unknown,
        }
    }

    /// The newest pack made of a game's, whatever it holds (for its dates on a list).
    pub fn newest(&self, pack_id: &str) -> Option<Made> {
        self.cache.newest(pack_id)
    }

    /// Wait until `w`'s pack is made or has failed, at most `wait` (for tests).
    pub fn wait(&self, w: &Wanted, wait: Duration) -> Status {
        let until = std::time::Instant::now() + wait;
        loop {
            let s = self.status(w);
            if !matches!(s, Status::Pending) || std::time::Instant::now() >= until {
                return s;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn work(&self) {
        loop {
            let next = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if let Some(id) = q.pop_front() {
                        break id;
                    }
                    q = self.cv.wait_timeout(q, Duration::from_millis(500)).unwrap().0;
                }
            };
            let w = match self.jobs.lock().unwrap().get(&next).cloned() {
                Some(Job::Waiting(w)) => w,
                _ => continue,
            };
            self.jobs.lock().unwrap().insert(next.clone(), Job::Making);
            let r = self.cache.make(&w, &AtomicBool::new(false));
            let mut jobs = self.jobs.lock().unwrap();
            match r {
                Ok(_) => {
                    jobs.remove(&next);
                }
                Err(e) => {
                    jobs.insert(next, Job::Failed(format!("{e:#}")));
                }
            }
        }
    }
}

/// The name a game's pack of guidebooks has in Atlas's packs folder: `<Game title> -
/// Guidebooks.atlaspack`, as the contract names a pack by its game and source.
pub fn file_name(f: &Folder) -> String {
    safe_name(&format!("{} - Guidebooks.{}", f.game_title(), gamerepo_core::atlaspack::EXTENSION))
}

/// Where a pack of a recipe is fetched.
fn made_url(pack_id: &str, id: &str) -> String {
    format!("/v1/packs/{}/made/{id}", crate::http::encode_path(pack_id))
}

/// A game's folder of guidebooks, by its catalogue key (any of its discs' Library keys).
fn folder_of(ix: &Index, lib: &Library, key: &str) -> Result<Option<Folder>> {
    let Some(keys) = catalogue::keys_of(ix, key)? else { return Ok(None) };
    gb::folder_of(ix, lib, &keys)
}

/// What a device's page shows of a game's pack of guidebooks: as a pack (sort `guidebooks`),
/// its size the newest made one's when that holds all of them, else reckoned, and each
/// guidebook with its size, pages and whether its text is read.
pub fn info(maker: &Maker, f: &Folder) -> Option<Pack> {
    let pack_id = f.pack_id()?;
    let whole = gb::wanted(f, &[]);
    let made = whole.as_ref().and_then(|w| maker.cache.current(w));
    let guidebooks: Vec<Guidebook> = f
        .guidebooks
        .iter()
        .map(|g| {
            let pages = gb::pages(g);
            Guidebook {
                id: g.file.clone(),
                title: g.title.clone(),
                kind: g.kind.clone(),
                pages: pages.as_ref().map(|n| *n as u64).unwrap_or(0),
                size: g.size,
                text: gamerepo_core::magtext::kept(&g.path).is_some(),
                cannot_go: whole.as_ref().and_then(|w| w.left.iter().find(|(t, _)| t == &g.title)).map(|(_, why)| why.clone()).unwrap_or_default(),
            }
        })
        .collect();
    Some(Pack {
        pack_id,
        // as Atlas shows it beside the game's other packs
        title: format!("{} - Guidebooks", f.game_title()),
        sort: "guidebooks".into(),
        file: file_name(f),
        size: made.as_ref().map(|m| m.size).or(whole.as_ref().map(Wanted::estimate)).unwrap_or(0),
        guide_pages: guidebooks.iter().filter(|g| g.cannot_go.is_empty()).map(|g| g.pages).sum(),
        updated: made.as_ref().map(|m| m.updated.clone()).unwrap_or_default(),
        guidebooks,
        ..Default::default()
    })
}

/// A catalogue game's pack of guidebooks, for its page on a device.
pub fn of_game(ix: &Index, lib: &Library, data: &Path, key: &str) -> Result<Option<Pack>> {
    Ok(folder_of(ix, lib, key)?.and_then(|f| info(&maker(data), &f)))
}

/// A plan's file for a pack of guidebooks: made (its size, SHA-1 and dates), or being made
/// (its size reckoned, no SHA-1, the plan not ready).
fn file_of(maker: &Maker, w: &Wanted, name: String, plan: &mut Plan) -> gamerepo_wire::File {
    let mut f = gamerepo_wire::File { place: crate::packs::PLACE.into(), path: name, role: crate::packs::ROLE.into(), pack: w.pack_id.clone(), url: made_url(&w.pack_id, &w.recipe()), ..Default::default() };
    match maker.status(w) {
        Status::Ready(_, m) => {
            f.size = m.size;
            f.sha1 = Some(m.sha1);
            f.pack_updated = m.updated;
            f.pack_exported_at = m.exported_at;
        }
        Status::Failed(e) => {
            plan.left.push(Left { what: f.path.clone(), why: format!("Its pack couldn't be made: {e}") });
            f.url.clear();
        }
        _ => {
            f.size = w.estimate();
            plan.ready = false;
        }
    }
    for (title, why) in &w.left {
        plan.left.push(Left { what: title.clone(), why: why.clone() });
    }
    f
}

/// The games' packs of guidebooks added to a plan (D237): each made of the guidebooks the
/// device left ticked, unless it unticked the pack itself (`packs_left`); once however many
/// of the plan's games it's for (`needed_by` the others).
pub fn add(ix: &Index, lib: &Library, data: &Path, ask: &PlanAsk, keys: &[String], plan: &mut Plan) -> Result<()> {
    if gb::list(lib).is_empty() {
        return Ok(());
    }
    let maker = maker(data);
    for key in keys {
        let Some(f) = folder_of(ix, lib, key)? else { continue };
        let Some(pack_id) = f.pack_id() else { continue };
        if ask.packs_left.iter().any(|(k, v)| k == key && v.contains(&pack_id)) {
            continue;
        }
        if let Some(x) = plan.files.iter_mut().find(|x| x.role == crate::packs::ROLE && x.pack == pack_id) {
            if &x.key != key && !x.needed_by.contains(key) {
                x.needed_by.push(key.clone());
            }
            continue;
        }
        let left: &[String] = ask.guidebooks_left.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice()).unwrap_or(&[]);
        let Some(w) = gb::wanted(&f, left) else { continue };
        let mut file = file_of(&maker, &w, file_name(&f), plan);
        if file.url.is_empty() {
            continue;
        }
        let lib_keys = catalogue::keys_of(ix, key)?.unwrap_or_default();
        let first = lib_keys.first().cloned().unwrap_or_else(|| key.clone());
        let (system, game) = first.split_once('/').unwrap_or(("", first.as_str()));
        file.system = system.to_string();
        file.game = game.to_string();
        file.key = key.clone();
        plan.files.push(file);
    }
    Ok(())
}

/// Packs of guidebooks asked for on their own (D225's Install), by their ids: every
/// guidebook of the game's.
pub fn add_alone(ix: &Index, lib: &Library, data: &Path, ids: &[String], plan: &mut Plan) -> Result<()> {
    let ids: Vec<&String> = ids.iter().filter(|i| gb::is_pack_id(i)).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let maker = maker(data);
    let all = gb::folders(ix, lib)?;
    for id in ids {
        if plan.files.iter().any(|f| f.role == crate::packs::ROLE && &f.pack == id) {
            continue;
        }
        let Some((f, w)) = all.iter().filter(|f| f.pack_id().as_ref() == Some(id)).find_map(|f| gb::wanted(f, &[]).map(|w| (f, w))) else {
            plan.left.push(Left { what: id.clone(), why: "The library has no guidebooks for this game now: look at the list again.".into() });
            continue;
        };
        let mut file = file_of(&maker, &w, file_name(f), plan);
        if file.url.is_empty() {
            continue;
        }
        file.game = f.game_title();
        plan.files.push(file);
    }
    Ok(())
}

/// Every game's pack of guidebooks, for `GET /v1/packs` (grouped under the game).
pub fn library(ix: &Index, lib: &Library, data: &Path, game_of: &dyn Fn(&gamerepo_core::maps::PackGame) -> Result<PackGame>) -> Result<Vec<LibraryPack>> {
    if gb::list(lib).is_empty() {
        return Ok(vec![]);
    }
    let maker = maker(data);
    let mut out = vec![];
    for f in gb::folders(ix, lib)? {
        let Some(pack) = info(&maker, &f) else { continue };
        if gb::wanted(&f, &[]).is_none() {
            continue;
        }
        let games = f.games.iter().map(game_of).collect::<Result<Vec<_>>>()?;
        let made = maker.cache.current(&gb::wanted(&f, &[]).unwrap_or_default());
        out.push(LibraryPack {
            group: games.first().map(|g| g.title.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| f.game_title()),
            system: games.first().map(|g| g.system.clone()).unwrap_or_default(),
            exported_at: made.map(|m| m.exported_at).unwrap_or_default(),
            games,
            cover: String::new(),
            pack,
        });
    }
    Ok(out)
}

/// `GET /v1/packs/<packId>/file` for a game's pack of guidebooks: every guidebook's.
pub fn whole(ix: &Index, lib: &Library, data: &Path, pack_id: &str) -> Result<Status> {
    if !gb::is_pack_id(pack_id) {
        return Ok(Status::Unknown);
    }
    let Some(w) = gb::folders(ix, lib)?.iter().filter(|f| f.pack_id().as_deref() == Some(pack_id)).find_map(|f| gb::wanted(f, &[])) else { return Ok(Status::Unknown) };
    Ok(maker(data).status(&w))
}

/// `GET /v1/packs/<packId>/made/<recipe>`: the pack of a set, as a plan gave it.
pub fn made(data: &Path, pack_id: &str, id: &str) -> Status {
    if !gb::is_pack_id(pack_id) || id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Status::Unknown;
    }
    maker(data).file(pack_id, id)
}
