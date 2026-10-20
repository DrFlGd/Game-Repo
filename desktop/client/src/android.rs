// Game Repo Go's Android side (docs/PLAN.md, "Phase 9b design", D97), through its Kotlin
// plugin (desktop/client/plugin, `DevicePlugin`): the pairing token in the Android Keystore,
// ES-DE looked for on every storage volume, All files access, a folder chosen with Android's
// picker, the multicast lock while the library is looked for, and the downloads' foreground
// service, its notification kept up to date and its Pause, Resume and Cancel heard. Each
// plugin call waits for Android's main thread, so none is made from it.

use crate::Go;
use gamerepo_device::app::Secrets;
use gamerepo_device::queue::State;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_gamerepo_device::Android;

fn plugin(app: &AppHandle) -> Android<Wry> {
    app.state::<Android<Wry>>().inner().clone()
}

/// The pairing token, kept with a key the Android Keystore holds (Secrets.kt), and in
/// memory once read: every call to the library needs it, and each unsealing waits for
/// Android's main thread. One at a time, so a read that started before a pairing can't put
/// back what was there before it.
struct Keystore(Android<Wry>, Mutex<HashMap<String, Option<String>>>);

impl Secrets for Keystore {
    fn get(&self, name: &str) -> Option<String> {
        let mut kept = self.1.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = kept.get(name) {
            return v.clone();
        }
        let v: Value = self.0.call("secretGet", json!({ "name": name })).ok()?;
        let value = v["found"].as_bool().unwrap_or(false).then(|| v["value"].as_str().unwrap_or_default().to_string());
        kept.insert(name.to_string(), value.clone());
        value
    }

    fn set(&self, name: &str, value: &str) -> anyhow::Result<()> {
        let mut kept = self.1.lock().unwrap_or_else(|e| e.into_inner());
        kept.remove(name);
        self.0.call::<Value>("secretSet", json!({ "name": name, "value": value })).map_err(anyhow::Error::msg)?;
        kept.insert(name.to_string(), Some(value.to_string()));
        Ok(())
    }

    fn delete(&self, name: &str) -> anyhow::Result<()> {
        let mut kept = self.1.lock().unwrap_or_else(|e| e.into_inner());
        kept.remove(name);
        self.0.call::<Value>("secretDelete", json!({ "name": name })).map_err(anyhow::Error::msg)?;
        kept.insert(name.to_string(), None);
        Ok(())
    }
}

pub fn secrets(app: &AppHandle) -> Box<dyn Secrets> {
    Box::new(Keystore(plugin(app), Mutex::new(HashMap::new())))
}

/// Where ES-DE is looked for: each storage volume that's there (internal storage, SD cards).
fn homes(p: &Android<Wry>) -> Vec<PathBuf> {
    let v: Value = p.call("volumes", json!({})).unwrap_or_default();
    v["volumes"].as_array().into_iter().flatten().filter_map(|x| x["path"].as_str()).map(PathBuf::from).collect()
}

/// All files access (D87): whether it's allowed, and with `ask`, Android's screen for it
/// first. ES-DE is looked for again once it is.
pub fn storage(app: &AppHandle, ask: bool) -> Result<Value, String> {
    let p = plugin(app);
    let v: Value = p.call(if ask { "askStorageAccess" } else { "storageAccess" }, json!({}))?;
    let granted = v["granted"].as_bool().unwrap_or(false);
    if granted && ask {
        app.state::<Go>().app.set_homes(homes(&p));
    }
    Ok(json!({ "needed": true, "granted": granted }))
}

pub fn pick_folder(app: &AppHandle) -> Result<Option<String>, String> {
    let v: Value = plugin(app).call("pickFolder", json!({}))?;
    Ok(v["path"].as_str().filter(|s| !s.is_empty()).map(String::from))
}

pub fn leave(app: &AppHandle) -> Result<(), String> {
    plugin(app).call::<Value>("moveToBack", json!({})).map(|_| ())
}

fn bytes(n: u64) -> String {
    let (n, unit) = match n {
        n if n >= 1 << 30 => (n as f64 / (1u64 << 30) as f64, "GB"),
        n if n >= 1 << 20 => (n as f64 / (1u64 << 20) as f64, "MB"),
        n => (n as f64 / 1024.0, "KB"),
    };
    format!("{n:.1} {unit}")
}

fn games(n: usize) -> String {
    format!("{n} game{}", if n == 1 { "" } else { "s" })
}

/// The notification while the queue has games coming or paused; none once it's done.
fn shown(app: &AppHandle, showing: bool) -> Option<Value> {
    let q = app.state::<Go>().app.queue.clone();
    let busy = q.busy();
    let s = q.shown();
    let paused = s.items.iter().filter(|i| i.state == State::Paused).count();
    // a queue paused from the notification keeps it, with Resume
    if !busy && !(showing && paused > 0) {
        return None;
    }
    let pct = if s.size > 0 { (s.done * 100 / s.size) as i64 } else { -1 };
    let (title, text) = if !busy {
        ("Downloads paused".to_string(), format!("{} paused", games(paused)))
    } else if s.offline {
        ("Waiting for the library".to_string(), format!("{} to come when it can be reached", games(s.left)))
    } else {
        (format!("Downloading {}", games(s.left)), format!("{}% · {} of {}", pct.max(0), bytes(s.done), bytes(s.size)))
    };
    Some(json!({ "running": true, "title": title, "text": text, "percent": if s.offline { -1 } else { pct }, "paused": !busy }))
}

/// ES-DE looked for on the storage volumes; the multicast lock while the library's looked
/// for; the notification kept up to date; its buttons heard.
pub fn start(app: AppHandle) {
    let p = plugin(&app);
    let (a, pp) = (app.clone(), p.clone());
    std::thread::spawn(move || a.state::<Go>().app.set_homes(homes(&pp)));

    let pm = p.clone();
    gamerepo_device::on_browse(Box::new(move |on| {
        let _ = pm.call::<Value>("multicast", json!({ "on": on }));
    }));

    let (a, pn) = (app.clone(), p.clone());
    std::thread::spawn(move || {
        let (mut showing, mut last, mut retry) = (false, Value::Null, Instant::now());
        loop {
            std::thread::sleep(Duration::from_secs(1));
            match shown(&a, showing) {
                Some(v) if v != last || !showing => {
                    if Instant::now() < retry {
                        continue;
                    }
                    match pn.call::<Value>("downloads", v.clone()) {
                        Ok(_) => (showing, last) = (true, v),
                        // Android won't start the service while Game Repo Go is in the
                        // background: tried again now and then
                        Err(_) => retry = Instant::now() + Duration::from_secs(30),
                    }
                }
                None if showing => {
                    let _ = pn.call::<Value>("downloads", json!({ "running": false }));
                    (showing, last) = (false, Value::Null);
                }
                _ => {}
            }
        }
    });

    std::thread::spawn(move || loop {
        let Ok(v) = p.call::<Value>("waitAction", json!({})) else {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };
        let go = app.state::<Go>().app.clone();
        let _ = match v["action"].as_str().unwrap_or_default() {
            "pause" => go.call("pause", &json!({})),
            "resume" => go.call("resume", &json!({})),
            "cancel" => {
                let ids: Vec<u64> = go.queue.shown().items.iter().filter(|i| i.state != State::Done).map(|i| i.id).collect();
                go.call("cancel", &json!({ "ids": ids }))
            }
            _ => Ok(Value::Null),
        };
    });
}
