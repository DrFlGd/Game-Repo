//! Writing a pack file of guidebooks: one game's guidebooks, each a PDF or
//! a CBZ file with the text of its pages, in a pack file of their own
//! (version 8 of the format), so a large pack of maps is never written
//! again to add one. See [`write_guidebook_pack`].

use std::{
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Seek, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::json;
use zip::{write::SimpleFileOptions, CompressionMethod, DateTime, ZipArchive, ZipWriter};

use crate::{
    guidebook::{guidebook_name_of, text_name_of, GUIDEBOOK_LIST_FORMAT},
    is_pack_id, pdf, read_header, AppInfo, Counts, Detail, GameFacts, GuidebookFacts,
    GuidebookList, GuidebookText, GuidebookType, Header, ImagesFacts, Origin, PackFacts,
    SourceFacts, SourceKind, FORMAT, GUIDEBOOK_LIST_FILE, HEADER_FILE, MAX_GUIDEBOOKS,
    MAX_GUIDEBOOK_BYTES, MAX_GUIDEBOOK_PAGES, MAX_GUIDEBOOK_TEXT_BYTES, MAX_GUIDEBOOK_TITLE,
    VERSION_WITH_GUIDEBOOKS,
};

/// The most entries a CBZ file may have, pictures or not.
pub const MAX_CBZ_PICTURES: usize = 20_000;
/// The pack's manifest, as a pack folder has it.
const MANIFEST_FILE: &str = "manifest.json";
/// A ZIP entry larger than this needs ZIP64 records.
const ZIP32_LIMIT: u64 = 0xFFFF_FFFF;

/// Why a guidebook pack was not written: one or two plain sentences, to
/// show as they are (`Display`, or `String::from`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteError(String);

impl WriteError {
    fn new(what: impl Into<String>) -> Self {
        Self(what.into())
    }

    /// The sentence.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for WriteError {}

impl From<WriteError> for String {
    fn from(error: WriteError) -> Self {
        error.0
    }
}

/// A guidebook to put in a pack: its file on disk, its title, and what is
/// known of its pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewGuidebook {
    /// The PDF (`.pdf`) or CBZ (`.cbz`) file, by its name's extension.
    pub file: PathBuf,
    /// Its title, as it is shown: 1 to [`MAX_GUIDEBOOK_TITLE`] characters.
    pub title: String,
    /// How many pages it has. None: read from the file, for a PDF from its
    /// page tree (see [`guidebook_pages`]), for a CBZ its pictures.
    pub pages: Option<u32>,
    /// The text of each page, the first being page 1; none when it is not
    /// known. A page whose text is only space is left out of the file.
    pub text: Option<Vec<String>>,
}

/// A pack of one game's guidebooks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuidebookPack {
    /// The pack's id (see [`is_pack_id`]), such as `guidebooks-<game id>`.
    /// A pack is known by it: write the same game's guidebooks under the
    /// same id, so the new file replaces the old one.
    pub pack_id: String,
    /// The game's title, which an app shows the guidebooks with: the
    /// title of the game they are for.
    pub game_title: String,
    /// The app that writes the file, written as the pack's `writtenBy` and
    /// the file's `exportedBy`. An app tells the packs it wrote by it.
    pub written_by: AppInfo,
    /// The guidebooks, in the order they are listed.
    pub guidebooks: Vec<NewGuidebook>,
}

/// Writes a complete pack file of a game's guidebooks at `target`, through
/// `<target>.part` beside it, renamed to `target` when it is complete (so
/// an app that watches the folder never reads half a file, and a file
/// already at `target` is replaced whole). Returns its description.
///
/// The file is version 8 of the format, with `source.origin`
/// `guidebooks`. Each guidebook is stored as it is, as
/// `guidebooks/<name>.pdf` or `.cbz`, and the text of its pages, when
/// given, as `guidebooks/<name>.text.json`; the name is made from its
/// title. The pack also has a `manifest.json` and a `guidebooks.json`
/// listing the guidebooks, so it reads as a pack unpacked into a folder
/// too.
///
/// Refuses, with a plain sentence and nothing written: a pack id that
/// cannot be one, an empty game title, no guidebooks or more than
/// [`MAX_GUIDEBOOKS`], a file that is not a PDF or a CBZ by its name and
/// its first bytes, a file over [`MAX_GUIDEBOOK_BYTES`], a title that is
/// empty or too long, a PDF whose pages cannot be counted and were not
/// given, a CBZ with no pictures, text of more pages than the guidebook
/// has, and text over [`MAX_GUIDEBOOK_TEXT_BYTES`].
pub fn write_guidebook_pack(pack: &GuidebookPack, target: &Path) -> Result<Header, WriteError> {
    write_pack(pack, None, target)
}

/// Writes the guidebook pack `existing` again at `target` (which may be
/// `existing` itself), with the guidebooks it holds that `keep` keeps and
/// those of `pack` after them, as [`write_guidebook_pack`] writes one. The
/// kept guidebooks and their text are copied as they are. `pack.pack_id`
/// must be the existing pack's id, and the pack's version goes up by one.
/// Used to add a guidebook to a game's pack, or to take one out.
pub fn rewrite_guidebook_pack(
    existing: &Path,
    keep: &dyn Fn(&GuidebookFacts) -> bool,
    pack: &GuidebookPack,
    target: &Path,
) -> Result<Header, WriteError> {
    let header = read_header(existing).map_err(|refusal| WriteError::new(refusal.to_string()))?;
    if header.guidebooks.is_empty() || header.pack_id != pack.pack_id {
        return Err(WriteError::new(
            "This pack file is not the pack of guidebooks of this game.",
        ));
    }
    write_pack(pack, Some((existing, &header, keep)), target)
}

/// How many pages a guidebook file has: for a PDF, the count of its page
/// tree, read without rendering anything; for a CBZ, its pictures. Refuses
/// a file of another type, and one whose pages cannot be counted.
pub fn guidebook_pages(file: &Path) -> Result<u32, WriteError> {
    let file_type = type_of(file)?;
    let mut opened = File::open(file).map_err(|error| cannot_read(file, &error))?;
    let len = opened
        .metadata()
        .map_err(|error| cannot_read(file, &error))?
        .len();
    check_start(file, &mut opened, file_type)?;
    let pages = match file_type {
        GuidebookType::Pdf => {
            pdf::pdf_pages(&mut BufReader::new(opened), len).ok_or_else(|| {
                WriteError::new(format!(
                    "The number of pages of {} could not be read from the file.",
                    shown(file)
                ))
            })?
        }
        GuidebookType::Cbz => cbz_pictures(file, opened)?,
    };
    if pages > MAX_GUIDEBOOK_PAGES {
        return Err(WriteError::new(format!(
            "{} has too many pages to be a guidebook.",
            shown(file)
        )));
    }
    Ok(pages)
}

/// The pictures of a CBZ file: its entries whose names end in a picture's
/// extension, leaving out folders, hidden files and a Mac's
/// `__MACOSX/` folder.
fn cbz_pictures(path: &Path, file: File) -> Result<u32, WriteError> {
    let damaged = || {
        WriteError::new(format!(
            "{} is not a CBZ file that can be read.",
            shown(path)
        ))
    };
    let archive = ZipArchive::new(BufReader::new(file)).map_err(|_| damaged())?;
    if archive.len() > MAX_CBZ_PICTURES {
        return Err(WriteError::new(format!(
            "{} holds too many files to be a guidebook.",
            shown(path)
        )));
    }
    let pictures = archive
        .file_names()
        .filter(|name| name.as_deref().is_ok_and(is_picture_entry))
        .count();
    if pictures == 0 {
        return Err(WriteError::new(format!(
            "{} holds no pictures of pages.",
            shown(path)
        )));
    }
    u32::try_from(pictures).map_err(|_| damaged())
}

/// Whether a CBZ entry is a picture of a page.
fn is_picture_entry(name: &str) -> bool {
    if name.ends_with('/') || name.starts_with("__MACOSX/") {
        return false;
    }
    let last = name.rsplit('/').next().unwrap_or(name);
    if last.starts_with('.') {
        return false;
    }
    let Some((_, extension)) = last.rsplit_once('.') else {
        return false;
    };
    ["jpg", "jpeg", "png", "webp", "gif", "avif"]
        .iter()
        .any(|known| extension.eq_ignore_ascii_case(known))
}

fn type_of(file: &Path) -> Result<GuidebookType, WriteError> {
    file.file_name()
        .and_then(|name| name.to_str())
        .and_then(GuidebookType::of_file_name)
        .ok_or_else(|| {
            WriteError::new(format!(
                "{} is not a PDF or CBZ file. Only PDF and CBZ files can be guidebooks.",
                shown(file)
            ))
        })
}

/// Checks that a file starts as its type does.
fn check_start(path: &Path, file: &mut File, file_type: GuidebookType) -> Result<(), WriteError> {
    let mut start = Vec::with_capacity(1024);
    Read::by_ref(file)
        .take(1024)
        .read_to_end(&mut start)
        .map_err(|error| cannot_read(path, &error))?;
    file.rewind().map_err(|error| cannot_read(path, &error))?;
    let fits = match file_type {
        GuidebookType::Pdf => pdf::looks_like_pdf(&start),
        GuidebookType::Cbz => start.starts_with(b"PK\x03\x04"),
    };
    if fits {
        Ok(())
    } else {
        Err(WriteError::new(format!(
            "{} is not the {} file its name says it is.",
            shown(path),
            match file_type {
                GuidebookType::Pdf => "PDF",
                GuidebookType::Cbz => "CBZ",
            }
        )))
    }
}

fn shown(file: &Path) -> String {
    file.file_name().map_or_else(
        || file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn cannot_read(file: &Path, error: &io::Error) -> WriteError {
    WriteError::new(format!("Could not read {}: {error}", shown(file)))
}

/// What goes into the file after the description, in order.
enum Source<'a> {
    Bytes(Vec<u8>),
    /// A new guidebook file, of this many bytes.
    File(&'a Path, u64),
    /// An entry of the existing pack, copied as it is.
    Kept(String),
}

struct Entry<'a> {
    name: String,
    deflate: bool,
    size: u64,
    source: Source<'a>,
}

type Existing<'a> = (&'a Path, &'a Header, &'a dyn Fn(&GuidebookFacts) -> bool);

fn write_pack(
    pack: &GuidebookPack,
    existing: Option<Existing<'_>>,
    target: &Path,
) -> Result<Header, WriteError> {
    if !is_pack_id(&pack.pack_id) {
        return Err(WriteError::new(
            "The id of a guidebook pack must be 1 to 64 lower case letters, digits, - and _.",
        ));
    }
    let game_title = pack.game_title.trim();
    if game_title.is_empty() || game_title.chars().count() > MAX_GUIDEBOOK_TITLE {
        return Err(WriteError::new(
            "The title of the game must be 1 to 200 characters.",
        ));
    }
    if pack.written_by.name.trim().is_empty() {
        return Err(WriteError::new(
            "The name of the app that writes the pack is missing.",
        ));
    }

    let mut list: Vec<GuidebookFacts> = Vec::new();
    let mut entries: Vec<Entry<'_>> = Vec::new();
    let mut kept_from: Option<ZipArchive<BufReader<File>>> = None;
    if let Some((path, header, keep)) = existing {
        let file = File::open(path).map_err(|error| cannot_read(path, &error))?;
        let mut archive = ZipArchive::new(BufReader::new(file)).map_err(|_| {
            WriteError::new("The pack file of guidebooks is damaged, so it was not changed.")
        })?;
        for facts in header.guidebooks.iter().filter(|facts| keep(facts)) {
            let names = std::iter::once(facts.file.clone()).chain(facts.text.clone());
            for name in names {
                let Some(index) = archive.index_for_name(&name) else {
                    return Err(WriteError::new(
                        "The pack file of guidebooks is missing a guidebook it lists, so it was not changed.",
                    ));
                };
                let size = archive
                    .by_index_raw(index)
                    .map(|entry| entry.size())
                    .unwrap_or(0);
                entries.push(Entry {
                    name: name.clone(),
                    deflate: false,
                    size,
                    source: Source::Kept(name),
                });
            }
            list.push(facts.clone());
        }
        // Read through the same archive below.
        kept_from = Some(archive);
    }

    for guidebook in &pack.guidebooks {
        let file_type = type_of(&guidebook.file)?;
        let title = guidebook.title.trim();
        if title.is_empty() || title.chars().count() > MAX_GUIDEBOOK_TITLE {
            return Err(WriteError::new(format!(
                "The title of a guidebook must be 1 to {MAX_GUIDEBOOK_TITLE} characters."
            )));
        }
        let mut opened =
            File::open(&guidebook.file).map_err(|error| cannot_read(&guidebook.file, &error))?;
        let bytes = opened
            .metadata()
            .map_err(|error| cannot_read(&guidebook.file, &error))?
            .len();
        if bytes == 0 {
            return Err(WriteError::new(format!(
                "{} is empty.",
                shown(&guidebook.file)
            )));
        }
        if bytes > MAX_GUIDEBOOK_BYTES {
            return Err(WriteError::new(format!(
                "{} is larger than 16 GB, the most a guidebook can be.",
                shown(&guidebook.file)
            )));
        }
        check_start(&guidebook.file, &mut opened, file_type)?;
        drop(opened);
        let pages = match guidebook.pages {
            Some(pages) if (1..=MAX_GUIDEBOOK_PAGES).contains(&pages) => pages,
            Some(_) => {
                return Err(WriteError::new(format!(
                    "The number of pages of {} must be 1 to {MAX_GUIDEBOOK_PAGES}.",
                    shown(&guidebook.file)
                )))
            }
            None => guidebook_pages(&guidebook.file)?,
        };
        let taken: Vec<&str> = list.iter().map(|facts| facts.file.as_str()).collect();
        let name = guidebook_name_of(title, file_type, &taken);
        let text = match &guidebook.text {
            Some(pages_text) => {
                if pages_text.len() > pages as usize {
                    return Err(WriteError::new(format!(
                        "The text given for {} has more pages than the guidebook.",
                        shown(&guidebook.file)
                    )));
                }
                let text = GuidebookText::of_pages(pages_text);
                if text.pages.is_empty() {
                    None
                } else {
                    let json = serde_json::to_vec(&text)
                        .map_err(|error| WriteError::new(error.to_string()))?;
                    if json.len() as u64 > MAX_GUIDEBOOK_TEXT_BYTES {
                        return Err(WriteError::new(format!(
                            "The text of {} is larger than 64 MB, the most a guidebook's text can be.",
                            shown(&guidebook.file)
                        )));
                    }
                    Some(json)
                }
            }
            None => None,
        };
        let text_name = text.as_ref().and_then(|_| text_name_of(&name));
        if let (Some(json), Some(text_name)) = (text, &text_name) {
            entries.push(Entry {
                name: text_name.clone(),
                deflate: false,
                size: json.len() as u64,
                source: Source::Bytes(json),
            });
        }
        entries.push(Entry {
            name: name.clone(),
            deflate: false,
            size: bytes,
            source: Source::File(&guidebook.file, bytes),
        });
        list.push(GuidebookFacts {
            file: name,
            title: title.to_owned(),
            pages,
            bytes,
            text: text_name,
        });
    }
    if list.is_empty() {
        return Err(WriteError::new(
            "A pack of guidebooks holds at least one guidebook.",
        ));
    }
    if list.len() > MAX_GUIDEBOOKS {
        return Err(WriteError::new(format!(
            "A pack of guidebooks holds at most {MAX_GUIDEBOOKS} guidebooks."
        )));
    }

    let (now, time) = now();
    let old = existing.map(|(_, header, _)| header);
    let pack_version = old.map_or_else(
        || "1.0.0".to_owned(),
        |header| next_version(&header.pack_version),
    );
    let created = old
        .and_then(|header| header.pack.created.clone())
        .unwrap_or_else(|| now.clone());
    let written_by = json!({ "name": pack.written_by.name, "version": pack.written_by.version });
    let manifest = json!({
        "schemaVersion": 2,
        "gameId": pack.pack_id,
        "title": game_title,
        "packVersion": pack_version,
        "gamePatch": "any",
        "attribution": [],
        "idMigrations": {},
        "created": created,
        "updated": now,
        "writtenBy": written_by,
    });
    let game = GameFacts {
        title: game_title.to_owned(),
        system: None,
    };
    let guidebooks_file = GuidebookList {
        format: GUIDEBOOK_LIST_FORMAT.to_owned(),
        version: 1,
        game: game.clone(),
        guidebooks: list.clone(),
    };
    let manifest_bytes = to_json(&manifest)?;
    let list_bytes = to_json(&guidebooks_file)?;
    entries.insert(
        0,
        Entry {
            name: GUIDEBOOK_LIST_FILE.to_owned(),
            deflate: true,
            size: list_bytes.len() as u64,
            source: Source::Bytes(list_bytes),
        },
    );
    entries.insert(
        0,
        Entry {
            name: MANIFEST_FILE.to_owned(),
            deflate: true,
            size: manifest_bytes.len() as u64,
            source: Source::Bytes(manifest_bytes),
        },
    );

    let header = Header {
        format: FORMAT.to_owned(),
        version: VERSION_WITH_GUIDEBOOKS,
        pack_id: pack.pack_id.clone(),
        title: game_title.to_owned(),
        game: Some(game),
        pack_version: pack_version.clone(),
        detail: Detail::Full,
        exported_at: now.clone(),
        exported_by: pack.written_by.clone(),
        pack: PackFacts {
            id: pack.pack_id.clone(),
            title: game_title.to_owned(),
            pack_version,
            game_patch: Some("any".to_owned()),
            schema_version: 2,
            attribution: Vec::new(),
            created: Some(created),
            updated: Some(now),
            written_by: Some(written_by),
        },
        personal_use: false,
        source: SourceFacts {
            kind: SourceKind::Unknown,
            origin: Some(Origin::Guidebooks),
            ..SourceFacts::default()
        },
        contents: Counts::default(),
        maps: Vec::new(),
        guides: Vec::new(),
        cover: None,
        images: ImagesFacts {
            detail: Detail::Full,
            files: 0,
            bytes: 0,
            layers: Vec::new(),
        },
        files: entries.len() as u64,
        bytes: entries.iter().map(|entry| entry.size).sum(),
        mine: None,
        guidebooks: list,
    };
    let header_bytes = to_json(&header)?;

    let part = part_path(target);
    let written = write_archive(&part, &header_bytes, &entries, kept_from.as_mut(), time);
    // The pack read from is let go before the new one takes its name.
    drop(kept_from);
    if let Err(error) = written {
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    if let Err(error) = fs::rename(&part, target) {
        let _ = fs::remove_file(&part);
        return Err(WriteError::new(format!(
            "Could not save {}: {error}",
            shown(target)
        )));
    }
    Ok(header)
}

/// One of the JSON files, with a sentence when it cannot be written.
fn to_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, WriteError> {
    serde_json::to_vec_pretty(value).map_err(|error| WriteError::new(error.to_string()))
}

fn write_archive(
    part: &Path,
    header: &[u8],
    entries: &[Entry<'_>],
    mut kept_from: Option<&mut ZipArchive<BufReader<File>>>,
    time: DateTime,
) -> Result<(), WriteError> {
    let failed = |error: &dyn std::fmt::Display| {
        WriteError::new(format!("Could not write {}: {error}", shown(part)))
    };
    let file = File::create(part).map_err(|error| failed(&error))?;
    let mut zip = ZipWriter::new(BufWriter::with_capacity(1 << 20, file));
    let base = SimpleFileOptions::default()
        .last_modified_time(time)
        .unix_permissions(0o644);
    zip.start_file(
        HEADER_FILE,
        base.compression_method(CompressionMethod::Deflated),
    )
    .map_err(|error| failed(&error))?;
    zip.write_all(header).map_err(|error| failed(&error))?;
    for entry in entries {
        if let Source::Kept(name) = &entry.source {
            let archive = kept_from
                .as_deref_mut()
                .ok_or_else(|| failed(&"no pack to copy from"))?;
            let index = archive
                .index_for_name(name)
                .ok_or_else(|| failed(&"a kept guidebook is missing"))?;
            let raw = archive
                .by_index_raw(index)
                .map_err(|error| failed(&error))?;
            zip.raw_copy_file(raw).map_err(|error| failed(&error))?;
            continue;
        }
        let options = base
            .compression_method(if entry.deflate {
                CompressionMethod::Deflated
            } else {
                CompressionMethod::Stored
            })
            // A guidebook over 4 GB needs ZIP64 records.
            .large_file(entry.size >= ZIP32_LIMIT);
        zip.start_file(entry.name.as_str(), options)
            .map_err(|error| failed(&error))?;
        match &entry.source {
            Source::Bytes(bytes) => zip.write_all(bytes).map_err(|error| failed(&error))?,
            Source::File(path, size) => {
                let source = File::open(path).map_err(|error| cannot_read(path, &error))?;
                let copied = io::copy(&mut source.take(size + 1), &mut zip)
                    .map_err(|error| failed(&error))?;
                if copied != *size {
                    return Err(WriteError::new(format!(
                        "{} changed while it was being written into the pack, so nothing was written.",
                        shown(path)
                    )));
                }
            }
            Source::Kept(_) => unreachable!("copied above"),
        }
    }
    let mut inner = zip.finish().map_err(|error| failed(&error))?;
    inner.flush().map_err(|error| failed(&error))?;
    let file = inner
        .into_inner()
        .map_err(|error| failed(&error.into_error()))?;
    file.sync_all().map_err(|error| failed(&error))?;
    Ok(())
}

/// `<target>.part`.
fn part_path(target: &Path) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

/// `major.minor.patch` with the patch one higher; `1.0.0` for anything
/// else.
fn next_version(version: &str) -> String {
    let mut parts = version.split('.').map(|part| part.parse::<u64>().ok());
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(Some(major)), Some(Some(minor)), Some(Some(patch)), None) => {
            format!("{major}.{minor}.{}", patch.saturating_add(1))
        }
        _ => "1.0.0".to_owned(),
    }
}

/// Now, as an ISO 8601 date and time in UTC and as a ZIP time.
fn now() -> (String, DateTime) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let (year, month, day, hour, minute, second) = civil(seconds);
    let iso = format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    let narrow = |value: i64| u8::try_from(value).unwrap_or(0);
    let time = DateTime::from_date_and_time(
        u16::try_from(year).unwrap_or(1980),
        narrow(month).max(1),
        narrow(day).max(1),
        narrow(hour),
        narrow(minute),
        narrow(second),
    )
    .unwrap_or_default();
    (iso, time)
}

/// The UTC date and time of a count of seconds since 1970 (the days to a
/// civil date as in the proleptic Gregorian calendar).
fn civil(seconds: u64) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = i64::try_from(seconds).unwrap_or(0);
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}
