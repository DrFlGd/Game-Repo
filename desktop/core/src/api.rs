//! The app's commands, behind one entry point ([`App::call`]) so the Tauri app
//! and `gamerepo-cli serve` (the tests' stand-in for the window) share them exactly.

use crate::config::{AppConfig, Prefs};
use crate::index::{self, Index};
use crate::library::{self, Library};
use crate::{audit, collection, dat, export, makedat, organise, platforms, roots, scan, starter, watch};
use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// A lock's guard, poisoned or not (review L7): a panic on another thread that held
/// one (caught by its job) mustn't take the app's jobs, changes and server with it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Where the app keeps things on this computer.
#[derive(Clone, Debug)]
pub struct AppPaths {
    /// config.json and prefs.json
    pub config_dir: PathBuf,
    /// the index of each library (index/<library id>.sqlite) and caches
    pub data_dir: PathBuf,
    /// how the page reaches library files: "library://localhost/", "/library/"...
    pub library_url: String,
    /// Game Repo Server's data folder (one server per computer, D89)
    pub server_data: PathBuf,
    /// How to start Game Repo Server: its program and the arguments before its own
    /// (empty: it isn't installed beside the app)
    pub server_program: Vec<String>,
}

pub enum Reply {
    Json(Value),
    Bytes(Vec<u8>),
}

/// Long work runs in the background as a job; the page polls `job` and can stop it.
#[derive(Clone, Default)]
struct Job {
    id: String,
    label: String,
    done: bool,
    error: Option<String>,
    result: Value,
    progress: Value,
    started: String,
    cancel: Arc<AtomicBool>,
    /// started by the app itself (a scan of what the folder watcher saw change), not the page
    auto: bool,
}

impl Job {
    fn to_json(&self) -> Value {
        json!({ "id": self.id, "label": self.label, "done": self.done, "error": self.error, "result": self.result, "progress": self.progress, "started": self.started, "auto": self.auto })
    }
}

/// The label of a scan's job (one at a time).
const SCANNING: &str = "Scanning";

/// What the folder watcher saw change, waiting for a scan.
#[derive(Default)]
struct Changed {
    /// too much to list: scan everything
    all: bool,
    paths: Vec<PathBuf>,
}

/// What marks the running "change" as work writing games' details, before its label.
const META_WRITER: &str = "meta:";

/// Why something can't start now: what's running (`running`, the change's id or a
/// `META_WRITER` label), then what to do.
fn busy(running: &str, then: &str) -> String {
    match running.strip_prefix(META_WRITER) {
        Some(label) => format!("{label} is running: {then}."),
        None => format!("A change is running: {then}."),
    }
}

/// Clears the running change when its job ends, however it ends.
struct ChangeDone(Arc<App>);

impl Drop for ChangeDone {
    fn drop(&mut self) {
        *lock(&self.0.change) = None;
    }
}

/// The open library's index, with the id of the library it belongs to.
type OpenIndex = Option<(String, Index)>;

pub struct App {
    pub paths: AppPaths,
    library: RwLock<Option<Library>>,
    /// the open library's index, opened on first use
    index: Arc<Mutex<OpenIndex>>,
    jobs: Mutex<Vec<Job>>,
    job_seq: Mutex<u64>,
    /// the change (organising, Undo) running now, by its id: one at a time
    change: Mutex<Option<String>>,
    /// what the folder watcher watches (library id and folders), and the watcher
    watched: Mutex<Option<String>>,
    watcher: Mutex<Option<watch::Watcher>>,
    changed: Mutex<Option<Changed>>,
    /// off for the command line, which runs one command and ends
    watching: AtomicBool,
    /// the library.json the systems added by hand were last read from, and when it changed
    systems_read: Mutex<Option<(PathBuf, Option<std::time::SystemTime>)>>,
    /// the layout editor's sample games (D39): (collection and what matched then, the sample)
    layout_sample: Mutex<Option<(String, Value)>>,
    /// Game Repo Server when this app started it: holding its input, which it watches to
    /// stop when the app ends (unless it's kept)
    server: Mutex<Option<std::process::Child>>,
    /// whether the server starts with Windows, as last read
    with_windows: Mutex<Option<bool>>,
    /// Why the server didn't start when the app opened (shown in Settings)
    serve_error: Mutex<Option<String>>,
}

/// The commands that change the library without a job: Game Repo Server is told after
/// them (and after every job), so the devices see the change.
const TELLS_SERVER: &[&str] = &[
    "addon_link", "collection_delete", "collection_from", "collection_hand", "collection_save", "collection_undo", "collection_waiting_leave", "dat_remove", "dat_use",
    "library_rename", "meta_choose", "meta_clear", "meta_edit", "meta_pick", "meta_pin", "meta_set_details_from", "roots_add", "roots_remove",
    "roots_update", "systems_remove", "systems_set",
];

/// What `library()` says before any library was ever opened (the page then asks where).
const NO_LIBRARY: &str = "No library is open yet.";

fn e2s(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn arg<'a>(args: &'a Value, k: &str) -> Result<&'a str> {
    args[k].as_str().ok_or_else(|| anyhow!("missing {k}"))
}

impl App {
    pub fn new(paths: AppPaths) -> Result<Arc<Self>> {
        crate::tools::configure(&AppConfig::load(&paths.config_dir.join("config.json")).tools);
        Ok(Arc::new(Self {
            paths,
            library: RwLock::new(None),
            index: Arc::new(Mutex::new(None)),
            jobs: Mutex::new(vec![]),
            job_seq: Mutex::new(0),
            change: Mutex::new(None),
            watched: Mutex::new(None),
            watcher: Mutex::new(None),
            changed: Mutex::new(None),
            watching: AtomicBool::new(true),
            systems_read: Mutex::new(None),
            layout_sample: Mutex::new(None),
            server: Mutex::new(None),
            with_windows: Mutex::new(None),
            serve_error: Mutex::new(None),
        }))
    }

    fn config_path(&self) -> PathBuf {
        self.paths.config_dir.join("config.json")
    }
    pub fn config(&self) -> AppConfig {
        AppConfig::load(&self.config_path())
    }
    fn prefs(&self) -> Prefs {
        Prefs::new(self.paths.config_dir.join("prefs.json"))
    }

    /// The open library (opening the configured one on first use). On the very
    /// first start there's none: the page asks where it should go.
    pub fn library(&self) -> Result<Library, String> {
        let open = self.library.read().unwrap().clone();
        let lib = match open {
            Some(l) => l,
            None => {
                let Some(path) = self.config().library else {
                    return Err(NO_LIBRARY.into());
                };
                self.open_library(&path).map_err(e2s)?
            }
        };
        self.read_systems(&lib);
        Ok(lib)
    }

    /// The library's own systems (D22) join the table whenever its library.json changes
    /// (systems added, a DAT given a new system, or another computer's changes).
    fn read_systems(&self, lib: &Library) {
        let path = lib.meta_path();
        let stamp = Some((path.clone(), std::fs::metadata(&path).and_then(|m| m.modified()).ok()));
        let mut read = lock(&self.systems_read);
        if *read != stamp {
            platforms::configure(&lib.meta());
            *read = stamp;
        }
    }

    fn open_library(&self, path: &Path) -> Result<Library> {
        let lib = Library::open(path)?;
        let mut cfg = self.config();
        if cfg.library.as_deref() != Some(path)
            || cfg.recent_libraries.first().map(PathBuf::as_path) != Some(path)
        {
            cfg.set_library(path);
            cfg.save(&self.config_path())?;
        }
        *self.library.write().unwrap() = Some(lib.clone());
        // the index follows the library: opened again on first use
        *lock(&self.index) = None;
        Ok(lib)
    }

    // ------------------------------------------------------------ the index

    /// Run `f` on the open library's index (opening or making it first), off the
    /// async threads: SQLite blocks.
    async fn with_index<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&Index) -> Result<T> + Send + 'static,
    {
        let lib = self.library()?;
        let (slot, data_dir) = (self.index.clone(), self.paths.data_dir.clone());
        tokio::task::spawn_blocking(move || index_of(&slot, &data_dir, &lib, f))
            .await
            .map_err(|e| e.to_string())?
            .map_err(e2s)
    }

    // ------------------------------------------------------------ jobs

    fn start_job(&self, label: &str) -> Job {
        let mut seq = lock(&self.job_seq);
        *seq += 1;
        let job = Job {
            id: format!("job{}", *seq),
            label: label.into(),
            started: library::now(),
            ..Default::default()
        };
        let mut jobs = lock(&self.jobs);
        // keep running jobs and the last few finished ones
        let finished = jobs.iter().filter(|j| j.done).count();
        let mut drop_n = finished.saturating_sub(20);
        jobs.retain(|j| {
            let gone = j.done && drop_n > 0;
            if gone {
                drop_n -= 1;
            }
            !gone
        });
        jobs.push(job.clone());
        job
    }

    /// What a running job has got to: `{ item, items, name }` or `{ text }`, as the page shows it.
    fn job_progress(&self, id: &str, progress: Value) {
        if let Some(j) = lock(&self.jobs).iter_mut().find(|j| j.id == id) {
            j.progress = progress;
        }
    }

    fn job_done(&self, id: &str, r: Result<Value, String>) {
        if let Some(j) = lock(&self.jobs).iter_mut().find(|j| j.id == id) {
            j.done = true;
            match r {
                Ok(v) => j.result = v,
                Err(e) => j.error = Some(e),
            }
        }
    }

    /// Run `work` on a blocking thread as a job: the page follows it with `job`
    /// and stops it with `job_cancel` (which sets the flag `work` is given).
    fn spawn_job<F>(self: &Arc<Self>, label: &str, work: F) -> Value
    where
        F: FnOnce(Arc<Self>, String, Arc<AtomicBool>) -> Result<Value> + Send + 'static,
    {
        self.spawn_job_as(label, false, work)
    }

    fn spawn_job_as<F>(self: &Arc<Self>, label: &str, auto: bool, work: F) -> Value
    where
        F: FnOnce(Arc<Self>, String, Arc<AtomicBool>) -> Result<Value> + Send + 'static,
    {
        let mut job = self.start_job(label);
        if auto {
            job.auto = true;
            if let Some(j) = lock(&self.jobs).iter_mut().find(|j| j.id == job.id) {
                j.auto = true;
            }
        }
        let (app, id, cancel) = (self.clone(), job.id.clone(), job.cancel.clone());
        tokio::spawn(async move {
            let (a, jid) = (app.clone(), id.clone());
            let r = tokio::task::spawn_blocking(move || work(a, jid, cancel)).await;
            let r = match r {
                Ok(r) => r.map_err(e2s),
                Err(e) => Err(e.to_string()),
            };
            app.job_done(&id, r);
            app.tell_server();
            // changes seen while it ran are scanned now
            app.scan_changed();
        });
        json!({ "job": job.id })
    }

    /// A connection of a job's own to the library's index, so a long scan or import
    /// doesn't hold up the page's reads (SQLite's WAL lets them run side by side).
    fn job_index(data_dir: &Path, lib: &Library) -> Result<Index> {
        let id = lib.id().ok_or_else(|| anyhow!("{}", lib.read_only().unwrap_or("This library has no id in _library/library.json, so its index can't be found.")))?;
        Index::open(&index::path_for(data_dir, &id), &id)
    }

    /// Refuse to start a job while one with the same label is running.
    fn not_running(&self, label: &str) -> Result<(), String> {
        if lock(&self.jobs).iter().any(|j| !j.done && j.label == label) {
            return Err(format!("{label} is already running."));
        }
        Ok(())
    }

    fn scanning(&self) -> bool {
        lock(&self.jobs).iter().any(|j| !j.done && j.label == SCANNING)
    }

    /// Mark the change `id` as running (one at a time, and not during a scan); the
    /// job holds the returned guard, which clears it when the job ends.
    fn begin_change(self: &Arc<Self>, id: &str) -> Result<ChangeDone, String> {
        if self.scanning() {
            return Err("A scan is running: wait for it to finish first.".into());
        }
        let mut c = lock(&self.change);
        if let Some(r) = c.as_deref() {
            return Err(busy(r, "wait for it to finish first"));
        }
        *c = Some(id.to_string());
        Ok(ChangeDone(self.clone()))
    }

    /// Mark work that writes games' details as running (a scrape, Making details again,
    /// Identify): it holds the change's place, so no change, other such work or hand edit
    /// runs beside it (H17), though a scan may.
    fn begin_meta(self: &Arc<Self>, label: &str) -> Result<ChangeDone, String> {
        let mut c = lock(&self.change);
        if let Some(r) = c.as_deref() {
            return Err(busy(r, "wait for it to finish first"));
        }
        *c = Some(format!("{META_WRITER}{label}"));
        Ok(ChangeDone(self.clone()))
    }

    /// Refuse a hand edit of a game while a change or a scrape is writing games (H17):
    /// whichever wrote last would win.
    fn editable(&self) -> Result<(), String> {
        match lock(&self.change).as_deref() {
            Some(r) => Err(busy(r, "change it when that's finished")),
            None => Ok(()),
        }
    }

    /// A change that isn't a scrape (or the like) is running: no scan now.
    fn changing(&self) -> bool {
        lock(&self.change).as_deref().is_some_and(|r| !r.starts_with(META_WRITER))
    }

    /// Start a scan job (from the page or the folder watcher).
    fn start_scan(self: &Arc<Self>, lib: Library, opts: scan::Options, auto: bool) -> Value {
        let data_dir = self.paths.data_dir.clone();
        self.spawn_job_as(SCANNING, auto, move |app, id, cancel| {
            let ix = Self::job_index(&data_dir, &lib)?;
            scan::scan(&ix, &lib, &opts, &cancel, &mut |p| app.job_progress(&id, p))
        })
    }

    // ------------------------------------------------------------ changes made outside the app

    /// What the folder watcher should watch now: (what identifies it, the folders,
    /// the library's own folder). None when nothing is (watching off, read-only library).
    fn watch_wanted(&self) -> Option<(String, Vec<PathBuf>, PathBuf)> {
        if !self.watching.load(Ordering::Relaxed) {
            return None;
        }
        let lib = self.library.read().unwrap().clone()?;
        if !self.config().scan.watch || lib.writable().is_err() {
            return None;
        }
        let dirs: Vec<PathBuf> = roots::list(&lib).into_iter().filter(|r| r.found()).map(|r| r.dir).collect();
        if dirs.is_empty() {
            return None;
        }
        let key = format!("{}|{}", lib.root().display(), dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join("|"));
        Some((key, dirs, lib.root().join(library::APP_DIR)))
    }

    /// Whether the folders with games are watched at all (the command line doesn't).
    pub fn watch_folders(&self, on: bool) {
        self.watching.store(on, Ordering::Relaxed);
    }

    /// Watch the folders with games on local drives (again, when they or the setting
    /// changed); what changes there is scanned once it settles.
    fn ensure_watch(self: &Arc<Self>) {
        let want = self.watch_wanted();
        {
            let mut w = lock(&self.watched);
            if w.as_deref() == want.as_ref().map(|w| w.0.as_str()) {
                return;
            }
            *w = want.as_ref().map(|w| w.0.clone());
        }
        *lock(&self.watcher) = None;
        let Some((key, dirs, skip)) = want else { return };
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let app = Arc::downgrade(self);
        // watching a big tree takes a while on some systems: not on the caller's time
        tokio::task::spawn_blocking(move || {
            let dirs: Vec<PathBuf> = dirs.into_iter().filter(|d| !watch::is_network(d)).collect();
            if dirs.is_empty() {
                return;
            }
            let on = app.clone();
            let r = watch::watch(&dirs, &skip, watch::SETTLE, move |paths| {
                if let Some(a) = on.upgrade() {
                    let _rt = handle.enter();
                    a.changed_outside(paths);
                }
            });
            let Some(a) = app.upgrade() else { return };
            match r {
                // only if it's still what should be watched
                Ok(w) if lock(&a.watched).as_deref() == Some(key.as_str()) => *lock(&a.watcher) = Some(w),
                Ok(_) => {}
                Err(e) => eprintln!("can't watch the folders with games: {e}"),
            }
        });
    }

    fn changed_outside(self: &Arc<Self>, paths: Option<Vec<PathBuf>>) {
        {
            let mut c = lock(&self.changed);
            let c = c.get_or_insert_with(Changed::default);
            match paths {
                Some(p) => c.paths.extend(p),
                None => c.all = true,
            }
        }
        self.scan_changed();
    }

    /// Scan what the watcher saw change, unless a scan or a change is running (then
    /// this runs again when it ends).
    fn scan_changed(self: &Arc<Self>) {
        if self.scanning() || self.changing() {
            return;
        }
        let Some(c) = lock(&self.changed).take() else { return };
        let Ok(lib) = self.library() else { return };
        let settings = self.config().scan;
        if lib.writable().is_err() || !settings.watch {
            return;
        }
        let only = if c.all { None } else { Some(c.paths) };
        self.start_scan(lib, scan::Options { full: settings.full, readers: settings.readers, only }, true);
    }

    // ------------------------------------------------------------ Game Repo Server (D89)

    fn settings_set(&self, f: impl FnOnce(&mut crate::config::ServeSettings)) -> Result<()> {
        let mut cfg = self.config();
        f(&mut cfg.serve);
        cfg.save(&self.config_path())
    }

    /// Start Game Repo Server with the open library.
    fn serve_start(&self) -> Result<()> {
        let lib = self.library().map_err(|e| anyhow!(e))?;
        let cfg = self.config().serve;
        let index_from = lib.id().map(|id| index::path_for(&self.paths.data_dir, &id));
        let child = crate::serving::start(&crate::serving::Start {
            program: &self.paths.server_program,
            data: &self.paths.server_data,
            library: lib.root(),
            port: cfg.port,
            index_from: index_from.as_deref(),
            keep: cfg.keep,
        })?;
        *lock(&self.server) = Some(child);
        *lock(&self.serve_error) = None;
        Ok(())
    }

    fn serve_stop(&self) -> Result<()> {
        if gamerepo_wire::control::running(&self.paths.server_data).is_some() {
            crate::serving::stop(&self.paths.server_data)?;
        }
        if let Some(mut c) = lock(&self.server).take() {
            let _ = c.wait();
        }
        Ok(())
    }

    fn serve_status(&self) -> Result<Value> {
        let (running, status, devices) = crate::serving::state(&self.paths.server_data);
        let lib = self.library.read().unwrap().clone();
        let this = running.as_ref().is_some_and(|r| lib.as_ref().is_some_and(|l| Path::new(&r.library) == l.root()));
        let with_windows = *lock(&self.with_windows).get_or_insert_with(crate::serving::with_windows);
        Ok(json!({
            "installed": !self.paths.server_program.is_empty(),
            "running": running.is_some(),
            "this_library": this,
            "status": status,
            "devices": devices,
            "settings": self.config().serve,
            "windows": cfg!(windows),
            "with_windows": with_windows,
            // (the installed server: not `gamerepo-cli serve-devices`)
            "can_start_with_windows": cfg!(windows) && self.paths.server_program.len() == 1,
            "start_error": lock(&self.serve_error).clone(),
        }))
    }

    fn serve_call(&self, cmd: &str, args: &Value) -> Result<Value> {
        let data = &self.paths.server_data;
        let call = |method: &str, path: &str, body: Option<&Value>| gamerepo_wire::control::call(data, method, path, body).map_err(|e| anyhow!("{e}"));
        match cmd {
            "serve_status" => self.serve_status(),
            "serve_start" => {
                self.serve_start()?;
                self.settings_set(|s| s.on = true)?;
                self.serve_status()
            }
            "serve_stop" => {
                self.serve_stop()?;
                self.settings_set(|s| s.on = false)?;
                self.serve_status()
            }
            // the open library instead of the one it's serving
            "serve_restart" => {
                self.serve_stop()?;
                self.serve_start()?;
                self.settings_set(|s| s.on = true)?;
                self.serve_status()
            }
            "serve_pair" => call("POST", "/pair", None),
            "serve_unpair" => call("DELETE", "/pair", None),
            "serve_remove_device" => call("DELETE", &format!("/devices/{}", arg(args, "id")?), None),
            "serve_set" => {
                if let Some(keep) = args["keep"].as_bool() {
                    self.settings_set(|s| s.keep = keep)?;
                    if gamerepo_wire::control::running(data).is_some() {
                        call("POST", "/keep", Some(&json!({ "keep": keep })))?;
                    }
                }
                if let Some(port) = args.get("port").filter(|p| !p.is_null()) {
                    let port = port.as_u64().filter(|p| (1024..=65535).contains(p)).ok_or_else(|| anyhow!("A port is a number from 1024 to 65535."))? as u16;
                    if port != self.config().serve.port {
                        self.settings_set(|s| s.port = port)?;
                        // a server of this app's moves to it
                        if lock(&self.server).is_some() && gamerepo_wire::control::running(data).is_some() {
                            self.serve_stop()?;
                            self.serve_start()?;
                        }
                    }
                }
                if let Some(on) = args["with_windows"].as_bool() {
                    let [program] = self.paths.server_program.as_slice() else { bail!("Start with Windows is for Game Repo Server installed beside Game Repo.") };
                    crate::serving::set_with_windows(Path::new(program), on)?;
                    *lock(&self.with_windows) = Some(crate::serving::with_windows());
                }
                self.serve_status()
            }
            "serve_networks" => Ok(json!(crate::serving::networks())),
            _ => bail!("unknown command {cmd}"),
        }
    }

    /// Tell Game Repo Server the library changed, when it's serving this one.
    fn tell_server(&self) {
        if let Some(lib) = self.library.read().unwrap().as_ref() {
            crate::serving::changed(&self.paths.server_data, lib.root());
        }
    }

    /// When the app opens: serve again if it was serving when it closed (and the server
    /// didn't stay).
    pub fn serve_resume(&self) {
        if !self.config().serve.on || self.paths.server_program.is_empty() || gamerepo_wire::control::running(&self.paths.server_data).is_some() {
            return;
        }
        if let Err(e) = self.serve_start() {
            *lock(&self.serve_error) = Some(format!("Game Repo Server didn't start when Game Repo opened: {e:#}"));
        }
    }

    /// Before an update replaces the app (D137): the server it started stops, kept or not, as
    /// the installer replaces the server's program beside it too. Game Repo starts it again
    /// when it opens after the update (`serve_resume`).
    pub fn serve_before_update(&self) {
        let Some(child) = lock(&self.server).take() else { return };
        let _ = crate::serving::stop(&self.paths.server_data);
        drop(child);
    }

    /// When the app closes: the server it started stops with it, unless it's kept.
    pub fn serve_exit(&self) {
        let Some(child) = lock(&self.server).take() else { return };
        if !self.config().serve.keep {
            let _ = gamerepo_wire::control::call(&self.paths.server_data, "POST", "/stop", None);
        }
        // its input closes with it: a server that isn't kept stops then anyway
        drop(child);
    }

    // ------------------------------------------------------------ commands

    pub async fn call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        let r = self.call_inner(cmd, args).await;
        if r.is_ok() && TELLS_SERVER.contains(&cmd) {
            self.tell_server();
        }
        r
    }

    async fn call_inner(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        let j = |v: Value| Ok(Reply::Json(v));
        if !cmd.starts_with("job") {
            self.ensure_watch();
        }
        if ["organise_", "change_", "naming", "convert_", "tools", "format"].iter().any(|p| cmd.starts_with(p)) {
            return self.organise_call(cmd, args).await;
        }
        if ["pc_", "identify_", "sources", "launchbox_", "steam_", "add_"].iter().any(|p| cmd.starts_with(p)) {
            return self.pc_call(cmd, args).await;
        }
        if ["meta_", "scrape_", "docs_", "browse", "addon_"].iter().any(|p| cmd.starts_with(p)) {
            return self.meta_call(cmd, args).await;
        }
        if ["systems_", "collection", "export"].iter().any(|p| cmd.starts_with(p)) {
            return self.collection_call(cmd, args).await;
        }
        if cmd.starts_with("cheats_") {
            return self.cheat_call(cmd, args).await;
        }
        if cmd.starts_with("patches_") {
            return self.patch_call(cmd, args).await;
        }
        if ["layout", "bios", "magazine", "video", "backlog", "saves_", "devices_"].iter().any(|p| cmd.starts_with(p)) {
            return self.device_call(cmd, args).await;
        }
        if cmd.starts_with("serve_") {
            let (app, cmd) = (self.clone(), cmd.to_string());
            return tokio::task::spawn_blocking(move || app.serve_call(&cmd, &args)).await.map_err(|e| e.to_string())?.map(Reply::Json).map_err(e2s);
        }
        match cmd {
            "app_info" => {
                let lib = self.library();
                let first_run = lib.as_ref().err().is_some_and(|e| e == NO_LIBRARY);
                j(json!({
                    "version": crate::VERSION,
                    "os": std::env::consts::OS,
                    "library_format": library::FORMAT,
                    "index_version": index::schema_version(),
                    "library": lib.as_ref().ok().map(Library::info),
                    "library_error": lib.err().filter(|_| !first_run),
                    "first_run": first_run,
                    "default_library": crate::config::default_library_path(),
                    "library_url": self.paths.library_url,
                    "recent_libraries": self.config().recent_libraries,
                    // where releases are published (D136)
                    "releases": format!("https://github.com/{}/releases", gamerepo_wire::RELEASES),
                }))
            }
            "prefs_get" => j(self.prefs().get()),
            "prefs_set" => {
                self.prefs().set(&args["prefs"]).map_err(e2s)?;
                j(Value::Null)
            }
            "library_open" => {
                let path = PathBuf::from(arg(&args, "path").map_err(e2s)?);
                let lib = self.open_library(&path).map_err(e2s)?;
                j(lib.info())
            }
            "library_rename" => {
                let name = arg(&args, "name").map_err(e2s)?.trim();
                if name.is_empty() {
                    return Err("A library needs a name.".into());
                }
                if name.chars().count() > 80 {
                    return Err("A library's name can be at most 80 characters.".into());
                }
                let lib = self.library()?;
                lib.update_meta(json!({ "name": name })).map_err(e2s)?;
                j(lib.info())
            }
            "index_info" => j(self.with_index(|ix| ix.info()).await?),
            "index_check" => {
                // SQLite's own check of the index file, as a job (it reads the whole file)
                let lib = self.library()?;
                let (slot, data_dir) = (self.index.clone(), self.paths.data_dir.clone());
                j(self.spawn_job("Checking the index", move |_, _, _| {
                    let found = index_of(&slot, &data_dir, &lib, |ix| ix.check())?;
                    Ok(json!({ "ok": found.is_empty(), "problems": found }))
                }))
            }
            "platforms" => j(platforms::list()),

            // ---------------------------------------------------- folders with games
            "roots" => {
                let lib = self.library()?;
                let on = self.config().scan.watch;
                j(json!(roots::list(&lib)
                    .iter()
                    .map(|r| {
                        let mut v = r.to_json();
                        let network = r.found() && watch::is_network(&r.dir);
                        v["network"] = json!(network);
                        // (a folder left where it was: Import's Move into the library… takes its games)
                        v["outside"] = json!(!roots::in_library(&lib, &r.dir));
                        v["watched"] = json!(on && r.found() && !network);
                        v
                    })
                    .collect::<Vec<_>>()))
            }
            "roots_add" => {
                let lib = self.library()?;
                let path = PathBuf::from(arg(&args, "path").map_err(e2s)?);
                j(roots::add(&lib, &path, args["platform"].as_str()).map_err(e2s)?.to_json())
            }
            "roots_locate" => {
                // the folder with games a folder is in, and where in it (D166), or null
                let lib = self.library()?;
                let path = PathBuf::from(arg(&args, "path").map_err(e2s)?);
                j(roots::locate(&lib, &path).map(|(r, folder)| json!({ "root": r.to_json(), "folder": folder })).unwrap_or(Value::Null))
            }
            "roots_update" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?;
                let path = args["path"].as_str().map(PathBuf::from);
                // "platform": null clears it; leaving it out keeps it
                let platform = args.get("platform").map(|p| p.as_str());
                let root = roots::update(&lib, id, path.as_deref(), platform).map_err(e2s)?;
                // what's already scanned there takes the new system hint now (review M10)
                let id = root.id.clone();
                self.with_index(move |ix| scan::reguess_root(ix, &lib, &id)).await?;
                j(root.to_json())
            }
            "roots_remove" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                roots::remove(&lib, &id).map_err(e2s)?;
                // what was scanned there leaves the index (nothing on disk is touched)
                self.with_index(move |ix| {
                    ix.conn().execute("DELETE FROM files WHERE root_id = ?1", [&id])?;
                    ix.conn().execute("DELETE FROM roots WHERE id = ?1", [&id])?;
                    audit::changed(ix.conn())
                })
                .await?;
                j(Value::Null)
            }

            // ---------------------------------------------------- scanning
            "scan_settings" => j(json!(self.config().scan)),
            "scan_settings_set" => {
                let mut cfg = self.config();
                if let Some(full) = args["full"].as_bool() {
                    cfg.scan.full = full;
                }
                if let Some(n) = args["readers"].as_u64() {
                    cfg.scan.readers = (n as usize).clamp(1, 16);
                }
                if let Some(w) = args["watch"].as_bool() {
                    cfg.scan.watch = w;
                }
                cfg.save(&self.config_path()).map_err(e2s)?;
                j(json!(cfg.scan))
            }
            "scan" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                self.not_running(SCANNING)?;
                if self.changing() {
                    return Err("A change is running: scan when it's finished.".into());
                }
                let settings = self.config().scan;
                // `paths`: only these folders (one just added for a system, D166)
                let only: Option<Vec<PathBuf>> = args["paths"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect());
                if only.is_none() {
                    // everything is read, so what the watcher saw is covered
                    *lock(&self.changed) = None;
                }
                let opts = scan::Options { full: args["full"].as_bool().unwrap_or(settings.full), readers: settings.readers, only };
                j(self.start_scan(lib, opts, false))
            }

            // ---------------------------------------------------- DATs
            "dats" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| {
                    dat::sync_library(ix, &lib)?;
                    let mut rows = dat::list(ix)?;
                    audit::mark_roles(ix.conn(), &lib, &mut rows)?;
                    Ok(rows)
                })
                .await?)
            }
            "dat_import" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let paths: Vec<PathBuf> = match &args["paths"] {
                    Value::Array(a) => a.iter().filter_map(Value::as_str).map(PathBuf::from).collect(),
                    _ => vec![PathBuf::from(arg(&args, "path").map_err(e2s)?)],
                };
                let answers = args["answers"].as_object().cloned().unwrap_or_default();
                let data_dir = self.paths.data_dir.clone();
                j(self.spawn_job("Importing DATs", move |app, id, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let (mut imported, mut ask, mut failed) = (vec![], vec![], vec![]);
                    for (i, p) in paths.iter().enumerate() {
                        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                            bail!("stopped");
                        }
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let of = if paths.len() > 1 { format!(" ({} of {})", i + 1, paths.len()) } else { String::new() };
                        app.job_progress(&id, json!({ "text": format!("{name}{of}"), "name": name }));
                        // read as it's parsed (D149): how far into the file, for a big one
                        let mut told = |done: u64, total: u64| {
                            let pc = if total > 0 { done * 100 / total } else { 0 };
                            app.job_progress(&id, json!({ "text": format!("{name}{of}, {pc}% read"), "name": name, "done": done, "total": total }));
                        };
                        match dat::import_with(&ix, &lib, p, &answers, &cancel, &mut told) {
                            Ok(done) => {
                                for d in done {
                                    match d {
                                        dat::Imported::Loaded(v) => imported.push(v),
                                        dat::Imported::NeedsPlatform { name, file } => {
                                            ask.push(json!({ "name": name, "file": file, "path": p.display().to_string() }))
                                        }
                                    }
                                }
                            }
                            Err(_) if cancel.load(std::sync::atomic::Ordering::Relaxed) => bail!("stopped"),
                            Err(e) => failed.push(json!({ "path": p.display().to_string(), "error": format!("{e:#}") })),
                        }
                    }
                    if imported.is_empty() && ask.is_empty() && !failed.is_empty() {
                        bail!("{}", failed[0]["error"].as_str().unwrap_or(""));
                    }
                    Ok(json!({ "imported": imported, "needs_platform": ask, "failed": failed }))
                }))
            }
            "starter_dats" => {
                // the No-Intro cartridge DATs from libretro's public copy (D17), for platforms without one
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                self.not_running("Getting starter DATs")?;
                let only: Vec<String> = args["platforms"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
                let again = args["again"].as_bool().unwrap_or(false);
                let source = self.config().dat_source.unwrap_or_else(|| starter::SOURCE.to_string());
                let data_dir = self.paths.data_dir.clone();
                j(self.spawn_job("Getting starter DATs", move |app, id, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    starter::get(&ix, &lib, &source, &only, again, &cancel, &mut |p| app.job_progress(&id, p))
                }))
            }
            "redump_dats" => {
                // Redump's own DATs for the disc systems (D167), downloaded again each time
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                self.not_running("Getting Redump's DATs")?;
                let only: Vec<String> = args["platforms"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect();
                let source = self.config().redump_source.unwrap_or_else(|| crate::redump::SOURCE.to_string());
                let data_dir = self.paths.data_dir.clone();
                j(self.spawn_job("Getting Redump's DATs", move |app, id, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    crate::redump::get(&ix, &lib, &source, &only, &cancel, &mut |p| app.job_progress(&id, p))
                }))
            }
            "dats_update_info" => {
                // what Update DATs… covers (D172): the systems with a DAT, and every one
                let lib = self.library()?;
                let updated = lib.meta()["dats_updated"].clone();
                j(self
                    .with_index(move |ix| {
                        dat::sync_library(ix, &lib)?;
                        let (have, arcade) = crate::datupdate::systems(ix, false)?;
                        let (all, _) = crate::datupdate::systems(ix, true)?;
                        Ok(json!({ "have": have, "every": all.len(), "arcade": arcade, "updated": updated }))
                    })
                    .await?)
            }
            "dats_update" => {
                // the newest DATs of every system (D172): Redump's own, then libretro's copies
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                self.not_running("Updating DATs")?;
                let cfg = self.config();
                let ask = crate::datupdate::Ask {
                    libretro: (args["libretro"].as_bool() != Some(false)).then(|| cfg.dat_source.clone().unwrap_or_else(|| starter::SOURCE.to_string())),
                    redump: (args["redump"].as_bool() != Some(false)).then(|| cfg.redump_source.clone().unwrap_or_else(|| crate::redump::SOURCE.to_string())),
                    every: args["every"].as_bool().unwrap_or(false),
                };
                if ask.libretro.is_none() && ask.redump.is_none() {
                    return Err("Tick libretro, Redump or both.".into());
                }
                let data_dir = self.paths.data_dir.clone();
                j(self.spawn_job("Updating DATs", move |app, id, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    crate::datupdate::update(&ix, &lib, &ask, &cancel, &mut |p| app.job_progress(&id, p))
                }))
            }
            // Phase 7 (D60): each arcade system's set type
            "arcade_sets" => {
                let lib = self.library()?;
                j(crate::arcade::set_types(&lib))
            }
            "arcade_set" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let kind = arg(&args, "set_type").map_err(e2s)?.to_string();
                let l2 = lib.clone();
                self.with_index(move |ix| crate::arcade::set_set_type(ix.conn(), &l2, &platform, &kind)).await?;
                j(crate::arcade::set_types(&lib))
            }
            "dat_use" => {
                let lib = self.library()?;
                let id = args["id"].as_i64().ok_or("missing id")?;
                self.with_index(move |ix| dat::use_version(ix, &lib, id)).await?;
                j(Value::Null)
            }
            "dat_remove" => {
                // one DAT, or several (`ids`: a DAT's older versions, D173)
                let lib = self.library()?;
                let ids: Vec<i64> = match args["ids"].as_array() {
                    Some(a) => a.iter().filter_map(Value::as_i64).collect(),
                    None => vec![args["id"].as_i64().ok_or("missing id")?],
                };
                self.with_index(move |ix| ids.iter().try_for_each(|id| dat::remove(ix, &lib, *id))).await?;
                j(Value::Null)
            }

            // ---------------------------------------------------- Home and the audit
            "home" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| {
                    dat::sync_library(ix, &lib)?;
                    audit::home(ix, &lib)
                })
                .await?)
            }
            "audit_summary" => {
                let lib = self.library()?;
                let (dat_arg, platform_arg) = (args["dat"].as_i64(), args["platform"].as_str().map(String::from));
                // `view`: a DAT's lists looked at, not made the system's main DAT (D118)
                let view = args["view"].as_bool().unwrap_or(false);
                j(self.with_index(move |ix| {
                    dat::sync_library(ix, &lib)?;
                    let conn = ix.conn();
                    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != 'bios'")?;
                    let mut with_dats: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    with_dats.sort_by_key(|p| platforms::name_of(p));
                    let dat_id = match (dat_arg, platform_arg.as_deref().or(with_dats.first().map(String::as_str))) {
                        (Some(d), _) => {
                            // the DAT picked for a platform stays its DAT
                            if lib.writable().is_ok() && !view {
                                audit::choose_dat(conn, &lib, d)?;
                            }
                            Some(d)
                        }
                        (None, Some(p)) => audit::primary_dat(conn, &lib, p)?,
                        (None, None) => None,
                    };
                    let plats: Vec<Value> = with_dats.iter().map(|p| json!({ "id": p, "name": platforms::name_of(p) })).collect();
                    // a system with no DAT (removed, or one of another library's) still gets the
                    // systems that have one, to pick from (review H23)
                    let Some(dat_id) = dat_id else {
                        return Ok(json!({ "summary": null, "platforms": plats, "dats": [] }));
                    };
                    let summary = audit::summary(conn, dat_id)?;
                    let platform = summary["dat"]["platform"].as_str().unwrap_or("").to_string();
                    let mut q = conn.prepare("SELECT id, name, version, also FROM dats WHERE platform = ?1 AND current = 1 AND version != 'custom' ORDER BY name")?;
                    let dats: Vec<Value> = q
                        .query_map([&platform], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "version": r.get::<_, String>(2)?, "also": r.get::<_, bool>(3)? })))?
                        .collect::<rusqlite::Result<_>>()?;
                    // the main DAT, and the counts of each DAT it's audited with too (D118)
                    let main = audit::primary_dat(conn, &lib, &platform)?;
                    let also: Vec<Value> = audit::also_dats(conn, &lib, &platform)?.into_iter().map(|d| audit::summary(conn, d)).collect::<Result<_>>()?;
                    // its custom DAT's counts, for the Variants tab (D119)
                    let variants = match crate::variants::custom_dat(conn, &platform)? {
                        Some(c) => audit::summary(conn, c)?,
                        None => Value::Null,
                    };
                    Ok(json!({ "summary": summary, "platforms": plats, "dats": dats, "variants": variants, "main": main, "also": also }))
                })
                .await?)
            }
            "audit_list" => {
                let lib = self.library()?;
                let dat_id = args["dat"].as_i64().ok_or("missing dat")?;
                let tab = arg(&args, "tab").map_err(e2s)?.to_string();
                let search = args["search"].as_str().unwrap_or("").to_string();
                let region = args["region"].as_str().unwrap_or("").to_string();
                let sort = args["sort"].as_str().unwrap_or("name").to_string();
                let (offset, limit) = (args["offset"].as_i64().unwrap_or(0).max(0), args["limit"].as_i64().unwrap_or(100));
                j(self.with_index(move |ix| audit::list_by(ix, &lib, dat_id, &tab, &search, &region, &sort, offset, limit)).await?)
            }
            // variants: the custom DAT of each system (docs/PLAN.md, "Phase 10 design", D119–D123)
            "audit_dats_set" => {
                // a system's main DAT and the DATs it's audited with too (D118)
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let main = args["main"].as_i64();
                let also: Vec<String> = serde_json::from_value(args["also"].clone()).map_err(|e| format!("also: {e}"))?;
                j(self.with_index(move |ix| {
                    lib.writable()?;
                    if let Some(m) = main {
                        audit::choose_dat(ix.conn(), &lib, m)?;
                    }
                    audit::set_also(ix.conn(), &lib, &platform, &also)?;
                    Ok(json!({ "main": audit::primary_dat(ix.conn(), &lib, &platform)?, "also": audit::also_dats(ix.conn(), &lib, &platform)? }))
                })
                .await?)
            }
            "variants_list" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                j(self.with_index(move |ix| crate::variants::list(ix, &lib, &platform)).await?)
            }
            "variants_add" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let adds: Vec<crate::variants::Add> = serde_json::from_value(args["adds"].clone()).map_err(|e| format!("adds: {e}"))?;
                j(self.with_index(move |ix| crate::variants::add(ix, &lib, &platform, &adds)).await?)
            }
            "variants_edit" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let name = arg(&args, "name").map_err(e2s)?.to_string();
                let new_name = args["new_name"].as_str().unwrap_or(&name).to_string();
                let original = args["original"].as_str().filter(|o| !o.is_empty()).map(String::from);
                j(self.with_index(move |ix| crate::variants::edit(ix, &lib, &platform, &name, &new_name, original)).await?)
            }
            "variants_remove" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let names: Vec<String> = serde_json::from_value(args["names"].clone()).map_err(|e| format!("names: {e}"))?;
                j(self.with_index(move |ix| crate::variants::remove(ix, &lib, &platform, &names)).await?)
            }
            "variants_undo" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                j(self.with_index(move |ix| crate::variants::undo(ix, &lib, &platform)).await?)
            }
            "variants_search" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let text = args["text"].as_str().unwrap_or("").to_string();
                j(self.with_index(move |ix| crate::variants::search(ix.conn(), &lib, &platform, &text, 50)).await?)
            }
            "variants_tags" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| Ok(json!(crate::variants::tag_list(ix.conn(), &lib)?))).await?)
            }
            "variants_tags_set" => {
                let lib = self.library()?;
                let tags: Vec<String> = serde_json::from_value(args["tags"].clone()).map_err(|e| format!("tags: {e}"))?;
                j(self.with_index(move |ix| {
                    crate::variants::set_tags(&lib, &tags)?;
                    Ok(json!(crate::variants::tag_list(ix.conn(), &lib)?))
                })
                .await?)
            }
            "audit_overview" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| audit::overview(ix, &lib)).await?)
            }
            "audit_fixdat" => {
                let lib = self.library()?;
                let dat_id = args["dat"].as_i64().ok_or("missing dat")?;
                j(self.with_index(move |ix| audit::fixdat(ix, &lib, dat_id, None)).await?)
            }
            "audit_save_report" => {
                let lib = self.library()?;
                let dat_id = args["dat"].as_i64().ok_or("missing dat")?;
                let format = args["format"].as_str().unwrap_or("txt").to_string();
                j(self.with_index(move |ix| audit::save_report(ix, &lib, dat_id, &format)).await?)
            }
            "make_dat" => {
                let lib = self.library()?;
                let root = arg(&args, "root").map_err(e2s)?.to_string();
                let name = arg(&args, "name").map_err(e2s)?.to_string();
                let folder = args["folder"].as_str().unwrap_or("").to_string();
                let platform = args["platform"].as_str().map(String::from);
                let data_dir = self.paths.data_dir.clone();
                j(self.spawn_job("Making a DAT", move |app, id, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    makedat::make(&ix, &lib, &root, &folder, &name, platform.as_deref(), &cancel, &mut |p| app.job_progress(&id, p))
                }))
            }
            "audit_report" => {
                let dat_id = args["dat"].as_i64().ok_or("missing dat")?;
                j(json!({ "text": self.with_index(move |ix| audit::report(ix, dat_id)).await? }))
            }

            "job" => {
                let id = arg(&args, "id").map_err(e2s)?;
                let jobs = lock(&self.jobs);
                j(jobs
                    .iter()
                    .find(|x| x.id == id)
                    .map(Job::to_json)
                    .ok_or("No such job.")?)
            }
            "jobs" => j(json!(self
                .jobs
                .lock()
                .unwrap()
                .iter()
                .map(Job::to_json)
                .collect::<Vec<_>>())),
            "job_cancel" => {
                let id = arg(&args, "id").map_err(e2s)?;
                if let Some(x) = lock(&self.jobs).iter().find(|x| x.id == id) {
                    x.cancel.store(true, Ordering::Relaxed);
                }
                j(Value::Null)
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// Organising a set (docs/PLAN.md, "Phase 2 design", 2a): planning and previewing a
    /// change, running it as a job, the history and Undo, and how games are named.
    async fn organise_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        match cmd {
            "organise_plan" => {
                let lib = self.library()?;
                let o = organise::Options {
                    dat_id: args["dat"].as_i64().ok_or("missing dat")?,
                    mode: args["mode"].as_str().unwrap_or("copy").to_string(),
                    out: args["out"].as_str().filter(|o| !o.trim().is_empty()).map(PathBuf::from),
                    format: args["format"].as_str().unwrap_or("keep").to_string(),
                    partial: args["partial"].as_bool().unwrap_or(false),
                    always_copy: args["always_copy"].as_bool().unwrap_or(false),
                    only_misnamed: args["only_misnamed"].as_bool().unwrap_or(false),
                    ..Default::default()
                };
                j(self
                    .with_index(move |ix| {
                        let plan = organise::plan(ix, &lib, &o)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            "organise_gather" => {
                let lib = self.library()?;
                let platform = args["platform"].as_str().filter(|p| !p.is_empty()).map(String::from);
                j(self
                    .with_index(move |ix| {
                        let plan = organise::gather_plan(ix, &lib, platform.as_deref())?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            "convert_plan" => {
                let lib = self.library()?;
                let o = crate::convert::Options {
                    dat_id: args["dat"].as_i64().ok_or("missing dat")?,
                    to: arg(&args, "to").map_err(e2s)?.to_string(),
                    games: args["games"].as_array().map(|a| a.iter().filter_map(|g| g.as_str().map(String::from)).collect()),
                    out: args["out"].as_str().filter(|o| !o.trim().is_empty()).map(PathBuf::from),
                };
                j(self
                    .with_index(move |ix| {
                        let plan = crate::convert::plan(ix, &lib, &o)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            "formats" => j(crate::convert::formats(&self.library()?)),
            "format_set" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                crate::convert::set_preferred(&lib, arg(&args, "platform").map_err(e2s)?, arg(&args, "format").map_err(e2s)?).map_err(e2s)?;
                j(crate::convert::formats(&lib))
            }
            "convert_on_import_set" => {
                // D177: games imported are converted to their system's preferred format
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let on = args["on"].as_bool().ok_or("missing on")?;
                lib.update_meta(json!({ "convert_on_import": on })).map_err(e2s)?;
                j(lib.info())
            }
            "tools" => j(tokio::task::spawn_blocking(crate::tools::list).await.map_err(|e| e.to_string())?),
            "tools_get" => {
                // Get it… (D178): a pinned build downloaded into the app's tools folder and checked
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                if !crate::tools::TOOLS.iter().any(|t| t.id == id) {
                    return Err(format!("There's no tool called {id}."));
                }
                let label = format!("Getting {id}");
                self.not_running(&label)?;
                let dir = crate::tools::app_dir().ok_or("Game Repo can't tell where its tools folder is.")?;
                j(self.spawn_job(&label, move |app, jid, cancel| {
                    let r = crate::tools::get(&id, &dir, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    // the one downloaded is used from now on, not a path set by hand
                    let mut cfg = app.config();
                    if cfg.tools.remove(&id).is_some() {
                        cfg.save(&app.config_path())?;
                        crate::tools::configure(&cfg.tools);
                    }
                    Ok(r)
                }))
            }
            "tools_set" => {
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                if !crate::tools::TOOLS.iter().any(|t| t.id == id) {
                    return Err(format!("There's no tool called {id}."));
                }
                let mut cfg = self.config();
                match args["path"].as_str().map(str::trim).filter(|p| !p.is_empty()) {
                    Some(p) => {
                        let p = PathBuf::from(p);
                        if !p.is_file() {
                            return Err(format!("There's no file at {}.", p.display()));
                        }
                        cfg.tools.insert(id, p);
                    }
                    None => {
                        cfg.tools.remove(&id);
                    }
                }
                cfg.save(&self.config_path()).map_err(e2s)?;
                crate::tools::configure(&cfg.tools);
                j(tokio::task::spawn_blocking(crate::tools::list).await.map_err(|e| e.to_string())?)
            }
            "organise_rename_folders" => {
                // game folders to their DAT's new names (D45)
                let lib = self.library()?;
                j(self
                    .with_index(move |ix| {
                        crate::meta::sync(ix, &lib)?;
                        let plan = crate::meta::rename_folders_plan(ix, &lib)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            "organise_duplicates" => {
                let lib = self.library()?;
                let dat_id = args["dat"].as_i64().ok_or("missing dat")?;
                j(self
                    .with_index(move |ix| {
                        let plan = organise::plan_duplicates(ix, &lib, dat_id)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            "organise_preview" => {
                let id = arg(&args, "id").map_err(e2s)?;
                let plan = organise::load_pending(&data_dir, id).map_err(e2s)?;
                let what = args["what"].as_str().unwrap_or("all");
                let (offset, limit) = (args["offset"].as_u64().unwrap_or(0) as usize, args["limit"].as_u64().unwrap_or(100) as usize);
                j(organise::preview(&plan, what, offset, limit))
            }
            "organise_run" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?;
                let plan = organise::load_pending(&data_dir, id).map_err(e2s)?;
                let label = match plan.kind.as_str() {
                    "aside" => "Setting aside duplicates",
                    "convert" => "Converting",
                    "extract" => "Extracting",
                    "export" => "Exporting",
                    "import" => "Importing",
                    "add" => "Adding",
                    "bring" => "Importing games",
                    "docs" => "Importing documents",
                    "magazines" => "Importing magazines",
                    "videos" => "Importing videos",
                    "cheats" => "Bringing in cheats",
                    "patches" => "Bringing in patches",
                    "patch" => "Applying patches",
                    "video_remove" => "Removing videos",
                    "magazine_move" => "Moving magazine issues",
                    "bios" => "Importing BIOS files",
                    "metadata" => "Deleting metadata",
                    "pc_remove" => "Removing a PC game",
                    _ => "Organising",
                };
                let running = self.begin_change(&plan.id)?;
                let convert = args["convert"].as_bool().unwrap_or(false);
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let mut r = organise::run(&ix, &lib, &plan, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    let _ = std::fs::remove_file(organise::pending_path(&data_dir, &plan.id));
                    match plan.kind.as_str() {
                        // the new games' details kept as their source's, and the artwork it offered (D55)
                        "import" => {
                            app.job_progress(&jid, json!({ "text": "Getting artwork" }));
                            let games: Vec<PathBuf> = plan.ops.iter().filter(|o| o.op == "text").filter_map(|o| Path::new(&o.to).parent().map(Path::to_path_buf)).collect();
                            r["art"] = json!(crate::pcmeta::settle_imports(&ix, &lib, &games, &cancel));
                        }
                        // the games brought in are scanned, so the full set has them; then, when
                        // asked, converted to their systems' preferred formats (D177)
                        "bring" => {
                            let only: Vec<PathBuf> = plan.ops.iter().map(|o| PathBuf::from(&o.to)).filter(|p| p.exists()).collect();
                            if !only.is_empty() {
                                let opts = scan::Options { only: Some(only), ..Default::default() };
                                scan::scan(&ix, &lib, &opts, &cancel, &mut |p| app.job_progress(&jid, p))?;
                            }
                            if convert && !plan.brought.is_empty() && !cancel.load(Ordering::Relaxed) {
                                r["converted"] = crate::convert::after_import(&ix, &lib, &plan.brought, &cancel, &mut |p| app.job_progress(&jid, p));
                            }
                        }
                        // what the added files are, once scanned
                        "add" => {
                            let only: Vec<PathBuf> = plan.ops.iter().map(|o| PathBuf::from(&o.to)).filter(|p| p.exists()).collect();
                            if !only.is_empty() {
                                let opts = scan::Options { only: Some(only), ..Default::default() };
                                scan::scan(&ix, &lib, &opts, &cancel, &mut |p| app.job_progress(&jid, p))?;
                            }
                            r["added"] = crate::addgame::results(&ix, &lib, &plan)?;
                        }
                        // the documents (or magazines) brought in, listed and their text read for search
                        "docs" | "magazines" => {
                            crate::meta::sync(&ix, &lib)?;
                            app.job_progress(&jid, json!({ "text": "Reading the documents" }));
                            crate::docs::read_texts(&ix, &lib, &cancel, &mut |p| app.job_progress(&jid, p))?;
                        }
                        _ => {}
                    }
                    Ok(r)
                }))
            }
            "change_items" => {
                // what a change did, a page at a time (History's groups, D78)
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let (offset, limit) = (args["offset"].as_u64().unwrap_or(0) as usize, args["limit"].as_u64().unwrap_or(100) as usize);
                j(organise::preview(&organise::load(&lib, &id).map_err(e2s)?, "all", offset, limit))
            }
            "change_history" => {
                let lib = self.library()?;
                let running: Vec<String> = lock(&self.change).iter().cloned().collect();
                j(tokio::task::spawn_blocking(move || organise::history(&lib, &running)).await.map_err(|e| e.to_string())?.map_err(e2s)?)
            }
            "change_undo" | "change_finish" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let running = self.begin_change(&id)?;
                let undo = cmd == "change_undo";
                j(self.spawn_job(if undo { "Undoing" } else { "Finishing a change" }, move |app, jid, cancel| {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let progress = &mut |p| app.job_progress(&jid, p);
                    let r = if undo { organise::undo(&ix, &lib, &id, &cancel, progress) } else { organise::finish(&ix, &lib, &id, &cancel, progress) };
                    // a change may have brought in documents or a game's folder
                    crate::meta::sync(&ix, &lib)?;
                    r
                }))
            }
            "change_aside" => {
                // what a change set aside, or every change's (D168)
                let lib = self.library()?;
                let id = args["id"].as_str().map(String::from);
                j(tokio::task::spawn_blocking(move || match id {
                    Some(id) => organise::aside_files(&lib, &id).map(|v| json!(v)),
                    None => organise::aside_all(&lib),
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(e2s)?)
            }
            "change_aside_delete" => {
                // some of what a change set aside, deleted for good (D168)
                let lib = self.library()?;
                if lock(&self.change).is_some() {
                    return Err("A change is running: wait for it to finish first.".into());
                }
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let held: Vec<String> = args["held"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
                j(tokio::task::spawn_blocking(move || organise::delete_set_aside(&lib, &id, &held)).await.map_err(|e| e.to_string())?.map_err(e2s)?)
            }
            "change_empty" => {
                // set-aside files of one change (or all), deleted for good
                let lib = self.library()?;
                if lock(&self.change).is_some() {
                    return Err("A change is running: wait for it to finish first.".into());
                }
                let id = args["id"].as_str().map(String::from);
                j(tokio::task::spawn_blocking(move || organise::empty_set_aside(&lib, id.as_deref())).await.map_err(|e| e.to_string())?.map_err(e2s)?)
            }
            "naming" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?;
                let n = organise::naming(&lib, platform);
                let mut v = organise::example(&n.template).map_err(e2s)?;
                v["template"] = json!(n.template);
                v["folder"] = json!(n.folder);
                v["default"] = json!(organise::DEFAULT_TEMPLATE);
                j(v)
            }
            "naming_example" => j(organise::example(arg(&args, "template").map_err(e2s)?).map_err(e2s)?),
            "naming_set" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let platform = arg(&args, "platform").map_err(e2s)?;
                let template = args["template"].as_str().unwrap_or(organise::DEFAULT_TEMPLATE);
                organise::set_naming(&lib, platform, template, args["folder"].as_bool().unwrap_or(false)).map_err(e2s)?;
                j(Value::Null)
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// PC games, Identify and its sources, Steam and Add a game… (docs/PLAN.md, "Phase 4 design").
    /// The Metadata page, the scrape and documents (Phase 5).
    async fn meta_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        use crate::{docs, meta, scrape};
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        let cfg = self.config();
        let id_arg = |k: &str| args[k].as_i64().ok_or_else(|| format!("missing {k}"));
        match cmd {
            "meta_list" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let show = args["show"].as_str().unwrap_or("have").to_string();
                let search = args["search"].as_str().unwrap_or("").to_string();
                let (offset, limit) = (args["offset"].as_i64().unwrap_or(0), args["limit"].as_i64().unwrap_or(100));
                j(self
                    .with_index(move |ix| {
                        if offset == 0 {
                            meta::sync(ix, &lib)?;
                        }
                        meta::list(ix, &lib, &platform, &show, &search, offset, limit)
                    })
                    .await?)
            }
            "meta_game" => {
                let lib = self.library()?;
                let id = id_arg("id")?;
                j(self.with_index(move |ix| meta::game(ix, &lib, id)).await?)
            }
            "meta_edit" => {
                let lib = self.library()?;
                self.editable()?;
                let id = id_arg("id")?;
                let field = arg(&args, "field").map_err(e2s)?.to_string();
                let value = args.get("value").cloned();
                j(self.with_index(move |ix| meta::edit(ix, &lib, id, &field, value.as_ref())).await?)
            }
            // the Library page (Phase 6): every game, a page per game
            "browse" => {
                let lib = self.library()?;
                j(self
                    .with_index(move |ix| {
                        meta::sync(ix, &lib)?;
                        let rebuilt = crate::browse::refresh(ix, &lib)?;
                        let mut v = crate::browse::overview(ix, &lib)?;
                        v["rebuilt"] = json!(rebuilt);
                        Ok(v)
                    })
                    .await?)
            }
            "browse_query" => {
                let lib = self.library()?;
                let q: crate::browse::Query = serde_json::from_value(args["query"].clone()).map_err(|e| e.to_string())?;
                j(self.with_index(move |ix| crate::browse::query(ix, &lib, &q)).await?)
            }
            "browse_keys" => {
                let lib = self.library()?;
                let q: crate::browse::Query = serde_json::from_value(args["query"].clone()).map_err(|e| e.to_string())?;
                j(self.with_index(move |ix| crate::browse::keys(ix, &lib, &q)).await?)
            }
            "browse_game" => {
                let lib = self.library()?;
                let key = arg(&args, "key").map_err(e2s)?.to_string();
                j(self.with_index(move |ix| crate::browse::game(ix, &lib, &key)).await?)
            }
            "addon_link" => {
                // an update, DLC or patch linked to another game by hand ("base": null, to none),
                // or ("clear": true) back to the game its name gives it (D66)
                let lib = self.library()?;
                self.editable()?;
                let (platform, addon) = (arg(&args, "platform").map_err(e2s)?.to_string(), arg(&args, "addon").map_err(e2s)?.to_string());
                let base = args["base"].as_str().map(String::from);
                let clear = args["clear"].as_bool().unwrap_or(false);
                // and the system's rows made again, so an add-on's own row and page show its game (D163)
                self.with_index(move |ix| {
                    crate::addons::set(ix.conn(), &lib, &platform, &addon, base.as_deref(), clear)?;
                    crate::browse::refresh(ix, &lib)
                })
                .await?;
                j(json!({ "ok": true }))
            }
            "browse_edit" => {
                let lib = self.library()?;
                self.editable()?;
                let (key, field) = (arg(&args, "key").map_err(e2s)?.to_string(), arg(&args, "field").map_err(e2s)?.to_string());
                // "value": null clears it; no "value" takes the edit mark off
                let value = args.get("value").cloned();
                j(self.with_index(move |ix| crate::browse::edit(ix, &lib, &key, &field, value.as_ref())).await?)
            }
            "meta_clear" => {
                let lib = self.library()?;
                self.editable()?;
                let id = id_arg("id")?;
                j(self.with_index(move |ix| meta::clear(ix, &lib, id)).await?)
            }
            // each source's answer, its pictures, deleting and sizes (Phase 6b, D46–D48)
            "meta_pin" => {
                let lib = self.library()?;
                self.editable()?;
                let (folder, field) = (arg(&args, "folder").map_err(e2s)?.to_string(), arg(&args, "field").map_err(e2s)?.to_string());
                let source = args["source"].as_str().map(String::from);
                j(self
                    .with_index(move |ix| {
                        crate::sources::pin(ix, &lib, &meta::resolve(&lib, &folder)?, &field, source.as_deref())?;
                        crate::browse::refresh(ix, &lib)?;
                        Ok(json!({ "ok": true }))
                    })
                    .await?)
            }
            "meta_choose" => {
                let lib = self.library()?;
                self.editable()?;
                let (folder, kind, name) = (arg(&args, "folder").map_err(e2s)?.to_string(), arg(&args, "kind").map_err(e2s)?.to_string(), arg(&args, "name").map_err(e2s)?.to_string());
                j(self
                    .with_index(move |ix| {
                        crate::sources::choose(ix, &lib, &meta::resolve(&lib, &folder)?, &kind, &name)?;
                        crate::browse::refresh(ix, &lib)?;
                        Ok(json!({ "ok": true }))
                    })
                    .await?)
            }
            "meta_delete" => {
                // a change setting aside a system's or a game's metadata, with a preview and Undo
                let lib = self.library()?;
                let platform = args["platform"].as_str().filter(|s| !s.is_empty()).map(String::from);
                let folder = args["folder"].as_str().filter(|s| !s.is_empty()).map(String::from);
                let what: Vec<String> = serde_json::from_value(args["what"].clone()).map_err(|e| e.to_string())?;
                // planned on a connection of its own: it can read every game's folder (M28)
                j(tokio::task::spawn_blocking(move || {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    meta::sync(&ix, &lib)?;
                    let plan = crate::sources::delete_plan(&ix, &lib, platform.as_deref(), folder.as_deref(), &what)?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok::<_, anyhow::Error>(plan.summary())
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(e2s)?)
            }
            "meta_sizes" => {
                let lib = self.library()?;
                j(tokio::task::spawn_blocking(move || crate::sources::sizes(&lib)).await.map_err(|e| e.to_string())?)
            }
            "meta_details_from" => {
                let lib = self.library()?;
                let fields: Vec<Value> = [("title", "Title"), ("year", "Year")].into_iter().chain(meta::FIELDS.iter().copied()).map(|(k, n)| json!({ "id": k, "name": n })).collect();
                j(json!({
                    "from": crate::sources::details_from(&lib),
                    "fields": fields,
                    "sources": scrape::SCRAPE_SOURCES.iter().map(|s| json!({ "id": s, "name": crate::identify::source_name(s) })).collect::<Vec<_>>(),
                }))
            }
            "meta_set_details_from" => {
                // every game's details made again from the answers kept
                let lib = self.library()?;
                let field = arg(&args, "field").map_err(e2s)?.to_string();
                let label = "Making details again";
                let running = self.begin_meta(label)?;
                crate::sources::set_details_from(&lib, &field, args["source"].as_str()).map_err(e2s)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    meta::sync(&ix, &lib)?;
                    let n = crate::sources::remake_all(&ix, &lib, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    crate::browse::refresh(&ix, &lib)?;
                    Ok(json!({ "games": n }))
                }))
            }
            "meta_pick" => {
                // fetched on a connection of its own, so the pages aren't held up while the
                // pictures come (M28)
                let lib = self.library()?;
                let game = id_arg("game")?;
                let (source, id) = (arg(&args, "source").map_err(e2s)?.to_string(), arg(&args, "id").map_err(e2s)?.to_string());
                let running = self.begin_meta("Identify")?;
                j(tokio::task::spawn_blocking(move || {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    scrape::pick(&ix, &lib, &cfg, &data_dir, game, &source, &id)
                })
                .await
                .map_err(|e| e.to_string())?
                .map_err(e2s)?)
            }
            "meta_games" => {
                // DAT games by name, to pair a document with by hand
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let q = args["query"].as_str().unwrap_or("").to_string();
                j(self.with_index(move |ix| meta::find_games(ix, &lib, &platform, &q, 30)).await?)
            }
            "scrape_choices" => {
                let lib = self.library()?;
                j(scrape::choices(&cfg, &data_dir, &lib))
            }
            "scrape_run" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let o: scrape::Options = serde_json::from_value(args["options"].clone()).map_err(|e| e.to_string())?;
                let label = "Scraping";
                let running = self.begin_meta(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let r = scrape::run(&ix, &lib, &cfg, &data_dir, &o, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    // the manuals it kept, read for searching
                    docs::read_texts(&ix, &lib, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    Ok(r)
                }))
            }
            "docs_list" => {
                // D155: the filters as lists, the sort and group, and their counts (a kind,
                // system or source on its own as before)
                let mut q: docs::DocsQuery = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
                for (one, list) in [("kind", &mut q.kinds), ("platform", &mut q.platforms), ("source", &mut q.sources)] {
                    if let Some(v) = args[one].as_str().filter(|v| !v.is_empty()) {
                        list.push(v.to_string());
                    }
                }
                if args["limit"].is_null() {
                    q.limit = 100;
                }
                let lib = self.library()?;
                j(self
                    .with_index(move |ix| {
                        if q.offset == 0 {
                            meta::sync(ix, &lib)?;
                        }
                        docs::query(ix, &q)
                    })
                    .await?)
            }
            "docs_read" => {
                let lib = self.library()?;
                let label = "Reading documents";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let n = docs::read_texts(&ix, &lib, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    Ok(json!({ "read": n }))
                }))
            }
            "docs_get" => {
                let lib = self.library()?;
                let id = id_arg("id")?;
                j(self
                    .with_index(move |ix| {
                        let mut d = docs::get(ix, id)?;
                        let p = lib.resolve(d["path"].as_str().unwrap_or(""))?;
                        if matches!(d["view"].as_str(), Some("text" | "markdown")) {
                            d["html"] = json!(docs::to_html(d["name"].as_str().unwrap_or(""), &std::fs::read(&p)?));
                        }
                        d["full"] = json!(p.display().to_string());
                        Ok(d)
                    })
                    .await?)
            }
            "docs_set" => {
                let lib = self.library()?;
                let id = id_arg("id")?;
                let (kind, title) = (args["kind"].as_str().map(String::from), args["title"].as_str().map(String::from));
                j(self.with_index(move |ix| docs::set(ix, &lib, id, kind.as_deref(), title.as_deref())).await?)
            }
            "docs_items" => {
                // the files to attach or import, each paired with a game (a job: a folder can be big)
                let lib = self.library()?;
                let paths: Vec<PathBuf> = args["paths"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect();
                if paths.is_empty() {
                    return Err("Choose the documents or a folder of them.".into());
                }
                let platform = args["platform"].as_str().filter(|p| !p.is_empty()).map(String::from);
                let game = args["game"].as_i64();
                // a game's folder instead of a DAT game (a PC game's, D55), with its title
                let folder = args["folder"].as_str().filter(|f| !f.is_empty()).map(|f| (f.to_string(), args["game_name"].as_str().unwrap_or("").to_string()));
                let label = "Reading the documents";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    meta::sync(&ix, &lib)?;
                    let mut items = docs::read_items(&ix, &lib, &paths, platform.as_deref(), game, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    if let Some((f, name)) = &folder {
                        docs::pair_with_folder(&lib, &mut items, f, name)?;
                    }
                    Ok(json!({ "items": items, "kinds": docs::KINDS.iter().map(|(id, name, _)| json!({ "id": id, "name": name })).collect::<Vec<_>>() }))
                }))
            }
            "docs_plan" => {
                let lib = self.library()?;
                let items: Vec<docs::Item> = serde_json::from_value(args["items"].clone()).map_err(|e| e.to_string())?;
                let mode = args["mode"].as_str().unwrap_or("copy").to_string();
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(self
                    .with_index(move |ix| {
                        let plan = docs::plan(ix, &lib, &items, &mode, always_copy)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            _ => Err(format!("unknown command {cmd}")),
        }
    }

    async fn pc_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        use crate::{addgame, identify, launchbox, pcgames, steam};
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        let cfg = self.config();
        let blocking = |f: Box<dyn FnOnce() -> Result<Value> + Send>| async move { tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(e2s) };
        let path_arg = |k: &str| args[k].as_str().map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from);
        match cmd {
            "pc_games" => {
                let lib = self.library()?;
                let inbox = cfg.pc_inbox.clone();
                j(blocking(Box::new(move || {
                    let games: Vec<Value> = pcgames::list(&lib).iter().map(|e| e.to_json(&lib)).collect();
                    let waiting = inbox.as_ref().and_then(|d| pcgames::inbox_paths(d).ok()).map(|v| v.len());
                    Ok(json!({ "folder": pcgames::folder(&lib).display().to_string(), "inbox": inbox, "inbox_waiting": waiting, "games": games }))
                }))
                .await?)
            }
            "pc_get" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || {
                    let e = pcgames::get(&lib, &id)?;
                    let mut v = e.to_json(&lib);
                    v["programs"] = json!(pcgames::programs(&lib, &id).unwrap_or_default());
                    v["innoextract"] = json!(crate::tools::find("innoextract").is_some());
                    Ok(v)
                }))
                .await?)
            }
            "pc_update" => {
                let lib = self.library()?;
                self.editable()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let patch = args["patch"].clone();
                j(blocking(Box::new(move || Ok(pcgames::update(&lib, &id, &patch)?.to_json(&lib)))).await?)
            }
            "pc_folder_set" => {
                let lib = self.library()?;
                let p = path_arg("path");
                j(json!(pcgames::set_folder(&lib, p.as_deref()).map_err(e2s)?.display().to_string()))
            }
            "pc_inbox_set" => {
                let mut cfg = cfg;
                let p = path_arg("path");
                if let Some(d) = &p {
                    if !d.is_dir() {
                        return Err(format!("There's no folder at {}.", d.display()));
                    }
                }
                cfg.pc_inbox = p;
                cfg.save(&self.config_path()).map_err(e2s)?;
                j(json!(cfg.pc_inbox))
            }
            "pc_read" => {
                let lib = self.library()?;
                let paths: Vec<PathBuf> = match &args["paths"] {
                    Value::Array(a) if !a.is_empty() => a.iter().filter_map(Value::as_str).map(PathBuf::from).collect(),
                    _ => {
                        let d = cfg.pc_inbox.clone().ok_or("Choose an inbox folder in Settings › PC games first, or pick what to import.")?;
                        pcgames::inbox_paths(&d).map_err(e2s)?
                    }
                };
                let look = args["search"].as_bool().unwrap_or(true);
                self.not_running("Reading the inbox")?;
                j(self.spawn_job("Reading the inbox", move |app, jid, cancel| {
                    let search = |title: &str, year: Option<i32>| if look { identify::best(&cfg, &data_dir, title, year) } else { None };
                    let items = pcgames::read_items(&lib, &paths, &search, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    Ok(json!({ "items": items }))
                }))
            }
            "pc_item_match" => {
                // a source's answer put into an inbox item (Identify on the review list)
                let mut it: pcgames::Item = serde_json::from_value(args["item"].clone()).map_err(|e| e.to_string())?;
                let (source, id) = (arg(&args, "source").map_err(e2s)?.to_string(), arg(&args, "id").map_err(e2s)?.to_string());
                j(blocking(Box::new(move || {
                    let m = identify::details(&cfg, &data_dir, &source, &id)?;
                    pcgames::apply_match(&mut it, &m);
                    it.found = "you".into();
                    if it.status == "look" || it.status == "own" {
                        it.status = "ready".into();
                    }
                    it.include = true;
                    Ok(json!(it))
                }))
                .await?)
            }
            "pc_item_unmatch" => {
                // No match on the review list (D128): a game of its own, matched to no source
                let mut it: pcgames::Item = serde_json::from_value(args["item"].clone()).map_err(|e| e.to_string())?;
                pcgames::unmatch(&mut it);
                j(json!(it))
            }
            "pc_unmatch" => {
                // None of these (Identify…) for a game in the library (D128)
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let running = self.begin_meta("Identify")?;
                j(blocking(Box::new(move || {
                    let _running = running;
                    Ok(crate::pcmeta::unmatch(&Self::job_index(&data_dir, &lib)?, &lib, &id)?.to_json(&lib))
                }))
                .await?)
            }
            "pc_find_details" => {
                // Find details… (D138): the PC games matched to no source, asked of the sources
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let label = "Finding details";
                let running = self.begin_meta(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    crate::pcmeta::find_details(&ix, &lib, &cfg, &data_dir, &cancel, &mut |p| app.job_progress(&jid, p))
                }))
            }
            "pc_remove_plan" => {
                // Remove from the library… (D129): the change's preview, and whether Add to
                // Steam… wrote its shortcut, which stays until it's run again
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || {
                    let plan = pcgames::remove_plan(&lib, &id)?;
                    organise::save_pending(&data_dir, &plan)?;
                    let mut s = plan.summary();
                    s["steam"] = json!(steam::added(&lib, &data_dir, &id));
                    Ok(s)
                }))
                .await?)
            }
            "pc_import_plan" => {
                let lib = self.library()?;
                let items: Vec<pcgames::Item> = serde_json::from_value(args["items"].clone()).map_err(|e| e.to_string())?;
                let mode = args["mode"].as_str().unwrap_or("copy").to_string();
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(blocking(Box::new(move || {
                    let plan = pcgames::plan_import(&lib, &items, &mode, always_copy)?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
                .await?)
            }
            "pc_programs" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || Ok(json!(pcgames::programs(&lib, &id)?)))).await?)
            }
            "pc_unpack" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let file = args["file"].as_str().map(String::from);
                self.not_running("Unpacking")?;
                j(self.spawn_job("Unpacking", move |app, jid, cancel| pcgames::unpack(&lib, &id, file.as_deref(), &cancel, &mut |p| app.job_progress(&jid, p))))
            }
            "pc_unpack_remove" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || {
                    pcgames::remove_unpacked(&lib, &id)?;
                    Ok(pcgames::get(&lib, &id)?.to_json(&lib))
                }))
                .await?)
            }
            "identify_search" => {
                let query = arg(&args, "query").map_err(e2s)?.to_string();
                let platform = args["platform"].as_str().unwrap_or("pc").to_string();
                let year = args["year"].as_i64().map(|y| y as i32);
                let only = args["source"].as_str().filter(|s| !s.is_empty()).map(String::from);
                j(blocking(Box::new(move || Ok(identify::search(&cfg, &data_dir, &query, &platform, year, only.as_deref())))).await?)
            }
            "identify_details" => {
                let (source, id) = (arg(&args, "source").map_err(e2s)?.to_string(), arg(&args, "id").map_err(e2s)?.to_string());
                j(blocking(Box::new(move || Ok(identify::match_json(&identify::details(&cfg, &data_dir, &source, &id)?)))).await?)
            }
            "identify_pick" => {
                // a PC game (`game`) or a file no DAT knows (`platform` and `file`, from Add's results)
                let lib = self.library()?;
                let (source, id) = (arg(&args, "source").map_err(e2s)?.to_string(), arg(&args, "id").map_err(e2s)?.to_string());
                let retitle = args["retitle"].as_bool().unwrap_or(true);
                let (game, platform, file) = (args["game"].as_str().map(String::from), args["platform"].as_str().map(String::from), args["file"].clone());
                // it can rename the game's folder: no change or scrape beside it (H17); and
                // fetched on a connection of its own, so the pages aren't held up (M28)
                let running = self.begin_meta("Identify")?;
                j(blocking(Box::new(move || {
                    let _running = running;
                    let m = identify::details(&cfg, &data_dir, &source, &id)?;
                    match (game, platform) {
                        (Some(g), _) => Ok(crate::pcmeta::identify(&Self::job_index(&data_dir, &lib)?, &lib, &g, &m, retitle)?.to_json(&lib)),
                        (None, Some(p)) => addgame::identify_rom(&lib, &p, &file, &m),
                        _ => bail!("missing game"),
                    }
                }))
                .await?)
            }
            "sources" => j(identify::sources(&cfg, &data_dir)),
            "sources_set" => {
                let mut cfg = cfg;
                cfg.sources = args["sources"]
                    .as_array()
                    .ok_or("missing sources")?
                    .iter()
                    .filter_map(|s| Some(crate::config::SourceSetting { id: s["id"].as_str()?.to_string(), on: s["on"].as_bool().unwrap_or(true) }))
                    .collect();
                // chosen now, so kept as it is (D138)
                cfg.sources_rev = 1;
                cfg.save(&self.config_path()).map_err(e2s)?;
                j(identify::sources(&cfg, &data_dir))
            }
            "sources_keys_set" => {
                // a source's account or key (blank fields keep what's set; `clear` forgets them)
                let source = arg(&args, "source").map_err(e2s)?.to_string();
                let Some((_, fields)) = crate::scrapers::KEYS.iter().find(|(s, _)| *s == source) else { return Err(format!("{source} needs no key.")) };
                let mut cfg = cfg;
                let keys = cfg.source_keys.entry(source.clone()).or_default();
                if args["clear"] == true {
                    keys.clear();
                }
                for (f, _, _) in fields.iter() {
                    if let Some(v) = args["keys"][*f].as_str().map(str::trim).filter(|v| !v.is_empty()) {
                        if *f == "threads" && !v.parse::<usize>().is_ok_and(|n| (1..=16).contains(&n)) {
                            return Err("Threads at once is a number from 1 to 16 (left empty, it's as many as your account allows).".into());
                        }
                        keys.insert(f.to_string(), v.to_string());
                    }
                }
                // a field put back as it was (ScreenScraper's threads: the account's again)
                for f in args["unset"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    keys.remove(f);
                }
                if keys.is_empty() {
                    cfg.source_keys.remove(&source);
                }
                cfg.save(&self.config_path()).map_err(e2s)?;
                j(identify::sources(&cfg, &data_dir))
            }
            "sources_check" => {
                let source = arg(&args, "source").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || Ok(json!(crate::scrapers::check(&cfg, &data_dir, &source)?)))).await?)
            }
            "launchbox_fetch" => {
                let label = "Getting LaunchBox's database";
                self.not_running(label)?;
                let url = identify::url(&cfg, "launchbox_zip");
                j(self.spawn_job(label, move |app, jid, cancel| launchbox::fetch(&data_dir, &url, &cancel, &mut |p| app.job_progress(&jid, p))))
            }
            "steam_info" => {
                let set = cfg.steam_dir.clone();
                j(blocking(Box::new(move || {
                    let found = steam::find(set.as_deref());
                    let users: Vec<Value> = found
                        .as_ref()
                        .map(|s| steam::users(s))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|mut u| {
                            u["last_written"] = steam::last_written(&data_dir, u["id"].as_str().unwrap_or(""));
                            u
                        })
                        .collect();
                    Ok(json!({ "steam": found, "set": set, "running": steam::running(), "users": users }))
                }))
                .await?)
            }
            "steam_dir_set" => {
                let mut cfg = cfg;
                let p = path_arg("path");
                if let Some(d) = &p {
                    if steam::find(Some(d)).as_deref() != Some(d.as_path()) {
                        return Err(format!("{} doesn't look like Steam's folder (it has no userdata folder).", d.display()));
                    }
                }
                cfg.steam_dir = p;
                cfg.save(&self.config_path()).map_err(e2s)?;
                j(json!(cfg.steam_dir))
            }
            "steam_preview" | "steam_write" | "steam_remove" => {
                let lib = self.library()?;
                let user = arg(&args, "user").map_err(e2s)?.to_string();
                let steam_dir = steam::find(cfg.steam_dir.as_deref()).ok_or("Steam isn't on this computer (or choose its folder in Settings › Steam).")?;
                let cmd = cmd.to_string();
                j(blocking(Box::new(move || match cmd.as_str() {
                    "steam_preview" => steam::preview(&lib, &steam_dir, &user),
                    "steam_write" => steam::write_shortcuts(&lib, &data_dir, &steam_dir, &user, steam::running()),
                    _ => steam::remove_shortcuts(&data_dir, &steam_dir, &user, steam::running()),
                }))
                .await?)
            }
            "add_places" => {
                let lib = self.library()?;
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                j(self.with_index(move |ix| Ok(json!(addgame::places(Some(ix.conn()), &lib, &platform)))).await?)
            }
            "add_plan" => {
                let lib = self.library()?;
                let files: Vec<PathBuf> = args["files"].as_array().ok_or("missing files")?.iter().filter_map(Value::as_str).map(PathBuf::from).collect();
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let into = path_arg("into").ok_or("missing into")?;
                let mode = args["mode"].as_str().unwrap_or("copy").to_string();
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(blocking(Box::new(move || {
                    let plan = addgame::plan(&lib, &files, &platform, &into, &mode, always_copy)?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
                .await?)
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// Export layouts and their editor, and BIOS files (docs/PLAN.md, "Phase 5b design").
    /// The Cheats page (D197–D199, cheats.rs).
    async fn cheat_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        use crate::cheats;
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
        let strings = |k: &str| -> Vec<String> { args[k].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect() };
        let lib = self.library()?;
        match cmd {
            "cheats_list" => {
                let show = args["show"].as_str().unwrap_or("with").to_string();
                j(self
                    .with_index(move |ix| {
                        let mut v = cheats::list(ix, &lib, &show)?;
                        v["pack"] = cheats::pack_info(Some(&data_dir));
                        Ok(v)
                    })
                    .await?)
            }
            "cheats_game" => {
                let key = s("key");
                j(self
                    .with_index(move |ix| {
                        cheats::sync(ix, &lib)?;
                        let mut v = cheats::game(ix, &lib, &data_dir, &key)?;
                        v["removed"] = json!(cheats::removed(ix, &lib, &key)?);
                        Ok(v)
                    })
                    .await?)
            }
            "cheats_save" => {
                let (key, name) = (s("key"), s("name"));
                let cht: Option<cheats::Cht> = if args["cht"].is_object() { Some(serde_json::from_value(args["cht"].clone()).map_err(|e| e.to_string())?) } else { None };
                let text = args["text"].as_str().map(String::from);
                j(self.with_index(move |ix| cheats::save(ix, &lib, &data_dir, &key, &name, cht.as_ref(), text.as_deref())).await?)
            }
            "cheats_new" => {
                let (key, format) = (s("key"), s("format"));
                let name = args["name"].as_str().map(String::from);
                j(self.with_index(move |ix| cheats::create(ix, &lib, &data_dir, &key, &format, name.as_deref())).await?)
            }
            "cheats_remove" => {
                let (key, name) = (s("key"), s("name"));
                j(self.with_index(move |ix| cheats::remove(ix, &lib, &data_dir, &key, &name)).await?)
            }
            "cheats_versions" => {
                let (key, name) = (s("key"), s("name"));
                j(self.with_index(move |ix| cheats::versions(ix, &lib, &key, &name)).await?)
            }
            "cheats_restore" => {
                let (key, name, when) = (s("key"), s("name"), s("when"));
                j(self.with_index(move |ix| cheats::restore(ix, &lib, &data_dir, &key, &name, &when)).await?)
            }
            "cheats_newer" => {
                let (key, name) = (s("key"), s("name"));
                j(self.with_index(move |ix| cheats::take_newer(ix, &lib, &data_dir, &key, &name)).await?)
            }
            "cheats_get" => {
                // libretro's pack fetched (or the one kept), matched to the games you have (a job)
                let (platforms, keys) = (strings("platforms"), strings("keys"));
                let again = args["again"].as_bool().unwrap_or(false);
                let src = match self.config().cheats_source {
                    Some(base) => {
                        let b = base.trim_end_matches('/').to_string();
                        cheats::Sources { zip: format!("{b}/cheats.zip"), api: format!("{b}/api"), raw: format!("{b}/raw/") }
                    }
                    None => cheats::Sources::default(),
                };
                let label = "Getting cheats";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let wanted = cheats::wanted(&ix, &lib, &platforms, &keys)?;
                    let pack = cheats::get_pack(&data_dir, &src, again, &wanted, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    app.job_progress(&jid, json!({ "text": "Matching them to your games" }));
                    cheats::find(&ix, &lib, &pack, &platforms, &keys)
                }))
            }
            "cheats_files" => {
                // cheat files picked (or a folder's), paired with games by their names (a job)
                let paths: Vec<PathBuf> = strings("paths").into_iter().map(PathBuf::from).collect();
                if paths.is_empty() {
                    return Err("Choose the cheat files or a folder of them.".into());
                }
                let key = args["key"].as_str().filter(|k| !k.is_empty()).map(String::from);
                let platform = args["platform"].as_str().filter(|p| !p.is_empty()).map(String::from);
                let label = "Reading the cheat files";
                self.not_running(label)?;
                j(self.spawn_job(label, move |_app, _jid, _cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    Ok(json!({ "found": cheats::read_files(&ix, &lib, &paths, key.as_deref(), platform.as_deref())? }))
                }))
            }
            "cheats_plan" => {
                let found: Vec<cheats::Found> = serde_json::from_value(args["found"].clone()).map_err(|e| e.to_string())?;
                j(self
                    .with_index(move |ix| {
                        let pack = if found.iter().any(|f| f.include && f.stem.is_some()) { Some(cheats::Pack::open_cached(&data_dir)?) } else { None };
                        let plan = cheats::plan(ix, &lib, pack.as_ref(), &found)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            _ => Err(format!("unknown command {cmd}")),
        }
    }

    /// The Patches page's ROM patches (D201–D203, patches.rs and rompatch.rs).
    async fn patch_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        use crate::patches;
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
        let strings = |k: &str| -> Vec<String> { args[k].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect() };
        let lib = self.library()?;
        match cmd {
            "patches_list" => j(self.with_index(move |ix| patches::list(ix, &lib)).await?),
            "patches_game" => {
                let key = s("key");
                j(self
                    .with_index(move |ix| {
                        patches::sync(ix, &lib)?;
                        patches::game(ix, &lib, &key)
                    })
                    .await?)
            }
            "patches_edit" => {
                let (key, name) = (s("key"), s("name"));
                let d: patches::Details = serde_json::from_value(args["details"].clone()).map_err(|e| e.to_string())?;
                j(self.with_index(move |ix| patches::edit(ix, &lib, &key, &name, &d)).await?)
            }
            "patches_remove" => {
                let (key, name) = (s("key"), s("name"));
                j(self.with_index(move |ix| patches::remove(ix, &lib, &key, &name)).await?)
            }
            "patches_files" => {
                // patch files picked (or a folder's, or ZIPs of them), paired with games (a job)
                let paths: Vec<PathBuf> = strings("paths").into_iter().map(PathBuf::from).collect();
                if paths.is_empty() {
                    return Err("Choose the patch files, ZIPs of them or a folder.".into());
                }
                let key = args["key"].as_str().filter(|k| !k.is_empty()).map(String::from);
                let platform = args["platform"].as_str().filter(|p| !p.is_empty()).map(String::from);
                let label = "Reading the patches";
                self.not_running(label)?;
                j(self.spawn_job(label, move |_app, _jid, _cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    Ok(json!({ "found": patches::read_files(&ix, &lib, &data_dir, &paths, key.as_deref(), platform.as_deref())? }))
                }))
            }
            "patches_test" => {
                let (key, path) = (s("key"), s("path"));
                j(self.with_index(move |ix| patches::test(ix, &lib, &key, Path::new(&path))).await?)
            }
            "patches_plan" => {
                let found: Vec<patches::Found> = serde_json::from_value(args["found"].clone()).map_err(|e| e.to_string())?;
                j(self
                    .with_index(move |ix| {
                        let plan = patches::plan(ix, &lib, &found)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            "patches_apply" => {
                // the patched copy planned (it's made when the change runs)
                let (key, names) = (s("key"), strings("names"));
                let name = args["name"].as_str().map(String::from);
                j(self
                    .with_index(move |ix| {
                        let plan = patches::apply_plan(ix, &lib, &key, &names, name.as_deref())?;
                        organise::save_pending(&data_dir, &plan)?;
                        let mut v = plan.summary();
                        v["variant"] = plan.dat["variant"].clone();
                        Ok(v)
                    })
                    .await?)
            }
            "patches_name" => {
                // the name *Apply* would give the copy, for the dialog
                let (key, names) = (s("key"), strings("names"));
                j(self
                    .with_index(move |ix| {
                        let g = crate::cheats::game_of(ix.conn(), &key)?;
                        let dir = patches::game_dir(ix.conn(), &lib, &g)?.map(|d| d.join(patches::FOLDER));
                        let m = dir.map(|d| crate::config::read_json_object(&d.join(patches::MANIFEST))).unwrap_or(json!({}));
                        let ds: Vec<patches::Details> = names.iter().map(|n| patches::Details::of(&m["files"][n], n)).collect();
                        Ok(json!({ "name": patches::variant_name(&g.name, &ds) }))
                    })
                    .await?)
            }
            _ => Err(format!("unknown command {cmd}")),
        }
    }

    async fn device_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        let blocking = |f: Box<dyn FnOnce() -> Result<Value> + Send>| async move { tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(e2s) };
        let layout_of = |lib: &Library, args: &Value| -> Result<crate::layout::Layout> {
            match &args["layout"] {
                Value::Object(_) => crate::layout::parse(&args["layout"]),
                Value::String(id) => crate::layout::get(Some(lib), id),
                _ => crate::layout::get(Some(lib), args["id"].as_str().unwrap_or("")),
            }
        };
        match cmd {
            "layouts" => {
                let lib = self.library().ok();
                j(json!(crate::layout::list(lib.as_ref())))
            }
            "layout_get" => {
                let lib = self.library()?;
                let l = layout_of(&lib, &args).map_err(e2s)?;
                j(json!({ "layout": crate::layout::to_json(&l), "summary": l.summary(), "problems": crate::layout::check(&l) }))
            }
            "layout_check" => {
                let lib = self.library()?;
                let l = layout_of(&lib, &args).map_err(e2s)?;
                j(json!({ "problems": crate::layout::check(&l), "carries": l.carries() }))
            }
            "layout_save" => {
                let lib = self.library()?;
                let v = args["layout"].clone();
                let as_new = args["as_new"].as_bool().unwrap_or(false);
                j(blocking(Box::new(move || {
                    let l = crate::layout::save(&lib, &v, as_new)?;
                    Ok(json!({ "layout": crate::layout::to_json(&l), "summary": l.summary() }))
                }))
                .await?)
            }
            "layout_remove" => {
                let lib = self.library()?;
                crate::layout::remove(&lib, arg(&args, "id").map_err(e2s)?).map_err(e2s)?;
                j(Value::Null)
            }
            "layout_import" => {
                let lib = self.library()?;
                let path = PathBuf::from(arg(&args, "path").map_err(e2s)?);
                let l = crate::layout::import(&lib, &path).map_err(e2s)?;
                j(l.summary())
            }
            "layout_write" => {
                let lib = self.library()?;
                let to = PathBuf::from(arg(&args, "to").map_err(e2s)?);
                crate::layout::write_file(Some(&lib), arg(&args, "id").map_err(e2s)?, &to).map_err(e2s)?;
                j(json!({ "path": to.display().to_string() }))
            }
            "layout_folder" => {
                let lib = self.library()?;
                let d = crate::layout::folder(&lib);
                if lib.writable().is_ok() {
                    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                }
                j(json!({ "path": d.display().to_string() }))
            }
            "layout_preview" => {
                let lib = self.library()?;
                let l = layout_of(&lib, &args).map_err(e2s)?;
                let collection = args["collection"].as_str().unwrap_or("").to_string();
                let cached = lock(&self.layout_sample).clone();
                let sample = self
                    .with_index(move |ix| {
                        let key = format!("{collection}\n{}", crate::audit::generation(ix.conn())?);
                        if let Some((k, v)) = cached.filter(|(k, _)| *k == key) {
                            return Ok((k, v));
                        }
                        crate::meta::sync(ix, &lib)?;
                        Ok((key, export::sample(ix, &lib, Some(collection.as_str()))?))
                    })
                    .await?;
                *lock(&self.layout_sample) = Some(sample.clone());
                j(export::preview(&l, &sample.1))
            }
            "bios" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| {
                    dat::sync_library(ix, &lib)?;
                    crate::bios::summary(ix, &lib)
                })
                .await?)
            }
            "bios_report" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                j(self.with_index(move |ix| {
                    let text = crate::bios::report(ix, &lib)?;
                    let path = lib.root().join(crate::library::APP_DIR).join("reports").join(format!("BIOS files missing ({}).txt", &crate::library::now()[..10]));
                    crate::config::write_atomic(&path, text.as_bytes())?;
                    Ok(json!({ "path": path.display().to_string(), "rel": lib.relative(&path), "text": text }))
                })
                .await?)
            }
            "bios_gather_plan" => {
                let lib = self.library()?;
                let mode = args["mode"].as_str().unwrap_or("copy").to_string();
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(self
                    .with_index(move |ix| {
                        let plan = crate::bios::gather_plan(ix, &lib, &mode, always_copy)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
            }
            // the Magazines page (D68)
            "magazines" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| {
                    crate::docs::sync(ix, &lib)?;
                    crate::magazines::overview(ix, &lib)
                })
                .await?)
            }
            "magazine_items" => {
                // the issues in the files and folders picked (a job: they're hashed)
                let lib = self.library()?;
                let paths: Vec<PathBuf> = args["paths"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect();
                if paths.is_empty() {
                    return Err("Choose the magazines' files or a folder of them.".into());
                }
                let label = "Reading the magazines";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let items = crate::magazines::read_items(&lib, &paths, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    Ok(json!({ "items": items }))
                }))
            }
            "magazine_plan" => {
                let lib = self.library()?;
                let items: Vec<crate::magazines::Item> = serde_json::from_value(args["items"].clone()).map_err(|e| e.to_string())?;
                let mode = args["mode"].as_str().unwrap_or("copy").to_string();
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(blocking(Box::new(move || {
                    let plan = crate::magazines::plan(&lib, &items, &mode, always_copy)?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
                .await?)
            }
            "magazine_edit" => {
                let lib = self.library()?;
                let title = arg(&args, "title").map_err(e2s)?.to_string();
                let file = args["file"].as_str().map(String::from);
                let patch = args["patch"].clone();
                j(self.with_index(move |ix| {
                    let r = crate::magazines::edit(&lib, &title, file.as_deref(), &patch)?;
                    crate::docs::sync(ix, &lib)?;
                    Ok(r)
                })
                .await?)
            }
            // issues tagged, or a tag taken off (D154); `items`: [{ title, file }]
            "magazine_issues_edit" => {
                let lib = self.library()?;
                let items = issue_refs(&args["items"]);
                let patch = args["patch"].clone();
                j(blocking(Box::new(move || crate::magazines::edit_issues(&lib, &items, &patch))).await?)
            }
            // issues moved into another magazine: planned, for the preview and Undo (D154)
            "magazine_move" => {
                let lib = self.library()?;
                let items = issue_refs(&args["items"]);
                let to = arg(&args, "to").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || {
                    let plan = crate::magazines::move_plan(&lib, &items, &to)?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
                .await?)
            }
            "magazine_cover" => {
                let lib = self.library()?;
                let (title, file, picture) = (arg(&args, "title").map_err(e2s)?.to_string(), arg(&args, "file").map_err(e2s)?.to_string(), PathBuf::from(arg(&args, "picture").map_err(e2s)?));
                blocking(Box::new(move || crate::magazines::set_cover(&lib, &title, &file, &picture).map(|_| json!({ "ok": true })))).await.and_then(j)
            }
            "magazine_pages" => {
                let lib = self.library()?;
                let rel = arg(&args, "path").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || Ok(json!({ "count": crate::magazines::pages(&lib.resolve(&rel)?)?.len() })))).await?)
            }
            "magazine_list_import" => {
                let lib = self.library()?;
                let path = PathBuf::from(arg(&args, "path").map_err(e2s)?);
                j(blocking(Box::new(move || crate::magazines::import_list(&lib, &path))).await?)
            }
            "magazine_list_remove" => {
                let lib = self.library()?;
                let name = arg(&args, "name").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || crate::magazines::remove_list(&lib, &name).map(|_| json!({ "ok": true })))).await?)
            }
            "magazine_missing" => {
                // the issues missing, saved as a list (in the library's reports, or where asked)
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let title = args["title"].as_str().filter(|t| !t.is_empty()).map(String::from);
                let to = args["to"].as_str().filter(|t| !t.trim().is_empty()).map(PathBuf::from);
                j(blocking(Box::new(move || {
                    let name = format!("Magazines missing{} ({}).csv", title.as_deref().map(|t| format!(", {t}")).unwrap_or_default(), &crate::library::now()[..10]);
                    let path = to.unwrap_or_else(|| lib.root().join(crate::library::APP_DIR).join("reports").join(organise::safe_name(&name)));
                    if let Some(d) = path.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    let n = crate::magazines::save_missing(&lib, title.as_deref(), &path)?;
                    Ok(json!({ "path": path.display().to_string(), "rel": lib.relative(&path), "issues": n }))
                }))
                .await?)
            }
            // the issues' text read, and the games in it found (D145): one magazine's, or every one's
            "magazine_read" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let title = args["title"].as_str().filter(|t| !t.is_empty()).map(String::from);
                let again = args["again"].as_bool().unwrap_or(false);
                let label = "Reading magazines' text";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    crate::magtext::read_issues(&ix, &lib, title.as_deref(), again, &cancel, &mut |p| app.job_progress(&jid, p))
                }))
            }
            "magazine_text" => {
                let lib = self.library()?;
                let rel = arg(&args, "path").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || crate::magtext::text_of(&lib, &rel))).await?)
            }
            "magazine_game" => {
                // a game found in an issue ticked or unticked, or one added by hand ("ticked": null takes it off)
                let lib = self.library()?;
                let (title, file, key) = (arg(&args, "title").map_err(e2s)?.to_string(), arg(&args, "file").map_err(e2s)?.to_string(), arg(&args, "key").map_err(e2s)?.to_string());
                let ticked = args["ticked"].as_bool();
                let page = args["page"].as_u64().map(|p| p as u32);
                j(self.with_index(move |ix| crate::magtext::set_game(ix, &lib, &title, &file, &key, ticked, page)).await?)
            }
            // the Devices page (D187, D188): each device's record, contents and waiting changes
            "devices_list" => {
                let lib = self.library()?;
                j(crate::devices::Records::of(lib.root()).list())
            }
            "devices_view" => {
                let lib = self.library()?;
                j(crate::devices::Records::of(lib.root()).view(arg(&args, "id").map_err(e2s)?).map_err(e2s)?)
            }
            "devices_queue" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let changes: Vec<gamerepo_wire::devices::Change> = serde_json::from_value(args["changes"].clone()).map_err(|e| e.to_string())?;
                let r = crate::devices::Records::of(lib.root());
                let added = r.add(&id, changes).map_err(e2s)?;
                let mut v = r.view(&id).map_err(e2s)?;
                v["added"] = json!(added);
                j(v)
            }
            "devices_cancel" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let ids: Vec<String> = serde_json::from_value(args["ids"].clone()).map_err(|e| e.to_string())?;
                let r = crate::devices::Records::of(lib.root());
                r.cancel(&id, &ids).map_err(e2s)?;
                j(r.view(&id).map_err(e2s)?)
            }
            "devices_forget" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                crate::devices::Records::of(lib.root()).forget(arg(&args, "id").map_err(e2s)?).map_err(e2s)?;
                j(json!({}))
            }
            // folder devices (D189): a card, drive or share made a device, found again, read and changed
            "devices_layouts" => {
                let lib = self.library()?;
                let v: Vec<Value> = crate::layout::all(Some(&lib)).iter().filter(|l| crate::export::device_place(l).is_ok()).map(|l| json!({ "id": l.id, "name": l.name })).collect();
                j(json!(v))
            }
            "devices_folder_add" => {
                let lib = self.library()?;
                let folder = PathBuf::from(arg(&args, "folder").map_err(e2s)?.trim());
                let info = crate::devices::add_folder(&lib, &folder, args["name"].as_str().unwrap_or(""), arg(&args, "layout").map_err(e2s)?).map_err(e2s)?;
                crate::devices::read_folder(&lib, &info.id, &folder).map_err(e2s)?;
                j(json!(info))
            }
            "devices_present" => {
                let lib = self.library()?;
                let v: Vec<Value> = crate::devices::present(&lib).into_iter().map(|(i, at)| json!({ "id": i.id, "at": at.display().to_string(), "eject": crate::devices::can_eject(&at) })).collect();
                j(json!(v))
            }
            "devices_folder_read" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let at = crate::devices::where_is(&lib, &id).ok_or("It isn't connected: put the card in, or plug the drive in, and try again.")?;
                crate::devices::read_folder(&lib, &id, &at).map_err(e2s)?;
                j(crate::devices::Records::of(lib.root()).view(&id).map_err(e2s)?)
            }
            "devices_folder_sync" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let at = crate::devices::where_is(&lib, &id).ok_or("It isn't connected: put the card in, or plug the drive in, and try again.")?;
                let name = crate::devices::Records::of(lib.root()).info(&id).map(|i| i.name).unwrap_or_default();
                let running = self.begin_change(&format!("device-{id}"))?;
                j(self.spawn_job_as(&format!("Changing the games on {name}"), args["auto"].as_bool().unwrap_or(false), move |app, jid, cancel| {
                    let _running = running;
                    let ix = Self::job_index(&data_dir, &lib)?;
                    crate::meta::sync(&ix, &lib)?;
                    let waiting = crate::devices::Records::of(lib.root()).waiting(&id);
                    let (r, contents) = if waiting.is_empty() {
                        (json!({ "done": 0, "failed": [], "nothing": true }), crate::devices::read_folder(&lib, &id, &at)?)
                    } else {
                        let (plan, changes) = crate::devices::plan_folder(&ix, &lib, &id, &at)?;
                        let r = organise::run(&ix, &lib, &plan, &cancel, &mut |p| app.job_progress(&jid, p))?;
                        let c = crate::devices::folder_done(&lib, &id, &at, Some(&plan), &changes, &r)?;
                        (r, c)
                    };
                    let mut r = r;
                    // its saves too (D189), where its layout says they are
                    r["saves"] = json!(crate::devices::sync_folder_saves(&lib, &id, &at, &contents));
                    r["device"] = json!(id);
                    r["games"] = json!(contents.games.len());
                    r["free"] = json!(contents.free);
                    Ok(r)
                }))
            }
            // *Eject* (D189): made safe to take out, unless it's being changed
            "devices_folder_eject" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let at = crate::devices::where_is(&lib, &id).ok_or("It isn't connected.")?;
                let _running = self.begin_change(&format!("device-{id}"))?;
                crate::devices::eject(&at).map_err(e2s)?;
                j(json!({}))
            }
            "devices_holding" => {
                let lib = self.library()?;
                j(json!(crate::devices::Records::of(lib.root()).holding(arg(&args, "key").map_err(e2s)?)))
            }
            // saves (D181, D184, D191): a game's, *Use this one*, and Settings › Saves
            "saves_of" => {
                let lib = self.library()?;
                let key = arg(&args, "key").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || Ok(crate::saves::of_game(&lib, &key)))).await?)
            }
            "saves_use" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let unit = gamerepo_wire::saves::Unit::from_id(arg(&args, "unit").map_err(e2s)?).ok_or("That isn't a save.")?;
                let version = arg(&args, "version").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || Ok(serde_json::to_value(crate::saves::Store::of(lib.root()).choose(&unit, &version, "game-repo", "Game Repo")?)?))).await?)
            }
            "saves_settings" => {
                let lib = self.library()?;
                j(blocking(Box::new(move || Ok(crate::saves::settings(&lib)))).await?)
            }
            // saves not matched (D195): given a game, or thrown away
            "saves_loose_give" | "saves_loose_drop" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let sha1 = arg(&args, "sha1").map_err(e2s)?.to_string();
                let key = args["key"].as_str().unwrap_or("").to_string();
                let give = cmd == "saves_loose_give";
                j(blocking(Box::new(move || {
                    let store = crate::saves::Store::of(lib.root());
                    if give {
                        store.give(&sha1, &key)?;
                    } else {
                        store.drop_loose(&sha1)?;
                    }
                    Ok(crate::saves::settings(&lib))
                })).await?)
            }
            "saves_set" => {
                let lib = self.library()?;
                let states = args["states"].as_bool().ok_or("Say whether states are synced.")?;
                lib.update_meta(json!({ "saves": { "states": states } })).map_err(e2s)?;
                j(crate::saves::settings(&lib))
            }
            // the backlog (D157-D159): every change returns it with what it was before, for Undo
            "backlog" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| crate::backlog::view(ix, &lib)).await?)
            }
            "backlog_add" => {
                let lib = self.library()?;
                let games: Vec<crate::backlog::Game> = serde_json::from_value(args["games"].clone()).map_err(|e| e.to_string())?;
                if games.is_empty() {
                    return Err("Choose a game first.".into());
                }
                let top = args["top"].as_bool().unwrap_or(false);
                j(self.with_index(move |ix| crate::backlog::add(ix, &lib, games, top)).await?)
            }
            "backlog_move" | "backlog_playing" | "backlog_complete" | "backlog_remove" => {
                let lib = self.library()?;
                let keys = crate::backlog::keys_of(&args).map_err(e2s)?;
                let (to, on) = (args["to"].as_u64().unwrap_or(0) as usize, args["on"].as_bool().unwrap_or(true));
                let cmd = cmd.to_string();
                j(self
                    .with_index(move |ix| {
                        crate::backlog::change(ix, &lib, |b| {
                            match cmd.as_str() {
                                "backlog_move" => b.move_to(&keys, to),
                                "backlog_playing" => b.set_playing(&keys, on),
                                "backlog_complete" => b.complete(&keys),
                                _ => b.remove(&keys),
                            }
                            Ok(json!({}))
                        })
                    })
                    .await?)
            }
            "backlog_restore" => {
                let lib = self.library()?;
                let was = args["backlog"].clone();
                j(self.with_index(move |ix| crate::backlog::restore(ix, &lib, was)).await?)
            }
            // the Videos page (D146)
            "videos" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| Ok(crate::videos::overview(ix, &lib))).await?)
            }
            "video_items" => {
                // the videos in the files and folders picked, and the games in their titles (a job)
                let lib = self.library()?;
                let paths: Vec<PathBuf> = args["paths"].as_array().into_iter().flatten().filter_map(Value::as_str).map(PathBuf::from).collect();
                if paths.is_empty() {
                    return Err("Choose the videos or a folder of them.".into());
                }
                let label = "Reading the videos";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let items = crate::videos::read_items(&ix, &lib, &paths, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    let mut items = serde_json::to_value(&items)?;
                    for g in items.as_array_mut().into_iter().flatten().flat_map(|it| it["games"].as_array_mut().into_iter().flatten()) {
                        g["platform_name"] = json!(crate::browse::system_name(g["platform"].as_str().unwrap_or("")));
                    }
                    Ok(json!({ "items": items }))
                }))
            }
            "video_plan" => {
                let lib = self.library()?;
                let items: Vec<crate::videos::Item> = serde_json::from_value(args["items"].clone()).map_err(|e| e.to_string())?;
                let mode = args["mode"].as_str().unwrap_or("copy").to_string();
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(blocking(Box::new(move || {
                    let plan = crate::videos::plan(&lib, &items, &mode, always_copy)?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
                .await?)
            }
            "video_link" => {
                let lib = self.library()?;
                let (url, title) = (arg(&args, "url").map_err(e2s)?.to_string(), arg(&args, "title").map_err(e2s)?.to_string());
                let kind = args["kind"].as_str().filter(|k| !k.is_empty()).map(String::from);
                let date = args["date"].as_str().map(String::from);
                j(self.with_index(move |ix| crate::videos::add_link(ix, &lib, &url, &title, kind.as_deref(), date.as_deref())).await?)
            }
            "video_edit" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let patch = args["patch"].clone();
                j(blocking(Box::new(move || crate::videos::edit(&lib, &id, &patch))).await?)
            }
            // several videos changed at once (D153): kind, series, creator, a tag, a game
            "video_edit_many" => {
                let lib = self.library()?;
                let ids: Vec<String> = args["ids"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
                let patch = args["patch"].clone();
                j(self.with_index(move |ix| crate::videos::edit_many(ix, &lib, &ids, &patch)).await?)
            }
            // the picture a video's tile shows (none: taken off)
            "video_picture" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let picture = args["picture"].as_str().filter(|p| !p.is_empty()).map(PathBuf::from);
                j(blocking(Box::new(move || crate::videos::set_picture(&lib, &id, picture.as_deref()))).await?)
            }
            "video_game" => {
                // a game added to a video ("on": true), or taken off
                let lib = self.library()?;
                let (id, key) = (arg(&args, "id").map_err(e2s)?.to_string(), arg(&args, "key").map_err(e2s)?.to_string());
                let on = args["on"].as_bool().unwrap_or(true);
                j(self.with_index(move |ix| crate::videos::set_game(ix, &lib, &id, &key, on)).await?)
            }
            "video_remove" => {
                // a link taken off at once; a file's removal planned, for the preview and Undo
                let lib = self.library()?;
                let mut ids: Vec<String> = args["ids"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
                if let Some(id) = args["id"].as_str() {
                    ids.push(id.to_string());
                }
                j(blocking(Box::new(move || {
                    Ok(match crate::videos::remove(&lib, &ids)? {
                        Some(plan) => {
                            organise::save_pending(&data_dir, &plan)?;
                            json!({ "plan": plan.summary() })
                        }
                        None => json!({ "removed": true }),
                    })
                }))
                .await?)
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// Systems added by hand, collections and exports (docs/PLAN.md, "Phase 3 design").
    async fn collection_call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        let j = |v: Value| Ok(Reply::Json(v));
        let data_dir = self.paths.data_dir.clone();
        let blocking = |f: Box<dyn FnOnce() -> Result<Value> + Send>| async move { tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(e2s) };
        match cmd {
            "systems_set" => {
                let lib = self.library()?;
                let s = crate::systems::System::from_json(&args["system"]).map_err(e2s)?;
                let old = args["old"].as_str().filter(|o| !o.is_empty()).map(String::from);
                let saved = blocking(Box::new({
                    let lib = lib.clone();
                    move || crate::systems::set(&lib, &s, old.as_deref())
                }))
                .await?;
                // a system made arcade (or not) is audited as one from now on (review M11)
                self.with_index(move |ix| {
                    if crate::arcade::refresh_places(ix.conn(), &lib)? {
                        audit::changed(ix.conn())?;
                    }
                    Ok(())
                })
                .await?;
                j(saved)
            }
            "systems_remove" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                j(blocking(Box::new(move || crate::systems::remove(&lib, &id).map(|_| Value::Null))).await?)
            }
            "collections" => {
                let lib = self.library()?;
                j(json!(collection::list(&lib)))
            }
            "collection_get" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                j(self
                    .with_index(move |ix| {
                        // files that waited on it and a DAT knows now join it (D140)
                        crate::subset::settle(ix, &lib, &id)?;
                        let c = collection::load(&lib, &id)?;
                        let w = collection::work_out(ix, &lib, &c)?;
                        Ok(json!({ "collection": c, "summary": w.summary() }))
                    })
                    .await?)
            }
            "collection_games" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let platform = args["platform"].as_str().filter(|p| !p.is_empty()).map(String::from);
                let group = args["group"].as_str().map(String::from);
                let removed = args["removed"].as_bool().unwrap_or(false);
                let search = args["search"].as_str().unwrap_or("").to_string();
                let by_size = args["sort"].as_str() == Some("size");
                let (offset, limit) = (args["offset"].as_u64().unwrap_or(0) as usize, args["limit"].as_u64().unwrap_or(100) as usize);
                j(self
                    .with_index(move |ix| {
                        let c = collection::load(&lib, &id)?;
                        let w = collection::work_out(ix, &lib, &c)?;
                        Ok(collection::page(&w, platform.as_deref(), group.as_deref(), removed, &search, by_size, offset, limit))
                    })
                    .await?)
            }
            // games left out, put back or added by hand, each in the group that has them (or,
            // added, in `group`), as a step of its history (D67, D75)
            "collection_hand" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let list = |k: &str| -> Vec<String> { args[k].as_array().map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default() };
                let (out, back, add) = (list("leave_out"), list("put_back"), list("add"));
                let group = args["group"].as_str().unwrap_or("").to_string();
                j(self
                    .with_index(move |ix| {
                        let before = crate::history::before(ix, &lib, &id, "", "")?;
                        let mut c = before.collection.clone();
                        let w = collection::work_out(ix, &lib, &c)?;
                        c.leave_out(&w, &out)?;
                        c.put_back(&back);
                        if !add.is_empty() {
                            c.add_games(&group, &add)?;
                        }
                        // put back, a game the rules don't bring in is added by hand to its group
                        if !back.is_empty() {
                            let inside: std::collections::HashSet<String> = collection::work_out(ix, &lib, &c)?.games().map(collection::Game::key).collect();
                            for k in back.iter().filter(|k| !inside.contains(*k)) {
                                let g = w.group_of(k).and_then(|i| w.groups.get(i)).map(|g| g.id.clone()).unwrap_or_default();
                                c.add_games(&g, std::slice::from_ref(k))?;
                            }
                        }
                        let c = collection::save(&lib, c)?;
                        crate::history::record(ix, &lib, before, "hand", None)?;
                        let w = collection::work_out(ix, &lib, &c)?;
                        Ok(json!({ "collection": c, "summary": w.summary() }))
                    })
                    .await?)
            }
            "collection_save" => {
                let lib = self.library()?;
                let c: collection::Collection = serde_json::from_value(args["collection"].clone()).map_err(|e| format!("That isn't a collection: {e}"))?;
                j(self
                    .with_index(move |ix| {
                        // each change is a step of its history (D67)
                        let before = if c.id.is_empty() { None } else { Some(crate::history::before(ix, &lib, &c.id, "", "")?) };
                        let c = collection::save(&lib, c)?;
                        let how = if before.is_none() { "made" } else { "" };
                        crate::history::record(ix, &lib, before.unwrap_or_else(|| crate::history::new(&c)), how, None)?;
                        let w = collection::work_out(ix, &lib, &c)?;
                        Ok(json!({ "collection": c, "summary": w.summary() }))
                    })
                    .await?)
            }
            "collection_history" => {
                let lib = self.library()?;
                crate::history::view(&lib, arg(&args, "id").map_err(e2s)?).map_err(e2s).and_then(j)
            }
            "collection_undo" => {
                let lib = self.library()?;
                let (id, step) = (arg(&args, "id").map_err(e2s)?.to_string(), arg(&args, "step").map_err(e2s)?.to_string());
                j(self.with_index(move |ix| crate::history::undo(ix, &lib, &id, &step)).await?)
            }
            "collection_delete" => {
                let lib = self.library()?;
                collection::delete(&lib, arg(&args, "id").map_err(e2s)?).map_err(e2s)?;
                j(Value::Null)
            }
            "collection_candidates" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let platform = arg(&args, "platform").map_err(e2s)?.to_string();
                let group = args["group"].as_str().unwrap_or("").to_string();
                let search = args["search"].as_str().unwrap_or("").to_string();
                let limit = args["limit"].as_u64().unwrap_or(100) as usize;
                j(self
                    .with_index(move |ix| {
                        let c = collection::load(&lib, &id)?;
                        collection::candidates(ix, &lib, &c, &group, &platform, &search, limit)
                    })
                    .await?)
            }
            "collection_dats" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                let to = args["to"].as_str().filter(|t| !t.trim().is_empty()).map(PathBuf::from);
                j(self
                    .with_index(move |ix| {
                        let c = collection::load(&lib, &id)?;
                        collection::save_dats(ix, &lib, &c, to.as_deref())
                    })
                    .await?)
            }
            // a collection from folders or from a DAT (D49): read, review, then make it
            "collection_read_folders" => {
                let lib = self.library()?;
                let paths: Vec<PathBuf> = args["paths"].as_array().map(|a| a.iter().filter_map(|p| p.as_str()).map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).collect()).unwrap_or_default();
                if paths.is_empty() {
                    return Err("Pick a folder to read.".into());
                }
                if let Some(p) = paths.iter().find(|p| !p.is_dir()) {
                    return Err(format!("{} isn't a folder.", p.display()));
                }
                self.not_running("Reading folders")?;
                j(self.spawn_job("Reading folders", move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    // a file as big as no ROM of the audit DATs isn't read (D133)
                    let sizes = crate::subset::dat_sizes(ix.conn(), &lib)?;
                    let found = crate::subset::read_folders(&paths, Some(&sizes), &cancel, &mut |p| app.job_progress(&jid, p))?;
                    if cancel.load(Ordering::Relaxed) {
                        bail!("Stopped.");
                    }
                    app.job_progress(&jid, json!({ "text": "Matching them to your DATs" }));
                    let mut v = crate::subset::review(&ix, &lib, &found)?;
                    v["made_from"] = json!({ "kind": "folders", "paths": paths, "read": library::now() });
                    Ok(v)
                }))
            }
            "collection_read_dat" => {
                let lib = self.library()?;
                let path = PathBuf::from(arg(&args, "path").map_err(e2s)?.trim());
                j(self
                    .with_index(move |ix| {
                        let found = crate::subset::read_dat(&path)?;
                        let mut v = crate::subset::review(ix, &lib, &found)?;
                        v["made_from"] = json!({ "kind": "dat", "path": path, "read": library::now() });
                        Ok(v)
                    })
                    .await?)
            }
            // a collection, or (`id` and `group_name`) a new group of one, made from folders or
            // a DAT; or (`id`, and `group`: "" is the first) one of its groups read again
            "collection_from" => {
                let lib = self.library()?;
                let name = args["name"].as_str().unwrap_or("").trim().to_string();
                let id = args["id"].as_str().filter(|i| !i.is_empty()).map(String::from);
                let group = args["group"].as_str().unwrap_or("").to_string();
                let group_name = args["group_name"].as_str().unwrap_or("").trim().to_string();
                if name.is_empty() && id.is_none() {
                    return Err("Give the collection a name.".into());
                }
                if id.is_some() && args["group_name"].is_string() && group_name.is_empty() {
                    return Err("Give the group a name.".into());
                }
                let games: Vec<i64> = args["games"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                let made_from = args["made_from"].clone();
                // files imported into it that no DAT knows yet (D140)
                let waiting: Vec<collection::Waiting> = serde_json::from_value(args["waiting"].clone()).unwrap_or_default();
                j(self
                    .with_index(move |ix| {
                        // read again, or a group added: a step of its history, with the DATs it had kept (D67)
                        let how = if group_name.is_empty() { "read" } else { "" };
                        let before = match &id { Some(id) => Some(crate::history::before(ix, &lib, id, how, &group)?), None => None };
                        let into = match &id {
                            None => crate::subset::Target::New(&name),
                            Some(id) if !group_name.is_empty() => crate::subset::Target::Group(id, &group_name),
                            Some(id) => crate::subset::Target::Again(id, &group),
                        };
                        let c = crate::subset::save(ix, &lib, into, &games, made_from, waiting)?;
                        match before {
                            Some(b) => crate::history::record(ix, &lib, b, how, None)?,
                            None => crate::history::record(ix, &lib, crate::history::new(&c), "made", None)?,
                        };
                        let w = collection::work_out(ix, &lib, &c)?;
                        Ok(json!({ "collection": c, "summary": w.summary() }))
                    })
                    .await?)
            }
            "collection_waiting_leave" => {
                let lib = self.library()?;
                let c = crate::subset::leave_waiting(&lib, arg(&args, "id").map_err(e2s)?, args["group"].as_str().unwrap_or(""), arg(&args, "platform").map_err(e2s)?, arg(&args, "name").map_err(e2s)?)
                    .map_err(e2s)?;
                j(json!(c))
            }
            "collection_bring_plan" => {
                let lib = self.library()?;
                let paths = |v: &Value| -> Vec<PathBuf> { v.as_array().map(|a| a.iter().filter_map(Value::as_str).map(PathBuf::from).collect()).unwrap_or_default() };
                let imp = crate::subset::Import {
                    games: args["games"].as_array().map(|a| a.iter().filter_map(|g| Some((g["id"].as_i64()?, paths(&g["files"])))).collect()).unwrap_or_default(),
                    files: args["files"].as_array().map(|a| a.iter().filter_map(|g| Some((g["platform"].as_str()?.to_string(), paths(&g["files"])))).collect()).unwrap_or_default(),
                    places: args["places"].as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), PathBuf::from(v.as_str()?.trim())))).collect()).unwrap_or_default(),
                    mode: args["mode"].as_str().unwrap_or("copy").to_string(),
                    always_copy: args["always_copy"].as_bool().unwrap_or(false),
                };
                // a job: every file is read, which can take a while; Stop ends it
                j(self.spawn_job("Checking the files to import", move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let plan = crate::subset::bring_plan(&ix, &lib, &imp, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
            }
            "export_defaults" => j(export::esde_default()),
            "export_plan" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                let mut o = match args["export"].as_str() {
                    Some(id) => export::options_of(&lib, id).map_err(e2s)?,
                    None => export::Options {
                        collection: arg(&args, "collection").map_err(e2s)?.to_string(),
                        layout: args["layout"].as_str().unwrap_or("").to_string(),
                        places: args["places"].as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(str::trim).filter(|v| !v.is_empty()).map(|v| (k.clone(), PathBuf::from(v)))).collect()).unwrap_or_default(),
                        target: args["target"].as_str().unwrap_or("esde").to_string(),
                        out: PathBuf::from(args["out"].as_str().unwrap_or("").trim()),
                        esde: args["esde"].as_str().filter(|e| !e.trim().is_empty()).map(|e| PathBuf::from(e.trim())),
                        always_copy: args["always_copy"].as_bool().unwrap_or(false),
                        short_names: args["short_names"].as_bool().unwrap_or(true),
                        export_id: None,
                        carry: Default::default(),
                        only: args["only"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(String::from)).collect()).unwrap_or_default(),
                        groups: args["groups"].as_str().unwrap_or("").to_string(),
                        rom_dir: None,
                    },
                };
                if args["carry"].is_object() {
                    o.carry = serde_json::from_value(args["carry"].clone()).map_err(|e| e.to_string())?;
                }
                // an Update can place the groups another way
                if let Some(g) = args["groups"].as_str().filter(|_| args["export"].is_string()) {
                    o.groups = g.to_string();
                }
                j(self
                    .with_index(move |ix| {
                        crate::meta::sync(ix, &lib)?;
                        let plan = export::plan(ix, &lib, &o)?;
                        organise::save_pending(&data_dir, &plan)?;
                        let mut v = plan.summary();
                        v["present"] = json!(plan.present.len());
                        // what the user must set up by hand (a group's own system in ES-DE, D75)
                        v["notes"] = plan.export.as_ref().map(|e| e["notes"].clone()).unwrap_or_else(|| json!([]));
                        v["groups"] = plan.export.as_ref().map(|e| e["groups"].clone()).unwrap_or(Value::Null);
                        Ok(v)
                    })
                    .await?)
            }
            "exports" => {
                let lib = self.library()?;
                j(json!(export::list(&lib)))
            }
            "export_forget" => {
                let lib = self.library()?;
                export::forget(&lib, arg(&args, "id").map_err(e2s)?).map_err(e2s)?;
                j(Value::Null)
            }
            "export_check" => {
                let lib = self.library()?;
                let id = arg(&args, "id").map_err(e2s)?.to_string();
                export::load(&lib, &id).map_err(e2s)?;
                self.not_running("Checking an export")?;
                j(self.spawn_job("Checking an export", move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    export::check(&ix, &lib, &id, &cancel, &mut |p| app.job_progress(&jid, p))
                }))
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// Part of a file in the open library for an HTTP `Range` header ("bytes=a-b"):
    /// (status, bytes, Content-Range). Without one, the whole file (200).
    pub fn library_range(
        &self,
        rel: &str,
        range: Option<&str>,
    ) -> Result<(u16, Vec<u8>, Option<String>)> {
        use std::io::{Read, Seek, SeekFrom};
        let path = self.served_path(rel)?;
        let Some(r) = range.and_then(|r| r.trim().strip_prefix("bytes=")) else {
            return Ok((200, std::fs::read(&path)?, None));
        };
        let total = std::fs::metadata(&path)?.len();
        let (a, b) = r.split_once('-').unwrap_or((r, ""));
        let (start, end) = match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
            (Some(s), Some(e)) => (s, e.min(total.saturating_sub(1))),
            (Some(s), None) => (s, (s + (8 << 20)).min(total).saturating_sub(1)), // 8 MB at a time
            (None, Some(n)) => (total.saturating_sub(n), total.saturating_sub(1)),
            _ => return Ok((200, std::fs::read(&path)?, None)),
        };
        if start >= total || end < start {
            return Ok((416, vec![], Some(format!("bytes */{total}"))));
        }
        let mut f = std::fs::File::open(&path)?;
        f.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0; (end - start + 1) as usize];
        f.read_exact(&mut buf)?;
        Ok((206, buf, Some(format!("bytes {start}-{end}/{total}"))))
    }

    /// A file in the open library, by its library-relative path (the `library://` protocol).
    pub fn library_file(&self, rel: &str) -> Result<Vec<u8>> {
        Ok(std::fs::read(self.served_path(rel)?)?)
    }

    /// The name a `library://` path's type is told by: its own, but for a magazine's cover
    /// or page, whose file (a JPEG, a plain SVG cover, a comic's PNG) differs from the issue's.
    pub fn served_name(&self, rel: &str) -> String {
        let bare = rel.trim_start_matches('/');
        if bare.starts_with("~cover/") || bare.starts_with("~page/") || bare.starts_with("~first/") {
            if let Ok(p) = self.served_path(rel) {
                return p.to_string_lossy().into_owned();
            }
        }
        rel.split('?').next().unwrap_or("").to_string()
    }

    /// What a `library://` path is: a file in the open library.
    fn served_path(&self, rel: &str) -> Result<PathBuf> {
        let rel = percent_decode(rel.trim_start_matches('/'));
        // `?v=…` after a picture's name only keeps the page from showing an old one (D47)
        let rel = rel.split('?').next().unwrap_or("").to_string();
        let lib = self.library().map_err(|e| anyhow!(e))?;
        // ~thumb/<size>/<path>: a thumbnail of a picture (D43), made the first time
        if let Some(rest) = rel.strip_prefix("~thumb/") {
            let (size, pic) = rest.split_once('/').ok_or_else(|| anyhow!("no picture"))?;
            return crate::browse::thumb(&lib, &self.paths.data_dir, pic, size.parse().unwrap_or(0));
        }
        // ~cover/<path>: a magazine issue's cover; ~page/<n>/<path>: a page of a CBZ or CBR (D68)
        if let Some(rest) = rel.strip_prefix("~cover/") {
            return crate::magazines::cover_file(&lib, &self.paths.data_dir, rest);
        }
        // ~first/<path>: a document's first page as a picture (D155)
        if let Some(rest) = rel.strip_prefix("~first/") {
            return crate::magazines::first_file(&lib, &self.paths.data_dir, rest);
        }
        if let Some(rest) = rel.strip_prefix("~page/") {
            let (n, issue) = rest.split_once('/').ok_or_else(|| anyhow!("no issue"))?;
            return crate::magazines::page_file(&lib, &self.paths.data_dir, n.parse().unwrap_or(1), issue);
        }
        lib.resolve(&rel)
    }
}

/// Magazines' issues named in a command, `[{ title, file }]`: (magazine, file).
fn issue_refs(v: &Value) -> Vec<(String, String)> {
    v.as_array().into_iter().flatten().filter_map(|x| Some((x["title"].as_str()?.to_string(), x["file"].as_str()?.to_string()))).collect()
}

/// Run `f` on `lib`'s index in `slot`, opening it first if the slot holds none or
/// another library's.
fn index_of<T>(
    slot: &Mutex<OpenIndex>,
    data_dir: &Path,
    lib: &Library,
    f: impl FnOnce(&Index) -> Result<T>,
) -> Result<T> {
    let id = lib
        .id()
        .ok_or_else(|| anyhow!("This library has no id in _library/library.json, so its index can't be found."))?;
    let mut g = lock(&slot);
    if g.as_ref().is_none_or(|(open, _)| open != &id) {
        let ix = Index::open(&index::path_for(data_dir, &id), &id)?;
        *g = Some((id, ix));
    }
    f(&g.as_ref().unwrap().1)
}

/// "%20" -> " " (paths in library URLs and file names sent in headers).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> (Arc<App>, PathBuf) {
        let home = std::env::temp_dir().join(format!("gamerepo-api-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let paths = AppPaths {
            config_dir: home.join("config"),
            data_dir: home.join("data"),
            library_url: "/library/".into(),
            server_data: home.join("server"),
            server_program: vec![],
        };
        (App::new(paths).unwrap(), home)
    }

    async fn call(app: &Arc<App>, cmd: &str, args: Value) -> Value {
        match app.call(cmd, args).await.unwrap() {
            Reply::Json(v) => v,
            Reply::Bytes(_) => panic!("bytes"),
        }
    }

    async fn wait(app: &Arc<App>, job: &Value) -> Value {
        let id = job["job"].as_str().unwrap().to_string();
        for _ in 0..500 {
            let done = call(app, "job", json!({ "id": id })).await;
            if done["done"] == true {
                return done;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("job didn't finish")
    }

    #[tokio::test]
    async fn opens_renames_and_remembers_libraries() {
        let (app, home) = app("open");
        let a = home.join("Games A");
        let info = call(
            &app,
            "library_open",
            json!({ "path": a.display().to_string() }),
        )
        .await;
        assert_eq!(info["name"], "Games A");
        let info = call(&app, "library_rename", json!({ "name": "  Retro " })).await;
        assert_eq!(info["name"], "Retro");
        assert!(app
            .call("library_rename", json!({ "name": " " }))
            .await
            .is_err());
        call(
            &app,
            "library_open",
            json!({ "path": home.join("Games B").display().to_string() }),
        )
        .await;
        let info = call(&app, "app_info", json!({})).await;
        assert_eq!(info["library"]["name"], "Games B");
        assert_eq!(info["recent_libraries"].as_array().unwrap().len(), 2);
        assert_eq!(info["library_format"], library::FORMAT);
        assert_eq!(info["index_version"], index::schema_version());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn first_start_asks_where_the_library_goes() {
        let (app, home) = app("first");
        let info = call(&app, "app_info", json!({})).await;
        assert_eq!(info["first_run"], true);
        assert!(info["library"].is_null() && info["library_error"].is_null());
        assert!(app.call("index_info", json!({})).await.is_err());
        call(
            &app,
            "library_open",
            json!({ "path": home.join("Lib").display().to_string() }),
        )
        .await;
        assert_eq!(call(&app, "app_info", json!({})).await["first_run"], false);
        // Phase 8b's calls reach their commands
        let e = app.call("addon_link", json!({ "platform": "gb", "addon": "Nothing (USA) (DLC)" })).await.err().unwrap();
        assert!(e.contains("isn't an update"), "{e}");
        let e = app.call("collection_history", json!({ "id": "none" })).await.err().unwrap();
        assert!(e.contains("There's no collection"), "{e}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn prefs_are_kept_on_this_computer() {
        let (app, home) = app("prefs");
        call(
            &app,
            "prefs_set",
            json!({ "prefs": { "gr-ui": { "theme": "dark" } } }),
        )
        .await;
        assert_eq!(call(&app, "prefs_get", json!({})).await["gr-ui"]["theme"], "dark");
        assert!(app.call("prefs_set", json!({ "prefs": [1] })).await.is_err());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn the_index_follows_the_library_and_stays_on_this_computer() {
        let (app, home) = app("index");
        let a = home.join("Games A");
        call(&app, "library_open", json!({ "path": a.display().to_string() })).await;
        let ia = call(&app, "index_info", json!({})).await;
        let id_a = Library::open(&a).unwrap().id().unwrap();
        assert_eq!(
            PathBuf::from(ia["path"].as_str().unwrap()),
            home.join("data/index").join(format!("{id_a}.sqlite"))
        );
        assert_eq!(ia["version"], index::schema_version());
        assert!(!a.join("_library").read_dir().unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".sqlite")));
        // another library has its own index
        call(&app, "library_open", json!({ "path": home.join("Games B").display().to_string() })).await;
        let ib = call(&app, "index_info", json!({})).await;
        assert_ne!(ia["path"], ib["path"]);
        // a library that was moved keeps its index (it's found by the library's id)
        let moved = home.join("moved/Games A");
        std::fs::create_dir_all(moved.join("_library")).unwrap();
        std::fs::copy(a.join("_library/library.json"), moved.join("_library/library.json")).unwrap();
        call(&app, "library_open", json!({ "path": moved.display().to_string() })).await;
        let again = call(&app, "index_info", json!({})).await;
        assert_eq!(again["path"], ia["path"]);
        assert_eq!(again["created"], ia["created"]);
        // SQLite's check runs as a job
        let job = call(&app, "index_check", json!({})).await;
        let done = wait(&app, &job).await;
        assert_eq!(done["result"]["ok"], true, "{done}");
        assert_eq!(done["label"], "Checking the index");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn folders_scan_dats_and_the_audit() {
        use crate::audit::tests::{dat_xml, rom, rom_xml};
        let (app, home) = app("audit");
        call(&app, "library_open", json!({ "path": home.join("Lib").display().to_string() })).await;
        let roms = home.join("Roms/GB");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(roms.join("Alpha (USA).gb"), rom(1, 4096)).unwrap();
        std::fs::write(roms.join("x.gb"), rom(2, 4096)).unwrap();
        let root = call(&app, "roots_add", json!({ "path": home.join("Roms").display().to_string() })).await;
        assert_eq!(root["found"], true);
        assert!(app.call("roots_add", json!({ "path": roms.display().to_string() })).await.is_err());
        call(&app, "scan_settings_set", json!({ "readers": 3 })).await;
        assert_eq!(call(&app, "scan_settings", json!({})).await, json!({ "full": false, "readers": 3, "watch": true }));
        let done = wait(&app, &call(&app, "scan", json!({})).await).await;
        assert_eq!(done["result"]["hashed"], 2, "{done}");
        let dat = dat_xml("Nintendo - Game Boy", "1", &[
            ("Alpha (USA)", None, rom_xml("Alpha (USA).gb", &rom(1, 4096))),
            ("Beta (USA)", None, rom_xml("Beta (USA).gb", &rom(2, 4096))),
            ("Gamma (USA)", None, rom_xml("Gamma (USA).gb", &rom(3, 4096))),
        ]);
        std::fs::write(home.join("gb.dat"), dat).unwrap();
        let done = wait(&app, &call(&app, "dat_import", json!({ "paths": [home.join("gb.dat").display().to_string()] })).await).await;
        assert_eq!(done["result"]["imported"][0]["platform"], "gb", "{done}");
        assert_eq!(call(&app, "dats", json!({})).await.as_array().unwrap().len(), 1);
        let a = call(&app, "audit_summary", json!({})).await;
        assert_eq!((a["summary"]["have"].as_i64(), a["summary"]["missing"].as_i64(), a["summary"]["misnamed"].as_i64()), (Some(2), Some(1), Some(1)), "{a}");
        let dat_id = a["summary"]["dat"]["id"].clone();
        let l = call(&app, "audit_list", json!({ "dat": dat_id, "tab": "misnamed" })).await;
        assert_eq!(l["items"][0]["expected"], "Beta (USA).gb");
        let fx = call(&app, "audit_fixdat", json!({ "dat": dat_id })).await;
        assert_eq!(fx["games"], 1);
        assert!(call(&app, "audit_report", json!({ "dat": dat_id })).await["text"].as_str().unwrap().contains("Gamma (USA)"));
        let h = call(&app, "home", json!({})).await;
        assert_eq!(h["platforms"][0]["have"], 2);
        let made = wait(&app, &call(&app, "make_dat", json!({ "root": root["id"], "folder": "GB", "name": "Mine" })).await).await;
        assert_eq!(made["result"]["games"], 2, "{made}");
        call(&app, "roots_remove", json!({ "id": root["id"] })).await;
        assert_eq!(call(&app, "home", json!({})).await["files"], 0);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn organise_undo_and_the_folder_watcher() {
        use crate::audit::tests::{dat_xml, rom, rom_xml};
        let (app, home) = app("organise");
        call(&app, "library_open", json!({ "path": home.join("Lib").display().to_string() })).await;
        let roms = home.join("Roms/GB");
        std::fs::create_dir_all(&roms).unwrap();
        std::fs::write(roms.join("Alpha (USA).gb"), rom(1, 4096)).unwrap();
        std::fs::write(roms.join("x.gb"), rom(2, 4096)).unwrap();
        call(&app, "roots_add", json!({ "path": home.join("Roms").display().to_string() })).await;
        wait(&app, &call(&app, "scan", json!({})).await).await;
        let dat = dat_xml("Nintendo - Game Boy", "1", &[
            ("Alpha (USA)", None, rom_xml("Alpha (USA).gb", &rom(1, 4096))),
            ("Beta (USA)", None, rom_xml("Beta (USA).gb", &rom(2, 4096))),
            ("Gamma (USA)", None, rom_xml("Gamma (USA).gb", &rom(3, 4096))),
        ]);
        std::fs::write(home.join("gb.dat"), dat).unwrap();
        wait(&app, &call(&app, "dat_import", json!({ "paths": [home.join("gb.dat").display().to_string()] })).await).await;
        let dat_id = call(&app, "audit_summary", json!({})).await["summary"]["dat"]["id"].clone();

        // the folder is watched: a file added outside the app is scanned without asking
        let roots = call(&app, "roots", json!({})).await;
        assert_eq!((roots[0]["watched"].as_bool(), roots[0]["network"].as_bool()), (Some(true), Some(false)), "{roots}");
        let t = std::time::Instant::now();
        while lock(&app.watcher).is_none() {
            assert!(t.elapsed().as_secs() < 10, "the folders are watched");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        std::fs::write(roms.join("Gamma (USA).gb"), rom(3, 4096)).unwrap();
        let auto = loop {
            let jobs = call(&app, "jobs", json!({})).await;
            if let Some(job) = jobs.as_array().unwrap().iter().find(|j| j["auto"] == true && j["done"] == true) {
                break job.clone();
            }
            assert!(t.elapsed().as_secs() < 20, "the change was scanned: {jobs}");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        };
        assert_eq!((auto["result"]["added"].as_u64(), auto["result"]["places"].as_u64()), (Some(1), Some(1)), "{auto}");
        let a = call(&app, "audit_summary", json!({ "dat": dat_id })).await;
        assert_eq!(a["summary"]["have"], 3, "{a}");

        // rename the misnamed file where it is, through a preview
        let plan = call(&app, "organise_plan", json!({ "dat": dat_id, "mode": "rename", "only_misnamed": true })).await;
        assert_eq!(plan["ops"], 1, "{plan}");
        let p = call(&app, "organise_preview", json!({ "id": plan["id"] })).await;
        assert_eq!(p["items"][0]["op"], "rename");
        assert!(p["items"][0]["to"].as_str().unwrap().ends_with("Beta (USA).gb"), "{p}");
        // H17: while a scrape writes games, no change, other scrape or hand edit runs beside
        // it (a scan may)
        let scraping = app.begin_meta("Scraping").unwrap();
        for (cmd, a) in [("organise_run", json!({ "id": plan["id"] })), ("scrape_run", json!({ "options": {} })), ("browse_edit", json!({ "key": "gb/Alpha (USA)", "field": "description", "value": "x" })), ("meta_set_details_from", json!({ "field": "description" }))] {
            let e = app.call(cmd, a).await.err().unwrap_or_default();
            assert!(e.starts_with("Scraping is running"), "{cmd}: {e}");
        }
        assert!(!app.changing());
        drop(scraping);
        let done = wait(&app, &call(&app, "organise_run", json!({ "id": plan["id"] })).await).await;
        assert_eq!(done["result"]["done"], 1, "{done}");
        assert!(roms.join("Beta (USA).gb").is_file() && !roms.join("x.gb").exists());
        assert!(app.call("organise_run", json!({ "id": plan["id"] })).await.is_err(), "a plan runs once");
        let h = call(&app, "change_history", json!({})).await;
        assert_eq!((h[0]["state"].as_str(), h[0]["can_undo"].as_bool()), (Some("done"), Some(true)), "{h}");
        let undone = wait(&app, &call(&app, "change_undo", json!({ "id": plan["id"] })).await).await;
        assert!(undone["error"].is_null(), "{undone}");
        assert!(roms.join("x.gb").is_file() && !roms.join("Beta (USA).gb").exists());
        assert_eq!(call(&app, "change_history", json!({})).await[0]["state"], "undone");

        // naming
        let n = call(&app, "naming", json!({ "platform": "gb" })).await;
        assert_eq!(n["template"], "{name}");
        call(&app, "naming_set", json!({ "platform": "gb", "template": "{title} ({region})", "folder": true })).await;
        let n = call(&app, "naming", json!({ "platform": "gb" })).await;
        assert_eq!((n["folder"].as_bool(), n["examples"][0][1].as_str()), (Some(true), Some("Super Game (USA, Europe)")), "{n}");
        assert!(app.call("naming_set", json!({ "platform": "gb", "template": "{nope}" })).await.is_err());
        call(&app, "scan_settings_set", json!({ "watch": false })).await;
        assert_eq!(call(&app, "roots", json!({})).await[0]["watched"], false);
        assert!(lock(&app.watcher).is_none());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn library_files_stay_inside() {
        let (app, home) = app("files");
        let lib = home.join("Lib");
        app.open_library(&lib).unwrap();
        std::fs::create_dir_all(lib.join("PC/A b")).unwrap();
        std::fs::write(lib.join("PC/A b/x.txt"), "hi").unwrap();
        assert_eq!(app.library_file("/PC/A%20b/x.txt").unwrap(), b"hi");
        assert!(app.library_file("../config/config.json").is_err());
        let (code, part, range) = app.library_range("PC/A b/x.txt", Some("bytes=1-")).unwrap();
        assert_eq!((code, part.as_slice(), range.as_deref()), (206, &b"i"[..], Some("bytes 1-1/2")));
        let _ = std::fs::remove_dir_all(&home);
    }
}
