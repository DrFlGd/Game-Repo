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
use anyhow::{anyhow, bail, Context, Result};
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
    /// It has a FOMOD installer (D211), in its `inside` folder
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub fomod: bool,
    /// The FOMOD's options as chosen (none: not chosen yet, so it doesn't go in)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<crate::fomod::Choices>,
    /// The files the choices put in: each from the mod to the game's folder (D211)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<crate::fomod::Put>,
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
            "readme": std::fs::read_to_string(readme_of(&dir, &n)).ok(), "added": e["added"], "installed": e["installed"], "path": p.display().to_string(),
        }));
    }
    let at = installed_at(lib, &g);
    // the mods put in by hand and recorded where it's installed (D210), with when
    let hand: BTreeMap<String, String> = at.as_ref().filter(|p| p.is_dir()).map(|p| gamefiles::State::read(p).hand.into_iter().map(|(n, f)| (n, f.when)).collect()).unwrap_or_default();
    Ok(json!({
        "key": key, "title": g.title, "mods": out, "hows": HOWS.iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
        "installed_at": at.as_ref().map(|p| p.display().to_string()), "installed_found": at.as_ref().is_some_and(|p| p.is_dir()), "hand": hand,
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
        // how it goes in, from how its files are laid out (D211)
        let game_dir = installed_at(lib, &g).filter(|d| d.is_dir());
        let lay = if guess_how(p) == "run" {
            Layout { how: "run", ..Default::default() }
        } else if p.is_file() && is_archive(&name) {
            let at = data_dir.join("mod-import").join(organise::new_id());
            let found = unpack(p, &at).map(|_| layout(&at, game_dir.as_deref())).unwrap_or_default();
            let _ = std::fs::remove_dir_all(&at);
            found
        } else if p.is_dir() {
            layout(p, game_dir.as_deref())
        } else {
            let game = game_dir.as_deref().map(|d| game_files(d, 50_000)).unwrap_or_default();
            let (how, folder) = guess_tops(&[(name.clone(), false)], &game);
            Layout { how, folder, ..Default::default() }
        };
        let h = if lay.how.is_empty() { "hand" } else { lay.how };
        let mut d = Details { title: guessed.title, author: guessed.author, version: guessed.version, how: h.into(), folder: lay.folder, inside: lay.inside, on: matches!(h, "game" | "folder"), ..Default::default() };
        let i = &lay.info;
        for (to, from) in [(&mut d.title, &i.name), (&mut d.author, &i.author), (&mut d.version, &i.version), (&mut d.url, &i.website)] {
            if !from.trim().is_empty() {
                *to = from.trim().to_string();
            }
        }
        if !i.description.trim().is_empty() {
            d.notes = i.description.trim().chars().take(2000).collect();
        }
        if let Some(f) = &lay.fomod {
            d.fomod = true;
            if f.asks() {
                // not in until its options are chosen
                d.on = false;
            } else {
                let r = f.resolve(&Default::default(), &|_| false);
                d.choices = Some(Default::default());
                d.files = r.puts;
            }
        }
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

/// A mod's files where the game is installed: (where each goes, the file it comes from, an
/// archive's unpacked into `staging`). None: it doesn't go in by itself.
fn mod_files(dir: &Path, n: &str, d: &Details, target: &Path, staging: &Path) -> Result<Option<Vec<(PathBuf, PathBuf)>>> {
    let src = dir.join(n);
    if !goes_in_itself(d) {
        return Ok(None);
    }
    let rel_of = |f: &Path, root: &Path| f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/");
    // each of the mod's files with its path in the mod (the archive's, `inside` and all)
    let list: Vec<(PathBuf, String)> = if src.is_dir() {
        crate::cheats::walk(&src).into_iter().map(|f| (f.clone(), rel_of(&f, &src))).collect()
    } else if is_archive(n) {
        let at = staging.join(organise::new_id());
        unpack(&src, &at)?;
        crate::cheats::walk(&at).into_iter().map(|f| (f.clone(), rel_of(&f, &at))).collect()
    } else {
        let inside = d.inside.trim_matches('/');
        vec![(src.clone(), if inside.is_empty() { n.to_string() } else { format!("{inside}/{n}") })]
    };
    // where each goes, in rank order, a later one winning at the same place
    let mut placed: Vec<(usize, String, PathBuf)> = vec![];
    for (f, rel) in list {
        for (at, rank) in device_paths(d, &rel) {
            placed.push((rank, at, f.clone()));
        }
    }
    placed.sort_by_key(|x| x.0);
    let mut out: Vec<(PathBuf, PathBuf)> = vec![];
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (_, at, f) in placed {
        let to = at.split('/').filter(|x| !x.is_empty() && *x != "." && *x != "..").fold(target.to_path_buf(), |p, x| p.join(x));
        match seen.get(&at.to_lowercase()) {
            Some(&i) => out[i] = (to, f),
            None => {
                seen.insert(at.to_lowercase(), out.len());
                out.push((to, f));
            }
        }
    }
    if out.is_empty() {
        bail!("The mod {n} has no files in it{}.", if d.fomod { " for the options chosen" } else { "" });
    }
    Ok(Some(out))
}

/// *Take it off…* (D209 on this computer): an installed mod's files taken off where the game
/// is installed, as a change with Undo: each is set aside, and the file it had replaced
/// put back, from another mod installed that gives the same file, else the library's
/// unpacked copy, else the game's own file the install set aside; a file only it gave is
/// only set aside.
pub fn uninstall_plan(ix: &Index, lib: &Library, data_dir: &Path, key: &str, name: &str) -> Result<Plan> {
    crate::cheats::fresh(ix, lib)?;
    let g = game_of(ix.conn(), key)?;
    let dir = dir_of(lib, &g)?;
    let target = installed_at(lib, &g).ok_or_else(|| anyhow!("Say where {} is installed on this computer first.", g.title))?;
    let mut m = read_json_object(&manifest_of(&dir));
    if m["mods"][name]["installed"]["to"].as_str().is_none() {
        bail!("{name} isn't installed where {} is.", g.title);
    }
    let staging = data_dir.join("mods-staging");
    let d = Details::of(&m["mods"][name], name);
    let Some(mine) = mod_files(&dir, name, &d, &target, &staging)? else { bail!("{name} doesn't go in by itself, so Game Repo didn't install it.") };
    // what each place would have without it: the library's copy, then the other installed mods in order
    let mut under: BTreeMap<String, PathBuf> = BTreeMap::new();
    if let Ok((t, base)) = copies(lib, &g) {
        if t == target {
            for (rel, _) in gamefiles::files_in(&base) {
                under.insert(rel.to_lowercase(), rel.split('/').fold(base.clone(), |p, x| p.join(x)));
            }
        }
    }
    let rel_of = |p: &Path| p.strip_prefix(&target).ok().map(|r| r.to_string_lossy().replace('\\', "/").to_lowercase());
    // a file a mod after it in the order gives too is that mod's now, and stays
    let mut later: HashSet<String> = HashSet::new();
    let mut after = false;
    for n in names_in(&dir, &m) {
        if n == name {
            after = true;
            continue;
        }
        if !m["mods"][&n]["installed"]["to"].is_string() {
            continue;
        }
        let od = Details::of(&m["mods"][&n], &n);
        if let Some(list) = mod_files(&dir, &n, &od, &target, &staging)? {
            for (to, f) in list {
                if let Some(r) = rel_of(&to) {
                    if after {
                        later.insert(r);
                    } else {
                        under.insert(r, f);
                    }
                }
            }
        }
    }
    let mut ops = vec![];
    for (to, _) in &mine {
        if !to.is_file() || rel_of(to).is_some_and(|r| later.contains(&r)) {
            continue;
        }
        let s = crate::export::outside(&target, to);
        ops.push(Op { op: "aside".into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some(format!("{name} taken off")) });
        // else the game's own file the install set aside, the first kept
        let first_aside = || -> Option<PathBuf> {
            let rel = to.strip_prefix(&target).ok()?;
            let mut v: Vec<PathBuf> = std::fs::read_dir(target.join(".gamerepo-set-aside")).ok()?.flatten().map(|e| e.path().join(rel)).filter(|p| p.is_file()).collect();
            v.sort();
            v.into_iter().next()
        };
        if let Some(from) = rel_of(to).and_then(|r| under.get(&r).cloned()).or_else(first_aside) {
            let from = &from;
            let s = outside(from);
            ops.push(Op { op: "copy".into(), game: g.title.clone(), to: to.display().to_string(), size: s.size, from: vec![s], names: vec![], aside: vec![], note: Some("as it was before".into()) });
        }
    }
    if let Some(o) = m["mods"][name].as_object_mut() {
        o.remove("installed");
    }
    ops.push(text_op(&g.title, &manifest_of(&dir), serde_json::to_string_pretty(&sorted(&m))? + "\n", "patches.json: the mod taken off"));
    let mut p = plan_of("mods_install", format!("Took {name} off {}", g.title), format!("Take {name} off {}", g.title), lib, ops, json!({ "key": key, "mods": [name], "skipped": [], "to": target.display().to_string() }));
    p.out = Some(target.display().to_string());
    Ok(p)
}

/// A PC game's ticked mods that go in by themselves, in order, for a device installing it
/// (D205): each one's file or folder in the library and its details.
pub fn device_mods(lib: &Library, id: &str) -> Vec<(PathBuf, Details)> {
    all_mods(lib, id).into_iter().filter(|(_, d)| d.on && goes_in_itself(d)).collect()
}

/// Whether a mod goes in by itself: into the game's folder, or a folder of the game's that's
/// set; one with a FOMOD once its options are chosen (D211).
pub fn goes_in_itself(d: &Details) -> bool {
    (d.how == "game" || (d.how == "folder" && !d.folder.trim().is_empty())) && (!d.fomod || d.choices.is_some())
}

/// Every mod of a PC game, in order, ticked or not (D209: a device picks its own): each
/// one's file or folder in the library and its details.
pub fn all_mods(lib: &Library, id: &str) -> Vec<(PathBuf, Details)> {
    let dir = crate::pcmeta::dir(lib, id).join(patches::FOLDER).join(FOLDER);
    let m = read_json_object(&manifest_of(&dir));
    names_in(&dir, &m).into_iter().map(|n| (dir.join(&n), Details::of(&m["mods"][&n], &n))).collect()
}

/// Where a mod's file goes in the game's folder: `rel` (its path inside the mod, `/`) with
/// its `inside` folder left out and its `folder` in front; None: outside `inside`. A mod
/// with a FOMOD has its files where its choices put them (D211): one may go to several
/// places, or none, each with its rank (a later one wins at the same place).
pub fn device_paths(d: &Details, rel: &str) -> Vec<(String, usize)> {
    let inside = d.inside.trim_matches('/');
    let rel = rel.replace('\\', "/");
    let rel = if inside.is_empty() {
        rel
    } else {
        match rel.get(..inside.len()).filter(|h| h.eq_ignore_ascii_case(inside)).and_then(|_| rel[inside.len()..].strip_prefix('/')) {
            Some(r) => r.to_string(),
            None => return vec![],
        }
    };
    let folder: Vec<&str> = if d.how == "folder" { d.folder.split(['/', '\\']).filter(|x| !x.is_empty() && *x != "." && *x != "..").collect() } else { vec![] };
    let ats = if d.fomod { crate::fomod::places(&d.files, &rel) } else { vec![(rel, 0)] };
    ats.into_iter().map(|(at, i)| (folder.iter().map(|x| x.to_string()).chain(std::iter::once(at)).collect::<Vec<_>>().join("/"), i)).collect()
}

/// [`device_paths`]' first place (a mod without a FOMOD has one).
pub fn device_path(d: &Details, rel: &str) -> Option<String> {
    device_paths(d, rel).into_iter().last().map(|(at, _)| at)
}

// ------------------------------------------------------------ layouts and FOMODs (D211)

/// Folders that are a layout of their own at a mod's top: never taken as a folder its
/// files are wrapped in.
const LAYOUT_TOPS: [&str; 17] = ["data", "bepinex", "plugins", "melonloader", "mods", "bin", "fomod", "textures", "meshes", "scripts", "interface", "sound", "music", "userlibs", "userdata", "config", "content"];
/// A Bethesda game's plugins and archives: they go in its `Data/`.
const BETHESDA: [&str; 5] = [".esp", ".esm", ".esl", ".bsa", ".ba2"];
/// Unreal Engine's packed files: they go where the game's own `.pak` files are.
const PAKS: [&str; 4] = [".pak", ".utoc", ".ucas", ".sig"];

/// How a mod's files are laid out, as a guess (D211).
#[derive(Debug, Default)]
struct Layout {
    inside: String,
    how: &'static str,
    folder: String,
    fomod: Option<crate::fomod::Fomod>,
    info: crate::fomod::Info,
}

fn ext_in(n: &str, list: &[&str]) -> bool {
    let l = n.to_lowercase();
    list.iter().any(|x| l.ends_with(x))
}

/// A game's files, by their paths in its folder (`/`), the first `max` found.
fn game_files(dir: &Path, max: usize) -> Vec<String> {
    let mut out = vec![];
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if e.file_name().to_string_lossy().starts_with(".gamerepo") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

fn parent_of(rel: &str) -> String {
    rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
}

/// Where a game keeps its `.pak` files (Unreal Engine): the folder with most of them, its
/// `~mods` folder when it's a `Paks` folder.
fn pak_folder(game: &[String]) -> Option<String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for f in game.iter().filter(|f| f.to_lowercase().ends_with(".pak")) {
        *count.entry(parent_of(f)).or_default() += 1;
    }
    let (dir, _) = count.into_iter().max_by_key(|(d, n)| (*n, std::cmp::Reverse(d.len())))?;
    let dir = dir.split('/').filter(|x| *x != "~mods").collect::<Vec<_>>().join("/");
    Some(if dir.to_lowercase().ends_with("paks") { format!("{dir}/~mods") } else { dir })
}

/// The folder of the game's that files named like `names` are in (all in one), if any.
fn named_like(game: &[String], names: &[String]) -> Option<String> {
    let mut common: Option<std::collections::BTreeSet<String>> = None;
    for n in names {
        let dirs: std::collections::BTreeSet<String> = game.iter().filter(|f| f.rsplit('/').next().is_some_and(|x| x.eq_ignore_ascii_case(n))).map(|f| parent_of(f)).collect();
        common = Some(match common {
            None => dirs,
            Some(c) => c.intersection(&dirs).cloned().collect(),
        });
    }
    common.and_then(|c| c.into_iter().min_by_key(|d| d.len()))
}

/// The layout of a mod's files in `root` (an archive unpacked, or its folder), with the game
/// installed in `game` when that's known (D211): a FOMOD; else its folder of the game's
/// from its top (`Data/`, `BepInEx/`, `plugins/`, `MelonLoader/`, `Mods/`, `bin/`, plugins
/// and archives of a Bethesda game, `.pak` files, files named like the game's); else by hand.
fn layout(root: &Path, game: Option<&Path>) -> Layout {
    let rels: Vec<String> = crate::cheats::walk(root).iter().map(|f| f.strip_prefix(root).unwrap_or(f).to_string_lossy().replace('\\', "/")).collect();
    let game_list = game.map(|g| game_files(g, 50_000)).unwrap_or_default();
    if let Some(r) = crate::fomod::root_of(rels.iter().map(String::as_str)) {
        let at = |rel: &str| r.split('/').chain(rel.split('/')).filter(|x| !x.is_empty()).fold(root.to_path_buf(), |p, x| p.join(x));
        let find = |name: &str| rels.iter().find(|x| x.eq_ignore_ascii_case(&[r.as_str(), "fomod", name].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join("/"))).cloned();
        let config = find("ModuleConfig.xml").and_then(|c| std::fs::read(root.join(c)).ok()).unwrap_or_default();
        let info = find("info.xml").and_then(|i| std::fs::read(at(&format!("fomod/{}", i.rsplit('/').next().unwrap_or("info.xml")))).ok()).map(|b| crate::fomod::info(&b)).unwrap_or_default();
        // a Bethesda game's FOMOD puts its files in Data/
        let text = crate::fomod::text_of(&config).to_lowercase();
        let bethesda = BETHESDA.iter().any(|x| text.contains(x)) || game_list.iter().any(|f| f.to_lowercase().starts_with("data/") && ext_in(f, &[".esm"]));
        return Layout { inside: r, how: if bethesda { "folder" } else { "game" }, folder: if bethesda { "Data".into() } else { String::new() }, fomod: crate::fomod::parse(&config).ok(), info };
    }
    // a folder its files are wrapped in (not one of the layouts' own)
    let mut inside = String::new();
    let mut here = root.to_path_buf();
    for _ in 0..3 {
        let tops: Vec<PathBuf> = std::fs::read_dir(&here).into_iter().flatten().flatten().map(|e| e.path()).collect();
        match tops.as_slice() {
            [one] if one.is_dir() && !LAYOUT_TOPS.contains(&one.file_name().unwrap_or_default().to_string_lossy().to_lowercase().as_str()) => {
                let n = one.file_name().unwrap_or_default().to_string_lossy().into_owned();
                inside = if inside.is_empty() { n } else { format!("{inside}/{n}") };
                here = one.clone();
            }
            _ => break,
        }
    }
    let tops: Vec<(String, bool)> = std::fs::read_dir(&here).into_iter().flatten().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().is_dir())).collect();
    let (how, folder) = guess_tops(&tops, &game_list);
    Layout { inside, how, folder, ..Default::default() }
}

/// How a mod goes in, by what's at its top: (how, folder).
fn guess_tops(tops: &[(String, bool)], game: &[String]) -> (&'static str, String) {
    let dirs: Vec<String> = tops.iter().filter(|t| t.1).map(|t| t.0.to_lowercase()).collect();
    let files: Vec<&String> = tops.iter().filter(|t| !t.1).map(|t| &t.0).collect();
    let has = |n: &str| dirs.iter().any(|d| d == n);
    if ["data", "bepinex", "melonloader", "mods", "bin"].iter().any(|n| has(n)) {
        return ("game", String::new());
    }
    if has("plugins") && dirs.iter().all(|d| d == "plugins" || d == "config" || d == "patchers") {
        return ("folder", "BepInEx".into());
    }
    let bethesda_dirs = ["textures", "meshes", "scripts", "interface", "sound", "music", "skse", "f4se"];
    if (!files.is_empty() || !dirs.is_empty()) && files.iter().all(|f| ext_in(f, &BETHESDA) || f.to_lowercase().ends_with(".txt")) && files.iter().any(|f| ext_in(f, &BETHESDA)) && dirs.iter().all(|d| bethesda_dirs.contains(&d.as_str())) {
        return ("folder", "Data".into());
    }
    if files.is_empty() && !dirs.is_empty() && dirs.iter().all(|d| bethesda_dirs.contains(&d.as_str())) {
        return ("folder", "Data".into());
    }
    if !files.is_empty() && dirs.is_empty() && files.iter().all(|f| ext_in(f, &PAKS)) {
        if let Some(d) = pak_folder(game) {
            return ("folder", d);
        }
    }
    // files (and folders) named like the game's own: where those are
    if !game.is_empty() && !tops.is_empty() {
        let top_dirs: std::collections::BTreeSet<String> = game.iter().filter_map(|f| f.split_once('/').map(|(d, _)| d.to_lowercase())).collect();
        if !dirs.is_empty() && dirs.iter().all(|d| top_dirs.contains(d)) && files.iter().all(|f| game.iter().any(|g| g.eq_ignore_ascii_case(f))) {
            return ("game", String::new());
        }
        if dirs.is_empty() {
            let names: Vec<String> = files.iter().filter(|f| !f.to_lowercase().starts_with("readme")).map(|f| f.to_string()).collect();
            if !names.is_empty() {
                match named_like(game, &names) {
                    Some(d) if d.is_empty() => return ("game", String::new()),
                    Some(d) => return ("folder", d),
                    None => {}
                }
            }
        }
    }
    ("hand", String::new())
}

/// The mod `name`'s FOMOD files (`fomod/` and its pictures) where they can be read: its own
/// folder for a mod kept as a folder; for an archive, read out once into the mods folder's
/// `.fomod/<name>/` (as the module's folder has them). The folder they're in.
fn fomod_files(dir: &Path, name: &str, d: &Details) -> Result<PathBuf> {
    let src = dir.join(name);
    let inside: Vec<&str> = d.inside.split('/').filter(|x| !x.is_empty()).collect();
    if src.is_dir() {
        return Ok(inside.iter().fold(src, |p, x| p.join(x)));
    }
    let cache = dir.join(".fomod").join(name);
    let stamp = std::fs::metadata(&src).map(|m| format!("{} {:?}", m.len(), m.modified().ok())).unwrap_or_default();
    if std::fs::read_to_string(cache.join(".from")).ok().as_deref() == Some(stamp.as_str()) {
        return Ok(cache);
    }
    let _ = std::fs::remove_dir_all(&cache);
    std::fs::create_dir_all(&cache)?;
    let prefix = if inside.is_empty() { String::new() } else { format!("{}/", inside.join("/")).to_lowercase() };
    let pics = [".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp"];
    let want = |m: &str| {
        let l = m.replace('\\', "/").to_lowercase();
        l.strip_prefix(&prefix).is_some_and(|r| r.starts_with("fomod/") || ext_in(r, &pics))
    };
    let never = std::sync::atomic::AtomicBool::new(false);
    crate::archive::each_wanted(&src, &want, &never, &mut |m, _, r| {
        let l = m.replace('\\', "/");
        let rel = &l[prefix.len().min(l.len())..];
        let to = rel.split('/').filter(|x| !x.is_empty() && *x != "." && *x != "..").fold(cache.clone(), |p, x| p.join(x));
        if l.ends_with('/') || to == cache {
            std::io::copy(r, &mut std::io::sink())?;
            return Ok(());
        }
        if let Some(p) = to.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::io::copy(r, &mut std::fs::File::create(&to)?)?;
        Ok(())
    })
    .with_context(|| format!("couldn't read {name}'s installer"))?;
    std::fs::write(cache.join(".from"), stamp)?;
    Ok(cache)
}

/// A file found in `root` by its path there, any case (`/`).
fn found_in(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut at = root.to_path_buf();
    for part in rel.split('/').filter(|x| !x.is_empty()) {
        let exact = at.join(part);
        at = if exact.exists() {
            exact
        } else {
            std::fs::read_dir(&at).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?.path()
        };
    }
    Some(at)
}

fn fomod_of(lib: &Library, g: &LibGame, name: &str) -> Result<(PathBuf, Details, crate::fomod::Fomod, PathBuf, Value)> {
    let dir = dir_of(lib, g)?;
    if name.contains(['/', '\\']) || !dir.join(name).exists() {
        bail!("{} has no mod {name}.", g.title);
    }
    let m = read_json_object(&manifest_of(&dir));
    let d = Details::of(&m["mods"][name], name);
    if !d.fomod {
        bail!("{name} has no installer (FOMOD) to choose its options in.");
    }
    let root = fomod_files(&dir, name, &d)?;
    let config = found_in(&root, "fomod/ModuleConfig.xml").ok_or_else(|| anyhow!("{name}'s installer (fomod/ModuleConfig.xml) isn't in it any more."))?;
    let f = crate::fomod::parse(&std::fs::read(&config)?)?;
    Ok((dir, d, f, root, m))
}

/// Whether a file a FOMOD asks about is in the game's folder (or its mod's folder, `Data/`).
fn here_fn(lib: &Library, g: &LibGame, d: &Details) -> impl Fn(&str) -> bool {
    let target = installed_at(lib, g).filter(|p| p.is_dir());
    let folder = d.folder.clone();
    move |f: &str| {
        let Some(t) = &target else { return false };
        [String::new(), folder.clone(), "Data".into()].iter().any(|b| found_in(t, &format!("{b}/{f}")).is_some())
    }
}

/// *Choose its options…* (D211): the mod's FOMOD worked through with `choices` (none: those
/// kept, else its defaults), for the page to show one step at a time; its pictures by
/// their paths in the library.
pub fn fomod_show(ix: &Index, lib: &Library, key: &str, name: &str, choices: Option<crate::fomod::Choices>) -> Result<Value> {
    let g = game_of(ix.conn(), key)?;
    let (_, d, f, root, _) = fomod_of(lib, &g, name)?;
    let c = choices.or(d.choices.clone()).unwrap_or_default();
    let r = f.resolve(&c, &here_fn(lib, &g, &d));
    let pic = |p: &str| -> Option<String> {
        if p.is_empty() {
            return None;
        }
        let at = found_in(&root, p)?;
        lib.relative(&at)
    };
    let mut v = serde_json::to_value(&r)?;
    v["image"] = json!(pic(&r.image));
    for s in v["steps"].as_array_mut().into_iter().flatten() {
        for g in s["groups"].as_array_mut().into_iter().flatten() {
            for o in g["options"].as_array_mut().into_iter().flatten() {
                let p = o["image"].as_str().unwrap_or("").to_string();
                o["image"] = json!(pic(&p));
            }
        }
    }
    v["chosen"] = json!(d.choices.is_some());
    v["choices"] = serde_json::to_value(&c)?;
    v["files"] = json!(r.puts.len());
    Ok(v)
}

/// The FOMOD's options chosen, kept with the mod with the files they put in (D211): it goes
/// in by itself from then on, ticked.
pub fn fomod_choose(ix: &Index, lib: &Library, key: &str, name: &str, choices: &crate::fomod::Choices) -> Result<Value> {
    lib.writable()?;
    let g = game_of(ix.conn(), key)?;
    let (dir, d, f, _, mut m) = fomod_of(lib, &g, name)?;
    crate::fomod::check(&f, choices)?;
    let r = f.resolve(choices, &here_fn(lib, &g, &d));
    if !r.cannot.is_empty() {
        bail!("{}", r.cannot);
    }
    if r.puts.is_empty() {
        bail!("Those options put no files in: choose at least one that does.");
    }
    let how = if d.how == "folder" && !d.folder.trim().is_empty() { "folder" } else { "game" };
    let e = &mut m["mods"][name];
    e["choices"] = serde_json::to_value(choices)?;
    e["files"] = serde_json::to_value(&r.puts)?;
    e["how"] = json!(how);
    e["on"] = json!(true);
    e["edited"] = json!(now());
    crate::config::write_json(&manifest_of(&dir), &sorted(&m))?;
    mods(ix, lib, key)
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
        let Some(list) = mod_files(&dir, n, &d, &target, &staging)? else {
            skipped.push(json!({ "name": n, "how": d.how }));
            continue;
        };
        for (to, f) in list {
            files.insert(to, (f, n.clone()));
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

/// The files a PC game installed on this computer should have: the library's unpacked copy,
/// with the mods Game Repo installed there over it in their order (D209): each as a check
/// wants it, and where each comes from. (where it's installed, the files, their sources)
fn want_of(lib: &Library, g: &LibGame, data_dir: Option<&Path>) -> Result<(PathBuf, Vec<gamefiles::Want>, BTreeMap<String, PathBuf>)> {
    let (target, base) = copies(lib, g)?;
    let mut st = gamefiles::State::read(&target);
    let mut want: BTreeMap<String, gamefiles::Want> = BTreeMap::new();
    let mut from: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut cache = std::mem::take(&mut st.library);
    let mut still = BTreeMap::new();
    for (rel, m) in gamefiles::files_in(&base) {
        let sha1 = gamefiles::sha1_seen(&mut cache, &rel, &base.join(&rel), &m, &sha1_path)?;
        if let Some(x) = cache.remove(&rel) {
            still.insert(rel.clone(), x);
        }
        from.insert(rel.to_lowercase(), rel.split('/').fold(base.clone(), |p, x| p.join(x)));
        want.insert(rel.to_lowercase(), gamefiles::Want { path: rel, size: m.len(), sha1 });
    }
    st.library = still;
    st.write(&target)?;
    // the mods installed here by Game Repo, in their order
    if let (Ok(dir), Some(data)) = (dir_of(lib, g), data_dir) {
        let m = read_json_object(&manifest_of(&dir));
        let staging = data.join("mods-staging");
        let here = target.display().to_string();
        for n in names_in(&dir, &m).into_iter().filter(|n| m["mods"][n]["installed"]["to"].as_str() == Some(here.as_str())) {
            let d = Details::of(&m["mods"][&n], &n);
            for (to, f) in mod_files(&dir, &n, &d, &target, &staging)?.unwrap_or_default() {
                let Ok(rel) = to.strip_prefix(&target) else { continue };
                let rel = rel.to_string_lossy().replace('\\', "/");
                let size = std::fs::metadata(&f).map(|x| x.len()).unwrap_or(0);
                want.insert(rel.to_lowercase(), gamefiles::Want { path: rel.clone(), size, sha1: sha1_path(&f)? });
                from.insert(rel.to_lowercase(), f);
            }
        }
    }
    Ok((target, want.into_values().collect(), from))
}

/// The installed game checked (D207): (where, each file's source, what the check found).
fn check_of(lib: &Library, g: &LibGame, data_dir: Option<&Path>) -> Result<(PathBuf, BTreeMap<String, PathBuf>, gamefiles::Check)> {
    let (target, want, from) = want_of(lib, g, data_dir)?;
    let c = gamefiles::check(&target, &want, &sha1_path)?;
    Ok((target, from, c))
}

/// *I've installed it* on this computer (D210): what changed where the game is installed
/// recorded as the mod `name`'s, put in by hand.
pub fn record_hand(ix: &Index, lib: &Library, data_dir: &Path, key: &str, name: &str) -> Result<Value> {
    let g = game_of(ix.conn(), key)?;
    let (target, want, _) = want_of(lib, &g, Some(data_dir))?;
    let fp = gamefiles::record_hand(&target, &want, name, &sha1_path)?;
    Ok(serde_json::to_value(fp)?)
}

/// *Keep these files in the library* (D210): a mod put in by hand, its files as recorded
/// where the game is installed, kept as a mod of its own that goes in by itself (into the
/// game's folder, not ticked). Its name in the game's mods.
pub fn keep_recorded(lib: &Library, id: &str, name: &str, files: &[(String, PathBuf)], note: &str) -> Result<String> {
    if files.is_empty() {
        bail!("{name} changed no files to keep (it only took some away).");
    }
    let dir = crate::pcmeta::dir(lib, id).join(patches::FOLDER).join(FOLDER);
    std::fs::create_dir_all(&dir)?;
    let stem = stem_of(name).trim().to_string();
    let mut new = format!("{stem} (as installed)");
    let mut n = 2;
    while dir.join(&new).exists() {
        new = format!("{stem} (as installed {n})");
        n += 1;
    }
    let to = dir.join(&new);
    for (rel, src) in files {
        let at = rel.split('/').filter(|x| !x.is_empty() && *x != "." && *x != "..").fold(to.clone(), |p, x| p.join(x));
        if let Some(p) = at.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(src, &at).with_context(|| format!("couldn't copy {rel}"))?;
    }
    let path = manifest_of(&dir);
    let mut m = read_json_object(&path);
    let order = m["mods"].as_object().map(|o| o.values().filter_map(|v| v["order"].as_u64()).max().unwrap_or(0)).unwrap_or(0) + 1;
    let title = Details::of(&m["mods"][name], name).title;
    m["mods"][&new] = json!({ "title": format!("{title} (as installed)"), "how": "game", "on": false, "order": order, "notes": note, "added": now() });
    crate::config::write_json(&path, &sorted(&m))?;
    Ok(new)
}

/// *Check the game's files* on this computer (D207): the PC game where it's installed,
/// against the library's unpacked copy.
pub fn check_installed(ix: &Index, lib: &Library, data_dir: &Path, key: &str) -> Result<Value> {
    let g = game_of(ix.conn(), key)?;
    Ok(serde_json::to_value(check_of(lib, &g, Some(data_dir))?.2)?)
}

/// *Put the game back as it was installed…* on this computer (D208), as a change with Undo:
/// a file changed is set aside and copied from the library (or the mod Game Repo installed
/// that gives it, D209), one gone is copied, one added is set aside; those in `leave` stay
/// as they are and are kept as the game's own. The mods recorded by hand go with it (D210);
/// with `only`, it's that one mod put in by hand taken off: only the files it changed.
pub fn put_back_plan(ix: &Index, lib: &Library, data_dir: &Path, key: &str, leave: &[String], only: Option<&str>) -> Result<Plan> {
    let g = game_of(ix.conn(), key)?;
    let (target, want, from) = want_of(lib, &g, Some(data_dir))?;
    let st = gamefiles::State::read(&target);
    let drop: Vec<String> = match only {
        Some(n) if !st.hand.contains_key(n) => bail!("{n} isn't recorded as put in by hand here."),
        Some(n) => vec![n.to_string()],
        None => st.hand.keys().cloned().collect(),
    };
    let mut c = gamefiles::check_without(&target, &want, &drop, &sha1_path)?;
    if let Some(n) = only {
        let paths: HashSet<String> = st.hand[n].paths().iter().map(|p| p.to_lowercase()).collect();
        for l in [&mut c.changed, &mut c.missing, &mut c.added] {
            l.retain(|r| paths.contains(&r.to_lowercase()));
        }
    }
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
    let source = |rel: &str| from.get(&rel.to_lowercase()).filter(|p| p.is_file());
    let mut steps: BTreeMap<String, Vec<Op>> = BTreeMap::new();
    let mut skipped = vec![];
    for rel in c.changed.iter().filter(|r| !left(r)) {
        let to = at(&target, rel);
        match source(rel) {
            Some(f) => {
                steps.insert(rel.clone(), vec![aside(&to, "changed since it was installed"), copy(f, &to)]);
            }
            None => skipped.push(rel.clone()),
        }
    }
    for rel in c.missing.iter().filter(|r| !left(r)) {
        match source(rel) {
            Some(f) => {
                steps.insert(rel.clone(), vec![copy(f, &at(&target, rel))]);
            }
            None => skipped.push(rel.clone()),
        }
    }
    for rel in c.added.iter().filter(|r| !left(r)) {
        steps.insert(rel.clone(), vec![aside(&at(&target, rel), "added since it was installed")]);
    }
    let any = !steps.is_empty();
    for (_, v) in steps {
        ops.extend(v);
    }
    let kept: Vec<String> = c.changed.iter().chain(&c.missing).chain(&c.added).filter(|r| left(r)).cloned().collect();
    if !any && drop.is_empty() {
        bail!("{} is as it was installed: there's nothing to put back.", g.title);
    }
    // the state: the mods put in by hand gone with their files, the files left kept
    let mut after = gamefiles::State::read(&target);
    after.hand.retain(|n, _| !drop.contains(n));
    for k in &kept {
        if !after.keeps(k) {
            after.keep.push(k.clone());
        }
    }
    if after != gamefiles::State::read(&target) {
        ops.push(text_op(&g.title, &gamefiles::State::path(&target), after.text(), "the game's own files and mods, as Game Repo knows them"));
    }
    let n = c.changed.len() + c.missing.len() + c.added.len() - kept.len() - skipped.len();
    let (label, title) = match only {
        Some(m) => (format!("Took {m} off {}", g.title), format!("Take {m} off {} ({n} files)", g.title)),
        None => (format!("Put {} back as the library has it", g.title), format!("Put {} back ({n} files)", g.title)),
    };
    let mut p = plan_of("pc_put_back", label, title, lib, ops, json!({ "key": key, "to": target.display().to_string(), "changed": c.changed, "missing": c.missing, "added": c.added, "kept": kept, "skipped": skipped, "hand": drop }));
    p.out = Some(target.display().to_string());
    Ok(p)
}

/// *Keep these files in the library* on this computer (D210): a mod put in by hand, its
/// files as recorded copied from where the game is installed into a mod of its own.
pub fn keep_hand(ix: &Index, lib: &Library, key: &str, name: &str) -> Result<Value> {
    let g = game_of(ix.conn(), key)?;
    let target = installed_at(lib, &g).ok_or_else(|| anyhow!("Say where {} is installed on this computer first.", g.title))?;
    let st = gamefiles::State::read(&target);
    let fp = st.hand.get(name).ok_or_else(|| anyhow!("{name} isn't recorded as put in by hand here: say *I've installed it* first."))?;
    let mut files = vec![];
    for w in &fp.files {
        let p = w.path.split('/').fold(target.clone(), |p, x| p.join(x));
        if !p.is_file() || (!w.sha1.is_empty() && !sha1_path(&p)?.eq_ignore_ascii_case(&w.sha1)) {
            bail!("{} has changed since {name} was recorded: say *I've changed it* first.", w.path);
        }
        files.push((w.path.clone(), p));
    }
    let id = g.key.strip_prefix("pc/").unwrap_or(&g.key);
    let note = format!("{name}'s files as it was put in by hand, recorded on this computer ({}).", fp.when);
    let new = keep_recorded(lib, id, name, &files, &note)?;
    Ok(json!({ "name": new, "files": files.len() }))
}

/// A mod's readme, for a device's page (D210): the first 20,000 characters.
pub fn readme_text(src: &Path) -> Option<String> {
    let n = src.file_name()?.to_string_lossy().into_owned();
    let t = std::fs::read_to_string(readme_of(src.parent()?, &n)).ok()?;
    Some(t.chars().take(20_000).collect())
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
        // D209: taken off again; then into the game's folder and taken off, the game's own file back
        let p = uninstall_plan(&ix, &lib, &data, key, "HD Textures v1.2.zip").unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert!(!installed.join("Mods/Data/textures.pak").exists());
        assert!(mods(&ix, &lib, key).unwrap()["mods"][0]["installed"].is_null(), "not installed now");
        d.how = "game".into();
        edit(&ix, &lib, key, "HD Textures v1.2.zip", &d).unwrap();
        let p = install_plan(&ix, &lib, &data, key, None).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/textures.pak")).unwrap(), b"NEW");
        let p = uninstall_plan(&ix, &lib, &data, key, "HD Textures v1.2.zip").unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/textures.pak")).unwrap(), b"OLD", "the game's own file back");
        assert!(!installed.join("Data/extra.pak").exists());
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/textures.pak")).unwrap(), b"NEW", "undone: on again");
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
        let data = dir.join("data");
        let c = check_installed(&ix, &lib, &data, key).unwrap();
        assert_eq!(c["state"], "as_installed", "{c}");
        assert!(put_back_plan(&ix, &lib, &data, key, &[], None).unwrap_err().to_string().contains("nothing to put back"));
        // a mod put in by hand: a file over, one added, one deleted, a save made
        std::fs::write(installed.join("Data/level.dat"), b"MODDED").unwrap();
        std::fs::write(installed.join("Data/mod.pak"), b"MOD").unwrap();
        std::fs::write(installed.join("save.sav"), b"SAVE").unwrap();
        std::fs::remove_file(installed.join("game.exe")).unwrap();
        let c = check_installed(&ix, &lib, &data, key).unwrap();
        assert_eq!((c["changed"].clone(), c["added"].clone(), c["missing"].clone()), (json!(["Data/level.dat"]), json!(["Data/mod.pak", "save.sav"]), json!(["game.exe"])), "{c}");
        let p = put_back_plan(&ix, &lib, &data, key, &["save.sav".into()], None).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/level.dat")).unwrap(), b"LEVEL");
        assert_eq!(std::fs::read(installed.join("game.exe")).unwrap(), b"EXE");
        assert!(!installed.join("Data/mod.pak").exists());
        assert_eq!(std::fs::read(installed.join("save.sav")).unwrap(), b"SAVE", "left as it is");
        let c = check_installed(&ix, &lib, &data, key).unwrap();
        assert_eq!((c["state"].as_str(), c["kept"].as_u64()), (Some("as_installed"), Some(1)), "{c}");
        // undone: the mod is back
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/level.dat")).unwrap(), b"MODDED");
        assert_eq!(std::fs::read(installed.join("Data/mod.pak")).unwrap(), b"MOD");
        assert!(!installed.join("game.exe").exists());
        // D210: the same recorded as a mod put in by hand, then the check names it
        let fp = record_hand(&ix, &lib, &data, key, "hand-mod.zip").unwrap();
        assert_eq!(fp["gone"], json!(["game.exe"]), "{fp}");
        let c = check_installed(&ix, &lib, &data, key).unwrap();
        assert_eq!((c["state"].as_str(), c["hand"].clone()), (Some("as_installed"), json!(["hand-mod.zip"])), "{c}");
        // kept in the library as a mod of its own, its files as they are here
        let k = keep_hand(&ix, &lib, key, "hand-mod.zip").unwrap();
        assert_eq!((k["name"].as_str(), k["files"].as_u64()), (Some("hand-mod (as installed)"), Some(3)), "{k}");
        let kept = mods(&ix, &lib, key).unwrap();
        let m = kept["mods"].as_array().unwrap().iter().find(|m| m["name"] == "hand-mod (as installed)").cloned().unwrap();
        assert_eq!((m["details"]["how"].as_str(), m["details"]["on"].as_bool(), m["folder"].as_bool()), (Some("game"), Some(false), Some(true)), "{m}");
        let lib_dir = crate::pcmeta::dir(&lib, "pc-back1").join(patches::FOLDER).join(FOLDER).join("hand-mod (as installed)");
        assert_eq!(std::fs::read(lib_dir.join("Data/level.dat")).unwrap(), b"MODDED");
        // taken off by itself: only its files put back, as Organise changes
        let p = put_back_plan(&ix, &lib, &data, key, &[], Some("hand-mod.zip")).unwrap();
        assert!(p.label.starts_with("Took hand-mod.zip off"), "{}", p.label);
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(installed.join("Data/level.dat")).unwrap(), b"LEVEL");
        assert_eq!(std::fs::read(installed.join("game.exe")).unwrap(), b"EXE");
        assert!(!installed.join("Data/mod.pak").exists() && !installed.join("save.sav").exists());
        let c = check_installed(&ix, &lib, &data, key).unwrap();
        assert_eq!((c["state"].as_str(), c.get("hand")), (Some("as_installed"), None), "{c}");
        organise::undo(&ix, &lib, &p.id, &cancel, &mut |_| {}).unwrap();
        let c = check_installed(&ix, &lib, &data, key).unwrap();
        assert_eq!((c["state"].as_str(), c["hand"].clone()), (Some("as_installed"), json!(["hand-mod.zip"])), "undone: on again, recorded: {c}");
        assert!(put_back_plan(&ix, &lib, &data, key, &[], Some("other.zip")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_mod_with_a_fomod_has_its_options_chosen_and_goes_in_as_they_say() {
        let _turn = crate::platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (dir, lib, ix) = crate::collection::tests::fixture("mods-fomod");
        let cancel = AtomicBool::new(false);
        // a Bethesda game, installed here
        let installed = dir.join("Installed").join("Elder Game");
        std::fs::create_dir_all(installed.join("Data")).unwrap();
        std::fs::write(installed.join("Data/Elder.esm"), b"ESM").unwrap();
        let pc = crate::pcgames::folder(&lib).join("Elder Game (2011)");
        std::fs::create_dir_all(&pc).unwrap();
        let game = crate::pcgames::PcGame { id: "pc-fomod1".into(), title: "Elder Game".into(), year: Some(2011), installed: Some(installed.display().to_string()), ..Default::default() };
        crate::pcgames::write_manifest(&pc, &game).unwrap();
        let key = "pc/pc-fomod1";
        let zip = dir.join("Better Things-123-1-2.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
            let o: zip::write::SimpleFileOptions = Default::default();
            let config = crate::fomod::tests::utf16(crate::fomod::tests::CONFIG);
            let info = br#"<fomod><Name>Better Things</Name><Author>Someone</Author><Version>1.2</Version><Website>https://www.nexusmods.com/elder/mods/123</Website><Description>Makes things better.</Description></fomod>"#;
            for (n, b) in [
                ("Better Things/fomod/ModuleConfig.xml", &config[..]),
                ("Better Things/fomod/info.xml", &info[..]),
                ("Better Things/fomod/images/2k.png", b"PNG"),
                ("Better Things/Core/Better.esp", b"ESP"),
                ("Better Things/Textures/2K/sky.dds", b"2K"),
                ("Better Things/Textures/1K/sky.dds", b"1K"),
                ("Better Things/Extra/2k.ini", b"INI"),
                ("Better Things/readme.txt", b"README"),
                ("Better Things/Patches/fix.esp", b"FIX"),
            ] {
                w.start_file(n, o).unwrap();
                std::io::Write::write_all(&mut w, b).unwrap();
            }
            w.finish().unwrap();
        }
        let data = dir.join("data");
        let p = import_plan(&ix, &lib, &data, key, &[zip.clone()], "copy").unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        let name = "Better Things-123-1-2.zip";
        let v = mods(&ix, &lib, key).unwrap();
        let d = &v["mods"][0]["details"];
        assert_eq!((d["title"].as_str(), d["author"].as_str(), d["fomod"].as_bool(), d["on"].as_bool(), d["how"].as_str(), d["folder"].as_str(), d["inside"].as_str()), (Some("Better Things"), Some("Someone"), Some(true), Some(false), Some("folder"), Some("Data"), Some("Better Things")), "{v}");
        assert!(install_plan(&ix, &lib, &data, key, None).is_err(), "not in until its options are chosen");
        // its options shown, the defaults picked, its pictures from the library
        let s = fomod_show(&ix, &lib, key, name, None).unwrap();
        assert_eq!((s["name"].as_str(), s["chosen"].as_bool()), (Some("Better Things"), Some(false)), "{s}");
        assert_eq!(s["steps"][0]["groups"][0]["options"][1]["picked"], json!(true));
        let pic = s["steps"][0]["groups"][0]["options"][0]["image"].as_str().unwrap_or("").to_string();
        assert!(pic.ends_with("fomod/images/2k.png") && lib.resolve(&pic).unwrap().is_file(), "{pic}");
        // chosen: 2K and its readme, kept with the mod, ticked
        let mut c = crate::fomod::Choices::default();
        c.picks.insert("0/0".into(), vec!["2K".into()]);
        c.picks.insert("1/0".into(), vec!["A readme".into()]);
        assert!(fomod_choose(&ix, &lib, key, name, &crate::fomod::Choices { picks: [("0/0".to_string(), vec!["8K".to_string()])].into(), ..Default::default() }).is_err());
        let v = fomod_choose(&ix, &lib, key, name, &c).unwrap();
        assert_eq!(v["mods"][0]["details"]["on"], json!(true), "{v}");
        let p = install_plan(&ix, &lib, &data, key, None).unwrap();
        organise::run(&ix, &lib, &p, &cancel, &mut |_| {}).unwrap();
        let got = |r: &str| std::fs::read(installed.join(r)).ok();
        assert_eq!((got("Data/Better.esp"), got("Data/textures/sky.dds"), got("Data/Better.ini"), got("Data/Docs/readme.txt")), (Some(b"ESP".to_vec()), Some(b"2K".to_vec()), Some(b"INI".to_vec()), Some(b"README".to_vec())));
        assert!(!installed.join("Data/Patches").exists() && !installed.join("Data/fomod").exists(), "only what the options put in");
        // and the same files for a device (D205)
        let (_, dd) = all_mods(&lib, "pc-fomod1").into_iter().next().unwrap();
        assert!(goes_in_itself(&dd));
        assert_eq!(device_paths(&dd, "Better Things/Textures/2K/sky.dds"), vec![("Data/textures/sky.dds".to_string(), 3)]);
        assert!(device_paths(&dd, "Better Things/Textures/1K/sky.dds").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_common_layouts_say_where_a_mod_goes() {
        let t = |tops: &[(&str, bool)], game: &[&str]| {
            let tops: Vec<(String, bool)> = tops.iter().map(|(n, d)| (n.to_string(), *d)).collect();
            let game: Vec<String> = game.iter().map(|x| x.to_string()).collect();
            let (h, f) = guess_tops(&tops, &game);
            (h.to_string(), f)
        };
        let s = |h: &str, f: &str| (h.to_string(), f.to_string());
        assert_eq!(t(&[("BepInEx", true)], &[]), s("game", ""));
        assert_eq!(t(&[("Data", true), ("readme.txt", false)], &[]), s("game", ""));
        assert_eq!(t(&[("plugins", true)], &[]), s("folder", "BepInEx"));
        assert_eq!(t(&[("MyMod.esp", false), ("textures", true)], &[]), s("folder", "Data"));
        assert_eq!(t(&[("meshes", true), ("textures", true)], &[]), s("folder", "Data"));
        assert_eq!(t(&[("MyMod_P.pak", false)], &["Game/Content/Paks/Game-Windows.pak", "Game/Binaries/Win64/Game.exe"]), s("folder", "Game/Content/Paks/~mods"));
        assert_eq!(t(&[("d3d11.dll", false)], &["bin/x64/game.exe", "bin/x64/d3d11.dll"]), s("folder", "bin/x64"));
        assert_eq!(t(&[("game.ini", false)], &["game.exe", "game.ini"]), s("game", ""));
        assert_eq!(t(&[("random.txt", false)], &["game.exe"]), s("hand", ""));
        assert_eq!(t(&[("Stuff", true)], &["game.exe"]), s("hand", ""));
    }

}
