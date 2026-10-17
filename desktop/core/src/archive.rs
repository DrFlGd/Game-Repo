//! The archives a scan looks inside (docs/PLAN.md, "Phase 2 design", 7z and RAR):
//! ZIP, 7z and RAR. Each records its members' CRC32s, so the quick scan (D16) lists
//! them without unpacking; the full scan unpacks each member as it goes and hashes
//! it. Members of a password-protected archive can't be read: the archive is
//! *locked* and its members are left out. Model Library's `archive.rs` is where the
//! 7z and RAR reading comes from.

use crate::hash::{self, Hashes};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The kinds of archive, as `files.kind` names them (the extension, lower case).
pub const KINDS: &[&str] = &["zip", "7z", "rar"];

/// `kind IN (…)` for SQL: the files that are archives.
pub const SQL_KINDS: &str = "('zip', '7z', 'rar')";

/// The archive kind of a file name, if it's one.
pub fn kind_of(name: &str) -> Option<&'static str> {
    let ext = crate::platforms::ext_of(name);
    KINDS.iter().copied().find(|k| *k == ext)
}

/// The kind of a file a scan looks inside: an archive or a disc image ([`crate::image`]).
pub fn container_of(name: &str) -> Option<&'static str> {
    kind_of(name).or_else(|| crate::image::kind_of(name))
}

/// A member of an archive, hashed in full or known by the CRC32 the archive records.
pub struct Member {
    pub name: String,
    pub hashes: Hashes,
    pub full: bool,
}

/// What reading an archive found.
pub struct Listing {
    pub members: Vec<Member>,
    /// Some members (or the whole archive) need a password: they're left out.
    pub locked: bool,
}

fn stopped() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "stopped")
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Names inside archives with `/`, and the junk some tools add left out.
fn member_name(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let base = name.rsplit('/').next().unwrap_or(&name);
    if name.starts_with("__MACOSX/") || base.starts_with("._") || base.eq_ignore_ascii_case(".DS_Store") || base.is_empty() {
        return None;
    }
    Some(name)
}

/// The members of the archive at `path` of `kind`: by their recorded CRC32s (quick),
/// or unpacked and hashed (`full`). A ZIP whose central directory is damaged is read
/// from its local headers, in full.
pub fn read(path: &Path, kind: &str, full: bool, cancel: &AtomicBool) -> io::Result<Listing> {
    match kind {
        "chd" | "cso" | "zso" | "rvz" => crate::image::read(path, kind, cancel),
        "7z" => read_7z(path, full, cancel),
        "rar" => read_rar(path, full, cancel),
        _ => read_zip(path, full, cancel),
    }
}

/// Every member of an archive (or disc image), unpacked and hashed: (name, hashes).
pub fn members(path: &Path, cancel: &AtomicBool) -> io::Result<Vec<(String, Hashes)>> {
    let kind = container_of(&path.to_string_lossy()).unwrap_or("zip");
    Ok(read(path, kind, true, cancel)?.members.into_iter().map(|m| (m.name, m.hashes)).collect())
}

// ------------------------------------------------------------ ZIP

fn read_zip(path: &Path, full: bool, cancel: &AtomicBool) -> io::Result<Listing> {
    let f = io::BufReader::new(std::fs::File::open(path)?);
    let mut zip = match zip::ZipArchive::new(f) {
        Ok(z) => z,
        Err(_) => return read_zip_stream(path, cancel),
    };
    let mut out = Listing { members: vec![], locked: false };
    for i in 0..zip.len() {
        if cancel.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        let raw = zip.by_index_raw(i).map_err(other)?;
        if raw.is_dir() {
            continue;
        }
        let Some(name) = member_name(&raw.name().map_err(other)?) else { continue };
        if raw.encrypted() {
            out.locked = true;
            continue;
        }
        let (size, crc) = (raw.size(), raw.crc32());
        drop(raw);
        if full {
            let mut m = zip.by_index(i).map_err(other)?;
            let hashes = hash::hash_reader(&mut m, size, &name, cancel)?;
            out.members.push(Member { name, hashes, full: true });
        } else {
            out.members.push(Member { hashes: Hashes::quick(size, crc), name, full: false });
        }
    }
    Ok(out)
}

/// A ZIP read from its local headers, when its central directory is damaged.
fn read_zip_stream(path: &Path, cancel: &AtomicBool) -> io::Result<Listing> {
    let mut f = io::BufReader::new(std::fs::File::open(path)?);
    let mut out = Listing { members: vec![], locked: false };
    while let Some(mut m) = zip::read::read_zipfile_from_stream(&mut f).map_err(other)? {
        if m.is_dir() {
            continue;
        }
        let Some(name) = member_name(&m.name().map_err(other)?) else { continue };
        let size = m.size();
        let hashes = hash::hash_reader(&mut m, size, &name, cancel)?;
        out.members.push(Member { name, hashes, full: true });
    }
    if out.members.is_empty() {
        return Err(other("not a ZIP that can be read"));
    }
    Ok(out)
}

// ------------------------------------------------------------ 7z

fn seven_error(e: sevenz_rust2::Error) -> io::Error {
    match e {
        sevenz_rust2::Error::Io(e, _) => e,
        e => other(e),
    }
}

fn is_locked_7z(e: &sevenz_rust2::Error) -> bool {
    matches!(e, sevenz_rust2::Error::PasswordRequired | sevenz_rust2::Error::MaybeBadPassword(_))
}

fn read_7z(path: &Path, full: bool, cancel: &AtomicBool) -> io::Result<Listing> {
    let mut out = Listing { members: vec![], locked: false };
    if !full {
        let a = match sevenz_rust2::Archive::open(path) {
            Ok(a) => a,
            Err(e) if is_locked_7z(&e) => return Ok(Listing { members: vec![], locked: true }),
            Err(e) => return Err(seven_error(e)),
        };
        for (i, e) in a.files.iter().enumerate() {
            if e.is_directory() || e.is_anti_item() {
                continue;
            }
            let Some(name) = member_name(e.name()) else { continue };
            let encrypted = a.stream_map.file_block_index.get(i).copied().flatten().and_then(|b| a.blocks.get(b))
                .is_some_and(|b| b.coders.iter().any(|c| c.encoder_method_id() == sevenz_rust2::EncoderMethod::ID_AES256_SHA256));
            if encrypted {
                out.locked = true;
                continue;
            }
            if e.has_crc || e.size() == 0 {
                out.members.push(Member { hashes: Hashes::quick(e.size(), e.crc as u32), name, full: false });
            } else {
                // a member without a recorded CRC: the whole archive is read instead
                return read_7z(path, true, cancel);
            }
        }
        return Ok(out);
    }
    let mut r = match sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()) {
        Ok(r) => r,
        Err(e) if is_locked_7z(&e) => return Ok(Listing { members: vec![], locked: true }),
        Err(e) => return Err(seven_error(e)),
    };
    let mut failed: Option<io::Error> = None;
    let result = r.for_each_entries(|e, data| {
        if e.is_directory() || e.is_anti_item() {
            return Ok(true);
        }
        if cancel.load(Ordering::Relaxed) {
            failed = Some(stopped());
            return Ok(false);
        }
        let Some(name) = member_name(e.name()) else {
            io::copy(data, &mut io::sink())?;
            return Ok(true);
        };
        match hash::hash_reader(data, e.size(), &name, cancel) {
            Ok(hashes) => {
                out.members.push(Member { name, hashes, full: true });
                Ok(true)
            }
            Err(err) => {
                failed = Some(err);
                Ok(false)
            }
        }
    });
    if let Some(e) = failed {
        return Err(e);
    }
    match result {
        Ok(()) => Ok(out),
        Err(e) if is_locked_7z(&e) => Ok(Listing { members: out.members, locked: true }),
        Err(e) => Err(seven_error(e)),
    }
}

// ------------------------------------------------------------ RAR

fn rar_name(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// A scratch folder for RAR members (unrar writes files, so each goes through one).
fn scratch() -> PathBuf {
    std::env::temp_dir().join(format!("gamerepo-rar-{}", std::process::id()))
}

fn read_rar(path: &Path, full: bool, cancel: &AtomicBool) -> io::Result<Listing> {
    let mut out = Listing { members: vec![], locked: false };
    if !full {
        let listing = unrar::Archive::new(path).open_for_listing().map_err(other)?;
        for e in listing {
            if cancel.load(Ordering::Relaxed) {
                return Err(stopped());
            }
            let e = e.map_err(other)?;
            if e.is_directory() || e.is_split_before() {
                continue;
            }
            let Some(name) = member_name(&rar_name(&e.filename)) else { continue };
            if e.is_encrypted() {
                out.locked = true;
                continue;
            }
            out.members.push(Member { hashes: Hashes::quick(e.unpacked_size, e.file_crc), name, full: false });
        }
        return Ok(out);
    }
    let mut at = unrar::Archive::new(path).open_for_processing().map_err(other)?;
    let dir = scratch();
    std::fs::create_dir_all(&dir)?;
    let result = (|| -> io::Result<()> {
        while let Some(h) = at.read_header().map_err(other)? {
            if cancel.load(Ordering::Relaxed) {
                return Err(stopped());
            }
            let e = h.entry();
            let name = member_name(&rar_name(&e.filename));
            if e.is_directory() || e.is_encrypted() || name.is_none() {
                out.locked |= e.is_encrypted();
                at = h.skip().map_err(other)?;
                continue;
            }
            let (name, size) = (name.unwrap_or_default(), e.unpacked_size);
            let tmp = dir.join("member");
            at = h.extract_to(&tmp).map_err(other)?;
            let mut f = std::fs::File::open(&tmp)?;
            let hashes = hash::hash_reader(&mut f, size, &name, cancel)?;
            drop(f);
            let _ = std::fs::remove_file(&tmp);
            out.members.push(Member { name, hashes, full: true });
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.map(|_| out)
}

/// Each member of an archive in turn, unpacked, with its bytes to read: `f(name, size, reader)`.
pub fn each(path: &Path, cancel: &AtomicBool, f: &mut dyn FnMut(&str, u64, &mut dyn Read) -> io::Result<()>) -> io::Result<()> {
    each_wanted(path, &|_| true, cancel, f)
}

/// The same, for the members `want` names (others are skipped, or read past in a ZIP, 7z or RAR).
pub fn each_wanted(path: &Path, want: &dyn Fn(&str) -> bool, cancel: &AtomicBool, f: &mut dyn FnMut(&str, u64, &mut dyn Read) -> io::Result<()>) -> io::Result<()> {
    let mut f = |name: &str, size: u64, r: &mut dyn Read| -> io::Result<()> {
        if want(name) {
            f(name, size, r)
        } else {
            io::copy(r, &mut io::sink()).map(|_| ())
        }
    };
    let f = &mut f;
    match container_of(&path.to_string_lossy()).unwrap_or("zip") {
        kind @ ("chd" | "cso" | "zso" | "rvz") => crate::image::each_wanted(path, kind, want, cancel, f),
        "7z" => {
            let mut r = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(seven_error)?;
            let mut failed: Option<io::Error> = None;
            let result = r.for_each_entries(|e, data| {
                if e.is_directory() || e.is_anti_item() {
                    return Ok(true);
                }
                let Some(name) = member_name(e.name()) else {
                    io::copy(data, &mut io::sink())?;
                    return Ok(true);
                };
                let r = if cancel.load(Ordering::Relaxed) { Err(stopped()) } else { f(&name, e.size(), data) };
                match r {
                    Ok(()) => Ok(true),
                    Err(err) => {
                        failed = Some(err);
                        Ok(false)
                    }
                }
            });
            if let Some(e) = failed {
                return Err(e);
            }
            result.map_err(seven_error)
        }
        "rar" => {
            let mut at = unrar::Archive::new(path).open_for_processing().map_err(other)?;
            let dir = scratch();
            std::fs::create_dir_all(&dir)?;
            let result = (|| -> io::Result<()> {
                while let Some(h) = at.read_header().map_err(other)? {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(stopped());
                    }
                    let e = h.entry();
                    let name = member_name(&rar_name(&e.filename));
                    if e.is_directory() || e.is_encrypted() || name.is_none() {
                        at = h.skip().map_err(other)?;
                        continue;
                    }
                    let (name, size) = (name.unwrap_or_default(), e.unpacked_size);
                    let tmp = dir.join("member");
                    at = h.extract_to(&tmp).map_err(other)?;
                    let mut file = std::fs::File::open(&tmp)?;
                    f(&name, size, &mut file)?;
                    drop(file);
                    let _ = std::fs::remove_file(&tmp);
                }
                Ok(())
            })();
            let _ = std::fs::remove_dir_all(&dir);
            result
        }
        _ => {
            let mut z = zip::ZipArchive::new(io::BufReader::new(std::fs::File::open(path)?)).map_err(other)?;
            for i in 0..z.len() {
                if cancel.load(Ordering::Relaxed) {
                    return Err(stopped());
                }
                let mut e = z.by_index(i).map_err(other)?;
                if e.is_dir() {
                    continue;
                }
                let Some(name) = member_name(&e.name().map_err(other)?) else { continue };
                let size = e.size();
                f(&name, size, &mut e)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::library::tests::temp_dir;

    /// A 7z of these members, written with sevenz-rust2 (LZMA2).
    pub fn write_7z(path: &Path, members: &[(&str, &[u8])]) {
        let mut w = sevenz_rust2::ArchiveWriter::create(path).unwrap();
        for (name, data) in members {
            w.push_archive_entry(sevenz_rust2::ArchiveEntry::new_file(name), Some(*data)).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn a_7z_is_read_quick_and_full() {
        let dir = temp_dir("archive-7z");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("Set.7z");
        let a = crate::audit::tests::rom(1, 5000);
        let b = crate::audit::tests::rom(2, 3000);
        write_7z(&p, &[("One (USA).gb", &a), ("Two (USA).gb", &b)]);
        let stop = AtomicBool::new(false);
        let quick = read(&p, "7z", false, &stop).unwrap();
        let full = read(&p, "7z", true, &stop).unwrap();
        assert_eq!(quick.members.len(), 2);
        for (q, f) in quick.members.iter().zip(&full.members) {
            assert_eq!((&q.name, q.hashes.crc32.as_str(), q.hashes.size), (&f.name, f.hashes.crc32.as_str(), f.hashes.size));
            assert!(!q.full && f.full && f.hashes.sha1.is_some());
        }
        let mut seen = vec![];
        each(&p, &stop, &mut |n, size, r| {
            let mut v = vec![];
            r.read_to_end(&mut v)?;
            assert_eq!(v.len() as u64, size);
            seen.push(n.to_string());
            Ok(())
        })
        .unwrap();
        assert_eq!(seen, ["One (USA).gb", "Two (USA).gb"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rar_is_read_quick_and_full() {
        // from the unrar crate's test data (tests/fixtures/README.md): one member, VERSION
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/version.rar");
        let stop = AtomicBool::new(false);
        let quick = read(&p, "rar", false, &stop).unwrap();
        let full = read(&p, "rar", true, &stop).unwrap();
        assert_eq!(quick.members.len(), 1);
        assert_eq!(full.members[0].name, "VERSION");
        assert_eq!(quick.members[0].hashes.crc32, full.members[0].hashes.crc32);
        assert_eq!(full.members[0].hashes.crc32, format!("{:08x}", crc32fast::hash(b"unrar-0.4.0")));
    }
}
