// Game Repo desktop app: the front end in web/ in a window, with the library
// folder behind web/platform-desktop.js. Nearly every command goes through
// gamerepo_core::api::App::call (the same table `gamerepo-cli serve` uses for testing);
// this file adds what needs the window: pick dialogs, the file manager, the
// library:// protocol, and Game Repo Server beside the app (started and stopped with it).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, RunEvent, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use gamerepo_core::api::{App, AppPaths, Reply};

struct AppState {
    app: Arc<App>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// One of the app's commands (see gamerepo_core::api), answered as JSON.
#[tauri::command]
async fn api(cmd: String, args: Value, state: State<'_, AppState>) -> Result<Value, String> {
    match state.app.call(&cmd, args).await? {
        Reply::Json(v) => Ok(v),
        Reply::Bytes(b) => Ok(json!({ "bytes": b.len() })),
    }
}

/// One of the app's commands, answered as raw bytes (files).
#[tauri::command]
async fn api_bytes(cmd: String, args: Value, state: State<'_, AppState>) -> Result<Response, String> {
    match state.app.call(&cmd, args).await? {
        Reply::Bytes(b) => Ok(Response::new(b)),
        Reply::Json(v) => Ok(Response::new(serde_json::to_vec(&v).map_err(err)?)),
    }
}

/// Pick a folder (opening a library). None if cancelled.
/// (async: blocking dialogs must not run on the main thread)
#[tauri::command]
async fn pick_folder(app: AppHandle, title: Option<String>) -> Result<Option<String>, String> {
    let picked = app.dialog().file().set_title(title.unwrap_or_else(|| "Choose a folder".into())).blocking_pick_folder();
    match picked {
        Some(p) => Ok(Some(p.into_path().map_err(err)?.display().to_string())),
        None => Ok(None),
    }
}

/// Pick a file. None if cancelled.
#[tauri::command]
async fn pick_file(app: AppHandle, title: Option<String>, extensions: Option<Vec<String>>) -> Result<Option<String>, String> {
    let mut d = app.dialog().file().set_title(title.unwrap_or_else(|| "Choose a file".into()));
    if let Some(exts) = extensions.filter(|e| !e.is_empty()) {
        let refs: Vec<&str> = exts.iter().map(String::as_str).collect();
        d = d.add_filter(exts.join(", ").to_uppercase(), &refs);
    }
    match d.blocking_pick_file() {
        Some(p) => Ok(Some(p.into_path().map_err(err)?.display().to_string())),
        None => Ok(None),
    }
}

/// Choose any number of files (documents to attach to a game, DATs to import).
#[tauri::command]
async fn pick_files(app: AppHandle, title: Option<String>) -> Result<Vec<String>, String> {
    let selected = app.dialog().file().set_title(title.unwrap_or_else(|| "Choose files".into())).blocking_pick_files();
    selected.unwrap_or_default().into_iter()
        .map(|p| p.into_path().map(|v| v.display().to_string()).map_err(err))
        .collect()
}

#[tauri::command]
fn reveal(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(PathBuf::from(path)).map_err(err)
}

/// Open a folder (the library, a game's folder) in the file manager.
#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(err)
}

/// Open a web link (a game's page on a metadata site, the releases page) in the default browser.
#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only web links open in the browser.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

/// The app's own addresses: its pages, the library's files, and the frames it makes (a README,
/// a PDF). Anything else a link or a PDF leads to opens in the browser instead of replacing the app.
fn in_app(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" | "library" | "asset" | "ipc" | "about" | "blob" | "data" => true,
        "http" | "https" => url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h.ends_with(".localhost")),
        _ => false,
    }
}

/// A web or mail link that tried to open in the app: hand it to the system.
fn open_outside(app: &AppHandle, url: &tauri::Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.to_string(), None::<&str>);
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        "json" => "application/json",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        // Matroska plays where its codecs do (WebView2 reads it as WebM); an AVI never does,
        // and the page offers the system's own player
        "mkv" => "video/webm",
        "avi" => "video/x-msvideo",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // library://localhost/<path> (http://library.localhost/<path> on Windows): files in the open library
        .register_asynchronous_uri_scheme_protocol("library", |ctx, request, responder| {
            let app = ctx.app_handle().state::<AppState>().app.clone();
            let path = request.uri().path().to_string();
            // range requests, so videos seek without reading whole files
            let range = request.headers().get("range").and_then(|v| v.to_str().ok()).map(String::from);
            tauri::async_runtime::spawn(async move {
                let res = tokio::task::spawn_blocking(move || {
                    let body = app.library_range(&path, range.as_deref());
                    // a magazine's cover or page is told by its own file's type (D68)
                    (app.served_name(&path), body)
                })
                .await;
                let response = match res {
                    Ok((path, Ok((code, body, content_range)))) => {
                        let mut r = tauri::http::Response::builder()
                            .status(code)
                            .header("Content-Type", mime(&path))
                            .header("Accept-Ranges", "bytes")
                            .header("Access-Control-Allow-Origin", "*");
                        if let Some(cr) = content_range {
                            r = r.header("Content-Range", cr);
                        }
                        r.body(body)
                    }
                    Ok((_, Err(e))) => tauri::http::Response::builder().status(404).body(e.to_string().into_bytes()),
                    Err(e) => tauri::http::Response::builder().status(500).body(e.to_string().into_bytes()),
                };
                responder.respond(response.unwrap_or_else(|_| tauri::http::Response::new(Vec::new())));
            });
        })
        .setup(|app| {
            // Game Repo Server, installed beside the app (tauri.windows.conf.json's externalBin)
            let server = std::env::current_exe()?.with_file_name(if cfg!(windows) { "gamerepo-server.exe" } else { "gamerepo-server" });
            let paths = AppPaths {
                config_dir: app.path().app_config_dir()?,
                data_dir: app.path().app_local_data_dir()?,
                library_url: if cfg!(windows) { "http://library.localhost/".into() } else { "library://localhost/".into() },
                server_data: gamerepo_core::serving::default_data(),
                server_program: if server.is_file() { vec![server.display().to_string()] } else { vec![] },
            };
            let core = App::new(paths)?;
            app.manage(AppState { app: core.clone() });
            // serving again if it was when the app closed
            std::thread::spawn(move || core.serve_resume());
            // the window (tauri.conf.json, "create": false), made here so that a link in a readme or a
            // PDF can't take it over: the web opens in the browser, and no other windows open
            let cfg = app.config().app.windows.iter().find(|w| w.label == "main").cloned().ok_or("tauri.conf.json has no main window")?;
            let (to_browser, to_browser2) = (app.handle().clone(), app.handle().clone());
            tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
                .on_navigation(move |url| {
                    if in_app(url) {
                        return true;
                    }
                    open_outside(&to_browser, url);
                    false
                })
                .on_new_window(move |url, _features| {
                    if !in_app(&url) {
                        open_outside(&to_browser2, &url);
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api, api_bytes, pick_folder, pick_file, pick_files, reveal, open_path, open_url])
        .build(tauri::generate_context!())
        .expect("error while running Game Repo")
        .run(|app, e| {
            // the server it started stops with it, unless it's kept serving
            if let RunEvent::Exit = e {
                app.state::<AppState>().app.serve_exit();
            }
        });
}
