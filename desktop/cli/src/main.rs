//! Command-line access to the desktop app's backend (same code, no window).
//!
//!   gamerepo-cli --version
//!   gamerepo-cli --help
//!   gamerepo-cli library-info --library <dir>
//!   gamerepo-cli index-info --library <dir> --data <dir>
//!   gamerepo-cli serve --ui <dir> --home <dir> [--library <dir>] [--port 8790]
//!   gamerepo-cli roots add|list|remove …, scan, dat import|list, audit, make-dat
//!   gamerepo-cli organise, duplicates, convert, extract, history, undo, finish, set-aside empty, tools
//!   gamerepo-cli systems, collections, collection, export, exports
//!   gamerepo-cli pc, identify, sources, launchbox, steam, add
//!   gamerepo-cli games, game, scrape, docs
//!   gamerepo-cli serve-devices [status|pair|devices|remove|changed|stop]
//!   gamerepo-cli device find|pair|unpair|systems|collections|games|game|add|remove|holdings
//!
//! `serve` runs the app's commands behind a local web server, so the page can be
//! tested in an ordinary browser (with tests/tauri_shim.js standing in for Tauri's
//! `invoke`). The library commands (roots, scan, dat, audit, make-dat) work on the
//! app's own library and index unless --library and --home say otherwise; they go
//! through the same commands as the window (gamerepo_core::api).

mod devices;

use anyhow::{bail, Context, Result};
use gamerepo_core::api::{App, AppPaths, Reply};
use gamerepo_core::config::app_dirs;
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

  systems [list]             every system, with those added by hand marked
  systems add --name <name> [--id <id>] [--dat-names <a,b>] [--folders <a,b>]
        [--exts <a,b>] [--media cartridge|cd|nintendo|umd|audit]
                             add a system (systems edit <id> … changes one)
  systems remove <id>        remove a system added by hand (not one with DATs)
  collections                the collections
  collection new --name <name> [--platform <id>]... [--all] [--no-1g1r]
        [--regions <USA,Europe,…>] [--only-regions] [--languages <En,Fr>]
        [--search <text>] [--from-dat <DAT name>]... [--size-limit <64GB|none>]
                             make a collection (collection edit <id> … changes one);
                             a size limit in GB is a card's (64GB: 64,000,000,000 bytes)
  collection show <id>       its games, by group and system, and its size on disk
  collection dats <id> [--out <dir>]
                             save it as DATs, one per system of each group
  collection delete <id>
  export <collection id> --out <dir> [--to esde|folder] [--esde <ES-DE folder>]
        [--layout <id> --place <place>=<dir>...]
        [--always-copy] [--full-names] [--dry-run] [--fields <a,b>|all]
        [--media <a,b>|all] [--manuals] [--docs] [--bios] [--image-size <pixels>]
        [--groups collections|subfolders|systems]
                             export a collection to ES-DE's ROMs folder (with its
                             gamelists and a custom collection when --esde is given),
                             to a folder, or in any layout (layouts lists them, with
                             the places each asks for: --place card=E:/ for a card),
                             hardlinked where it can be, checked; --fields, --media,
                             --manuals, --docs and --bios carry the games' details,
                             pictures, documents and their systems' BIOS files,
                             --image-size makes the pictures smaller; --groups puts a
                             collection's groups after the first in their systems'
                             folders (the default), in folders of their own there, or
                             (ES-DE) in systems of their own; each is a collection too
  exports                    the exports made
  export update <export id> [--dry-run] [--groups …]
                             export it again: new games added, those gone set aside
  export check <export id>   read the export's files back against the DATs
  export forget <export id>  forget an export (its files stay)
  layouts                    the export layouts: built in (ES-DE, the devices, a plain
                             folder) and your own, with the places each asks for
  layout show <id>           a layout as JSON
  layout check <file.json>   what's wrong with a layout, if anything
  layout import <file.json>  bring in a layout someone shared (or you edited)
  layout write <id> <file.json>
                             save a layout to a file, to change it or give it away
  layout remove <id>         remove one of your own layouts
  bios                       each system's BIOS files: have, missing, or a file of
                             its name that isn't it (needs a BIOS DAT: dat starter bios
                             brings in libretro's System.dat)
  bios report                save a list of the missing BIOS files
  bios gather [--dry-run]    copy each BIOS file found into the library's BIOS folder

  pc [list]                  the PC games
  pc show <id>               a PC game: its files, program and details
  pc import [<path>...] [--move] [--always-copy] [--no-search] [--dry-run]
                             import PC games (the inbox in Settings when no paths are
                             given): installers, archives, discs or folders, each into
                             its own folder in the PC games folder
  pc set <id> [--title <t>] [--year <y>] [--program <path>] [--args <a>]
                             change a PC game (a new title or year renames its folder)
  pc unpack <id> [--file <installer or archive>]
                             unpack a game into its extracted/ folder (GOG installers
                             need innoextract)
  pc folder [<dir>|--clear]  where PC games go (default: PC in the library)
  pc inbox [<dir>|--clear]   the inbox folder Import reads
  identify <title> [--platform <id>] [--year <y>]
                             search the sources for a game
  sources                    the sources Identify and Scrape ask, in order
  sources key <source> --<field> <value>... [--clear]
                             set a source's account or key (sources lists the fields)
  sources check <source>     try a source's account or key
  launchbox get              download LaunchBox Games DB's data (for Identify)
  steam users                the Steam users on this computer
  steam preview|write|remove --user <id>
                             the PC games as Steam shortcuts (non-Steam games) with
                             their artwork; Steam must be closed to write or remove
  add <file>... --platform <id> [--into <dir>] [--move] [--dry-run]
                             add a game for a system (not PC): copied (or moved) into
                             a folder with games, scanned, and checked against its DAT

  games <platform> [--show have|all|look|none] [--search <text>]
                             a system's games in its DAT, with what the library has
                             for each (details, pictures, documents); look: those a
                             scrape wasn't sure about
  game <id> [--clear] [--pick <source> <id>] [--edit <field> <value>] [--unmark <field>]
        [--use <field> <source>] [--show <kind> <source>] [--delete-metadata <a,b>]
                             a game's details, pictures and documents; --clear forgets
                             what was scraped, --pick takes a source's game for it,
                             --edit changes a detail by hand (kept over later scrapes
                             until --unmark), --use takes a detail from that source,
                             --show shows that source's picture of a kind,
                             --delete-metadata sets aside its details, kinds of
                             picture or manuals, with a preview and Undo
  games rename-folders [--dry-run]
                             rename the game folders a DAT's new version renamed the
                             games of, with a preview and Undo
  games delete-metadata <platform> --what <details,cover,…,manual> [--dry-run]
                             set aside a system's metadata, with a preview and Undo
  games sizes                what the library's metadata takes, by system and kind
  games details-from [<field> <source|order>]
                             which source each detail is taken from (else the
                             scrape's order); every game's details made again
  library [--search <text>] [--all] [--one] [--system <a,b>] [--genre <a,b>]
        [--decade <1990,2000>] [--developer <a>] [--publisher <a>] [--region <a>]
        [--language <a>] [--players <n>] [--rating <0-1>] [--has <pictures,manual,…>]
        [--look] [--collection <id>] [--sort title|system|year|rating|size|date]
        [--desc] [--group system|genre|decade|developer|publisher] [--facets]
                             every game in the library, across its systems (--all: in
                             the DATs too; --one: one per title), with the filters'
                             counts (--facets)
  scrape <platform> [--all] [--game <id>]... [--collection <id>] [--sources <a,b>]
        [--fields <a,b>] [--media <a,b>] [--regions <us,eu,…>] [--language <en>]
        [--existing fill|replace|skip] [--variants <cover,…>] [--again]
                             scrape a system's games in the library (--all: every
                             game in its DAT) for their details, pictures and manual;
                             --variants: the kinds kept from every source that has
                             one; --again replaces and asks the sources afresh; the
                             choices left out are the last scrape's
  docs [--search <text>] [--kind <kind>] [--platform <id>] [--yours]
                             the documents (guides, manuals, …), searched by their text
  docs read                  read the documents' text for searching
  docs import <file or folder>... [--platform <id>] [--game <id>] [--move]
        [--always-copy] [--dry-run]
                             bring documents into the library, each with the game it's
                             paired with (shown first)

Game Repo Server serves the library to paired devices on the home network; its data
(index, certificate, devices) is kept in --data <dir> (by default its own folder).

  serve-devices [--library <dir>] [--port 47210] [--index-from <file>] [--pair]
        [--no-announce] [--no-watch] [--parent [--keep]]
                             run the server until Ctrl+C or serve-devices stop (the library
                             it served last, else Game Repo's; its first index copied from
                             Game Repo's); --pair shows a pairing code at once; --parent
                             stops it when what started it ends (its input closes), unless
                             --keep or Game Repo's Keep serving says otherwise
  serve-devices status       how the running server is
  serve-devices pair         a code to pair a device with (for 10 minutes)
  serve-devices unpair       stop pairing
  serve-devices devices      the paired devices, with what each holds
  serve-devices remove <id>  remove a device: it has to pair again
  serve-devices changed [<path>...]
                             tell the server the library changed (those paths, or all)
  serve-devices stop         stop the server

The device commands stand in for a device's client: --to <dir> is a folder holding
ES-DE/ (with its settings) and ROMs/, as ES-DE has them.

  device find [--wait <seconds>]
                             the servers announcing themselves on the network
  device pair <address> <code> --to <dir> [--name <name>]
                             pair with a server (its address, port 47210 unless given)
  device unpair --to <dir>   forget the server, and tell it
  device systems|collections --to <dir>
                             the server's systems, or its collections with their groups
  device games --to <dir> [--system <id>] [--search <text>] [--collection <id>]
        [--group <id>] [--sort title|year|size] [--desc] [--offset <n>] [--limit <n>]
                             the games, a page at a time, with their keys
  device game <key> --to <dir>
                             a game's details and files
  device add <key>... --to <dir> [--media cover,screenshot,…] [--manuals]
        [--size <pixels>] [--collection <name>] [--bios]
                             fetch games into ES-DE's folders with their pictures (made
                             as small as --size, the screen's longest side), manuals and
                             gamelist entries, in a custom collection if named
  device remove <key>... --to <dir>
                             remove games with their files and entries (files another
                             game it holds needs stay)
  device holdings --to <dir> what the device holds from the server
  device app <command> [<args as JSON>] --to <dir> --data <dir> [--wait]
                             Game Repo Go's backend, as its pages call it (status,
                             pair, sync, systems, games, prepare, add, queue,
                             on_device, removal, remove, add_media, settings…), with
                             --data as its own folder; --wait waits for the queue
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
        "serve-devices" => devices::serve_devices(a).await,
        "device" => devices::device_command(a),
        "roots" | "scan" | "dat" | "audit" | "make-dat" | "platforms" | "organise" | "duplicates" | "history" | "undo" | "finish"
        | "set-aside" | "convert" | "extract" | "formats" | "tools" | "systems" | "collections" | "collection" | "export" | "exports" | "pc" | "identify"
        | "sources" | "launchbox" | "steam" | "add" | "games" | "game" | "scrape" | "docs" | "layouts" | "layout" | "bios" | "library" => library_command(&cmd, a).await,
        other => bail!("unknown command {other} (gamerepo-cli --help lists them)"),
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

/// A size as Windows shows it (1 GB = 1024 MB).
fn bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut x, mut i) = (n as f64, 0);
    while x >= 1024.0 && i < units.len() - 1 {
        x /= 1024.0;
        i += 1;
    }
    if i > 0 && x < 10.0 { format!("{x:.1} {}", units[i]) } else { format!("{} {}", x.round(), units[i]) }
}

/// "64GB" (as a card's label says: 64,000,000,000 bytes), "59.6GiB", "500MB" or bytes.
fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_uppercase().replace(' ', "");
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().with_context(|| format!("{s} isn't a size (try 64GB)"))?;
    let mult: f64 = match unit {
        "" | "B" => 1.0,
        "KB" => 1e3, "MB" => 1e6, "GB" => 1e9, "TB" => 1e12,
        "KIB" => 1024.0, "MIB" => 1024f64.powi(2), "GIB" => 1024f64.powi(3), "TIB" => 1024f64.powi(4),
        _ => bail!("{s} isn't a size (try 64GB)"),
    };
    Ok((n * mult).round() as u64)
}

fn print(v: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(v)?);
    Ok(())
}

/// roots, scan, dat, audit, make-dat and platforms: the window's commands on the app's library.
async fn library_command(cmd: &str, mut a: Args) -> Result<()> {
    let (config_dir, data_dir, server_data) = match a.flag("--home") {
        Some(h) => (PathBuf::from(&h).join("config"), PathBuf::from(&h).join("data"), PathBuf::from(&h).join("server")),
        None => {
            let (c, d) = app_dirs();
            (c, d, gamerepo_wire::control::default_data())
        }
    };
    let app = App::new(AppPaths { config_dir, data_dir, library_url: String::new(), server_data, server_program: server_program() })?;
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
    let subs = matches!(cmd, "roots" | "dat" | "set-aside" | "systems" | "collection" | "export" | "pc" | "launchbox" | "steam" | "layout" | "bios")
        || (matches!(cmd, "sources" | "docs") && a.rest.first().is_some_and(|x| matches!(x.as_str(), "key" | "check" | "read" | "import")))
        || (cmd == "games" && a.rest.first().is_some_and(|x| ["rename-folders", "delete-metadata", "sizes", "details-from"].contains(&x.as_str())));
    let sub = if subs && !a.rest.is_empty() { a.rest.remove(0) } else { String::new() };
    let list_of = |v: Option<String>, all: &[&str]| -> Option<Vec<String>> {
        v.map(|v| if v == "all" { all.iter().map(|s| s.to_string()).collect() } else { v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() })
    };
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
            show_and_run(&app, plan, dry, json_out).await
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
        ("systems", "") | ("systems", "list") => {
            let p = invoke(&app, "platforms", json!({})).await?;
            if json_out {
                return print(&json!(p.as_array().into_iter().flatten().filter(|x| x["custom"] == true).collect::<Vec<_>>()));
            }
            for x in p.as_array().into_iter().flatten() {
                println!("{:<14} {}{}", x["id"].as_str().unwrap_or(""), x["name"].as_str().unwrap_or(""), if x["custom"] == true { "  (added)" } else { "" });
            }
            Ok(())
        }
        ("systems", "add") | ("systems", "edit") => {
            let old = if sub == "edit" { Some(a.rest.first().cloned().context("systems edit needs the system's id")?) } else { None };
            let name = a.need("--name")?;
            let system = json!({
                "id": a.flag("--id").or(old.clone()), "name": name, "dat_names": a.flag("--dat-names").unwrap_or_default(),
                "folders": a.flag("--folders").unwrap_or_default(), "exts": a.flag("--exts").unwrap_or_default(),
                "media": a.flag("--media").unwrap_or_else(|| "cartridge".into()),
            });
            print(&invoke(&app, "systems_set", json!({ "system": system, "old": old })).await?)
        }
        ("systems", "remove") => {
            let id = a.rest.first().context("systems remove needs the system's id")?;
            invoke(&app, "systems_remove", json!({ "id": id })).await?;
            Ok(())
        }
        ("collections", _) => {
            let v = invoke(&app, "collections", json!({})).await?;
            if json_out {
                return print(&v);
            }
            for c in v.as_array().into_iter().flatten() {
                println!("{}  {}", c["id"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("collection", "new") | ("collection", "edit") => {
            let mut c = if sub == "edit" {
                let id = a.rest.first().cloned().context("collection edit needs the collection's id")?;
                invoke(&app, "collection_get", json!({ "id": id })).await?["collection"].clone()
            } else {
                json!({ "id": "", "name": "", "rules": {} })
            };
            if let Some(n) = a.flag("--name") {
                c["name"] = json!(n);
            }
            let mut platforms = vec![];
            while let Some(p) = a.flag("--platform") {
                platforms.push(p);
            }
            let mut from = vec![];
            while let Some(d) = a.flag("--from-dat") {
                from.push(d);
            }
            if let Some(x) = a.flag("--size-limit") {
                c["size_limit"] = if x == "none" { Value::Null } else { json!(parse_size(&x)?) };
            }
            let r = &mut c["rules"];
            if !platforms.is_empty() {
                r["platforms"] = json!(platforms);
            }
            if !from.is_empty() {
                r["from_dats"] = json!(from);
            }
            if let Some(x) = a.flag("--regions") {
                r["regions"] = json!(x.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>());
            }
            if let Some(x) = a.flag("--languages") {
                r["languages"] = json!(x.split(',').map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>());
            }
            if let Some(x) = a.flag("--search") {
                r["search"] = json!(x);
            }
            let has = |f: &str| a.rest.iter().any(|x| x == f);
            if has("--all") {
                r["include"] = json!("all");
            }
            if has("--have") {
                r["include"] = json!("have");
            }
            if has("--no-1g1r") {
                r["one_per_title"] = json!(false);
            }
            if has("--only-regions") {
                r["only_regions"] = json!(true);
            }
            let saved = invoke(&app, "collection_save", json!({ "collection": c })).await?;
            if json_out {
                return print(&saved);
            }
            let sum = &saved["summary"];
            println!("{}  {}: {} games ({} you have), {} on disk", saved["collection"]["id"].as_str().unwrap_or(""), saved["collection"]["name"].as_str().unwrap_or(""), sum["games"], sum["have"], bytes(sum["size"].as_u64().unwrap_or(0)));
            Ok(())
        }
        ("collection", "show") => {
            let id = a.rest.first().cloned().context("collection show needs the collection's id")?;
            let c = invoke(&app, "collection_get", json!({ "id": id })).await?;
            let games = invoke(&app, "collection_games", json!({ "id": id, "limit": 1000 })).await?;
            if json_out {
                return print(&json!({ "collection": c["collection"], "summary": c["summary"], "games": games }));
            }
            let sum = &c["summary"];
            let size = sum["size"].as_u64().unwrap_or(0);
            println!("{} games ({} you have) on {} systems: {} on disk{}", sum["games"], sum["have"], sum["systems"].as_array().map_or(0, Vec::len), bytes(size),
                match sum["missing_size"].as_u64() { Some(m) if m > 0 => format!(", about {} more for the games you don't have (unpacked)", bytes(m)), _ => String::new() });
            if let Some(limit) = c["collection"]["size_limit"].as_u64() {
                println!("Size limit {}: {}", bytes(limit), if size <= limit { format!("fits, {} to spare", bytes(limit - size)) } else { format!("over by {}", bytes(size - limit)) });
            }
            let groups = sum["groups"].as_array().cloned().unwrap_or_default();
            for (i, gr) in groups.iter().enumerate() {
                // a collection of groups (D75): each one's systems under it
                let indent = if groups.len() > 1 {
                    println!("{}: {} games ({} you have), {}", gr["name"].as_str().unwrap_or(""), gr["games"], gr["have"], bytes(gr["size"].as_u64().unwrap_or(0)));
                    "  "
                } else {
                    ""
                };
                for s in gr["systems"].as_array().into_iter().flatten() {
                    println!("{indent}{}: {} games ({} you have), {}", s["name"].as_str().unwrap_or(""), s["games"], s["have"], bytes(s["size"].as_u64().unwrap_or(0)));
                }
                for g in games["items"].as_array().into_iter().flatten().filter(|g| g["group"] == gr["id"] || (i == 0 && g["group"].is_null())) {
                    println!("{indent}  {:<6} {}", g["status"].as_str().unwrap_or(""), g["key"].as_str().unwrap_or(""));
                }
            }
            Ok(())
        }
        ("collection", "dats") => {
            let id = a.rest.first().cloned().context("collection dats needs the collection's id")?;
            let to = a.flag("--out").map(|o| std::path::absolute(&o).map(|p| p.display().to_string()).unwrap_or(o));
            print(&invoke(&app, "collection_dats", json!({ "id": id, "to": to })).await?)
        }
        ("collection", "delete") => {
            let id = a.rest.first().context("collection delete needs the collection's id")?;
            invoke(&app, "collection_delete", json!({ "id": id })).await?;
            Ok(())
        }
        ("exports", _) => print(&invoke(&app, "exports", json!({})).await?),
        ("export", "check") => {
            let id = a.rest.first().context("export check needs the export's id (exports lists them)")?;
            let r = follow(&app, invoke(&app, "export_check", json!({ "id": id })).await?).await?;
            if json_out {
                return print(&r);
            }
            println!("{} files: {} as they should be, {} missing, {} changed", r["files"], r["ok"], r["missing"].as_array().map(Vec::len).unwrap_or(0), r["changed"].as_array().map(Vec::len).unwrap_or(0));
            for f in r["missing"].as_array().into_iter().flatten() {
                println!("  missing  {}", f["path"].as_str().unwrap_or(""));
            }
            for f in r["changed"].as_array().into_iter().flatten() {
                println!("  changed  {}  ({})", f["path"].as_str().unwrap_or(""), f["why"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("export", "forget") => {
            let id = a.rest.first().context("export forget needs the export's id")?;
            invoke(&app, "export_forget", json!({ "id": id })).await?;
            Ok(())
        }
        ("export", what) => {
            let dry = a.rest.iter().any(|x| x == "--dry-run");
            let plan = if what == "update" {
                let groups = a.flag("--groups");
                let id = a.rest.iter().find(|x| !x.starts_with("--")).cloned().context("export update needs the export's id (exports lists them)")?;
                invoke(&app, "export_plan", json!({ "export": id, "groups": groups })).await?
            } else if what.is_empty() {
                bail!("export needs a collection's id (collections lists them)")
            } else {
                let abs = |o: String| std::path::absolute(&o).map(|p| p.display().to_string()).unwrap_or(o);
                let layout = a.flag("--layout");
                let mut places = serde_json::Map::new();
                while let Some(p) = a.flag("--place") {
                    let (k, v) = p.split_once('=').context("--place is <place>=<dir>, such as card=E:\\")?;
                    places.insert(k.trim().to_string(), json!(abs(v.trim().to_string())));
                }
                let target = a.flag("--to").unwrap_or_else(|| "esde".into());
                let out = if layout.is_some() { a.flag("--out").map(abs).unwrap_or_default() } else { a.need("--out").map(abs)? };
                let esde = a.flag("--esde").map(abs);
                let always_copy = a.rest.iter().any(|x| x == "--always-copy");
                let short_names = !a.rest.iter().any(|x| x == "--full-names");
                let fields: Vec<&str> = gamerepo_core::meta::FIELDS.iter().map(|(k, _)| *k).collect();
                let media: Vec<&str> = gamerepo_core::meta::MEDIA.iter().map(|(k, _, _)| *k).collect();
                let carry = json!({
                    "fields": list_of(a.flag("--fields"), &fields).unwrap_or_default(), "media": list_of(a.flag("--media"), &media).unwrap_or_default(),
                    "manuals": a.rest.iter().any(|x| x == "--manuals"), "docs": a.rest.iter().any(|x| x == "--docs"),
                    "image_size": a.flag("--image-size").map(|n| n.parse::<u32>()).transpose().context("--image-size is a number of pixels")?.unwrap_or(0),
                    "bios": a.rest.iter().any(|x| x == "--bios"),
                });
                let groups = a.flag("--groups").unwrap_or_default();
                invoke(&app, "export_plan", json!({ "collection": what, "layout": layout, "places": places, "target": target, "out": out, "esde": esde, "always_copy": always_copy, "short_names": short_names, "carry": carry, "groups": groups })).await?
            };
            if !json_out {
                for n in plan["notes"].as_array().into_iter().flatten() {
                    println!("{}", n.as_str().unwrap_or(""));
                }
            }
            show_and_run(&app, plan, dry, json_out).await
        }
        ("layouts", _) => {
            let v = invoke(&app, "layouts", json!({})).await?;
            if json_out {
                return print(&v);
            }
            for l in v.as_array().into_iter().flatten() {
                let places: Vec<String> = l["places"].as_array().into_iter().flatten().map(|p| format!("--place {}=<{}>", p["id"].as_str().unwrap_or(""), p["label"].as_str().unwrap_or(""))).collect();
                println!("{:<14} {}{}", l["id"].as_str().unwrap_or(""), l["name"].as_str().unwrap_or(""), if l["builtin"] == true { "  (built in)" } else { "" });
                println!("{:<14} {}", "", places.join(" "));
            }
            Ok(())
        }
        ("layout", "show") => {
            let id = a.rest.first().cloned().context("layout show needs a layout's id (layouts lists them)")?;
            print(&invoke(&app, "layout_get", json!({ "id": id })).await?["layout"])
        }
        ("layout", "check") => {
            let f = a.rest.first().cloned().context("layout check needs a layout's file")?;
            let v: Value = serde_json::from_str(&std::fs::read_to_string(&f).with_context(|| format!("couldn't read {f}"))?).with_context(|| format!("{f} isn't JSON"))?;
            let r = invoke(&app, "layout_check", json!({ "layout": v })).await?;
            if json_out {
                return print(&r);
            }
            let problems = r["problems"].as_array().cloned().unwrap_or_default();
            if problems.is_empty() {
                println!("{f}: no problems");
            }
            for p in &problems {
                println!("{}: {}", p["at"].as_str().unwrap_or(""), p["message"].as_str().unwrap_or(""));
            }
            if !problems.is_empty() {
                std::process::exit(1);
            }
            Ok(())
        }
        ("layout", "import") => {
            let f = a.rest.first().cloned().context("layout import needs a layout's file")?;
            let f = std::path::absolute(&f).map(|p| p.display().to_string()).unwrap_or(f);
            let r = invoke(&app, "layout_import", json!({ "path": f })).await?;
            println!("Brought in {} as {}", r["name"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
            Ok(())
        }
        ("layout", "write") => {
            let id = a.rest.first().cloned().context("layout write needs a layout's id and a file")?;
            let to = a.rest.get(1).cloned().context("layout write needs a file to write")?;
            let to = std::path::absolute(&to).map(|p| p.display().to_string()).unwrap_or(to);
            print(&invoke(&app, "layout_write", json!({ "id": id, "to": to })).await?)
        }
        ("layout", "remove") => {
            let id = a.rest.first().context("layout remove needs a layout's id")?;
            invoke(&app, "layout_remove", json!({ "id": id })).await?;
            Ok(())
        }
        ("bios", "") => {
            let v = invoke(&app, "bios", json!({})).await?;
            if json_out {
                return print(&v);
            }
            if v["dats"].as_array().is_none_or(|d| d.is_empty()) {
                println!("No BIOS DAT yet: dat starter bios brings in libretro's System.dat.");
                return Ok(());
            }
            for s in v["systems"].as_array().into_iter().flatten() {
                println!("{}{}: {} have, {} missing{}", s["name"].as_str().unwrap_or(""), if s["yours"] == true { "" } else { " (no games of it)" }, s["have"], s["missing"],
                         if s["wrong"].as_i64().unwrap_or(0) > 0 { format!(", {} not what their names say", s["wrong"]) } else { String::new() });
                for f in s["files"].as_array().into_iter().flatten().filter(|f| f["status"] != "have") {
                    println!("  {:<8} {}{}", f["status"].as_str().unwrap_or(""), f["name"].as_str().unwrap_or(""), f["at"].as_str().map(|a| format!("  ({a})")).unwrap_or_default());
                }
            }
            Ok(())
        }
        ("bios", "report") => {
            let r = invoke(&app, "bios_report", json!({})).await?;
            println!("{}", r["path"].as_str().unwrap_or(""));
            Ok(())
        }
        ("bios", "gather") => {
            let dry = a.rest.iter().any(|x| x == "--dry-run");
            let plan = invoke(&app, "bios_gather_plan", json!({})).await?;
            show_and_run(&app, plan, dry, json_out).await
        }
        ("pc", "") | ("pc", "list") => {
            let v = invoke(&app, "pc_games", json!({})).await?;
            if json_out {
                return print(&v);
            }
            println!("PC games in {}", v["folder"].as_str().unwrap_or(""));
            for g in v["games"].as_array().into_iter().flatten() {
                let year = g["year"].as_i64().map(|y| format!(" ({y})")).unwrap_or_default();
                let ids: Vec<String> = g["ids"].as_object().into_iter().flatten().map(|(k, v)| format!("{k} {}", v.as_str().unwrap_or(""))).collect();
                println!("  {}  {}{year}  {}  {}", g["id"].as_str().unwrap_or(""), g["title"].as_str().unwrap_or(""), bytes(g["size"].as_u64().unwrap_or(0)), ids.join(", "));
            }
            Ok(())
        }
        ("pc", "show") => {
            let id = a.rest.first().cloned().context("pc show needs a game's id (pc list lists them)")?;
            print(&invoke(&app, "pc_get", json!({ "id": id })).await?)
        }
        ("pc", "set") => {
            let id = a.rest.first().cloned().context("pc set needs a game's id")?;
            let mut patch = json!({});
            if let Some(t) = a.flag("--title") {
                patch["title"] = json!(t);
            }
            if let Some(y) = a.flag("--year") {
                patch["year"] = json!(y.parse::<i32>().context("--year is a year")?);
            }
            if let Some(p) = a.flag("--program") {
                patch["launch"] = json!({ "program": p, "args": a.flag("--args").unwrap_or_default() });
            }
            print(&invoke(&app, "pc_update", json!({ "id": id, "patch": patch })).await?)
        }
        ("pc", "unpack") => {
            let id = a.rest.first().cloned().context("pc unpack needs a game's id")?;
            let file = a.flag("--file");
            print(&follow(&app, invoke(&app, "pc_unpack", json!({ "id": id, "file": file })).await?).await?)
        }
        ("pc", "folder") | ("pc", "inbox") => {
            let clear = a.rest.iter().any(|x| x == "--clear");
            let path = a.rest.iter().find(|x| !x.starts_with("--")).map(|p| std::path::absolute(p).map(|p| p.display().to_string()).unwrap_or(p.clone()));
            let cmd = if sub == "folder" { "pc_folder_set" } else { "pc_inbox_set" };
            if path.is_none() && !clear {
                let v = invoke(&app, "pc_games", json!({})).await?;
                println!("{}", v[sub.as_str()].as_str().unwrap_or("(none)"));
                return Ok(());
            }
            print(&invoke(&app, cmd, json!({ "path": path })).await?)
        }
        ("pc", "import") => {
            let has = |f: &str| a.rest.iter().any(|x| x == f);
            let (mv, always_copy, no_search, dry) = (has("--move"), has("--always-copy"), has("--no-search"), has("--dry-run"));
            let paths: Vec<String> = a.rest.iter().filter(|x| !x.starts_with("--")).map(|p| std::path::absolute(p).map(|p| p.display().to_string()).unwrap_or(p.clone())).collect();
            let r = follow(&app, invoke(&app, "pc_read", json!({ "paths": paths, "search": !no_search })).await?).await?;
            let items = r["items"].as_array().cloned().unwrap_or_default();
            if !json_out {
                for it in &items {
                    let year = it["year"].as_i64().map(|y| format!(" ({y})")).unwrap_or_default();
                    let what = match it["status"].as_str().unwrap_or("") {
                        "have" => format!("in the library already ({})", it["game_title"].as_str().unwrap_or("")),
                        "update" => format!("goes into {}", it["game_title"].as_str().unwrap_or("")),
                        "look" => "needs a look (imported under the name)".into(),
                        _ => format!("found by {}", it["found"].as_str().unwrap_or("")),
                    };
                    println!("  {} → {}{year}: {what}", it["name"].as_str().unwrap_or(""), it["title"].as_str().unwrap_or(""));
                }
            }
            if items.iter().all(|i| i["include"] != true) {
                println!("Nothing to import.");
                return Ok(());
            }
            let plan = invoke(&app, "pc_import_plan", json!({ "items": items, "mode": if mv { "move" } else { "copy" }, "always_copy": always_copy })).await?;
            show_and_run(&app, plan, dry, json_out).await
        }
        ("identify", _) => {
            let platform = a.flag("--platform").unwrap_or_else(|| "pc".into());
            let year = a.flag("--year").map(|y| y.parse::<i64>()).transpose()?;
            let query = a.rest.iter().filter(|x| !x.starts_with("--")).cloned().collect::<Vec<_>>().join(" ");
            if query.is_empty() {
                bail!("identify needs a title to search for");
            }
            let v = invoke(&app, "identify_search", json!({ "query": query, "platform": platform, "year": year })).await?;
            if json_out {
                return print(&v);
            }
            for r in v["results"].as_array().into_iter().flatten() {
                let year = r["year"].as_i64().map(|y| format!(" ({y})")).unwrap_or_default();
                println!("  {:<10} {:<12} {}{year}  {}", r["source_name"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""), r["title"].as_str().unwrap_or(""), r["platform"].as_str().unwrap_or(""));
            }
            for e in v["errors"].as_array().into_iter().flatten() {
                eprintln!("  {}: {}", e["source"].as_str().unwrap_or(""), e["error"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("sources", "key") => {
            let source = a.rest.first().filter(|x| !x.starts_with("--")).cloned().context("sources key needs a source (sources lists them)")?;
            a.rest.remove(0);
            let clear = a.rest.iter().any(|x| x == "--clear");
            a.rest.retain(|x| x != "--clear");
            let mut keys = json!({});
            while let Some(f) = a.rest.first().cloned() {
                let field = f.strip_prefix("--").with_context(|| format!("{f}: give each field as --<field> <value>"))?.to_string();
                let v = a.flag(&f).with_context(|| format!("{f} needs a value"))?;
                keys[field] = json!(v);
            }
            let v = invoke(&app, "sources_keys_set", json!({ "source": source, "keys": keys, "clear": clear })).await?;
            if json_out {
                return print(&v);
            }
            Ok(())
        }
        ("sources", "check") => {
            let source = a.rest.first().cloned().context("sources check needs a source")?;
            let v = invoke(&app, "sources_check", json!({ "source": source })).await?;
            if json_out {
                return print(&v);
            }
            println!("{}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string()));
            Ok(())
        }
        ("sources", _) => print(&invoke(&app, "sources", json!({})).await?),
        ("games", "rename-folders") => {
            let dry = a.rest.iter().any(|x| x == "--dry-run");
            let plan = invoke(&app, "organise_rename_folders", json!({})).await?;
            show_and_run(&app, plan, dry, json_out).await
        }
        ("games", "delete-metadata") => {
            let dry = a.rest.iter().any(|x| x == "--dry-run");
            let what = list_of(a.flag("--what"), &[]).context("--what <details,cover,…,manual>")?;
            let platform = a.rest.iter().find(|x| !x.starts_with("--")).cloned().context("games delete-metadata needs a system")?;
            let plan = invoke(&app, "meta_delete", json!({ "platform": platform, "what": what })).await?;
            show_and_run(&app, plan, dry, json_out).await
        }
        ("games", "sizes") => {
            let v = invoke(&app, "meta_sizes", json!({})).await?;
            if json_out {
                return print(&v);
            }
            let mb = |n: &Value| format!("{:.1} MB", n.as_f64().unwrap_or(0.0) / 1e6);
            println!("{} in all", mb(&v["total"]));
            for sys in v["systems"].as_array().into_iter().flatten() {
                let kinds: Vec<String> = sys["kinds"].as_object().into_iter().flatten().map(|(k, n)| format!("{k} {}", mb(n))).collect();
                println!("  {} ({} games): {}  [{}]", sys["name"].as_str().unwrap_or(""), sys["games"], mb(&sys["total"]), kinds.join(", "));
            }
            Ok(())
        }
        ("games", "details-from") => {
            let rest: Vec<&String> = a.rest.iter().filter(|x| !x.starts_with("--")).collect();
            if let (Some(field), Some(source)) = (rest.first(), rest.get(1)) {
                let source = if *source == "order" { Value::Null } else { json!(source) };
                follow(&app, invoke(&app, "meta_set_details_from", json!({ "field": field, "source": source })).await?).await?;
            }
            print(&invoke(&app, "meta_details_from", json!({})).await?)
        }
        ("library", _) => {
            let rebuilt = invoke(&app, "browse", json!({})).await?;
            let mut q = json!({ "limit": 1000, "facets": a.rest.iter().any(|x| x == "--facets") });
            q["search"] = json!(a.flag("--search").unwrap_or_default());
            for (flag, key) in [("--sort", "sort"), ("--group", "group"), ("--collection", "collection")] {
                if let Some(v) = a.flag(flag) {
                    q[key] = json!(v);
                }
            }
            for (flag, key) in [("--all", "all"), ("--one", "one_per_title"), ("--desc", "desc"), ("--look", "look")] {
                q[key] = json!(a.rest.iter().any(|x| x == flag));
            }
            for (flag, key) in [("--system", "platform"), ("--genre", "genre"), ("--developer", "developer"), ("--publisher", "publisher"), ("--region", "region"), ("--language", "language"), ("--has", "has")] {
                if let Some(v) = list_of(a.flag(flag), &[]) {
                    q[key] = json!(v);
                }
            }
            if let Some(v) = list_of(a.flag("--decade"), &[]) {
                q["decade"] = json!(v.iter().filter_map(|d| d.trim_end_matches('s').parse::<i64>().ok()).collect::<Vec<_>>());
            }
            for (flag, key) in [("--players", "players"), ("--rating", "rating")] {
                if let Some(v) = a.flag(flag) {
                    q[key] = json!(v.parse::<f64>().with_context(|| format!("{flag} takes a number"))?);
                }
            }
            if let Some(n) = q["players"].as_f64() {
                q["players"] = json!(n as i64);
            }
            let v = invoke(&app, "browse_query", json!({ "query": q })).await?;
            if json_out {
                return print(&v);
            }
            println!("{} games ({} in the library across {} systems, {} in the DATs)", v["total"], rebuilt["have"], rebuilt["systems"], rebuilt["all"]);
            let mut group = Value::Null;
            for g in v["items"].as_array().into_iter().flatten() {
                if g["group"] != group && q["group"].is_string() {
                    group = g["group"].clone();
                    println!("{}", g["group_name"].as_str().map(String::from).unwrap_or_else(|| if group.is_null() { "(none)".into() } else { group.to_string().trim_matches('"').to_string() }));
                }
                let year = g["year"].as_i64().map(|y| format!(" ({y})")).unwrap_or_default();
                let more: Vec<String> = [g["developer"].as_str().map(String::from), g["genres"].as_array().filter(|a| !a.is_empty()).map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))]
                    .into_iter()
                    .flatten()
                    .collect();
                println!("  {}{year}  [{}]{}{}  {}", g["title"].as_str().unwrap_or(""), g["platform"].as_str().unwrap_or(""), if more.is_empty() { String::new() } else { format!("  {}", more.join(" · ")) }, if g["have"] == true { "" } else { "  (not in the library)" }, g["key"].as_str().unwrap_or(""));
            }
            if let Some(f) = v.get("facets") {
                println!("\nFilters:");
                for (k, list) in f.as_object().into_iter().flatten() {
                    match list {
                        Value::Array(a) if !a.is_empty() => println!("  {k}: {}", a.iter().take(12).map(|x| format!("{} {}", x["name"].as_str().map(String::from).unwrap_or_else(|| x["value"].to_string().trim_matches('"').to_string()), x["count"])).collect::<Vec<_>>().join(", ")),
                        Value::Object(o) => println!("  {k}: {}", o.iter().map(|(n, c)| format!("{n} {c}")).collect::<Vec<_>>().join(", ")),
                        Value::Number(n) => println!("  {k}: {n}"),
                        _ => {}
                    }
                }
            }
            Ok(())
        }
        ("games", _) => {
            let platform = a.rest.iter().find(|x| !x.starts_with("--")).cloned().context("games needs a system (platforms lists them)")?;
            let show = a.flag("--show").unwrap_or_else(|| "have".into());
            let search = a.flag("--search").unwrap_or_default();
            let v = invoke(&app, "meta_list", json!({ "platform": platform, "show": show, "search": search, "limit": 1000 })).await?;
            if json_out {
                return print(&v);
            }
            let c = &v["counts"];
            println!("{} games: {} in the library, {} with details, {} with pictures, {} need a look", c["all"], c["have"], c["details"], c["media"], c["look"]);
            for g in v["items"].as_array().into_iter().flatten() {
                let mut has = vec![];
                if g["details"] == true {
                    has.push("details".to_string());
                }
                let media: Vec<&str> = g["media"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                if !media.is_empty() {
                    has.push(media.join(" "));
                }
                if g["docs"].as_i64().unwrap_or(0) > 0 {
                    has.push(format!("{} documents", g["docs"]));
                }
                if g["review"] == true {
                    has.push("needs a look".into());
                }
                println!("  {:>7}  {}  [{}]", g["id"], g["name"].as_str().unwrap_or(""), has.join(", "));
            }
            Ok(())
        }
        ("game", _) => {
            let id: i64 = a.rest.iter().find(|x| !x.starts_with("--")).context("game needs a game's id (games lists them)")?.parse().context("a game's id is a number")?;
            if let Some(i) = a.rest.iter().position(|x| x == "--pick") {
                let (source, sid) = (a.rest.get(i + 1).cloned().context("--pick <source> <id>")?, a.rest.get(i + 2).cloned().context("--pick <source> <id>")?);
                return print(&invoke(&app, "meta_pick", json!({ "game": id, "source": source, "id": sid })).await?);
            }
            if a.rest.iter().any(|x| x == "--clear") {
                return print(&invoke(&app, "meta_clear", json!({ "id": id })).await?);
            }
            if let Some(i) = a.rest.iter().position(|x| x == "--edit") {
                let (field, value) = (a.rest.get(i + 1).cloned().context("--edit <field> <value>")?, a.rest.get(i + 2).cloned().context("--edit <field> <value>")?);
                return print(&invoke(&app, "meta_edit", json!({ "id": id, "field": field, "value": value })).await?);
            }
            if let Some(field) = a.flag("--unmark") {
                return print(&invoke(&app, "meta_edit", json!({ "id": id, "field": field })).await?);
            }
            // the rest work on the game's folder
            for (flag, cmd, k) in [("--use", "meta_pin", "field"), ("--show", "meta_choose", "kind")] {
                if let Some(i) = a.rest.iter().position(|x| x == flag) {
                    let (x, source) = (a.rest.get(i + 1).cloned().context("<what> <source>")?, a.rest.get(i + 2).cloned().context("<what> <source>")?);
                    let g = invoke(&app, "meta_game", json!({ "id": id })).await?;
                    let folder = g["folder"].as_str().context("the game has nothing scraped")?.to_string();
                    let mut args = json!({ "folder": folder });
                    args[k] = json!(x);
                    args[if cmd == "meta_pin" { "source" } else { "name" }] = json!(source);
                    invoke(&app, cmd, args).await?;
                    return print(&invoke(&app, "meta_game", json!({ "id": id })).await?);
                }
            }
            if let Some(what) = list_of(a.flag("--delete-metadata"), &[]) {
                let g = invoke(&app, "meta_game", json!({ "id": id })).await?;
                let folder = g["folder"].as_str().context("the game has nothing scraped")?.to_string();
                let plan = invoke(&app, "meta_delete", json!({ "folder": folder, "what": what })).await?;
                return show_and_run(&app, plan, a.rest.iter().any(|x| x == "--dry-run"), json_out).await;
            }
            print(&invoke(&app, "meta_game", json!({ "id": id })).await?)
        }
        ("scrape", _) => {
            let platform = a.rest.iter().find(|x| !x.starts_with("--")).cloned().context("scrape needs a system (platforms lists them)")?;
            let choices = invoke(&app, "scrape_choices", json!({})).await?;
            let mut o = choices["options"].clone();
            o["platform"] = json!(platform);
            o["scope"] = json!(if a.rest.iter().any(|x| x == "--all") { "all" } else { "have" });
            let mut games = vec![];
            while let Some(g) = a.flag("--game") {
                games.push(g.parse::<i64>().context("--game takes a game's id (games lists them)")?);
            }
            if !games.is_empty() {
                o["scope"] = json!("games");
                o["games"] = json!(games);
            }
            if let Some(c) = a.flag("--collection") {
                o["scope"] = json!("collection");
                o["collection"] = json!(c);
            }
            let ids = |k: &str| -> Vec<String> { choices[k].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str().map(String::from)).collect() };
            let (sources, fields, media) = (ids("sources"), ids("fields"), ids("media"));
            if let Some(v) = list_of(a.flag("--variants"), &refs(&media)) {
                o["variants"] = json!(v);
            }
            if a.rest.iter().any(|x| x == "--again") {
                o["existing"] = json!("replace");
                o["fresh"] = json!(true);
            }
            fn refs(v: &[String]) -> Vec<&str> {
                v.iter().map(String::as_str).collect()
            }
            for (flag, key, all) in [("--sources", "sources", &sources), ("--fields", "fields", &fields), ("--media", "media", &media)] {
                if let Some(v) = list_of(a.flag(flag), &refs(all)) {
                    o[key] = json!(v);
                }
            }
            if let Some(v) = list_of(a.flag("--regions"), &[]) {
                o["regions"] = json!(v);
            }
            for (flag, key) in [("--language", "language"), ("--existing", "existing")] {
                if let Some(v) = a.flag(flag) {
                    o[key] = json!(v);
                }
            }
            let r = follow(&app, invoke(&app, "scrape_run", json!({ "options": o })).await?).await?;
            if json_out {
                return print(&r);
            }
            println!("{} games: {} found, {} need a look, {} not found, {} skipped, {} pictures", r["games"], r["found"], r["look"], r["none"], r["skipped"], r["pictures"]);
            for e in r["errors"].as_array().into_iter().flatten() {
                eprintln!("  {}: {}", e["name"].as_str().unwrap_or(""), e["error"].as_str().unwrap_or(""));
            }
            Ok(())
        }
        ("docs", "read") => print(&follow(&app, invoke(&app, "docs_read", json!({})).await?).await?),
        ("docs", "import") => {
            let abs = |o: &String| std::path::absolute(o).map(|p| p.display().to_string()).unwrap_or(o.clone());
            let platform = a.flag("--platform");
            let game = a.flag("--game").map(|g| g.parse::<i64>()).transpose().context("--game takes a game's id")?;
            let has = |f: &str| a.rest.iter().any(|x| x == f);
            let (mv, always_copy, dry) = (has("--move"), has("--always-copy"), has("--dry-run"));
            let paths: Vec<String> = a.rest.iter().filter(|x| !x.starts_with("--")).map(abs).collect();
            let r = follow(&app, invoke(&app, "docs_items", json!({ "paths": paths, "platform": platform, "game": game })).await?).await?;
            let items = r["items"].as_array().cloned().unwrap_or_default();
            if !json_out {
                for it in &items {
                    let with = match it["game_name"].as_str() {
                        Some(g) => format!("{g} ({})", it["pairing"].as_str().unwrap_or("")),
                        None => "no game".into(),
                    };
                    println!("  {} → {} [{}]: {with}", it["name"].as_str().unwrap_or(""), it["title"].as_str().unwrap_or(""), it["kind"].as_str().unwrap_or(""));
                }
            }
            if items.iter().all(|i| i["include"] != true) {
                println!("No documents to bring in.");
                return Ok(());
            }
            let plan = invoke(&app, "docs_plan", json!({ "items": items, "mode": if mv { "move" } else { "copy" }, "always_copy": always_copy })).await?;
            show_and_run(&app, plan, dry, json_out).await
        }
        ("docs", _) => {
            let q = a.flag("--search").unwrap_or_default();
            let kind = a.flag("--kind").unwrap_or_default();
            let platform = a.flag("--platform").unwrap_or_default();
            let source = if a.rest.iter().any(|x| x == "--yours") { "yours" } else { "" };
            let v = invoke(&app, "docs_list", json!({ "query": q, "kind": kind, "platform": platform, "source": source, "limit": 1000 })).await?;
            if json_out {
                return print(&v);
            }
            println!("{} documents", v["total"]);
            for d in v["items"].as_array().into_iter().flatten() {
                let game = d["game"].as_str().map(|g| format!("  ({g})")).unwrap_or_default();
                println!("  {:>6}  {:<9} {}{game}", d["id"], d["kind"].as_str().unwrap_or(""), d["title"].as_str().unwrap_or(""));
                if let Some(sn) = d["snippet"].as_str().filter(|s| !s.is_empty()) {
                    println!("          {}", sn.replace('\n', " "));
                }
            }
            Ok(())
        }
        ("launchbox", "get") => print(&follow(&app, invoke(&app, "launchbox_fetch", json!({})).await?).await?),
        ("steam", "users") | ("steam", "") => print(&invoke(&app, "steam_info", json!({})).await?),
        ("steam", what @ ("preview" | "write" | "remove")) => {
            let user = a.need("--user")?;
            print(&invoke(&app, &format!("steam_{what}"), json!({ "user": user })).await?)
        }
        ("add", _) => {
            let platform = a.need("--platform")?;
            let abs = |o: &str| std::path::absolute(o).map(|p| p.display().to_string()).unwrap_or(o.to_string());
            let into = match a.flag("--into") {
                Some(d) => abs(&d),
                None => {
                    let places = invoke(&app, "add_places", json!({ "platform": platform })).await?;
                    places[0]["dir"].as_str().map(String::from).context("there's no folder with games for that system: add one (roots add) or give --into")?
                }
            };
            let has = |f: &str| a.rest.iter().any(|x| x == f);
            let (mv, dry) = (has("--move"), has("--dry-run"));
            let files: Vec<String> = a.rest.iter().filter(|x| !x.starts_with("--")).map(|p| abs(p)).collect();
            if files.is_empty() {
                bail!("add needs a file");
            }
            let plan = invoke(&app, "add_plan", json!({ "files": files, "platform": platform, "into": into, "mode": if mv { "move" } else { "copy" } })).await?;
            show_and_run(&app, plan, dry, json_out).await
        }
        (c, s) => bail!("unknown command {c} {s} (gamerepo-cli --help lists them)"),
    }
}

/// Show a planned change (all of it with --dry-run), then run it.
async fn show_and_run(app: &Arc<App>, plan: Value, dry: bool, json_out: bool) -> Result<()> {
    let ops = invoke(app, "organise_preview", json!({ "id": plan["id"], "limit": 1000 })).await?;
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
    let r = follow(app, invoke(app, "organise_run", json!({ "id": plan["id"] })).await?).await?;
    if json_out {
        return print(&r);
    }
    println!("{} of {} steps done{}; change {} (undo {} puts it back)", r["done"], r["ops"], if r["stopped"] == true { " (stopped)" } else { "" }, r["id"].as_str().unwrap_or(""), r["id"].as_str().unwrap_or(""));
    for f in r["failed"].as_array().into_iter().flatten().take(20) {
        eprintln!("  {}", f["error"].as_str().or(f.as_str()).unwrap_or(""));
    }
    for f in r["added"]["files"].as_array().into_iter().flatten() {
        let what = match (f["game"].as_str(), f["named"] == true) {
            (Some(g), true) => format!("{g} (named as its DAT says)"),
            (Some(g), false) => format!("{g} (organise renames it to its DAT's name)"),
            (None, _) => "no DAT knows it (identify finds what it is)".into(),
        };
        println!("  {}: {what}", f["file"].as_str().unwrap_or(""));
    }
    if let Some(art) = r["art"].as_array().filter(|a| !a.is_empty()) {
        println!("  artwork for {} games", art.len());
    }
    Ok(())
}

/// The app's backend with its settings under `home` (config/, data/).
fn app_for(home: &Path, library_url: &str) -> Result<Arc<App>> {
    App::new(AppPaths {
        config_dir: home.join("config"),
        data_dir: home.join("data"),
        library_url: library_url.into(),
        server_data: home.join("server"),
        server_program: server_program(),
    })
}

/// Game Repo Server for the app's Settings: this program's `serve-devices`.
fn server_program() -> Vec<String> {
    std::env::current_exe().map(|p| vec![p.display().to_string(), "serve-devices".into()]).unwrap_or_default()
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
        // as the app's own (main.rs `mime`)
        "mp4" | "m4v" => "video/mp4",
        "webm" | "mkv" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
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
                                content_type(&app.served_name(rel)),
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
