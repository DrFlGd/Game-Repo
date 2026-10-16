//! Writing ZIPs TorrentZip-style (docs/PLAN.md, "Phase 2 design", Rebuilding ZIPs), so
//! the same game always gives the same bytes, here and in other collectors' tools:
//! members sorted by lower-case name, the fixed date TorrentZip uses (1996-12-24
//! 23:32), no extra fields, zlib's deflate at level 9, and the
//! `TORRENTZIPPED-<CRC32 of the central directory>` comment. The members are hashed as
//! they're compressed, so the caller gets their hashes from the one read.

use crate::hash::{self, Hashes};
use flate2::write::DeflateEncoder;
use flate2::Compression;
use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

const DOS_TIME: u16 = 0xBC00; // 23:32:00
const DOS_DATE: u16 = 0x2198; // 1996-12-24
/// General purpose flags: 2 (deflate at maximum compression), and 0x800 for UTF-8 names.
const MAX_COMPRESSION: u16 = 2;
const UTF8: u16 = 0x800;

/// What goes in: a member's name and where its bytes come from.
pub struct Entry {
    pub name: String,
    pub size: u64,
    pub open: Box<dyn FnOnce() -> io::Result<Box<dyn Read>>>,
}

/// TorrentZip's order: by lower-case name (then as written, so the order is total).
pub fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
}

struct Counting<W: Write> {
    inner: W,
    n: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let k = self.inner.write(buf)?;
        self.n += k as u64;
        Ok(k)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Reads from `r`, compressing what passes into `w`.
struct Tee<'a, W: Write> {
    r: &'a mut dyn Read,
    w: &'a mut W,
}

impl<W: Write> Read for Tee<'_, W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.r.read(buf)?;
        self.w.write_all(&buf[..n])?;
        Ok(n)
    }
}

fn too_big(what: &str) -> io::Error {
    io::Error::other(format!("{what} is too big for a ZIP this version writes (4 GB a member, 65,535 members)"))
}

/// Write `entries` to a new ZIP at `path` (replacing what's there). Returns each
/// member's name and hashes, in the ZIP's order.
pub fn write(path: &Path, mut entries: Vec<Entry>, cancel: &AtomicBool) -> io::Result<Vec<(String, Hashes)>> {
    if entries.len() >= 0xFFFF {
        return Err(too_big("A game with this many files"));
    }
    sort(&mut entries);
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let mut central: Vec<u8> = vec![];
    let mut hashes = vec![];
    let mut offset: u64 = 0;
    for e in entries {
        if e.size >= 0xFFFF_FFFF {
            return Err(too_big(&e.name));
        }
        let name = e.name.as_bytes();
        let flags = MAX_COMPRESSION | if e.name.is_ascii() { 0 } else { UTF8 };
        // the local header, its CRC and sizes filled in once the member is written
        let header_at = offset;
        let mut local = Vec::with_capacity(30 + name.len());
        local.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        local.extend_from_slice(&20u16.to_le_bytes());
        local.extend_from_slice(&flags.to_le_bytes());
        local.extend_from_slice(&8u16.to_le_bytes());
        local.extend_from_slice(&DOS_TIME.to_le_bytes());
        local.extend_from_slice(&DOS_DATE.to_le_bytes());
        local.extend_from_slice(&[0u8; 12]);
        local.extend_from_slice(&(name.len() as u16).to_le_bytes());
        local.extend_from_slice(&0u16.to_le_bytes());
        local.extend_from_slice(name);
        out.write_all(&local)?;
        let mut enc = DeflateEncoder::new(Counting { inner: &mut out, n: 0 }, Compression::new(9));
        let mut src = (e.open)()?;
        let h = hash::hash_reader(&mut Tee { r: &mut src, w: &mut enc }, e.size, &e.name, cancel)?;
        let counting = enc.finish()?;
        let csize = counting.n;
        if h.size != e.size {
            return Err(io::Error::other(format!("{} changed while it was read ({} bytes, not {})", e.name, h.size, e.size)));
        }
        if csize >= 0xFFFF_FFFF || offset >= 0xFFFF_FFFF {
            return Err(too_big(&e.name));
        }
        let crc = u32::from_str_radix(&h.crc32, 16).unwrap_or(0);
        let mut sizes = Vec::with_capacity(12);
        sizes.extend_from_slice(&crc.to_le_bytes());
        sizes.extend_from_slice(&(csize as u32).to_le_bytes());
        sizes.extend_from_slice(&(h.size as u32).to_le_bytes());
        out.flush()?;
        {
            let f = out.get_mut();
            f.seek(SeekFrom::Start(header_at + 14))?;
            f.write_all(&sizes)?;
            f.seek(SeekFrom::End(0))?;
        }
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // made by: MS-DOS
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&flags.to_le_bytes());
        central.extend_from_slice(&8u16.to_le_bytes());
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&sizes);
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&(header_at as u32).to_le_bytes());
        central.extend_from_slice(name);
        offset = header_at + local.len() as u64 + csize;
        hashes.push((e.name, h));
    }
    if offset >= 0xFFFF_FFFF {
        return Err(too_big("This game"));
    }
    let comment = format!("TORRENTZIPPED-{:08X}", crc32fast::hash(&central));
    out.write_all(&central)?;
    let mut end = Vec::with_capacity(22 + comment.len());
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&[0u8; 4]); // this disk, the disk the directory starts on
    end.extend_from_slice(&(hashes.len() as u16).to_le_bytes());
    end.extend_from_slice(&(hashes.len() as u16).to_le_bytes());
    end.extend_from_slice(&(central.len() as u32).to_le_bytes());
    end.extend_from_slice(&(offset as u32).to_le_bytes());
    end.extend_from_slice(&(comment.len() as u16).to_le_bytes());
    end.extend_from_slice(comment.as_bytes());
    out.write_all(&end)?;
    out.flush()?;
    out.get_ref().sync_all()?;
    Ok(hashes)
}

/// Whether the ZIP at `path` is already a TorrentZip: its comment says so and is right.
pub fn is_torrentzip(path: &Path) -> bool {
    (|| -> io::Result<bool> {
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        if len < 22 + 22 {
            return Ok(false);
        }
        let mut end = [0u8; 44];
        f.seek(SeekFrom::Start(len - 44))?;
        f.read_exact(&mut end)?;
        if end[0..4] != 0x0605_4b50u32.to_le_bytes() || &end[22..36] != b"TORRENTZIPPED-" {
            return Ok(false);
        }
        let cd_size = u32::from_le_bytes(end[12..16].try_into().unwrap()) as u64;
        let cd_at = u32::from_le_bytes(end[16..20].try_into().unwrap()) as u64;
        if cd_at + cd_size + 44 != len {
            return Ok(false);
        }
        let mut cd = vec![0u8; cd_size as usize];
        f.seek(SeekFrom::Start(cd_at))?;
        f.read_exact(&mut cd)?;
        Ok(format!("{:08X}", crc32fast::hash(&cd)).as_bytes() == &end[36..44])
    })()
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::tests::rom;
    use crate::library::tests::temp_dir;

    fn entry(name: &str, data: Vec<u8>) -> Entry {
        Entry { name: name.into(), size: data.len() as u64, open: Box::new(move || Ok(Box::new(io::Cursor::new(data)) as Box<dyn Read>)) }
    }

    #[test]
    fn the_same_bytes_as_torrentzip() {
        let dir = temp_dir("torrentzip");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Game (USA).zip");
        let h = write(&p, vec![entry("b (Track 2).bin", rom(2, 70_000)), entry("A (Track 1).bin", rom(1, 100_000)), entry("empty.txt", vec![])], &AtomicBool::new(false)).unwrap();
        assert_eq!(h.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["A (Track 1).bin", "b (Track 2).bin", "empty.txt"]);
        assert!(is_torrentzip(&p));
        // the members read back as written
        let back = crate::archive::members(&p, &AtomicBool::new(false)).unwrap();
        assert_eq!(back.iter().map(|(n, h)| (n.clone(), h.sha1.clone())).collect::<Vec<_>>(), h.iter().map(|(n, h)| (n.clone(), h.sha1.clone())).collect::<Vec<_>>());
        // the SHA1 of this ZIP as written by tests/fixtures/torrentzip.py (Python's zlib)
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(hex::encode(<sha1::Sha1 as sha1::Digest>::digest(&bytes)), TORRENTZIP_SHA1, "{} bytes", bytes.len());
        // written twice, the same bytes
        let q = dir.join("again.zip");
        write(&q, vec![entry("empty.txt", vec![]), entry("A (Track 1).bin", rom(1, 100_000)), entry("b (Track 2).bin", rom(2, 70_000))], &AtomicBool::new(false)).unwrap();
        assert_eq!(std::fs::read(&q).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    const TORRENTZIP_SHA1: &str = "2e7b6fe66f2b2c4ae194dd07736d093065700387";
}
