//! A magazine's issues for Atlas on devices (docs/PLAN.md, "Maps and Guides notes, part 12",
//! D246–D248): each magazine the library keeps travels in a pack of its own (Atlas 0.29.0's
//! format 10), made by Game Repo Server with Atlas's crate (`gamerepo_core::magpack`, cached in
//! the server's data folder under `magazines/`) when a device asks for it, and kept, so it
//! isn't made twice.
//!
//! `GET /v1/packs` lists every magazine's pack under the group *Magazines* (sort `magazines`,
//! with its issues), so Game Repo Go's *Maps and guides* page offers each to install on its
//! own (D225); a pack is of no game, so a game's page and plan never carry one. A plan asked
//! for by its packId (`PlanAsk::pack_ids`) gives it as a pack file (role `pack`, place
//! `atlas`): not made yet, it's made in the background, the plan isn't `ready`, gives a size
//! reckoned from the issues and no SHA-1, and its address answers 202 until it's made. A
//! magazine changed since its pack was last made is made again when the list is read, so a
//! device that keeps it on its own takes the newer one after its next read of the library.

use crate::guidebooks::{maker_in, Maker, Status};
use anyhow::Result;
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::magpack::{self, Wanted};
use gamerepo_wire::api::{LibraryPack, Pack};
use gamerepo_wire::{Left, Plan};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The group a magazine's pack is listed under on Game Repo Go's Maps and guides.
pub const GROUP: &str = "Magazines";

/// The magazines' maker for a data folder: its packs made into `<data>/magazines/`.
pub fn maker(data: &Path) -> Arc<Maker> {
    maker_in(data, magpack::DIR)
}

/// The pack for `w`: ready (made, and the newest of its magazine's), else asked for.
pub fn status(maker: &Maker, w: &Wanted) -> Status {
    let w = w.clone();
    maker.status_of(&w.pack_id, &w.recipe(), &|| {
        let w = w.clone();
        Arc::new(move |cache, cancel: &AtomicBool| magpack::make(cache, &w, cancel))
    })
}

/// Where a pack of a recipe is fetched.
fn made_url(pack_id: &str, id: &str) -> String {
    format!("/v1/packs/{}/made/{id}", crate::http::encode_path(pack_id))
}

/// What a device's page shows of a magazine's pack: as a pack (sort `magazines`), with its
/// issues, and its size and pages the made one's when it's made for the magazine as it is,
/// else its size reckoned.
pub fn info(maker: &Maker, w: &Wanted) -> Pack {
    let made = maker.cache.current_of(&w.pack_id, &w.recipe());
    Pack {
        pack_id: w.pack_id.clone(),
        title: w.title.clone(),
        sort: gamerepo_core::atlaspack::Sort::Magazines.word().into(),
        file: w.file_name(),
        size: made.as_ref().map_or_else(|| w.estimate(), |m| m.size),
        issues: made.as_ref().map_or(w.items.len(), |m| m.items) as u64,
        guide_pages: made.as_ref().map_or(0, |m| m.pages),
        updated: made.map(|m| m.updated).unwrap_or_default(),
        ..Default::default()
    }
}

/// Every magazine's pack that has an issue that can go, for `GET /v1/packs` (the group
/// *Magazines*). One made before for the magazine as it was is made again now (D248).
pub fn library(ix: &Index, lib: &Library, data: &Path) -> Result<Vec<LibraryPack>> {
    let all = magpack::all(ix, lib)?;
    if all.is_empty() {
        return Ok(vec![]);
    }
    let maker = maker(data);
    let mut out = vec![];
    for w in all.iter().filter(|w| !w.items.is_empty()) {
        let made = maker.cache.current_of(&w.pack_id, &w.recipe());
        if made.is_none() && maker.cache.newest(&w.pack_id).is_some() {
            let _ = status(&maker, w);
        }
        out.push(LibraryPack {
            pack: info(&maker, w),
            exported_at: made.map(|m| m.exported_at).unwrap_or_default(),
            group: GROUP.into(),
            ..Default::default()
        });
    }
    Ok(out)
}

/// A plan's file for a magazine's pack: made (its size, SHA-1 and dates, and the issues left
/// out as it was made), or being made (its size reckoned, no SHA-1, the plan not ready).
fn file_of(maker: &Maker, w: &Wanted, plan: &mut Plan) -> gamerepo_wire::File {
    let mut f = gamerepo_wire::File { place: crate::packs::PLACE.into(), path: w.file_name(), role: crate::packs::ROLE.into(), pack: w.pack_id.clone(), url: made_url(&w.pack_id, &w.recipe()), game: w.title.clone(), ..Default::default() };
    let mut left = w.left.clone();
    match status(maker, w) {
        Status::Ready(_, m) => {
            f.size = m.size;
            f.sha1 = Some(m.sha1);
            f.pack_updated = m.updated;
            f.pack_exported_at = m.exported_at;
            left = m.left;
        }
        Status::Failed(e) => {
            plan.left.push(Left { what: f.path.clone(), why: format!("Its pack couldn't be made: {e}") });
            f.url.clear();
        }
        _ => {
            f.size = w.estimate();
            plan.ready = false;
        }
    }
    for (title, why) in left {
        plan.left.push(Left { what: format!("{} {title}", w.title), why });
    }
    f
}

/// Magazines' packs asked for on their own (D225's Install), by their ids.
pub fn add_alone(ix: &Index, lib: &Library, data: &Path, ids: &[String], plan: &mut Plan) -> Result<()> {
    let ids: Vec<&String> = ids.iter().filter(|i| magpack::is_pack_id(i)).collect();
    if ids.is_empty() {
        return Ok(());
    }
    let maker = maker(data);
    let all = magpack::all(ix, lib)?;
    for id in ids {
        if plan.files.iter().any(|f| f.role == crate::packs::ROLE && &f.pack == id) {
            continue;
        }
        // none: the library's packs are looked for by it next, and say why when there's none
        let Some(w) = all.iter().find(|w| &w.pack_id == id && !w.items.is_empty()) else { continue };
        let file = file_of(&maker, w, plan);
        if !file.url.is_empty() {
            plan.files.push(file);
        }
    }
    Ok(())
}

/// `GET /v1/packs/<packId>/file` for a magazine's pack: None when the library has no magazine
/// of that id (a pack of maps and guides of that id is then looked for).
pub fn whole(ix: &Index, lib: &Library, data: &Path, pack_id: &str) -> Result<Option<Status>> {
    Ok(magpack::by_id(ix, lib, pack_id)?.map(|w| status(&maker(data), &w)))
}

/// `GET /v1/packs/<packId>/made/<recipe>`: the pack of a magazine as a plan gave it.
pub fn made(data: &Path, pack_id: &str, id: &str) -> Status {
    if !magpack::is_pack_id(pack_id) || id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Status::Unknown;
    }
    maker(data).file(pack_id, id)
}
