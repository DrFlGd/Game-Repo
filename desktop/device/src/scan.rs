//! What ES-DE has on the device (docs/PLAN.md, "Phase 9b design", D100): every game in
//! each system's folder of the ROMs folder, found as ES-DE finds them (the system's
//! extensions, a `<name>.m3u/` folder as one game, the tracks a `.cue` or `.gdi` names and
//! the discs an `.m3u` lists as part of their game), with its media from ES-DE's media
//! folder, when it was last played (its gamelist), and what's needed to ask the server
//! which of the library's games it is (a ZIP's CRCs from its directory, a CHD's SHA-1 from
//! its header, a disc's cue sheet), never reading a large file whole.

use crate::Device;
use gamerepo_wire::{api, gamelist, MANUAL, MEDIA};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// A loose file this size or smaller is read whole for its CRC32 (a cartridge's ROM).
pub const CRC_WHOLE: u64 = 64 << 20;

/// A game ES-DE lists on the device.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Found {
    /// Its path in the ROMs folder (`gb/Alpha (Europe).gb`, `psx/Quest (USA).m3u`)
    pub id: String,
    /// Its system's folder
    pub system: String,
    /// As its gamelist lists it, else its file's name
    pub name: String,
    /// Its files in the ROMs folder with their sizes, the one ES-DE lists first
    pub files: Vec<(String, u64)>,
    pub media: Vec<MediaFile>,
    /// Its files' and media's
    pub size: u64,
    /// When it was last played (ES-DE's `20250102T030405`); empty: never
    pub last_played: String,
    pub play_count: u32,
    /// Its file's modified time (seconds since 1970)
    pub modified: i64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MediaFile {
    /// cover, screenshot… (the library's kinds), manual, or ES-DE's folder's name for others
    pub kind: String,
    /// Its path in the media folder
    pub path: String,
    pub size: u64,
}

/// A media folder's kind (`covers`: cover).
pub fn kind_of_folder(folder: &str) -> String {
    MEDIA.iter().chain([&MANUAL]).find(|(_, _, f)| f.eq_ignore_ascii_case(folder)).map(|(k, _, _)| k.to_string()).unwrap_or_else(|| folder.to_string())
}

fn lower_ext(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
}

fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((s, _)) if !s.is_empty() => s,
        _ => name,
    }
}

fn join(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else if name.is_empty() {
        rel.to_string()
    } else {
        format!("{rel}/{name}")
    }
}

/// The files a `.cue`, `.gdi` or `.m3u` names, as written in it.
pub fn named_in(path: &Path) -> Vec<String> {
    let Ok(meta) = std::fs::metadata(path) else { return vec![] };
    if meta.len() > 1 << 20 {
        return vec![];
    }
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match lower_ext(&name).as_str() {
        "cue" => text
            .lines()
            .filter_map(|l| {
                let l = l.trim();
                let rest = l.strip_prefix("FILE ").or_else(|| l.strip_prefix("file "))?.trim();
                Some(match rest.strip_prefix('"') {
                    Some(q) => q.split('"').next()?.to_string(),
                    None => rest.rsplit_once(' ').map(|(n, _)| n.to_string()).unwrap_or_else(|| rest.to_string()),
                })
            })
            .collect(),
        "gdi" => text
            .lines()
            .skip(1)
            .filter_map(|l| {
                let l = l.trim();
                if let Some(at) = l.find('"') {
                    return l[at + 1..].split('"').next().map(String::from);
                }
                l.split_whitespace().nth(4).map(String::from)
            })
            .collect(),
        "m3u" => text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(String::from).collect(),
        _ => vec![],
    }
}

/// Every file under a folder, with its path from `rel` and size.
fn files_under(dir: &Path, rel: &str, out: &mut Vec<(String, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            files_under(&e.path(), &join(rel, &name), out);
        } else if t.is_file() {
            out.push((join(rel, &name), e.metadata().map(|m| m.len()).unwrap_or(0)));
        }
    }
}

fn modified(p: &Path) -> i64 {
    std::fs::metadata(p).ok().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The games in one folder of a system's folder (`rel` from the system's folder), and in
/// the folders inside it.
fn walk(dir: &Path, rel: &str, folder: &str, exts: &HashSet<String>, out: &mut Vec<Found>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).collect();
    entries.sort_by_key(|e| e.file_name());
    let m3u = exts.contains("m3u");
    // the files named by the folder's cue sheets, GD-ROM lists and playlists are part of
    // the game that names them, not games of their own
    let mut named: HashMap<String, Vec<String>> = HashMap::new();
    let mut part_of: HashSet<String> = HashSet::new();
    for e in &entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if matches!(lower_ext(&name).as_str(), "cue" | "gdi" | "m3u") && e.file_type().is_ok_and(|t| t.is_file()) {
            let v: Vec<String> = named_in(&e.path()).into_iter().map(|n| n.replace('\\', "/")).filter(|n| !n.contains("..") && dir.join(n).is_file()).collect();
            part_of.extend(v.iter().map(|n| n.to_lowercase()));
            named.insert(name, v);
        }
    }
    for e in &entries {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            if m3u && lower_ext(&name) == "m3u" {
                // ES-DE's "directories interpreted as files": one game
                let mut files = vec![];
                files_under(&e.path(), &join(&join(folder, rel), &name), &mut files);
                if files.is_empty() {
                    continue;
                }
                let inner = format!("{}/{name}", join(folder, rel));
                // the playlist inside first
                files.sort_by_key(|(p, _)| !(p.to_lowercase() == format!("{}/{}", inner.to_lowercase(), name.to_lowercase())));
                out.push(Found { id: join(&join(folder, rel), &name), system: folder.to_string(), name: stem(&name).to_string(), files, modified: modified(&e.path()), ..Default::default() });
            } else {
                walk(&e.path(), &join(rel, &name), folder, exts, out);
            }
            continue;
        }
        if !t.is_file() || !exts.contains(&lower_ext(&name)) || part_of.contains(&name.to_lowercase()) {
            continue;
        }
        let at = join(folder, rel);
        let mut files = vec![(join(&at, &name), e.metadata().map(|m| m.len()).unwrap_or(0))];
        // what it names, and what those name (a playlist's cue sheets' tracks)
        let mut todo: Vec<String> = named.get(&name).cloned().unwrap_or_default();
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(n) = todo.pop() {
            if !seen.insert(n.to_lowercase()) {
                continue;
            }
            let p = dir.join(&n);
            files.push((join(&at, &n), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)));
            if let Some(more) = named.get(&n) {
                todo.extend(more.iter().cloned());
            }
        }
        out.push(Found { id: join(&at, &name), system: folder.to_string(), name: stem(&name).to_string(), files, modified: modified(&e.path()), ..Default::default() });
    }
}

/// Every game ES-DE lists on the device, for the systems it knows (their folders and
/// extensions, from the server's systems kept in the catalogue).
pub fn scan(d: &Device, systems: &[api::System]) -> Vec<Found> {
    let roms = &d.places["roms"];
    let mut out = vec![];
    let mut done: HashSet<String> = HashSet::new();
    for sys in systems {
        if sys.extensions.is_empty() || !done.insert(sys.folder.to_lowercase()) {
            continue;
        }
        let dir = roms.join(&sys.folder);
        if !dir.is_dir() {
            continue;
        }
        let exts: HashSet<String> = sys.extensions.iter().map(|e| e.to_lowercase()).collect();
        let first = out.len();
        walk(&dir, "", &sys.folder, &exts, &mut out);
        if out.len() == first {
            continue;
        }
        // the gamelist's names, plays and when each was last played
        let gl = std::fs::read_to_string(d.places["esde"].join("gamelists").join(&sys.folder).join("gamelist.xml")).unwrap_or_default();
        let entries: HashMap<String, String> = gamelist::blocks(&gl).into_iter().collect();
        // the media, by the file's name without its extension, from the system's folder
        let mut media: HashMap<String, Vec<MediaFile>> = HashMap::new();
        let mdir = d.places["media"].join(&sys.folder);
        for k in std::fs::read_dir(&mdir).into_iter().flatten().flatten() {
            if !k.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let kfolder = k.file_name().to_string_lossy().into_owned();
            let mut files = vec![];
            files_under(&k.path(), "", &mut files);
            for (p, size) in files {
                media.entry(stem(&p).to_lowercase()).or_default().push(MediaFile { kind: kind_of_folder(&kfolder), path: format!("{}/{kfolder}/{p}", sys.folder), size });
            }
        }
        for g in &mut out[first..] {
            let rel = g.id.split_once('/').map(|(_, r)| r).unwrap_or(&g.id);
            if let Some(b) = entries.get(&gamelist::norm_path(rel)) {
                if let Some(n) = gamelist::tag(b, "name") {
                    g.name = n;
                }
                g.last_played = gamelist::tag(b, "lastplayed").unwrap_or_default();
                g.play_count = gamelist::tag(b, "playcount").and_then(|c| c.parse().ok()).unwrap_or(0);
            }
            g.media = media.remove(&stem(rel).to_lowercase()).unwrap_or_default();
            g.size = g.files.iter().map(|(_, s)| s).sum::<u64>() + g.media.iter().map(|m| m.size).sum::<u64>();
        }
    }
    out
}

/// A ZIP's members' CRC32s and sizes, from its central directory (none for a ZIP64 or one
/// that can't be read).
pub fn zip_crcs(path: &Path) -> Option<Vec<(String, u64)>> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let tail = len.min(65_557);
    f.seek(SeekFrom::Start(len - tail)).ok()?;
    let mut buf = vec![0u8; tail as usize];
    f.read_exact(&mut buf).ok()?;
    let at = (0..buf.len().saturating_sub(21)).rev().find(|&i| buf[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])?;
    let e = &buf[at..];
    let entries = u16::from_le_bytes([e[10], e[11]]) as usize;
    let size = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as u64;
    let offset = u32::from_le_bytes([e[16], e[17], e[18], e[19]]) as u64;
    if offset == 0xffff_ffff || size > 64 << 20 {
        return None;
    }
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut cd = vec![0u8; size as usize];
    f.read_exact(&mut cd).ok()?;
    let mut out = vec![];
    let mut i = 0;
    for _ in 0..entries {
        if i + 46 > cd.len() || cd[i..i + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let crc = u32::from_le_bytes([cd[i + 16], cd[i + 17], cd[i + 18], cd[i + 19]]);
        let usize_ = u32::from_le_bytes([cd[i + 24], cd[i + 25], cd[i + 26], cd[i + 27]]) as u64;
        let (n, x, c) = (u16::from_le_bytes([cd[i + 28], cd[i + 29]]) as usize, u16::from_le_bytes([cd[i + 30], cd[i + 31]]) as usize, u16::from_le_bytes([cd[i + 32], cd[i + 33]]) as usize);
        let name = String::from_utf8_lossy(cd.get(i + 46..i + 46 + n)?).into_owned();
        if !name.ends_with('/') {
            out.push((format!("{crc:08x}"), usize_));
        }
        i += 46 + n + x + c;
    }
    Some(out)
}

/// A CHD's SHA-1 from its header (versions 3 to 5).
pub fn chd_sha1(path: &Path) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = [0u8; 124];
    f.read_exact(&mut h[..16]).ok()?;
    if &h[..8] != b"MComprHD" {
        return None;
    }
    let version = u32::from_be_bytes([h[12], h[13], h[14], h[15]]);
    let at = match version {
        5 => 84,
        4 => 48,
        3 => 80,
        _ => return None,
    };
    f.read_exact(&mut h[16..at + 20]).ok()?;
    Some(hex::encode(&h[at..at + 20]))
}

fn crc_whole(path: &Path) -> Option<(String, u64)> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let r = f.read(&mut buf).ok()?;
        if r == 0 {
            break;
        }
        h.update(&buf[..r]);
        n += r as u64;
    }
    Some((format!("{:08x}", h.finalize()), n))
}

/// What the server is told of a game for it to say which of the library's it is (C18):
/// a ZIP's members' CRCs, a CHD's SHA-1, a disc's cue sheet's (or GD-ROM list's) CRC, or a
/// small loose file's. None: nothing to tell it by (a 7-Zip archive, a large image).
pub fn found(d: &Device, g: &Found) -> Option<api::Found> {
    let roms = &d.places["roms"];
    let main = roms.join(&g.files.first()?.0);
    let size = g.files.first()?.1;
    let ask = |crcs: Vec<(String, u64)>, chd: Option<String>| Some(api::Found { name: g.id.clone(), size, crcs, chd_sha1: chd });
    // a game in an .m3u folder, or an .m3u, by its first disc
    let first_disc = |files: &[(String, u64)]| -> Option<std::path::PathBuf> {
        let playlist = files.iter().find(|(p, _)| lower_ext(p) == "m3u").map(|(p, _)| roms.join(p))?;
        let first = named_in(&playlist).into_iter().next()?;
        Some(playlist.parent()?.join(first))
    };
    let target = if main.is_dir() || lower_ext(&g.files[0].0) == "m3u" { first_disc(&g.files).unwrap_or(main) } else { main };
    match lower_ext(&target.to_string_lossy()).as_str() {
        "zip" => ask(zip_crcs(&target)?, None),
        "chd" => ask(vec![], Some(chd_sha1(&target)?)),
        "7z" => None,
        _ => {
            let len = std::fs::metadata(&target).ok()?.len();
            if len > CRC_WHOLE {
                return None;
            }
            ask(vec![crc_whole(&target)?], None)
        }
    }
}

/// The games to offer when there isn't room (C20): those played longest ago first, then
/// those never played, the oldest first, until `need` bytes would be freed. Each game's
/// id, size, when it was last played and when it came.
pub fn make_room(games: &[(String, u64, String, i64)], need: u64) -> Vec<String> {
    let mut v: Vec<&(String, u64, String, i64)> = games.iter().collect();
    // played ones by when (ES-DE's times sort as text), then the never played by age
    v.sort_by(|a, b| (a.2.is_empty(), &a.2, a.3, &a.0).cmp(&(b.2.is_empty(), &b.2, b.3, &b.0)));
    let mut out = vec![];
    let mut freed = 0u64;
    for g in v {
        if freed >= need {
            break;
        }
        freed += g.1;
        out.push(g.0.clone());
    }
    out
}

/// The games found by system, each system's in name order.
pub fn by_system(found: Vec<Found>) -> BTreeMap<String, Vec<Found>> {
    let mut m: BTreeMap<String, Vec<Found>> = BTreeMap::new();
    for g in found {
        m.entry(g.system.clone()).or_default().push(g);
    }
    for v in m.values_mut() {
        v.sort_by_key(|g| g.name.to_lowercase());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as Map;

    fn device(dir: &Path) -> Device {
        std::fs::create_dir_all(dir.join("ES-DE/settings")).unwrap();
        std::fs::write(dir.join("ES-DE/settings/es_settings.xml"), "<?xml version=\"1.0\"?>\n<string name=\"ROMDirectory\" value=\"\" />\n").unwrap();
        Device::open(dir).unwrap()
    }

    fn put(dir: &Path, rel: &str, b: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b).unwrap();
    }

    fn sys(id: &str, ext: &[&str]) -> api::System {
        api::System { id: id.into(), folder: id.into(), extensions: ext.iter().map(|e| e.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn es_des_games_are_found_with_their_tracks_discs_and_media() {
        let dir = std::env::temp_dir().join(format!("gamerepo-device-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = device(&dir);
        put(&dir, "ROMs/gb/Alpha.gb", b"alpha");
        put(&dir, "ROMs/gb/Alpha.srm", b"a save");
        put(&dir, "ROMs/gb/More/Beta.zip", b"not really a zip");
        put(&dir, "ROMs/gb/.hidden/Gamma.gb", b"x");
        put(&dir, "ROMs/psx/Quest.m3u/Quest.m3u", b"Quest (Disc 1).cue\nQuest (Disc 2).cue\n");
        put(&dir, "ROMs/psx/Quest.m3u/Quest (Disc 1).cue", b"FILE \"Quest (Disc 1).bin\" BINARY\n");
        put(&dir, "ROMs/psx/Quest.m3u/Quest (Disc 1).bin", &[0; 100]);
        put(&dir, "ROMs/psx/Quest.m3u/Quest (Disc 2).cue", b"FILE \"Quest (Disc 2).bin\" BINARY\n");
        put(&dir, "ROMs/psx/Quest.m3u/Quest (Disc 2).bin", &[0; 100]);
        put(&dir, "ROMs/psx/Saga.m3u", b"Saga (Disc 1).cue\r\nSaga (Disc 2).cue\r\n");
        put(&dir, "ROMs/psx/Saga (Disc 1).cue", b"FILE \"Saga (Disc 1) (Track 1).bin\" BINARY\nFILE \"Saga (Disc 1) (Track 2).bin\" BINARY\n");
        put(&dir, "ROMs/psx/Saga (Disc 1) (Track 1).bin", &[1; 10]);
        put(&dir, "ROMs/psx/Saga (Disc 1) (Track 2).bin", &[2; 10]);
        put(&dir, "ROMs/psx/Saga (Disc 2).cue", b"FILE Saga2.bin BINARY\n");
        put(&dir, "ROMs/psx/Saga2.bin", &[3; 10]);
        put(&dir, "ROMs/psx/Lone.chd", &[4; 30]);
        put(&dir, "ROMs/dreamcast/Disc.gdi", b"3\n1 0 4 2352 track01.bin 0\n2 600 0 2352 \"track 02.raw\" 0\n3 45000 4 2352 track03.bin 0\n");
        put(&dir, "ROMs/dreamcast/track01.bin", &[5; 4]);
        put(&dir, "ROMs/dreamcast/track 02.raw", &[6; 4]);
        put(&dir, "ROMs/dreamcast/track03.bin", &[7; 4]);
        put(&dir, "ROMs/nes/Unknown.nes", b"nes");
        put(&dir, "ES-DE/downloaded_media/gb/covers/Alpha.png", &[8; 7]);
        put(&dir, "ES-DE/downloaded_media/gb/miximages/Alpha.png", &[8; 3]);
        put(&dir, "ES-DE/downloaded_media/gb/screenshots/More/Beta.jpg", &[8; 5]);
        put(&dir, "ES-DE/downloaded_media/psx/manuals/Quest.pdf", &[8; 2]);
        put(&dir, "ES-DE/gamelists/gb/gamelist.xml", b"<gameList><game><path>./Alpha.gb</path><name>Alpha &amp; Co</name><playcount>3</playcount><lastplayed>20250102T030405</lastplayed></game></gameList>");
        let systems = [sys("gb", &["gb", "zip", "7z"]), sys("psx", &["cue", "bin", "img", "chd", "m3u"]), sys("dreamcast", &["gdi", "cdi", "cue", "bin", "chd"])];
        let found = scan(&d, &systems);
        let v: Map<&str, (&str, Vec<&str>, u64)> = found.iter().map(|g| (g.id.as_str(), (g.name.as_str(), g.files.iter().map(|(p, _)| p.as_str()).collect(), g.size))).collect();
        assert_eq!(v.keys().copied().collect::<Vec<_>>(), ["dreamcast/Disc.gdi", "gb/Alpha.gb", "gb/More/Beta.zip", "psx/Lone.chd", "psx/Quest.m3u", "psx/Saga.m3u"], "saves, hidden folders and tracks aren't games; nes isn't asked for");
        assert_eq!(v["gb/Alpha.gb"], ("Alpha & Co", vec!["gb/Alpha.gb"], 5 + 7 + 3));
        assert_eq!(v["psx/Quest.m3u"].1, ["psx/Quest.m3u/Quest.m3u", "psx/Quest.m3u/Quest (Disc 1).bin", "psx/Quest.m3u/Quest (Disc 1).cue", "psx/Quest.m3u/Quest (Disc 2).bin", "psx/Quest.m3u/Quest (Disc 2).cue"]);
        let quest: u64 = std::fs::read_dir(dir.join("ROMs/psx/Quest.m3u")).unwrap().map(|e| e.unwrap().metadata().unwrap().len()).sum();
        assert_eq!(v["psx/Quest.m3u"].2, quest + 2, "the discs and the manual");
        let mut saga = v["psx/Saga.m3u"].1.clone();
        saga.sort();
        assert_eq!(saga, ["psx/Saga (Disc 1) (Track 1).bin", "psx/Saga (Disc 1) (Track 2).bin", "psx/Saga (Disc 1).cue", "psx/Saga (Disc 2).cue", "psx/Saga.m3u", "psx/Saga2.bin"]);
        let mut gdi = v["dreamcast/Disc.gdi"].1.clone();
        gdi.sort();
        assert_eq!(gdi, ["dreamcast/Disc.gdi", "dreamcast/track 02.raw", "dreamcast/track01.bin", "dreamcast/track03.bin"]);
        let alpha = found.iter().find(|g| g.id == "gb/Alpha.gb").unwrap();
        assert_eq!((alpha.last_played.as_str(), alpha.play_count), ("20250102T030405", 3));
        assert_eq!(alpha.media.iter().map(|m| (m.kind.as_str(), m.path.as_str())).collect::<Vec<_>>().len(), 2);
        assert!(alpha.media.iter().any(|m| m.kind == "cover" && m.path == "gb/covers/Alpha.png"));
        assert!(found.iter().find(|g| g.id == "gb/More/Beta.zip").unwrap().media.iter().any(|m| m.kind == "screenshot" && m.path == "gb/screenshots/More/Beta.jpg"));
        assert!(found.iter().find(|g| g.id == "psx/Quest.m3u").unwrap().media.iter().any(|m| m.kind == "manual"));
        // what the server is asked: a disc game by its first disc's cue sheet
        let q = found.iter().find(|g| g.id == "psx/Quest.m3u").unwrap();
        let cue = crc_whole(&dir.join("ROMs/psx/Quest.m3u/Quest (Disc 1).cue")).unwrap();
        assert_eq!(found_of(&d, q).crcs, vec![cue]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn found_of(d: &Device, g: &Found) -> api::Found {
        found(d, g).unwrap()
    }

    #[test]
    fn a_zips_crcs_come_from_its_directory_and_a_chds_sha1_from_its_header() {
        let dir = std::env::temp_dir().join(format!("gamerepo-device-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // a ZIP of two stored members, written by hand
        let mut zip = vec![];
        let mut cd = vec![];
        for (name, data) in [("a.bin", &b"hello"[..]), ("dir/", &b""[..]), ("b.bin", &b"world!"[..])] {
            let crc = crc32fast::hash(data);
            let at = zip.len() as u32;
            let head = |sig: [u8; 4], central: bool| {
                let mut h = sig.to_vec();
                if central {
                    h.extend([20, 0]);
                }
                h.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                h.extend(crc.to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((data.len() as u32).to_le_bytes());
                h.extend((name.len() as u16).to_le_bytes());
                h.extend([0, 0]);
                if central {
                    h.extend([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
                    h.extend(at.to_le_bytes());
                }
                h.extend(name.as_bytes());
                h
            };
            zip.extend(head([0x50, 0x4b, 0x03, 0x04], false));
            zip.extend(data);
            cd.extend(head([0x50, 0x4b, 0x01, 0x02], true));
        }
        let cd_at = zip.len() as u32;
        zip.extend(&cd);
        zip.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 3, 0, 3, 0]);
        zip.extend((cd.len() as u32).to_le_bytes());
        zip.extend(cd_at.to_le_bytes());
        zip.extend([0, 0]);
        std::fs::write(dir.join("t.zip"), &zip).unwrap();
        assert_eq!(zip_crcs(&dir.join("t.zip")).unwrap(), [(format!("{:08x}", crc32fast::hash(b"hello")), 5), (format!("{:08x}", crc32fast::hash(b"world!")), 6)]);
        assert_eq!(zip_crcs(&dir.join("t.zip").with_extension("none")), None);
        std::fs::write(dir.join("not.zip"), b"PK no").unwrap();
        assert_eq!(zip_crcs(&dir.join("not.zip")), None);
        // a CHD (version 5) header with its SHA-1 at 84
        let mut chd = b"MComprHD".to_vec();
        chd.extend(124u32.to_be_bytes());
        chd.extend(5u32.to_be_bytes());
        chd.resize(124, 0);
        chd[84..104].copy_from_slice(&[0xab; 20]);
        std::fs::write(dir.join("t.chd"), &chd).unwrap();
        assert_eq!(chd_sha1(&dir.join("t.chd")).unwrap(), "ab".repeat(20));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn room_is_made_from_the_games_played_longest_ago() {
        let g = |id: &str, size: u64, played: &str, added: i64| (id.to_string(), size, played.to_string(), added);
        let games = [g("new", 10, "", 300), g("old", 10, "", 100), g("lately", 10, "20250301T000000", 50), g("long ago", 10, "20230101T000000", 400)];
        assert_eq!(make_room(&games, 25), ["long ago", "lately", "old"]);
        assert_eq!(make_room(&games, 0), Vec::<String>::new());
        assert_eq!(make_room(&games, 1000).len(), 4);
    }
}
