//! Systems added by hand (docs/PLAN.md, "Phase 3 design", D22): kept in library.json
//! `systems` and taken into the platform table by `platforms::configure`.

use crate::library::Library;
use crate::platforms::{self, Media};
use anyhow::{bail, Result};
use serde_json::{json, Value};

/// A system as Settings › Systems edits it.
pub struct System {
    pub id: String,
    pub name: String,
    pub dat_names: Vec<String>,
    pub folders: Vec<String>,
    pub exts: Vec<String>,
    pub media: String,
}

impl System {
    pub fn from_json(v: &Value) -> Result<System> {
        let list = |k: &str| -> Vec<String> {
            match &v[k] {
                Value::Array(a) => a.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
                // a comma-separated line, as the form sends it
                Value::String(s) => s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect(),
                _ => vec![],
            }
        };
        let name = v["name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() {
            bail!("Give the system a name.");
        }
        let id = v["id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| platforms::id_from_name(&name));
        let mut exts: Vec<String> = list("exts").into_iter().map(|e| e.trim_start_matches('.').to_ascii_lowercase()).collect();
        exts.dedup();
        Ok(System {
            id,
            name,
            // as their base names (no version or "(Parent-Clone)"), so every version is found
            dat_names: list("dat_names").iter().map(|n| platforms::dat_base_name(n)).collect(),
            folders: list("folders"),
            exts,
            media: v["media"].as_str().unwrap_or("cartridge").to_string(),
        })
    }

    fn to_json(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "dat_names": self.dat_names, "folders": self.folders, "exts": self.exts, "media": self.media })
    }
}

/// Add a system, or change one added before (`old` is its id until now, to rename it).
pub fn set(lib: &Library, s: &System, old: Option<&str>) -> Result<Value> {
    lib.writable()?;
    platforms::valid_id(&s.id)?;
    if platforms::PLATFORMS.iter().any(|p| p.id == s.id) {
        bail!("{} is one of the built-in systems ({}): choose another id.", s.id, platforms::name_of(&s.id));
    }
    if Media::from_id(&s.media).is_none() {
        bail!("There's no kind of system called {}.", s.media);
    }
    let meta = lib.meta();
    let mut list: Vec<Value> = meta["systems"].as_array().cloned().unwrap_or_default();
    let replacing = old.unwrap_or(&s.id);
    if s.id != replacing && list.iter().any(|x| x["id"] == s.id.as_str()) {
        bail!("There's a system with the id {} already.", s.id);
    }
    if old.is_some_and(|o| o != s.id) && !lib.root().join(crate::library::APP_DIR).join("dats").join(replacing).read_dir().map(|mut d| d.next().is_none()).unwrap_or(true) {
        bail!("{replacing} has DATs: its id can't change (its name and the rest can).");
    }
    let taken = |n: &String| platforms::from_dat_name(n).filter(|p| p.id != replacing && p.id != s.id);
    if let Some(p) = s.dat_names.iter().find_map(taken) {
        bail!("That DAT name is {}'s already.", p.name);
    }
    match list.iter_mut().find(|x| x["id"] == replacing) {
        Some(x) => *x = s.to_json(),
        None => list.push(s.to_json()),
    }
    lib.update_meta(json!({ "systems": list }))?;
    platforms::configure(&lib.meta());
    Ok(s.to_json())
}

/// Remove a system added by hand (not one whose DATs are still in the library).
pub fn remove(lib: &Library, id: &str) -> Result<()> {
    lib.writable()?;
    let dats = lib.root().join(crate::library::APP_DIR).join("dats").join(id);
    if dats.read_dir().is_ok_and(|mut d| d.next().is_some()) {
        bail!("{} has DATs: remove them first (Settings › DAT files).", platforms::name_of(id));
    }
    let meta = lib.meta();
    let list: Vec<Value> = meta["systems"].as_array().cloned().unwrap_or_default().into_iter().filter(|x| x["id"] != id).collect();
    // and the DATs it was given before (so it isn't shown by its id again)
    let given: serde_json::Map<String, Value> = meta["dat_platforms"].as_object().cloned().unwrap_or_default().into_iter().filter(|(_, v)| v != id).collect();
    lib.update_meta(json!({ "systems": list, "dat_platforms": given }))?;
    platforms::configure(&lib.meta());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    #[test]
    fn systems_added_by_hand_are_found_like_built_in_ones() {
        let _turn = platforms::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = temp_dir("systems");
        let lib = Library::open(&dir).unwrap();
        let s = System::from_json(&json!({ "name": "Acme Gizmo", "dat_names": "Acme - Gizmo (Parent-Clone) (20261001), Acme - Gizmo Plus", "folders": ["Gizmo"], "exts": ".GZM", "media": "cd" })).unwrap();
        assert_eq!(s.id, "acmegizmo");
        assert_eq!(s.dat_names, ["Acme - Gizmo", "Acme - Gizmo Plus"]);
        set(&lib, &s, None).unwrap();
        assert_eq!(platforms::from_dat_name("Acme - Gizmo (20261001)").map(|p| p.id), Some("acmegizmo"));
        assert_eq!(platforms::from_folder("gizmo").map(|p| p.id), Some("acmegizmo"));
        assert_eq!(platforms::from_ext("gzm").map(|p| p.id), Some("acmegizmo"));
        assert_eq!(platforms::formats("acmegizmo"), ["chd"]);
        assert!(platforms::is_custom("acmegizmo") && !platforms::is_custom("gb"));
        assert!(set(&lib, &System::from_json(&json!({ "id": "gb", "name": "Mine" })).unwrap(), None).is_err(), "a built-in id");
        assert!(set(&lib, &System::from_json(&json!({ "name": "Other", "dat_names": ["Acme - Gizmo"] })).unwrap(), None).is_err(), "a DAT name taken");
        // renamed (its id too, as it has no DATs), then removed
        let mut s2 = System::from_json(&json!({ "id": "gizmo", "name": "Gizmo", "dat_names": ["Acme - Gizmo"], "media": "cartridge" })).unwrap();
        set(&lib, &s2, Some("acmegizmo")).unwrap();
        assert!(platforms::by_id("acmegizmo").is_none());
        assert_eq!(platforms::formats("gizmo"), ["zip"]);
        // an id a DAT was given before shows as a system until it's described
        lib.update_meta(json!({ "dat_platforms": { "Nintendo - Satellite": "sat" } })).unwrap();
        platforms::configure(&lib.meta());
        assert_eq!(platforms::name_of("sat"), "sat");
        s2.name = "Gizmo Two".into();
        set(&lib, &s2, None).unwrap();
        assert_eq!(platforms::name_of("gizmo"), "Gizmo Two");
        remove(&lib, "gizmo").unwrap();
        remove(&lib, "sat").unwrap();
        assert!(platforms::by_id("gizmo").is_none() && platforms::by_id("sat").is_none());
        platforms::configure(&json!({}));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
