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

use crate::catalogue;
use crate::files;
use crate::made::Hashes;
use anyhow::Result;
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::maps::{self, Linked};
use gamerepo_core::roots;
use gamerepo_wire::api::{Pack, PlanAsk};
use gamerepo_wire::{Left, Plan};
use std::collections::HashSet;

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
