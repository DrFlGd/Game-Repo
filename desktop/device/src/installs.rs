//! Where ES-DE is set up on a device (docs/PLAN.md, "Phase 9e design", D116): by itself
//! (`~/ES-DE`), by EmuDeck (`~/ES-DE` on Linux, a portable ES-DE of its own on Windows) or
//! by RetroDECK (its Flatpak's config folder, Linux only). Each is opened as ES-DE opens it,
//! so its ROMs and media folders are the ones its own settings name.

use crate::Device;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// ES-DE as one of them set it up.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Install {
    /// ES-DE, EmuDeck or RetroDECK
    pub by: String,
    pub esde: PathBuf,
    pub roms: PathBuf,
    pub media: PathBuf,
}

/// Where each could have put ES-DE's folder under `home` (the user's folder).
pub fn places(home: &Path) -> Vec<(&'static str, PathBuf)> {
    let emudeck = home.join(".config/EmuDeck/settings.sh").is_file() || home.join("emudeck/settings.sh").is_file();
    let mut v = vec![(if emudeck && !cfg!(windows) { "EmuDeck" } else { "ES-DE" }, home.join("ES-DE"))];
    let rd = home.join(".var/app/net.retrodeck.retrodeck/config");
    v.push(("RetroDECK", rd.join("ES-DE")));
    v.push(("RetroDECK", rd.join("emulationstation/.emulationstation")));
    if cfg!(windows) {
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"));
        v.push(("EmuDeck", appdata.join("EmuDeck").join("EmulationStation-DE").join("ES-DE")));
    }
    v
}

/// Every ES-DE set up under these folders, the same folder found twice (a link) once.
pub fn find(homes: &[PathBuf]) -> Vec<(Install, Device)> {
    let mut out: Vec<(Install, Device)> = vec![];
    let mut seen: Vec<PathBuf> = vec![];
    for home in homes {
        for (by, esde) in places(home) {
            let Ok(d) = Device::open_esde(&esde, home) else { continue };
            let real = std::fs::canonicalize(&esde).unwrap_or_else(|_| esde.clone());
            if seen.contains(&real) {
                continue;
            }
            seen.push(real);
            out.push((Install { by: by.into(), esde, roms: d.places["roms"].clone(), media: d.places["media"].clone() }, d));
        }
    }
    out
}
