//! Game Repo Server (docs/PLAN.md, "Phase 9a design", D89–D95): a library served to
//! devices on the home network. It opens the library read-only and keeps an index of its
//! own in its data folder (D90), announces itself with DNS-SD (D91), pairs devices with a
//! six-digit code and answers only them, over HTTPS with its own certificate (D91), gives
//! them the catalogue (D92), plans laid out by the export's engine (D93) and the plans'
//! files, resumable (D95).
//!
//! Game Repo (or `gamerepo-cli serve-devices`) starts it and talks to it on a control
//! channel: plain HTTP on 127.0.0.1 only, each request carrying the secret in
//! `server.json` (D89).

pub mod catalogue;
pub mod control;
pub mod devices;
pub mod discovery;
pub mod files;
pub mod http;
pub mod made;
pub mod plans;
pub mod tls;

use anyhow::{anyhow, bail, Context, Result};
use devices::{Device, Devices};
use gamerepo_core::index::{self, Index};
use gamerepo_core::library::Library;
use gamerepo_core::{browse, dat, meta, roots, scan};
use gamerepo_wire::api::{self, Holding, Identified, Info, PairAsk, PlanAsk};
use http::{Request, Response};
use made::{Hashes, Made, Maker};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Downloads at once per device, at most (D95).
pub use api::DOWNLOADS;
/// Connections at once, at most.
const CONNECTIONS: usize = 64;
/// With nothing to tell it, the server looks at the library again this often (folders on
/// network drives don't report changes).
const LOOK_AGAIN: Duration = Duration::from_secs(30 * 60);

pub struct Options {
    pub library: PathBuf,
    /// The server's data folder: its index, certificate, devices and made files
    pub data: PathBuf,
    /// The HTTPS port devices use (0: any free one, for tests)
    pub port: u16,
    /// The control channel's port on 127.0.0.1 (0: any free one)
    pub control_port: u16,
    /// Game Repo's own index of the library, copied for the first index when there's none
    pub index_from: Option<PathBuf>,
    /// Announce the server on the network (DNS-SD)
    pub announce: bool,
    /// Watch the folders with games on local drives for changes
    pub watch: bool,
    /// The made files kept, at most, in bytes
    pub made_limit: u64,
}

impl Default for Options {
    fn default() -> Self {
        Self { library: PathBuf::new(), data: PathBuf::new(), port: api::PORT, control_port: 0, index_from: None, announce: true, watch: true, made_limit: 20 << 30 }
    }
}

/// The server's data folder on this computer (`%LOCALAPPDATA%\Game Repo Server\` on Windows).
pub fn default_data() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Game Repo Server")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_default()
            .join("game-repo-server")
    }
}

/// What `server.json` says about a running server.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Running {
    pub pid: u32,
    pub port: u16,
    pub control: u16,
    pub secret: String,
    pub library: String,
    pub version: String,
}

/// How the server is, as Settings shows it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub server: String,
    pub library: String,
    pub library_name: String,
    pub version: String,
    pub port: u16,
    /// The address devices on the network see it at
    pub address: Option<String>,
    pub fingerprint: String,
    /// The first index is made and the catalogue can be browsed
    pub ready: bool,
    /// What it's doing
    pub doing: String,
    pub error: Option<String>,
    pub devices: usize,
    /// The code being shown, and the seconds it has left
    pub pairing: Option<(String, u64)>,
    pub announced: bool,
}

#[derive(Default)]
struct Wake {
    /// Look at everything again (None paths), or only these
    all: bool,
    paths: Vec<PathBuf>,
    asked: bool,
}

struct Shared {
    lib: Library,
    lib_id: String,
    data: PathBuf,
    index_path: PathBuf,
    info: Info,
    fingerprint: String,
    port: u16,
    devices: Mutex<Devices>,
    maker: Arc<Maker>,
    hashes: Arc<Hashes>,
    stop: Arc<AtomicBool>,
    ready: AtomicBool,
    doing: Mutex<String>,
    error: Mutex<Option<String>>,
    wake: (Mutex<Wake>, Condvar),
    downloads: Mutex<HashMap<String, usize>>,
    connections: AtomicUsize,
    announced: AtomicBool,
}

pub struct Server {
    sh: Arc<Shared>,
    control_port: u16,
    threads: Vec<std::thread::JoinHandle<()>>,
    announced: Option<discovery::Announced>,
    _watcher: Option<gamerepo_core::watch::Watcher>,
}

/// Is this address on the home network (D86)? Private IPv4 ranges, link-local, loopback,
/// IPv6 unique-local and link-local.
pub fn is_home(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => a.is_private() || a.is_link_local() || a.is_loopback(),
        IpAddr::V6(a) => match a.to_ipv4_mapped() {
            Some(v4) => is_home(IpAddr::V4(v4)),
            None => a.is_loopback() || (a.segments()[0] & 0xfe00) == 0xfc00 || (a.segments()[0] & 0xffc0) == 0xfe80,
        },
    }
}

/// The address this computer has on its network (the one its traffic leaves from).
pub fn local_address() -> Option<IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.168.255.255:9").ok()?;
    s.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_unspecified())
}

/// The server's own settings, kept from its first start: its id.
fn server_id(data: &Path) -> Result<String> {
    let p = data.join("settings.json");
    let mut v: Value = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({}));
    if let Some(id) = v["id"].as_str().filter(|s| !s.is_empty()) {
        return Ok(id.to_string());
    }
    let id = tls::random_hex(8);
    v["id"] = json!(id);
    tls::write_new(&p, &serde_json::to_vec_pretty(&v)?)?;
    Ok(id)
}

/// The first index: Game Repo's copied when it's there and of this version (SQLite's own
/// copy, safe while Game Repo writes to it), else none (it's made from the library).
fn copy_index(from: &Path, to: &Path) -> Result<()> {
    let src = rusqlite::Connection::open_with_flags(from, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    src.busy_timeout(Duration::from_secs(10))?;
    let v: u32 = src.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if v != index::schema_version() {
        bail!("Game Repo's index is of another version ({v}, this server's {})", index::schema_version());
    }
    if let Some(d) = to.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = to.with_extension("copying");
    let _ = std::fs::remove_file(&tmp);
    src.execute("VACUUM INTO ?1", [tmp.to_string_lossy()])?;
    std::fs::rename(&tmp, to)?;
    Ok(())
}

impl Shared {
    fn index(&self) -> Result<Index> {
        let ix = Index::open(&self.index_path, &self.lib_id)?;
        Ok(ix)
    }

    fn set_doing(&self, s: &str) {
        *self.doing.lock().unwrap() = s.to_string();
    }

    /// Bring the index and the catalogue up to date: the files (all, or these), the DATs,
    /// the game folders, the Library's rows, the catalogue.
    fn look(&self, only: Option<Vec<PathBuf>>) -> Result<()> {
        let ix = self.index()?;
        self.set_doing("Reading the library's DATs");
        dat::sync_library(&ix, &self.lib)?;
        self.set_doing("Looking at the folders with games");
        let opts = scan::Options { only, ..Default::default() };
        scan::scan(&ix, &self.lib, &opts, &self.stop, &mut |_| {})?;
        self.set_doing("Reading the games' details");
        meta::sync(&ix, &self.lib)?;
        browse::refresh(&ix, &self.lib)?;
        catalogue::refresh(&ix)?;
        Ok(())
    }

    fn keep(&self) {
        let first = self.look(None);
        self.set_doing("");
        match first {
            Ok(()) => self.ready.store(true, Ordering::SeqCst),
            Err(e) => *self.error.lock().unwrap() = Some(format!("{e:#}")),
        }
        while !self.stop.load(Ordering::Relaxed) {
            let w = {
                let (lock, cv) = &self.wake;
                let mut w = lock.lock().unwrap();
                let until = std::time::Instant::now() + LOOK_AGAIN;
                while !w.asked && !self.stop.load(Ordering::Relaxed) {
                    let left = until.saturating_duration_since(std::time::Instant::now());
                    if left.is_zero() {
                        w.all = true;
                        break;
                    }
                    w = cv.wait_timeout(w, left.min(Duration::from_millis(500))).unwrap().0;
                }
                std::mem::take(&mut *w)
            };
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let only = (!w.all && !w.paths.is_empty()).then_some(w.paths);
            let looked = self.look(only);
            self.set_doing("");
            match looked {
                Ok(()) => {
                    self.ready.store(true, Ordering::SeqCst);
                    *self.error.lock().unwrap() = None;
                }
                Err(e) => *self.error.lock().unwrap() = Some(format!("{e:#}")),
            }
        }
    }

    fn changed(&self, paths: Option<Vec<PathBuf>>) {
        let (lock, cv) = &self.wake;
        let mut w = lock.lock().unwrap();
        match paths {
            Some(p) => w.paths.extend(p),
            None => w.all = true,
        }
        w.asked = true;
        cv.notify_all();
    }

    fn status(&self) -> Status {
        let mut d = self.devices.lock().unwrap();
        Status {
            server: self.info.server.clone(),
            library: self.lib.root().display().to_string(),
            library_name: self.info.library.clone(),
            version: VERSION.into(),
            port: self.port,
            address: local_address().map(|a| format!("{a}:{}", self.port)),
            fingerprint: self.fingerprint.clone(),
            ready: self.ready.load(Ordering::SeqCst),
            doing: self.doing.lock().unwrap().clone(),
            error: self.error.lock().unwrap().clone(),
            devices: d.list().len(),
            pairing: d.pairing().map(|(c, left)| (c, left.as_secs())),
            announced: self.announced.load(Ordering::SeqCst),
        }
    }
}

impl Server {
    /// Start serving: the library opened read-only, the index brought up to date in the
    /// background (the catalogue answers once it is), devices answered at once.
    pub fn start(o: Options) -> Result<Server> {
        let lib = Library::open_read_only(&o.library)?;
        let lib_id = lib.id().ok_or_else(|| anyhow!("The library has no id in _library/library.json."))?;
        std::fs::create_dir_all(&o.data).with_context(|| format!("couldn't make {}", o.data.display()))?;
        if let Some(r) = control::running(&o.data) {
            bail!("Game Repo Server is running already on this computer (process {}, serving {}).", r.pid, r.library);
        }
        let id = server_id(&o.data)?;
        let identity = tls::identity(&o.data)?;
        let index_path = index::path_for(&o.data, &lib_id);
        if !index_path.is_file() {
            if let Some(from) = o.index_from.as_ref().filter(|f| f.is_file()) {
                if let Err(e) = copy_index(from, &index_path) {
                    eprintln!("Game Repo's index wasn't copied ({e:#}): the server makes its own.");
                }
            }
        }
        {
            let ix = Index::open(&index_path, &lib_id)?;
            catalogue::ensure_tables(&ix)?;
        }
        let name = lib.meta()["name"].as_str().map(String::from).unwrap_or_else(|| lib.root().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let listener = TcpListener::bind(("0.0.0.0", o.port)).with_context(|| format!("couldn't listen on port {} (is something else using it?)", o.port))?;
        let port = listener.local_addr()?.port();
        let control_listener = TcpListener::bind(("127.0.0.1", o.control_port))?;
        let control_port = control_listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let sh = Arc::new(Shared {
            info: Info { server: id, library: name, version: VERSION.into(), api: api::API },
            fingerprint: identity.fingerprint.clone(),
            port,
            devices: Mutex::new(Devices::load(&o.data)),
            maker: Maker::new(o.data.join("made"), o.made_limit, stop.clone()),
            hashes: Hashes::new(index_path.clone(), stop.clone()),
            lib,
            lib_id,
            data: o.data.clone(),
            index_path,
            stop: stop.clone(),
            ready: AtomicBool::new(false),
            doing: Mutex::new("Starting".into()),
            error: Mutex::new(None),
            wake: (Mutex::default(), Condvar::new()),
            downloads: Mutex::default(),
            connections: AtomicUsize::new(0),
            announced: AtomicBool::new(false),
        });
        let secret = tls::random_hex(32);
        let mut threads = vec![];
        {
            let sh = sh.clone();
            let config = identity.config.clone();
            listener.set_nonblocking(true)?;
            threads.push(std::thread::Builder::new().name("serve".into()).spawn(move || accept(sh, listener, config))?);
        }
        {
            let sh = sh.clone();
            let secret = secret.clone();
            control_listener.set_nonblocking(true)?;
            threads.push(std::thread::Builder::new().name("control".into()).spawn(move || control::accept(sh, control_listener, secret))?);
        }
        {
            let sh = sh.clone();
            threads.push(std::thread::Builder::new().name("keep".into()).spawn(move || sh.keep())?);
        }
        let running = Running { pid: std::process::id(), port, control: control_port, secret, library: sh.lib.root().display().to_string(), version: VERSION.into() };
        tls::write_new(&o.data.join("server.json"), &serde_json::to_vec_pretty(&running)?)?;
        let announced = if o.announce {
            match discovery::announce(&sh.info, port) {
                Ok(a) => {
                    sh.announced.store(true, Ordering::SeqCst);
                    Some(a)
                }
                Err(e) => {
                    eprintln!("The server couldn't announce itself on the network ({e:#}); devices can be given its address.");
                    None
                }
            }
        } else {
            None
        };
        // the folders with games on local drives, watched (those on network drives are looked at every 30 minutes)
        let watcher = if o.watch {
            let dirs: Vec<PathBuf> = roots::list(&sh.lib).into_iter().map(|r| r.dir).filter(|d| d.is_dir() && !gamerepo_core::watch::is_network(d)).collect();
            let w = sh.clone();
            gamerepo_core::watch::watch(&dirs, &sh.lib.root().join(gamerepo_core::library::APP_DIR), gamerepo_core::watch::SETTLE, move |paths| w.changed(paths)).ok()
        } else {
            None
        };
        Ok(Server { sh, control_port, threads, announced, _watcher: watcher })
    }

    pub fn port(&self) -> u16 {
        self.sh.port
    }

    pub fn control_port(&self) -> u16 {
        self.control_port
    }

    pub fn fingerprint(&self) -> &str {
        &self.sh.fingerprint
    }

    pub fn info(&self) -> &Info {
        &self.sh.info
    }

    pub fn status(&self) -> Status {
        self.sh.status()
    }

    /// Start pairing: the code to show, and the seconds it works for.
    pub fn pair(&self) -> (String, u64) {
        (self.sh.devices.lock().unwrap().start_pairing(), devices::CODE_LIFE.as_secs())
    }

    pub fn devices(&self) -> Vec<Device> {
        self.sh.devices.lock().unwrap().list().to_vec()
    }

    pub fn remove_device(&self, id: &str) -> Result<()> {
        self.sh.devices.lock().unwrap().remove(id)
    }

    /// The library changed (Game Repo finished a job): these paths, or everything.
    pub fn changed(&self, paths: Option<Vec<PathBuf>>) {
        self.sh.changed(paths)
    }

    /// Wait until the catalogue can be browsed and nothing is waiting to be looked at.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let until = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < until {
            let asked = self.sh.wake.0.lock().unwrap().asked;
            if self.sh.ready.load(Ordering::SeqCst) && !asked && self.sh.doing.lock().unwrap().is_empty() {
                return true;
            }
            if self.sh.error.lock().unwrap().is_some() && !self.sh.ready.load(Ordering::SeqCst) {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Asked to stop (by the control channel's `stop`).
    pub fn stopping(&self) -> bool {
        self.sh.stop.load(Ordering::Relaxed)
    }

    /// Run until stopped (by the control channel's `stop`).
    pub fn wait(&self) {
        while !self.sh.stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    pub fn stop(mut self) {
        self.sh.stop.store(true, Ordering::SeqCst);
        self.sh.wake.1.notify_all();
        self.announced.take();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        let p = self.sh.data.join("server.json");
        if control::read_running(&self.sh.data).is_some_and(|r| r.pid == std::process::id() && r.control == self.control_port) {
            let _ = std::fs::remove_file(p);
        }
    }
}

fn accept(sh: Arc<Shared>, listener: TcpListener, config: Arc<rustls::ServerConfig>) {
    while !sh.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((tcp, peer)) => {
                // home network only (D86), whatever the firewall lets through
                if !is_home(peer.ip()) || sh.connections.load(Ordering::SeqCst) >= CONNECTIONS {
                    continue;
                }
                sh.connections.fetch_add(1, Ordering::SeqCst);
                let (sh, config) = (sh.clone(), config.clone());
                let _ = std::thread::Builder::new().name("device".into()).spawn(move || {
                    let _ = connection(&sh, tcp, peer, config);
                    sh.connections.fetch_sub(1, Ordering::SeqCst);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// A download's place among a device's (D95), given back when it's sent.
struct Slot {
    sh: Arc<Shared>,
    device: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(n) = self.sh.downloads.lock().unwrap().get_mut(&self.device) {
            *n = n.saturating_sub(1);
        }
    }
}

fn connection(sh: &Arc<Shared>, tcp: TcpStream, peer: SocketAddr, config: Arc<rustls::ServerConfig>) -> Result<()> {
    tcp.set_nonblocking(false)?;
    tcp.set_nodelay(true)?;
    tcp.set_read_timeout(Some(Duration::from_secs(120)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(120)))?;
    let conn = rustls::ServerConnection::new(config)?;
    let stream = rustls::StreamOwned::new(conn, tcp);
    let mut r = BufReader::with_capacity(64 * 1024, stream);
    let mut ix: Option<Index> = None;
    while !sh.stop.load(Ordering::Relaxed) {
        let req = match http::read_request(&mut r) {
            Ok(Some(q)) => q,
            _ => break,
        };
        let close = req.closes();
        let (resp, slot) = match route(sh, &req, peer, &mut ix) {
            Ok(x) => x,
            Err(e) => (Response::problem(500, format!("{e:#}")), None),
        };
        let w = r.get_mut();
        let sent = http::write_response(w, &resp, req.method == "HEAD", close);
        drop(slot);
        if sent.is_err() || close {
            break;
        }
    }
    let s = r.get_mut();
    s.conn.send_close_notify();
    let _ = s.flush();
    Ok(())
}

fn body<T: for<'a> Deserialize<'a>>(req: &Request) -> std::result::Result<T, Response> {
    serde_json::from_slice(&req.body).map_err(|e| Response::problem(400, format!("That request can't be read: {e}")))
}

const NOT_READY: &str = "The server is still reading the library: ask again in a moment.";

fn route(sh: &Arc<Shared>, req: &Request, peer: SocketAddr, ix: &mut Option<Index>) -> Result<(Response, Option<Slot>)> {
    let path = req.path.as_str();
    let m = req.method.as_str();
    let ok = |r: Response| Ok((r, None));
    match (m, path) {
        ("GET", "/v1/info") => return ok(Response::json(200, &sh.info)),
        ("POST", "/v1/pair") => {
            let ask: PairAsk = match body(req) {
                Ok(a) => a,
                Err(r) => return ok(r),
            };
            let r = sh.devices.lock().unwrap().pair(&ask, &peer.ip().to_string(), &sh.info.server)?;
            return ok(match r {
                Ok(p) => Response::json(200, &p),
                Err(why) => Response::problem(403, why.message()),
            });
        }
        _ => {}
    }
    if !path.starts_with("/v1/") {
        return ok(Response::problem(404, "There's nothing here."));
    }
    // every other request is from a paired device
    let token = req.header("authorization").and_then(|a| a.strip_prefix("Bearer ")).map(str::trim).unwrap_or("");
    let Some(device) = (!token.is_empty()).then(|| sh.devices.lock().unwrap().by_token(token, &peer.ip().to_string())).flatten() else {
        return ok(Response::problem(401, "This device isn't paired with this server (or was removed from it): pair it again."));
    };
    match (m, path) {
        ("PUT", "/v1/device") => {
            let h: Holding = match body(req) {
                Ok(h) => h,
                Err(r) => return ok(r),
            };
            sh.devices.lock().unwrap().set_holding(&device.id, h)?;
            return ok(Response::json(200, &json!({})));
        }
        ("DELETE", "/v1/device") => {
            sh.devices.lock().unwrap().remove(&device.id)?;
            return ok(Response::json(200, &json!({})));
        }
        _ => {}
    }
    if path.starts_with("/v1/files/") && (m == "GET" || m == "HEAD") {
        return files_route(sh, req, &device);
    }
    if !sh.ready.load(Ordering::SeqCst) {
        return ok(Response::problem(503, NOT_READY).with("Retry-After", "5"));
    }
    if ix.is_none() {
        *ix = Some(sh.index()?);
    }
    let ix = ix.as_ref().unwrap();
    let lib = &sh.lib;
    match (m, path) {
        ("GET", "/v1/systems") => ok(Response::json(200, &catalogue::systems(ix, lib)?)),
        ("GET", "/v1/collections") => ok(Response::json(200, &catalogue::collections(ix, lib)?)),
        ("GET", "/v1/games") => {
            let a = catalogue::Ask {
                system: req.param("system").unwrap_or("").into(),
                collection: req.param("collection").unwrap_or("").into(),
                group: req.param("group").map(String::from),
                search: req.param("q").unwrap_or("").into(),
                sort: req.param("sort").unwrap_or("title").into(),
                desc: req.param("desc").is_some_and(|d| d == "1" || d == "true"),
                offset: req.param("offset").and_then(|o| o.parse().ok()).unwrap_or(0),
                limit: req.param("limit").and_then(|o| o.parse().ok()).unwrap_or(catalogue::PAGE),
            };
            match catalogue::games(ix, lib, &a) {
                Ok(p) => ok(Response::json(200, &p)),
                Err(e) if !a.collection.is_empty() => ok(Response::problem(404, format!("{e:#}"))),
                Err(e) => Err(e),
            }
        }
        ("GET", "/v1/changes") => {
            let since: i64 = req.param("since").and_then(|s| s.parse().ok()).unwrap_or(0);
            let offset = req.param("offset").and_then(|o| o.parse().ok()).unwrap_or(0);
            ok(match catalogue::changes(ix, since, offset)? {
                Ok(c) => Response::json(200, &c),
                Err(catalogue::TooOld) => Response::problem(410, "The list has changed too much since then: fetch it again."),
            })
        }
        ("GET", p) if p.starts_with("/v1/games/") => ok(game_details(ix, lib, &p["/v1/games/".len()..])?),
        ("GET", p) if p.starts_with("/v1/pictures/") => ok(picture(sh, ix, req, &p["/v1/pictures/".len()..])?),
        ("POST", "/v1/identify") => {
            let found: Vec<api::Found> = match body(req) {
                Ok(f) => f,
                Err(r) => return ok(r),
            };
            ok(Response::json(200, &identify(ix, &found)?))
        }
        ("POST", "/v1/plans") => {
            let ask: PlanAsk = match body(req) {
                Ok(a) => a,
                Err(r) => return ok(r),
            };
            ok(match plans::plan(ix, lib, &sh.data, &sh.maker, &sh.hashes, &ask)? {
                Ok(p) => Response::json(200, &p),
                Err(why) => Response::problem(404, why),
            })
        }
        _ => ok(Response::problem(404, "There's nothing here.")),
    }
}

/// A plan's file, or its SHA-1 (`?sha1`, waited for while it's read or made).
fn files_route(sh: &Arc<Shared>, req: &Request, device: &Device) -> Result<(Response, Option<Slot>)> {
    let rest = &req.path["/v1/files/".len()..];
    let wait = Duration::from_secs(20);
    let pending = || Response::json(202, &json!({ "pending": true })).with("Retry-After", "2");
    let (path, sha1) = if let Some(id) = rest.strip_prefix("made/") {
        let m = if req.has_param("sha1") { sh.maker.wait(id, wait) } else { sh.maker.status(id) };
        match m {
            Made::Ready { path, sha1, .. } => (path, Some(sha1)),
            Made::Pending => return Ok((pending(), None)),
            Made::Failed(e) => return Ok((Response::problem(500, format!("That file couldn't be made: {e}")), None)),
            Made::Unknown => return Ok((Response::problem(404, "That file isn't being made now: ask for the plan again."), None)),
        }
    } else {
        let roots = roots::list(&sh.lib);
        match files::resolve(&sh.lib, &roots, rest) {
            Some(p) if p.is_file() => (p, None),
            _ => return Ok((Response::problem(404, "That file isn't one the server gives, or isn't in the library now: ask for the plan again."), None)),
        }
    };
    if req.has_param("sha1") {
        let h = match sha1 {
            Some(h) => Some(h),
            None => sh.hashes.wait(&path, wait)?,
        };
        return Ok((match h {
            Some(h) => Response::json(200, &json!({ "sha1": h, "etag": files::etag(&path) })),
            None => pending(),
        }, None));
    }
    // at most four downloads at once per device (D95)
    let slot = if req.method == "GET" {
        let mut d = sh.downloads.lock().unwrap();
        let n = d.entry(device.id.clone()).or_default();
        if *n >= DOWNLOADS {
            return Ok((Response::problem(429, "Four files are being sent to this device already: ask again when one is done.").with("Retry-After", "1"), None));
        }
        *n += 1;
        Some(Slot { sh: sh.clone(), device: device.id.clone() })
    } else {
        None
    };
    Ok((files::send(req, &path, "application/octet-stream"), slot))
}

/// One game's details: its details, files, discs and each media kind with its size.
fn game_details(ix: &Index, lib: &Library, key: &str) -> Result<Response> {
    let Some(g) = catalogue::game(ix, key)? else { return Ok(Response::problem(404, format!("There's no game {key} here now."))) };
    let keys = catalogue::keys_of(ix, key)?.unwrap_or_default();
    let v = browse::game(ix, lib, &keys[0])?;
    let mut files = vec![];
    for k in &keys {
        let d = if k == &keys[0] { v.clone() } else { browse::game(ix, lib, k)? };
        for f in d["files"].as_array().into_iter().flatten() {
            let name = f["path"].as_str().unwrap_or("").rsplit(['/', '\\']).next().unwrap_or("").to_string();
            files.push(json!({ "name": name, "size": f["size"], "roms": f["roms"] }));
        }
    }
    let media: Vec<Value> = v["media"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(kind, rel)| {
            let p = lib.resolve(rel.as_str()?).ok()?;
            Some(json!({ "kind": kind, "size": std::fs::metadata(p).ok()?.len() }))
        })
        .collect();
    Ok(Response::json(200, &json!({
        "game": g,
        "details": v["details"],
        "discs": if keys.len() > 1 { json!(keys) } else { json!([]) },
        "files": files,
        "media": media,
        "addons": v["addons"]["items"].as_array().into_iter().flatten().map(|a| a["name"].clone()).collect::<Vec<_>>(),
    })))
}

/// A game's picture of a kind, as a thumbnail (D43's, cached in the server's data folder).
fn picture(sh: &Arc<Shared>, ix: &Index, req: &Request, rest: &str) -> Result<Response> {
    let Some((key, kind)) = rest.rsplit_once('/') else { return Ok(Response::problem(404, "No picture.")) };
    let size: u32 = req.param("size").and_then(|s| s.parse().ok()).unwrap_or(256);
    let Some(keys) = catalogue::keys_of(ix, key)? else { return Ok(Response::problem(404, format!("There's no game {key} here now."))) };
    let folder: Option<String> = ix.conn().query_row("SELECT folder FROM library WHERE key = ?1", [&keys[0]], |r| r.get(0)).ok().flatten();
    let Some(folder) = folder else { return Ok(Response::problem(404, "That game has no pictures.")) };
    let dir = sh.lib.resolve(&format!("{folder}/media"))?;
    let file = std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.split_once('.').is_some_and(|(k, _)| k == kind));
    let Some(file) = file else { return Ok(Response::problem(404, format!("That game has no {kind} picture."))) };
    let thumb = match browse::thumb(&sh.lib, &sh.data, &format!("{folder}/media/{file}"), size) {
        Ok(t) => t,
        Err(e) => return Ok(Response::problem(404, format!("{e:#}"))),
    };
    Ok(files::send(req, &thumb, "image/jpeg").with("Cache-Control", "max-age=3600"))
}

/// The library's game for each file a device found (C18): by its ROMs' CRC32s and sizes (all
/// of a game's, for a ZIP), or a CHD's SHA-1.
fn identify(ix: &Index, found: &[api::Found]) -> Result<Vec<Identified>> {
    let conn = ix.conn();
    let mut out = vec![];
    for f in found {
        let mut candidates: Vec<(i64, String, String)> = vec![];
        if let Some(sha1) = &f.chd_sha1 {
            candidates = conn
                .prepare("SELECT g.id, g.name, d.platform FROM dat_roms r JOIN dat_games g ON g.id = r.game_id JOIN dats d ON d.id = g.dat_id WHERE d.current = 1 AND r.sha1 = ?1")?
                .query_map([sha1.to_lowercase()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
        } else if let Some((crc, size)) = f.crcs.first() {
            let all: Vec<(String, i64)> = f.crcs.iter().map(|(c, s)| (c.to_lowercase(), *s as i64)).collect();
            let firsts: Vec<(i64, String, String)> = conn
                .prepare("SELECT DISTINCT g.id, g.name, d.platform FROM dat_roms r JOIN dat_games g ON g.id = r.game_id JOIN dats d ON d.id = g.dat_id WHERE d.current = 1 AND r.crc32 = ?1 AND r.size = ?2")?
                .query_map(rusqlite::params![crc.to_lowercase(), *size as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?;
            // a game holding every ROM found, one with exactly those first
            let mut scored = vec![];
            for c in firsts {
                let roms: Vec<(String, i64)> = conn
                    .prepare("SELECT lower(COALESCE(crc32, '')), COALESCE(size, -1) FROM dat_roms WHERE game_id = ?1 AND disk = 0")?
                    .query_map([c.0], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<rusqlite::Result<_>>()?;
                if all.iter().all(|x| roms.contains(x)) {
                    scored.push((roms.len() != all.len(), c));
                }
            }
            scored.sort_by(|a, b| a.0.cmp(&b.0));
            candidates = scored.into_iter().map(|(_, c)| c).collect();
        }
        let mut game = None;
        for (_, name, platform) in candidates {
            if let Some(k) = catalogue::game_for(ix, &format!("{platform}/{name}"))? {
                game = Some(k);
                break;
            }
        }
        out.push(Identified { name: f.name.clone(), game });
    }
    Ok(out)
}
