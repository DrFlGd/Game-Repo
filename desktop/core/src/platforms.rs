//! The platforms Game Repo knows (docs/platforms.md is the readable copy of this
//! table; the two are kept in step): their ids (ES-DE's), the names DAT groups and
//! folders use for them, their extensions, and how a DAT or a file finds its platform.
//! Systems added in Settings (library.json `systems`, D22) join the table when the
//! library opens (`configure`).

use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::RwLock;

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

/// The cartridge systems of Phase 1, then the disc systems of Phase 2 (Redump's DAT
/// names first, then libretro's copies'), then arcade (Phase 7, D57: their DATs are found by
/// [`from_dat_name`]'s prefixes as well, as each MAME release names its own).
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
    p!("psx", "PlayStation", ["Sony - PlayStation"], ["PSX", "PS1", "PlayStation", "PlayStation 1", "Sony PlayStation"], ["cue", "bin", "img", "chd", "m3u"]),
    p!("ps2", "PlayStation 2", ["Sony - PlayStation 2"], ["PS2", "PlayStation 2", "Sony PlayStation 2"], ["iso", "cue", "bin", "chd"]),
    p!("psp", "PlayStation Portable", ["Sony - PlayStation Portable"], ["PSP", "PlayStation Portable", "Sony PlayStation Portable"], ["iso", "cso", "zso", "chd"]),
    p!("saturn", "Sega Saturn", ["Sega - Saturn"], ["Saturn", "Sega Saturn"], ["cue", "bin", "iso", "chd"]),
    p!("segacd", "Sega CD / Mega-CD", ["Sega - Mega CD & Sega CD", "Sega - Mega-CD - Sega CD"], ["Sega CD", "Mega CD", "Mega-CD", "SegaCD", "MegaCD"], ["cue", "bin", "iso", "chd"]),
    p!("dreamcast", "Sega Dreamcast", ["Sega - Dreamcast"], ["Dreamcast", "DC", "Sega Dreamcast"], ["gdi", "cdi", "cue", "bin", "chd"]),
    p!("pcenginecd", "PC Engine CD / TurboGrafx-CD", ["NEC - PC Engine CD & TurboGrafx CD", "NEC - PC Engine CD - TurboGrafx-CD"], ["PC Engine CD", "TurboGrafx-CD", "TurboGrafx CD", "PCE-CD", "TG-CD"], ["cue", "bin", "chd"]),
    p!("neogeocd", "Neo Geo CD", ["SNK - Neo Geo CD"], ["Neo Geo CD", "NeoGeo CD", "NGCD"], ["cue", "bin", "chd"]),
    p!("3do", "3DO", ["Panasonic - 3DO Interactive Multiplayer", "The 3DO Company - 3DO"], ["3DO", "Panasonic 3DO"], ["cue", "bin", "iso", "chd"]),
    p!("pcfx", "PC-FX", ["NEC - PC-FX & PC-FXGA", "NEC - PC-FX"], ["PC-FX", "PCFX"], ["cue", "bin", "chd"]),
    p!("atarijaguarcd", "Atari Jaguar CD", ["Atari - Jaguar CD Interactive Multimedia System", "Atari - Jaguar CD"], ["Jaguar CD", "Atari Jaguar CD"], ["cue", "bin", "chd"]),
    p!("cdimono1", "Philips CD-i", ["Philips - CD-i"], ["CD-i", "CDi", "Philips CD-i"], ["cue", "bin", "iso", "chd"]),
    p!("gc", "Nintendo GameCube", ["Nintendo - GameCube"], ["GameCube", "GC", "NGC", "Nintendo GameCube"], ["iso", "gcm", "rvz", "gcz", "ciso"]),
    p!("wii", "Nintendo Wii", ["Nintendo - Wii"], ["Wii", "Nintendo Wii"], ["iso", "wbfs", "rvz", "wia", "gcz"]),
    p!("xbox", "Xbox", ["Microsoft - Xbox"], ["Xbox", "Microsoft Xbox"], ["iso", "xiso"]),
    // Phase 8b (D66): the systems whose updates and DLC come as files of their own,
    // audited only, like Xbox
    p!("n3ds", "Nintendo 3DS", ["Nintendo - Nintendo 3DS"], ["3DS", "N3DS", "Nintendo 3DS"], ["3ds", "cia", "cci", "cxi"]),
    p!("wiiu", "Nintendo Wii U", ["Nintendo - Wii U"], ["Wii U", "WiiU", "Nintendo Wii U"], ["wua", "wux", "wud", "rpx"]),
    p!("switch", "Nintendo Switch", ["Nintendo - Nintendo Switch"], ["Switch", "NSW", "Nintendo Switch"], ["nsp", "xci", "nsz", "xcz"]),
    p!("ps3", "PlayStation 3", ["Sony - PlayStation 3"], ["PS3", "PlayStation 3", "Sony PlayStation 3"], ["iso", "pkg"]),
    p!("ps4", "PlayStation 4", ["Sony - PlayStation 4"], ["PS4", "PlayStation 4", "Sony PlayStation 4"], ["pkg"]),
    p!("psvita", "PlayStation Vita", ["Sony - PlayStation Vita"], ["PS Vita", "PSVita", "Vita", "PlayStation Vita", "Sony PlayStation Vita"], ["vpk", "pkg"]),
    p!("xbox360", "Xbox 360", ["Microsoft - Xbox 360"], ["Xbox 360", "X360", "Microsoft Xbox 360"], ["iso", "xex"]),
    p!("mame", "Arcade (MAME)", ["MAME"], ["MAME", "Arcade"], []),
    p!("fbneo", "Arcade (FinalBurn Neo)", ["FinalBurn Neo - Arcade Games", "FBNeo - Arcade Games"], ["FBNeo", "FinalBurn Neo", "FBA", "FinalBurn Alpha"], []),
    // not a system of games: each system's BIOS files, from a BIOS DAT (bios.rs, D40)
    p!("bios", "BIOS files", ["System"], ["BIOS", "Firmware"], []),
];

/// How a platform's games are stored, which decides what they can be converted to (D12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Media {
    Cartridge,
    /// CD systems (and PS2, which has CDs and DVDs): CHD
    Cd,
    /// GameCube and Wii: RVZ
    Nintendo,
    /// PSP: CSO (or ZSO, or CHD)
    Umd,
    /// Audited, not converted
    AuditOnly,
    /// MAME and FinalBurn Neo: a ZIP per machine, kept as a set type says (arcade.rs)
    Arcade,
}

impl Media {
    pub fn id(self) -> &'static str {
        match self {
            Media::Cartridge => "cartridge",
            Media::Cd => "cd",
            Media::Nintendo => "nintendo",
            Media::Umd => "umd",
            Media::AuditOnly => "audit",
            Media::Arcade => "arcade",
        }
    }

    pub fn from_id(s: &str) -> Option<Media> {
        [Media::Cartridge, Media::Cd, Media::Nintendo, Media::Umd, Media::AuditOnly, Media::Arcade].into_iter().find(|m| m.id() == s)
    }
}

pub fn media(id: &str) -> Media {
    if let Some(m) = custom().iter().find(|c| c.0.id == id).map(|c| c.1) {
        return m;
    }
    match id {
        "psx" | "ps2" | "saturn" | "segacd" | "dreamcast" | "pcenginecd" | "neogeocd" | "3do" | "pcfx" | "atarijaguarcd" | "cdimono1" => Media::Cd,
        "gc" | "wii" => Media::Nintendo,
        "psp" => Media::Umd,
        "xbox" | "n3ds" | "wiiu" | "switch" | "ps3" | "ps4" | "psvita" | "xbox360" => Media::AuditOnly,
        "mame" | "fbneo" => Media::Arcade,
        _ => Media::Cartridge,
    }
}

pub fn is_disc(id: &str) -> bool {
    !matches!(media(id), Media::Cartridge | Media::Arcade)
}

/// A system whose games on several discs ES-DE shows as one, from a folder `<name>.m3u`
/// holding the discs and an `.m3u` playlist of them (D94): those whose ES-DE system
/// takes `.m3u` files, on Windows and Android alike (ES-DE's es_systems.xml).
pub fn m3u_folders(id: &str) -> bool {
    matches!(id, "psx" | "ps2" | "saturn" | "segacd" | "dreamcast" | "pcenginecd" | "pcfx" | "gc" | "wii")
}

/// A disc of a game on several, by its DAT name (Redump's `(Disc 1)`, `(Disc 2 of 3)`,
/// `(Disc A)`): the name without that tag, and which disc it is ("1", "A"). None: a game
/// on one disc.
pub fn disc_of(name: &str) -> Option<(String, String)> {
    let at = name.find(" (Disc ")?;
    let rest = &name[at + " (Disc ".len()..];
    let end = rest.find(')')?;
    let which = rest[..end].split(" of ").next().unwrap_or("").trim();
    if which.is_empty() || !which.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some((format!("{}{}", &name[..at], &rest[end + 1..]), which.to_string()))
}

/// An arcade system (Phase 7): machines, BIOS sets, devices, disks and set types.
pub fn is_arcade(id: &str) -> bool {
    media(id) == Media::Arcade
}

/// The formats a platform's games can be converted to, its preferred one (D12) first.
pub fn formats(id: &str) -> &'static [&'static str] {
    match media(id) {
        Media::Cartridge => &["zip"],
        Media::Cd => &["chd"],
        Media::Nintendo => &["rvz"],
        Media::Umd => &["cso", "zso", "chd"],
        Media::AuditOnly | Media::Arcade => &[],
    }
}

/// Bracketed words at the end of a DAT's name that say how it was made, not what it's for.
const DAT_SUFFIXES: &[&str] = &[
    "headered", "headerless", "parent-clone", "decrypted", "encrypted", "bigendian",
    "byteswapped", "a78", "lnx", "lyx", "j64", "rom", "private", "no-intro", "retool",
    // and how a system's DATs are split (D66): its games, and its updates and DLC
    "digital", "cdn", "psn", "content", "standard", "updates and dlc", "updates", "update", "dlc", "patches",
];

// ------------------------------------------------------------ systems added by hand (D22)

/// The systems added in this library (and ids DATs were given that aren't described yet).
static CUSTOM: RwLock<Vec<(&'static Platform, Media)>> = RwLock::new(Vec::new());

fn custom() -> Vec<(&'static Platform, Media)> {
    CUSTOM.read().map(|c| c.clone()).unwrap_or_default()
}

/// Every platform: the built-in table, then the library's own.
pub fn all() -> impl Iterator<Item = &'static Platform> {
    PLATFORMS.iter().chain(custom().into_iter().map(|c| c.0))
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_list(v: &Value) -> &'static [&'static str] {
    let items: Vec<&'static str> = v.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(leak).collect();
    Box::leak(items.into_boxed_slice())
}

/// Take the library's systems (library.json `systems`, and the ids its `dat_platforms`
/// gave DATs) into the table. Called when the library opens and when its systems change.
pub fn configure(meta: &Value) {
    let mut out: Vec<(&'static Platform, Media)> = vec![];
    for s in meta["systems"].as_array().into_iter().flatten() {
        let Some(id) = s["id"].as_str().filter(|id| valid_id(id).is_ok() && PLATFORMS.iter().all(|p| p.id != *id)) else { continue };
        if out.iter().any(|(p, _)| p.id == id) {
            continue;
        }
        let exts: Vec<String> = s["exts"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).collect();
        let p = Platform {
            id: leak(id),
            name: leak(s["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(id).trim()),
            dat_names: leak_list(&s["dat_names"]),
            folders: leak_list(&s["folders"]),
            exts: leak_list(&json!(exts)),
        };
        out.push((Box::leak(Box::new(p)), s["media"].as_str().and_then(Media::from_id).unwrap_or(Media::Cartridge)));
    }
    // ids typed for a DAT before systems could be added: a system named by its id
    for id in meta["dat_platforms"].as_object().into_iter().flat_map(|m| m.values()).filter_map(Value::as_str) {
        if valid_id(id).is_ok() && PLATFORMS.iter().all(|p| p.id != id) && out.iter().all(|(p, _)| p.id != id) {
            let p = Platform { id: leak(id), name: leak(id), dat_names: &[], folders: &[], exts: &[] };
            out.push((Box::leak(Box::new(p)), Media::Cartridge));
        }
    }
    // (the folder names are held while the table changes, so none are made from the old one)
    let mut folders = FOLDERS.write().ok();
    if let Ok(mut c) = CUSTOM.write() {
        *c = out;
    }
    if let Some(f) = folders.as_mut() {
        **f = None;
    }
}

/// A system's id: lower case letters, digits, `-`, `_` and `.`, as a folder name.
pub fn valid_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 40 || id.starts_with('.') || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_.".contains(c)) {
        bail!("A system's id is a short folder name: lower-case letters, digits, - _ and . (like satellaview).");
    }
    Ok(())
}

/// An id made from a system's name: "Sharp X68000" -> "sharpx68000".
pub fn id_from_name(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).flat_map(|c| c.to_lowercase()).take(40).collect()
}

/// Whether the system is one of the library's own (not built in).
pub fn is_custom(id: &str) -> bool {
    PLATFORMS.iter().all(|p| p.id != id) && custom().iter().any(|c| c.0.id == id)
}

/// Tests that add systems take turns (the table is the process's).
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn by_id(id: &str) -> Option<&'static Platform> {
    all().find(|p| p.id == id)
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

/// The platform a DAT is for, from its header name. Arcade DATs name their release or set
/// type ("MAME 0.270 ROMs (non-merged)", "FinalBurn Neo (ClrMame Pro XML, Arcade only)"),
/// so a name starting with MAME is MAME's, and one naming FinalBurn Neo is FBNeo's.
pub fn from_dat_name(name: &str) -> Option<&'static Platform> {
    let base = squash(&dat_base_name(name));
    if let Some(p) = all().find(|p| p.dat_names.iter().any(|d| squash(d) == base)) {
        return Some(p);
    }
    let lower = name.trim().to_ascii_lowercase();
    if lower == "mame" || lower.starts_with("mame ") || lower.starts_with("mame-") {
        return by_id("mame");
    }
    // FBNeo's DATs for consoles ("FinalBurn Neo - NES Games") are asked about
    let fbneo = lower.contains("finalburn neo") || lower.starts_with("fbneo") || lower.starts_with("fb neo");
    if fbneo && (lower.contains("arcade") || lower.contains("neo geo") || lower.contains("neogeo") || !lower.contains(" games")) {
        return by_id("fbneo");
    }
    None
}

/// The platform a folder's name means: one of its folder names, its id, its name or a DAT name.
pub fn from_folder(name: &str) -> Option<&'static Platform> {
    let n = squash(name);
    if n.is_empty() {
        return None;
    }
    if let Some(found) = FOLDERS.read().ok().and_then(|f| f.as_ref().map(|f| f.get(&n).copied())) {
        return found;
    }
    let mut f = FOLDERS.write().ok()?;
    // every name a system's folder can have, the first system's when two share one; made
    // once, as a scan asks for each file's folders (review M45)
    let names = f.get_or_insert_with(|| {
        let mut names = HashMap::new();
        for p in all() {
            for name in [p.id, p.name].into_iter().chain(p.folders.iter().copied()).chain(p.dat_names.iter().copied()) {
                names.entry(squash(name)).or_insert(p);
            }
        }
        names
    });
    names.get(&n).copied()
}

/// [`from_folder`]'s names, made again when the library's systems change.
static FOLDERS: RwLock<Option<HashMap<String, &'static Platform>>> = RwLock::new(None);

/// The platform an extension belongs to, when only one platform uses it.
pub fn from_ext(ext: &str) -> Option<&'static Platform> {
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let mut found = all().filter(|p| p.exts.contains(&ext.as_str()));
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
    json!(all()
        .map(|p| json!({
            "id": p.id, "name": p.name, "dat": p.dat_names.first(), "dat_names": p.dat_names, "folders": p.folders, "exts": p.exts,
            "disc": is_disc(p.id), "formats": formats(p.id), "media": media(p.id).id(), "custom": is_custom(p.id),
        }))
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
        // Redump's names and libretro's copies'
        assert_eq!(id("Sony - PlayStation"), Some("psx"));
        assert_eq!(id("Sega - Mega CD & Sega CD"), Some("segacd"));
        assert_eq!(id("Sega - Mega-CD - Sega CD"), Some("segacd"));
        assert_eq!(id("The 3DO Company - 3DO"), Some("3do"));
        assert_eq!(id("Nintendo - GameCube"), Some("gc"));
        // arcade (D57)
        assert_eq!(id("MAME"), Some("mame"));
        assert_eq!(id("MAME 0.270 ROMs (non-merged)"), Some("mame"));
        assert_eq!(id("MAME 2003-Plus"), Some("mame"));
        assert_eq!(id("FinalBurn Neo - Arcade Games"), Some("fbneo"));
        assert_eq!(id("FBNeo - Arcade Games"), Some("fbneo"));
        assert_eq!(id("Mamemoto - Things"), None);
        assert_eq!(id("FinalBurn Neo - NES Games"), None);
        assert!(is_arcade("mame") && !is_disc("mame") && formats("fbneo").is_empty());
    }

    #[test]
    fn disc_platforms_and_their_formats() {
        assert_eq!(from_ext("gdi").unwrap().id, "dreamcast");
        assert_eq!(from_ext("cso").unwrap().id, "psp");
        assert!(from_ext("chd").is_none() && from_ext("rvz").is_none());
        assert_eq!(from_folder("PS1").unwrap().id, "psx");
        assert_eq!(from_folder("Mega-CD").unwrap().id, "segacd");
        assert_eq!(formats("gc"), ["rvz"]);
        assert_eq!(formats("snes"), ["zip"]);
        assert!(formats("xbox").is_empty() && is_disc("xbox") && !is_disc("gb"));
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
