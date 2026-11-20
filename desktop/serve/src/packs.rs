//! A game's packs of maps and guides for Atlas on devices (docs/PLAN.md, "Maps and Guides:
//! Atlas packs", D222): the packs linked to a game in the library's `Maps and Guides/`
//! (`gamerepo_core::maps::linked`), listed with the game for a device's page (`GET
//! /v1/games/<key>`'s `packs`) and added to its plan as files of their own when the device
//! asks for them (`PlanAsk::packs`): role `pack`, place `atlas` (Atlas's packs folder,
//! which the device knows), its file's name, size and SHA-1 (read once and remembered by
//! size and time, as every file is), its `packId`, and its `pack.updated` and `exportedAt`
//! (so Go can leave out a pack when Atlas's folder holds a copy it didn't deliver that's as
//! new or newer, "Maps and Guides notes, part 6"). A pack Game Repo can't read (a
//! newer format) is sent as it is. Packs go to paired devices only, as every file does,
//! and are never in an export to a folder or card.
//!
//! Maps and guides on their own (D225, "Maps and Guides notes, part 8"): `GET /v1/packs`
//! lists every pack in the library's `Maps and Guides/` that Game Repo can read, linked to a
//! game or not, each with the title it's grouped under ([`library`]); `GET
//! /v1/packs/<packId>/file` sends one by its `packId` as every file is sent (ranges, ETags,
//! `?sha1`; `lib.rs`), and `GET /v1/packs/<packId>/cover` its picture ([`cover`]). A plan
//! asked with `pack_ids` and no games has those packs alone, with no game's key
//! ([`add_alone`]), so Game Repo Go fetches them through its queue as it fetches a game's.

use crate::catalogue;
use crate::files;
use crate::made::Hashes;
use anyhow::Result;
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::maps::{self, Linked};
use gamerepo_core::roots;
use gamerepo_wire::api::{LibraryPack, Pack, PackGame, PackList, PlanAsk};
use gamerepo_wire::{Left, Plan};
use rusqlite::OptionalExtension;
use std::collections::HashSet;
use std::path::PathBuf;

/// The place a device puts packs in: Atlas's packs folder.
pub const PLACE: &str = "atlas";
/// A pack's role in a plan.
pub const ROLE: &str = "pack";

/// The packs linked to any of a catalogue game's Library keys (its discs), each once.
fn of_keys<'a>(all: &'a [Linked], lib_keys: &[String]) -> Vec<&'a Linked> {
    let mut seen = HashSet::new();
    all.iter().filter(|p| p.games.iter().any(|g| lib_keys.contains(&g.key)) && seen.insert(p.pack_id.as_str())).collect()
}

fn file_name(p: &Linked) -> String {
    p.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What a device's page shows of a pack: from its description, or its file's name and why
/// it can't be read.
pub fn info(p: &Linked) -> Pack {
    let d = &p.description;
    let file = file_name(p);
    let guides = |wiki: bool| d.guides.iter().filter(|g| (g.kind.as_deref() == Some("wiki")) == wiki).map(|g| g.pages.unwrap_or(0)).sum::<u64>();
    let maps = if d.maps.is_empty() { d.contents.get("maps").copied().unwrap_or(0) } else { d.maps.len() as u64 };
    let locations = d.contents.get("locations").copied().unwrap_or_else(|| d.maps.iter().filter_map(|m| m.locations).sum());
    let guide_pages = if d.guides.is_empty() { d.contents.get("guides").copied().unwrap_or(0) } else { guides(false) };
    let stem = file.strip_suffix(&format!(".{}", gamerepo_core::atlaspack::EXTENSION)).unwrap_or(&file).to_string();
    Pack {
        pack_id: p.pack_id.clone(),
        title: d.game_title().map(String::from).unwrap_or(stem),
        source: d.site().unwrap_or_default().to_string(),
        made_in: d.made_in().unwrap_or_default().to_string(),
        sort: d.sort().map(|s| s.word().to_string()).unwrap_or_default(),
        own_things: d.own_things().unwrap_or_default(),
        file,
        size: p.size,
        maps,
        locations,
        guide_pages,
        wiki_pages: guides(true),
        detail: d.detail.clone().unwrap_or_default(),
        updated: d.updated.clone().unwrap_or_default(),
        personal_use: d.personal_use,
        error: p.error.clone().unwrap_or_default(),
    }
}

/// A catalogue game's packs, for its page on a device.
pub fn of_game(ix: &Index, lib: &Library, key: &str) -> Result<Vec<Pack>> {
    let Some(lib_keys) = catalogue::keys_of(ix, key)? else { return Ok(vec![]) };
    // the folder's read only when the record links a pack to one of its keys
    let record = maps::read_record(lib);
    if !record.packs.values().any(|k| k.games.iter().any(|g| lib_keys.contains(&g.key))) {
        return Ok(vec![]);
    }
    let all = maps::linked(ix, lib)?;
    Ok(of_keys(&all, &lib_keys).into_iter().map(info).collect())
}

/// The games' packs added to a plan (D222), those the device left out of a game passed
/// over: each into the place `atlas` under its file's name, once however many of the
/// plan's games it's linked to (`needed_by` the others).
pub fn add(ix: &Index, lib: &Library, hashes: &Hashes, ask: &PlanAsk, keys: &[String], plan: &mut Plan) -> Result<()> {
    let record = maps::read_record(lib);
    if record.packs.values().all(|k| k.games.is_empty()) {
        return Ok(());
    }
    let all = maps::linked(ix, lib)?;
    let roots = roots::list(lib);
    let mut hash = vec![];
    for key in keys {
        let Some(lib_keys) = catalogue::keys_of(ix, key)? else { continue };
        let left: &[String] = ask.packs_left.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice()).unwrap_or(&[]);
        for p in of_keys(&all, &lib_keys).into_iter().filter(|p| !left.contains(&p.pack_id)) {
            if let Some(f) = plan.files.iter_mut().find(|f| f.role == ROLE && f.pack == p.pack_id) {
                if &f.key != key && !f.needed_by.contains(key) {
                    f.needed_by.push(key.clone());
                }
                continue;
            }
            let name = file_name(p);
            let Some(url) = files::url_of(lib, &roots, &p.path) else {
                plan.left.push(Left { what: name, why: "Its file is in a folder the server doesn't serve.".into() });
                continue;
            };
            let sha1 = Hashes::known(ix.conn(), &p.path);
            if sha1.is_none() {
                hash.push(p.path.clone());
            }
            let first = &lib_keys[0];
            let (system, game) = first.split_once('/').unwrap_or(("", first.as_str()));
            plan.files.push(gamerepo_wire::File {
                place: PLACE.into(),
                path: name,
                size: p.size,
                sha1,
                system: system.to_string(),
                game: game.to_string(),
                key: key.clone(),
                role: ROLE.into(),
                pack: p.pack_id.clone(),
                pack_updated: p.description.updated.clone().unwrap_or_default(),
                pack_exported_at: p.description.exported_at.clone().unwrap_or_default(),
                url,
                ..Default::default()
            });
        }
    }
    hashes.want(hash);
    Ok(())
}

// ------------------------------------------------------------ on their own (D225)

/// Where a pack's file is fetched by its `packId`.
pub fn file_url(pack_id: &str) -> String {
    format!("/v1/packs/{}/file", crate::http::encode_path(pack_id))
}

/// A pack as Game Repo Server takes it from the folder (its file, record and description).
fn linked_of(base: &std::path::Path, p: &maps::Pack) -> Linked {
    Linked { pack_id: p.id.clone(), path: base.join(&p.file.rel), size: p.file.size, games: p.kept.games.clone(), description: p.description().clone(), error: None }
}

/// The folder a pack's file is in, below `Maps and Guides/`: its first part (`No game yet`
/// for a file at the top).
fn folder_name(rel: &str) -> String {
    match rel.split_once('/') {
        Some((top, _)) if !top.is_empty() => top.to_string(),
        _ => maps::NO_GAME.to_string(),
    }
}

/// A short mark that changes with a file (its size and time), for a picture's address.
fn mark(p: &maps::Pack) -> String {
    use sha1::Digest;
    let h = sha1::Sha1::digest(format!("{}|{}|{}|{:?}", p.file.rel, p.file.size, p.file.mtime, p.kept.picture).as_bytes());
    hex::encode(&h[..6])
}

/// Whether a pack has a picture: its own cover, else one chosen for it in Game Repo.
fn has_cover(lib: &Library, p: &maps::Pack) -> bool {
    let d = p.description();
    d.cover.as_ref().is_some_and(|c| d.previews().contains(c)) || chosen_picture(lib, p).is_some()
}

/// The picture chosen for a pack in Game Repo (D219), below `Maps and Guides/`, if it's there.
fn chosen_picture(lib: &Library, p: &maps::Pack) -> Option<String> {
    let pic = p.kept.picture.as_deref().filter(|x| gamerepo_core::library::rel_inside(x).is_ok())?;
    maps::folder(lib).join(pic).is_file().then(|| format!("{}/{pic}", maps::FOLDER))
}

/// A Library key's game as a pack's (D225): its catalogue key when the server gives it,
/// and its title and system as the library has them.
fn game_of(ix: &Index, g: &maps::PackGame) -> Result<PackGame> {
    let key = catalogue::game_for(ix, &g.key)?;
    let served = match &key {
        Some(k) => catalogue::game(ix, k)?,
        None => None,
    };
    let title = match &served {
        Some(x) => x.title.clone(),
        None => ix.conn().query_row("SELECT title FROM library WHERE key = ?1", [&g.key], |r| r.get::<_, String>(0)).optional()?.unwrap_or_else(|| g.title.clone()),
    };
    let platform = if g.platform.is_empty() { g.key.split('/').next().unwrap_or("").to_string() } else { g.platform.clone() };
    let system = if platform.is_empty() { String::new() } else if platform == "pc" { "PC".into() } else { gamerepo_core::platforms::name_of(&platform) };
    Ok(PackGame { key: key.unwrap_or_default(), title, system })
}

/// Every pack the library holds that Game Repo can read (D225), linked to a game or not,
/// for Game Repo Go's Maps and guides: what a game's page shows of each, its dates, the
/// games it's linked to, the title it's grouped under (the first game's title in the
/// library, else `game.title`, else its folder's name) and its picture's address, in order
/// of that title and then its own. A file that can't be read (damaged, a newer format) is
/// left out and counted: Go can't tell which pack it is, nor whether it's newer than a copy
/// on the device, so it's never offered on its own (one linked to a game still goes with
/// the game, D222). Another copy of a pack (D216) isn't listed: the newer is the pack.
pub fn library(ix: &Index, lib: &Library) -> Result<PackList> {
    let state = maps::sync(ix, lib)?;
    let base = maps::folder(lib);
    let mut items = vec![];
    for p in &state.packs {
        let d = p.description();
        let games = p.kept.games.iter().map(|g| game_of(ix, g)).collect::<Result<Vec<_>>>()?;
        let group = games
            .first()
            .map(|g| g.title.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| d.game.title.clone().filter(|t| !t.trim().is_empty()))
            .unwrap_or_else(|| folder_name(&p.file.rel));
        let system = games.first().map(|g| g.system.clone()).filter(|s| !s.is_empty()).or_else(|| d.game.system.clone()).unwrap_or_default();
        let cover = if has_cover(lib, p) { format!("/v1/packs/{}/cover?v={}", crate::http::encode_path(&p.id), mark(p)) } else { String::new() };
        items.push(LibraryPack { pack: info(&linked_of(&base, p)), exported_at: d.exported_at.clone().unwrap_or_default(), group, system, games, cover });
    }
    items.sort_by(|a, b| {
        let k = |x: &LibraryPack| (x.group.to_lowercase(), x.pack.title.to_lowercase(), x.pack.pack_id.clone());
        k(a).cmp(&k(b))
    });
    Ok(PackList { items, unreadable: state.unreadable.len() as u64 })
}

/// Whether a request's `packId` could be one (the contract's: 1 to 64 lower case letters,
/// digits, `-` and `_`), before the folder is read for it.
fn could_be_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

/// A pack's file by its `packId` (D225): the one Game Repo reads as the pack (the newer of
/// two copies), only a pack it can read. None: there's none now.
pub fn file_of(ix: &Index, lib: &Library, pack_id: &str) -> Result<Option<PathBuf>> {
    if !could_be_id(pack_id) {
        return Ok(None);
    }
    let state = maps::sync(ix, lib)?;
    Ok(state.pack(pack_id).map(|p| maps::folder(lib).join(&p.file.rel)).filter(|p| p.is_file()))
}

/// A pack's picture (D225) for `GET /v1/packs/<packId>/cover`: its own cover read out of it
/// (`maps::preview_file`, kept in the server's data folder), else the picture chosen for it in
/// Game Repo made `size` pixels at most. None: it has neither.
pub fn cover(ix: &Index, lib: &Library, data: &std::path::Path, pack_id: &str, size: u32) -> Result<Option<PathBuf>> {
    if !could_be_id(pack_id) {
        return Ok(None);
    }
    let state = maps::sync(ix, lib)?;
    let Some(p) = state.pack(pack_id) else { return Ok(None) };
    let d = p.description();
    if d.cover.as_ref().is_some_and(|c| d.previews().contains(c)) {
        let rel = format!("{}/{}", maps::FOLDER, p.file.rel);
        if let Ok(at) = maps::preview_file(lib, data, 0, pack_id, &rel) {
            return Ok(Some(at));
        }
    }
    let Some(rel) = chosen_picture(lib, p) else { return Ok(None) };
    let size = if size <= 128 { 128 } else if size <= 256 { 256 } else { 512 };
    Ok(gamerepo_core::browse::thumb(lib, data, &rel, size).ok())
}

/// Packs asked for on their own (`PlanAsk::pack_ids`, D225), each into the place `atlas`
/// under its file's name with no game's key, fetched by its `packId`; one the library hasn't
/// now (or can't read) is said in `left`.
pub fn add_alone(ix: &Index, lib: &Library, hashes: &Hashes, ids: &[String], plan: &mut Plan) -> Result<()> {
    let state = maps::sync(ix, lib)?;
    let base = maps::folder(lib);
    let mut hash = vec![];
    for id in ids {
        if plan.files.iter().any(|f| f.role == ROLE && &f.pack == id) {
            continue;
        }
        let Some(p) = state.pack(id) else {
            plan.left.push(Left { what: id.clone(), why: "The library has no pack of maps and guides by this id now (it may have gone, or Game Repo can't read it): look at the list again.".into() });
            continue;
        };
        let path = base.join(&p.file.rel);
        let sha1 = Hashes::known(ix.conn(), &path);
        if sha1.is_none() {
            hash.push(path.clone());
        }
        let d = p.description();
        plan.files.push(gamerepo_wire::File {
            place: PLACE.into(),
            path: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            size: p.file.size,
            sha1,
            game: d.game_title().unwrap_or(id).to_string(),
            role: ROLE.into(),
            pack: id.clone(),
            pack_updated: d.updated.clone().unwrap_or_default(),
            pack_exported_at: d.exported_at.clone().unwrap_or_default(),
            url: file_url(id),
            ..Default::default()
        });
    }
    hashes.want(hash);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamerepo_core::atlaspack::Description;
    use serde_json::json;

    /// A made-up pack's description, from `source` and its writer (`pack.writtenBy.name`).
    fn linked(id: &str, source: serde_json::Value, written_by: &str) -> Linked {
        let description = Description::from_value(json!({
            "format": "atlas-pack-file", "version": 5, "packId": id, "title": "Delta", "packVersion": "1.0.0",
            "exportedAt": "2026-11-18T08:05:00Z", "exportedBy": { "name": written_by, "version": "0.23.0", "platform": "linux" },
            "pack": { "id": id, "title": "Delta", "updated": "2026-11-18T08:00:00Z", "writtenBy": { "name": written_by, "version": "0.23.0" } },
            "game": { "title": "Delta" }, "source": source,
        }));
        Linked { pack_id: id.into(), path: format!("Delta/{id}.atlaspack").into(), size: 1, games: vec![], description, error: None }
    }

    /// The sort of a pack as Game Repo Server tells a device, by the contract's rule: a map
    /// made in Atlas Importer is made in Atlas, never "Atlas Importer"; a pack that came with
    /// an app names no site and no app it was made in.
    #[test]
    fn a_device_is_told_the_sort_of_a_pack_by_the_contracts_rule() {
        let said = |p: &Linked| {
            let i = info(p);
            (i.source, i.made_in, i.sort)
        };
        let s = |x: &str| x.to_string();
        assert_eq!(said(&linked("delta-importer-map", json!({ "kind": "unknown" }), "Atlas Importer")), (s(""), s("Atlas"), s("made-in-atlas")));
        assert_eq!(said(&linked("delta-atlas-map", json!({ "kind": "unknown" }), "Atlas")), (s(""), s("Atlas"), s("made-in-atlas")));
        assert_eq!(said(&linked("delta-map-maker", json!({ "kind": "unknown" }), "Atlas Map Maker")), (s(""), s("Atlas Map Maker"), s("map-maker")));
        assert_eq!(said(&linked("delta-app", json!({ "kind": "app", "name": "Delta Companion" }), "Atlas")), (s(""), s(""), s("app")));
        assert_eq!(said(&linked("delta-example-maps", json!({ "kind": "web-pages", "name": "Example Maps" }), "Atlas Importer")), (s("Example Maps"), s(""), s("site")));
        assert_eq!(said(&linked("delta-old", json!({ "importer": { "id": "example-maps", "title": "Example Maps" } }), "Atlas")), (s("Example Maps"), s(""), s("")));
        let j = serde_json::to_value(info(&linked("delta-app", json!({ "kind": "app" }), "Atlas"))).unwrap();
        assert_eq!((j["sort"].as_str(), j.get("made_in")), (Some("app"), None), "{j}");
    }
}
