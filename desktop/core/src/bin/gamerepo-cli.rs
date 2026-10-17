//! Command-line access to the desktop app's backend (same code, no window).
//!
//!   gamerepo-cli --version
//!   gamerepo-cli --help
//!   gamerepo-cli library-info --library <dir>
//!   gamerepo-cli index-info --library <dir> --data <dir>
//!   gamerepo-cli serve --ui <dir> --home <dir> [--library <dir>] [--port 8790]
//!   gamerepo-cli roots add|list|remove …, scan, dat import|list, audit, make-dat
//!   gamerepo-cli organise, duplicates, convert, extract, history, undo, finish, set-aside empty, tools
//!
//! `serve` runs the app's commands behind a local web server, so the page can be
//! tested in an ordinary browser (with tests/tauri_shim.js standing in for Tauri's
//! `invoke`). The library commands (roots, scan, dat, audit, make-dat) work on the
//! app's own library and index unless --library and --home say otherwise; they go
//! through the same commands as the window (gamerepo_core::api).

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

The library commands use the app's open library and its index on this computer;
--library <dir> picks another library, --home <dir> keeps settings and the index
under <dir>/config and <dir>/data instead of the app's own folders.

  roots list                 the folders with games
  roots add <dir> [--platform <id>]
                             add a folder with games (optionally marked with a platform)
  roots remove <id>          stop scanning a folder (nothing on disk is touched)
  scan [--full]              scan the folders with games; --full unpacks and hashes
                             every file in ZIPs instead of trusting their CRC32s
  dat import <file>... [--platform <id>]
                             import DATs (.dat, .xml or a .zip of them); --platform
                             answers for a DAT whose platform isn't recognised
  dat list                   the imported DATs
  dat starter [<platform>...] [--again]
                             download and import the No-Intro cartridge DATs from
                             libretro's public copy, for platforms without a DAT
                             (--again: for every platform named, or all)
  audit <platform> [--dat <id>] [--list <list>] [--search <text>] [--fixdat <file>]
        [--report txt|csv] [--json]
                             the audit of a platform; lists: have, partial, missing,
                             matched, misnamed, baddump, unknown, duplicates
  make-dat <root id> --name <name> [--folder <path in it>] [--platform <id>]
                             write a DAT of a scanned set (and import it with --platform)
  platforms                  the platforms this version knows

  organise <platform> --mode copy|rename|move [--out <dir>] [--format keep|zip|loose]
        [--partial] [--always-copy] [--misnamed] [--dat <id>] [--dry-run]
                             sort a platform's set by its DAT: copy it to --out, rename
                             the files where they are, or move them into the library
                             (or --out); --misnamed renames only the misnamed files;
                             --dry-run shows the plan without changing anything
  duplicates <platform> [--dat <id>] [--dry-run]
                             set aside the extra copies of each matched file
  convert <platform> [--to chd|cso|zso|rvz|zip] [--out <dir>] [--game <name>]...
        [--dat <id>] [--dry-run]
                             convert the platform's games (or those named) to a format
                             (its preferred one by default), into --out or in place
                             (the originals are set aside); every result is checked
  extract <platform> [--out <dir>] [--game <name>]... [--dat <id>] [--dry-run]
                             extract games from their images or archives (CHD to
                             cue/bin or ISO, CSO/ZSO/RVZ to ISO, ZIP/7z/RAR to files)
  formats [<platform> <format>]
                             the preferred format of each platform, or set one
  tools [<tool> <path>|--clear <tool>]
                             the external tools found (dolphin-tool), or set where one is
  history                    the changes made, newest first
  undo <change id>           put a change back (the newest that can be undone)
  finish <change id>         finish a change that was interrupted
  set-aside empty [<change id>]
                             delete the files set aside by a change (or by all of them)
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
        "roots" | "scan" | "dat" | "audit" | "make-dat" | "platforms" | "organise" | "duplicates" | "history" | "undo" | "finish"
        | "set-aside" | "convert" | "extract" | "formats" | "tools" => library_command(&cmd, a).await,
        other => bail!("unknown command {other} (gamerepo-cli --help lists them)"),
    }
}

/// The app's own folders on this computer (as Tauri names them for the window).
fn app_dirs() -> (PathBuf, PathBuf) {
    const ID: &str = "io.github.drflgd.gamerepo";
    let home = gamerepo_core::config::home_dir();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if cfg!(windows) {
        let roaming = env("APPDATA").unwrap_or_else(|| home.join("AppData/Roaming"));
        let local = env("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData/Local"));
        (roaming.join(ID), local.join(ID))
    } else if cfg!(target_os = "macos") {
        let d = home.join("Library/Application Support").join(ID);
        (d.clone(), d)
    } else {
        let config = env("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data = env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        (config.join(ID), data.join(ID))
    }
}

async fn invoke(app: &Arc<App>, cmd: &str, args: Value) -> Result<Value> {
    match app.call(cmd, args).await.map_err(|e| anyhow::anyhow!(e))? {
        Reply::Json(v) => Ok(v),
        Reply::Bytes(_) => bail!("{cmd} answered with bytes"),
    }
}

/// Follow a job to its end, showing its progress on stderr.
async fn follow(app: &Arc<App>, job: Value) -> Result<Value> {
    let id = job["job"].clone();
    let mut last = String::new();
    loop {
        let j = invoke(app, "job", json!({ "id": id })).await?;
        let p = &j["progress"];
        let line = match (p["item"].as_u64(), p["items"].as_u64()) {
            (Some(i), Some(n)) => format!("{} {i} of {n} {}", p["text"].as_str().unwrap_or(""), p["name"].as_str().unwrap_or("")).trim().to_string(),
            _ => p["text"].as_str().unwrap_or("").to_string(),
        };
        if !line.is_empty() && line != last {
            eprint!("\r\x1b[2K{}", line.chars().take(110).collect::<String>());
            last = line;
        }
        if j["done"] == true {
            if !last.is_empty() {
                eprintln!();
            }
            if let Some(e) = j["error"].as_str() {
                bail!("{e}");
            }
            return Ok(j["result"].clone());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn print(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// roots, scan, dat, audit, make-dat and platforms: the window's commands on the app's library.
async fn library_command(cmd: &str, mut a: Args) -> Result<()> {
    let (config_dir, data_dir) = match a.flag("--home") {
        Some(h) => (PathBuf::from(&h).join("config"), PathBuf::from(&h).join("data")),
        None => app_dirs(),
    };
    let app = App::new(AppPaths { config_dir, data_dir, library_url: String::new() })?;
    // one command and done: nothing to watch
    app.watch_folders(false);
    if let Some(lib) = a.flag("--library") {
        // for this run only: the app's own choice of library is kept
        let saved = app.config();
        invoke(&app, "library_open", json!({ "path": lib })).await?;
        let cfg_path = app.paths.config_dir.join("config.json");
        saved.save(&cfg_path)?;
    }
    let json_out = a.rest.iter().any(|x| x == "--json");
    a.rest.retain(|x| x != "--json");
    let sub = if matches!(cmd, "roots" | "dat" | "set-aside") && !a.rest.is_empty() { a.rest.remove(0) } else { String::new() };
    match (cmd, sub.as_str()) {
        ("platforms", _) => print(&invoke(&app, "platforms", json!({})).await?),
        ("roots", "list") | ("roots", "") => {
            let roots = invoke(&app, "roots", json!({})).await?;
            if json_out {
                return print(&roots);
            }
            for r in roots.as_array().into_iter().flatten() {
                println!(
                    "{}  {}{}{}",
                    r["id"].as_str().unwrap_or(""), r["dir"].as_str().unwrap_or(""),
                    r["platform"].as_str().map(|p| format!("  [{p}]")).unwrap_or_default(),
                    if r["found"] == true { "" } else { "  (not found on this computer)" }
                );
            }
            Ok(())
        }
        ("roots", "add") => {
            let platform = a.flag("--platform");
            let dir = a.rest.first().context("roots add needs a folder")?;
            let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| PathBuf::from(dir));
            print(&invoke(&app, "roots_add", json!({ "path": dir.display().to_string(), "platform": platform })).await?)
        }
        ("roots", "remove") => {
            let id = a.rest.first().context("roots remove needs a folder's id (roots list shows them)")?;
            invoke(&app, "roots_remove", json!({ "id": id })).await?;
            Ok(())
        }
        ("scan", _) => {
            let full = a.rest.iter().any(|x| x == "--full");
            let job = invoke(&app, "scan", json!({ "full": full })).await?;
            let r = follow(&app, job).await?;
            if json_out {
                return print(&r);
            }
            println!(
                "{} files: {} new, {} changed, {} moved, {} gone; {} read ({} MB) in {} s{}",
                r["files"], r["added"], r["changed"], r["moved"], r["removed"], r["hashed"],
                r["bytes"].as_u64().unwrap_or(0) / 1_000_000, r["seconds"],
                if r["stopped"] == true { " (stopped)" } else { "" }
            );
            for e in r["errors"].as_array().into_iter().flatten() {
                eprintln!("  couldn't read {}", e.as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("dat", "import") => {
            let platform = a.flag("--platform");
            let paths: Vec<String> = a.rest.iter().map(|p| std::fs::canonicalize(p).map(|p| p.display().to_string()).unwrap_or_else(|_| p.clone())).collect();
            if paths.is_empty() {
                bail!("dat import needs a file");
            }
            let mut answers = serde_json::Map::new();
            let r = follow(&app, invoke(&app, "dat_import", json!({ "paths": paths })).await?).await?;
            let mut all = r.clone();
            if let (Some(p), Some(ask)) = (&platform, r["needs_platform"].as_array().filter(|a| !a.is_empty())) {
                for x in ask {
                    answers.insert(x["name"].as_str().unwrap_or("").to_string(), json!(p));
                }
                let again: Vec<&str> = ask.iter().filter_map(|x| x["path"].as_str()).collect();
                all = follow(&app, invoke(&app, "dat_import", json!({ "paths": again, "answers": answers })).await?).await?;
                all["imported"] = json!([r["imported"].as_array().cloned().unwrap_or_default(), all["imported"].as_array().cloned().unwrap_or_default()].concat());
            }
            if json_out {
                return print(&all);
            }
            for d in all["imported"].as_array().into_iter().flatten() {
                println!(
                    "{} ({}) for {}: {} games, {} matched{}",
                    d["name"].as_str().unwrap_or(""), d["version"].as_str().unwrap_or(""), d["platform_name"].as_str().unwrap_or(""),
                    d["games"], d["matched"], if d["already"] == true { " (already imported)" } else { "" }
                );
            }
            for d in all["needs_platform"].as_array().into_iter().flatten() {
                eprintln!("{}: which platform is it for? Import it again with --platform <id> (gamerepo-cli platforms lists them).", d["name"].as_str().unwrap_or(""));
            }
            for d in all["failed"].as_array().into_iter().flatten() {
                eprintln!("{}: {}", d["path"].as_str().unwrap_or(""), d["error"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("dat", "starter") => {
            let again = a.rest.iter().any(|x| x == "--again");
            let only: Vec<&String> = a.rest.iter().filter(|x| !x.starts_with("--")).collect();
            let r = follow(&app, invoke(&app, "starter_dats", json!({ "platforms": only, "again": again })).await?).await?;
            if json_out {
                return print(&r);
            }
            for d in r["imported"].as_array().into_iter().flatten() {
                println!("{} ({}) for {}: {} games", d["name"].as_str().unwrap_or(""), d["version"].as_str().unwrap_or(""), d["platform_name"].as_str().unwrap_or(""), d["games"]);
            }
            let n = |k: &str| r[k].as_array().map(Vec::len).unwrap_or(0);
            eprintln!("{} imported, {} already had a DAT, {} not found at {}", n("imported"), n("skipped"), n("missing"), r["source"].as_str().unwrap_or(""));
            for f in r["failed"].as_array().into_iter().flatten() {
                eprintln!("{}: {}", f["platform"].as_str().unwrap_or(""), f["error"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("dat", "list") | ("dat", "") => {
            let dats = invoke(&app, "dats", json!({})).await?;
            if json_out {
                return print(&dats);
            }
            for d in dats.as_array().into_iter().flatten() {
                println!(
                    "{:>4}  {:<12} {} ({}){}",
                    d["id"], d["platform"].as_str().unwrap_or(""), d["name"].as_str().unwrap_or(""),
                    d["version"].as_str().unwrap_or(""), if d["current"] == true { "  current" } else { "" }
                );
            }
            Ok(())
        }
        ("audit", _) => {
            let dat = a.flag("--dat").map(|d| d.parse::<i64>()).transpose()?;
            let tab = a.flag("--list");
            let search = a.flag("--search").unwrap_or_default();
            let fix = a.flag("--fixdat");
            let report = a.flag("--report");
            let platform = a.rest.first().cloned();
            let s = invoke(&app, "audit_summary", json!({ "platform": platform, "dat": dat })).await?;
            let sum = &s["summary"];
            if sum.is_null() {
                bail!("No DAT has been imported for {} (dat import adds one).", platform.as_deref().unwrap_or("any platform"));
            }
            if platform.is_some() && sum["dat"]["platform"].as_str() != platform.as_deref() && dat.is_none() {
                bail!("No DAT has been imported for {}.", platform.unwrap_or_default());
            }
            let dat_id = sum["dat"]["id"].clone();
            if let Some(f) = fix {
                let r = invoke(&app, "audit_fixdat", json!({ "dat": dat_id })).await?;
                let from = PathBuf::from(r["path"].as_str().unwrap_or(""));
                std::fs::copy(&from, &f)?;
                let _ = std::fs::remove_file(&from);
                eprintln!("{} missing games written to {f}", r["games"]);
            }
            if let Some(fmt) = report {
                let r = invoke(&app, "audit_save_report", json!({ "dat": dat_id, "format": fmt })).await?;
                eprintln!("report saved to {}", r["path"].as_str().unwrap_or(""));
            }
            if let Some(tab) = tab {
                let mut offset = 0;
                let mut items = vec![];
                loop {
                    let page = invoke(&app, "audit_list", json!({ "dat": dat_id, "tab": tab, "search": search, "offset": offset, "limit": 1000 })).await?;
                    let got = page["items"].as_array().cloned().unwrap_or_default();
                    let n = got.len();
                    items.extend(got);
                    if n < 1000 {
                        break;
                    }
                    offset += 1000;
                }
                if json_out {
                    return print(&json!(items));
                }
                for it in items {
                    let mut line = it["shown"].as_str().or(it["name"].as_str()).unwrap_or("").to_string();
                    if let Some(e) = it["expected"].as_str().filter(|_| tab == "misnamed") {
                        line.push_str(&format!("  ->  {e}"));
                    }
                    println!("{line}");
                }
                return Ok(());
            }
            if json_out {
                return print(sum);
            }
            print!("{}", invoke(&app, "audit_report", json!({ "dat": dat_id })).await?["text"].as_str().unwrap_or(""));
            Ok(())
        }
        ("make-dat", _) => {
            let name = a.need("--name")?;
            let folder = a.flag("--folder").unwrap_or_default();
            let platform = a.flag("--platform");
            let root = a.rest.first().context("make-dat needs a folder with games' id (roots list shows them)")?;
            let r = follow(&app, invoke(&app, "make_dat", json!({ "root": root, "folder": folder, "name": name, "platform": platform })).await?).await?;
            if json_out {
                return print(&r);
            }
            println!("{} games, {} ROMs written to {}", r["games"], r["roms"], r["path"].as_str().unwrap_or(""));
            Ok(())
        }
        ("formats", _) => {
            let v = if a.rest.len() >= 2 {
                invoke(&app, "format_set", json!({ "platform": a.rest[0], "format": a.rest[1] })).await?
            } else {
                invoke(&app, "formats", json!({})).await?
            };
            if json_out {
                return print(&v);
            }
            for p in v.as_array().into_iter().flatten() {
                println!("{:<16} {:<5} (can be: {})", p["platform"].as_str().unwrap_or(""), p["preferred"].as_str().unwrap_or(""),
                    p["formats"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
            }
            Ok(())
        }
        ("tools", _) => {
            let v = if let Some(id) = a.flag("--clear") {
                invoke(&app, "tools_set", json!({ "id": id, "path": null })).await?
            } else if a.rest.len() >= 2 {
                let path = std::path::absolute(&a.rest[1]).map(|p| p.display().to_string()).unwrap_or(a.rest[1].clone());
                invoke(&app, "tools_set", json!({ "id": a.rest[0], "path": path })).await?
            } else {
                invoke(&app, "tools", json!({})).await?
            };
            if json_out {
                return print(&v);
            }
            for t in v["tools"].as_array().into_iter().flatten() {
                println!("{:<14} {}", t["id"].as_str().unwrap_or(""), match t["path"].as_str() {
                    Some(p) => format!("{p}{}", t["version"].as_str().map(|v| format!(" ({v})")).unwrap_or_default()),
                    None => "not found".into(),
                });
            }
            Ok(())
        }
        ("organise", _) | ("duplicates", _) | ("convert", _) | ("extract", _) => {
            let dat = a.flag("--dat").map(|d| d.parse::<i64>()).transpose()?;
            let to = a.flag("--to");
            let mut games = vec![];
            while let Some(g) = a.flag("--game") {
                games.push(g);
            }
            let mode = a.flag("--mode");
            let out = a.flag("--out").map(|o| std::path::absolute(&o).map(|p| p.display().to_string()).unwrap_or(o));
            let format = a.flag("--format").unwrap_or_else(|| "keep".into());
            let has = |f: &str| a.rest.iter().any(|x| x == f);
            let (partial, always_copy, misnamed, dry) = (has("--partial"), has("--always-copy"), has("--misnamed"), has("--dry-run"));
            let platform = a.rest.iter().find(|x| !x.starts_with("--")).cloned().with_context(|| format!("{cmd} needs a platform (gamerepo-cli platforms lists them)"))?;
            let s = invoke(&app, "audit_summary", json!({ "platform": platform, "dat": dat })).await?;
            if s["summary"].is_null() || (dat.is_none() && s["summary"]["dat"]["platform"].as_str() != Some(platform.as_str())) {
                bail!("No DAT has been imported for {platform} (dat import adds one).");
            }
            let dat_id = s["summary"]["dat"]["id"].clone();
            let plan = if cmd == "duplicates" {
                invoke(&app, "organise_duplicates", json!({ "dat": dat_id })).await?
            } else if cmd == "convert" || cmd == "extract" {
                let to = match (cmd, to) {
                    ("extract", _) => "extract".to_string(),
                    (_, Some(t)) => t,
                    _ => {
                        let f = invoke(&app, "formats", json!({})).await?;
                        f.as_array().into_iter().flatten().find(|p| p["platform"] == platform.as_str()).and_then(|p| p["preferred"].as_str().map(String::from))
                            .with_context(|| format!("{platform} games aren't converted"))?
                    }
                };
                let games = (!games.is_empty()).then_some(games);
                invoke(&app, "convert_plan", json!({ "dat": dat_id, "to": to, "out": out, "games": games })).await?
            } else {
                let mode = match (mode, misnamed) {
                    (Some(m), _) => m,
                    (None, true) => "rename".into(),
                    (None, false) => bail!("organise needs --mode copy, rename or move"),
                };
                invoke(&app, "organise_plan", json!({ "dat": dat_id, "mode": mode, "out": out, "format": format, "partial": partial, "always_copy": always_copy, "only_misnamed": misnamed })).await?
            };
            let ops = invoke(&app, "organise_preview", json!({ "id": plan["id"], "limit": 1000 })).await?;
            if json_out && dry {
                return print(&json!({ "plan": plan, "ops": ops }));
            }
            if !json_out {
                println!("{}: {} games, {} already in place", plan["label"].as_str().unwrap_or(""), plan["games"], plan["in_place"]);
                for o in plan["by_op"].as_array().into_iter().flatten() {
                    println!("  {:<8} {:>6}  ({} MB)", o["op"].as_str().unwrap_or(""), o["count"], o["bytes"].as_u64().unwrap_or(0) / 1_000_000);
                }
                for (k, what) in [("conflicts", "in the way (left as they are)"), ("duplicates", "duplicates"), ("left", "games left out")] {
                    if plan[k].as_u64().unwrap_or(0) > 0 {
                        println!("  {} {what}", plan[k]);
                    }
                }
                for sp in plan["space"].as_array().into_iter().flatten().filter(|s| s["short"] == true) {
                    eprintln!("  not enough space on {}: {} MB needed, {} MB free", sp["drive"].as_str().unwrap_or(""), sp["needed"].as_u64().unwrap_or(0) / 1_000_000, sp["free"].as_u64().unwrap_or(0) / 1_000_000);
                }
                if dry {
                    for o in ops["items"].as_array().into_iter().flatten() {
                        println!("{:<8} {}  <-  {}", o["op"].as_str().unwrap_or(""), o["to"].as_str().unwrap_or(""), o["from"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(", "));
                    }
                    if ops["total"].as_u64().unwrap_or(0) > 1000 {
                        println!("… and {} more", ops["total"].as_u64().unwrap_or(0) - 1000);
                    }
                }
            }
            if dry || plan["ops"].as_u64().unwrap_or(0) == 0 {
                return Ok(());
            }
            let r = follow(&app, invoke(&app, "organise_run", json!({ "id": plan["id"] })).await?).await?;
            if json_out {
                return print(&r);
            }
            println!("{} of {} steps done{}; change {} (undo {} puts it back)", r["done"], r["ops"], if r["stopped"] == true { " (stopped)" } else { "" }, r["id"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
            for f in r["failed"].as_array().into_iter().flatten().take(20) {
                eprintln!("  {}", f["error"].as_str().or(f.as_str()).unwrap_or(""));
            }
            Ok(())
        }
        ("history", _) => {
            let h = invoke(&app, "change_history", json!({})).await?;
            if json_out {
                return print(&h);
            }
            for c in h.as_array().into_iter().flatten() {
                let held = c["held"].as_u64().unwrap_or(0);
                println!(
                    "{}  {:<11} {}{}{}",
                    c["id"].as_str().unwrap_or(""), c["state"].as_str().unwrap_or(""), c["label"].as_str().unwrap_or(""),
                    if held > 0 { format!("  ({} MB set aside)", held / 1_000_000) } else { String::new() },
                    if c["can_undo"] == true { "  [can undo]" } else { "" }
                );
            }
            Ok(())
        }
        ("undo", _) | ("finish", _) => {
            let id = a.rest.first().with_context(|| format!("{cmd} needs a change's id (history shows them)"))?;
            let r = follow(&app, invoke(&app, &format!("change_{cmd}"), json!({ "id": id })).await?).await?;
            print(&r)
        }
        ("set-aside", "empty") => {
            let r = invoke(&app, "change_empty", json!({ "id": a.rest.first() })).await?;
            if json_out {
                return print(&r);
            }
            println!("{} MB freed ({} changes)", r["freed"].as_u64().unwrap_or(0) / 1_000_000, r["changes"]);
            Ok(())
        }
        (c, s) => bail!("unknown command {c} {s} (gamerepo-cli --help lists them)"),
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
                            "pick_files" => {
                                // every queued answer, as the files chosen at once
                                let all: Vec<String> = picks.lock().unwrap().drain(..).collect();
                                respond(&mut stream, 200, "application/json", &serde_json::to_vec(&json!(all))?)
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
