//! PC games' mods (docs/PLAN.md, "Cheats and patches", D205): kept in the game's metadata
//! folder's `patches/mods/` as they came (an archive, a folder, or an installer), each with
//! its title, author, version, web address and how it goes in, in `patches.json`'s `mods`,
//! in an order. How it goes in is one of:
//! - `game`, into the game's folder: its files unpacked over the game's, what they replace
//!   set aside first, so Undo puts it back;
//! - `folder`, into a folder of the game's (`Mods/`, `Data/Plugins/`…);
//! - `run`, a file to run (an installer or patcher): Game Repo never runs it, and says so;
//! - `hand`, by hand: only kept, its readme shown.
//!
//! Mods go in where the game is installed, never into the library's copy: here, for a PC
//! game whose `game.json` says where it's installed (`installed`), *Install the mods* is a
//! change with Undo. Game Repo Go applying them as it installs a game is a later part.

use crate::cheats::{game_of, LibGame};
use crate::config::{read_json_object, sorted};
use crate::docs::{outside, text_op};
use crate::index::Index;
use crate::library::{now, Library};
use crate::organise::{self, safe_name, Op, Plan};
use crate::patches;
use crate::textures::{is_archive, size_of, unpack};
use gamerepo_wire::gamefiles;
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

pub const FOLDER: &str = "mods";

/// How a mod goes in: (id, what the page calls it).
pub const HOWS: [(&str, &str); 4] = [("game", "Into the game's folder"), ("folder", "Into a folder of the game's"), ("run", "A file to run"), ("hand", "By hand")];

/// A mod's details, as `patches.json` keeps them and the page edits them.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Details {
    pub title: String,
    pub author: String,
    pub version: String,
    pub url: String,
    pub notes: String,
    /// game | folder | run | hand
    pub how: String,
    /// For `folder`: where in the game's folder (`Mods/`)
    pub folder: String,
    /// The folder inside the archive its files are in (a lone top folder named like it)
    pub inside: String,
    /// Ticked to go in with *Install the mods*
    pub on: bool,
}

impl Details {
    fn of(v: &Value, name: &str) -> Details {
        let mut d: Details = serde_json::from_value(v.clone()).unwrap_or_default();
        if d.title.is_empty() {
            d.title = name.to_string();
        }
        if d.how.is_empty() {
            d.how = "hand".into();
        }
        d
    }
}

/// A PC game's mods folder: in its metadata folder (`_library/games/pc/<id>/patches/mods/`).
fn dir_of(lib: &Library, g: &LibGame) -> Result<PathBuf> {
    if g.kind != "pc" {
        bail!("Mods are for PC games: {} isn't one.", g.title);
    }
    let id = g.key.strip_prefix("pc/").unwrap_or(&g.key);
    Ok(crate::pcmeta::dir(lib, id).join(patches::FOLDER).join(FOLDER))
}

fn manifest_of(dir: &Path) -> PathBuf {
    dir.parent().unwrap_or(dir).join(patches::MANIFEST)
}

/// The mods kept in `dir`, in their order (those with none after, by name).
fn names_in(dir: &Path, m: &Value) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.starts_with('.') && !is_readme(n)).collect();
    let order = |n: &String| m["mods"][n]["order"].as_u64().unwrap_or(u64::MAX);
    names.sort_by(|a, b| order(a).cmp(&order(b)).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())));
    names
}

/// A mod's readme is kept beside it as `<mod>.txt`.
fn is_readme(n: &str) -> bool {
    n.to_lowercase().ends_with(".readme.txt")
}

fn readme_of(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.readme.txt"))
}

/// Where a PC game is installed on this computer, when its `game.json` says.
fn installed_at(lib: &Library, g: &LibGame) -> Option<PathBuf> {
    let id = g.key.strip_prefix("pc/")?;
    crate::pcgames::get(lib, id).ok()?.game.installed.map(PathBuf::from)
}

/// How a mod is likely to go in, from what it is.
fn guess_how(p: &Path) -> &'static str {
    let n = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
    if [".exe", ".msi", ".bat", ".cmd"].iter().any(|x| n.ends_with(x)) {
        "run"
    } else if p.is_dir() || is_archive(&n) {
        "game"
    } else {
        "hand"
    }
}

/// A game's mods, in order, with their details.
pub fn mods(ix: &Index, lib: &Library, key: &str) -> Result<Value> {
    crate::cheats::fresh(ix, lib)?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    let m = read_json_object(&manifest_of(&dir));
    let mut out = vec![];
    for n in names_in(&dir, &m) {
        let p = dir.join(&n);
        let e = &m["mods"][&n];
        out.push(json!({
            "name": n, "details": Details::of(e, &n), "size": size_of(&p), "archive": p.is_file() && is_archive(&n), "folder": p.is_dir(),
            "readme": std::fs::read_to_string(readme_of(&dir, &n)).ok(), "added": e["added"], "installed": e["installed"],
        }));
    }
    let at = installed_at(lib, &g);
    Ok(json!({
        "key": key, "title": g.title, "mods": out, "hows": HOWS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "installed_at": at.as_ref().map(|p| p.display().to_string()), "installed_found": at.as_ref().is_some_and(|p| p.is_dir()),
    }))
}

/// Every PC game's mods, for the page's tab.
pub fn list(ix: &Index, lib: &Library) -> Result<Value> {
    crate::cheats::fresh(ix, lib)?;
    let conn = ix.conn();
    let mut q = conn.prepare("SELECT key, kind, platform, dat_game, name, title, folder, picture FROM library WHERE kind = 'pc'")?;
    let games: Vec<LibGame> = q
        .query_map([], |r| Ok(LibGame { key: r.get(0)?, kind: r.get(1)?, platform: r.get(2)?, dat_game: r.get(3)?, name: r.get(4)?, title: r.get(5)?, folder: r.get(6)?, picture: r.get(7)? }))?
        .collect::<rusqlite::Result<_>>()?;
    let mut items = vec![];
    for g in games {
        let Ok(dir) = dir_of(lib, &g) else { continue };
        if !dir.is_dir() {
            continue;
        }
        let m = read_json_object(&manifest_of(&dir));
        for (i, n) in names_in(&dir, &m).into_iter().enumerate() {
            let d = Details::of(&m["mods"][&n], &n);
            let mut o = crate::cheats::shown(&g);
            o["id"] = json!(format!("{}/{n}", g.key));
            o["game_key"] = json!(g.key);
            o["game_title"] = json!(g.title);
            o["file"] = json!(n);
            o["title"] = json!(d.title);
            o["author"] = json!(d.author);
            o["version"] = json!(d.version);
            o["how"] = json!(d.how);
            o["how_name"] = json!(HOWS.iter().find(|h| h.0 == d.how).map(|h| h.1).unwrap_or("By hand"));
            o["on"] = json!(d.on);
            o["order"] = json!(i + 1);
            o["size"] = json!(size_of(&dir.join(&n)));
            o["installed"] = m["mods"][&n]["installed"].clone();
            items.push(o);
        }
    }
    Ok(json!({ "items": items }))
}

/// The folder inside an archive its files are in: a lone top folder named like the mod.
fn inside_of(root: &Path, stem: &str) -> String {
    let tops: Vec<PathBuf> = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let squash = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
    match tops.as_slice() {
        [one] if one.is_dir() && squash(&one.file_name().unwrap_or_default().to_string_lossy()) == squash(stem) => one.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

fn stem_of(name: &str) -> &str {
    if is_archive(name) || name.contains('.') { name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name) } else { name }
}

/// Plan bringing mods in to a PC game: archives, folders or files, hardlinked or copied
/// (`how` "copy"), or moved ("move"), each with how it goes in guessed and a readme found
/// beside it kept with it.
pub fn import_plan(ix: &Index, lib: &Library, data_dir: &Path, key: &str, paths: &[PathBuf], how: &str) -> Result<Plan> {
    lib.writable()?;
    crate::cheats::fresh(ix, lib)?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    let mut m = read_json_object(&manifest_of(&dir));
    let mut next = names_in(&dir, &m).len() as u64 + 1;
    let kind = if how == "move" { "move" } else { "link" };
    let op_of = |from: &Path, to: &Path| -> Op {
        let s = outside(from);
        Op { op: kind.into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some("a mod".into()) }
    };
    let mut ops = vec![];
    for p in paths {
        let name = safe_name(&p.file_name().unwrap_or_default().to_string_lossy());
        if name.is_empty() {
            continue;
        }
        if dir.join(&name).exists() {
            bail!("{} has a mod called {name} already.", g.title);
        }
        if p.is_dir() {
            for f in crate::cheats::walk(p) {
                let rel = f.strip_prefix(p).unwrap_or(&f);
                ops.push(op_of(&f, &dir.join(&name).join(rel)));
            }
        } else if p.is_file() {
            ops.push(op_of(p, &dir.join(&name)));
        } else {
            bail!("{} isn't there.", p.display());
        }
        let stem = stem_of(&name).to_string();
        // a readme beside it: <stem>.txt, readme.txt
        let readme = p.parent().and_then(|d| [format!("{stem}.txt"), "readme.txt".into(), "README.txt".into(), "ReadMe.txt".into()].into_iter().map(|n| d.join(n)).find(|r| r.is_file() && r != p));
        let text = readme.as_ref().and_then(|r| std::fs::read_to_string(r).ok()).unwrap_or_default();
        if let Some(r) = &readme {
            ops.push(Op { op: "copy".into(), game: g.title.clone(), to: readme_of(&dir, &name).display().to_string(), size: std::fs::metadata(r).map(|x| x.len()).unwrap_or(0), from: vec![outside(r)], names: vec![], aside: vec![], note: Some("its readme".into()) });
        }
        let guessed = patches::guess(&format!("{stem}.x"), &text);
        let h = guess_how(p);
        let inside = if p.is_file() && is_archive(&name) {
            let at = data_dir.join("mod-import").join(organise::new_id());
            let found = unpack(p, &at).map(|_| inside_of(&at, &stem)).unwrap_or_default();
            let _ = std::fs::remove_dir_all(&at);
            found
        } else {
            String::new()
        };
        let d = Details { title: guessed.title, author: guessed.author, version: guessed.version, how: h.into(), inside, on: matches!(h, "game" | "folder"), ..Default::default() };
        let mut e = serde_json::to_value(&d)?;
        e["order"] = json!(next);
        e["added"] = json!(now());
        next += 1;
        m["mods"][&name] = e;
    }
    if ops.is_empty() {
        bail!("Choose a mod: its archive, its folder or its file.");
    }
    ops.push(text_op(&g.title, &manifest_of(&dir), serde_json::to_string_pretty(&sorted(&m))? + "\n", "patches.json: the mods' details"));
    let n = paths.len();
    let label = if n == 1 { "a mod".to_string() } else { format!("{n} mods") };
    Ok(plan_of("mods", format!("Brought in {label} for {}", g.title), format!("Bring in {label} for {}", g.title), lib, ops, json!({ "key": key })))
}

fn plan_of(kind: &str, label: String, title: String, lib: &Library, ops: Vec<Op>, dat: Value) -> Plan {
    let mut p = crate::textures::plan_of(kind, label, title, lib, ops, "pc", dat);
    p.games = 1;
    p
}

/// A PC game's ticked mods that go in by themselves, in order, for a device installing it
/// (D205): each one's file or folder in the library and its details.
pub fn device_mods(lib: &Library, id: &str) -> Vec<(PathBuf, Details)> {
    let dir = crate::pcmeta::dir(lib, id).join(patches::FOLDER).join(FOLDER);
    let m = read_json_object(&manifest_of(&dir));
    names_in(&dir, &m)
        .into_iter()
        .map(|n| (dir.join(&n), Details::of(&m["mods"][&n], &n)))
        .filter(|(_, d)| d.on && (d.how == "game" || (d.how == "folder" && !d.folder.trim().is_empty())))
        .collect()
}

/// Where a mod's file goes in the game's folder: `rel` (its path inside the mod, `/`) with
/// its `inside` folder left out and its `folder` in front; None: outside `inside`.
pub fn device_path(d: &Details, rel: &str) -> Option<String> {
    let inside = d.inside.trim_matches('/');
    let rel = if inside.is_empty() { rel.to_string() } else { rel.strip_prefix(inside)?.strip_prefix('/')?.to_string() };
    let folder: Vec<&str> = if d.how == "folder" { d.folder.split(['/', '\\']).filter(|x| !x.is_empty() && *x != "." && *x != "..").collect() } else { vec![] };
    Some(folder.into_iter().map(String::from).chain(std::iter::once(rel)).collect::<Vec<_>>().join("/"))
}

/// Change a mod's details (written straight away: it's the mod's description, not a change).
pub fn edit(ix: &Index, lib: &Library, key: &str, name: &str, d: &Details) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    if name.contains(['/', '\\']) || !dir.join(name).exists() {
        bail!("{} has no mod {name}.", g.title);
    }
    if !HOWS.iter().any(|h| h.0 == d.how) {
        bail!("A mod goes in one of four ways: into the game's folder, into a folder of the game's, a file to run, or by hand.");
    }
    if d.folder.split(['/', '\\']).any(|x| x == "..") || Path::new(d.folder.trim()).is_absolute() || d.folder.contains(':') {
        bail!("Say which folder of the game's it goes into, inside it, such as Mods/.");
    }
    let mut d = d.clone();
    if d.how == "folder" && d.folder.trim().is_empty() {
        // not ticked until it says where
        d.on = false;
    }
    let d = &d;
    let mut m = read_json_object(&manifest_of(&dir));
    let mut e = m["mods"].get(name).cloned().unwrap_or(json!({}));
    for (k, v) in serde_json::to_value(d)?.as_object().into_iter().flatten() {
        e[k] = v.clone();
    }
    e["edited"] = json!(now());
    m["mods"][name] = e;
    crate::config::write_json(&manifest_of(&dir), &sorted(&m))?;
    mods(ix, lib, key)
}

/// Put a game's mods in this order (the names first, the rest after them as they were).
pub fn order(ix: &Index, lib: &Library, key: &str, names: &[String]) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    let mut m = read_json_object(&manifest_of(&dir));
    let had = names_in(&dir, &m);
    let mut all: Vec<String> = names.iter().filter(|n| had.contains(n)).cloned().collect();
    all.extend(had.into_iter().filter(|n| !names.contains(n)));
    for (i, n) in all.iter().enumerate() {
        if m["mods"].get(n).is_none() {
            m["mods"][n] = json!({});
        }
        m["mods"][n]["order"] = json!(i + 1);
    }
    crate::config::write_json(&manifest_of(&dir), &sorted(&m))?;
    mods(ix, lib, key)
}

/// Take a mod away from its game: moved to `patches/mods/.removed/<when>/` with its readme.
/// What it put where the game is installed stays: History undoes that.
pub fn remove(ix: &Index, lib: &Library, key: &str, name: &str) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    let src = dir.join(name);
    if name.contains(['/', '\\']) || !src.exists() {
        bail!("{} has no mod {name}.", g.title);
    }
    let to = dir.join(".removed").join(now().replace([':', 'T'], "-").trim_end_matches('Z'));
    std::fs::create_dir_all(&to)?;
    std::fs::rename(&src, to.join(name))?;
    let r = readme_of(&dir, name);
    if r.is_file() {
        let _ = std::fs::rename(&r, to.join(r.file_name().unwrap_or_default()));
    }
    mods(ix, lib, key)
}

/// Plan *Install the mods*: each ticked mod (or `names`) that goes into the game's folder
/// or a folder of it, in order, its files put where the game is installed, what they
/// replace set aside first (a later mod's file over an earlier's); archives unpacked into
/// `<data dir>/mods-staging/` first. Files to run and mods by hand are said, not done.
pub fn install_plan(ix: &Index, lib: &Library, data_dir: &Path, key: &str, names: Option<&[String]>) -> Result<Plan> {
    crate::cheats::fresh(ix, lib)?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    let target = installed_at(lib, &g).ok_or_else(|| anyhow!("Say where {} is installed on this computer first (on its page: Where it's installed).", g.title))?;
    if !target.is_dir() {
        bail!("{} isn't there: is {} still installed?", target.display(), g.title);
    }
    let mut m = read_json_object(&manifest_of(&dir));
    let chosen: Vec<String> = names_in(&dir, &m)
        .into_iter()
        .filter(|n| match names {
            Some(ns) => ns.contains(n),
            None => Details::of(&m["mods"][n], n).on,
        })
        .collect();
    if chosen.is_empty() {
        bail!("Tick the mods to install.");
    }
    let staging = data_dir.join("mods-staging");
    for e in std::fs::read_dir(&staging).into_iter().flatten().flatten() {
        if e.metadata().and_then(|x| x.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 86_400) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    // each place's file, the last mod's (BTreeMap: the steps in path order)
    let mut files: BTreeMap<PathBuf, (PathBuf, String)> = BTreeMap::new();
    let mut put = vec![];
    let mut skipped = vec![];
    for n in &chosen {
        let d = Details::of(&m["mods"][n], n);
        let src = dir.join(n);
        let base = match d.how.as_str() {
            "game" => target.clone(),
            "folder" if !d.folder.trim().is_empty() => d.folder.split(['/', '\\']).filter(|x| !x.is_empty() && *x != "." && *x != "..").fold(target.clone(), |p, x| p.join(x)),
            _ => {
                skipped.push(json!({ "name": n, "how": d.how }));
                continue;
            }
        };
        let list: Vec<(PathBuf, PathBuf)> = if src.is_dir() {
            crate::cheats::walk(&src).into_iter().map(|f| (f.clone(), f.strip_prefix(&src).unwrap_or(&f).to_path_buf())).collect()
        } else if is_archive(n) {
            let at = staging.join(organise::new_id());
            unpack(&src, &at)?;
            let root = if d.inside.is_empty() { at.clone() } else { d.inside.split(['/', '\\']).filter(|x| !x.is_empty() && *x != "..").fold(at.clone(), |p, x| p.join(x)) };
            crate::cheats::walk(&root).into_iter().map(|f| (f.clone(), f.strip_prefix(&root).unwrap_or(&f).to_path_buf())).collect()
        } else {
            vec![(src.clone(), PathBuf::from(n))]
        };
        if list.is_empty() {
            bail!("The mod {n} has no files in it.");
        }
        for (f, rel) in list {
            files.insert(base.join(rel), (f, n.clone()));
        }
        put.push(n.clone());
    }
    if put.is_empty() {
        bail!("None of those go in by themselves: a file to run is run by you (Game Repo never runs it), and a mod by hand is put in as its readme says.");
    }
    let mut ops = vec![];
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for (to, (from, name)) in &files {
        if !seen.insert(to.clone()) {
            continue;
        }
        if to.is_file() {
            // what it replaces, kept to put back (in the game's .gamerepo-set-aside/)
            let s = crate::export::outside(&target, to);
            ops.push(Op { op: "aside".into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some(format!("replaced by {name}")) });
        }
        let s = outside(from);
        ops.push(Op { op: "copy".into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some(name.clone()) });
    }
    for n in &put {
        m["mods"][n]["installed"] = json!({ "to": target.display().to_string(), "when": now() });
    }
    ops.push(text_op(&g.title, &manifest_of(&dir), serde_json::to_string_pretty(&sorted(&m))? + "\n", "patches.json: where the mods went"));
    let label = if put.len() == 1 { put[0].clone() } else { format!("{} mods", put.len()) };
    let mut p = plan_of("mods_install", format!("Installed {label} for {}", g.title), format!("Install {label} for {}", g.title), lib, ops, json!({ "key": key, "mods": put, "skipped": skipped, "to": target.display().to_string() }));
    p.out = Some(target.display().to_string());
    Ok(p)
}

/// A file's SHA-1, read a piece at a time.
fn sha1_path(p: &Path) -> std::io::Result<String> {
    use sha1::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut h = sha1::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A PC game installed on this computer and the library's unpacked copy of it, the folder
/// its files are in (a copy zipped as its folder: that folder, D124), for D207.
fn copies(lib: &Library, g: &LibGame) -> Result<(PathBuf, PathBuf)> {
    let target = installed_at(lib, g).ok_or_else(|| anyhow!("Say where {} is installed on this computer first (on its page: Where it's installed).", g.title))?;
    if !target.is_dir() {
        bail!("{} isn't there: is {} still installed?", target.display(), g.title);
    }
    let id = g.key.strip_prefix("pc/").unwrap_or(&g.key);
    let e = crate::pcgames::get(lib, id)?;
    let mut base = e.dir.join("extracted");
    if gamefiles::files_in(&base).is_empty() {
        bail!("The library has no unpacked copy of {} to check it against: unpack it first (its PC section, Unpack).", g.title);
    }
    let tops: Vec<PathBuf> = std::fs::read_dir(&base).into_iter().flatten().flatten().map(|e| e.path()).collect();
    if let [only] = tops.as_slice() {
        if only.is_dir() && !target.join(only.file_name().unwrap_or_default()).is_dir() {
            base = only.clone();
        }
    }
    Ok((target, base))
}

/// The installed game checked against the library's copy: (where, the library's copy, what
/// the check found).
fn check_of(lib: &Library, g: &LibGame) -> Result<(PathBuf, PathBuf, gamefiles::Check)> {
    let (target, base) = copies(lib, g)?;
    let mut st = gamefiles::State::read(&target);
    let mut want = vec![];
    let mut cache = std::mem::take(&mut st.library);
    let mut still = BTreeMap::new();
    for (rel, m) in gamefiles::files_in(&base) {
        let sha1 = gamefiles::sha1_seen(&mut cache, &rel, &base.join(&rel), &m, &sha1_path)?;
        if let Some(x) = cache.remove(&rel) {
            still.insert(rel.clone(), x);
        }
        want.push(gamefiles::Want { path: rel, size: m.len(), sha1 });
    }
    st.library = still;
    st.write(&target)?;
    let c = gamefiles::check(&target, &want, &sha1_path)?;
    Ok((target, base, c))
}

/// *Check the game's files* on this computer (D207): the PC game where it's installed,
/// against the library's unpacked copy.
pub fn check_installed(ix: &Index, lib: &Library, key: &str) -> Result<Value> {
    let g = game_of(ix.conn(), key)?;
    Ok(serde_json::to_value(check_of(lib, &g)?.2)?)
}

/// *Put the game back as it was installed…* on this computer (D208), as a change with Undo:
/// a file changed is set aside and copied from the library, one gone is copied, one added
/// is set aside; those in `leave` stay as they are and are kept as the game's own.
pub fn put_back_plan(ix: &Index, lib: &Library, key: &str, leave: &[String]) -> Result<Plan> {
    let g = game_of(ix.conn(), key)?;
    let (target, base, c) = check_of(lib, &g)?;
    let left = |r: &String| leave.iter().any(|l| l.eq_ignore_ascii_case(r));
    let at = |root: &Path, rel: &str| rel.split('/').fold(root.to_path_buf(), |p, x| p.join(x));
    let mut ops = vec![];
    let aside = |to: &Path, why: &str| {
        let s = crate::export::outside(&target, to);
        Op { op: "aside".into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some(why.into()) }
    };
    let copy = |from: &Path, to: &Path| {
        let s = outside(from);
        Op { op: "copy".into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some("as the library has it".into()) }
    };
    let mut steps: BTreeMap<String, Vec<Op>> = BTreeMap::new();
    for rel in c.changed.iter().filter(|r| !left(r)) {
        let to = at(&target, rel);
        steps.insert(rel.clone(), vec![aside(&to, "changed since it was installed"), copy(&at(&base, rel), &to)]);
    }
    for rel in c.missing.iter().filter(|r| !left(r)) {
        steps.insert(rel.clone(), vec![copy(&at(&base, rel), &at(&target, rel))]);
    }
    for rel in c.added.iter().filter(|r| !left(r)) {
        steps.insert(rel.clone(), vec![aside(&at(&target, rel), "added since it was installed")]);
    }
    for (_, v) in steps {
        ops.extend(v);
    }
    let kept: Vec<String> = c.changed.iter().chain(&c.missing).chain(&c.added).filter(|r| left(r)).cloned().collect();
    if !kept.is_empty() {
        let mut st = gamefiles::State::read(&target);
        for k in &kept {
            if !st.keeps(k) {
                st.keep.push(k.clone());
            }
        }
        st.write(&target)?;
    }
    if ops.is_empty() {
        bail!("{} is as it was installed: there's nothing to put back.", g.title);
    }
    let n = c.changed.len() + c.missing.len() + c.added.len() - kept.len();
    let mut p = plan_of("pc_put_back", format!("Put {} back as the library has it", g.title), format!("Put {} back ({n} files)", g.title), lib, ops, json!({ "key": key, "to": target.display().to_string(), "changed": c.changed, "missing": c.missing, "added": c.added, "kept": kept }));
    p.out = Some(target.display().to_string());
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn a_mod_is_brought_in_ordered_and_installed_over_the_game_then_undone() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("mods");
        let cancel = AtomicBool::new(false);
        // a PC game installed on this computer
        let installed = dir.join("Installed").join("Mod Game");
        std::fs::create_dir_all(installed.join("Data")).unwrap();
        std::fs::write(installed.join("Data/textures.pak"), b"OLD").unwrap();
        std::fs::write(installed.join("game.exe"), b"EXE").unwrap();
        let pc = crate::pcgames::folder(&lib).join("Mod Game (2001)");
        std::fs::create_dir_all(&pc).unwrap();
        let game = crate::pcgames::PcGame { id: "pc-mods1".into(), title: "Mod Game".into(), year: Some(2001), installed: Some(installed.display().to_string()), ..Default::default() };
        crate::pcgames::write_manifest(&pc, &game).unwrap();
        let key = "pc/pc-mods1";
        // the mod: a ZIP with a lone top folder named like it, and a readme beside it
        let downloads = dir.join("downloads");
        std::fs::create_dir_all(&downloads).unwrap();
        let zip = downloads.join("HD Textures v1.2.zip");
        {
            let f = std::fs::File::create(&zip).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let o: zip::write::SimpleFileOptions = Default::default();
            for (n, b) in [("HD Textures v1.2/Data/textures.pak", &b"NEW"[..]), ("HD Textures v1.2/Data/extra.pak", b"EXTRA")] {
                w.start_file(n, o).unwrap();
                std::io::Write::write_all(&mut w, b).unwrap();
            }
            w.finish().unwrap();
        }
        std::fs::write(downloads.join("HD Textures v1.2.txt"), "HD Textures\nby Modder\n").unwrap();
        std::fs::write(downloads.join("patcher.exe"), b"MZ").unwrap();
        let data = dir.join("data");
        let p = import_plan(&ix, &lib, &data, key, &[zip.clone(), downloads.join("patcher.exe")], "copy").unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        let v = mods(&ix, &lib, key).unwrap();
        let ms = v["mods"].as_array().unwrap();
        assert_eq!(ms.len(), 2, "{v}");
        assert_eq!(ms[0]["name"], json!("HD Textures v1.2.zip"));
        assert_eq!((ms[0]["details"]["how"].as_str(), ms[0]["details"]["inside"].as_str(), ms[0]["details"]["on"].as_bool()), (Some("game"), Some("HD Textures v1.2"), Some(true)), "{v}");
        assert_eq!(ms[0]["details"]["version"], json!("1.2"));
        assert!(ms[0]["readme"].as_str().unwrap_or("").contains("Modder"));
        assert_eq!((ms[1]["details"]["how"].as_str(), ms[1]["details"]["on"].as_bool()), (Some("run"), Some(false)));
        assert!(zip.is_file(), "the download stays");
        // the order changed: the patcher first
        let v = order(&ix, &lib, key, &["patcher.exe".into()]).unwrap();
        assert_eq!(v["mods"][0]["name"], json!("patcher.exe"));
        assert_eq!(list(&ix, &lib).unwrap()["items"].as_array().unwrap().len(), 2);
        // a file to run alone isn't installed
        assert!(install_plan(&ix, &lib, &data, key, Some(&["patcher.exe".into()])).is_err());
        // installed: the ticked ZIP's files over the game's, the one it replaces set aside
        let p = install_plan(&ix, &lib, &data, key, None).unwrap();
        assert_eq!(p.dat["mods"], json!(["HD Textures v1.2.zip"]));
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/textures.pak")).unwrap(), b"NEW");
        assert_eq!(std::fs::read(installed.join("Data/extra.pak")).unwrap(), b"EXTRA");
        assert!(mods(&ix, &lib, key).unwrap()["mods"][1]["installed"]["to"].is_string());
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/textures.pak")).unwrap(), b"OLD", "undone: the game's own file is back");
        assert!(!installed.join("Data/extra.pak").exists());
        // a folder of the game's, then edited details
        let mut d: Details = serde_json::from_value(mods(&ix, &lib, key).unwrap()["mods"][1]["details"].clone()).unwrap();
        d.how = "folder".into();
        d.folder = "../x".into();
        assert!(edit(&ix, &lib, key, "HD Textures v1.2.zip", &d).is_err());
        d.folder = "Mods/".into();
        d.author = "Modder".into();
        edit(&ix, &lib, key, "HD Textures v1.2.zip", &d).unwrap();
        let p = install_plan(&ix, &lib, &data, key, None).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Mods/Data/textures.pak")).unwrap(), b"NEW");
        // removed: kept aside
        remove(&ix, &lib, key, "patcher.exe").unwrap();
        assert_eq!(mods(&ix, &lib, key).unwrap()["mods"].as_array().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_installed_game_is_checked_against_the_librarys_copy_and_put_back_with_undo() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("mods-put-back");
        let cancel = AtomicBool::new(false);
        // the library's copy, zipped as its folder and unpacked; the game installed from it
        let pc = crate::pcgames::folder(&lib).join("Back Game (2004)");
        let copy = pc.join("extracted").join("Back Game");
        let installed = dir.join("Installed").join("Back Game");
        for root in [&copy, &installed] {
            std::fs::create_dir_all(root.join("Data")).unwrap();
            std::fs::write(root.join("game.exe"), b"EXE").unwrap();
            std::fs::write(root.join("Data/level.dat"), b"LEVEL").unwrap();
        }
        let game = crate::pcgames::PcGame { id: "pc-back1".into(), title: "Back Game".into(), year: Some(2004), installed: Some(installed.display().to_string()), ..Default::default() };
        crate::pcgames::write_manifest(&pc, &game).unwrap();
        crate::cheats::fresh(&ix, &lib).unwrap();
        let key = "pc/pc-back1";
        let c = check_installed(&ix, &lib, key).unwrap();
        assert_eq!(c["state"], "as_installed", "{c}");
        assert!(put_back_plan(&ix, &lib, key, &[]).unwrap_err().to_string().contains("nothing to put back"));
        // a mod put in by hand: a file over, one added, one deleted, a save made
        std::fs::write(installed.join("Data/level.dat"), b"MODDED").unwrap();
        std::fs::write(installed.join("Data/mod.pak"), b"MOD").unwrap();
        std::fs::write(installed.join("save.sav"), b"SAVE").unwrap();
        std::fs::remove_file(installed.join("game.exe")).unwrap();
        let c = check_installed(&ix, &lib, key).unwrap();
        assert_eq!((c["changed"].clone(), c["added"].clone(), c["missing"].clone()), (json!(["Data/level.dat"]), json!(["Data/mod.pak", "save.sav"]), json!(["game.exe"])), "{c}");
        let p = put_back_plan(&ix, &lib, key, &["save.sav".into()]).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/level.dat")).unwrap(), b"LEVEL");
        assert_eq!(std::fs::read(installed.join("game.exe")).unwrap(), b"EXE");
        assert!(!installed.join("Data/mod.pak").exists());
        assert_eq!(std::fs::read(installed.join("save.sav")).unwrap(), b"SAVE", "left as it is");
        let c = check_installed(&ix, &lib, key).unwrap();
        assert_eq!((c["state"].as_str(), c["kept"].as_u64()), (Some("as_installed"), Some(1)), "{c}");
        // undone: the mod is back
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/level.dat")).unwrap(), b"MODDED");
        assert_eq!(std::fs::read(installed.join("Data/mod.pak")).unwrap(), b"MOD");
        assert!(!installed.join("game.exe").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
