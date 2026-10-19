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
use std::sync::{Arc, Mutex, RwLock};

/// Where the app keeps things on this computer.
#[derive(Clone, Debug)]
pub struct AppPaths {
    /// config.json and prefs.json
    pub config_dir: PathBuf,
    /// the index of each library (index/<library id>.sqlite) and caches
    pub data_dir: PathBuf,
    /// how the page reaches library files: "library://localhost/", "/library/"...
    pub library_url: String,
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

/// Clears the running change when its job ends, however it ends.
struct ChangeDone(Arc<App>);

impl Drop for ChangeDone {
    fn drop(&mut self) {
        *self.0.change.lock().unwrap() = None;
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
}

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
        let mut read = self.systems_read.lock().unwrap();
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
        *self.index.lock().unwrap() = None;
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
        let mut seq = self.job_seq.lock().unwrap();
        *seq += 1;
        let job = Job {
            id: format!("job{}", *seq),
            label: label.into(),
            started: library::now(),
            ..Default::default()
        };
        let mut jobs = self.jobs.lock().unwrap();
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
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
            j.progress = progress;
        }
    }

    fn job_done(&self, id: &str, r: Result<Value, String>) {
        if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == id) {
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
            if let Some(j) = self.jobs.lock().unwrap().iter_mut().find(|j| j.id == job.id) {
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
        if self.jobs.lock().unwrap().iter().any(|j| !j.done && j.label == label) {
            return Err(format!("{label} is already running."));
        }
        Ok(())
    }

    fn scanning(&self) -> bool {
        self.jobs.lock().unwrap().iter().any(|j| !j.done && j.label == SCANNING)
    }

    /// Mark the change `id` as running (one at a time, and not during a scan); the
    /// job holds the returned guard, which clears it when the job ends.
    fn begin_change(self: &Arc<Self>, id: &str) -> Result<ChangeDone, String> {
        if self.scanning() {
            return Err("A scan is running: wait for it to finish first.".into());
        }
        let mut c = self.change.lock().unwrap();
        if c.is_some() {
            return Err("Another change is running: wait for it to finish first.".into());
        }
        *c = Some(id.to_string());
        Ok(ChangeDone(self.clone()))
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
            let mut w = self.watched.lock().unwrap();
            if w.as_deref() == want.as_ref().map(|w| w.0.as_str()) {
                return;
            }
            *w = want.as_ref().map(|w| w.0.clone());
        }
        *self.watcher.lock().unwrap() = None;
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
                Ok(w) if a.watched.lock().unwrap().as_deref() == Some(key.as_str()) => *a.watcher.lock().unwrap() = Some(w),
                Ok(_) => {}
                Err(e) => eprintln!("can't watch the folders with games: {e}"),
            }
        });
    }

    fn changed_outside(self: &Arc<Self>, paths: Option<Vec<PathBuf>>) {
        {
            let mut c = self.changed.lock().unwrap();
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
        if self.scanning() || self.change.lock().unwrap().is_some() {
            return;
        }
        let Some(c) = self.changed.lock().unwrap().take() else { return };
        let Ok(lib) = self.library() else { return };
        let settings = self.config().scan;
        if lib.writable().is_err() || !settings.watch {
            return;
        }
        let only = if c.all { None } else { Some(c.paths) };
        self.start_scan(lib, scan::Options { full: settings.full, readers: settings.readers, only }, true);
    }

    // ------------------------------------------------------------ commands

    pub async fn call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
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
        if ["layout", "bios", "magazine"].iter().any(|p| cmd.starts_with(p)) {
            return self.device_call(cmd, args).await;
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
                if self.change.lock().unwrap().is_some() {
                    return Err("A change is running: scan when it's finished.".into());
                }
                let settings = self.config().scan;
                let opts = scan::Options { full: args["full"].as_bool().unwrap_or(settings.full), readers: settings.readers, only: None };
                // everything is read, so what the watcher saw is covered
                *self.changed.lock().unwrap() = None;
                j(self.start_scan(lib, opts, false))
            }

            // ---------------------------------------------------- DATs
            "dats" => {
                let lib = self.library()?;
                j(self.with_index(move |ix| {
                    dat::sync_library(ix, &lib)?;
                    dat::list(ix)
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
                j(self.spawn_job("Importing DATs", move |app, id, _| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let (mut imported, mut ask, mut failed) = (vec![], vec![], vec![]);
                    for (i, p) in paths.iter().enumerate() {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        app.job_progress(&id, json!({ "item": i + 1, "items": paths.len(), "name": name }));
                        match dat::import(&ix, &lib, p, &answers) {
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
                let lib = self.library()?;
                let id = args["id"].as_i64().ok_or("missing id")?;
                self.with_index(move |ix| dat::remove(ix, &lib, id)).await?;
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
                j(self.with_index(move |ix| {
                    dat::sync_library(ix, &lib)?;
                    let conn = ix.conn();
                    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1 AND platform != 'bios'")?;
                    let mut with_dats: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
                    with_dats.sort_by_key(|p| platforms::name_of(p));
                    let dat_id = match (dat_arg, platform_arg.as_deref().or(with_dats.first().map(String::as_str))) {
                        (Some(d), _) => {
                            // the DAT picked for a platform stays its DAT
                            if lib.writable().is_ok() {
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
                    let mut q = conn.prepare("SELECT id, name, version FROM dats WHERE platform = ?1 AND current = 1 ORDER BY name")?;
                    let dats: Vec<Value> = q
                        .query_map([&platform], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "version": r.get::<_, String>(2)? })))?
                        .collect::<rusqlite::Result<_>>()?;
                    Ok(json!({ "summary": summary, "platforms": plats, "dats": dats }))
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
                let jobs = self.jobs.lock().unwrap();
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
                if let Some(x) = self.jobs.lock().unwrap().iter().find(|x| x.id == id) {
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
            "tools" => j(tokio::task::spawn_blocking(crate::tools::list).await.map_err(|e| e.to_string())?),
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
                    "metadata" => "Deleting metadata",
                    _ => "Organising",
                };
                let running = self.begin_change(&plan.id)?;
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
                        // the games brought in are scanned, so the full set has them
                        "bring" => {
                            let only: Vec<PathBuf> = plan.ops.iter().map(|o| PathBuf::from(&o.to)).filter(|p| p.exists()).collect();
                            if !only.is_empty() {
                                let opts = scan::Options { only: Some(only), ..Default::default() };
                                scan::scan(&ix, &lib, &opts, &cancel, &mut |p| app.job_progress(&jid, p))?;
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
            "change_history" => {
                let lib = self.library()?;
                let running: Vec<String> = self.change.lock().unwrap().iter().cloned().collect();
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
            "change_empty" => {
                // set-aside files of one change (or all), deleted for good
                let lib = self.library()?;
                if self.change.lock().unwrap().is_some() {
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
                let (platform, addon) = (arg(&args, "platform").map_err(e2s)?.to_string(), arg(&args, "addon").map_err(e2s)?.to_string());
                let base = args["base"].as_str().map(String::from);
                let clear = args["clear"].as_bool().unwrap_or(false);
                self.with_index(move |ix| crate::addons::set(ix.conn(), &lib, &platform, &addon, base.as_deref(), clear)).await?;
                j(json!({ "ok": true }))
            }
            "browse_edit" => {
                let lib = self.library()?;
                let (key, field) = (arg(&args, "key").map_err(e2s)?.to_string(), arg(&args, "field").map_err(e2s)?.to_string());
                // "value": null clears it; no "value" takes the edit mark off
                let value = args.get("value").cloned();
                j(self.with_index(move |ix| crate::browse::edit(ix, &lib, &key, &field, value.as_ref())).await?)
            }
            "meta_clear" => {
                let lib = self.library()?;
                let id = id_arg("id")?;
                j(self.with_index(move |ix| meta::clear(ix, &lib, id)).await?)
            }
            // each source's answer, its pictures, deleting and sizes (Phase 6b, D46–D48)
            "meta_pin" => {
                let lib = self.library()?;
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
                j(self
                    .with_index(move |ix| {
                        meta::sync(ix, &lib)?;
                        let plan = crate::sources::delete_plan(ix, &lib, platform.as_deref(), folder.as_deref(), &what)?;
                        organise::save_pending(&data_dir, &plan)?;
                        Ok(plan.summary())
                    })
                    .await?)
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
                crate::sources::set_details_from(&lib, &field, args["source"].as_str()).map_err(e2s)?;
                let label = "Making details again";
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    meta::sync(&ix, &lib)?;
                    let n = crate::sources::remake_all(&ix, &lib, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    crate::browse::refresh(&ix, &lib)?;
                    Ok(json!({ "games": n }))
                }))
            }
            "meta_pick" => {
                let lib = self.library()?;
                let game = id_arg("game")?;
                let (source, id) = (arg(&args, "source").map_err(e2s)?.to_string(), arg(&args, "id").map_err(e2s)?.to_string());
                j(self.with_index(move |ix| scrape::pick(ix, &lib, &cfg, &data_dir, game, &source, &id)).await?)
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
                self.not_running(label)?;
                j(self.spawn_job(label, move |app, jid, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    let r = scrape::run(&ix, &lib, &cfg, &data_dir, &o, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    // the manuals it kept, read for searching
                    docs::read_texts(&ix, &lib, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    Ok(r)
                }))
            }
            "docs_list" => {
                let q = args["query"].as_str().unwrap_or("").to_string();
                let (kind, platform, source) = (args["kind"].as_str().unwrap_or("").to_string(), args["platform"].as_str().unwrap_or("").to_string(), args["source"].as_str().unwrap_or("").to_string());
                let (offset, limit) = (args["offset"].as_i64().unwrap_or(0), args["limit"].as_i64().unwrap_or(100));
                let lib = self.library()?;
                j(self
                    .with_index(move |ix| {
                        if offset == 0 {
                            meta::sync(ix, &lib)?;
                        }
                        docs::search(ix, &q, &kind, &platform, &source, offset, limit)
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
                    if it.status == "look" {
                        it.status = "ready".into();
                    }
                    it.include = true;
                    Ok(json!(it))
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
                j(blocking(Box::new(move || Ok(identify::search(&cfg, &data_dir, &query, &platform, year)))).await?)
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
                j(self
                    .with_index(move |ix| {
                        let m = identify::details(&cfg, &data_dir, &source, &id)?;
                        match (game, platform) {
                            (Some(g), _) => Ok(crate::pcmeta::identify(ix, &lib, &g, &m, retitle)?.to_json(&lib)),
                            (None, Some(p)) => addgame::identify_rom(&lib, &p, &file, &m),
                            _ => bail!("missing game"),
                        }
                    })
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
                        keys.insert(f.to_string(), v.to_string());
                    }
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
                    let plan = addgame::plan(&lib, &files, &platform, &into, &mode, always_copy, &AtomicBool::new(false))?;
                    organise::save_pending(&data_dir, &plan)?;
                    Ok(plan.summary())
                }))
                .await?)
            }
            other => Err(format!("unknown command {other}")),
        }
    }

    /// Export layouts and their editor, and BIOS files (docs/PLAN.md, "Phase 5b design").
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
                let cached = self.layout_sample.lock().unwrap().clone();
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
                *self.layout_sample.lock().unwrap() = Some(sample.clone());
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
                j(self
                    .with_index(move |ix| {
                        let plan = crate::bios::gather_plan(ix, &lib)?;
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
                let always_copy = args["always_copy"].as_bool().unwrap_or(false);
                j(blocking(Box::new(move || {
                    let plan = crate::magazines::plan(&lib, &items, always_copy)?;
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
                    let found = crate::subset::read_folders(&paths, &cancel, &mut |p| app.job_progress(&jid, p))?;
                    if cancel.load(Ordering::Relaxed) {
                        bail!("Stopped.");
                    }
                    app.job_progress(&jid, json!({ "text": "Matching them to your audit DATs" }));
                    let ix = Self::job_index(&data_dir, &lib)?;
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
                        let c = crate::subset::save(ix, &lib, into, &games, made_from)?;
                        match before {
                            Some(b) => crate::history::record(ix, &lib, b, how, None)?,
                            None => crate::history::record(ix, &lib, crate::history::new(&c), "made", None)?,
                        };
                        let w = collection::work_out(ix, &lib, &c)?;
                        Ok(json!({ "collection": c, "summary": w.summary() }))
                    })
                    .await?)
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
        if bare.starts_with("~cover/") || bare.starts_with("~page/") {
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
        if let Some(rest) = rel.strip_prefix("~page/") {
            let (n, issue) = rest.split_once('/').ok_or_else(|| anyhow!("no issue"))?;
            return crate::magazines::page_file(&lib, &self.paths.data_dir, n.parse().unwrap_or(1), issue);
        }
        lib.resolve(&rel)
    }
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
    let mut g = slot.lock().unwrap();
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
        while app.watcher.lock().unwrap().is_none() {
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
        assert!(app.watcher.lock().unwrap().is_none());
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
