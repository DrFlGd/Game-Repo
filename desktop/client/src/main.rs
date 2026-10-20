// Game Repo Go (docs/PLAN.md, "Phase 9b design", D96–D104): a Game Repo library browsed
// on the home network and its games downloaded into ES-DE's folders on this device. A thin
// shell over gamerepo-device's command table (`App::call`, the same the stand-in runs as
// `gamerepo-cli device app` and `client-ui`): the window is web/client.html, pictures come
// through `picture://` (from the device's cache or the library; a web view won't trust the
// server's own certificate), and the backend's changes reach the page as "go-changed"
// events. Closing the window while games are downloading leaves an icon by the clock with
// their progress, and it quits when they're done (D99). Android's side is part 3.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod secrets;

use gamerepo_device::app::App;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_dialog::DialogExt;

struct Go {
    app: Arc<App>,
    /// The window's hidden while games download, with the icon by the clock showing
    hidden: AtomicBool,
}

/// One of the backend's commands (gamerepo_device::app), answered as JSON.
#[tauri::command]
async fn api(app: AppHandle, cmd: String, args: Value) -> Result<Value, String> {
    let go = app.state::<Go>().app.clone();
    tauri::async_runtime::spawn_blocking(move || go.call(&cmd, &args)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
}

/// What the page needs to know about the shell: where pictures come from.
#[tauri::command]
fn app_info() -> Value {
    json!({
        "kind": KIND,
        "version": env!("CARGO_PKG_VERSION"),
        // Windows' and Android's web views reach a protocol at http://<name>.localhost/
        "picture_url": if cfg!(any(windows, target_os = "android")) { "http://picture.localhost/" } else { "picture://localhost/" },
    })
}

/// A folder chosen (ES-DE's folder or the ROMs folder in Settings). None if cancelled.
#[tauri::command]
async fn pick_folder(app: AppHandle, title: Option<String>) -> Result<Option<String>, String> {
    let picked = app.dialog().file().set_title(title.unwrap_or_else(|| "Choose a folder".into())).blocking_pick_folder();
    match picked {
        Some(p) => Ok(Some(p.into_path().map_err(|e| e.to_string())?.display().to_string())),
        None => Ok(None),
    }
}

const KIND: &str = if cfg!(target_os = "android") { "android" } else if cfg!(windows) { "windows" } else { "linux" };

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `picture://localhost/<game's key>/<kind>?size=<pixels>`: a game's picture, from the
/// device's cache or the library (D98). 404 when there's none (or the library's away and
/// it isn't kept), and the page shows the title.
fn picture(app: &App, path: &str, query: Option<&str>) -> tauri::http::Response<Vec<u8>> {
    let path = percent_decode(path.trim_start_matches('/'));
    let size = query.into_iter().flat_map(|q| q.split('&')).find_map(|kv| kv.strip_prefix("size=")).and_then(|v| v.parse().ok()).unwrap_or(256u32);
    let found = path.rsplit_once('/').and_then(|(key, kind)| app.catalogue.picture(app.server().as_ref(), key, kind, size.clamp(64, 2048)).ok().flatten());
    let r = tauri::http::Response::builder().header("Access-Control-Allow-Origin", "*");
    match found {
        Some(b) => r.status(200).header("Content-Type", "image/jpeg").header("Cache-Control", "max-age=86400").body(b),
        None => r.status(404).body(Vec::new()),
    }
    .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
}

fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    app.state::<Go>().hidden.store(false, Ordering::SeqCst);
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_visible(false);
    }
}

/// The icon by the clock, shown while the window's closed and games download: a click
/// opens the window again; Quit ends it (the queue goes on when Game Repo Go next opens).
fn tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Game Repo Go", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit (downloads go on next time)", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?, &quit])?;
    let mut t = TrayIconBuilder::with_id("main").tooltip("Game Repo Go").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        t = t.icon(icon.clone());
    }
    let t = t
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    t.set_visible(false)?;
    Ok(())
}

/// While the window's hidden: the icon's tooltip says how far the downloads are, and Game
/// Repo Go quits once they're done.
fn watch(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let go = app.state::<Go>();
        if !go.hidden.load(Ordering::SeqCst) {
            continue;
        }
        if !go.app.queue.busy() {
            app.exit(0);
            return;
        }
        let q = go.app.queue.shown();
        let tip = if q.offline {
            "Game Repo Go: waiting for the library".to_string()
        } else {
            let pct = if q.size > 0 { q.done * 100 / q.size } else { 0 };
            format!("Game Repo Go: downloading {} game{}, {pct}%", q.left, if q.left == 1 { "" } else { "s" })
        };
        if let Some(t) = app.tray_by_id("main") {
            let _ = t.set_tooltip(Some(&tip));
        }
    });
}

fn main() {
    static HANDLE: OnceLock<AppHandle> = OnceLock::new();
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show(app)));
    builder
        .plugin(tauri_plugin_dialog::init())
        // picture://localhost/<key>/<kind> (http://picture.localhost/… on Windows)
        .register_asynchronous_uri_scheme_protocol("picture", |ctx, request, responder| {
            let app = ctx.app_handle().state::<Go>().app.clone();
            let (path, query) = (request.uri().path().to_string(), request.uri().query().map(String::from));
            std::thread::spawn(move || responder.respond(picture(&app, &path, query.as_deref())));
        })
        .setup(|app| {
            let _ = HANDLE.set(app.handle().clone());
            let data = app.path().app_local_data_dir()?;
            // ES-DE's folder is ~/ES-DE (Settings can name another, for a portable install)
            let homes = app.path().home_dir().into_iter().collect();
            let events = Box::new(|what: &str| {
                if let Some(h) = HANDLE.get() {
                    let _ = h.emit("go-changed", what);
                }
            });
            let go = App::open(&data, homes, KIND, secrets::store(&data), events)?;
            app.manage(Go { app: go, hidden: AtomicBool::new(false) });
            tray(app)?;
            watch(app.handle().clone());
            Ok(())
        })
        // closing the window while games download keeps them going, with the icon by the clock
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                let go = w.state::<Go>();
                if go.app.queue.busy() {
                    api.prevent_close();
                    let _ = w.hide();
                    go.hidden.store(true, Ordering::SeqCst);
                    if let Some(t) = w.app_handle().tray_by_id("main") {
                        let _ = t.set_visible(true);
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![api, app_info, pick_folder])
        .build(tauri::generate_context!())
        .expect("error while starting Game Repo Go")
        .run(|app, e| {
            if let RunEvent::Exit = e {
                app.state::<Go>().app.queue.quit();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn a_picture_address_is_read_back() {
        assert_eq!(percent_decode("gb/Alpha%20(USA)/cover"), "gb/Alpha (USA)/cover");
        assert_eq!(percent_decode("snes/Caf%C3%A9%2FBar/box"), "snes/Café/Bar/box");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("end%"), "end%");
    }
}
