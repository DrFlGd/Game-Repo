// Game Repo Go's updates on Android (docs/PLAN.md, "In-app updates", D139): the same
// commands and panel as on Windows and the Steam Deck (gamerepo-update, D134), with Android's
// own pieces. The Kotlin plugin (Updates.kt) fetches the update file and the APK with
// Android's HTTPS; the APK is checked here against the public key built in and the version
// it was signed for (gamerepo_device::update); then Android's package installer asks the
// person, checks it's signed with the same key as the copy installed, and replaces Game Repo
// Go, which they open again. It checks a few seconds after it starts unless Settings says
// not to. Each plugin call waits for Android's main thread, so none is made from it.

use gamerepo_device::update::{self, Release};
use gamerepo_device::wire::RELEASES;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_gamerepo_device::Android;

const TITLE: &str = "Game Repo Go";

/// The update file: the newest release's, or the one the emulator's test serves (its debug
/// build is made with GAMEREPO_UPDATE_FILE).
fn endpoint() -> String {
    match option_env!("GAMEREPO_UPDATE_FILE") {
        Some(f) if !f.is_empty() => f.to_string(),
        _ => format!("https://github.com/{RELEASES}/releases/latest/download/latest-game-repo-go.json"),
    }
}

struct Updates {
    plugin: Android<Wry>,
    /// updates.json in Game Repo Go's settings folder: whether it checks when it starts
    settings: PathBuf,
    pubkey: String,
    why_not: Option<String>,
    now: Mutex<Now>,
    found: Mutex<Option<Release>>,
}

#[derive(Default)]
struct Now {
    check_at_start: bool,
    checking: bool,
    checked: Option<u64>,
    available: Option<Value>,
    error: Option<String>,
    installing: Option<(u64, Option<u64>)>,
    /// Downloaded and checked: Android's installer is asking
    restarting: bool,
}

pub fn manage(app: &AppHandle) {
    let plugin = app.state::<Android<Wry>>().inner().clone();
    let settings = app.path().app_config_dir().unwrap_or_default().join("updates.json");
    let check_at_start = checks_at_start(&settings);
    let pubkey = app.config().plugins.0.get("updater").and_then(|u| u.get("pubkey")).and_then(Value::as_str).unwrap_or_default().to_string();
    let why_not = pubkey.trim().is_empty().then(|| format!("Updates aren't set up in this copy of {TITLE}: it was built without the key that signs them."));
    let start = why_not.is_none() && check_at_start;
    app.manage(Updates { plugin, settings, pubkey, why_not, now: Mutex::new(Now { check_at_start, checking: start, ..Default::default() }), found: Mutex::new(None) });
    if start {
        let app = app.clone();
        std::thread::spawn(move || {
            // after Game Repo Go's own start
            std::thread::sleep(Duration::from_secs(3));
            check(&app);
        });
    }
}

/// One of the update commands, answered with how updates stand (as gamerepo-update's).
/// Blocking: from a thread of its own.
pub fn call(app: &AppHandle, cmd: &str, args: &Value) -> Result<Value, String> {
    let u = app.try_state::<Updates>().ok_or("Updates aren't set up.")?;
    match cmd {
        "update_state" => {
            if args["wait"].as_bool() == Some(true) {
                done_checking(&u);
            }
        }
        "update_check" => {
            if let Some(why) = &u.why_not {
                return Err(why.clone());
            }
            if u.now.lock().unwrap().checking {
                done_checking(&u);
            } else {
                check(app);
            }
        }
        "update_install" => install(app)?,
        "update_set" => {
            if let Some(on) = args["check_at_start"].as_bool() {
                save_settings(&u.settings, on).map_err(|e| format!("Couldn't save it: {e}"))?;
                u.now.lock().unwrap().check_at_start = on;
            }
        }
        _ => return Err(format!("unknown command {cmd}")),
    }
    Ok(shown(app))
}

fn shown(app: &AppHandle) -> Value {
    let Some(u) = app.try_state::<Updates>() else { return Value::Null };
    let n = u.now.lock().unwrap();
    json!({
        "title": TITLE,
        "version": app.package_info().version.to_string(),
        "can": u.why_not.is_none(),
        "why_not": u.why_not,
        "check_at_start": n.check_at_start,
        "checking": n.checking,
        "checked": n.checked,
        "available": n.available,
        "error": n.error,
        "installing": n.installing.map(|(done, size)| json!({ "done": done, "size": size })),
        "restarting": n.restarting,
        // Android's installer asks, and Game Repo Go is opened again by hand
        "android": true,
        "releases": format!("https://github.com/{RELEASES}/releases"),
    })
}

fn done_checking(u: &Updates) {
    for _ in 0..450 {
        if !u.now.lock().unwrap().checking {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn tell(app: &AppHandle) {
    let _ = app.emit("go-changed", "update");
}

fn check(app: &AppHandle) {
    let u = app.state::<Updates>();
    {
        let mut n = u.now.lock().unwrap();
        n.checking = true;
        n.error = None;
    }
    let version = app.package_info().version.to_string();
    let got = u
        .plugin
        .call::<Value>("updateFetch", json!({ "url": endpoint() }))
        .map_err(|e| format!("Couldn't check for updates: {e}"))
        .and_then(|v| update::release(v["text"].as_str().unwrap_or_default(), &format!("android-{}", std::env::consts::ARCH), &version));
    {
        let mut n = u.now.lock().unwrap();
        n.checking = false;
        n.checked = Some(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default());
        match &got {
            Ok(r) => n.available = r.as_ref().map(|r| json!({ "version": r.version, "notes": r.notes, "date": r.date })),
            Err(e) => n.error = Some(e.clone()),
        }
    }
    if let Ok(r) = got {
        *u.found.lock().unwrap() = r;
    }
    tell(app);
}

/// Downloads the update found, with its progress shown, checks it, and hands it to
/// Android's installer. The page follows it with `update_state`.
fn install(app: &AppHandle) -> Result<(), String> {
    let u = app.state::<Updates>();
    let r = u.found.lock().unwrap().clone().ok_or("Check for updates first.")?;
    {
        let mut n = u.now.lock().unwrap();
        if n.installing.is_some() || n.restarting {
            return Ok(());
        }
        n.installing = Some((0, None));
        n.error = None;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let u = app.state::<Updates>();
        let failed = match download(&u, &r).and_then(|path| {
            let data = std::fs::read(&path).map_err(|e| format!("The update can't be read: {e}"))?;
            // one that doesn't match isn't kept
            update::verify(&data, &r.signature, &u.pubkey, &r.version).inspect_err(|_| {
                let _ = std::fs::remove_file(&path);
            })?;
            Ok(path)
        }) {
            Ok(path) => {
                {
                    let mut n = u.now.lock().unwrap();
                    n.installing = None;
                    n.restarting = true;
                }
                tell(&app);
                // answered once the person says no (said yes, Android replaces Game Repo Go)
                match u.plugin.call::<Value>("updateInstall", json!({ "path": path })) {
                    Ok(_) => String::new(),
                    Err(e) => e,
                }
            }
            Err(e) => e,
        };
        {
            let mut n = u.now.lock().unwrap();
            n.installing = None;
            n.restarting = false;
            n.error = (!failed.is_empty()).then_some(failed);
        }
        tell(&app);
    });
    Ok(())
}

/// The APK, into Game Repo Go's cache, with its progress read as it comes.
fn download(u: &Updates, r: &Release) -> Result<String, String> {
    let (p, url) = (u.plugin.clone(), r.url.clone());
    let fetching = std::thread::spawn(move || p.call::<Value>("updateDownload", json!({ "url": url })));
    while !fetching.is_finished() {
        std::thread::sleep(Duration::from_millis(400));
        if let Ok(v) = u.plugin.call::<Value>("updateProgress", json!({})) {
            let size = v["size"].as_i64().filter(|s| *s > 0).map(|s| s as u64);
            u.now.lock().unwrap().installing = Some((v["done"].as_u64().unwrap_or(0), size));
        }
    }
    let v = fetching.join().map_err(|_| "The download stopped.".to_string())?.map_err(|e| format!("The update didn't download: {e}"))?;
    v["path"].as_str().map(String::from).ok_or_else(|| "The update didn't download.".to_string())
}

fn checks_at_start(settings: &Path) -> bool {
    let v: Value = std::fs::read(settings).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    v["check_at_start"].as_bool().unwrap_or(true)
}

fn save_settings(settings: &Path, check_at_start: bool) -> std::io::Result<()> {
    if let Some(dir) = settings.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(settings, serde_json::to_vec_pretty(&json!({ "check_at_start": check_at_start }))?)
}
