//! The app's commands, behind one entry point ([`App::call`]) so the Tauri app
//! and `gamerepo-cli serve` (the tests' stand-in for the window) share them exactly.

use crate::config::{AppConfig, Prefs};
use crate::index::{self, Index};
use crate::library::{self, Library};
use crate::{audit, dat, makedat, platforms, roots, scan, starter};
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
}

impl Job {
    fn to_json(&self) -> Value {
        json!({ "id": self.id, "label": self.label, "done": self.done, "error": self.error, "result": self.result, "progress": self.progress, "started": self.started })
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
        Ok(Arc::new(Self {
            paths,
            library: RwLock::new(None),
            index: Arc::new(Mutex::new(None)),
            jobs: Mutex::new(vec![]),
            job_seq: Mutex::new(0),
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
        if let Some(l) = self.library.read().unwrap().as_ref() {
            return Ok(l.clone());
        }
        let Some(path) = self.config().library else {
            return Err(NO_LIBRARY.into());
        };
        self.open_library(&path).map_err(e2s)
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
        let job = self.start_job(label);
        let (app, id, cancel) = (self.clone(), job.id.clone(), job.cancel.clone());
        tokio::spawn(async move {
            let (a, jid) = (app.clone(), id.clone());
            let r = tokio::task::spawn_blocking(move || work(a, jid, cancel)).await;
            let r = match r {
                Ok(r) => r.map_err(e2s),
                Err(e) => Err(e.to_string()),
            };
            app.job_done(&id, r);
        });
        json!({ "job": job.id })
    }

    /// A connection of a job's own to the library's index, so a long scan or import
    /// doesn't hold up the page's reads (SQLite's WAL lets them run side by side).
    fn job_index(data_dir: &Path, lib: &Library) -> Result<Index> {
        let id = lib.id().ok_or_else(|| anyhow!("This library has no id in _library/library.json, so its index can't be found."))?;
        Index::open(&index::path_for(data_dir, &id), &id)
    }

    /// Refuse to start a job while one with the same label is running.
    fn not_running(&self, label: &str) -> Result<(), String> {
        if self.jobs.lock().unwrap().iter().any(|j| !j.done && j.label == label) {
            return Err(format!("{label} is already running."));
        }
        Ok(())
    }

    // ------------------------------------------------------------ commands

    pub async fn call(self: &Arc<Self>, cmd: &str, args: Value) -> Result<Reply, String> {
        let j = |v: Value| Ok(Reply::Json(v));
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
                j(json!(roots::list(&lib).iter().map(roots::Root::to_json).collect::<Vec<_>>()))
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
                j(roots::update(&lib, id, path.as_deref(), platform).map_err(e2s)?.to_json())
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
                cfg.save(&self.config_path()).map_err(e2s)?;
                j(json!(cfg.scan))
            }
            "scan" => {
                let lib = self.library()?;
                lib.writable().map_err(e2s)?;
                self.not_running("Scanning")?;
                let settings = self.config().scan;
                let opts = scan::Options { full: args["full"].as_bool().unwrap_or(settings.full), readers: settings.readers };
                let data_dir = self.paths.data_dir.clone();
                j(self.spawn_job("Scanning", move |app, id, cancel| {
                    let ix = Self::job_index(&data_dir, &lib)?;
                    scan::scan(&ix, &lib, &opts, &cancel, &mut |p| app.job_progress(&id, p))
                }))
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
                    let mut q = conn.prepare("SELECT DISTINCT platform FROM dats WHERE current = 1")?;
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
                    let Some(dat_id) = dat_id else {
                        return Ok(json!({ "summary": null, "platforms": [], "dats": [] }));
                    };
                    let summary = audit::summary(conn, dat_id)?;
                    let platform = summary["dat"]["platform"].as_str().unwrap_or("").to_string();
                    let mut q = conn.prepare("SELECT id, name, version FROM dats WHERE platform = ?1 AND current = 1 ORDER BY name")?;
                    let dats: Vec<Value> = q
                        .query_map([&platform], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "name": r.get::<_, String>(1)?, "version": r.get::<_, String>(2)? })))?
                        .collect::<rusqlite::Result<_>>()?;
                    let plats: Vec<Value> = with_dats.iter().map(|p| json!({ "id": p, "name": platforms::name_of(p) })).collect();
                    Ok(json!({ "summary": summary, "platforms": plats, "dats": dats }))
                })
                .await?)
            }
            "audit_list" => {
                let lib = self.library()?;
                let dat_id = args["dat"].as_i64().ok_or("missing dat")?;
                let tab = arg(&args, "tab").map_err(e2s)?.to_string();
                let search = args["search"].as_str().unwrap_or("").to_string();
                let (offset, limit) = (args["offset"].as_i64().unwrap_or(0).max(0), args["limit"].as_i64().unwrap_or(100));
                j(self.with_index(move |ix| audit::list(ix, &lib, dat_id, &tab, &search, offset, limit)).await?)
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

    /// What a `library://` path is: a file in the open library.
    fn served_path(&self, rel: &str) -> Result<PathBuf> {
        let rel = percent_decode(rel.trim_start_matches('/'));
        let lib = self.library().map_err(|e| anyhow!(e))?;
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
        assert_eq!(call(&app, "scan_settings", json!({})).await, json!({ "full": false, "readers": 3 }));
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
