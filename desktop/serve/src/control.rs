//! The control channel (D89): Game Repo (or `gamerepo-cli serve-devices …`) talks to the
//! server over plain HTTP on 127.0.0.1 only, each request carrying the secret written in
//! the server's `server.json`, which only this computer's user can read. Through it come
//! the server's state, the pairing code, the devices, the library's changes and *Stop*.
//!
//!   GET /status            how the server is
//!   POST /pair             start pairing: the code and the seconds it works for
//!   DELETE /pair           stop pairing
//!   GET /devices           the paired devices
//!   DELETE /devices/<id>   remove one
//!   POST /changed          the library changed: {"paths": [...]} or everything
//!   POST /stop             stop the server

use crate::http::{self, Response};
use crate::{tls, Running, Shared};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};
use std::io::{BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// The header carrying the secret.
pub const SECRET: &str = "X-Game-Repo-Secret";

pub fn read_running(data: &Path) -> Option<Running> {
    serde_json::from_slice(&std::fs::read(data.join("server.json")).ok()?).ok()
}

/// The server running for this data folder, if one answers.
pub fn running(data: &Path) -> Option<Running> {
    let r = read_running(data)?;
    call_at(&r, "GET", "/status", None).ok().map(|_| r)
}

/// Ask the server running for `data`.
pub fn call(data: &Path, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let r = read_running(data).ok_or_else(|| anyhow!("Game Repo Server isn't running (there's no {}).", data.join("server.json").display()))?;
    call_at(&r, method, path, body)
}

fn call_at(r: &Running, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], r.control));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).context("Game Repo Server isn't answering")?;
    s.set_read_timeout(Some(Duration::from_secs(30)))?;
    let body = body.map(|b| serde_json::to_vec(b).unwrap_or_default()).unwrap_or_default();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{SECRET}: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", r.secret, body.len())?;
    s.write_all(&body)?;
    let mut all = vec![];
    s.read_to_end(&mut all)?;
    let text = String::from_utf8_lossy(&all);
    let (head, rest) = text.split_once("\r\n\r\n").ok_or_else(|| anyhow!("Game Repo Server's answer can't be read"))?;
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let v: Value = serde_json::from_str(rest).unwrap_or(Value::Null);
    if status != 200 {
        bail!("{}", v["error"].as_str().unwrap_or("Game Repo Server refused that"));
    }
    Ok(v)
}

pub(crate) fn accept(sh: Arc<Shared>, listener: TcpListener, secret: String) {
    while !sh.stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((tcp, peer)) if peer.ip().is_loopback() => {
                let _ = answer(&sh, tcp, &secret);
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn answer(sh: &Arc<Shared>, tcp: TcpStream, secret: &str) -> Result<()> {
    tcp.set_nonblocking(false)?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut r = BufReader::new(tcp);
    let Some(req) = http::read_request(&mut r)? else { return Ok(()) };
    let resp = if !req.header(SECRET).is_some_and(|s| tls::same_secret(s, secret)) {
        Response::problem(403, "Not without the server's secret.")
    } else {
        route(sh, &req)
    };
    http::write_response(r.get_mut(), &resp, false, true)
}

fn route(sh: &Arc<Shared>, req: &http::Request) -> Response {
    let mut devices = sh.devices.lock().unwrap();
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/status") => {
            drop(devices);
            Response::json(200, &sh.status())
        }
        ("POST", "/pair") => {
            let code = devices.start_pairing();
            Response::json(200, &json!({ "code": code, "seconds": crate::devices::CODE_LIFE.as_secs() }))
        }
        ("DELETE", "/pair") => {
            devices.end_pairing();
            Response::json(200, &json!({}))
        }
        ("GET", "/devices") => {
            let list: Vec<Value> = devices
                .list()
                .iter()
                .map(|d| json!({ "id": d.id, "name": d.name, "kind": d.kind, "paired": d.paired, "seen": d.seen, "address": d.address, "holding": d.holding }))
                .collect();
            Response::json(200, &list)
        }
        ("DELETE", p) if p.starts_with("/devices/") => match devices.remove(&p["/devices/".len()..]) {
            Ok(()) => Response::json(200, &json!({})),
            Err(e) => Response::problem(404, e.to_string()),
        },
        ("POST", "/changed") => {
            drop(devices);
            let v: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let paths = v["paths"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(PathBuf::from)).collect());
            sh.changed(paths);
            Response::json(200, &json!({}))
        }
        ("POST", "/stop") => {
            sh.stop.store(true, Ordering::SeqCst);
            sh.wake.1.notify_all();
            Response::json(200, &json!({}))
        }
        _ => Response::problem(404, "There's nothing here."),
    }
}
