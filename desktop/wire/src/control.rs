//! The client side of Game Repo Server's control channel (D89): plain HTTP on 127.0.0.1
//! only, each request carrying the secret the server wrote in its `server.json`, which
//! only this computer's user can read. Game Repo, the server's own window and
//! `gamerepo-cli serve-devices …` use it; the server answers it (gamerepo-serve's
//! `control.rs`).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The header carrying the secret.
pub const SECRET: &str = "X-Game-Repo-Secret";

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

/// The server's data folder on this computer (`%LOCALAPPDATA%\io.github.drflgd.gamerepo.server\`
/// on Windows, as Tauri names it for the server's window; its installer's own folder is
/// another): one server per computer, whoever starts it.
pub fn default_data() -> PathBuf {
    const ID: &str = "io.github.drflgd.gamerepo.server";
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join(ID)
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_default()
            .join(ID)
    }
}

pub fn read_running(data: &Path) -> Option<Running> {
    serde_json::from_slice(&std::fs::read(data.join("server.json")).ok()?).ok()
}

/// The server running for this data folder, if one answers.
pub fn running(data: &Path) -> Option<Running> {
    let r = read_running(data)?;
    call_at(&r, "GET", "/status", None).ok().map(|_| r)
}

/// Why the control channel didn't answer as asked.
#[derive(Debug)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Ask the server running for `data`.
pub fn call(data: &Path, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Refused> {
    let r = read_running(data).ok_or_else(|| Refused("Game Repo Server isn't running.".into()))?;
    call_at(&r, method, path, body)
}

pub fn call_at(r: &Running, method: &str, path: &str, body: Option<&Value>) -> Result<Value, Refused> {
    let no_answer = |_| Refused("Game Repo Server isn't answering.".into());
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], r.control));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(no_answer)?;
    s.set_read_timeout(Some(Duration::from_secs(30))).map_err(no_answer)?;
    let body = body.map(|b| serde_json::to_vec(b).unwrap_or_default()).unwrap_or_default();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{SECRET}: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", r.secret, body.len()).map_err(no_answer)?;
    s.write_all(&body).map_err(no_answer)?;
    let mut all = vec![];
    s.read_to_end(&mut all).map_err(no_answer)?;
    let text = String::from_utf8_lossy(&all);
    let (head, rest) = text.split_once("\r\n\r\n").ok_or_else(|| Refused("Game Repo Server's answer can't be read.".into()))?;
    let status: u16 = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    let v: Value = serde_json::from_str(rest).unwrap_or(Value::Null);
    if status != 200 {
        return Err(Refused(v["error"].as_str().unwrap_or("Game Repo Server refused that.").to_string()));
    }
    Ok(v)
}
