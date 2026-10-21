//! Updates for Game Repo, Game Repo Server and Game Repo Go on Windows and Linux
//! (docs/PLAN.md, "In-app updates", D134–D137). Each release carries an update file per
//! program, `latest-<name>.json` (tools/update_files.py writes them): its version, what's
//! new, and for each system its installer's address and signature. A program reads its file
//! from the newest release on GitHub ([`RELEASES`]) a few seconds after it starts, unless
//! Settings says not to, and whenever asked; nothing is installed without a click. Before it
//! runs, the installer is checked against the public key built into the program
//! (tauri.conf.json's `plugins.updater.pubkey`, which CI fills in) and against the version it
//! was signed for. On Windows the installer then runs with only a progress bar and starts the
//! program again; on Linux the AppImage is replaced and started again. A copy built without
//! the key says so and never checks. The commands (`update_state`, `update_check`,
//! `update_install`, `update_set`) come through each program's own `api` command, and
//! web/ui/updates.js shows them.

use gamerepo_wire::RELEASES;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// Tauri's updater, set up from tauri.conf.json's `plugins.updater`: the public key, and that
/// an installer has to be signed for the version its update file announces.
pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new().build()
}

/// A program, as its updates need to know it.
pub struct Program {
    /// Its update file in each release: latest-<name>.json
    pub name: &'static str,
    /// As people know it ("Game Repo")
    pub title: &'static str,
    /// The systems its releases carry an installer for ("windows", "linux")
    pub systems: &'static [&'static str],
    /// Why this copy doesn't update itself, when the program knows (the copy of Game Repo
    /// Server beside Game Repo updates with Game Repo)
    pub not_here: Option<String>,
    /// Run just before the installer replaces it: serving stops, downloads end
    pub before: Box<dyn Fn(&AppHandle) + Send + Sync>,
    /// Told what changed after each check and once an update is downloaded (Game Repo Server's
    /// tray menu)
    pub changed: Box<dyn Fn(&AppHandle, &Value) + Send + Sync>,
}

struct Updates {
    program: Program,
    /// updates.json in the program's settings folder: whether it checks when it starts
    settings: PathBuf,
    why_not: Option<String>,
    now: Mutex<Now>,
    found: Mutex<Option<Update>>,
}

#[derive(Default)]
struct Now {
    check_at_start: bool,
    checking: bool,
    /// When it last checked (seconds since 1970)
    checked: Option<u64>,
    /// The newer release: { version, notes, date }
    available: Option<Value>,
    error: Option<String>,
    /// The update's bytes downloaded so far, and its size when GitHub says
    installing: Option<(u64, Option<u64>)>,
    /// Downloaded and checked: the installer's running or the program's starting again
    restarting: bool,
}

/// The update file a program reads, in the newest release.
pub fn endpoint(name: &str) -> String {
    format!("https://github.com/{RELEASES}/releases/latest/download/latest-{name}.json")
}

/// Sets up a program's updates (in Tauri's setup) and, unless Settings says not to, checks a
/// few seconds later.
pub fn manage(app: &AppHandle, program: Program) {
    let settings = app.path().app_config_dir().unwrap_or_default().join("updates.json");
    let check_at_start = checks_at_start(&settings);
    let pubkey = app.config().plugins.0.get("updater").and_then(|u| u.get("pubkey")).and_then(Value::as_str).unwrap_or_default().to_string();
    let why_not = program.not_here.clone().or_else(|| why_not(program.title, &pubkey, std::env::consts::OS, program.systems, installed()));
    let start = why_not.is_none() && check_at_start;
    app.manage(Updates { program, settings, why_not, now: Mutex::new(Now { check_at_start, checking: start, ..Default::default() }), found: Mutex::new(None) });
    if start {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // after the program's own start
            tokio::time::sleep(Duration::from_secs(3)).await;
            check(&app).await;
        });
    }
}

/// One of the update commands, answered with how updates stand (see [`shown`]).
pub async fn call(app: &AppHandle, cmd: &str, args: &Value) -> Result<Value, String> {
    let u = app.try_state::<Updates>().ok_or("Updates aren't part of this program.")?;
    match cmd {
        // `wait`: once the check it's doing (when it starts) is done
        "update_state" => {
            if args["wait"].as_bool() == Some(true) {
                done_checking(&u).await;
            }
        }
        "update_check" => {
            if let Some(why) = &u.why_not {
                return Err(why.clone());
            }
            if u.now.lock().unwrap().checking {
                done_checking(&u).await;
            } else {
                check(app).await;
            }
        }
        "update_install" => install(app)?,
        "update_set" => {
            if let Some(on) = args["check_at_start"].as_bool() {
                save_settings(&u.settings, on).map_err(|e| format!("Couldn't save it: {e}"))?;
                u.now.lock().unwrap().check_at_start = on;
            }
        }
        _ => return Err(format!("unknown command {cmd}")),
    }
    Ok(shown(app))
}

/// How updates stand, for the page: whether this copy can update itself (and why not), the
/// newer release found, a check or a download under way, and the releases' page.
pub fn shown(app: &AppHandle) -> Value {
    let Some(u) = app.try_state::<Updates>() else { return Value::Null };
    let n = u.now.lock().unwrap();
    json!({
        "title": u.program.title,
        "version": app.package_info().version.to_string(),
        "can": u.why_not.is_none(),
        "why_not": u.why_not,
        "check_at_start": n.check_at_start,
        "checking": n.checking,
        "checked": n.checked,
        "available": n.available,
        "error": n.error,
        "installing": n.installing.map(|(done, size)| json!({ "done": done, "size": size })),
        "restarting": n.restarting,
        "releases": format!("https://github.com/{RELEASES}/releases"),
    })
}

async fn done_checking(u: &Updates) {
    for _ in 0..450 {
        if !u.now.lock().unwrap().checking {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn tell(app: &AppHandle) {
    if let Some(u) = app.try_state::<Updates>() {
        (u.program.changed)(app, &shown(app));
    }
}

async fn check(app: &AppHandle) {
    let u = app.state::<Updates>();
    {
        let mut n = u.now.lock().unwrap();
        n.checking = true;
        n.error = None;
    }
    let got = find(app, u.program.name).await;
    let available = match &got {
        Ok(Some(up)) => Some(json!({ "version": up.version, "notes": up.body.clone().unwrap_or_default(), "date": up.raw_json.get("pub_date") })),
        _ => None,
    };
    {
        let mut n = u.now.lock().unwrap();
        n.checking = false;
        n.checked = Some(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default());
        match &got {
            Ok(_) => n.available = available,
            Err(e) => n.error = Some(e.clone()),
        }
    }
    if let Ok(found) = got {
        *u.found.lock().unwrap() = found;
    }
    tell(app);
}

/// The newer release in the program's update file, if there is one.
async fn find(app: &AppHandle, name: &str) -> Result<Option<Update>, String> {
    let url: tauri::Url = endpoint(name).parse().map_err(|e| format!("{e}"))?;
    let updater = app.updater_builder().endpoints(vec![url]).and_then(|b| b.build()).map_err(|e| format!("Couldn't check for updates: {e}"))?;
    // (a time limit on the check alone: the updater's own would cut the download short too)
    match tokio::time::timeout(Duration::from_secs(45), updater.check()).await {
        Ok(r) => r.map_err(|e| format!("Couldn't check for updates: {e}")),
        Err(_) => Err("Couldn't check for updates: GitHub didn't answer.".into()),
    }
}

/// Downloads the update found, checks it, then runs it (Windows: its installer, as this
/// program ends) or puts it in place and starts again (Linux). The page follows it with
/// `update_state`.
fn install(app: &AppHandle) -> Result<(), String> {
    let u = app.state::<Updates>();
    let up = u.found.lock().unwrap().clone().ok_or("Check for updates first.")?;
    {
        let mut n = u.now.lock().unwrap();
        if n.installing.is_some() || n.restarting {
            return Ok(());
        }
        n.installing = Some((0, None));
        n.error = None;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let u = app.state::<Updates>();
        let got = up
            .download(
                |chunk, size| {
                    if let Some((done, total)) = u.now.lock().unwrap().installing.as_mut() {
                        *done += chunk as u64;
                        *total = size;
                    }
                },
                || {},
            )
            .await;
        let failed = match got {
            Ok(bytes) => {
                u.now.lock().unwrap().restarting = true;
                tell(&app);
                (u.program.before)(&app);
                // on Windows a started installer ends this program here
                match up.install(bytes) {
                    Ok(()) => app.restart(),
                    Err(e) => format!("The update didn't install: {e}"),
                }
            }
            Err(e) => format!("The update didn't download: {e}"),
        };
        {
            let mut n = u.now.lock().unwrap();
            n.installing = None;
            n.restarting = false;
            n.error = Some(failed);
        }
        tell(&app);
    });
    Ok(())
}

/// Makes the key that signs updates in `dir` (tools/updater_key.cmd runs `game-repo.exe
/// --make-update-key <folder>`, D135): a minisign key pair as the Tauri CLI makes one, its
/// secret key encrypted with `password`. Written as CI reads them: `private.key` (the secret
/// TAURI_SIGNING_PRIVATE_KEY) and `public.key` (the public key built into the programs).
/// Never over a key that's there.
pub fn make_key(dir: &Path, password: &str) -> Result<(), String> {
    use base64::Engine;
    let (private, public) = (dir.join("private.key"), dir.join("public.key"));
    if private.exists() || public.exists() {
        return Err(format!("There's a key in {} already.", dir.display()));
    }
    if password.len() < 16 {
        return Err("The key's password is too short.".into());
    }
    let text = |e: minisign::PError| e.to_string();
    let pair = minisign::KeyPair::generate_encrypted_keypair(Some(password.to_string())).map_err(text)?;
    let b64 = |s: String| base64::engine::general_purpose::STANDARD.encode(s);
    let sk = b64(pair.sk.to_box(None).map_err(text)?.to_string());
    let pk = b64(pair.pk.to_box().map_err(text)?.to_string());
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(&private, sk).map_err(|e| e.to_string())?;
    std::fs::write(&public, pk).map_err(|e| e.to_string())
}

/// Copies the updater can replace: on Windows one its installer put there (with its
/// uninstaller beside it), on Linux an AppImage.
fn installed() -> bool {
    if cfg!(windows) {
        std::env::current_exe().is_ok_and(|e| e.with_file_name("uninstall.exe").is_file())
    } else if cfg!(target_os = "linux") {
        std::env::var_os("APPIMAGE").is_some()
    } else {
        false
    }
}

/// Why a copy of a program can't update itself, if it can't.
fn why_not(title: &str, pubkey: &str, os: &str, systems: &[&str], installed: bool) -> Option<String> {
    if !systems.contains(&os) {
        let on: Vec<&str> = systems.iter().map(|s| if *s == "windows" { "Windows" } else { "Linux" }).collect();
        return Some(format!("{title} updates itself on {} only.", on.join(" and ")));
    }
    if pubkey.trim().is_empty() {
        return Some(format!("Updates aren't set up in this copy of {title}: it was built without the key that signs them."));
    }
    if !installed {
        return Some(if os == "linux" {
            format!("Only {title}'s AppImage updates itself.")
        } else {
            format!("This copy of {title} wasn't put here by its installer, so it can't update itself.")
        });
    }
    None
}

fn checks_at_start(settings: &Path) -> bool {
    let v: Value = std::fs::read(settings).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    v["check_at_start"].as_bool().unwrap_or(true)
}

fn save_settings(settings: &Path, check_at_start: bool) -> std::io::Result<()> {
    if let Some(dir) = settings.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(settings, serde_json::to_vec_pretty(&json!({ "check_at_start": check_at_start }))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_copy_says_why_it_cant_update_itself() {
        let key = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWdu";
        assert_eq!(why_not("Game Repo", key, "windows", &["windows"], true), None);
        assert_eq!(why_not("Game Repo Go", key, "linux", &["windows", "linux"], true), None);
        assert_eq!(why_not("Game Repo", key, "linux", &["windows"], true).unwrap(), "Game Repo updates itself on Windows only.");
        assert!(why_not("Game Repo", "", "windows", &["windows"], true).unwrap().contains("built without the key"));
        assert!(why_not("Game Repo", key, "windows", &["windows"], false).unwrap().contains("wasn't put here by its installer"));
        assert_eq!(why_not("Game Repo Go", key, "linux", &["windows", "linux"], false).unwrap(), "Only Game Repo Go's AppImage updates itself.");
    }

    #[test]
    fn checking_at_start_is_on_until_turned_off() {
        let dir = std::env::temp_dir().join(format!("gamerepo-update-{}", std::process::id()));
        let file = dir.join("config/updates.json");
        assert!(checks_at_start(&file));
        save_settings(&file, false).unwrap();
        assert!(!checks_at_start(&file));
        save_settings(&file, true).unwrap();
        assert!(checks_at_start(&file));
        std::fs::write(&file, "not json").unwrap();
        assert!(checks_at_start(&file));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_key_made_signs_what_the_updater_accepts() {
        use base64::Engine;
        let dir = std::env::temp_dir().join(format!("gamerepo-update-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let password = "abcdefghijkmnopqrstuvwxyz23456789";
        assert!(make_key(&dir, "short").is_err());
        make_key(&dir, password).unwrap();
        assert!(make_key(&dir, password).unwrap_err().contains("already"));
        let read = |name: &str| String::from_utf8(base64::engine::general_purpose::STANDARD.decode(std::fs::read(dir.join(name)).unwrap()).unwrap()).unwrap();
        // signed as the Tauri CLI signs an installer: its version in the trusted comment
        let sk = minisign::SecretKeyBox::from_string(&read("private.key")).unwrap().into_secret_key(Some(password.into())).unwrap();
        let data = b"an installer".to_vec();
        let sig = minisign::sign(None, &sk, std::io::Cursor::new(&data), Some("timestamp:1\tfile:Game Repo_1.2.3_x64-setup.exe\tversion:1.2.3"), None).unwrap().to_string();
        let pk = minisign_verify::PublicKey::decode(&read("public.key")).unwrap();
        let signature = minisign_verify::Signature::decode(&sig).unwrap();
        pk.verify(&data, &signature, true).unwrap();
        assert!(signature.trusted_comment().ends_with("version:1.2.3"));
        assert!(pk.verify(b"another installer", &signature, true).is_err());
        // the wrong password opens nothing
        assert!(minisign::SecretKeyBox::from_string(&read("private.key")).unwrap().into_secret_key(Some("wrong password!!".into())).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_update_file_is_in_the_newest_release() {
        assert_eq!(endpoint("game-repo-go"), format!("https://github.com/{RELEASES}/releases/latest/download/latest-game-repo-go.json"));
        assert!(RELEASES.contains('/'));
    }
}
