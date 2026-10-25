//! Tailscale on the Steam Deck (docs/PLAN.md, "Setting up Tailscale on the Steam Deck",
//! D179): whether it's there, running and signed in, for Settings › Tailscale, and *Set up
//! Tailscale*, which runs Game Repo Go's own installer ([`SETUP`], `tailscale/setup.sh`) in
//! a terminal window as the Deck's user, who types the password there. The installer is
//! built into Game Repo Go, so Go's updates bring it up to date. While a set-up runs, Go
//! watches Tailscale and opens its sign-in page once Tailscale gives one. That Tailscale was
//! set up is kept in `tailscale.json`, so Go can say when a SteamOS update has taken it
//! away. `root` is `/`, or a folder standing in for it in the page test.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The installer, written out beside Go's data and run in the terminal window.
pub const SETUP: &str = include_str!("../tailscale/setup.sh");
/// Where the installer puts Tailscale's `tailscale` (under the root).
const OURS: &str = "opt/tailscale/tailscale";
/// How long a set-up is watched for its sign-in page.
const WATCH: Duration = Duration::from_secs(30 * 60);
/// Terminals the set-up can run in, with what comes before the command: Konsole (the Steam
/// Deck's Desktop Mode), then others a Linux PC may have.
const TERMINALS: &[(&str, &[&str])] = &[("konsole", &["-e"]), ("gnome-terminal", &["--"]), ("xfce4-terminal", &["-x"]), ("xterm", &["-e"])];
/// What an AppImage sets for its own libraries, which the programs Go starts mustn't get.
const APPIMAGE_ENV: &[&str] = &[
    "APPDIR", "APPIMAGE", "ARGV0", "OWD", "LD_LIBRARY_PATH", "LD_PRELOAD", "GDK_PIXBUF_MODULE_FILE", "GDK_PIXBUF_MODULEDIR", "GIO_MODULE_DIR",
    "GIO_EXTRA_MODULES", "GSETTINGS_SCHEMA_DIR", "GTK_DATA_PREFIX", "GTK_EXE_PREFIX", "GTK_IM_MODULE_FILE", "GTK_PATH", "PYTHONHOME", "PYTHONPATH",
    "PERLLIB", "QT_PLUGIN_PATH", "WEBKIT_DISABLE_DMABUF_RENDERER",
];

static WATCHING: AtomicBool = AtomicBool::new(false);

/// What Go remembers, in `tailscale.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Record {
    /// Tailscale was seen where the installer puts it
    set_up: bool,
    /// When a set-up was last started from Go
    started: i64,
    /// The sign-in page last opened
    opened: String,
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn record(data: &Path) -> Record {
    std::fs::read(data.join("tailscale.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(data: &Path, r: &Record) -> Result<()> {
    crate::write_whole(&data.join("tailscale.json"), &serde_json::to_vec_pretty(r)?)
}

/// Tailscale's `tailscale`: the installer's, else one on the PATH; and whether it's the
/// installer's.
fn program(root: &Path) -> Option<(PathBuf, bool)> {
    let ours = root.join(OURS);
    if ours.is_file() {
        return Some((ours, true));
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("tailscale")).find(|p| p.is_file()).map(|p| (p, false))
}

/// Whether this is SteamOS, by its `os-release`.
fn steamos(root: &Path) -> bool {
    let text = std::fs::read_to_string(root.join("etc/os-release")).or_else(|_| std::fs::read_to_string(root.join("usr/lib/os-release"))).unwrap_or_default();
    text.lines().filter_map(|l| l.split_once('=')).any(|(k, v)| (k == "ID" || k == "ID_LIKE") && v.trim_matches(['"', '\'']).split_whitespace().any(|x| x == "steamos"))
}

/// A program's output, if it finished within `wait` (else it's stopped).
fn output(program: &Path, args: &[&str], wait: Duration) -> Option<std::process::Output> {
    let child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(wait) {
        Ok(r) => r.ok(),
        Err(_) => {
            let _ = Command::new("kill").arg(id.to_string()).stdout(Stdio::null()).stderr(Stdio::null()).status();
            None
        }
    }
}

/// What `tailscale status --json` says, as the page needs it.
pub fn parse_status(v: &Value) -> Value {
    let me = &v["Self"];
    let ips: Vec<&str> = me["TailscaleIPs"].as_array().into_iter().flatten().filter_map(|x| x.as_str()).collect();
    json!({
        "state": v["BackendState"].as_str().unwrap_or(""),
        "auth_url": v["AuthURL"].as_str().unwrap_or(""),
        "ip": ips.iter().find(|ip| ip.contains('.')).or(ips.first()).copied().unwrap_or(""),
        "name": me["HostName"].as_str().unwrap_or(""),
        "dns": me["DNSName"].as_str().unwrap_or("").trim_end_matches('.'),
        "tailnet": v["CurrentTailnet"]["Name"].as_str().unwrap_or(""),
        "version": v["Version"].as_str().unwrap_or("").split('-').next().unwrap_or(""),
    })
}

/// Whether a user has a password, from a line of `passwd -S`: none, set, locked, or "" when
/// it can't be told.
pub fn password_state(line: &str) -> &'static str {
    match line.split_whitespace().nth(1).unwrap_or("") {
        "NP" => "none",
        "P" | "PS" => "set",
        "L" | "LK" => "locked",
        _ => "",
    }
}

fn user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default()
}

fn password() -> &'static str {
    let me = user();
    if me.is_empty() {
        return "";
    }
    output(Path::new("passwd"), &["-S", &me], Duration::from_secs(3)).map(|o| password_state(&String::from_utf8_lossy(&o.stdout))).unwrap_or("")
}

/// A PATH-like list without what's inside the AppImage Go runs from.
fn outside_appimage(list: &std::ffi::OsStr) -> std::ffi::OsString {
    let appdir = std::env::var_os("APPDIR").map(PathBuf::from).filter(|d| !d.as_os_str().is_empty());
    let kept: Vec<PathBuf> = std::env::split_paths(list).filter(|p| appdir.as_ref().is_none_or(|d| !p.starts_with(d))).collect();
    std::env::join_paths(kept).unwrap_or_default()
}

/// A program to start with the user's own surroundings, not the AppImage's, on its own:
/// it outlives Go, and nothing waits on it here.
fn start(program: &Path, args: &[&str]) -> Result<()> {
    let mut c = Command::new(program);
    c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    for k in APPIMAGE_ENV {
        c.env_remove(k);
    }
    for k in ["PATH", "XDG_DATA_DIRS", "XDG_CONFIG_DIRS"] {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, outside_appimage(&v));
        }
    }
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut c, 0);
    let mut child = c.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// A program on the PATH (as the programs Go starts see it).
fn on_path(name: &str) -> Option<PathBuf> {
    let path = outside_appimage(&std::env::var_os("PATH")?);
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The terminal the set-up runs in, with what comes before the command.
fn terminal() -> Option<(PathBuf, &'static str, &'static [&'static str])> {
    TERMINALS.iter().find_map(|(name, before)| on_path(name).map(|p| (p, *name, *before)))
}

/// For Settings › Tailscale: SteamOS or not, Tailscale found (and whether it's the
/// installer's), what it's doing, the Deck's password, the terminal, and whether a set-up's
/// under way or Tailscale has gone since it was set up.
pub fn status(data: &Path, root: &Path) -> Value {
    let mut r = record(data);
    let found = program(root);
    let answer = found.as_ref().and_then(|(p, _)| output(p, &["status", "--json"], Duration::from_secs(5))).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
    let ours = found.as_ref().is_some_and(|(_, o)| *o);
    if ours && !r.set_up {
        r.set_up = true;
        let _ = save(data, &r);
    }
    let mut v = answer.as_ref().map(parse_status).unwrap_or_else(|| parse_status(&Value::Null));
    let running = v["state"] == "Running";
    let extra = json!({
        "steamos": steamos(root),
        "found": found.is_some(),
        "ours": ours,
        "answers": answer.is_some(),
        "missing": r.set_up && found.is_none(),
        "setting_up": r.started > 0 && now() - r.started < WATCH.as_secs() as i64 && !running,
        "password": password(),
        "terminal": terminal().map(|(_, name, _)| name).unwrap_or(""),
    });
    for (k, x) in extra.as_object().into_iter().flatten() {
        v[k] = x.clone();
    }
    v
}

/// *Set up Tailscale*: the installer written out to `tailscale/setup.sh` in Go's data and
/// started in a terminal window.
pub fn set_up(data: &Path) -> Result<()> {
    let Some((term, name, before)) = terminal() else {
        bail!("There's no terminal program here to run the set-up in (Konsole, which the Steam Deck's Desktop Mode has).");
    };
    let script = data.join("tailscale").join("setup.sh");
    crate::write_whole(&script, SETUP.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))?;
    }
    let script = script.display().to_string();
    let args: Vec<&str> = before.iter().copied().chain(["bash", script.as_str()]).collect();
    if let Err(e) = start(&term, &args) {
        bail!("{name} didn't start: {e}");
    }
    let mut r = record(data);
    r.started = now();
    r.opened.clear();
    save(data, &r)
}

/// Tailscale's sign-in page in the user's browser.
pub fn open(url: &str) -> Result<()> {
    if !url.starts_with("https://") {
        bail!("That isn't Tailscale's sign-in page.");
    }
    let Some(opener) = on_path("xdg-open") else { bail!("There's no xdg-open here to open the browser with: go to {url}") };
    start(&opener, &[url])
}

/// While a set-up runs (from Go, for half an hour at most): Tailscale's sign-in page opened
/// once Tailscale gives one, and `changed` told as what Tailscale's doing changes, until
/// it's signed in. One watch at a time.
pub fn watch(data: PathBuf, root: PathBuf, changed: Box<dyn Fn() + Send>) {
    if WATCHING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let end = Instant::now() + WATCH;
        let mut last = Value::Null;
        while Instant::now() < end {
            std::thread::sleep(Duration::from_secs(2));
            let v = status(&data, &root);
            let url = v["auth_url"].as_str().unwrap_or("").to_string();
            let mut r = record(&data);
            if v["state"] == "NeedsLogin" && !url.is_empty() && url != r.opened && open(&url).is_ok() {
                r.opened = url;
                let _ = save(&data, &r);
            }
            let now = json!([v["found"], v["state"], v["auth_url"], v["ip"]]);
            if now != last {
                changed();
                last = now;
            }
            if v["state"] == "Running" || !v["setting_up"].as_bool().unwrap_or(false) {
                break;
            }
        }
        WATCHING.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailscale_status_and_passwd_are_read_as_the_page_needs_them() {
        let v = json!({
            "Version": "1.104.2-t1234abcd", "BackendState": "Running", "AuthURL": "",
            "Self": { "HostName": "steamdeck", "DNSName": "steamdeck.tail1234.ts.net.", "TailscaleIPs": ["fd7a:115c:a1e0::1", "100.101.102.103"] },
            "CurrentTailnet": { "Name": "jerred@example.com" }
        });
        assert_eq!(
            parse_status(&v),
            json!({ "state": "Running", "auth_url": "", "ip": "100.101.102.103", "name": "steamdeck", "dns": "steamdeck.tail1234.ts.net", "tailnet": "jerred@example.com", "version": "1.104.2" })
        );
        let login = parse_status(&json!({ "BackendState": "NeedsLogin", "AuthURL": "https://login.tailscale.com/a/abc" }));
        assert_eq!((login["state"].as_str(), login["auth_url"].as_str(), login["ip"].as_str()), (Some("NeedsLogin"), Some("https://login.tailscale.com/a/abc"), Some("")));
        assert_eq!(password_state("deck NP 2024-01-01 0 99999 7 -1"), "none");
        assert_eq!(password_state("deck P 10/24/2026 0 99999 7 -1"), "set");
        assert_eq!(password_state("deck L 2024-01-01 0 99999 7 -1"), "locked");
        assert_eq!(password_state("passwd: Permission denied."), "");
    }

    #[test]
    fn steamos_is_known_by_its_os_release() {
        let dir = std::env::temp_dir().join(format!("gamerepo-go-steamos-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("etc")).unwrap();
        std::fs::write(dir.join("etc/os-release"), "NAME=\"SteamOS\"\nID=steamos\nID_LIKE=arch\nVARIANT_ID=steamdeck\n").unwrap();
        assert!(steamos(&dir));
        std::fs::write(dir.join("etc/os-release"), "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n").unwrap();
        assert!(!steamos(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The installer itself, into a folder standing in for `/`, from a stand-in for
    /// Tailscale's server, with stand-ins for systemctl, passwd and sudo on the PATH.
    #[cfg(target_os = "linux")]
    mod installer {
        use super::super::SETUP;
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::process::Command;

        struct Bench {
            dir: PathBuf,
            root: PathBuf,
            bin: PathBuf,
            script: PathBuf,
        }

        fn executable(path: &Path, text: &str) {
            std::fs::write(path, text).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        /// A tarball laid out as Tailscale's are, its `tailscale` noting what it's asked
        /// in `asked` and answering `status` as `state` says.
        fn bench(name: &str, sha_right: bool) -> Bench {
            let dir = std::env::temp_dir().join(format!("gamerepo-go-tailscale-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let (root, bin, server) = (dir.join("root"), dir.join("bin"), dir.join("server"));
            let made = dir.join("made/tailscale_1.104.2_amd64");
            for d in [&root, &bin, &server, &made.join("systemd")] {
                std::fs::create_dir_all(d).unwrap();
            }
            let log = dir.join("asked");
            let state = dir.join("state");
            executable(
                &made.join("tailscale"),
                &format!(
                    "#!/bin/sh\necho \"tailscale $*\" >> '{log}'\ncase \"$1\" in\n  status) echo \"{{\\\"BackendState\\\": \\\"$(cat '{state}' 2>/dev/null || echo NeedsLogin)\\\"}}\" ;;\n  up) echo Running > '{state}' ;;\n  ip) echo 100.101.102.103 ;;\nesac\n",
                    log = log.display(),
                    state = state.display()
                ),
            );
            executable(&made.join("tailscaled"), "#!/bin/sh\n");
            std::fs::write(
                made.join("systemd/tailscaled.service"),
                "[Service]\nEnvironmentFile=/etc/default/tailscaled\nExecStart=/usr/sbin/tailscaled --state=/var/lib/tailscale/tailscaled.state --socket=/run/tailscale/tailscaled.sock --port=${PORT} $FLAGS\nExecStopPost=/usr/sbin/tailscaled --cleanup\n\n[Install]\nWantedBy=multi-user.target\n",
            )
            .unwrap();
            std::fs::write(made.join("systemd/tailscaled.defaults"), "PORT=\"41641\"\nFLAGS=\"\"\n").unwrap();
            let tgz = server.join("tailscale_1.104.2_amd64.tgz");
            assert!(Command::new("tar").arg("-czf").arg(&tgz).arg("-C").arg(dir.join("made")).arg("tailscale_1.104.2_amd64").status().unwrap().success());
            let sha = hex::encode(Sha256::digest(std::fs::read(&tgz).unwrap()));
            let sha = if sha_right { sha } else { "0".repeat(64) };
            std::fs::write(server.join("tailscale_1.104.2_amd64.tgz.sha256"), format!("{sha}\n")).unwrap();
            std::fs::write(
                server.join("index.json"),
                r#"{"TarballsVersion":"1.104.2","Tarballs":{"386":"tailscale_1.104.2_386.tgz","amd64":"tailscale_1.104.2_amd64.tgz","arm64":"tailscale_1.104.2_arm64.tgz"},"MSIs":{"amd64":"tailscale-setup-1.104.2-amd64.msi"}}"#,
            )
            .unwrap();
            // curl: the server's files by name, `?mode=json` its index
            executable(
                &bin.join("curl"),
                &format!(
                    "#!/bin/sh\nout=\"\"; url=\"\"\nwhile [ $# -gt 0 ]; do case \"$1\" in -o) out=\"$2\"; shift 2 ;; -*) shift ;; *) url=\"$1\"; shift ;; esac; done\nfile=\"${{url##*/}}\"; [ \"$file\" = \"?mode=json\" ] && file=index.json\n[ -f '{server}'/\"$file\" ] || exit 22\nif [ -n \"$out\" ]; then cp '{server}'/\"$file\" \"$out\"; else cat '{server}'/\"$file\"; fi\n",
                    server = server.display()
                ),
            );
            for (name, line) in [("systemctl", "systemctl $*"), ("systemd-sysext", "systemd-sysext $*"), ("passwd", "passwd $*")] {
                executable(&bin.join(name), &format!("#!/bin/sh\necho \"{line}\" >> '{}'\n[ \"$1\" = -S ] && echo \"tester NP 2024-01-01 0 99999 7 -1\"\nexit 0\n", log.display()));
            }
            executable(&bin.join("sudo"), &format!("#!/bin/sh\necho \"sudo $*\" >> '{}'\nexec \"$@\"\n", log.display()));
            let script = dir.join("setup.sh");
            std::fs::write(&script, SETUP).unwrap();
            Bench { dir, root, bin, script }
        }

        impl Bench {
            fn run(&self, args: &[&str]) -> std::process::Output {
                Command::new("bash")
                    .arg(&self.script)
                    .args(args)
                    .env("PATH", format!("{}:{}", self.bin.display(), std::env::var("PATH").unwrap_or_default()))
                    .env("GAMEREPO_ROOT", &self.root)
                    .env("GAMEREPO_TAILSCALE_PACKAGES", "https://pkgs.tailscale.com/stable")
                    .stdin(std::process::Stdio::null())
                    .output()
                    .unwrap()
            }
            fn asked(&self) -> String {
                std::fs::read_to_string(self.dir.join("asked")).unwrap_or_default()
            }
            fn read(&self, path: &str) -> String {
                std::fs::read_to_string(self.root.join(path)).unwrap_or_default()
            }
        }

        #[test]
        fn the_installer_puts_tailscale_where_steamos_keeps_it_and_again_over_it() {
            if Command::new("uname").arg("-m").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() != "x86_64").unwrap_or(true) {
                return;
            }
            let b = bench("install", true);
            let out = b.run(&["install"]);
            assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
            assert!(String::from_utf8_lossy(&out.stdout).contains("Tailscale 1.104.2 is installed."));
            for p in ["opt/tailscale/tailscale", "opt/tailscale/tailscaled"] {
                let m = std::fs::metadata(b.root.join(p)).unwrap();
                assert!(m.permissions().mode() & 0o111 != 0, "{p} runs");
            }
            let unit = b.read("etc/systemd/system/tailscaled.service");
            assert!(unit.contains("ExecStart=/opt/tailscale/tailscaled --state=") && unit.contains("ExecStopPost=/opt/tailscale/tailscaled --cleanup") && !unit.contains("/usr/sbin"), "{unit}");
            assert_eq!(b.read("etc/default/tailscaled"), "PORT=\"41641\"\nFLAGS=\"\"\n");
            let keep = b.read("etc/atomic-update.conf.d/gamerepo-tailscale.conf");
            for p in ["/etc/systemd/system/tailscaled.service", "/etc/systemd/system/multi-user.target.wants/tailscaled.service", "/etc/default/tailscaled"] {
                assert!(keep.lines().any(|l| l == p), "{p} kept through SteamOS updates: {keep}");
            }
            assert_eq!(std::fs::read_link(b.root.join("usr/local/bin/tailscale")).unwrap(), Path::new("/opt/tailscale/tailscale"));
            assert!(b.asked().contains("systemctl daemon-reload\nsystemctl enable tailscaled\nsystemctl restart tailscaled\n"), "{}", b.asked());
            // again, over it: Tailscale's settings as they were, an older system extension gone
            std::fs::write(b.root.join("etc/default/tailscaled"), "PORT=\"41641\"\nFLAGS=\"--verbose=1\"\n").unwrap();
            std::fs::create_dir_all(b.root.join("var/lib/extensions/tailscale/usr/bin")).unwrap();
            let again = b.run(&["install"]);
            assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
            assert_eq!(b.read("etc/default/tailscaled"), "PORT=\"41641\"\nFLAGS=\"--verbose=1\"\n");
            assert!(!b.root.join("var/lib/extensions/tailscale").exists());
            assert!(b.asked().contains("systemd-sysext unmerge"));
            let _ = std::fs::remove_dir_all(&b.dir);
        }

        #[test]
        fn a_download_that_isnt_what_tailscale_published_installs_nothing() {
            if Command::new("uname").arg("-m").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() != "x86_64").unwrap_or(true) {
                return;
            }
            let b = bench("wrong", false);
            let out = b.run(&["install"]);
            assert!(!out.status.success());
            assert!(String::from_utf8_lossy(&out.stderr).contains("SHA-256 doesn't match"), "{}", String::from_utf8_lossy(&out.stderr));
            assert!(!b.root.join("opt").exists() && !b.root.join("etc").exists());
            assert!(!b.asked().contains("systemctl"));
            let _ = std::fs::remove_dir_all(&b.dir);
        }

        #[test]
        fn the_set_up_asks_for_a_password_when_there_is_none_installs_and_signs_in() {
            if Command::new("uname").arg("-m").output().map(|o| String::from_utf8_lossy(&o.stdout).trim() != "x86_64").unwrap_or(true) {
                return;
            }
            let b = bench("whole", true);
            let out = b.run(&[]);
            let said = String::from_utf8_lossy(&out.stdout).to_string();
            assert!(out.status.success(), "{said}{}", String::from_utf8_lossy(&out.stderr));
            let me = String::from_utf8_lossy(&Command::new("id").arg("-un").output().unwrap().stdout).trim().to_string();
            let asked = b.asked();
            let order: Vec<usize> = [format!("passwd -S {me}\n"), "passwd \n".into(), format!("sudo bash {} install\n", b.script.display()), "systemctl restart tailscaled\n".into(), format!("tailscale set --operator={me}\n"), "tailscale up\n".into()]
                .iter()
                .map(|x| asked.find(x.as_str()).unwrap_or_else(|| panic!("{x:?} not in {asked}")))
                .collect();
            assert!(order.windows(2).all(|w| w[0] < w[1]), "in this order: {asked}");
            assert!(said.contains("a password for this Deck") && said.contains("This Deck is 100.101.102.103."), "{said}");
            // set up again once it's signed in: no sign-in this time
            std::fs::remove_file(b.dir.join("asked")).unwrap();
            let again = b.run(&[]);
            assert!(again.status.success());
            assert!(String::from_utf8_lossy(&again.stdout).contains("Tailscale is already signed in."));
            assert!(!b.asked().contains("tailscale up"));
            let _ = std::fs::remove_dir_all(&b.dir);
        }
    }
}
