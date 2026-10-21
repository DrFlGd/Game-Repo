//! A device's plan (D93): the files to write for some games, laid out by the export's own
//! engine (`export::lay_out`) with ES-DE's places, each with where it's fetched from; the
//! files to be made are made in the background, so the plan answers at once. A PC game's
//! files go in the device's PC games folder (place `pc`, D111), from its unpacked copy or
//! its install archives unpacked by the server, with its program for a shortcut (D112).

use crate::catalogue::PC;
use crate::made::{Hashes, Made, Maker};
use crate::unpacked::{Unpacked, Unpacker};
use crate::{catalogue, files, http};
use anyhow::Result;
use gamerepo_core::export::{self, Carry, DeviceAsk};
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::{archive, meta, organise::Op, pcgames, platforms, roots};
use gamerepo_wire::api::{KindSize, PlanAsk};
use gamerepo_wire::{Left, Plan, Program};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The pictures the export can make smaller.
const PICTURES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/// A picture made smaller is reckoned to keep its bytes per pixel.
fn estimate(op: &Op) -> u64 {
    let Some(src) = op.from.first() else { return op.size };
    let size = std::fs::metadata(&src.path).map(|m| m.len()).unwrap_or(op.size);
    let max: u32 = op.names.first().and_then(|n| n.parse().ok()).unwrap_or(0);
    match ::image::image_dimensions(&src.path) {
        Ok((w, h)) if max > 0 && w.max(h) > max => {
            let r = max as f64 / w.max(h) as f64;
            (size as f64 * r * r).ceil() as u64
        }
        _ => size,
    }
}

fn carry(ask: &PlanAsk, media: Vec<String>, manuals: bool) -> Carry {
    Carry {
        fields: if ask.fields.is_empty() { meta::FIELDS.iter().map(|(f, _)| f.to_string()).collect() } else { ask.fields.clone() },
        media,
        manuals,
        image_size: ask.size,
        bios: ask.bios,
        ..Default::default()
    }
}

/// A scratch folder for a plan's places, never made.
fn scratch(data: &Path) -> PathBuf {
    data.join("plans").join(crate::tls::random_hex(8))
}

/// A media kind or a manual: what a media-only plan carries.
fn is_media(role: &str) -> bool {
    role == "manual" || meta::MEDIA.iter().any(|(k, _, _)| *k == role)
}

/// A file's name without its extension (`Quest (USA).m3u`: `Quest (USA)`).
fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() && !s.ends_with('/') => s,
        _ => name,
    }
}

/// Games the device has under names of its own (D102): their media and manuals named for
/// the device's files, as ES-DE finds them (`<system>/<kind>/<the file's name>.<ext>` in
/// its media folder), and their entries' paths the device's.
fn name_for_device(plan: &mut Plan, named: &[(String, String)]) {
    for (key, on_device) in named {
        let on_device = on_device.replace('\\', "/");
        // from the system's folder: `gb/More/Mine.gb` is `More/Mine.gb`
        let Some((_, rel)) = on_device.split_once('/') else { continue };
        let Some(ours) = plan.lists.iter().flat_map(|l| &l.entries).find(|e| &e.key == key).map(|e| e.path.trim_start_matches("./").to_string()) else { continue };
        let (from, to) = (stem(&ours).to_string(), stem(rel).to_string());
        for f in plan.files.iter_mut().filter(|f| &f.key == key && f.place == "media") {
            let parts: Vec<&str> = f.path.splitn(3, '/').collect();
            if let [system, kind, name] = parts[..] {
                if let Some(ext) = name.strip_prefix(from.as_str()).and_then(|r| r.strip_prefix('.')) {
                    f.path = format!("{system}/{kind}/{to}.{ext}");
                }
            }
        }
        for e in plan.lists.iter_mut().flat_map(|l| l.entries.iter_mut()).filter(|e| &e.key == key) {
            e.path = rel.to_string();
        }
    }
}

/// A folder's files, those in hidden folders left out, in order.
pub fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// A PC game's install archives (ZIP, 7z or RAR) that are in its folder, as `game.json`
/// lists them.
pub fn install_archives(e: &pcgames::Entry) -> Vec<PathBuf> {
    e.game
        .files
        .iter()
        .filter(|f| f.kind == "install" && archive::kind_of(&f.path).is_some())
        .map(|f| f.path.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..").fold(e.dir.clone(), |p, part| p.join(part)))
        .filter(|p| p.is_file())
        .collect()
}

/// A path under `base`, with `/`.
fn slashed(base: &Path, p: &Path) -> String {
    p.strip_prefix(base).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// A PC game's pictures that Steam's grid shows (D112).
const STEAM_ART: &[&str] = &["cover", "banner", "hero", "logo", "icon"];

/// The program to start, from the game's files (paths in its folder): `game.json`'s when
/// it's in its unpacked copy, else the likeliest (`.exe`, `.bat`, `.cmd` or `.sh`, not a
/// setup or redistributable, nearest the top).
fn program_of(g: &pcgames::PcGame, names: &[String]) -> (String, String, String) {
    if let Some(l) = &g.launch {
        if let Some(p) = l.program.strip_prefix("extracted/").filter(|p| names.iter().any(|n| n == p)) {
            return (p.to_string(), l.args.clone(), l.dir.strip_prefix("extracted/").unwrap_or("").to_string());
        }
    }
    let mut c: Vec<(usize, &String)> = names
        .iter()
        .filter(|n| ["exe", "bat", "cmd", "sh"].contains(&platforms::ext_of(n).as_str()) && !pcgames::not_the_game(n.rsplit('/').next().unwrap_or(n)))
        .map(|n| (n.matches('/').count(), n))
        .collect();
    c.sort();
    (c.first().map(|(_, n)| n.to_string()).unwrap_or_default(), String::new(), String::new())
}

/// The one folder every file is in, with its `/` (`Game/` of `Game/game.exe` and
/// `Game/data/a.pak`), when there's no file beside it.
fn lone_top(names: &[String]) -> Option<String> {
    let first = names.first()?.split_once('/')?.0;
    names.iter().all(|n| n.split_once('/').is_some_and(|(t, rest)| t == first && !rest.is_empty())).then(|| format!("{first}/"))
}

/// The PC games of a plan: their files in the device's PC games folder, from their unpacked
/// copy in the library or their install archives unpacked here (a game still being unpacked
/// is `waiting`), and their programs.
fn pc_games(ix: &Index, lib: &Library, unpacker: &Unpacker, hashes: &Hashes, ask: &PlanAsk, keys: &[&String], plan: &mut Plan) -> Result<std::result::Result<(), String>> {
    let roots = roots::list(lib);
    let mut hash = vec![];
    for &key in keys {
        let id = &key[PC.len() + 1..];
        let Ok(e) = pcgames::get(lib, id) else { return Ok(Err(format!("There's no game {key} here now (it may have gone): fetch the list again."))) };
        let g = &e.game;
        let folder = g.dir_name();
        let file = |path: &str, size: u64, sha1: Option<String>, url: String| gamerepo_wire::File {
            place: "pc".into(),
            path: format!("{folder}/{path}"),
            size,
            sha1,
            system: PC.into(),
            game: id.to_string(),
            key: key.clone(),
            role: "game".into(),
            url,
            ..Default::default()
        };
        // (its files: path in the game's folder, size, SHA-1 and where it's fetched from)
        let mut got: Vec<(String, u64, Option<String>, String)> = vec![];
        let extracted = e.dir.join("extracted");
        let mut found = vec![];
        walk(&extracted, &mut found);
        if !found.is_empty() {
            for p in &found {
                let rel = slashed(&extracted, p);
                let Some(url) = files::pc_url(lib, p) else {
                    plan.left.push(Left { what: format!("{} › {rel}", g.title), why: "Its file is in a folder the server doesn't serve.".into() });
                    continue;
                };
                let sha1 = Hashes::known(ix.conn(), p);
                if sha1.is_none() {
                    hash.push(p.clone());
                }
                got.push((rel, std::fs::metadata(p).map(|m| m.len()).unwrap_or(0), sha1, url));
            }
        } else {
            let archives = install_archives(&e);
            if archives.is_empty() {
                let why = if g.files.iter().any(|f| f.kind == "install") {
                    "The library has only its installer, and Game Repo Go doesn't run installers: unpack it in Game Repo first (its PC section, Unpack)."
                } else {
                    "The library has none of its game's files."
                };
                plan.left.push(Left { what: g.title.clone(), why: why.into() });
                continue;
            }
            let uid = Unpacker::id_of(&archives);
            match unpacker.status(&uid) {
                Unpacked::Ready(list) => {
                    for f in list {
                        let url = format!("/v1/files/unpacked/{uid}/{}", http::encode_path(&f.path));
                        got.push((f.path, f.size, Some(f.sha1), url));
                    }
                }
                Unpacked::Failed(why) => {
                    plan.left.push(Left { what: g.title.clone(), why: format!("Its archive couldn't be unpacked: {why}") });
                    continue;
                }
                Unpacked::Pending | Unpacked::Unknown => {
                    unpacker.want(&uid, archives);
                    plan.waiting.push(key.clone());
                    if let Some((done, size)) = unpacker.progress(&uid) {
                        plan.unpacking.push(gamerepo_wire::plan::Unpacking { key: key.clone(), done, size });
                    }
                    continue;
                }
            }
        }
        // only the game's own files go in this version (D111)
        for (kind, what) in [("patch", "patches"), ("dlc", "DLC"), ("extra", "extras")] {
            let n = g.files.iter().filter(|f| f.kind == kind).count();
            if n > 0 {
                plan.left.push(Left { what: format!("{} › its {what} ({n})", g.title), why: "Game Repo Go doesn't add a PC game's patches, DLC or extras yet, only the game itself.".into() });
            }
        }
        let names: Vec<String> = got.iter().map(|f| f.0.clone()).collect();
        let (path, args, dir) = program_of(g, &names);
        // a game zipped as its folder: its files go at the top of the game's own folder, not
        // in that folder again (D124)
        let top = lone_top(&names);
        let inside = |p: &str| -> String {
            match &top {
                Some(t) if p == t.trim_end_matches('/') => String::new(),
                Some(t) => p.strip_prefix(t.as_str()).unwrap_or(p).to_string(),
                None => p.to_string(),
            }
        };
        for (rel, size, sha1, url) in got {
            plan.files.push(file(&inside(&rel), size, sha1, url));
        }
        let (path, dir) = (inside(&path), inside(&dir));
        if path.is_empty() {
            plan.notes.push(format!("{} has no program to start, so it gets no shortcut.", g.title));
        }
        let mut art = vec![];
        if ask.steam_art && !path.is_empty() {
            for (kind, rel) in pcgames::media(lib, id) {
                let Some(src) = rel.as_str().and_then(|r| lib.resolve(r).ok()).filter(|p| p.is_file()) else { continue };
                if !STEAM_ART.contains(&kind.as_str()) {
                    continue;
                }
                let Some(url) = files::url_of(lib, &roots, &src) else { continue };
                let ext = src.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
                let at = format!("{id}/{kind}.{ext}");
                let sha1 = Hashes::known(ix.conn(), &src);
                if sha1.is_none() {
                    hash.push(src.clone());
                }
                plan.files.push(gamerepo_wire::File {
                    place: "pcmedia".into(),
                    path: at.clone(),
                    size: std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0),
                    sha1,
                    system: PC.into(),
                    game: id.to_string(),
                    key: key.clone(),
                    role: kind.clone(),
                    url,
                    ..Default::default()
                });
                art.push((kind, "pcmedia".to_string(), at));
            }
        }
        let windows = ["exe", "bat", "cmd"].contains(&platforms::ext_of(&path).as_str());
        plan.programs.push(Program { key: key.clone(), name: g.title.clone(), place: "pc".into(), folder: folder.clone(), path, args, dir, windows, art });
    }
    hashes.want(hash);
    Ok(Ok(()))
}

/// The plan for `ask`. Err(why) when a game asked for isn't in the catalogue.
pub fn plan(ix: &Index, lib: &Library, data: &Path, maker: &Maker, unpacker: &Unpacker, hashes: &Hashes, ask: &PlanAsk) -> Result<std::result::Result<Plan, String>> {
    let (pc, games): (Vec<&String>, Vec<&String>) = ask.games.iter().partition(|k| k.strip_prefix(PC).is_some_and(|r| r.starts_with('/')));
    let mut plan = if games.is_empty() {
        Plan { ready: true, ..Default::default() }
    } else {
        match games_plan(ix, lib, data, maker, hashes, &PlanAsk { games: games.into_iter().cloned().collect(), ..ask.clone() })? {
            Ok(p) => p,
            Err(why) => return Ok(Err(why)),
        }
    };
    if !pc.is_empty() && !ask.media_only {
        if let Err(why) = pc_games(ix, lib, unpacker, hashes, ask, &pc, &mut plan)? {
            return Ok(Err(why));
        }
        plan.ready &= plan.waiting.is_empty();
    }
    Ok(Ok(plan))
}

/// The plan for games other than PC games.
fn games_plan(ix: &Index, lib: &Library, data: &Path, maker: &Maker, hashes: &Hashes, ask: &PlanAsk) -> Result<std::result::Result<Plan, String>> {
    let mut keys = vec![];
    for k in &ask.games {
        match catalogue::keys_of(ix, k)? {
            Some(v) => keys.extend(v),
            None => return Ok(Err(format!("There's no game {k} here now (it may have gone): fetch the list again."))),
        }
    }
    let device = |media: Vec<String>, manuals: bool| DeviceAsk {
        games: keys.clone(),
        layout: ask.layout.clone(),
        carry: carry(ask, media, manuals),
        short_names: false,
        collection: ask.collection.clone(),
    };
    let laid = export::lay_out(ix, lib, &device(ask.media.clone(), ask.manuals), &scratch(data))?;
    let roots = roots::list(lib);
    let mut plan = laid.plan;
    for f in plan.files.iter_mut() {
        // the catalogue game each file is for, or the games needing a shared set
        if !matches!(f.role.as_str(), "bios" | "device" | "bios-file") {
            f.key = catalogue::game_for(ix, &format!("{}/{}", f.system, f.game))?.unwrap_or_default();
        }
        let mut by = vec![];
        for g in &f.needed_by {
            by.extend(catalogue::game_for(ix, &format!("{}/{g}", f.system))?);
        }
        f.needed_by = by;
    }
    for l in &mut plan.lists {
        for e in &mut l.entries {
            e.key = catalogue::game_for(ix, &format!("{}/{}", l.system, e.game))?.unwrap_or_default();
        }
    }
    // only the media, for games the device has (D102), named for its own files
    let ops: Vec<Op> = if ask.media_only || !ask.named.is_empty() {
        let named: Vec<&str> = ask.named.iter().map(|(k, _)| k.as_str()).collect();
        let (files, ops): (Vec<_>, Vec<_>) =
            std::mem::take(&mut plan.files).into_iter().zip(laid.ops).filter(|(f, _)| is_media(&f.role) || !(ask.media_only || named.contains(&f.key.as_str()))).unzip();
        plan.files = files;
        if ask.media_only {
            plan.collections.clear();
        }
        name_for_device(&mut plan, &ask.named);
        ops
    } else {
        laid.ops
    };
    let mut ready = true;
    let mut hash = vec![];
    let mut left = vec![];
    for (f, op) in plan.files.iter_mut().zip(&ops) {
        if !f.made {
            let src = PathBuf::from(&op.from[0].path);
            match files::url_of(lib, &roots, &src) {
                Some(u) => f.url = u,
                None => {
                    left.push(gamerepo_wire::Left { what: f.path.clone(), why: "Its file is in a folder the server doesn't serve.".into() });
                    continue;
                }
            }
            if f.sha1.is_none() {
                f.sha1 = Hashes::known(ix.conn(), &src);
            }
            if f.sha1.is_none() {
                hash.push(src);
            }
            continue;
        }
        let (id, _) = Maker::recipe(op);
        f.url = format!("/v1/files/made/{id}");
        match maker.status(&id) {
            Made::Ready { size, sha1, .. } => {
                f.size = size;
                f.sha1 = Some(sha1);
            }
            _ => {
                maker.want(&id, op);
                ready = false;
                if op.op == "image" {
                    f.size = estimate(op);
                }
            }
        }
    }
    hashes.want(hash);
    plan.ready = ready;
    // each media kind's size, those not asked for too (C13), from a plan carrying every kind
    let mut kinds: BTreeMap<String, KindSize> = BTreeMap::new();
    let all_media: Vec<String> = meta::MEDIA.iter().map(|(k, _, _)| k.to_string()).collect();
    let whole = if all_media.iter().all(|k| ask.media.contains(k)) && ask.manuals { None } else { Some(export::lay_out(ix, lib, &device(all_media.clone(), true), &scratch(data))?) };
    let (files, ops): (&[gamerepo_wire::File], &[Op]) = match &whole {
        Some(w) => (&w.plan.files, &w.ops),
        None => (&plan.files, &ops),
    };
    for (f, op) in files.iter().zip(ops) {
        if f.role != "manual" && !all_media.contains(&f.role) {
            continue;
        }
        let asked = ask.media.contains(&f.role) || (f.role == "manual" && ask.manuals);
        let k = kinds.entry(f.role.clone()).or_insert_with(|| KindSize { kind: f.role.clone(), ..Default::default() });
        k.games += 1;
        let picture = PICTURES.contains(&Path::new(&f.path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default().as_str());
        let size = match plan.files.iter().find(|p| asked && p.place == f.place && p.path == f.path) {
            Some(p) => p.size,
            None if op.op == "image" && picture => {
                k.estimated = true;
                estimate(op)
            }
            None => f.size,
        };
        k.size += size;
    }
    plan.kinds = kinds.into_values().collect();
    plan.files.retain(|f| !f.url.is_empty());
    plan.left.extend(left);
    Ok(Ok(plan))
}

#[cfg(test)]
mod tests {
    use super::lone_top;

    #[test]
    fn a_game_zipped_as_its_folder_has_that_folder_left_out() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(lone_top(&names(&["Game/game.exe", "Game/data/a.pak"])), Some("Game/".into()));
        assert_eq!(lone_top(&names(&["Game/game.exe", "readme.txt"])), None);
        assert_eq!(lone_top(&names(&["Game/game.exe", "Other/b.dat"])), None);
        assert_eq!(lone_top(&names(&["game.exe"])), None);
        assert_eq!(lone_top(&[]), None);
    }
}
