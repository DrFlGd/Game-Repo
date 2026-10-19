// Game Repo Server (docs/PLAN.md, "Phase 9a design", D89): the library served read-only to
// paired devices on the home network (gamerepo-serve), with a tray icon and a small window.
// Game Repo starts it beside itself with the open library (`--library`, `--parent`: it
// stops when Game Repo ends unless it's kept); started alone (Start with Windows, or from
// its own installer) it serves the library and port it served last. One runs per computer:
// a second start shows the first one's window. The window is web/server.html, the panel of
// Game Repo's Settings › Serve to devices, calling the `api` command below.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::{anyhow, bail, Context, Result};
use gamerepo_core::serving;
use gamerepo_serve::{Options, Server};
use gamerepo_wire::control;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent, Wry};
use tauri_plugin_dialog::DialogExt;

/// How it was started (the options of `gamerepo-cli serve-devices` that Game Repo uses).
struct Args {
    library: Option<PathBuf>,
    data: PathBuf,
    port: Option<u16>,
    index_from: Option<PathBuf>,
    parent: bool,
    keep: bool,
}

fn args() -> Result<Args> {
    let mut a = Args { library: None, data: control::default_data(), port: None, index_from: None, parent: false, keep: false };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut value = || it.next().ok_or_else(|| anyhow!("{k} needs a value"));
        match k.as_str() {
            "--library" => a.library = Some(value()?.into()),
            "--data" => a.data = value()?.into(),
            "--port" => a.port = Some(value()?.parse().context("--port is a number")?),
            "--index-from" => a.index_from = Some(value()?.into()),
            "--parent" => a.parent = true,
            "--keep" => a.keep = true,
            other => bail!("unknown option {other}"),
        }
    }
    Ok(a)
}

/// The server this program runs, and what restarting it needs.
struct Serving {
    data: PathBuf,
    /// Started by Game Repo: it stops when Game Repo ends (its input closes), unless kept
    parent: bool,
    keep: bool,
    /// Game Repo's index of the library it was started with
    index_from: Option<(PathBuf, PathBuf)>,
    server: Mutex<Option<Server>>,
    /// Why it isn't serving
    problem: Mutex<Option<String>>,
    with_windows: Mutex<Option<bool>>,
    /// The tray menu's Start with Windows tick
    windows_item: Mutex<Option<CheckMenuItem<Wry>>>,
}

impl Serving {
    fn start(&self, library: &Path, port: u16, keep: bool) -> Result<()> {
        // Game Repo's index of the library, copied for the server's first
        let index_from = match &self.index_from {
            Some((lib, ix)) if lib == library => Some(ix.clone()),
            _ => gamerepo_core::Library::open_read_only(library)?.id().map(|id| gamerepo_core::index::path_for(&gamerepo_core::config::app_dirs().1, &id)),
        };
        let s = Server::start(Options { library: library.to_path_buf(), data: self.data.clone(), port, index_from, parent: self.parent, keep, ..Default::default() })?;
        let st = s.status();
        println!("Game Repo Server {} serving {} ({})", st.version, st.library_name, st.library);
        println!("at {}", st.address.unwrap_or_else(|| format!("127.0.0.1:{}", st.port)));
        *self.server.lock().unwrap() = Some(s);
        *self.problem.lock().unwrap() = None;
        Ok(())
    }

    /// Serve another library or on another port (the same when left out). When that
    /// doesn't start, it serves as it did and says why.
    fn restart(&self, library: Option<PathBuf>, port: Option<u16>) -> Result<()> {
        let old = self.server.lock().unwrap().take();
        let (was_lib, was_port, kept) = match &old {
            Some(s) => {
                let st = s.status();
                (Some(PathBuf::from(st.library)), st.port, st.kept)
            }
            None => (gamerepo_serve::last_library(&self.data), self.port(), self.keep),
        };
        if let Some(s) = old {
            s.stop();
        }
        let library = library.or_else(|| was_lib.clone()).ok_or_else(|| anyhow!("Choose a library to serve: Open another library…"))?;
        let port = port.unwrap_or(was_port);
        let Err(e) = self.start(&library, port, kept) else { return Ok(()) };
        let why = format!("{e:#}");
        match was_lib.filter(|l| *l != library || port != was_port) {
            Some(l) if self.start(&l, was_port, kept).is_ok() => {}
            _ => *self.problem.lock().unwrap() = Some(why.clone()),
        }
        bail!(why)
    }

    fn stop(&self) {
        if let Some(s) = self.server.lock().unwrap().take() {
            s.stop();
        }
    }

    /// The port it serves on, else the one it served on last.
    fn port(&self) -> u16 {
        match self.server.lock().unwrap().as_ref() {
            Some(s) => s.port(),
            None => gamerepo_serve::last_port(&self.data).unwrap_or(gamerepo_wire::api::PORT),
        }
    }

    fn with_windows(&self) -> bool {
        *self.with_windows.lock().unwrap().get_or_insert_with(serving::with_windows)
    }

    fn set_with_windows(&self, on: bool) -> Result<()> {
        let r = serving::set_with_windows(&std::env::current_exe()?, on);
        let now = serving::with_windows();
        *self.with_windows.lock().unwrap() = Some(now);
        if let Some(item) = self.windows_item.lock().unwrap().as_ref() {
            let _ = item.set_checked(now);
        }
        r
    }

    /// What the window's panel shows (Game Repo's `serve_status`, for this program).
    fn status(&self) -> Value {
        let (running, status, devices) = serving::state(&self.data);
        let ours = running.is_some_and(|r| r.pid == std::process::id());
        let last = gamerepo_serve::last_library(&self.data).map(|l| l.display().to_string());
        json!({
            "installed": true,
            "running": ours,
            "this_library": true,
            "status": if ours { status } else { Value::Null },
            "devices": if ours { devices } else { json!([]) },
            "settings": { "on": ours, "port": self.port(), "keep": true },
            "windows": cfg!(windows),
            "with_windows": self.with_windows(),
            "can_start_with_windows": cfg!(windows),
            "start_error": self.problem.lock().unwrap().clone(),
            // (Serve … again, when it isn't serving)
            "last_library": last,
            "last_library_name": last.as_deref().and_then(|l| Path::new(l).file_name()).map(|n| n.to_string_lossy()),
        })
    }
}

/// One of the window's commands (web/ui/serve.js).
#[tauri::command]
async fn api(app: AppHandle, cmd: String, args: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || call(&app, &cmd, &args)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

fn call(app: &AppHandle, cmd: &str, args: &Value) -> Result<Value> {
    let sv = app.state::<Serving>();
    let ctl = |method: &str, path: &str| control::call(&sv.data, method, path, None).map_err(|e| anyhow!("{e}"));
    match cmd {
        "serve_status" => Ok(sv.status()),
        "serve_pair" => ctl("POST", "/pair"),
        "serve_unpair" => ctl("DELETE", "/pair"),
        "serve_remove_device" => ctl("DELETE", &format!("/devices/{}", args["id"].as_str().unwrap_or_default())),
        "serve_networks" => Ok(json!(serving::networks())),
        "serve_set" => {
            if let Some(p) = args.get("port").filter(|p| !p.is_null()) {
                let port = p.as_u64().filter(|p| (1024..=65535).contains(p)).ok_or_else(|| anyhow!("A port is a number from 1024 to 65535."))? as u16;
                if port != sv.port() {
                    sv.restart(None, Some(port))?;
                }
            }
            if let Some(on) = args["with_windows"].as_bool() {
                sv.set_with_windows(on)?;
            }
            Ok(sv.status())
        }
        // the library it served last
        "serve_start" => {
            sv.restart(None, None)?;
            Ok(sv.status())
        }
        "serve_open_library" => {
            if let Some(dir) = app.dialog().file().set_title("Choose the library to serve").blocking_pick_folder() {
                sv.restart(Some(dir.into_path()?), None)?;
            }
            Ok(sv.status())
        }
        "serve_stop" => {
            quit(app);
            Ok(json!({}))
        }
        _ => bail!("unknown command {cmd}"),
    }
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn quit(app: &AppHandle) {
    app.state::<Serving>().stop();
    app.exit(0);
}

/// The icon by the clock: Pair a device…, Devices…, Open the library…, Start with Windows
/// and Stop; a click shows the window.
fn tray(app: &tauri::App) -> Result<()> {
    let pair = MenuItem::with_id(app, "pair", "Pair a device…", true, None::<&str>)?;
    let devices = MenuItem::with_id(app, "devices", "Devices…", true, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open the library…", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop", true, None::<&str>)?;
    let line = PredefinedMenuItem::separator(app)?;
    let menu = if cfg!(windows) {
        let sv = app.state::<Serving>();
        let windows = CheckMenuItem::with_id(app, "windows", "Start with Windows", true, sv.with_windows(), None::<&str>)?;
        *sv.windows_item.lock().unwrap() = Some(windows.clone());
        Menu::with_items(app, &[&pair, &devices, &open, &line, &windows, &PredefinedMenuItem::separator(app)?, &stop])?
    } else {
        Menu::with_items(app, &[&pair, &devices, &open, &line, &stop])?
    };
    let mut tray = TrayIconBuilder::with_id("main").tooltip("Game Repo Server").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, e| match e.id().as_ref() {
        "pair" => {
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = control::call(&app.state::<Serving>().data, "POST", "/pair", None);
                show(&app);
            });
        }
        "devices" => show(app),
        "open" => {
            let app = app.clone();
            app.dialog().file().set_title("Choose the library to serve").pick_folder(move |dir| {
                let Some(Ok(dir)) = dir.map(|d| d.into_path()) else { return };
                std::thread::spawn(move || {
                    let _ = app.state::<Serving>().restart(Some(dir), None);
                    show(&app);
                });
            });
        }
        "windows" => {
            let sv = app.state::<Serving>();
            let on = !sv.with_windows();
            if sv.set_with_windows(on).is_err() {
                show(app);
            }
        }
        "stop" => quit(app),
        _ => {}
    })
    .on_tray_icon_event(|tray, e| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
            show(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

/// Ends the program when the server is asked to stop (Stop in Game Repo, or Game Repo
/// ending when it isn't kept), shows the window when it's started again, keeps the tray
/// icon's tooltip saying what it serves, and says when it's ready.
fn watch(app: AppHandle) {
    std::thread::spawn(move || {
        let (mut said, mut ready) = (String::new(), false);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let sv = app.state::<Serving>();
            let (stopping, shown, tip, now_ready) = match sv.server.lock().unwrap().as_ref() {
                Some(s) => {
                    let st = s.status();
                    (s.stopping(), s.shown(), format!("Game Repo Server: serving {}", st.library_name), st.ready)
                }
                None => (false, false, "Game Repo Server: not serving".to_string(), false),
            };
            // (the log Game Repo reads, and tests/serve_devices.py)
            if now_ready && !ready {
                println!("ready: paired devices can browse the library");
            }
            ready = now_ready;
            if stopping {
                println!("stopped");
                quit(&app);
                return;
            }
            if shown {
                show(&app);
            }
            if tip != said {
                if let Some(t) = app.tray_by_id("main") {
                    let _ = t.set_tooltip(Some(&tip));
                }
                said = tip;
            }
        }
    });
}

fn main() {
    let a = match args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(2);
        }
    };
    // one per computer: a second start shows the first one's window
    if let Some(r) = control::running(&a.data) {
        if a.parent {
            eprintln!("Game Repo Server is running already (serving {}).", r.library);
            std::process::exit(1);
        }
        let _ = control::call_at(&r, "POST", "/show", None);
        return;
    }
    let sv = Serving {
        data: a.data.clone(),
        parent: a.parent,
        keep: a.keep,
        index_from: a.library.clone().zip(a.index_from.clone()),
        server: Mutex::new(None),
        problem: Mutex::new(None),
        with_windows: Mutex::new(None),
        windows_item: Mutex::new(None),
    };
    // the library asked for, else the one served last
    let library = a.library.clone().or_else(|| gamerepo_serve::last_library(&a.data));
    let port = a.port.or_else(|| gamerepo_serve::last_port(&a.data)).unwrap_or(gamerepo_wire::api::PORT);
    match &library {
        Some(lib) => {
            if let Err(e) = sv.start(lib, port, a.keep) {
                if a.parent {
                    // Game Repo reads why in the server's log
                    eprintln!("{e:#}");
                    std::process::exit(1);
                }
                *sv.problem.lock().unwrap() = Some(format!("{e:#}"));
            }
        }
        None => *sv.problem.lock().unwrap() = Some("No library yet: choose one with Open another library…".into()),
    }
    let show_now = sv.problem.lock().unwrap().is_some();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(sv)
        .setup(move |app| {
            tray(app)?;
            watch(app.handle().clone());
            if show_now {
                show(app.handle());
            }
            Ok(())
        })
        // closing the window hides it: the server keeps going, with its icon by the clock
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![api])
        .build(tauri::generate_context!())
        .expect("error while starting Game Repo Server")
        .run(|app, e| match e {
            // no window open isn't the end: only Stop is
            RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            RunEvent::Exit => app.state::<Serving>().stop(),
            _ => {}
        });
}
