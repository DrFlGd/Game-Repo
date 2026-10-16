//! The platforms Game Repo knows (docs/platforms.md is the readable copy of this
//! table; the two are kept in step): their ids (ES-DE's), the names DAT groups and
//! folders use for them, their extensions, and how a DAT or a file finds its platform.

use serde_json::{json, Value};

pub struct Platform {
    /// ES-DE's id, the app's id for the platform and its folder name on export
    pub id: &'static str,
    pub name: &'static str,
    /// The DAT groups' names for it (No-Intro's first)
    pub dat_names: &'static [&'static str],
    /// Folder names that mean this platform (compared without case, spaces or punctuation)
    pub folders: &'static [&'static str],
    /// Extensions its files have; those only it uses decide a file's platform on their own
    pub exts: &'static [&'static str],
}

macro_rules! p {
    ($id:literal, $name:literal, [$($dat:literal),*], [$($f:literal),*], [$($e:literal),*]) => {
        Platform { id: $id, name: $name, dat_names: &[$($dat),*], folders: &[$($f),*], exts: &[$($e),*] }
    };
}

/// The cartridge systems of Phase 1 (disc systems join in Phase 2, arcade in Phase 7).
pub static PLATFORMS: &[Platform] = &[
    p!("nes", "Nintendo Entertainment System", ["Nintendo - Nintendo Entertainment System"], ["NES", "Famicom", "FC", "Nintendo Entertainment System"], ["nes", "unf", "unif"]),
    p!("fds", "Famicom Disk System", ["Nintendo - Family Computer Disk System"], ["FDS", "Famicom Disk System", "Family Computer Disk System"], ["fds"]),
    p!("snes", "Super Nintendo", ["Nintendo - Super Nintendo Entertainment System"], ["SNES", "Super Nintendo", "Super Famicom", "SFC", "Super Nintendo Entertainment System"], ["sfc", "smc", "swc", "fig"]),
    p!("n64", "Nintendo 64", ["Nintendo - Nintendo 64"], ["N64", "Nintendo 64"], ["z64", "v64", "n64"]),
    p!("gb", "Game Boy", ["Nintendo - Game Boy"], ["GB", "Game Boy", "Gameboy"], ["gb"]),
    p!("gbc", "Game Boy Color", ["Nintendo - Game Boy Color"], ["GBC", "Game Boy Color", "Gameboy Color"], ["gbc"]),
    p!("gba", "Game Boy Advance", ["Nintendo - Game Boy Advance"], ["GBA", "Game Boy Advance", "Gameboy Advance"], ["gba"]),
    p!("nds", "Nintendo DS", ["Nintendo - Nintendo DS"], ["NDS", "DS", "Nintendo DS"], ["nds"]),
    p!("virtualboy", "Virtual Boy", ["Nintendo - Virtual Boy"], ["Virtual Boy", "VB"], ["vb"]),
    p!("pokemini", "Pokémon Mini", ["Nintendo - Pokemon Mini"], ["Pokemon Mini", "Pokémon Mini"], ["min"]),
    p!("genesis", "Sega Genesis / Mega Drive", ["Sega - Mega Drive - Genesis"], ["Genesis", "Mega Drive", "Megadrive", "MD", "Sega Genesis", "Sega Mega Drive"], ["md", "gen", "bin", "smd"]),
    p!("mastersystem", "Sega Master System", ["Sega - Master System - Mark III"], ["Master System", "SMS", "Mark III", "Sega Master System"], ["sms"]),
    p!("gamegear", "Sega Game Gear", ["Sega - Game Gear"], ["Game Gear", "GG", "Sega Game Gear"], ["gg"]),
    p!("sega32x", "Sega 32X", ["Sega - 32X"], ["32X", "Sega 32X"], ["32x"]),
    p!("sg-1000", "Sega SG-1000", ["Sega - SG-1000"], ["SG-1000", "SG1000"], ["sg"]),
    p!("atari2600", "Atari 2600", ["Atari - 2600", "Atari - Atari 2600"], ["Atari 2600", "2600", "VCS"], ["a26", "bin"]),
    p!("atari5200", "Atari 5200", ["Atari - 5200", "Atari - Atari 5200"], ["Atari 5200", "5200"], ["a52", "bin"]),
    p!("atari7800", "Atari 7800", ["Atari - 7800", "Atari - Atari 7800"], ["Atari 7800", "7800"], ["a78", "bin"]),
    p!("atarilynx", "Atari Lynx", ["Atari - Lynx", "Atari - Atari Lynx"], ["Lynx", "Atari Lynx"], ["lnx", "lyx"]),
    p!("atarijaguar", "Atari Jaguar", ["Atari - Jaguar", "Atari - Atari Jaguar"], ["Jaguar", "Atari Jaguar"], ["j64", "jag"]),
    p!("pcengine", "PC Engine / TurboGrafx-16", ["NEC - PC Engine - TurboGrafx-16", "NEC - PC Engine - TurboGrafx 16"], ["PC Engine", "TurboGrafx-16", "TurboGrafx 16", "TG16", "PCE"], ["pce"]),
    p!("ngp", "Neo Geo Pocket", ["SNK - Neo Geo Pocket", "SNK - NeoGeo Pocket"], ["Neo Geo Pocket", "NGP"], ["ngp"]),
    p!("ngpc", "Neo Geo Pocket Color", ["SNK - Neo Geo Pocket Color", "SNK - NeoGeo Pocket Color"], ["Neo Geo Pocket Color", "NGPC"], ["ngc"]),
    p!("wonderswan", "WonderSwan", ["Bandai - WonderSwan"], ["WonderSwan", "WS"], ["ws"]),
    p!("wonderswancolor", "WonderSwan Color", ["Bandai - WonderSwan Color"], ["WonderSwan Color", "WSC"], ["wsc"]),
    p!("colecovision", "ColecoVision", ["Coleco - ColecoVision"], ["ColecoVision", "Coleco"], ["col"]),
    p!("intellivision", "Intellivision", ["Mattel - Intellivision"], ["Intellivision"], ["int"]),
    p!("vectrex", "Vectrex", ["GCE - Vectrex"], ["Vectrex"], ["vec"]),
];

/// Bracketed words at the end of a DAT's name that say how it was made, not what it's for.
const DAT_SUFFIXES: &[&str] = &[
    "headered", "headerless", "parent-clone", "decrypted", "encrypted", "bigendian",
    "byteswapped", "a78", "lnx", "lyx", "j64", "rom", "private", "no-intro", "retool",
];

pub fn by_id(id: &str) -> Option<&'static Platform> {
    PLATFORMS.iter().find(|p| p.id == id)
}

/// The platform's name, or its id for one this build doesn't know.
pub fn name_of(id: &str) -> String {
    by_id(id).map(|p| p.name.to_string()).unwrap_or_else(|| id.to_string())
}

/// Lower case, letters and digits only: "Super Nintendo" and "super_nintendo" are the same.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .map(|c| if c == 'é' { 'e' } else { c })
        .collect()
}

/// A DAT's name without the group's suffixes: "Nintendo - Game Boy (Parent-Clone) (20261001)"
/// -> "Nintendo - Game Boy".
pub fn dat_base_name(name: &str) -> String {
    let mut s = name.trim().to_string();
    while s.ends_with(')') || s.ends_with(']') {
        let open = if s.ends_with(')') { '(' } else { '[' };
        let Some(i) = s.rfind(open) else { break };
        let inner = s[i + 1..s.len() - 1].trim().to_ascii_lowercase();
        let is_date = inner.len() >= 8
            && inner.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '.' || c == ' ');
        if is_date || DAT_SUFFIXES.contains(&inner.as_str()) {
            s = s[..i].trim_end().to_string();
        } else {
            break;
        }
    }
    s
}

/// The platform a DAT is for, from its header name.
pub fn from_dat_name(name: &str) -> Option<&'static Platform> {
    let base = squash(&dat_base_name(name));
    PLATFORMS
        .iter()
        .find(|p| p.dat_names.iter().any(|d| squash(d) == base))
}

/// The platform a folder's name means: one of its folder names, its id, its name or a DAT name.
pub fn from_folder(name: &str) -> Option<&'static Platform> {
    let n = squash(name);
    if n.is_empty() {
        return None;
    }
    PLATFORMS.iter().find(|p| {
        squash(p.id) == n
            || squash(p.name) == n
            || p.folders.iter().any(|f| squash(f) == n)
            || p.dat_names.iter().any(|d| squash(d) == n)
    })
}

/// The platform an extension belongs to, when only one platform uses it.
pub fn from_ext(ext: &str) -> Option<&'static Platform> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let mut found = PLATFORMS.iter().filter(|p| p.exts.contains(&ext.as_str()));
    let first = found.next()?;
    found.next().is_none().then_some(first)
}

/// A file's lower-case extension ("" when it has none).
pub fn ext_of(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 => base[i + 1..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The platform of a file no DAT recognised: the hint set on its folder with games, a
/// folder on its path, then its extension (docs/platforms.md).
pub fn guess(hint: Option<&str>, dirs: &[&str], name: &str) -> Option<String> {
    if let Some(h) = hint.filter(|h| !h.is_empty()) {
        return Some(h.to_string());
    }
    if let Some(p) = dirs.iter().rev().find_map(|d| from_folder(d)) {
        return Some(p.id.to_string());
    }
    from_ext(&ext_of(name)).map(|p| p.id.to_string())
}

/// The table, as the page's platform pickers list it.
pub fn list() -> Value {
    json!(PLATFORMS
        .iter()
        .map(|p| json!({ "id": p.id, "name": p.name, "dat": p.dat_names[0], "exts": p.exts }))
        .collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dats_find_their_platform() {
        let id = |n: &str| from_dat_name(n).map(|p| p.id);
        assert_eq!(id("Nintendo - Super Nintendo Entertainment System"), Some("snes"));
        assert_eq!(id("Nintendo - Nintendo Entertainment System (Headered) (Parent-Clone)"), Some("nes"));
        assert_eq!(id("Nintendo - Game Boy (20261001-123456)"), Some("gb"));
        assert_eq!(id("Atari - 7800 (A78)"), Some("atari7800"));
        assert_eq!(id("Sega - Mega Drive - Genesis"), Some("genesis"));
        assert_eq!(id("Nintendo - Satellaview"), None);
        assert_eq!(dat_base_name("Nintendo - Game Boy (Parent-Clone) (2026-10-01)"), "Nintendo - Game Boy");
    }

    #[test]
    fn files_find_their_platform() {
        assert_eq!(from_folder("super_nintendo").unwrap().id, "snes");
        assert_eq!(from_folder("Nintendo - Game Boy Advance").unwrap().id, "gba");
        assert_eq!(from_folder("gbc").unwrap().id, "gbc");
        assert!(from_folder("Roms").is_none());
        assert_eq!(from_ext(".SFC").unwrap().id, "snes");
        assert!(from_ext("bin").is_none(), ".bin belongs to several platforms");
        assert_eq!(guess(None, &["Roms", "SNES", "Hacks"], "x.bin").as_deref(), Some("snes"));
        assert_eq!(guess(Some("gb"), &["SNES"], "x.sfc").as_deref(), Some("gb"));
        assert_eq!(guess(None, &["Roms"], "Tetris.gb").as_deref(), Some("gb"));
        assert_eq!(guess(None, &["Roms"], "readme.txt"), None);
        assert_eq!(ext_of("A.B/Game (USA).GBA"), "gba");
        assert_eq!(ext_of(".hidden"), "");
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = PLATFORMS.iter().map(|p| p.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), PLATFORMS.len());
    }
}
