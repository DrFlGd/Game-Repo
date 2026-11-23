//! Magazines in a pack file (version 10 of the format): the issues of one
//! magazine title, each a PDF or a CBZ (a ZIP of page pictures), with the
//! text of its pages, a small picture of its cover, and the games it
//! covers and on which pages.
//!
//! An issue is the entry `magazines/<name>.pdf` or `magazines/<name>.cbz`,
//! stored as it is (not deflated), so a reader can read any part of it
//! without unpacking it. The text of its pages, when known, is the entry
//! `magazines/<name>.text.json`, in the format of a guidebook's text
//! ([`crate::GuidebookText`]); its cover, when there is one, is the entry
//! `magazines/<name>.cover.jpg`, a small JPEG. Both are stored too. The
//! description says which magazine it is ([`MagazineFacts`]) and lists
//! each issue ([`IssueFacts`]); the pack's own file `magazines.json`
//! ([`MagazineList`]) lists them again for an app that reads the pack
//! unpacked into a folder.
//!
//! A pack of magazines is of no game: its id is `magazines-<title id>`,
//! and each issue says which games it covers ([`IssueGame`]), by their
//! Atlas game id when the app that writes it knows it, else by title and
//! system.

use serde::{Deserialize, Serialize};

use crate::{guidebook::is_stem, is_pack_id, GuidebookType, MAX_GUIDEBOOK_PAGES};

/// The folder of issues in a pack: `magazines/<name>.pdf`,
/// `magazines/<name>.cbz`, `magazines/<name>.text.json` and
/// `magazines/<name>.cover.jpg`.
pub const MAGAZINE_DIR: &str = "magazines/";
/// The pack's own list of its issues, beside `manifest.json`.
pub const MAGAZINE_LIST_FILE: &str = "magazines.json";
/// `format` of [`MagazineList`].
pub const MAGAZINE_LIST_FORMAT: &str = "atlas-magazines";
/// The start of the id of every pack of magazines: `magazines-<title id>`.
pub const MAGAZINE_PACK_PREFIX: &str = "magazines-";

/// The most issues one file holds.
pub const MAX_ISSUES: usize = 1000;
/// The longest title of a magazine, of an issue, or of a game an issue
/// covers, in characters.
pub const MAX_MAGAZINE_TITLE: usize = 200;
/// The longest name of a publisher or of a system, in characters.
pub const MAX_MAGAZINE_NAME: usize = 100;
/// The most games one issue says it covers.
pub const MAX_ISSUE_GAMES: usize = 200;
/// The most runs of pages one game has in one issue.
pub const MAX_GAME_PAGE_RUNS: usize = 50;
/// The largest cover picture.
pub const MAX_COVER_BYTES: u64 = 512 << 10;
/// The longest side a cover picture should have, in pixels.
pub const COVER_SIDE: u32 = 512;

/// Which magazine a pack holds the issues of (`magazine` in the
/// description).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MagazineFacts {
    /// Its title, as it is shown: 1 to [`MAX_MAGAZINE_TITLE`] characters.
    pub title: String,
    /// Who publishes it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
}

/// One issue, as the description lists it (`issues[]`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueFacts {
    /// Its entry name: `magazines/<name>.pdf` or `magazines/<name>.cbz`
    /// (see [`is_issue_name`]).
    pub file: String,
    /// Its title, as it is shown, such as `Issue 42`.
    pub title: String,
    /// When it came out: `YYYY-MM` or `YYYY-MM-DD` (see [`is_issue_date`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// Its number, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
    /// How many pages it has: for a CBZ, its pictures.
    pub pages: u32,
    /// The size of its file in bytes.
    pub bytes: u64,
    /// The entry name of the text of its pages,
    /// `magazines/<name>.text.json`; absent when the file holds none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The entry name of its cover, `magazines/<name>.cover.jpg`; absent
    /// when the file holds none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// The games it covers, each with its pages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub games: Vec<IssueGame>,
}

impl IssueFacts {
    /// Whether it is a PDF or a CBZ, by its entry name.
    pub fn file_type(&self) -> GuidebookType {
        GuidebookType::of_file_name(&self.file).unwrap_or(GuidebookType::Pdf)
    }
}

/// A game an issue covers, and on which pages.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IssueGame {
    /// The game's title.
    pub title: String,
    /// The system the game is for, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// The game's id in Atlas (its pack's `gameId`), when the app that
    /// writes the pack knows it. Atlas matches the game by it first, else
    /// by title and system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    /// The runs of pages about it, each `[first, last]` (one page is
    /// `[12, 12]`), in the issue's page numbers counted from 1.
    pub pages: Vec<[u32; 2]>,
}

/// The pack's own list of its issues (`magazines.json`), as the
/// description lists them.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MagazineList {
    /// Always `atlas-magazines`.
    pub format: String,
    /// Always 1.
    pub version: u32,
    pub magazine: MagazineFacts,
    pub issues: Vec<IssueFacts>,
}

/// Whether `name` is an entry name an issue may have:
/// `magazines/<name>.pdf` or `magazines/<name>.cbz`, the name 1 to 80
/// characters of lower case letters, digits, `-` and `_`, starting with a
/// letter or a digit.
pub fn is_issue_name(name: &str) -> bool {
    issue_stem(name).is_some()
}

/// The name of an issue's entry without its folder and extension.
fn issue_stem(name: &str) -> Option<&str> {
    let rest = name.strip_prefix(MAGAZINE_DIR)?;
    let stem = rest
        .strip_suffix(".pdf")
        .or_else(|| rest.strip_suffix(".cbz"))?;
    is_stem(stem).then_some(stem)
}

/// The entry name of the text of the issue `file` names:
/// `magazines/<name>.text.json`; none for a name that is not an issue's.
pub fn issue_text_name(file: &str) -> Option<String> {
    issue_stem(file).map(|stem| format!("{MAGAZINE_DIR}{stem}.text.json"))
}

/// The entry name of the cover of the issue `file` names:
/// `magazines/<name>.cover.jpg`; none for a name that is not an issue's.
pub fn issue_cover_name(file: &str) -> Option<String> {
    issue_stem(file).map(|stem| format!("{MAGAZINE_DIR}{stem}.cover.jpg"))
}

/// An entry name for an issue titled `title`, out on `date`, of
/// `file_type`, that none of `taken` has: the date and the title in lower
/// case letters and digits joined by `-`, with `-2`, `-3`… after it when
/// that is taken.
pub fn issue_name_of(
    title: &str,
    date: Option<&str>,
    file_type: GuidebookType,
    taken: &[&str],
) -> String {
    let mut stem = String::new();
    for word in date
        .into_iter()
        .chain(std::iter::once(title))
        .flat_map(|text| text.split(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|word| !word.is_empty())
    {
        if stem.len() + word.len() + 1 > 80 - 4 {
            break;
        }
        if !stem.is_empty() {
            stem.push('-');
        }
        stem.push_str(&word.to_ascii_lowercase());
    }
    if stem.is_empty() {
        stem.push_str("issue");
    }
    let used = |stem: &str| {
        taken
            .iter()
            .any(|name| issue_stem(name).is_some_and(|other| other == stem))
    };
    let mut candidate = stem.clone();
    let mut next = 2;
    while used(&candidate) {
        candidate = format!("{stem}-{next}");
        next += 1;
    }
    format!("{MAGAZINE_DIR}{candidate}.{}", file_type.extension())
}

/// Whether `date` is the date of an issue: `YYYY-MM` or `YYYY-MM-DD`, a
/// month of 01 to 12 and a day of 01 to 31.
pub fn is_issue_date(date: &str) -> bool {
    let digits = |text: &str, len: usize| {
        text.len() == len && text.bytes().all(|byte| byte.is_ascii_digit())
    };
    let mut parts = date.split('-');
    let (Some(year), Some(month)) = (parts.next(), parts.next()) else {
        return false;
    };
    let day = parts.next();
    if parts.next().is_some() || !digits(year, 4) || !digits(month, 2) {
        return false;
    }
    if !(1..=12).contains(&month.parse::<u32>().unwrap_or(0)) {
        return false;
    }
    day.is_none_or(|day| digits(day, 2) && (1..=31).contains(&day.parse::<u32>().unwrap_or(0)))
}

/// Whether a title or name has 1 to `max` characters that are not all
/// space.
fn fits(text: &str, max: usize) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && trimmed.chars().count() <= max
}

/// Checks a game an issue of `pages` pages covers: a title, a system and
/// a game id that can be ones, and runs of pages within the issue.
pub(crate) fn check_game(game: &IssueGame, pages: u32) -> bool {
    fits(&game.title, MAX_MAGAZINE_TITLE)
        && game
            .system
            .as_deref()
            .is_none_or(|system| fits(system, MAX_MAGAZINE_NAME))
        && game.game_id.as_deref().is_none_or(is_pack_id)
        && !game.pages.is_empty()
        && game.pages.len() <= MAX_GAME_PAGE_RUNS
        && game
            .pages
            .iter()
            .all(|[first, last]| 1 <= *first && first <= last && *last <= pages)
}

/// Checks the magazine and the issues a description lists: a magazine
/// with a title, at most [`MAX_ISSUES`] issues each with a safe entry name
/// of its own, a title, a date that can be one, a number of pages and a
/// size within the limits, text and cover entries of the same name, and
/// games that can be ones. A plain sentence when one cannot be used.
pub(crate) fn check_issues(
    magazine: Option<&MagazineFacts>,
    issues: &[IssueFacts],
) -> Result<(), String> {
    if issues.is_empty() {
        return Ok(());
    }
    let Some(magazine) = magazine else {
        return Err("This pack file lists issues of no magazine.".to_owned());
    };
    if !fits(&magazine.title, MAX_MAGAZINE_TITLE)
        || magazine
            .publisher
            .as_deref()
            .is_some_and(|publisher| !fits(publisher, MAX_MAGAZINE_NAME))
    {
        return Err("This pack file names a magazine that cannot be used.".to_owned());
    }
    if issues.len() > MAX_ISSUES {
        return Err("This pack file lists too many issues.".to_owned());
    }
    let unusable = || "This pack file lists an issue that cannot be used.".to_owned();
    let mut stems: Vec<&str> = Vec::with_capacity(issues.len());
    for issue in issues {
        let stem = issue_stem(&issue.file).ok_or_else(unusable)?;
        if stems.contains(&stem) {
            return Err(unusable());
        }
        stems.push(stem);
        if !fits(&issue.title, MAX_MAGAZINE_TITLE)
            || issue
                .date
                .as_deref()
                .is_some_and(|date| !is_issue_date(date))
            || !(1..=MAX_GUIDEBOOK_PAGES).contains(&issue.pages)
            || !(1..=crate::MAX_GUIDEBOOK_BYTES).contains(&issue.bytes)
        {
            return Err(unusable());
        }
        if issue
            .text
            .as_ref()
            .is_some_and(|text| issue_text_name(&issue.file).as_ref() != Some(text))
            || issue
                .cover
                .as_ref()
                .is_some_and(|cover| issue_cover_name(&issue.file).as_ref() != Some(cover))
        {
            return Err(unusable());
        }
        if issue.games.len() > MAX_ISSUE_GAMES
            || !issue.games.iter().all(|game| check_game(game, issue.pages))
        {
            return Err(unusable());
        }
    }
    Ok(())
}
