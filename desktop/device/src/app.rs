//! Game Repo Go's backend (docs/PLAN.md, "Phase 9b design", D96–D104): the commands its
//! pages call, the same on Windows and Android (the Windows app and the Android plugin
//! each pass [`App::call`] through), and the stand-in's (`gamerepo-cli device app`). It
//! holds the catalogue kept on the device (D98), the queue (D99), what ES-DE has on the
//! device (D100), removing safely (D101) and media on their own (D102), and the client's
//! settings, kept in its own data folder; the pairing token is kept by [`Secrets`].

use crate::catalogue::{self, Catalogue};
use crate::queue::{self, Ctx, Queue};
use crate::removing::{self, Ask as RemoveAsk};
use crate::scan::{self, Found};
use crate::{client, AddOptions, Device, Server};
use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use gamerepo_wire::api::{self, HeldFile, Identified, Info};
use gamerepo_wire::Plan;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// Where the pairing token is kept: Windows Credential Manager, Android's Keystore, or a
/// file for the stand-in and the tests ([`FileSecrets`]).
pub trait Secrets: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// Secrets in a file (the stand-in's and the tests'; never a real client's).
pub struct FileSecrets(pub PathBuf);

impl FileSecrets {
    fn all(&self) -> BTreeMap<String, String> {
        std::fs::read(&self.0).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
}

impl Secrets for FileSecrets {
    fn get(&self, name: &str) -> Option<String> {
        self.all().remove(name)
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        let mut m = self.all();
        m.insert(name.into(), value.into());
        crate::write_whole(&self.0, &serde_json::to_vec_pretty(&m)?)
    }

    fn delete(&self, name: &str) -> Result<()> {
        let mut m = self.all();
        if m.remove(name).is_some() {
            crate::write_whole(&self.0, &serde_json::to_vec_pretty(&m)?)?;
        }
        Ok(())
    }
}

/// The client's settings (D103), kept in `settings.json` in its data folder.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// ES-DE's folder and the ROMs folder, when they aren't where they're found (C3)
    pub esde: String,
    pub roms: String,
    /// The media kinds ticked, as last time (C22), and manuals
    pub media: Vec<String>,
    pub manuals: bool,
    /// Files fetched at once (1–4)
    pub at_once: usize,
    /// The ES-DE custom collection games are put in (C16): none, one (`collection_name`,
    /// every game the client adds), or came-from (the collection they were added from)
    pub collection: String,
    pub collection_name: String,
    /// The picture cache's size, in MB
    pub pictures_mb: u64,
    /// Every system the library has, not only those ES-DE has a folder for (C6)
    pub show_every_system: bool,
    /// Games already on the device shown in the Library (C6)
    pub show_on_device: bool,
    /// The device's name on the library, and its screen's longest side (pictures are made
    /// for it)
    pub name: String,
    pub screen: u32,
    /// The library paired with (its name)
    pub library: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            esde: String::new(),
            roms: String::new(),
            media: vec!["cover".into(), "screenshot".into(), "marquee".into()],
            manuals: false,
            at_once: 3,
            collection: "none".into(),
            collection_name: "Game Repo Go".into(),
            pictures_mb: 500,
            show_every_system: false,
            show_on_device: false,
            name: String::new(),
            screen: 1280,
            library: String::new(),
        }
    }
}

/// A job in the background (reading the library, checking games from elsewhere): how far
/// it got.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Work {
    pub running: bool,
    pub done: u64,
    pub of: u64,
    /// What went wrong, the last time
    pub error: String,
    /// When it last finished (seconds since 1970)
    pub at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synced: Option<catalogue::Synced>,
}

/// A game on the device (D100), as *On this device* shows it.
#[derive(Clone, Debug, Default, Serialize)]
pub struct OnDevice {
    /// Its path in the ROMs folder (what removing asks by)
    pub id: String,
    /// The library's game: the record's, or the one it was identified as
    #[serde(skip_serializing_if = "String::is_empty")]
    pub key: String,
    /// library (added by the client), known (from elsewhere, but the library has it),
    /// elsewhere (the library doesn't have it), unchecked (not asked yet)
    pub from: String,
    pub name: String,
    pub system: String,
    pub size: u64,
    pub last_played: String,
    pub play_count: u32,
    /// When it came (seconds since 1970)
    pub added: i64,
    /// Each media kind it has, and its size
    pub media: Vec<(String, u64)>,
    pub files: usize,
}

pub struct App {
    data: PathBuf,
    kind: String,
    /// Where ES-DE's folder may be (the user's folder; Android's storage volumes)
    homes: Vec<PathBuf>,
    secrets: Box<dyn Secrets>,
    settings: Mutex<Settings>,
    pub catalogue: Arc<Catalogue>,
    pub queue: Arc<Queue>,
    sync: Mutex<Work>,
    checking: Mutex<Work>,
    /// The library's games on the device, while the record's as it was and for a while
    held: Mutex<Option<(std::time::Instant, Option<std::time::SystemTime>, HashSet<String>)>>,
    /// Told what changed (queue, sync, device, settings), for the page
    events: Box<dyn Fn(&str) + Send + Sync>,
    me: Weak<App>,
    /// When the library was last looked for at another address
    looked: Mutex<Option<std::time::Instant>>,
}

/// How often the library is looked for at another address while it can't be reached,
/// and how long each look lasts at most (it ends once the library answers).
const FIND_AGAIN: Duration = Duration::from_secs(60);
const FIND_WAIT: Duration = Duration::from_secs(8);

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn arg<T: for<'a> Deserialize<'a>>(args: &Value, name: &str) -> Result<T> {
    serde_json::from_value(args.get(name).cloned().unwrap_or(Value::Null)).with_context(|| format!("{name} isn't as it should be"))
}

fn arg_or<T: for<'a> Deserialize<'a> + Default>(args: &Value, name: &str) -> T {
    args.get(name).cloned().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
}

impl App {
    /// The client with its data in `data`, ES-DE looked for in `homes` (each with an
    /// `ES-DE` folder), `kind` being what it tells the library it is (windows, android).
    pub fn open(data: &Path, homes: Vec<PathBuf>, kind: &str, secrets: Box<dyn Secrets>, events: Box<dyn Fn(&str) + Send + Sync>) -> Result<Arc<App>> {
        std::fs::create_dir_all(data)?;
        let settings: Settings = std::fs::read(data.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let catalogue = Arc::new(Catalogue::open(data)?);
        catalogue.set_picture_limit(settings.pictures_mb << 20);
        let app = Arc::new_cyclic(|me: &Weak<App>| {
            let (a, b) = (me.clone(), me.clone());
            let queue = Queue::open(
                data.join("queue.json"),
                Box::new(move || a.upgrade().ok_or_else(|| anyhow!("The client is closing."))?.ctx()),
                Box::new(move || {
                    if let Some(app) = b.upgrade() {
                        (app.events)("queue");
                    }
                }),
            );
            App { data: data.to_path_buf(), kind: kind.into(), homes, secrets, settings: Mutex::new(settings), catalogue, queue, sync: Mutex::default(), checking: Mutex::default(), held: Mutex::default(), events, me: me.clone(), looked: Mutex::default() }
        });
        let me = Arc::downgrade(&app);
        app.queue.on_away(Box::new(move || me.upgrade().is_some_and(|a| a.find_again())));
        app.queue.start();
        Ok(app)
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The library paired with. None: not paired.
    pub fn server(&self) -> Option<Server> {
        serde_json::from_str(&self.secrets.get("server")?).ok()
    }

    /// The device's ES-DE folders, found (or as Settings has them).
    pub fn device(&self) -> Result<Device> {
        let s = self.settings();
        let mut d = if !s.esde.is_empty() {
            let esde = PathBuf::from(&s.esde);
            let home = esde.parent().map(Path::to_path_buf).unwrap_or_else(|| esde.clone());
            Device::open_esde(&esde, &home)?
        } else {
            self.homes.iter().find_map(|h| Device::open(h).ok()).ok_or_else(|| anyhow!("ES-DE isn't installed on this device (no ES-DE folder with its settings was found). Install ES-DE and start it once, or choose its folder in Settings."))?
        };
        if !s.roms.is_empty() {
            d.places.insert("roms".into(), PathBuf::from(&s.roms));
        }
        Ok(d)
    }

    fn options(&self) -> AddOptions {
        let s = self.settings();
        AddOptions {
            media: s.media.clone(),
            manuals: s.manuals,
            size: s.screen,
            bios: true,
            at_once: s.at_once.clamp(1, api::DOWNLOADS),
            ..Default::default()
        }
    }

    fn ctx(&self) -> Result<Ctx> {
        let server = self.server().ok_or_else(|| anyhow!("This device isn't paired with a library: pair it in Settings."))?;
        Ok(Ctx { server, device: self.device()?, options: self.options() })
    }

    fn changed(&self, what: &str) {
        if what == "device" {
            *self.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        (self.events)(what);
    }

    /// A command from the page, with its arguments, and its answer.
    pub fn call(&self, cmd: &str, args: &Value) -> Result<Value> {
        Ok(match cmd {
            "status" => self.status(),
            "find" => {
                let wait = args.get("wait").and_then(Value::as_u64).unwrap_or(3000).clamp(200, 30_000);
                let found = crate::find(Duration::from_millis(wait))?;
                json!(found.into_iter().map(|(i, address)| json!({ "server": i.server, "library": i.library, "version": i.version, "api": i.api, "address": address })).collect::<Vec<_>>())
            }
            "pair" => self.pair(&arg::<String>(args, "address")?, &arg::<String>(args, "code")?)?,
            "unpair" => {
                if let Some(s) = self.server() {
                    let _ = s.call::<Value>("DELETE", "/v1/device", None::<&()>);
                }
                self.secrets.delete("server")?;
                self.catalogue.forget()?;
                self.update_settings(|s| s.library.clear())?;
                self.changed("sync");
                json!({})
            }
            "sync" => {
                self.start_sync();
                serde_json::to_value(self.sync_state())?
            }
            "systems" => json!(self.systems()?),
            "collections" => json!(self.catalogue.collections()?),
            "games" => {
                let mut a: catalogue::Ask = serde_json::from_value(args.clone()).unwrap_or_default();
                let s = self.settings();
                if a.system.is_empty() && a.collection.is_empty() && !s.show_every_system {
                    let shown: Vec<String> = self.systems()?.into_iter().map(|x| x["id"].as_str().unwrap_or("").to_string()).collect();
                    // no filter when ES-DE has a folder for every system with games
                    if self.catalogue.systems()?.iter().any(|x| x.games > 0 && !shown.contains(&x.id)) {
                        a.systems = Some(shown);
                    }
                }
                let held = self.keys_on_device();
                if !s.show_on_device {
                    a.hide = held.iter().cloned().collect();
                }
                let mut v = serde_json::to_value(self.catalogue.games(&a)?)?;
                for g in v["items"].as_array_mut().into_iter().flatten() {
                    let on = g["key"].as_str().is_some_and(|k| held.contains(k));
                    g["on_device"] = json!(on);
                }
                v
            }
            "game" => {
                let key: String = arg(args, "key")?;
                let game = self.catalogue.game(&key);
                let details = self.catalogue.details(self.server().as_ref(), &key).unwrap_or(None);
                json!({ "game": game, "details": details, "on_device": self.keys_on_device().contains(&key) })
            }
            "picture" => {
                let (key, kind): (String, String) = (arg(args, "key")?, arg::<Option<String>>(args, "kind")?.unwrap_or_else(|| "cover".into()));
                let size = args.get("size").and_then(Value::as_u64).unwrap_or(256) as u32;
                match self.catalogue.picture(self.server().as_ref(), &key, &kind, size)? {
                    Some(b) => json!(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(b))),
                    None => Value::Null,
                }
            }
            "prepare" => self.prepare(&arg::<Vec<String>>(args, "keys")?, args)?,
            "add" => self.add(&arg::<Vec<String>>(args, "keys")?, args)?,
            "queue" => serde_json::to_value(self.queue.shown())?,
            "pause" => {
                self.queue.pause(&arg_or::<Vec<u64>>(args, "ids"));
                json!({})
            }
            "resume" => {
                self.queue.resume(&arg_or::<Vec<u64>>(args, "ids"));
                json!({})
            }
            "cancel" => {
                self.queue.cancel(&arg::<Vec<u64>>(args, "ids")?);
                json!({})
            }
            "clear_done" => {
                self.queue.clear_done();
                json!({})
            }
            "on_device" => self.on_device()?,
            "make_room" => {
                let need: u64 = arg(args, "need")?;
                let rows = self.rows()?.0;
                let v: Vec<(String, u64, String, i64)> = rows.into_iter().map(|r| (r.id, r.size, r.last_played, r.added)).collect();
                json!(scan::make_room(&v, need))
            }
            "removal" => {
                let asks: Vec<RemoveAsk> = arg(args, "asks")?;
                let d = self.device()?;
                json!(removing::work_out(&d, &self.found(&d), &asks)?)
            }
            "remove" | "remove_media" => {
                let asks: Vec<RemoveAsk> = arg(args, "asks")?;
                if cmd == "remove_media" && asks.iter().any(|a| a.kinds.is_empty()) {
                    bail!("Say which media to remove.");
                }
                let d = self.device()?;
                let r = removing::remove(&d, &self.found(&d), &asks)?;
                self.tell_holding(&d);
                self.changed("device");
                json!(r)
            }
            "pending_removal" => json!(removing::pending(&self.device()?)),
            "finish_removal" => {
                let d = self.device()?;
                let r = removing::finish(&d)?;
                self.tell_holding(&d);
                self.changed("device");
                json!(r)
            }
            "put_back" => {
                let r = removing::put_back(&self.device()?)?;
                self.changed("device");
                json!(r)
            }
            "add_media" => self.add_media(&arg_or::<String>(args, "id"), &arg_or::<String>(args, "key"), &arg::<Vec<String>>(args, "kinds")?)?,
            "settings" => json!(self.settings()),
            "set_settings" => {
                let mut v = serde_json::to_value(self.settings())?;
                for (k, x) in args.as_object().into_iter().flatten() {
                    if v.get(k).is_none() {
                        bail!("There's no setting {k}.");
                    }
                    v[k] = x.clone();
                }
                let new: Settings = serde_json::from_value(v).context("A setting isn't as it should be")?;
                if !new.esde.is_empty() && !Path::new(&new.esde).join("settings").join("es_settings.xml").is_file() {
                    bail!("{} isn't ES-DE's folder (it has no settings/es_settings.xml).", new.esde);
                }
                if !new.roms.is_empty() && !Path::new(&new.roms).is_dir() {
                    bail!("{} isn't a folder.", new.roms);
                }
                self.update_settings(|s| *s = new.clone())?;
                self.catalogue.set_picture_limit(new.pictures_mb << 20);
                self.changed("settings");
                json!(new)
            }
            _ => bail!("There's no command {cmd}."),
        })
    }

    fn update_settings(&self, f: impl FnOnce(&mut Settings)) -> Result<()> {
        let mut s = self.settings.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut s);
        crate::write_whole(&self.data.join("settings.json"), &serde_json::to_vec_pretty(&*s)?)
    }

    fn sync_state(&self) -> Work {
        self.sync.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn status(&self) -> Value {
        let s = self.settings();
        let server = self.server();
        let esde = match self.device() {
            Ok(d) => {
                let roms = &d.places["roms"];
                json!({ "esde": d.places["esde"], "roms": roms, "media": d.places["media"], "free": fs4::available_space(crate::existing(roms)).unwrap_or(0) })
            }
            Err(e) => json!({ "error": format!("{e:#}") }),
        };
        let (games, _) = self.catalogue.counts();
        let q = self.queue.shown();
        let (pictures, picture_bytes) = self.catalogue.pictures_kept();
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "kind": self.kind,
            "paired": server.is_some(),
            "library": server.as_ref().map(|x| json!({ "name": s.library, "address": x.address, "server": x.server })),
            "device": esde,
            "removal": self.device().ok().and_then(|d| removing::pending(&d)),
            "queue": { "left": q.left, "offline": q.offline, "added": q.added, "size": q.size, "done": q.done, "speed": q.speed },
            "sync": self.sync_state(),
            "checking": self.checking.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "catalogue": { "games": games, "synced": self.catalogue.meta("synced").and_then(|v| v.parse::<i64>().ok()) },
            "pictures": { "kept": pictures, "size": picture_bytes },
        })
    }

    fn pair(&self, address: &str, code: &str) -> Result<Value> {
        let (info, _): (Info, String) = client::info(address)?;
        if info.api > api::API {
            bail!("This library is newer than Game Repo Go: update Game Repo Go first.");
        }
        let name = Some(self.settings().name).filter(|n| !n.trim().is_empty()).unwrap_or_else(device_name);
        let s = client::pair(address, code, &name, &self.kind)?;
        if let Some(old) = self.server().filter(|o| o.server != s.server) {
            let _ = old.call::<Value>("DELETE", "/v1/device", None::<&()>);
        }
        self.secrets.set("server", &serde_json::to_string(&s)?)?;
        self.update_settings(|x| {
            x.library = info.library.clone();
            if x.name.trim().is_empty() {
                x.name = name.clone();
            }
        })?;
        self.start_sync();
        Ok(json!({ "library": info.library, "server": s.server }))
    }

    /// The library looked for on the home network by its id when it can't be reached where
    /// it was (its computer given another address, its port changed): found, its address is
    /// kept and the pairing stays as it was, since only the certificate pinned at pairing is
    /// trusted wherever the library is. At most once a minute; true when it had moved.
    pub fn find_again(&self) -> bool {
        let Some(mut s) = self.server() else { return false };
        {
            let mut last = self.looked.lock().unwrap_or_else(|e| e.into_inner());
            if last.is_some_and(|t| t.elapsed() < FIND_AGAIN) {
                return false;
            }
            *last = Some(std::time::Instant::now());
        }
        // an address it announces, answering there with the certificate pinned (not only
        // an announcement, which anyone could make)
        let answers = |a: &str| a != s.address && client::info(a).is_ok_and(|(i, seen)| i.server == s.server && seen == s.fingerprint);
        let Ok(Some(address)) = crate::find_server(&s.server, FIND_WAIT, answers) else { return false };
        s.address = address;
        if serde_json::to_string(&s).map_err(anyhow::Error::from).and_then(|t| self.secrets.set("server", &t)).is_err() {
            return false;
        }
        self.changed("settings");
        true
    }

    /// The catalogue brought up to date in the background (D98).
    fn start_sync(&self) {
        {
            let mut w = self.sync.lock().unwrap_or_else(|e| e.into_inner());
            if w.running {
                return;
            }
            *w = Work { running: true, ..w.clone() };
        }
        let Some(app) = self.me.upgrade() else { return };
        std::thread::spawn(move || {
            let read = || match app.server() {
                Some(s) => {
                    let a = app.clone();
                    let r = app.catalogue.sync(&s, &move |done, of| {
                        {
                            let mut w = a.sync.lock().unwrap_or_else(|e| e.into_inner());
                            w.done = done;
                            w.of = of;
                        }
                        a.changed("sync");
                    });
                    if r.is_ok() {
                        if let Ok(d) = app.device() {
                            app.tell_holding(&d);
                        }
                    }
                    r
                }
                None => Err(anyhow!("This device isn't paired with a library.")),
            };
            let mut result = read();
            if matches!(&result, Err(e) if queue::is_offline(e)) && app.find_again() {
                result = read();
            }
            {
                let mut w = app.sync.lock().unwrap_or_else(|e| e.into_inner());
                w.running = false;
                w.at = now();
                match result {
                    Ok(x) => {
                        w.error.clear();
                        w.synced = Some(x);
                    }
                    Err(e) => w.error = if queue::is_offline(&e) { "The library can't be reached.".into() } else { format!("{e:#}") },
                }
            }
            app.changed("sync");
        });
    }

    /// What the library's told the device holds (best done; it's only shown there).
    fn tell_holding(&self, d: &Device) {
        if let Some(s) = self.server() {
            let _ = s.call::<Value>("PUT", "/v1/device", Some(&d.holding()));
        }
    }

    /// The library's systems to show (those with games; with ES-DE's folder for them on
    /// the device unless *Show every system* is on, C6), each saying if the folder's there.
    fn systems(&self) -> Result<Vec<Value>> {
        let every = self.settings().show_every_system;
        let roms = self.device().ok().map(|d| d.places["roms"].clone());
        let mut out = vec![];
        for x in self.catalogue.systems()?.into_iter().filter(|x| x.games > 0) {
            let there = roms.as_ref().is_some_and(|r| r.join(&x.folder).is_dir());
            if every || there || roms.is_none() {
                out.push(json!({ "id": x.id, "name": x.name, "folder": x.folder, "games": x.games, "size": x.size, "on_device": there }));
            }
        }
        Ok(out)
    }

    fn found(&self, d: &Device) -> Vec<Found> {
        scan::scan(d, &self.catalogue.systems().unwrap_or_default())
    }

    /// The library's games on the device: those the client added, and those from elsewhere
    /// the library has (identified).
    fn keys_on_device(&self) -> HashSet<String> {
        let Ok(d) = self.device() else { return HashSet::new() };
        // the scan's kept for half a minute, or until the record changes
        let record = std::fs::metadata(d.places["esde"].join("gamerepo").join("device.json")).and_then(|m| m.modified()).ok();
        if let Some((at, r, keys)) = &*self.held.lock().unwrap_or_else(|e| e.into_inner()) {
            if at.elapsed() < Duration::from_secs(30) && *r == record {
                return keys.clone();
            }
        }
        let mut keys: HashSet<String> = d.record().games.into_iter().map(|h| h.key).collect();
        if let Ok((rows, _)) = self.rows() {
            keys.extend(rows.into_iter().filter(|r| r.from == "known").map(|r| r.key));
        }
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = Some((std::time::Instant::now(), record, keys.clone()));
        keys
    }

    /// Every game on the device (D100), and those still to be checked against the library.
    fn rows(&self) -> Result<(Vec<OnDevice>, Vec<Found>)> {
        let d = self.device()?;
        let found = self.found(&d);
        let rec = d.record();
        let mut rows = vec![];
        let mut unchecked = vec![];
        let mut seen_keys: HashSet<String> = HashSet::new();
        let held_starts: Vec<(HashSet<String>, &api::Held)> = rec.games.iter().map(|h| (removing::starts(h), h)).collect();
        // a BIOS or device set games share is theirs, not a game of its own
        let shared: HashSet<&str> = rec.games.iter().flat_map(|h| &h.files).filter(|f| f.place == "roms" && !f.needed_by.is_empty()).map(|f| f.path.as_str()).collect();
        for g in found.iter().filter(|g| !shared.contains(g.id.as_str())) {
            let mut media: BTreeMap<String, u64> = BTreeMap::new();
            for m in &g.media {
                *media.entry(m.kind.clone()).or_default() += m.size;
            }
            let mut row = OnDevice { id: g.id.clone(), name: g.name.clone(), system: g.system.clone(), size: g.size, last_played: g.last_played.clone(), play_count: g.play_count, added: g.modified, files: g.files.len(), ..Default::default() };
            if let Some((_, h)) = held_starts.iter().find(|(s, _)| s.contains(&g.id)) {
                seen_keys.insert(h.key.clone());
                row.key = h.key.clone();
                row.from = "library".into();
                row.added = h.added as i64;
                if !h.title.is_empty() {
                    row.name = h.title.clone();
                }
                // the record's media too, by the library's names
                let have: HashSet<&str> = g.media.iter().map(|m| m.path.as_str()).chain(g.files.iter().map(|(p, _)| p.as_str())).collect();
                for f in h.files.iter().filter(|f| !have.contains(f.path.as_str())) {
                    if f.place == "media" {
                        *media.entry(f.role.clone()).or_default() += f.size;
                    }
                    row.size += f.size;
                }
            } else {
                let size = g.files.first().map(|f| f.1).unwrap_or(0);
                match self.catalogue.identified(&g.id, size, g.modified) {
                    Some(Some(k)) => {
                        row.key = k;
                        row.from = "known".into();
                    }
                    Some(None) => row.from = "elsewhere".into(),
                    None => {
                        row.from = "unchecked".into();
                        unchecked.push(g.clone());
                    }
                }
            }
            row.media = media.into_iter().collect();
            rows.push(row);
        }
        // the record's games ES-DE doesn't list (a system the catalogue doesn't have yet)
        for h in rec.games.iter().filter(|h| !seen_keys.contains(&h.key)) {
            let first = h.files.iter().find(|f| f.place == "roms" && !f.role.is_empty() && f.needed_by.is_empty()).or_else(|| h.files.iter().find(|f| f.place == "roms"));
            let mut media: BTreeMap<String, u64> = BTreeMap::new();
            for f in h.files.iter().filter(|f| f.place == "media") {
                *media.entry(f.role.clone()).or_default() += f.size;
            }
            rows.push(OnDevice {
                id: first.map(|f| f.path.clone()).unwrap_or_default(),
                key: h.key.clone(),
                from: "library".into(),
                name: h.title.clone(),
                system: h.system.clone(),
                size: h.files.iter().filter(|f| f.needed_by.is_empty()).map(|f| f.size).sum(),
                added: h.added as i64,
                media: media.into_iter().collect(),
                files: h.files.len(),
                ..Default::default()
            });
        }
        Ok((rows, unchecked))
    }

    /// *On this device* (D100): every game, the free space, and the check of games from
    /// elsewhere against the library started for those not checked yet.
    fn on_device(&self) -> Result<Value> {
        let d = self.device()?;
        let (rows, unchecked) = self.rows()?;
        if !unchecked.is_empty() {
            self.start_checking(unchecked);
        }
        let free = fs4::available_space(crate::existing(&d.places["roms"])).unwrap_or(0);
        let size: u64 = rows.iter().map(|r| r.size).sum();
        Ok(json!({ "games": rows, "free": free, "size": size, "checking": self.checking.lock().unwrap_or_else(|e| e.into_inner()).clone(), "removal": removing::pending(&d) }))
    }

    /// Games from elsewhere checked against the library (`POST /v1/identify`), in the
    /// background, each remembered by its path, size and modified time.
    fn start_checking(&self, todo: Vec<Found>) {
        let Some(s) = self.server() else { return };
        {
            let mut w = self.checking.lock().unwrap_or_else(|e| e.into_inner());
            if w.running {
                return;
            }
            *w = Work { running: true, of: todo.len() as u64, ..Default::default() };
        }
        let Some(app) = self.me.upgrade() else { return };
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let d = app.device()?;
                for chunk in todo.chunks(100) {
                    let mut ask = vec![];
                    let mut rows = vec![];
                    for g in chunk {
                        let size = g.files.first().map(|f| f.1).unwrap_or(0);
                        match scan::found(&d, g) {
                            Some(f) => ask.push((g, size, f)),
                            // nothing to tell it by: from elsewhere
                            None => rows.push((g.id.clone(), size, g.modified, None)),
                        }
                    }
                    if !ask.is_empty() {
                        let body: Vec<&api::Found> = ask.iter().map(|(_, _, f)| f).collect();
                        let answer: Vec<Identified> = s.call("POST", "/v1/identify", Some(&body))?;
                        let by_name: HashMap<String, Option<String>> = answer.into_iter().map(|i| (i.name, i.game)).collect();
                        for (g, size, _) in &ask {
                            rows.push((g.id.clone(), *size, g.modified, by_name.get(&g.id).cloned().flatten()));
                        }
                    }
                    app.catalogue.set_identified(&rows)?;
                    app.checking.lock().unwrap_or_else(|e| e.into_inner()).done += chunk.len() as u64;
                    app.changed("device");
                }
                Ok(())
            })();
            {
                let mut w = app.checking.lock().unwrap_or_else(|e| e.into_inner());
                w.running = false;
                w.at = now();
                w.error = match result {
                    Ok(()) => String::new(),
                    Err(e) if queue::is_offline(&e) => "The library can't be reached.".into(),
                    Err(e) => format!("{e:#}"),
                };
            }
            app.changed("device");
        });
    }

    /// Before *Add to device* (D99, C9, C22): the games' size, each media kind the library
    /// has for them with its size (those ticked as last time), and the free space.
    fn prepare(&self, keys: &[String], args: &Value) -> Result<Value> {
        let s = self.server().ok_or_else(|| anyhow!("This device isn't paired with a library: pair it in Settings."))?;
        let mut o = self.options();
        if let Some(m) = args.get("media").and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok()) {
            o.media = m;
        }
        if let Some(m) = args.get("manuals").and_then(Value::as_bool) {
            o.manuals = m;
        }
        let plan: Plan = s.call("POST", "/v1/plans", Some(&o.ask(keys)))?;
        let media_kind = |role: &str| role == gamerepo_wire::MANUAL.0 || gamerepo_wire::MEDIA.iter().any(|(k, _, _)| *k == role);
        let games: u64 = plan.files.iter().filter(|f| !media_kind(&f.role)).map(|f| f.size).sum();
        let d = self.device()?;
        let (todo, _) = crate::to_fetch(&d, &plan)?;
        let to_fetch: u64 = todo.iter().map(|&i| plan.files[i].size).sum();
        let names: HashMap<&str, &str> = gamerepo_wire::MEDIA.iter().chain([&gamerepo_wire::MANUAL]).map(|(k, n, _)| (*k, *n)).collect();
        let kinds: Vec<Value> = plan
            .kinds
            .iter()
            .map(|k| json!({ "kind": k.kind, "name": names.get(k.kind.as_str()).copied().unwrap_or(&k.kind), "games": k.games, "size": k.size, "estimated": k.estimated, "ticked": if k.kind == "manual" { o.manuals } else { o.media.contains(&k.kind) } }))
            .collect();
        let free = fs4::available_space(crate::existing(&d.places["roms"])).unwrap_or(0);
        Ok(json!({ "games": keys.len(), "size": games, "kinds": kinds, "to_fetch": to_fetch, "free": free, "left": plan.left, "notes": plan.notes }))
    }

    /// Games put in the queue with the media kinds chosen, which are remembered (C22).
    fn add(&self, keys: &[String], args: &Value) -> Result<Value> {
        let s = self.settings();
        let media: Vec<String> = args.get("media").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or(s.media.clone());
        let manuals = args.get("manuals").and_then(Value::as_bool).unwrap_or(s.manuals);
        // the ES-DE custom collection they go in too (C16)
        let collection = match s.collection.as_str() {
            "one" => s.collection_name.clone(),
            "came-from" => {
                let id = arg_or::<String>(args, "collection");
                self.catalogue.collections()?.into_iter().find(|c| c.id == id).map(|c| c.name).unwrap_or_default()
            }
            _ => String::new(),
        };
        self.update_settings(|x| {
            x.media = media.clone();
            x.manuals = manuals;
        })?;
        let items: Vec<queue::Item> = keys
            .iter()
            .map(|k| {
                let g = self.catalogue.game(k);
                queue::Item {
                    key: k.clone(),
                    title: g.as_ref().map(|g| g.title.clone()).unwrap_or_else(|| k.clone()),
                    system: g.as_ref().map(|g| g.system.clone()).unwrap_or_else(|| k.split('/').next().unwrap_or("").to_string()),
                    media: media.clone(),
                    manuals,
                    collection: collection.clone(),
                    ..Default::default()
                }
            })
            .collect();
        Ok(json!(self.queue.add(items)))
    }

    /// Media added on its own to a game on the device (D102): one the client added, or one
    /// from elsewhere the library has (whose files the record then takes in).
    fn add_media(&self, id: &str, key: &str, kinds: &[String]) -> Result<Value> {
        let (rows, _) = self.rows()?;
        let row = rows.iter().find(|r| (!id.is_empty() && r.id == id) || (!key.is_empty() && r.key == key)).ok_or_else(|| anyhow!("That game isn't on this device."))?;
        if row.key.is_empty() {
            bail!("The library doesn't have {}, so it has no media for it.", row.name);
        }
        // the game's files as the device has them, so its media's named for them (and a
        // game from elsewhere is taken into the record with them)
        let d = self.device()?;
        let found = self.found(&d);
        let adopt: Vec<HeldFile> = match found.iter().find(|g| g.id == row.id) {
            Some(g) => g.files.iter().map(|(p, size)| HeldFile { place: "roms".into(), path: p.clone(), size: *size, role: "rom".into(), ..Default::default() }).collect(),
            None if row.from == "known" => bail!("{} isn't on this device any more.", row.name),
            None => vec![],
        };
        let item = queue::Item {
            key: row.key.clone(),
            title: row.name.clone(),
            system: self.catalogue.game(&row.key).map(|g| g.system).unwrap_or_else(|| row.system.clone()),
            media: kinds.iter().filter(|k| *k != gamerepo_wire::MANUAL.0).cloned().collect(),
            manuals: kinds.iter().any(|k| k == gamerepo_wire::MANUAL.0),
            media_only: true,
            adopt,
            ..Default::default()
        };
        Ok(json!(self.queue.add(vec![item])))
    }
}

/// The device's name, as the library shows it until it's named in Settings.
pub fn device_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Game Repo Go".into())
}
