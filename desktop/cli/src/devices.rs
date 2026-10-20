//! `serve-devices` runs Game Repo Server, or asks the one running; `device` stands in for a
//! device's client with a folder holding `ES-DE/` and `ROMs/` (docs/PLAN.md, "Phase 9a
//! design").

use crate::{bytes, Args};
use anyhow::{bail, Context, Result};
use gamerepo_core::config::{app_dirs, AppConfig};
use gamerepo_device::{self as device, AddOptions, Device};
use gamerepo_serve::{control, Options, Server};
use gamerepo_wire::api::{self, Collection, Page, System};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

fn switch(a: &mut Args, name: &str) -> bool {
    let had = a.rest.iter().any(|x| x == name);
    a.rest.retain(|x| x != name);
    had
}

fn number<T: std::str::FromStr>(a: &mut Args, name: &str) -> Result<Option<T>> {
    a.flag(name).map(|v| v.parse::<T>().map_err(|_| anyhow::anyhow!("{name} is a number, not {v}"))).transpose()
}

fn list(v: Option<String>) -> Vec<String> {
    v.map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// A query's value, every byte but letters, digits and `-._~` escaped.
fn query_value(s: &str) -> String {
    s.bytes().map(|c| if c.is_ascii_alphanumeric() || b"-._~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect()
}

fn show(v: &impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

pub async fn serve_devices(mut a: Args) -> Result<()> {
    let data = a.flag("--data").map(PathBuf::from).unwrap_or_else(gamerepo_serve::default_data);
    let json_out = switch(&mut a, "--json");
    let sub = if a.rest.first().is_some_and(|x| !x.starts_with("--")) { a.rest.remove(0) } else { String::new() };
    match sub.as_str() {
        "" | "run" => run(a, data).await,
        "status" => {
            let v = control::call(&data, "GET", "/status", None)?;
            if json_out {
                return show(&v);
            }
            let st: gamerepo_serve::Status = serde_json::from_value(v)?;
            println!("Game Repo Server {} serving {} ({})", st.version, st.library_name, st.library);
            println!("at {}{}", st.address.clone().unwrap_or_else(|| format!("127.0.0.1:{}", st.port)), if st.announced { ", announced on the network" } else { "" });
            println!("certificate {}", st.fingerprint);
            println!("{}", if st.ready { "ready".to_string() } else { format!("reading the library: {}", st.doing) });
            if let Some(e) = st.error {
                println!("problem: {e}");
            }
            println!("{} device{} paired", st.devices, if st.devices == 1 { "" } else { "s" });
            if let Some((code, secs)) = st.pairing {
                println!("pairing code {code} ({secs} s left)");
            }
            Ok(())
        }
        "pair" => {
            let v = control::call(&data, "POST", "/pair", None)?;
            if json_out {
                return show(&v);
            }
            println!("{}", v["code"].as_str().unwrap_or(""));
            eprintln!("Enter this code on the device within {} minutes.", v["seconds"].as_u64().unwrap_or(0) / 60);
            Ok(())
        }
        "unpair" => {
            control::call(&data, "DELETE", "/pair", None)?;
            println!("No device is being paired now.");
            Ok(())
        }
        "devices" => {
            let v = control::call(&data, "GET", "/devices", None)?;
            if json_out {
                return show(&v);
            }
            for d in v.as_array().into_iter().flatten() {
                let h = &d["holding"];
                println!(
                    "{}  {} ({})  {} games, {}, {} free  last seen from {}",
                    d["id"].as_str().unwrap_or(""), d["name"].as_str().unwrap_or(""), d["kind"].as_str().unwrap_or(""),
                    h["games"].as_u64().unwrap_or(0), bytes(h["size"].as_u64().unwrap_or(0)), bytes(h["free"].as_u64().unwrap_or(0)),
                    d["address"].as_str().unwrap_or("")
                );
            }
            Ok(())
        }
        "remove" => {
            let id = a.rest.first().context("serve-devices remove <device id> (serve-devices devices lists them)")?;
            control::call(&data, "DELETE", &format!("/devices/{}", query_value(id)), None)?;
            println!("Removed: that device has to pair again to be answered.");
            Ok(())
        }
        "changed" => {
            let body = if a.rest.is_empty() { json!({}) } else { json!({ "paths": a.rest }) };
            control::call(&data, "POST", "/changed", Some(&body))?;
            Ok(())
        }
        "stop" => {
            control::call(&data, "POST", "/stop", None)?;
            println!("Stopping.");
            Ok(())
        }
        other => bail!("unknown serve-devices command {other} (gamerepo-cli --help lists them)"),
    }
}

/// Serve the library until stopped (Ctrl+C, or `serve-devices stop`).
async fn run(mut a: Args, data: PathBuf) -> Result<()> {
    let (config_dir, app_data) = match a.flag("--home") {
        Some(h) => (PathBuf::from(&h).join("config"), PathBuf::from(&h).join("data")),
        None => app_dirs(),
    };
    // the library asked for, else the one served last, else Game Repo's
    let library = match a.flag("--library") {
        Some(l) => PathBuf::from(l),
        None => gamerepo_serve::last_library(&data)
            .or_else(|| AppConfig::load(&config_dir.join("config.json")).library)
            .context("No library is open in Game Repo: give --library <dir>.")?,
    };
    let port = number(&mut a, "--port")?.unwrap_or(api::PORT);
    let index_from = match a.flag("--index-from") {
        Some(p) => Some(PathBuf::from(p)),
        None => gamerepo_core::Library::open_read_only(&library)?.id().map(|id| gamerepo_core::index::path_for(&app_data, &id)),
    };
    let pair = switch(&mut a, "--pair");
    let o = Options {
        library,
        data,
        port,
        index_from,
        announce: !switch(&mut a, "--no-announce"),
        watch: !switch(&mut a, "--no-watch"),
        parent: switch(&mut a, "--parent"),
        keep: switch(&mut a, "--keep"),
        ..Default::default()
    };
    let s = Server::start(o)?;
    let st = s.status();
    println!("Game Repo Server {} serving {} ({})", st.version, st.library_name, st.library);
    println!("at {}{}", st.address.clone().unwrap_or_else(|| format!("127.0.0.1:{}", st.port)), if st.announced { ", announced on the network" } else { "" });
    println!("certificate {}", st.fingerprint);
    if pair {
        let (code, secs) = s.pair();
        println!("pairing code {code} (for {} minutes)", secs / 60);
    }
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    let (mut doing, mut ready, mut problem) = (String::new(), false, None);
    loop {
        tokio::select! {
            _ = &mut ctrl_c => break,
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
        if s.stopping() {
            break;
        }
        let st = s.status();
        if st.doing != doing {
            if !st.doing.is_empty() {
                eprintln!("{}", st.doing);
            }
            doing = st.doing;
        }
        if st.ready && !ready {
            ready = true;
            println!("ready: paired devices can browse the library");
        }
        if st.error != problem {
            if let Some(e) = &st.error {
                eprintln!("problem: {e}");
            }
            problem = st.error;
        }
    }
    s.stop();
    println!("stopped");
    Ok(())
}

pub fn device_command(mut a: Args) -> Result<()> {
    if a.rest.is_empty() {
        bail!("device find|pair|unpair|systems|collections|games|game|add|remove|holdings|app (gamerepo-cli --help says more)");
    }
    let sub = a.rest.remove(0);
    let json_out = switch(&mut a, "--json");
    if sub == "find" {
        let wait = number::<f64>(&mut a, "--wait")?.unwrap_or(3.0);
        let found = device::find(Duration::from_secs_f64(wait))?;
        if json_out {
            return show(&found.iter().map(|(i, addr)| json!({ "address": addr, "server": i.server, "library": i.library, "version": i.version, "api": i.api })).collect::<Vec<_>>());
        }
        for (i, addr) in &found {
            println!("{addr}  {}  (Game Repo Server {}, {})", i.library, i.version, i.server);
        }
        if found.is_empty() {
            eprintln!("No server answered on this network (a firewall may be keeping it out): give device pair its address.");
        }
        return Ok(());
    }
    let to = PathBuf::from(a.need("--to").context("--to <folder>: the stand-in device's folder, holding ES-DE/ and ROMs/")?);
    if sub == "app" {
        return app_command(a, &to);
    }
    let d = Device::open(&to)?;
    match sub.as_str() {
        "pair" => {
            let name = a.flag("--name").unwrap_or_else(|| "Stand-in device".into());
            let (Some(address), Some(code)) = (a.rest.first(), a.rest.get(1)) else { bail!("device pair <address> <code> --to <folder>") };
            let address = if address.contains(':') { address.clone() } else { format!("{address}:{}", api::PORT) };
            let (info, _) = device::client::info(&address)?;
            let s = device::client::pair(&address, code, &name, "stand-in")?;
            d.keep_server(&s)?;
            println!("Paired with {} at {address} (certificate {})", info.library, s.fingerprint);
            Ok(())
        }
        "unpair" => {
            let s = d.server()?;
            if let Err(e) = s.call::<Value>("DELETE", "/v1/device", None::<&()>) {
                eprintln!("The server wasn't told ({e:#}); this device forgets it all the same.");
            }
            d.forget_server()?;
            println!("Unpaired.");
            Ok(())
        }
        "systems" => {
            let v: Vec<System> = d.server()?.get("/v1/systems")?;
            if json_out {
                return show(&v);
            }
            for x in v {
                println!("{:<14} {:>7} games {:>10}  {}", x.id, x.games, bytes(x.size), x.name);
            }
            Ok(())
        }
        "collections" => {
            let v: Vec<Collection> = d.server()?.get("/v1/collections")?;
            if json_out {
                return show(&v);
            }
            for c in v {
                println!("{:<10} {:>7} games {:>10}  {}", c.id, c.games, bytes(c.size), c.name);
                for g in c.groups.iter().filter(|g| !g.id.is_empty()) {
                    println!("  {:<8} {:>7} games {:>10}  {}", g.id, g.games, bytes(g.size), g.name);
                }
            }
            Ok(())
        }
        "games" => {
            let mut q = vec![];
            for (flag, param) in [("--system", "system"), ("--search", "q"), ("--collection", "collection"), ("--group", "group"), ("--sort", "sort"), ("--offset", "offset"), ("--limit", "limit")] {
                if let Some(v) = a.flag(flag) {
                    q.push(format!("{param}={}", query_value(&v)));
                }
            }
            if switch(&mut a, "--desc") {
                q.push("desc=1".into());
            }
            let page: Page = d.server()?.get(&format!("/v1/games{}{}", if q.is_empty() { "" } else { "?" }, q.join("&")))?;
            if json_out {
                return show(&page);
            }
            for g in &page.items {
                let year = g.year.map(|y| format!(" ({y})")).unwrap_or_default();
                let discs = if g.discs > 1 { format!(", {} discs", g.discs) } else { String::new() };
                println!("{}  {}{year}  {}{discs}", g.key, g.title, bytes(g.size));
            }
            eprintln!("{} to {} of {}", if page.items.is_empty() { 0 } else { page.offset + 1 }, page.offset + page.items.len() as u64, page.total);
            Ok(())
        }
        "game" => {
            let key = a.rest.first().context("device game <key> --to <folder>")?;
            show(&d.server()?.get::<Value>(&format!("/v1/games/{}", gamerepo_serve::http::encode_path(key)))?)
        }
        "add" => {
            let mut media = list(a.flag("--media"));
            let manuals = switch(&mut a, "--manuals") || media.iter().any(|m| m == "manual" || m == "manuals");
            media.retain(|m| m != "manual" && m != "manuals");
            let o = AddOptions {
                media,
                manuals,
                size: number(&mut a, "--size")?.unwrap_or(0),
                collection: a.flag("--collection").unwrap_or_default(),
                bios: switch(&mut a, "--bios"),
                ..Default::default()
            };
            let keys: Vec<String> = a.rest.iter().filter(|k| !k.starts_with("--")).cloned().collect();
            if keys.is_empty() {
                bail!("device add <key>... --to <folder> (device games lists the keys)");
            }
            let s = d.server()?;
            let added = device::add(&s, &d, &keys, &o, &mut |p| eprintln!("{p}"))?;
            if json_out {
                return show(&json!({ "games": added.games, "files": added.files, "bytes": added.bytes, "there": added.there, "left": added.left, "notes": added.notes }));
            }
            println!("Added {} game{}: {} files fetched ({}), {} there already.", added.games, if added.games == 1 { "" } else { "s" }, added.files, bytes(added.bytes), added.there);
            for l in &added.left {
                println!("left out: {} ({})", l.what, l.why);
            }
            for n in &added.notes {
                println!("note: {n}");
            }
            Ok(())
        }
        "remove" => {
            let keys: Vec<String> = a.rest.clone();
            if keys.is_empty() {
                bail!("device remove <key>... --to <folder> (device holdings lists them)");
            }
            let r = device::remove(&d, &keys)?;
            if let Ok(s) = d.server() {
                let _ = s.call::<Value>("PUT", "/v1/device", Some(&d.holding()));
            }
            println!("Removed {} game{}: {} files deleted, {} kept for other games.", r.games, if r.games == 1 { "" } else { "s" }, r.files, r.kept);
            Ok(())
        }
        "holdings" => {
            let rec = d.record();
            let h = d.holding();
            if json_out {
                return show(&json!({ "games": rec.games, "holding": h }));
            }
            for g in &rec.games {
                let mut seen = std::collections::HashSet::new();
                let size: u64 = g.files.iter().filter(|f| seen.insert((&f.place, &f.path))).map(|f| f.size).sum();
                println!("{}  {}  {} files, {}", g.key, g.title, g.files.len(), bytes(size));
            }
            println!("{} games, {}; {} free", h.games, bytes(h.size), bytes(h.free));
            Ok(())
        }
        other => bail!("unknown device command {other} (gamerepo-cli --help lists them)"),
    }
}

/// Game Repo Go's backend run as its pages call it (`device app <command> [<args as JSON>]`),
/// with the stand-in device's folder as the place ES-DE is found and `--data` as the
/// client's own folder; `--wait` waits for the queue and the jobs in the background.
fn app_command(mut a: Args, to: &std::path::Path) -> Result<()> {
    let data = PathBuf::from(a.need("--data").context("--data <folder>: the client's own data folder")?);
    let wait = switch(&mut a, "--wait");
    let Some(cmd) = a.rest.first().cloned() else { bail!("device app <command> [<args as JSON>] --to <folder> --data <folder> [--wait]") };
    let args: Value = match a.rest.get(1) {
        Some(j) => serde_json::from_str(j).with_context(|| format!("{j} isn't JSON"))?,
        None => json!({}),
    };
    let secrets = Box::new(device::app::FileSecrets(data.join("secrets.json")));
    let app = device::app::App::open(&data, vec![to.to_path_buf()], "stand-in", secrets, Box::new(|_| {}))?;
    let answer = app.call(&cmd, &args)?;
    if !wait {
        app.queue.quit();
        return show(&answer);
    }
    let mut last = String::new();
    loop {
        let st = app.call("status", &json!({}))?;
        let (sync, checking) = (st["sync"]["running"].as_bool().unwrap_or(false), st["checking"]["running"].as_bool().unwrap_or(false));
        let q = &st["queue"];
        let busy = app.queue.busy() && !q["offline"].as_bool().unwrap_or(false);
        let line = format!("queue {} left, {} of {}; sync {}; checking {}", q["left"], bytes(q["done"].as_u64().unwrap_or(0)), bytes(q["size"].as_u64().unwrap_or(0)), if sync { "running" } else { "idle" }, if checking { "running" } else { "idle" });
        if line != last {
            eprintln!("{line}");
            last = line;
        }
        if !sync && !checking && !busy {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    app.queue.quit();
    show(&json!({ "answer": answer, "queue": app.call("queue", &json!({}))?, "status": app.call("status", &json!({}))? }))
}

/// Game Repo Go's pages and backend over HTTP, for the page test (tests/client_page.py) and
/// for trying the pages in a browser: POST /invoke/<command> as tests/tauri_shim.js sends
/// them (`api`, `app_info`, `pick_folder`), GET /picture/<key>/<kind> as the app's
/// `picture://`, GET /events the backend's changes (server-sent events, the app's
/// "go-changed"), POST /test/pick the answer to the next folder dialog, and the pages from
/// --ui (client.html at /).
pub fn client_ui(mut a: Args) -> Result<()> {
    use std::io::Write;
    use std::sync::mpsc::{channel, Sender};
    use std::sync::{Arc, Mutex};
    let ui = PathBuf::from(a.need("--ui")?);
    let home = PathBuf::from(a.need("--home")?);
    let data = a.flag("--data").map(PathBuf::from).unwrap_or_else(|| home.join(".gamerepo-go"));
    let port: u16 = number(&mut a, "--port")?.unwrap_or(8791);
    let kind = a.flag("--kind").unwrap_or_else(|| "stand-in".into());
    let listeners: Arc<Mutex<Vec<Sender<String>>>> = Arc::default();
    let tell = listeners.clone();
    let events = Box::new(move |what: &str| tell.lock().unwrap().retain(|l| l.send(what.to_string()).is_ok()));
    let secrets = Box::new(device::app::FileSecrets(data.join("secrets.json")));
    let app = device::app::App::open(&data, vec![home.clone()], &kind, secrets, events)?;
    let picks: Arc<Mutex<Vec<String>>> = Arc::default();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("Game Repo Go's pages on http://127.0.0.1:{port}/ (ES-DE looked for in {}, the client's folder {})", home.display(), data.display());
    for conn in listener.incoming() {
        let Ok(mut stream) = conn else { continue };
        let (app, ui, picks, listeners, kind) = (app.clone(), ui.clone(), picks.clone(), listeners.clone(), kind.clone());
        std::thread::spawn(move || {
            let Ok(req) = crate::read_request(&mut stream) else { return };
            let (path, query) = req.path.split_once('?').unwrap_or((req.path.as_str(), ""));
            let json = |stream: &mut std::net::TcpStream, code: u16, v: &Value| crate::respond(stream, code, "application/json", &serde_json::to_vec(v).unwrap_or_default());
            let r: Result<()> = (|| {
                if let Some(cmd) = path.strip_prefix("/invoke/") {
                    let args: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                    match cmd {
                        "api" => match app.call(args["cmd"].as_str().unwrap_or(""), &args["args"]) {
                            Ok(v) => json(&mut stream, 200, &v),
                            Err(e) => {
                                eprintln!("{} {}: {e:#}", args["cmd"].as_str().unwrap_or(""), args["args"]);
                                // 200 marked failed: the shim throws it as the app's invoke does,
                                // and the browser logs no failed request for an answer like a wrong code
                                let body = serde_json::to_vec(&json!(format!("{e:#}"))).unwrap_or_default();
                                crate::respond_with(&mut stream, 200, "application/json", &body, "x-failed: 1\r\n")
                            }
                        },
                        "app_info" => json(&mut stream, 200, &json!({ "kind": kind, "version": env!("CARGO_PKG_VERSION"), "picture_url": "/picture/" })),
                        "pick_folder" => {
                            let next = { let mut p = picks.lock().unwrap(); (!p.is_empty()).then(|| p.remove(0)) };
                            json(&mut stream, 200, &json!(next))
                        }
                        other => json(&mut stream, 404, &json!(format!("unknown command {other}"))),
                    }
                } else if path == "/test/pick" {
                    let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                    if let Some(p) = v["path"].as_str() {
                        picks.lock().unwrap().push(p.to_string());
                    }
                    json(&mut stream, 200, &json!(true))
                } else if let Some(rest) = path.strip_prefix("/picture/") {
                    let rest = gamerepo_core::api::percent_decode(rest);
                    let size = query.split('&').find_map(|kv| kv.strip_prefix("size=")).and_then(|v| v.parse().ok()).unwrap_or(256u32);
                    let found = rest.rsplit_once('/').and_then(|(key, kind)| app.catalogue.picture(app.server().as_ref(), key, kind, size.clamp(64, 2048)).ok().flatten());
                    match found {
                        Some(b) => crate::respond(&mut stream, 200, "image/jpeg", &b),
                        None => crate::respond(&mut stream, 404, "text/plain", b"no picture"),
                    }
                } else if path == "/events" {
                    let (tx, rx) = channel();
                    listeners.lock().unwrap().push(tx);
                    stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-store\r\naccess-control-allow-origin: *\r\nconnection: keep-alive\r\n\r\n: open\n\n")?;
                    // a comment every 15 s finds a page that's gone
                    loop {
                        let line = match rx.recv_timeout(Duration::from_secs(15)) {
                            Ok(what) => format!("data: {what}\n\n"),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ": still here\n\n".into(),
                            Err(_) => return Ok(()),
                        };
                        if stream.write_all(line.as_bytes()).and_then(|_| stream.flush()).is_err() {
                            return Ok(());
                        }
                    }
                } else if req.method == "GET" {
                    let rel = if path == "/" { "client.html".to_string() } else { gamerepo_core::api::percent_decode(path.trim_start_matches('/')) };
                    match gamerepo_core::library::rel_inside(&rel).ok().and_then(|r| std::fs::read(ui.join(r)).ok()) {
                        Some(b) => crate::respond(&mut stream, 200, crate::content_type(&rel), &b),
                        None => crate::respond(&mut stream, 404, "text/plain", b"not found"),
                    }
                } else {
                    crate::respond(&mut stream, 405, "text/plain", b"")
                }
            })();
            if let Err(e) = r {
                eprintln!("{path}: {e:#}");
            }
        });
    }
    Ok(())
}
