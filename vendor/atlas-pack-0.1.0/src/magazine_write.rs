//! Writing a pack file of magazines: the issues of one magazine title,
//! each a PDF or a CBZ file with the text of its pages, its cover and the
//! games it covers (version 10 of the format). See
//! [`write_magazine_pack`].

use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use serde_json::json;

use crate::{
    guidebook_pages,
    magazine::{check_game, is_issue_date, issue_cover_name, issue_name_of, issue_text_name},
    write::{
        check_start, now, part_path, shown, to_json, write_archive, Entry, Source, MANIFEST_FILE,
    },
    AppInfo, Counts, Detail, GuidebookText, GuidebookType, Header, ImagesFacts, IssueFacts,
    IssueGame, MagazineFacts, MagazineList, Origin, PackFacts, SourceFacts, SourceKind, WriteError,
    FORMAT, MAGAZINE_LIST_FILE, MAGAZINE_LIST_FORMAT, MAGAZINE_PACK_PREFIX, MAX_COVER_BYTES,
    MAX_GUIDEBOOK_BYTES, MAX_GUIDEBOOK_PAGES, MAX_GUIDEBOOK_TEXT_BYTES, MAX_HEADER_BYTES,
    MAX_ISSUES, MAX_ISSUE_GAMES, MAX_MAGAZINE_NAME, MAX_MAGAZINE_TITLE, VERSION_WITH_MAGAZINES,
};

/// An issue to put in a pack: its file on disk, its title and date, and
/// what is known of its pages and the games it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewIssue {
    /// The PDF (`.pdf`) or CBZ (`.cbz`) file, by its name's extension. A
    /// CBR file is not read: turn it into a CBZ first.
    pub file: PathBuf,
    /// Its title, as it is shown, such as `Issue 42`: 1 to
    /// [`MAX_MAGAZINE_TITLE`] characters.
    pub title: String,
    /// When it came out: `YYYY-MM` or `YYYY-MM-DD`.
    pub date: Option<String>,
    /// Its number, when it has one.
    pub number: Option<u32>,
    /// How many pages it has. None: read from the file, for a PDF from its
    /// page tree, for a CBZ its pictures.
    pub pages: Option<u32>,
    /// The text of each page, the first being page 1; none when it is not
    /// known. A page whose text is only space is left out of the file.
    pub text: Option<Vec<String>>,
    /// A small JPEG of its cover, at most 512 KB and best at most
    /// [`crate::COVER_SIDE`] pixels on its longest side.
    pub cover: Option<PathBuf>,
    /// The games it covers, each with its runs of pages.
    pub games: Vec<IssueGame>,
}

/// A pack of the issues of one magazine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MagazinePack {
    /// The pack's id: `magazines-<title id>` (see [`crate::is_pack_id`]).
    /// A pack is known by it: write the same magazine under the same id, so
    /// the new file replaces the old one.
    pub pack_id: String,
    /// Which magazine it is.
    pub magazine: MagazineFacts,
    /// The app that writes the file, written as the pack's `writtenBy` and
    /// the file's `exportedBy`.
    pub written_by: AppInfo,
    /// The issues, in the order they are listed (Atlas shows them by date).
    pub issues: Vec<NewIssue>,
}

/// Writes a complete pack file of a magazine's issues at `target`, through
/// `<target>.part` beside it, renamed to `target` when it is complete (so
/// an app that watches the folder never reads half a file, and a file
/// already at `target` is replaced whole). Returns its description.
///
/// The file is version 10 of the format, with `source.origin`
/// `magazines`. Each issue is stored as it is, as `magazines/<name>.pdf`
/// or `.cbz`, the text of its pages, when given, as
/// `magazines/<name>.text.json`, and its cover, when given, as
/// `magazines/<name>.cover.jpg`; the name is made from its date and title.
/// The covers come first, right after the pack's own files, so a list of
/// the issues can be shown from the start of the file. The pack also has
/// a `manifest.json` and a `magazines.json` listing the issues, so it
/// reads as a pack unpacked into a folder too.
///
/// Refuses, with a plain sentence and nothing written: a pack id that is
/// not `magazines-` and a name, a magazine without a title, no issues or
/// more than [`MAX_ISSUES`], a file that is not a PDF or a CBZ by its name
/// and its first bytes, a file over 16 GB, a title that is empty or too
/// long, a date that is not `YYYY-MM` or `YYYY-MM-DD`, a PDF whose pages
/// cannot be counted and were not given, a CBZ with no pictures, text of
/// more pages than the issue has, text over 64 MB, a cover that is not a
/// JPEG or is over 512 KB, more than [`MAX_ISSUE_GAMES`] games or a game
/// without a title or with pages outside the issue, and a description too
/// large to read.
pub fn write_magazine_pack(pack: &MagazinePack, target: &Path) -> Result<Header, WriteError> {
    let id_name = pack
        .pack_id
        .strip_prefix(MAGAZINE_PACK_PREFIX)
        .unwrap_or("");
    if id_name.is_empty() || !crate::is_pack_id(&pack.pack_id) {
        return Err(WriteError::new(
            "The id of a pack of magazines must be magazines- and a name: up to 64 lower case letters, digits, - and _ in all.",
        ));
    }
    let title = pack.magazine.title.trim();
    if title.is_empty() || title.chars().count() > MAX_MAGAZINE_TITLE {
        return Err(WriteError::new(format!(
            "The title of the magazine must be 1 to {MAX_MAGAZINE_TITLE} characters."
        )));
    }
    let publisher = pack
        .magazine
        .publisher
        .as_deref()
        .map(str::trim)
        .filter(|publisher| !publisher.is_empty());
    if publisher.is_some_and(|publisher| publisher.chars().count() > MAX_MAGAZINE_NAME) {
        return Err(WriteError::new(format!(
            "The name of the publisher can be at most {MAX_MAGAZINE_NAME} characters."
        )));
    }
    if pack.written_by.name.trim().is_empty() {
        return Err(WriteError::new(
            "The name of the app that writes the pack is missing.",
        ));
    }
    if pack.issues.is_empty() {
        return Err(WriteError::new(
            "A pack of magazines holds at least one issue.",
        ));
    }
    if pack.issues.len() > MAX_ISSUES {
        return Err(WriteError::new(format!(
            "A pack of magazines holds at most {MAX_ISSUES} issues."
        )));
    }
    let magazine = MagazineFacts {
        title: title.to_owned(),
        publisher: publisher.map(str::to_owned),
    };

    let mut list: Vec<IssueFacts> = Vec::with_capacity(pack.issues.len());
    let mut covers: Vec<Entry<'_>> = Vec::new();
    let mut files: Vec<Entry<'_>> = Vec::new();
    for issue in &pack.issues {
        let facts = check_issue(issue, &list, &mut covers, &mut files)?;
        list.push(facts);
    }

    let (now, time) = now();
    let written_by = json!({ "name": pack.written_by.name, "version": pack.written_by.version });
    let manifest = json!({
        "schemaVersion": 2,
        "gameId": pack.pack_id,
        "title": title,
        "packVersion": "1.0.0",
        "gamePatch": "any",
        "attribution": [],
        "idMigrations": {},
        "created": now,
        "updated": now,
        "writtenBy": written_by,
    });
    let list_file = MagazineList {
        format: MAGAZINE_LIST_FORMAT.to_owned(),
        version: 1,
        magazine: magazine.clone(),
        issues: list.clone(),
    };
    let manifest_bytes = to_json(&manifest)?;
    let list_bytes = to_json(&list_file)?;
    let mut entries: Vec<Entry<'_>> = vec![
        Entry {
            name: MANIFEST_FILE.to_owned(),
            deflate: true,
            size: manifest_bytes.len() as u64,
            source: Source::Bytes(manifest_bytes),
        },
        Entry {
            name: MAGAZINE_LIST_FILE.to_owned(),
            deflate: true,
            size: list_bytes.len() as u64,
            source: Source::Bytes(list_bytes),
        },
    ];
    entries.extend(covers);
    entries.extend(files);

    let header = Header {
        format: FORMAT.to_owned(),
        version: VERSION_WITH_MAGAZINES,
        pack_id: pack.pack_id.clone(),
        title: title.to_owned(),
        game: None,
        pack_version: "1.0.0".to_owned(),
        detail: Detail::Full,
        exported_at: now.clone(),
        exported_by: pack.written_by.clone(),
        pack: PackFacts {
            id: pack.pack_id.clone(),
            title: title.to_owned(),
            pack_version: "1.0.0".to_owned(),
            game_patch: Some("any".to_owned()),
            schema_version: 2,
            attribution: Vec::new(),
            created: Some(now.clone()),
            updated: Some(now),
            written_by: Some(written_by),
        },
        personal_use: false,
        source: SourceFacts {
            kind: SourceKind::Unknown,
            origin: Some(Origin::Magazines),
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
        guidebooks: Vec::new(),
        magazine: Some(magazine),
        issues: list,
    };
    let header_bytes = to_json(&header)?;
    if header_bytes.len() as u64 > MAX_HEADER_BYTES {
        return Err(WriteError::new(
            "The list of these issues and their games is too large for one pack file. Write the issues in two packs.",
        ));
    }

    let part = part_path(target);
    if let Err(error) = write_archive(&part, &header_bytes, &entries, None, time) {
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

/// Checks one issue and adds its entries: its cover to `covers`, its text
/// and file to `files`. Returns what the description lists of it.
fn check_issue<'a>(
    issue: &'a NewIssue,
    before: &[IssueFacts],
    covers: &mut Vec<Entry<'a>>,
    files: &mut Vec<Entry<'a>>,
) -> Result<IssueFacts, WriteError> {
    let file_type = issue
        .file
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(GuidebookType::of_file_name)
        .ok_or_else(|| {
            WriteError::new(format!(
                "{} is not a PDF or CBZ file. Only PDF and CBZ files can be issues: turn a CBR file into a CBZ first.",
                shown(&issue.file)
            ))
        })?;
    let title = issue.title.trim();
    if title.is_empty() || title.chars().count() > MAX_MAGAZINE_TITLE {
        return Err(WriteError::new(format!(
            "The title of an issue must be 1 to {MAX_MAGAZINE_TITLE} characters."
        )));
    }
    let date = issue.date.as_deref().map(str::trim);
    if date.is_some_and(|date| !is_issue_date(date)) {
        return Err(WriteError::new(format!(
            "The date of {title} must be a year and a month (YYYY-MM), or a day too (YYYY-MM-DD)."
        )));
    }
    let mut opened = File::open(&issue.file).map_err(|error| {
        WriteError::new(format!("Could not read {}: {error}", shown(&issue.file)))
    })?;
    let bytes = opened
        .metadata()
        .map_err(|error| {
            WriteError::new(format!("Could not read {}: {error}", shown(&issue.file)))
        })?
        .len();
    if bytes == 0 {
        return Err(WriteError::new(format!("{} is empty.", shown(&issue.file))));
    }
    if bytes > MAX_GUIDEBOOK_BYTES {
        return Err(WriteError::new(format!(
            "{} is larger than 16 GB, the most an issue can be.",
            shown(&issue.file)
        )));
    }
    check_start(&issue.file, &mut opened, file_type)?;
    drop(opened);
    let pages = match issue.pages {
        Some(pages) if (1..=MAX_GUIDEBOOK_PAGES).contains(&pages) => pages,
        Some(_) => {
            return Err(WriteError::new(format!(
                "The number of pages of {} must be 1 to {MAX_GUIDEBOOK_PAGES}.",
                shown(&issue.file)
            )))
        }
        None => guidebook_pages(&issue.file)?,
    };
    if issue.games.len() > MAX_ISSUE_GAMES {
        return Err(WriteError::new(format!(
            "{title} names more than {MAX_ISSUE_GAMES} games."
        )));
    }
    let games: Vec<IssueGame> = issue
        .games
        .iter()
        .map(|game| IssueGame {
            title: game.title.trim().to_owned(),
            system: game
                .system
                .as_deref()
                .map(str::trim)
                .filter(|system| !system.is_empty())
                .map(str::to_owned),
            game_id: game.game_id.clone(),
            pages: game.pages.clone(),
        })
        .collect();
    if let Some(game) = games.iter().find(|game| !check_game(game, pages)) {
        return Err(WriteError::new(format!(
            "{title} names a game that cannot be used ({}): it needs a title, a usable game id when it has one, and pages within the issue's {pages}, each run as [first, last].",
            if game.title.is_empty() { "no title" } else { game.title.as_str() }
        )));
    }

    let taken: Vec<&str> = before.iter().map(|facts| facts.file.as_str()).collect();
    let name = issue_name_of(title, date, file_type, &taken);
    let text = match &issue.text {
        Some(pages_text) => {
            if pages_text.len() > pages as usize {
                return Err(WriteError::new(format!(
                    "The text given for {} has more pages than the issue.",
                    shown(&issue.file)
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
                        "The text of {} is larger than 64 MB, the most an issue's text can be.",
                        shown(&issue.file)
                    )));
                }
                Some(json)
            }
        }
        None => None,
    };
    let cover = match &issue.cover {
        Some(path) => Some(read_cover(path)?),
        None => None,
    };
    let text_name = text.as_ref().and_then(|_| issue_text_name(&name));
    let cover_name = cover.as_ref().and_then(|_| issue_cover_name(&name));
    if let (Some(bytes), Some(cover_name)) = (cover, &cover_name) {
        covers.push(Entry {
            name: cover_name.clone(),
            deflate: false,
            size: bytes.len() as u64,
            source: Source::Bytes(bytes),
        });
    }
    if let (Some(json), Some(text_name)) = (text, &text_name) {
        files.push(Entry {
            name: text_name.clone(),
            deflate: false,
            size: json.len() as u64,
            source: Source::Bytes(json),
        });
    }
    files.push(Entry {
        name: name.clone(),
        deflate: false,
        size: bytes,
        source: Source::File(&issue.file, bytes),
    });
    Ok(IssueFacts {
        file: name,
        title: title.to_owned(),
        date: date.map(str::to_owned),
        number: issue.number,
        pages,
        bytes,
        text: text_name,
        cover: cover_name,
        games,
    })
}

/// A cover picture: a JPEG of at most [`MAX_COVER_BYTES`].
fn read_cover(path: &Path) -> Result<Vec<u8>, WriteError> {
    let file = File::open(path)
        .map_err(|error| WriteError::new(format!("Could not read {}: {error}", shown(path))))?;
    let mut bytes = Vec::new();
    file.take(MAX_COVER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| WriteError::new(format!("Could not read {}: {error}", shown(path))))?;
    if bytes.len() as u64 > MAX_COVER_BYTES {
        return Err(WriteError::new(format!(
            "{} is larger than 512 KB, the most a cover can be. Make it smaller: 512 pixels on its longest side is enough.",
            shown(path)
        )));
    }
    if !bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Err(WriteError::new(format!(
            "{} is not a JPEG picture. A cover must be one.",
            shown(path)
        )));
    }
    Ok(bytes)
}
