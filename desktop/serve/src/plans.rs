//! A device's plan (D93): the files to write for some games, laid out by the export's own
//! engine (`export::lay_out`) with ES-DE's places, each with where it's fetched from; the
//! files to be made are made in the background, so the plan answers at once.

use crate::made::{Hashes, Made, Maker};
use crate::{catalogue, files};
use anyhow::Result;
use gamerepo_core::export::{self, Carry, DeviceAsk};
use gamerepo_core::index::Index;
use gamerepo_core::library::Library;
use gamerepo_core::{meta, organise::Op, roots};
use gamerepo_wire::api::{KindSize, PlanAsk};
use gamerepo_wire::Plan;
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

/// The plan for `ask`. Err(why) when a game asked for isn't in the catalogue.
pub fn plan(ix: &Index, lib: &Library, data: &Path, maker: &Maker, hashes: &Hashes, ask: &PlanAsk) -> Result<std::result::Result<Plan, String>> {
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
    let mut ready = true;
    let mut hash = vec![];
    let mut left = vec![];
    for (f, op) in plan.files.iter_mut().zip(&laid.ops) {
        // the catalogue game each file is for, or the games needing a shared set
        if !matches!(f.role.as_str(), "bios" | "device" | "bios-file") {
            f.key = catalogue::game_for(ix, &format!("{}/{}", f.system, f.game))?.unwrap_or_default();
        }
        let mut by = vec![];
        for g in &f.needed_by {
            by.extend(catalogue::game_for(ix, &format!("{}/{g}", f.system))?);
        }
        f.needed_by = by;
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
    for l in &mut plan.lists {
        for e in &mut l.entries {
            e.key = catalogue::game_for(ix, &format!("{}/{}", l.system, e.game))?.unwrap_or_default();
        }
    }
    // each media kind's size, those not asked for too (C13), from a plan carrying every kind
    let mut kinds: BTreeMap<String, KindSize> = BTreeMap::new();
    let all_media: Vec<String> = meta::MEDIA.iter().map(|(k, _, _)| k.to_string()).collect();
    let whole = if all_media.iter().all(|k| ask.media.contains(k)) && ask.manuals { None } else { Some(export::lay_out(ix, lib, &device(all_media.clone(), true), &scratch(data))?) };
    let (files, ops): (&[gamerepo_wire::File], &[Op]) = match &whole {
        Some(w) => (&w.plan.files, &w.ops),
        None => (&plan.files, &laid.ops),
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
