//! Game Repo's side of Game Repo Server (docs/PLAN.md, "Phase 9a design", D89): starting
//! it with the open library, asking it how it is over its control channel, pairing,
//! removing devices, telling it when the library changed, and Start with Windows. The
//! server is a program of its own (`gamerepo-server`, beside the app; `gamerepo-cli
//! serve-devices` for the tests), so it can keep serving when Game Repo is closed.

use anyhow::{anyhow, bail, Context, Result};
use gamerepo_wire::control::{self, Running};
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub use gamerepo_wire::control::default_data;

/// Start Windows' value for the server (HKCU's Run key).
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Game Repo Server";

/// What starting the server needs.
pub struct Start<'a> {
    /// Its program and the arguments before its own
    pub program: &'a [String],
    pub data: &'a Path,
    pub library: &'a Path,
    pub port: u16,
    /// Game Repo's index of the library, copied for the server's first
    pub index_from: Option<&'a Path>,
    pub keep: bool,
}

/// Start the server and wait until it answers. It stops when Game Repo ends (its input,
/// held by the returned child, closes) unless it's kept.
pub fn start(s: &Start) -> Result<Child> {
    let (program, before) = s.program.split_first().ok_or_else(|| anyhow!("Game Repo Server isn't installed beside Game Repo: install Game Repo again."))?;
    if let Some(r) = control::running(s.data) {
        bail!("Game Repo Server is running already (serving {}).", r.library);
    }
    std::fs::create_dir_all(s.data).with_context(|| format!("couldn't make {}", s.data.display()))?;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(s.data.join("server.log"))?;
    let mut c = Command::new(program);
    c.args(before)
        .arg("--library")
        .arg(s.library)
        .arg("--data")
        .arg(s.data)
        .arg("--port")
        .arg(s.port.to_string())
        .arg("--parent")
        .stdin(Stdio::piped())
        .stdout(log.try_clone()?)
        .stderr(log);
    if let Some(i) = s.index_from {
        c.arg("--index-from").arg(i);
    }
    if s.keep {
        c.arg("--keep");
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = c.spawn().with_context(|| format!("couldn't start {program}"))?;
    let until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < until {
        if control::running(s.data).is_some_and(|r| r.pid == child.id()) {
            return Ok(child);
        }
        if let Some(code) = child.try_wait()? {
            bail!("Game Repo Server stopped as it started ({code}): {}", last_lines(&s.data.join("server.log")));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    bail!("Game Repo Server didn't answer: {}", last_lines(&s.data.join("server.log")))
}

/// The last lines of the server's log, for a message.
fn last_lines(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join(" ")
}

/// Stop the server and wait until it's gone.
pub fn stop(data: &Path) -> Result<()> {
    control::call(data, "POST", "/stop", None)?;
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        if control::running(data).is_none() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("Game Repo Server didn't stop.")
}

/// How the server is, for Settings: running or not, its state and devices.
pub fn state(data: &Path) -> (Option<Running>, Value, Value) {
    let Some(r) = control::read_running(data) else { return (None, Value::Null, json!([])) };
    let Ok(status) = control::call_at(&r, "GET", "/status", None) else { return (None, Value::Null, json!([])) };
    let devices = control::call_at(&r, "GET", "/devices", None).unwrap_or_else(|_| json!([]));
    (Some(r), status, devices)
}

/// Tell the server the library changed (after a job or an edit), without waiting for it.
pub fn changed(data: &Path, library: &Path) {
    let Some(r) = control::read_running(data) else { return };
    if Path::new(&r.library) != library {
        return;
    }
    std::thread::spawn(move || {
        let _ = control::call_at(&r, "POST", "/changed", Some(&json!({})));
    });
}

/// Is the server started with Windows?
pub fn with_windows() -> bool {
    cfg!(windows) && Command::new("reg").args(["query", RUN_KEY, "/v", RUN_VALUE]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Start the server with Windows (it serves the library it served last), or not.
pub fn set_with_windows(program: &Path, on: bool) -> Result<()> {
    if !cfg!(windows) {
        bail!("Only on Windows.");
    }
    let mut c = Command::new("reg");
    if on {
        c.args(["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &format!("\"{}\"", program.display()), "/f"]);
    } else {
        c.args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]);
    }
    let out = c.output()?;
    if !out.status.success() && on {
        bail!("Windows didn't take it: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// The networks this computer is on, and whether each is public (Windows keeps a public
/// network out of the server's firewall rule, which is for private networks only).
pub fn networks() -> Vec<Value> {
    if !cfg!(windows) {
        return vec![];
    }
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Get-NetConnectionProfile | Select-Object Name, InterfaceAlias, @{n='Category';e={[string]$_.NetworkCategory}} | ConvertTo-Json -Compress"])
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else { return vec![] };
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);
    let list = match v {
        Value::Array(a) => a,
        Value::Object(_) => vec![v],
        _ => vec![],
    };
    list.into_iter()
        .map(|n| json!({ "name": n["Name"], "interface": n["InterfaceAlias"], "public": n["Category"].as_str() == Some("Public") }))
        .collect()
}
