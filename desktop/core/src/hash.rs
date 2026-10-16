//! Hashing a file in one read: CRC32, MD5 and SHA1 together, and, for a file with a
//! header DATs leave out (iNES, FDS, A78, Lynx, SNES copier) or an N64 ROM in another
//! byte order, a second set for the data as the DATs list it (docs/PLAN.md, "Phase 1
//! design", Headers).

use md5::{Digest, Md5};
use sha1::Sha1;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files are read this much at a time (a multiple of 4, for N64 byte orders).
pub const BLOCK: usize = 1 << 20;

/// What a file's bytes hash to, as lowercase hex (as DATs write them).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hashes {
    pub size: u64,
    pub crc32: String,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    /// The header found ("ines", "fds", "a78", "lynx", "copier", "n64-v64", "n64-le"), if any
    pub header: Option<String>,
    /// The hashes without the header (or in N64's big-endian order)
    pub hl: Option<Box<Hashes>>,
}

impl Hashes {
    /// Only the CRC32 and size a ZIP records for a member (the quick scan).
    pub fn quick(size: u64, crc32: u32) -> Self {
        Self {
            size,
            crc32: format!("{crc32:08x}"),
            ..Default::default()
        }
    }
}

/// How a header changes what is hashed the second time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Header {
    /// The first n bytes aren't part of the ROM.
    Skip(&'static str, usize),
    /// An N64 ROM in byte-swapped (v64) order.
    N64Swapped,
    /// An N64 ROM in little-endian (n64) order.
    N64Little,
}

impl Header {
    fn name(self) -> &'static str {
        match self {
            Header::Skip(n, _) => n,
            Header::N64Swapped => "n64-v64",
            Header::N64Little => "n64-le",
        }
    }
}

/// Extensions a SNES copier header is looked for on (the rule is by size, so it's kept to SNES files).
const SNES_EXTS: &[&str] = &["sfc", "smc", "swc", "fig"];

/// The header the start of a file shows, given its size and extension.
pub fn detect(first: &[u8], size: u64, ext: &str) -> Option<Header> {
    let at = |off: usize, magic: &[u8]| first.len() >= off + magic.len() && &first[off..off + magic.len()] == magic;
    let bigger = |n: u64| size > n;
    if at(0, b"NES\x1a") && bigger(16) {
        return Some(Header::Skip("ines", 16));
    }
    if at(0, b"FDS\x1a") && bigger(16) {
        return Some(Header::Skip("fds", 16));
    }
    if at(1, b"ATARI7800") && bigger(128) {
        return Some(Header::Skip("a78", 128));
    }
    if at(0, b"LYNX") && bigger(64) {
        return Some(Header::Skip("lynx", 64));
    }
    if at(0, &[0x37, 0x80, 0x40, 0x12]) && size.is_multiple_of(4) {
        return Some(Header::N64Swapped);
    }
    if at(0, &[0x40, 0x12, 0x37, 0x80]) && size.is_multiple_of(4) {
        return Some(Header::N64Little);
    }
    if SNES_EXTS.contains(&ext) && size > 512 && size % 1024 == 512 {
        return Some(Header::Skip("copier", 512));
    }
    None
}

struct Digests {
    crc: crc32fast::Hasher,
    md5: Md5,
    sha1: Sha1,
    size: u64,
}

impl Digests {
    fn new() -> Self {
        Self {
            crc: crc32fast::Hasher::new(),
            md5: Md5::new(),
            sha1: Sha1::new(),
            size: 0,
        }
    }
    fn update(&mut self, b: &[u8]) {
        self.crc.update(b);
        self.md5.update(b);
        self.sha1.update(b);
        self.size += b.len() as u64;
    }
    fn finish(self) -> Hashes {
        Hashes {
            size: self.size,
            crc32: format!("{:08x}", self.crc.finalize()),
            md5: Some(hex::encode(self.md5.finalize())),
            sha1: Some(hex::encode(self.sha1.finalize())),
            header: None,
            hl: None,
        }
    }
}

/// Fill `buf` from `r` (short only at the end of the file).
fn fill(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Hash everything `r` gives (a file of `size` bytes named `name`), stopping with an
/// error of kind `Interrupted` if `cancel` is set.
pub fn hash_reader(r: &mut impl Read, size: u64, name: &str, cancel: &AtomicBool) -> io::Result<Hashes> {
    let ext = crate::platforms::ext_of(name);
    let mut buf = vec![0u8; BLOCK.min(size.max(4096) as usize).next_multiple_of(4)];
    let mut all = Digests::new();
    let mut hl: Option<(Header, Digests)> = None;
    let mut first = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        let n = fill(r, &mut buf)?;
        if n == 0 {
            break;
        }
        let block = &mut buf[..n];
        all.update(block);
        if first {
            first = false;
            if let Some(h) = detect(block, size, &ext) {
                hl = Some((h, Digests::new()));
            }
            if let Some((Header::Skip(_, skip), d)) = hl.as_mut() {
                d.update(&block[(*skip).min(n)..]);
                continue;
            }
        }
        match hl.as_mut() {
            Some((Header::Skip(..), d)) => d.update(block),
            Some((Header::N64Swapped, d)) => {
                for w in block.chunks_exact_mut(2) {
                    w.swap(0, 1);
                }
                d.update(block);
            }
            Some((Header::N64Little, d)) => {
                for w in block.chunks_exact_mut(4) {
                    w.reverse();
                }
                d.update(block);
            }
            None => {}
        }
        if n < buf.len() {
            break;
        }
    }
    let mut out = all.finish();
    if let Some((h, d)) = hl {
        out.header = Some(h.name().to_string());
        out.hl = Some(Box::new(d.finish()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(bytes: &[u8], name: &str) -> Hashes {
        hash_reader(&mut &bytes[..], bytes.len() as u64, name, &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn known_hashes() {
        let x = h(b"The quick brown fox jumps over the lazy dog", "fox.txt");
        assert_eq!(x.crc32, "414fa339");
        assert_eq!(x.md5.as_deref(), Some("9e107d9d372bb6826bd81d3542a419d6"));
        assert_eq!(x.sha1.as_deref(), Some("2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"));
        assert!(x.header.is_none() && x.hl.is_none());
        let empty = h(b"", "empty.gb");
        assert_eq!((empty.size, empty.crc32.as_str()), (0, "00000000"));
    }

    #[test]
    fn headers_get_a_second_set_of_hashes() {
        let rom: Vec<u8> = (0..40_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let plain = h(&rom, "x.nes");
        let mut ines = b"NES\x1a\x02\x01\0\0\0\0\0\0\0\0\0\0".to_vec();
        ines.extend_from_slice(&rom);
        let with = h(&ines, "x.nes");
        assert_eq!(with.header.as_deref(), Some("ines"));
        assert_eq!(with.hl.as_deref().unwrap().sha1, plain.sha1);
        assert_ne!(with.sha1, plain.sha1);

        let mut a78 = vec![0u8; 128];
        a78[1..10].copy_from_slice(b"ATARI7800");
        a78.extend_from_slice(&rom);
        assert_eq!(h(&a78, "x.a78").hl.unwrap().crc32, plain.crc32);

        let snes: Vec<u8> = (0..2048u32).map(|i| i as u8).collect();
        let mut copier = vec![0u8; 512];
        copier.extend_from_slice(&snes);
        assert_eq!(h(&copier, "x.smc").hl.unwrap().md5, h(&snes, "x.sfc").md5);
        assert!(h(&copier, "x.bin").hl.is_none(), "the copier rule is only for SNES files");
        assert!(h(&snes, "x.sfc").hl.is_none());
    }

    #[test]
    fn n64_byte_orders_hash_as_big_endian() {
        // a z64 ROM larger than a block, so the swap crosses block boundaries
        let mut z64 = vec![0x80, 0x37, 0x12, 0x40];
        z64.extend((0..(BLOCK as u32 + 4096)).map(|i| (i % 253) as u8));
        let big = h(&z64, "x.z64");
        assert!(big.header.is_none());
        let v64: Vec<u8> = z64.chunks(2).flat_map(|w| [w[1], w[0]]).collect();
        let le: Vec<u8> = z64.chunks(4).flat_map(|w| [w[3], w[2], w[1], w[0]]).collect();
        let sv = h(&v64, "x.v64");
        assert_eq!(sv.header.as_deref(), Some("n64-v64"));
        assert_eq!(sv.hl.unwrap().sha1, big.sha1);
        let sl = h(&le, "x.n64");
        assert_eq!(sl.header.as_deref(), Some("n64-le"));
        assert_eq!(sl.hl.unwrap().sha1, big.sha1);
    }

    #[test]
    fn stops_when_asked() {
        let data = vec![0u8; 3 * BLOCK];
        let stop = AtomicBool::new(true);
        let e = hash_reader(&mut &data[..], data.len() as u64, "x", &stop).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::Interrupted);
    }
}
