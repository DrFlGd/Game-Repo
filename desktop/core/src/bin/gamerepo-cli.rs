//! Command-line access to the desktop app's backend (same code, no window).
//!
//!   gamerepo-cli --version
//!   gamerepo-cli --help
//!   gamerepo-cli library-info --library <dir>
//!   gamerepo-cli index-info --library <dir> --data <dir>
//!   gamerepo-cli serve --ui <dir> --home <dir> [--library <dir>] [--port 8790]
//!
//! `serve` runs the app's commands behind a local web server, so the page can be
//! tested in an ordinary browser (with tests/tauri_shim.js standing in for Tauri's
//! `invoke`). The commands for scanning, auditing and exporting join this list in
//! the phases that build them (docs/PLAN.md, section 7).

use anyhow::{bail, Context, Result};
use gamerepo_core::api::{App, AppPaths, Reply};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const USAGE: &str = "Game Repo's backend, without the window.

usage: gamerepo-cli <command> [options]

  --version                  the version of the app this was built with
  --help                     this text
  library-info --library <dir>
                             open (or make) a library folder and print its details
  index-info --library <dir> --data <dir>
                             open (or make) that library's index in the data folder
                             <dir>/index/ and print its details
  serve --ui <dir> --home <dir> [--library <dir>] [--port 8790]
                             the app's commands and the page over HTTP on 127.0.0.1,
                             with settings kept under --home (used by the tests)
";

struct Args {
    rest: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> Option<String> {
        let i = self.rest.iter().position(|a| a == name)?;
        if i + 1 >= self.rest.len() {
            return None;
        }
        let v = self.rest.remove(i + 1);
        self.rest.remove(i);
        Some(v)
    }
    fn need(&mut self, name: &str) -> Result<String> {
        self.flag(name).with_context(|| format!("missing {name}"))
    }
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e:#}");
        std::process::exit(2);
    }
}

async fn run() -> Result<()> {
    let mut all: Vec<String> = std::env::args().skip(1).collect();
    if all.is_empty() {
        eprint!("{USAGE}");
        std::process::exit(2);
    }
    let cmd = all.remove(0);
    let mut a = Args { rest: all };
    match cmd.as_str() {
        "--version" | "-V" => {
            println!("gamerepo-cli {}", gamerepo_core::VERSION);
            Ok(())
        }
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        "library-info" => {
            let lib = gamerepo_core::Library::open(PathBuf::from(a.need("--library")?))?;
            println!("{}", serde_json::to_string_pretty(&lib.info())?);
            Ok(())
        }
        "index-info" => {
            let lib = gamerepo_core::Library::open(PathBuf::from(a.need("--library")?))?;
            let data = PathBuf::from(a.need("--data")?);
            let id = lib.id().context("the library has no id")?;
            let ix = gamerepo_core::index::Index::open(&gamerepo_core::index::path_for(&data, &id), &id)?;
            println!("{}", serde_json::to_string_pretty(&ix.info()?)?);
            Ok(())
        }
        "serve" => serve(a).await,
        other => bail!("unknown command {other} (gamerepo-cli --help lists them)"),
    }
}

/// The app's backend with its settings under `home` (config/, data/).
fn app_for(home: &Path, library_url: &str) -> Result<Arc<App>> {
    App::new(AppPaths {
        config_dir: home.join("config"),
        data_dir: home.join("data"),
        library_url: library_url.into(),
    })
}

fn content_type(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "ttf" => "font/ttf",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

struct Http {
    method: String,
    path: String,
    range: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut std::net::TcpStream) -> Result<Http> {
    use std::io::Read;
    let mut buf = vec![];
    let mut chunk = [0u8; 65536];
    let head_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            bail!("connection closed");
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 1 << 20 {
            bail!("headers too long");
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("/").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    let range = headers
        .iter()
        .find(|(k, _)| k == "range")
        .map(|(_, v)| v.clone());
    Ok(Http {
        method,
        path,
        range,
        body,
    })
}

fn respond(stream: &mut std::net::TcpStream, code: u16, ctype: &str, body: &[u8]) -> Result<()> {
    respond_with(stream, code, ctype, body, "")
}

fn respond_with(
    stream: &mut std::net::TcpStream,
    code: u16,
    ctype: &str,
    body: &[u8],
    extra: &str,
) -> Result<()> {
    use std::io::Write;
    let head = format!(
        "HTTP/1.1 {code} {}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\naccess-control-allow-origin: *\r\ncache-control: no-store\r\naccept-ranges: bytes\r\n{extra}connection: close\r\n\r\n",
        if code < 400 { "OK" } else { "Error" },
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}

/// The app's commands over HTTP: POST /invoke/<command> (as tests/tauri_shim.js
/// sends them), GET /library/<path> for library files, everything else from --ui.
/// POST /test/pick queues the answer to the next folder or file dialog.
async fn serve(mut a: Args) -> Result<()> {
    let ui = PathBuf::from(a.need("--ui")?);
    let home = PathBuf::from(a.need("--home")?);
    let port: u16 = a
        .flag("--port")
        .map(|p| p.parse())
        .transpose()?
        .unwrap_or(8790);
    if let Some(lib) = a.flag("--library") {
        let mut cfg = gamerepo_core::config::AppConfig::load(&home.join("config/config.json"));
        cfg.set_library(Path::new(&lib));
        cfg.save(&home.join("config/config.json"))?;
    }
    let app = app_for(&home, "/library/")?;
    let picks: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
    eprintln!(
        "serving on http://127.0.0.1:{port}/ (home {})",
        home.display()
    );
    let rt = tokio::runtime::Handle::current();
    let accept = tokio::task::spawn_blocking(move || -> Result<()> {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let app = app.clone();
            let ui = ui.clone();
            let picks = picks.clone();
            let rt = rt.clone();
            std::thread::spawn(move || {
                let Ok(req) = read_request(&mut stream) else {
                    return;
                };
                let path = req.path.split('?').next().unwrap_or("/").to_string();
                let r: Result<()> = rt.block_on(async {
                    if let Some(cmd) = path.strip_prefix("/invoke/") {
                        let args: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                        match cmd {
                            "api" | "api_bytes" => match app
                                .call(args["cmd"].as_str().unwrap_or(""), args["args"].clone())
                                .await
                            {
                                Ok(Reply::Json(v)) => respond(
                                    &mut stream,
                                    200,
                                    "application/json",
                                    &serde_json::to_vec(&v)?,
                                ),
                                Ok(Reply::Bytes(b)) => {
                                    respond(&mut stream, 200, "application/octet-stream", &b)
                                }
                                Err(e) => {
                                    eprintln!(
                                        "{} {}: {e}",
                                        args["cmd"].as_str().unwrap_or(""),
                                        args["args"]
                                    );
                                    respond(
                                        &mut stream,
                                        400,
                                        "application/json",
                                        &serde_json::to_vec(&json!(e))?,
                                    )
                                }
                            },
                            "pick_folder" | "pick_file" => {
                                let next = {
                                    let mut p = picks.lock().unwrap();
                                    if p.is_empty() {
                                        None
                                    } else {
                                        Some(p.remove(0))
                                    }
                                };
                                respond(
                                    &mut stream,
                                    200,
                                    "application/json",
                                    &serde_json::to_vec(&json!(next))?,
                                )
                            }
                            "reveal" | "open_path" | "open_url" => {
                                respond(&mut stream, 200, "application/json", b"null")
                            }
                            other => respond(
                                &mut stream,
                                404,
                                "application/json",
                                &serde_json::to_vec(&json!(format!("unknown command {other}")))?,
                            ),
                        }
                    } else if path == "/test/pick" {
                        let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                        if let Some(p) = v["path"].as_str() {
                            picks.lock().unwrap().push(p.to_string());
                        }
                        respond(&mut stream, 200, "application/json", b"true")
                    } else if let Some(rel) = path.strip_prefix("/library/") {
                        match app.library_range(rel, req.range.as_deref()) {
                            Ok((code, b, cr)) => respond_with(
                                &mut stream,
                                code,
                                content_type(rel),
                                &b,
                                &cr.map(|c| format!("content-range: {c}\r\n"))
                                    .unwrap_or_default(),
                            ),
                            Err(e) => {
                                respond(&mut stream, 404, "text/plain", e.to_string().as_bytes())
                            }
                        }
                    } else if req.method == "GET" {
                        let rel = if path == "/" {
                            "index.html".to_string()
                        } else {
                            gamerepo_core::api::percent_decode(path.trim_start_matches('/'))
                        };
                        let p = gamerepo_core::library::rel_inside(&rel).map(|r| ui.join(r));
                        match p.ok().and_then(|p| std::fs::read(p).ok()) {
                            Some(b) => respond(&mut stream, 200, content_type(&rel), &b),
                            None => respond(&mut stream, 404, "text/plain", b"not found"),
                        }
                    } else {
                        respond(&mut stream, 405, "text/plain", b"")
                    }
                });
                if let Err(e) = r {
                    eprintln!("{path}: {e:#}");
                }
            });
        }
        Ok(())
    });
    accept.await?
}
