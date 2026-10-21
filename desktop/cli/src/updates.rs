//! A stand-in for the updates of Game Repo, Game Repo Server and Game Repo Go (gamerepo-update
//! needs the program's window), for the page tests: `update_state`, `update_check`,
//! `update_install` and `update_set` answered as the programs answer them. The test says what
//! a check finds and how the copy stands with POST /test/update ({"found": {"version",
//! "notes"}, "can", "why_not", "error", "title"}), and reads back what the page asked for with
//! GET /test/update ("checks", "installs", "check_at_start").

use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct StandIn(Mutex<Value>);

/// What the programs answer (gamerepo_update::shown), without the test's own.
const SHOWN: &[&str] = &["title", "version", "can", "why_not", "check_at_start", "checking", "checked", "available", "error", "installing", "restarting", "releases"];

impl StandIn {
    pub fn new(title: &str) -> Self {
        StandIn(Mutex::new(json!({
            "title": title,
            "version": env!("CARGO_PKG_VERSION"),
            "can": true,
            "why_not": null,
            "check_at_start": true,
            "checking": false,
            "checked": null,
            "available": null,
            "error": null,
            "installing": null,
            "restarting": false,
            "releases": format!("https://github.com/{}/releases", gamerepo_wire::RELEASES),
            "found": null,
            "checks": 0,
            "installs": 0,
        })))
    }

    pub fn call(&self, cmd: &str, args: &Value) -> Result<Value, String> {
        let mut s = self.0.lock().unwrap();
        let can = s["can"] == true;
        match cmd {
            // the check a program makes as it starts, done by the time the page asks to wait for it
            "update_state" => {
                if args["wait"] == true && can && s["check_at_start"] == true && s["checked"].is_null() {
                    check(&mut s);
                }
            }
            "update_check" => {
                if !can {
                    return Err(s["why_not"].as_str().unwrap_or("Updates aren't set up here.").to_string());
                }
                check(&mut s);
            }
            "update_install" => {
                if s["available"].is_null() {
                    return Err("Check for updates first.".into());
                }
                s["installs"] = json!(s["installs"].as_u64().unwrap_or(0) + 1);
                s["restarting"] = json!(true);
            }
            "update_set" => {
                if let Some(on) = args["check_at_start"].as_bool() {
                    s["check_at_start"] = json!(on);
                }
            }
            _ => return Err(format!("unknown command {cmd}")),
        }
        Ok(shown(&s))
    }

    /// POST /test/update: what a check finds and how this copy stands.
    pub fn set(&self, v: &Value) {
        let mut s = self.0.lock().unwrap();
        for (k, x) in v.as_object().into_iter().flatten() {
            s[k] = x.clone();
        }
    }

    /// GET /test/update
    pub fn read(&self) -> Value {
        self.0.lock().unwrap().clone()
    }
}

fn check(s: &mut Value) {
    s["checks"] = json!(s["checks"].as_u64().unwrap_or(0) + 1);
    s["checked"] = json!(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default());
    s["available"] = s["found"].clone();
}

fn shown(s: &Value) -> Value {
    Value::Object(SHOWN.iter().map(|k| (k.to_string(), s[*k].clone())).collect())
}
