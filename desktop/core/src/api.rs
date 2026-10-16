//! The app's commands, behind one entry point ([`App::call`]) so the Tauri app
//! and `gamerepo-cli serve` (the tests' stand-in for the window) share them exactly.

use crate::config::{AppConfig, Prefs};
use crate::index::{self, Index};
use crate::library::{self, Library};
use anyhow::{anyhow, Result};
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
    #[allow(dead_code)] // the scans of Phase 1 report through it
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
