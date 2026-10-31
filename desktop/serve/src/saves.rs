//! Saves under `/v1/saves` (docs/PLAN.md, "Save sync and the Devices page", D181, D184):
//! the one place Game Repo Server writes in the library besides the devices' records. A
//! device sends each file of a save first (`PUT /v1/saves/blob/<sha1>`, kept in the
//! server's data folder until it's part of a version), then the save itself, which becomes
//! a version in the library's `Saves/` (gamerepo-core's `saves::Store`). Saves no game
//! there is named like come to `POST /v1/saves/loose` the same way (D195).

use crate::devices::Device;
use crate::files;
use crate::http::{Request, Response};
use anyhow::Result;
use gamerepo_core::library::Library;
use gamerepo_core::saves::Store;
use gamerepo_wire::saves::{SavesAsk, SavesList, Unit, Upload, MAX_FILE};
use sha1::Digest;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Files sent and not made part of a version are dropped after this long.
const KEEP_SENT: Duration = Duration::from_secs(24 * 3600);

fn incoming(data: &Path) -> PathBuf {
    data.join("saves-incoming")
}

fn is_sha1(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether states are synced (Settings › Saves in Game Repo; on unless turned off, D180).
pub fn states_on(lib: &Library) -> bool {
    lib.meta().pointer("/saves/states").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// A request under `/v1/saves`, or None for another.
pub fn route(lib: &Library, data: &Path, req: &Request, device: &Device) -> Option<Result<Response>> {
    let path = req.path.as_str();
    if !path.starts_with("/v1/saves") {
        return None;
    }
    Some(answer(lib, data, req, device))
}

fn answer(lib: &Library, data: &Path, req: &Request, device: &Device) -> Result<Response> {
    let store = Store::of(lib.root());
    let path = req.path.as_str();
    let m = req.method.as_str();
    if let Some(sha1) = path.strip_prefix("/v1/saves/blob/") {
        let sha1 = sha1.to_ascii_lowercase();
        if !is_sha1(&sha1) {
            return Ok(Response::problem(400, "That isn't a SHA-1."));
        }
        let at = incoming(data).join(&sha1);
        return Ok(match m {
            "GET" | "HEAD" if at.is_file() => Response::json(200, &serde_json::json!({})),
            "GET" | "HEAD" => Response::problem(404, "Not sent yet."),
            "PUT" => {
                if req.body.len() as u64 > MAX_FILE {
                    return Ok(Response::problem(413, "That save file is too big."));
                }
                if hex::encode(sha1::Sha1::digest(&req.body)) != sha1 {
                    return Ok(Response::problem(400, "That file isn't the one its SHA-1 says."));
                }
                std::fs::create_dir_all(incoming(data))?;
                let part = at.with_extension("part");
                std::fs::write(&part, &req.body)?;
                std::fs::rename(&part, &at)?;
                Response::json(200, &serde_json::json!({}))
            }
            _ => Response::problem(405, "Not like that."),
        });
    }
    match (m, path) {
        ("POST", "/v1/saves") => {
            let ask: SavesAsk = match serde_json::from_slice(&req.body) {
                Ok(a) => a,
                Err(e) => return Ok(Response::problem(400, format!("That request can't be read: {e}"))),
            };
            Ok(Response::json(200, &SavesList { units: store.list(&ask.keys), states: states_on(lib) }))
        }
        ("POST", "/v1/saves/versions") => {
            let up: Upload = match serde_json::from_slice(&req.body) {
                Ok(u) => u,
                Err(e) => return Ok(Response::problem(400, format!("That request can't be read: {e}"))),
            };
            if up.unit.is_state() && !states_on(lib) {
                return Ok(Response::problem(403, "Save states aren't synced: Game Repo's Settings › Saves has them turned off."));
            }
            let dir = incoming(data);
            let found = |f: &gamerepo_wire::saves::SaveFile| {
                let p = dir.join(f.sha1.to_ascii_lowercase());
                (is_sha1(&f.sha1) && std::fs::metadata(&p).is_ok_and(|m| m.len() == f.size)).then_some(p)
            };
            let r = match store.add(&up, &device.id, &device.name, &found) {
                Ok(r) => Response::json(200, &r),
                Err(e) => Response::problem(400, format!("{e:#}")),
            };
            tidy(&dir);
            Ok(r)
        }
        // saves not matched to a game, kept apart in the library (D195)
        ("POST", "/v1/saves/loose") => {
            let ask: gamerepo_wire::saves::LooseAsk = match serde_json::from_slice(&req.body) {
                Ok(a) => a,
                Err(e) => return Ok(Response::problem(400, format!("That request can't be read: {e}"))),
            };
            let dir = incoming(data);
            for l in &ask.saves {
                let sha1 = l.file.sha1.to_ascii_lowercase();
                let p = dir.join(&sha1);
                if !is_sha1(&sha1) || !std::fs::metadata(&p).is_ok_and(|m| m.len() == l.file.size) {
                    return Ok(Response::problem(400, format!("{} wasn't sent first.", l.file.name)));
                }
                if let Err(e) = store.add_loose(l, &device.id, &device.name, &p) {
                    return Ok(Response::problem(400, format!("{e:#}")));
                }
            }
            tidy(&dir);
            Ok(Response::json(200, &serde_json::json!({})))
        }
        ("GET", "/v1/saves/file") => {
            let unit = req.param("unit").and_then(Unit::from_id);
            let (Some(unit), Some(version), Some(name)) = (unit, req.param("version"), req.param("name")) else {
                return Ok(Response::problem(400, "Which save, version and file?"));
            };
            Ok(match store.file(&unit, version, name) {
                Ok(p) => files::send(req, &p, "application/octet-stream"),
                Err(e) => Response::problem(404, format!("{e:#}")),
            })
        }
        _ => Ok(Response::problem(404, "There's nothing here.")),
    }
}

/// Files sent long ago and never made part of a version.
fn tidy(dir: &Path) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > KEEP_SENT) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
