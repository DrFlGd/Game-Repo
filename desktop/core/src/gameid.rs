//! A game's own ID, as emulators name its saves after it (docs/PLAN.md, "Save sync notes,
//! part 4: emulators of their own"): a PlayStation, PlayStation 2 or PSP disc's serial
//! (`SLUS-20062`, `ULUS-10041`), a GameCube or Wii disc's six-letter ID (`GALE01`) and a
//! 3DS, Wii U or Switch game's title ID (`0004000000030700`, `00050000101C9500`,
//! `0100F2C0115B6000`) and a PlayStation 3 game's (`BLUS-30443`). Read from the image the
//! library has, whatever it's kept as (ISO, a CD's .bin, CHD, CSO/ZSO, RVZ/WIA, WBFS, .3ds,
//! NSP, WUA, PKG), and only as much of it as the ID takes; else from the file's name, as
//! the Switch's and Wii U's are named with their title IDs (`[0100F2C0115B6000]`).

use crate::image::{ChdImage, CsoReader, Layout, FRAME};
use crate::library::Library;
use rusqlite::{Connection, OptionalExtension};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// The systems a game ID is read for.
pub const SYSTEMS: &[&str] = &["psx", "ps2", "psp", "gc", "wii", "n3ds", "switch", "wiiu", "ps3"];

/// A Library game's ID, from the first of its files that has one.
pub fn of_key(conn: &Connection, lib: &Library, key: &str) -> Option<String> {
    let system = key.split('/').next()?;
    if !SYSTEMS.contains(&system) {
        return None;
    }
    let game: i64 = conn.query_row("SELECT dat_game FROM library WHERE key = ?1", [key], |r| r.get(0)).optional().ok()??;
    let files = crate::browse::files_of(conn, lib, game).ok()?;
    let files: Vec<&serde_json::Value> = files.iter().filter(|f| f["missing"] != true).collect();
    files
        .iter()
        .find_map(|f| of_file(Path::new(f["path"].as_str()?), f["kind"].as_str().unwrap_or("file"), system))
        .or_else(|| files.iter().find_map(|f| of_name(&Path::new(f["path"].as_str()?).file_name()?.to_string_lossy(), system)))
}

/// A game's ID from its file's name: a Switch or Wii U title ID in it (an update's or
/// DLC's is its game's), or a PlayStation 3 serial (`BLUS-30443`, `BLUS30443`).
pub fn of_name(name: &str, system: &str) -> Option<String> {
    match system {
        "switch" | "wiiu" => {
            let (id, _) = crate::addons::title_id(name)?;
            let ok = if system == "switch" { id >> 56 == 0x01 } else { id >> 32 == 0x0005_0000 };
            ok.then(|| format!("{id:016X}"))
        }
        "ps3" => name.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').find_map(ps3_serial),
        _ => None,
    }
}

/// `BLUS30443` or `BLUS-30443` as `BLUS-30443`: four letters (a disc's start with B, a
/// download's with N) and five digits.
fn ps3_serial(s: &str) -> Option<String> {
    let s = s.replace('-', "");
    let ok = s.len() == 9 && s[..4].bytes().all(|b| b.is_ascii_uppercase()) && matches!(s.as_bytes()[0], b'B' | b'N') && s[4..].bytes().all(|b| b.is_ascii_digit());
    ok.then(|| format!("{}-{}", &s[..4], &s[4..]))
}

/// The ID of the game in one file (`kind` as `files.kind` names it: `chd`, `cso`, … or
/// `file`).
pub fn of_file(path: &Path, kind: &str, system: &str) -> Option<String> {
    let ext = crate::platforms::ext_of(&path.to_string_lossy());
    let kind = if kind == "file" { ext.as_str() } else { kind };
    match system {
        "gc" | "wii" => nintendo_disc(path, kind),
        "n3ds" => three_ds(path, kind),
        "switch" => switch(path, kind),
        "wiiu" => wii_u(path, kind),
        "ps3" => match kind {
            "pkg" => ps3_package(path),
            _ => {
                let mut s = sectors(path, kind)?;
                ps3_disc(&mut *s)
            }
        },
        "psx" | "ps2" | "psp" => {
            let mut s = sectors(path, kind)?;
            if system == "psp" {
                psp(&mut *s)
            } else {
                playstation(&mut *s)
            }
        }
        _ => None,
    }
}

fn read_at(path: &Path, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut f = File::open(path).ok()?;
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut b = vec![0u8; len];
    f.read_exact(&mut b).ok()?;
    Some(b)
}

/// A GameCube or Wii disc's header (its first 0x20 bytes) says what it is: the ID, and
/// the magic word at 0x18 (Wii) or 0x1C (GameCube).
fn disc_id(h: &[u8]) -> Option<String> {
    let wii = h.get(0x18..0x1C)? == [0x5D, 0x1C, 0x9E, 0xA3];
    let gc = h.get(0x1C..0x20)? == [0xC2, 0x33, 0x9F, 0x3D];
    let id = h.get(..6)?;
    ((wii || gc) && id.iter().all(|b| b.is_ascii_alphanumeric())).then(|| String::from_utf8_lossy(id).into_owned())
}

fn nintendo_disc(path: &Path, kind: &str) -> Option<String> {
    match kind {
        // RVZ and WIA keep a copy of the disc's header at 0x58
        "rvz" | "wia" => disc_id(&read_at(path, 0x58, 0x20)?),
        "wbfs" => disc_id(&read_at(path, 0x200, 0x20)?),
        "iso" | "gcm" => disc_id(&read_at(path, 0, 0x20)?),
        "chd" => {
            let mut c = ChdImage::open(path).ok()?;
            let mut h = [0u8; 0x20];
            c.read_at(0, &mut h).ok()?;
            disc_id(&h)
        }
        _ => None,
    }
}

/// A 3DS cartridge image's (NCSD's) title ID, at 0x108, little-endian.
fn three_ds(path: &Path, kind: &str) -> Option<String> {
    if !matches!(kind, "3ds" | "cci") {
        return None;
    }
    let b = read_at(path, 0x100, 0x10)?;
    if &b[..4] != b"NCSD" {
        return None;
    }
    let id = u64::from_le_bytes(b[8..16].try_into().ok()?);
    (id != 0).then(|| format!("{id:016X}"))
}

/// A Switch NSP's (or NSZ's) title ID, from its ticket's name in its file list: the
/// ticket's the rights ID, the title ID and then the key's generation. An update's or
/// DLC's is its game's, as its saves are.
fn switch(path: &Path, kind: &str) -> Option<String> {
    if !matches!(kind, "nsp" | "nsz") {
        return None;
    }
    let h = read_at(path, 0, 16)?;
    if &h[..4] != b"PFS0" {
        return None;
    }
    let n = u32::from_le_bytes(h[4..8].try_into().ok()?) as u64;
    let len = u32::from_le_bytes(h[8..12].try_into().ok()?).min(1 << 16) as usize;
    let names = read_at(path, 16 + n * 24, len)?;
    names.split(|b| *b == 0).find_map(|n| {
        let n = std::str::from_utf8(n).ok()?;
        let rights = n.strip_suffix(".tik")?;
        let id = u64::from_str_radix(rights.get(..16)?, 16).ok()?;
        (rights.len() == 32 && id >> 56 == 0x01).then(|| format!("{:016X}", id & !0x1FFF))
    })
}

/// A Wii U WUA's (Cemu's ZArchive) title ID: its top folders are named by them
/// (`0005000010101c00_v0`), in the names its footer points to (the last 144 bytes,
/// big-endian, names the third of its sections).
fn wii_u(path: &Path, kind: &str) -> Option<String> {
    if kind != "wua" {
        return None;
    }
    let size = std::fs::metadata(path).ok()?.len();
    let foot = read_at(path, size.checked_sub(144)?, 144)?;
    if foot[140..144] != 0x169f_52d6u32.to_be_bytes() {
        return None;
    }
    let at = u64::from_be_bytes(foot[32..40].try_into().ok()?);
    let len = u64::from_be_bytes(foot[40..48].try_into().ok()?).min(1 << 20);
    let names = read_at(path, at, len as usize)?;
    let text = String::from_utf8_lossy(&names);
    text.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|w| w.len() == 16 && w.to_ascii_lowercase().starts_with("00050000"))
        .map(|w| w.to_ascii_uppercase())
        .next()
}

/// A PlayStation 3 package's title ID, in the content ID at 0x30
/// (`UP0001-BLUS30443_00-…`).
fn ps3_package(path: &Path) -> Option<String> {
    let h = read_at(path, 0, 0x60)?;
    if &h[..4] != b"\x7FPKG" {
        return None;
    }
    ps3_serial(std::str::from_utf8(h.get(0x37..0x40)?).ok()?)
}

/// A PlayStation 3 disc's title ID: in its PS3_DISC.SFB, or its PS3_GAME/PARAM.SFO's
/// `TITLE_ID`.
fn ps3_disc(s: &mut dyn Sectors) -> Option<String> {
    if let Some(sfb) = read_file(s, "PS3_DISC.SFB", 1024) {
        if let Some(id) = String::from_utf8_lossy(&sfb).split(|c: char| !c.is_ascii_alphanumeric() && c != '-').find_map(ps3_serial) {
            return Some(id);
        }
    }
    sfo_title(&read_file(s, "PS3_GAME/PARAM.SFO", 64 << 10)?)
}

/// A PARAM.SFO's `TITLE_ID`: its keys and values by an index, little-endian.
fn sfo_title(b: &[u8]) -> Option<String> {
    if b.get(..4)? != b"\0PSF" {
        return None;
    }
    let u32_at = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize);
    let (keys, data, n) = (u32_at(8)?, u32_at(12)?, u32_at(16)?);
    for i in 0..n.min(256) {
        let e = 20 + i * 16;
        let key_at = keys + u16::from_le_bytes(b.get(e..e + 2)?.try_into().ok()?) as usize;
        let key = b.get(key_at..)?.split(|c| *c == 0).next()?;
        if key == b"TITLE_ID" {
            let (len, at) = (u32_at(e + 4)?, data + u32_at(e + 12)?);
            let v = b.get(at..at + len)?.split(|c| *c == 0).next()?;
            return ps3_serial(std::str::from_utf8(v).ok()?);
        }
    }
    None
}

/// A disc read by its 2,048-byte sectors (the user data of a CD's), by their number.
trait Sectors {
    fn sector(&mut self, lba: u64) -> io::Result<[u8; 2048]>;
}

/// Where a raw 2,352-byte sector's data is: after the sync and header (mode 1), or after
/// the subheader too (mode 2).
fn raw_data(raw: &[u8]) -> io::Result<[u8; 2048]> {
    let at = if raw.get(15) == Some(&2) { 24 } else { 16 };
    raw.get(at..at + 2048).and_then(|d| d.try_into().ok()).ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
}

struct IsoFile(File);

impl Sectors for IsoFile {
    fn sector(&mut self, lba: u64) -> io::Result<[u8; 2048]> {
        let mut b = [0u8; 2048];
        self.0.seek(SeekFrom::Start(lba * 2048))?;
        self.0.read_exact(&mut b)?;
        Ok(b)
    }
}

/// A CD's .bin: 2,352 bytes a sector (or 2,336, mode 2 without sync and header).
struct BinFile(File, u64);

impl Sectors for BinFile {
    fn sector(&mut self, lba: u64) -> io::Result<[u8; 2048]> {
        let mut b = vec![0u8; self.1 as usize];
        self.0.seek(SeekFrom::Start(lba * self.1))?;
        self.0.read_exact(&mut b)?;
        if self.1 == 2336 {
            return raw_data(&[&[0u8; 16][..], &b].concat());
        }
        raw_data(&b)
    }
}

/// A CHD: a CD's first track by frames, or a DVD's bytes.
struct Chd(ChdImage, usize);

impl Sectors for Chd {
    fn sector(&mut self, lba: u64) -> io::Result<[u8; 2048]> {
        let mut out = [0u8; 2048];
        match self.1 {
            0 => self.0.read_at(lba * 2048, &mut out)?,
            2048 => self.0.read_at(lba * FRAME, &mut out)?,
            2336 => self.0.read_at(lba * FRAME + 8, &mut out)?,
            _ => {
                let mut raw = vec![0u8; 2352];
                self.0.read_at(lba * FRAME, &mut raw)?;
                out = raw_data(&raw)?;
            }
        }
        Ok(out)
    }
}

struct Cso(CsoReader);

impl Sectors for Cso {
    fn sector(&mut self, lba: u64) -> io::Result<[u8; 2048]> {
        let mut out = [0u8; 2048];
        self.0.read_at(lba * 2048, &mut out)?;
        Ok(out)
    }
}

fn sectors(path: &Path, kind: &str) -> Option<Box<dyn Sectors>> {
    Some(match kind {
        "iso" => Box::new(IsoFile(File::open(path).ok()?)),
        "bin" | "img" => {
            let f = File::open(path).ok()?;
            let len = f.metadata().ok()?.len();
            let head = read_at(path, 0, 12)?;
            let sync = head == [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0];
            if sync && len % 2352 == 0 {
                Box::new(BinFile(f, 2352))
            } else if len % 2336 == 0 {
                Box::new(BinFile(f, 2336))
            } else {
                Box::new(IsoFile(f))
            }
        }
        "chd" => {
            let c = ChdImage::open(path).ok()?;
            let size = match &c.layout {
                Layout::Cd(tracks) => tracks.first().filter(|t| !t.is_audio())?.data_size(),
                Layout::Dvd => 0,
                Layout::Raw => return None,
            };
            Box::new(Chd(c, size))
        }
        "cso" | "zso" => Box::new(Cso(CsoReader::open(path).ok()?)),
        _ => return None,
    })
}

/// A file in an ISO 9660 disc, by its path from the top folder (`PS3_GAME/PARAM.SFO`):
/// (its first sector, its size). Its names are matched without case or their `;1`.
fn find_file(s: &mut dyn Sectors, path: &str) -> Option<(u64, u32)> {
    let pvd = s.sector(16).ok()?;
    if &pvd[1..6] != b"CD001" {
        return None;
    }
    let root = &pvd[156..156 + 34];
    let mut at = (u32::from_le_bytes(root[2..6].try_into().ok()?) as u64, u32::from_le_bytes(root[10..14].try_into().ok()?));
    for name in path.split('/') {
        at = find_in(s, at.0, at.1, name)?;
    }
    Some(at)
}

/// A name in a folder of `len` bytes from sector `lba`.
fn find_in(s: &mut dyn Sectors, lba: u64, len: u32, name: &str) -> Option<(u64, u32)> {
    // a folder bigger than this isn't one
    for i in 0..(len as u64).div_ceil(2048).min(64) {
        let sec = s.sector(lba + i).ok()?;
        let mut at = 0usize;
        while at < 2048 {
            let n = sec[at] as usize;
            if n == 0 {
                break;
            }
            let rec = sec.get(at..at + n)?;
            let name_len = *rec.get(32)? as usize;
            let id = String::from_utf8_lossy(rec.get(33..33 + name_len)?);
            let id = id.split(';').next().unwrap_or("");
            if id.eq_ignore_ascii_case(name) {
                let first = u32::from_le_bytes(rec[2..6].try_into().ok()?) as u64;
                let size = u32::from_le_bytes(rec[10..14].try_into().ok()?);
                return Some((first, size));
            }
            at += n;
        }
    }
    None
}

fn read_file(s: &mut dyn Sectors, name: &str, most: u32) -> Option<Vec<u8>> {
    let (first, size) = find_file(s, name)?;
    let size = size.min(most);
    let mut out = vec![];
    for i in 0..(size as u64).div_ceil(2048) {
        out.extend_from_slice(&s.sector(first + i).ok()?);
    }
    out.truncate(size as usize);
    Some(out)
}

/// A PlayStation or PlayStation 2 disc's serial, from the program its SYSTEM.CNF starts
/// (`BOOT2 = cdrom0:\SLUS_200.62;1` is `SLUS-20062`).
fn playstation(s: &mut dyn Sectors) -> Option<String> {
    let cnf = String::from_utf8_lossy(&read_file(s, "SYSTEM.CNF", 4096)?).into_owned();
    let line = cnf.lines().find(|l| l.trim_start().to_ascii_uppercase().starts_with("BOOT2")).or_else(|| cnf.lines().find(|l| l.trim_start().to_ascii_uppercase().starts_with("BOOT")))?;
    let (_, v) = line.split_once('=')?;
    serial(v.trim().rsplit(['\\', '/', ':']).next()?.split(';').next()?)
}

/// `SLUS_200.62` as `SLUS-20062`.
fn serial(file: &str) -> Option<String> {
    let (head, rest) = file.split_once(['_', '-'])?;
    let digits: String = rest.chars().filter(|c| *c != '.').collect();
    let ok = head.len() == 4 && head.chars().all(|c| c.is_ascii_alphabetic()) && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
    ok.then(|| format!("{}-{digits}", head.to_ascii_uppercase()))
}

/// A PSP disc's ID, the first part of its UMD_DATA.BIN (`ULUS-10041|…`).
fn psp(s: &mut dyn Sectors) -> Option<String> {
    let b = read_file(s, "UMD_DATA.BIN", 64)?;
    let text = String::from_utf8_lossy(&b);
    let id = text.split('|').next()?.trim();
    serial(id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An ISO 9660 image with these files in its top folder, as 2,048-byte sectors.
    pub(crate) fn iso(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut img = vec![0u8; 2048 * 20];
        let root_lba = 18u32;
        let mut dir = vec![];
        let mut data_at = 20u32;
        let mut data = vec![];
        for (name, bytes) in files {
            let id = format!("{name};1");
            let n = 33 + id.len() + (id.len() + 1) % 2;
            let mut rec = vec![0u8; n];
            rec[0] = n as u8;
            rec[2..6].copy_from_slice(&data_at.to_le_bytes());
            rec[10..14].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            rec[32] = id.len() as u8;
            rec[33..33 + id.len()].copy_from_slice(id.as_bytes());
            dir.extend(rec);
            let mut b = bytes.to_vec();
            b.resize(bytes.len().div_ceil(2048).max(1) * 2048, 0);
            data_at += (b.len() / 2048) as u32;
            data.extend(b);
        }
        let pvd = 16 * 2048;
        img[pvd] = 1;
        img[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
        let root = pvd + 156;
        img[root] = 34;
        img[root + 2..root + 6].copy_from_slice(&root_lba.to_le_bytes());
        img[root + 10..root + 14].copy_from_slice(&2048u32.to_le_bytes());
        img[root_lba as usize * 2048..root_lba as usize * 2048 + dir.len()].copy_from_slice(&dir);
        img.extend(data);
        img
    }

    /// The same as a CD's .bin: each sector raw, mode 2 form 1.
    fn as_bin(iso: &[u8]) -> Vec<u8> {
        let mut out = vec![];
        for s in iso.chunks(2048) {
            let mut raw = vec![0u8; 2352];
            raw[1..11].fill(0xFF);
            raw[15] = 2;
            raw[24..24 + s.len()].copy_from_slice(s);
            out.extend(raw);
        }
        out
    }

    #[test]
    fn a_discs_id_is_read_from_its_image() {
        let dir = std::env::temp_dir().join(format!("gamerepo-test-gameid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, b: &[u8]| {
            let p = dir.join(n);
            std::fs::write(&p, b).unwrap();
            p
        };
        let ps2 = w("ps2.iso", &iso(&[("SYSTEM.CNF", b"BOOT2 = cdrom0:\\SLUS_200.62;1\r\nVER = 1.00\r\n")]));
        assert_eq!(of_file(&ps2, "file", "ps2").as_deref(), Some("SLUS-20062"));
        let ps1 = w("ps1.bin", &as_bin(&iso(&[("README.TXT", b"hi"), ("SYSTEM.CNF", b"BOOT = cdrom:\\SCUS_944.55;1\r\n")])));
        assert_eq!(of_file(&ps1, "file", "psx").as_deref(), Some("SCUS-94455"));
        let psp = w("psp.iso", &iso(&[("UMD_DATA.BIN", b"ULUS-10041|0001|G|")]));
        assert_eq!(of_file(&psp, "file", "psp").as_deref(), Some("ULUS-10041"));
        let mut gc = b"GALE01".to_vec();
        gc.resize(0x40, 0);
        gc[0x1C..0x20].copy_from_slice(&[0xC2, 0x33, 0x9F, 0x3D]);
        let gcm = w("gc.iso", &gc);
        assert_eq!(of_file(&gcm, "file", "gc").as_deref(), Some("GALE01"));
        let mut rvz = b"RVZ\x01".to_vec();
        rvz.resize(0x58, 0);
        rvz.extend(&gc);
        assert_eq!(of_file(&w("gc.rvz", &rvz), "rvz", "gc").as_deref(), Some("GALE01"));
        let mut n3ds = vec![0u8; 0x200];
        n3ds[0x100..0x104].copy_from_slice(b"NCSD");
        n3ds[0x108..0x110].copy_from_slice(&0x0004_0000_0003_0700u64.to_le_bytes());
        assert_eq!(of_file(&w("g.3ds", &n3ds), "file", "n3ds").as_deref(), Some("0004000000030700"));
        assert_eq!(of_file(&w("junk.iso", &[0u8; 4096]), "file", "ps2"), None, "no ID: none");
        assert_eq!(serial("SLES_123.45"), Some("SLES-12345".into()));

        // a Switch NSP by its ticket's name (an update's is its game's), a Wii U WUA by its
        // top folder's, a PlayStation 3 package by its content ID and disc by its SFB
        let names = b"0123456789abcdef0123456789abcdef.nca\x000100F2C0115B68000000000000000000.tik\x00";
        let mut nsp = b"PFS0".to_vec();
        nsp.extend(2u32.to_le_bytes());
        nsp.extend((names.len() as u32).to_le_bytes());
        nsp.extend([0u8; 4]);
        nsp.extend([0u8; 48]);
        nsp.extend(names);
        assert_eq!(of_file(&w("g.nsp", &nsp), "file", "switch").as_deref(), Some("0100F2C0115B6000"));
        let mut wua = vec![0u8; 64];
        wua.extend(b"\x13code\x130005000010101c00_v0");
        let mut foot = vec![0u8; 144];
        foot[32..40].copy_from_slice(&64u64.to_be_bytes());
        foot[40..48].copy_from_slice(&26u64.to_be_bytes());
        foot[140..144].copy_from_slice(&0x169f_52d6u32.to_be_bytes());
        wua.extend(foot);
        assert_eq!(of_file(&w("g.wua", &wua), "file", "wiiu").as_deref(), Some("0005000010101C00"));
        let mut pkg = b"\x7FPKG".to_vec();
        pkg.resize(0x30, 0);
        pkg.extend(b"UP0001-NPUB30024_00-0000000000000001");
        pkg.resize(0x80, 0);
        assert_eq!(of_file(&w("g.pkg", &pkg), "file", "ps3").as_deref(), Some("NPUB-30024"));
        let ps3 = w("ps3.iso", &iso(&[("PS3_DISC.SFB", b".SFB\x00\x00\x00\x01HYBRID_FLAG\x00g\x00TITLE_ID\x00BLUS-30443")]));
        assert_eq!(of_file(&ps3, "file", "ps3").as_deref(), Some("BLUS-30443"));
        let mut sfo = b"\0PSF\x01\x01\x00\x00".to_vec();
        sfo.extend(36u32.to_le_bytes());
        sfo.extend(48u32.to_le_bytes());
        sfo.extend(1u32.to_le_bytes());
        sfo.extend([0, 0, 4, 2, 10, 0, 0, 0, 16, 0, 0, 0, 0, 0, 0, 0]);
        sfo.extend(b"TITLE_ID\0\0\0\0");
        sfo.extend(b"BCES00104\0");
        assert_eq!(sfo_title(&sfo).as_deref(), Some("BCES-00104"));
        assert_eq!(of_name("Super Mario Odyssey [0100000000010000][v0].nsp", "switch").as_deref(), Some("0100000000010000"));
        assert_eq!(of_name("Super Mario Odyssey [0100000000010800][v131072].nsp", "switch").as_deref(), Some("0100000000010000"), "an update's is its game's");
        assert_eq!(of_name("Mario Kart 8 [0005000010145D00].wua", "wiiu").as_deref(), Some("0005000010145D00"));
        assert_eq!(of_name("Demon's Souls [BLUS30443].iso", "ps3").as_deref(), Some("BLUS-30443"));
        assert_eq!((of_name("Mario Kart 8 [0005000010145D00].wua", "switch"), of_name("Game (USA).iso", "ps3")), (None, None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
