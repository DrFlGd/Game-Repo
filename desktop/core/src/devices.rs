//! The devices Game Repo knows (docs/PLAN.md, "Save sync and the Devices page", D187–D188):
//! each one's record in `_library/devices/<id>/` (`gamerepo_wire::devices`), what it held when
//! last connected, and the changes waiting for it. Game Repo writes the queue; Game Repo
//! Server writes what a Game Repo Go device sends and what became of each change; so the
//! Devices page works from any computer the library's on, and from a server on a NAS.

use crate::config::write_json;
use crate::index::Index;
use crate::library::{Library, APP_DIR};
use crate::organise::{Op, Plan};
use crate::{export, layout, platforms};
use anyhow::{anyhow, bail, Result};
use gamerepo_wire::devices::{folder_name, Change, Contents, Done, Held, Info, Outcome, Queue, FOLDER};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The devices' records under a library's folder.
pub struct Records {
    dir: PathBuf,
}

fn read<T: DeserializeOwned + Default>(p: &Path) -> T {
    std::fs::read(p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write<T: serde::Serialize>(p: &Path, v: &T) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    write_json(p, &serde_json::to_value(v)?)
}

impl Records {
    pub fn of(library: &Path) -> Self {
        Self { dir: library.join(APP_DIR).join(FOLDER) }
    }

    fn at(&self, id: &str) -> PathBuf {
        self.dir.join(folder_name(id))
    }

    pub fn info(&self, id: &str) -> Option<Info> {
        let p = self.at(id).join("device.json");
        p.is_file().then(|| read(&p))
    }

    pub fn contents(&self, id: &str) -> Contents {
        read(&self.at(id).join("contents.json"))
    }

    pub fn queue(&self, id: &str) -> Queue {
        read(&self.at(id).join("queue.json"))
    }

    pub fn done(&self, id: &str) -> Done {
        read(&self.at(id).join("done.json"))
    }

    /// Every device with a record, the one seen last first.
    pub fn all(&self) -> Vec<Info> {
        let mut v: Vec<Info> = std::fs::read_dir(&self.dir).into_iter().flatten().flatten().filter_map(|e| {
            let p = e.path().join("device.json");
            p.is_file().then(|| read::<Info>(&p))
        }).filter(|i| !i.id.is_empty()).collect();
        v.sort_by(|a, b| b.seen.cmp(&a.seen).then(a.name.cmp(&b.name)));
        v
    }

    /// What a device sent when it connected (D188): its record and its contents, written
    /// by whoever it connected to.
    pub fn connected(&self, mut info: Info, contents: &Contents) -> Result<()> {
        info.seen = now();
        info.free = contents.free;
        info.size = contents.games.iter().map(|g| g.size).sum();
        info.games = contents.games.len() as u64;
        write(&self.at(&info.id).join("contents.json"), contents)?;
        write(&self.at(&info.id).join("device.json"), &info)
    }

    /// The changes waiting for a device.
    pub fn waiting(&self, id: &str) -> Vec<Change> {
        let q = self.queue(id);
        self.done(id).waiting(&q).into_iter().cloned().collect()
    }

    /// What became of some changes, added to `done.json`.
    pub fn record(&self, id: &str, outcomes: &[Outcome]) -> Result<()> {
        let mut d = self.done(id);
        for o in outcomes {
            let mut o = o.clone();
            if o.at == 0 {
                o.at = now();
            }
            d.outcomes.push(o);
        }
        write(&self.at(id).join("done.json"), &d)
    }

    /// Changes queued for a device (D187), each given an id; one already waiting (the same
    /// game, added or removed) isn't queued twice, and a game to add that's waiting to be
    /// removed (or the other way round) cancels that one instead.
    pub fn add(&self, id: &str, changes: Vec<Change>) -> Result<usize> {
        if self.info(id).is_none() {
            bail!("Game Repo doesn't know that device.");
        }
        let mut q = self.queue(id);
        let mut waiting: Vec<Change> = self.waiting(id);
        let mut added = 0;
        let mut n = q.changes.len();
        for mut c in changes {
            if c.action != "add" && c.action != "remove" {
                bail!("A device's change is add or remove, not {}.", c.action);
            }
            if waiting.iter().any(|w| w.key == c.key && w.action == c.action && w.game == c.game) {
                continue;
            }
            if let Some(at) = waiting.iter().position(|w| w.key == c.key && !c.key.is_empty() && w.action != c.action) {
                let w = waiting.remove(at);
                q.changes.retain(|x| x.id != w.id);
                added += 1;
                continue;
            }
            n += 1;
            // unique however many were cancelled before
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
            c.id = format!("{nanos:x}-{n}");
            c.made = now();
            waiting.push(c.clone());
            q.changes.push(c);
            added += 1;
        }
        write(&self.at(id).join("queue.json"), &q)?;
        Ok(added)
    }

    /// Changes taken back before the device took them.
    pub fn cancel(&self, id: &str, ids: &[String]) -> Result<()> {
        let waiting: Vec<String> = self.waiting(id).into_iter().map(|c| c.id).collect();
        let mut q = self.queue(id);
        q.changes.retain(|c| !(ids.contains(&c.id) && waiting.contains(&c.id)));
        write(&self.at(id).join("queue.json"), &q)
    }

    /// A folder device's own settings (D189), written by Game Repo.
    pub fn folder(&self, id: &str) -> FolderDevice {
        read(&self.at(id).join("folder.json"))
    }

    fn set_folder(&self, id: &str, f: &FolderDevice) -> Result<()> {
        write(&self.at(id).join("folder.json"), f)
    }

    /// A device's record gone (its saves stay in `Saves/`).
    pub fn forget(&self, id: &str) -> Result<()> {
        let d = self.at(id);
        if d.is_dir() {
            std::fs::remove_dir_all(&d)?;
        }
        Ok(())
    }

    /// The devices that held a game when last connected, for its page.
    pub fn holding(&self, key: &str) -> Vec<Value> {
        self.all().into_iter().filter(|i| self.contents(&i.id).games.iter().any(|g| g.key == key)).map(|i| json!({ "id": i.id, "name": i.name, "seen": i.seen })).collect()
    }

    /// The Devices page's list (D187): each device with its changes waiting.
    pub fn list(&self) -> Value {
        let v: Vec<Value> = self.all().into_iter().map(|i| {
            let w = self.waiting(&i.id);
            json!({ "info": i, "waiting": w.len(), "waiting_size": w.iter().filter(|c| c.action == "add").map(|c| c.size).sum::<u64>() })
        }).collect();
        json!(v)
    }

    /// A device as its page shows it: its record, its contents, and each change with what
    /// became of it (the newest hundred done).
    pub fn view(&self, id: &str) -> Result<Value> {
        let Some(info) = self.info(id) else { bail!("Game Repo doesn't know that device.") };
        let q = self.queue(id);
        let done = self.done(id);
        let contents = self.contents(id);
        let mut changes: Vec<Value> = q.changes.iter().map(|c| {
            let o = done.of(&c.id);
            // an added game the device has since is done, whatever it said when it took it
            let there = c.action == "add" && contents.games.iter().any(|g| g.key == c.key);
            let state = match (o.map(|o| o.state.as_str()), there) {
                (_, true) => "done",
                (Some(s), _) => s,
                (None, _) => "waiting",
            };
            json!({ "change": c, "state": state, "note": o.map(|o| o.note.clone()).unwrap_or_default(), "at": o.map(|o| o.at).unwrap_or(0) })
        }).collect();
        changes.reverse();
        let mut finished = 0;
        changes.retain(|c| {
            if c["state"] == "done" {
                finished += 1;
                finished <= 100
            } else {
                true
            }
        });
        Ok(json!({ "info": info, "contents": contents, "changes": changes }))
    }
}

// ------------------------------------------------------------ folder devices (D189)

/// The marker at a folder device's root, so it's known again on whichever drive or path it
/// comes back on.
pub const MARKER: &str = ".gamerepo/device.json";

/// `.gamerepo/device.json`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Marker {
    pub id: String,
    pub name: String,
    /// The library it's a device of
    pub library: String,
}

/// `folder.json` in a folder device's record: how its games are laid out on it, and the
/// export that placed them (its record says which games it holds from the library).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderDevice {
    pub layout: String,
    pub export: String,
}

fn marker_of(root: &Path) -> Option<Marker> {
    std::fs::read(root.join(MARKER)).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok()).filter(|m| !m.id.is_empty())
}

fn free_on(p: &Path) -> u64 {
    fs4::available_space(p).unwrap_or(0)
}

/// A card, drive or folder made a device of the library (*Add a device…*): its marker
/// written at its root, and its record.
pub fn add_folder(lib: &Library, folder: &Path, name: &str, layout_id: &str) -> Result<Info> {
    lib.writable()?;
    let Some(lib_id) = lib.id() else { bail!("The library has no id yet.") };
    if !folder.is_dir() {
        bail!("{} isn't a folder that's here.", folder.display());
    }
    let l = layout::get(Some(lib), layout_id).map_err(|_| anyhow!("There's no export layout {layout_id}."))?;
    export::device_place(&l)?;
    let records = Records::of(lib.root());
    if let Some(m) = marker_of(folder) {
        match records.info(&m.id) {
            Some(i) if m.library == lib_id => bail!("It's {} already.", i.name),
            _ if m.library != lib_id => bail!("It's {}, a device of another library.", m.name),
            _ => {}
        }
    }
    let name = name.trim();
    if name.is_empty() {
        bail!("Give the device a name.");
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let id = format!("card-{:x}", nanos);
    let marker = Marker { id: id.clone(), name: name.into(), library: lib_id };
    let at = folder.join(MARKER);
    std::fs::create_dir_all(at.parent().unwrap_or(folder)).map_err(|e| anyhow!("Couldn't write to {}: {e}", folder.display()))?;
    std::fs::write(&at, serde_json::to_vec_pretty(&marker)?).map_err(|e| anyhow!("Couldn't write to {}: {e}", folder.display()))?;
    let info = Info { id: id.clone(), name: name.into(), kind: "card".into(), by: "folder".into(), seen: now(), free: free_on(folder), layout: l.id.clone(), at: folder.display().to_string(), ..Default::default() };
    write(&records.at(&id).join("device.json"), &info)?;
    records.set_folder(&id, &FolderDevice { layout: l.id, export: String::new() })?;
    Ok(info)
}

/// Where a card, a drive or a share may be: each drive on Windows, the mount folders on
/// Linux and macOS, and wherever each folder device was found last (a share's path).
fn candidates(records: &Records) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if cfg!(windows) {
        v.extend((b'A'..=b'Z').map(|c| PathBuf::from(format!("{}:\\", c as char))));
    } else {
        let user = std::env::var("USER").unwrap_or_default();
        for top in [format!("/media/{user}"), format!("/run/media/{user}"), "/media".into(), "/mnt".into(), "/Volumes".into()] {
            v.extend(std::fs::read_dir(&top).into_iter().flatten().flatten().map(|e| e.path()));
        }
    }
    for i in records.all().into_iter().filter(|i| i.by == "folder" && !i.at.is_empty()) {
        v.push(PathBuf::from(i.at));
    }
    v
}

/// The folder devices that are here now, each with where it is.
pub fn present(lib: &Library) -> Vec<(Info, PathBuf)> {
    let Some(lib_id) = lib.id() else { return vec![] };
    let records = Records::of(lib.root());
    let mut seen: HashSet<String> = HashSet::new();
    let mut v = vec![];
    for c in candidates(&records) {
        let Some(m) = marker_of(&c) else { continue };
        if m.library != lib_id || !seen.insert(m.id.clone()) {
            continue;
        }
        if let Some(i) = records.info(&m.id).filter(|i| i.by == "folder") {
            v.push((i, c));
        }
    }
    v
}

/// Where a folder device is now, if it's here.
pub fn where_is(lib: &Library, id: &str) -> Option<PathBuf> {
    present(lib).into_iter().find(|(i, _)| i.id == id).map(|(_, p)| p)
}

/// A file's or folder's size, all of it.
fn size_of(p: &Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| size_of(&e.path())).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// The library's games an export placed (`system/name`), each with its files.
fn placed(lib: &Library, export_id: &str) -> BTreeMap<String, Vec<PathBuf>> {
    let mut v: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let Ok(e) = export::load(lib, export_id) else { return v };
    for f in e["files"].as_array().into_iter().flatten() {
        let (Some(game), Some(system), Some(path)) = (f["game"].as_str(), f["system"].as_str(), f["path"].as_str()) else { continue };
        if !f["carried"].as_str().unwrap_or("").is_empty() || game.is_empty() {
            continue;
        }
        v.entry(format!("{system}/{game}")).or_default().push(PathBuf::from(path));
    }
    v
}

/// What a folder device holds (D189): the library's games its export placed, and every other
/// game in its systems' folders, read through its layout. Its record is brought up to date.
pub fn read_folder(lib: &Library, id: &str, root: &Path) -> Result<Contents> {
    let records = Records::of(lib.root());
    let Some(mut info) = records.info(id).filter(|i| i.by == "folder") else { bail!("Game Repo doesn't know that device.") };
    let f = records.folder(id);
    let l = layout::get(Some(lib), &f.layout).map_err(|_| anyhow!("There's no export layout {}.", f.layout))?;
    let places = BTreeMap::from([(export::device_place(&l)?, root.to_path_buf())]);
    let rel = |p: &Path| p.strip_prefix(root).ok().map(|r| r.to_string_lossy().replace('\\', "/"));
    let mut games = vec![];
    let mut theirs: HashSet<PathBuf> = HashSet::new();
    for (key, files) in placed(lib, &f.export) {
        let here: Vec<&PathBuf> = files.iter().filter(|p| p.exists()).collect();
        let Some(first) = here.first() else { continue };
        theirs.extend(here.iter().map(|p| p.to_path_buf()));
        // a game on several discs is a folder of its own (D94): the folder's what's held
        for p in &here {
            if let Some(d) = p.parent().filter(|d| d.extension().is_some_and(|x| x == "m3u")) {
                theirs.insert(d.to_path_buf());
            }
        }
        let (system, name) = key.split_once('/').unwrap_or(("", &key));
        games.push(Held {
            id: rel(first).unwrap_or_default(), key: key.clone(), from: "library".into(), name: name.into(), system: system.into(),
            size: here.iter().map(|p| size_of(p)).sum(), ..Default::default()
        });
    }
    for p in platforms::all() {
        let Ok(Some(dir)) = layout::games_dir(&l, p.id, &places) else { continue };
        let mut exts: HashSet<&str> = p.exts.iter().copied().collect();
        exts.extend(platforms::formats(p.id).iter().copied().chain(["zip", "7z", "m3u", "cue", "gdi", "iso"]));
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = e.path();
            let file = e.file_name().to_string_lossy().to_string();
            if file.starts_with('.') || theirs.contains(&path) || !exts.contains(platforms::ext_of(&file).as_str()) {
                continue;
            }
            let name = Path::new(&file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(file.clone());
            games.push(Held { id: rel(&path).unwrap_or_default(), from: "elsewhere".into(), name, system: p.id.into(), size: size_of(&path), ..Default::default() });
        }
    }
    games.sort_by(|a, b| (&a.system, &a.name).cmp(&(&b.system, &b.name)));
    let contents = Contents { at: now(), free: free_on(root), games };
    info.at = root.display().to_string();
    records.connected(info, &contents)?;
    Ok(contents)
}

/// A folder device's waiting changes as one change on it (D189): the library's games it's
/// to hold laid out by its layout, games from elsewhere to remove set aside on it; with the
/// ids of the changes it carries.
pub fn plan_folder(ix: &Index, lib: &Library, id: &str, root: &Path) -> Result<(Plan, Vec<String>)> {
    let records = Records::of(lib.root());
    let Some(info) = records.info(id).filter(|i| i.by == "folder") else { bail!("Game Repo doesn't know that device.") };
    let f = records.folder(id);
    let waiting = records.waiting(id);
    let mut games: Vec<String> = placed(lib, &f.export).into_keys().collect();
    let mut aside: Vec<Op> = vec![];
    for c in &waiting {
        match c.action.as_str() {
            "add" if !games.contains(&c.key) => games.push(c.key.clone()),
            "remove" if !c.key.is_empty() => games.retain(|g| g != &c.key),
            "remove" => {
                // a game from elsewhere: its file (or folder) set aside on the device
                let p = root.join(&c.game);
                if c.game.split(['/', '\\']).any(|x| x == ".." || x.is_empty()) || !p.exists() {
                    continue;
                }
                let files: Vec<PathBuf> = if p.is_dir() { walk(&p) } else { vec![p] };
                for file in files {
                    let src = export::outside(root, &file);
                    aside.push(Op { op: "aside".into(), game: c.title.clone(), to: src.path.clone(), size: 0, from: vec![src], names: vec![], aside: vec![], note: None });
                }
            }
            _ => {}
        }
    }
    let export_id = Some(f.export.as_str()).filter(|e| !e.is_empty());
    let mut plan = if games.is_empty() && export_id.is_none() {
        // nothing of the library's yet: only what's set aside
        let mut p = export::device_plan(ix, lib, id, &info.name, &f.layout, root, &[], None)?;
        p.export = None;
        p
    } else {
        export::device_plan(ix, lib, id, &info.name, &f.layout, root, &games, export_id)?
    };
    plan.ops.extend(aside);
    Ok((plan, waiting.into_iter().map(|c| c.id).collect()))
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { v.extend(walk(&p)) } else { v.push(p) }
    }
    v
}

/// After a folder device's change ran: its export remembered, what became of each change,
/// and what it holds now.
pub fn folder_done(lib: &Library, id: &str, root: &Path, plan: Option<&Plan>, changes: &[String], result: &Value) -> Result<Contents> {
    let records = Records::of(lib.root());
    let mut f = records.folder(id);
    if let Some(e) = plan.and_then(|p| p.export.as_ref()).and_then(|e| e["id"].as_str()) {
        f.export = e.to_string();
        records.set_folder(id, &f)?;
    }
    let failed: HashMap<String, String> = result["failed"].as_array().into_iter().flatten()
        .filter_map(|x| Some((x["game"].as_str()?.to_string(), x["error"].as_str().or(x["why"].as_str()).unwrap_or("It couldn't be written.").to_string())))
        .collect();
    let queue = records.queue(id);
    let outcomes: Vec<Outcome> = changes.iter().map(|cid| {
        let c = queue.changes.iter().find(|c| &c.id == cid);
        let name = c.map(|c| c.key.split_once('/').map(|(_, n)| n.to_string()).unwrap_or_else(|| c.title.clone())).unwrap_or_default();
        match failed.get(&name) {
            Some(why) => Outcome { id: cid.clone(), state: "failed".into(), note: why.clone(), ..Default::default() },
            None => Outcome { id: cid.clone(), state: "done".into(), ..Default::default() },
        }
    }).collect();
    records.record(id, &outcomes)?;
    read_folder(lib, id, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organise;

    #[test]
    fn a_devices_changes_wait_for_it_and_follow_what_it_did() {
        let t = crate::library::tests::temp_dir("devices-records");
        let r = Records::of(&t);
        let info = Info { id: "dev-1".into(), name: "Steam Deck".into(), kind: "steamdeck".into(), by: "go".into(), ..Default::default() };
        let held = Held { id: "gb/Alpha.gb".into(), key: "gb/Alpha".into(), from: "library".into(), name: "Alpha".into(), system: "gb".into(), size: 10, ..Default::default() };
        r.connected(info, &Contents { at: 1, free: 99, games: vec![held] }).unwrap();
        assert_eq!(r.all()[0].games, 1);
        let add = |key: &str| Change { action: "add".into(), key: key.into(), size: 5, ..Default::default() };
        let remove = Change { action: "remove".into(), key: "gb/Alpha".into(), game: "gb/Alpha.gb".into(), ..Default::default() };
        assert_eq!(r.add("dev-1", vec![add("gb/Beta"), add("gb/Beta"), remove.clone()]).unwrap(), 2);
        assert_eq!(r.add("dev-1", vec![add("gb/Beta")]).unwrap(), 0, "waiting already");
        let w = r.waiting("dev-1");
        assert_eq!(w.len(), 2);
        assert_eq!(r.list()[0]["waiting_size"], 5);
        // adding back what's waiting to be removed takes the removal back
        assert_eq!(r.add("dev-1", vec![add("gb/Alpha")]).unwrap(), 1);
        assert_eq!(r.waiting("dev-1").len(), 1);
        // the device took Beta into its queue: it's shown done once it's there
        r.record("dev-1", &[Outcome { id: w[0].id.clone(), state: "taken".into(), ..Default::default() }]).unwrap();
        assert!(r.waiting("dev-1").is_empty());
        assert_eq!(r.view("dev-1").unwrap()["changes"][0]["state"], "taken");
        let beta = Held { id: "gb/Beta.gb".into(), key: "gb/Beta".into(), from: "library".into(), ..Default::default() };
        r.connected(r.info("dev-1").unwrap(), &Contents { at: 2, free: 90, games: vec![beta] }).unwrap();
        assert_eq!(r.view("dev-1").unwrap()["changes"][0]["state"], "done");
        assert_eq!(r.holding("gb/Beta").len(), 1);
        assert!(r.add("nobody", vec![add("x")]).is_err());
        r.forget("dev-1").unwrap();
        assert!(r.all().is_empty());
    }

    #[test]
    fn a_card_is_known_again_read_through_its_layout_and_changed_with_undo() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("devices-card");
        let card = dir.join("Card");
        std::fs::create_dir_all(card.join("roms/gb")).unwrap();
        std::fs::write(card.join("roms/gb/Mine.gb"), b"mine").unwrap();
        assert!(add_folder(&lib, &card, "Handheld", "esde").is_err(), "ES-DE asks for two folders");
        let info = add_folder(&lib, &card, "Handheld", "knulli").unwrap();
        assert!(card.join(MARKER).is_file());
        assert!(add_folder(&lib, &card, "Again", "knulli").unwrap_err().to_string().contains("Handheld already"));
        let at = where_is(&lib, &info.id).expect("found where it was last");
        let c = read_folder(&lib, &info.id, &at).unwrap();
        assert_eq!(c.games.iter().map(|g| (g.id.as_str(), g.from.as_str())).collect::<Vec<_>>(), vec![("roms/gb/Mine.gb", "elsewhere")]);
        let r = Records::of(lib.root());
        let add = Change { action: "add".into(), key: "gb/Alpha (Europe)".into(), ..Default::default() };
        r.add(&info.id, vec![add]).unwrap();
        let sync = |what: &str| {
            let (plan, ids) = plan_folder(&ix, &lib, &info.id, &card).unwrap();
            let done = organise::run(&ix, &lib, &plan, &std::sync::atomic::AtomicBool::new(false), &mut |_| {}).unwrap();
            assert_eq!(done["failed"], json!([]), "{what}: {done}");
            let c = folder_done(&lib, &info.id, &card, Some(&plan), &ids, &done).unwrap();
            (plan, c)
        };
        let (_, c) = sync("add");
        assert!(card.join("roms/gb/Alpha (Europe).gb").is_file() && card.join("roms/gb/gamelist.xml").is_file());
        let alpha = c.games.iter().find(|g| g.key == "gb/Alpha (Europe)").expect("Alpha's there from the library");
        assert_eq!((alpha.from.as_str(), alpha.id.as_str()), ("library", "roms/gb/Alpha (Europe).gb"));
        assert!(r.waiting(&info.id).is_empty() && r.view(&info.id).unwrap()["changes"][0]["state"] == "done");
        assert!(export::list(&lib).is_empty(), "a device's export is its own, not in Export's list");
        // both removed: the library's game and the one from elsewhere set aside on the card, with Undo
        let mine = Change { action: "remove".into(), game: "roms/gb/Mine.gb".into(), title: "Mine".into(), ..Default::default() };
        let gone = Change { action: "remove".into(), key: "gb/Alpha (Europe)".into(), game: "roms/gb/Alpha (Europe).gb".into(), ..Default::default() };
        r.add(&info.id, vec![mine, gone]).unwrap();
        let (plan, c) = sync("remove");
        assert!(c.games.is_empty(), "{:?}", c.games);
        assert!(!card.join("roms/gb/Mine.gb").exists() && !card.join("roms/gb/Alpha (Europe).gb").exists());
        organise::undo(&ix, &lib, &plan.id, &std::sync::atomic::AtomicBool::new(false), &mut |_| {}).unwrap();
        assert!(card.join("roms/gb/Mine.gb").is_file() && card.join("roms/gb/Alpha (Europe).gb").is_file());
        assert_eq!(read_folder(&lib, &info.id, &card).unwrap().games.len(), 2);
    }
}
