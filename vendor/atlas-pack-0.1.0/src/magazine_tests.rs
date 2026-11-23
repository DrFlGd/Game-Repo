//! Magazines (version 10): writing a pack of a magazine's issues, reading
//! it back, its checks and refusals, and what an app that reads only
//! version 9 does with it. The PDF, CBZ and JPEG files are tiny ones made
//! here; every name is made up.

use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use serde_json::{json, Value};
use zip::{write::SimpleFileOptions, CompressionMethod, ZipArchive, ZipWriter};

use super::*;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "atlas-magazines-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A PDF of `pages` empty pages.
fn pdf(pages: u32) -> Vec<u8> {
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 10 + i)).collect();
    let mut text = format!(
        "%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [{}] /Count {pages} >>\nendobj\n",
        kids.join(" ")
    );
    for i in 0..pages {
        text.push_str(&format!(
            "{} 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] >>\nendobj\n",
            10 + i
        ));
    }
    text.push_str("trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    text.into_bytes()
}

/// A CBZ of `pictures` page pictures.
fn cbz(pictures: u32) -> Vec<u8> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for i in 1..=pictures {
        zip.start_file(format!("page{i:02}.jpg"), options).unwrap();
        zip.write_all(b"\xFF\xD8\xFFnot really a picture").unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// The start of a JPEG, which is all a cover is checked for.
const JPEG: &[u8] = b"\xFF\xD8\xFF\xE0 a small cover";

fn library_app() -> AppInfo {
    AppInfo {
        name: "Fen Shelf".into(),
        version: "3.1.0".into(),
        platform: "windows".into(),
    }
}

fn game(title: &str, pages: &[[u32; 2]]) -> IssueGame {
    IssueGame {
        title: title.into(),
        system: None,
        game_id: None,
        pages: pages.to_vec(),
    }
}

fn issue(file: &Path, title: &str, date: &str) -> NewIssue {
    NewIssue {
        file: file.to_owned(),
        title: title.into(),
        date: Some(date.into()),
        number: None,
        pages: None,
        text: None,
        cover: None,
        games: Vec::new(),
    }
}

fn pack(issues: Vec<NewIssue>) -> MagazinePack {
    MagazinePack {
        pack_id: "magazines-marsh-monthly".into(),
        magazine: MagazineFacts {
            title: "Marsh Monthly".into(),
            publisher: Some(" Reed Press ".into()),
        },
        written_by: library_app(),
        issues,
    }
}

/// An entry's bytes, and whether it was stored.
fn entry(file: &Path, name: &str) -> (Vec<u8>, bool) {
    let mut archive = ZipArchive::new(fs::File::open(file).unwrap()).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let stored = entry.compression() == CompressionMethod::Stored;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes).unwrap();
    (bytes, stored)
}

fn names(file: &Path) -> Vec<String> {
    let archive = ZipArchive::new(fs::File::open(file).unwrap()).unwrap();
    archive
        .file_names()
        .map(|name| name.unwrap().into_owned())
        .collect()
}

#[test]
fn writes_a_pack_of_magazines_and_reads_it_back() {
    let dir = TempDir::new("write");
    let first = dir.path("Marsh Monthly 41.pdf");
    fs::write(&first, pdf(6)).unwrap();
    let second = dir.path("Marsh Monthly 42.cbz");
    fs::write(&second, cbz(4)).unwrap();
    let cover = dir.path("cover-41.jpg");
    fs::write(&cover, JPEG).unwrap();
    let target = dir.path("Marsh Monthly.atlaspack");

    let mut one = issue(&first, " Issue 41 ", "2026-04");
    one.number = Some(41);
    one.cover = Some(cover);
    one.text = Some(vec![
        "Marsh Monthly\nIssue 41".into(),
        String::new(),
        "Hollow Marsh reviewed: the heron returns.".into(),
    ]);
    one.games = vec![
        IssueGame {
            title: " Hollow Marsh ".into(),
            system: Some(" Desk ".into()),
            game_id: Some("hollow-marsh".into()),
            pages: vec![[3, 4], [6, 6]],
        },
        game("Reed Runner", &[[5, 5]]),
    ];
    let mut two = issue(&second, "Issue 42", "2026-05-15");
    two.number = Some(42);
    two.games = vec![game("Reed Runner", &[[2, 3]])];
    let written = write_magazine_pack(&pack(vec![one, two]), &target).unwrap();
    assert!(!dir.path("Marsh Monthly.atlaspack.part").exists());

    let header = read_header(&target).unwrap();
    assert_eq!(header, written);
    assert_eq!(header.version, VERSION_WITH_MAGAZINES);
    assert_eq!(header.pack_id, "magazines-marsh-monthly");
    assert_eq!(header.title, "Marsh Monthly");
    assert_eq!(header.game, None);
    assert_eq!(header.source.origin, Some(Origin::Magazines));
    assert_eq!(header.origin(), Some(Origin::Magazines));
    assert_eq!(header.exported_by, library_app());
    assert!(header.guidebooks.is_empty());
    assert_eq!(
        header.magazine,
        Some(MagazineFacts {
            title: "Marsh Monthly".into(),
            publisher: Some("Reed Press".into()),
        })
    );
    assert_eq!(
        header.issues,
        vec![
            IssueFacts {
                file: "magazines/2026-04-issue-41.pdf".into(),
                title: "Issue 41".into(),
                date: Some("2026-04".into()),
                number: Some(41),
                pages: 6,
                bytes: pdf(6).len() as u64,
                text: Some("magazines/2026-04-issue-41.text.json".into()),
                cover: Some("magazines/2026-04-issue-41.cover.jpg".into()),
                games: vec![
                    IssueGame {
                        title: "Hollow Marsh".into(),
                        system: Some("Desk".into()),
                        game_id: Some("hollow-marsh".into()),
                        pages: vec![[3, 4], [6, 6]],
                    },
                    game("Reed Runner", &[[5, 5]]),
                ],
            },
            IssueFacts {
                file: "magazines/2026-05-15-issue-42.cbz".into(),
                title: "Issue 42".into(),
                date: Some("2026-05-15".into()),
                number: Some(42),
                pages: 4,
                bytes: cbz(4).len() as u64,
                text: None,
                cover: None,
                games: vec![game("Reed Runner", &[[2, 3]])],
            },
        ]
    );
    assert_eq!(header.issues[1].file_type(), GuidebookType::Cbz);
    // The description, the pack's own files, the covers, then each issue's
    // text and file.
    assert_eq!(
        names(&target),
        [
            "atlaspack.json",
            "manifest.json",
            "magazines.json",
            "magazines/2026-04-issue-41.cover.jpg",
            "magazines/2026-04-issue-41.text.json",
            "magazines/2026-04-issue-41.pdf",
            "magazines/2026-05-15-issue-42.cbz",
        ]
    );
    assert_eq!(header.files, 6);
    for name in [
        "magazines/2026-04-issue-41.pdf",
        "magazines/2026-04-issue-41.text.json",
        "magazines/2026-04-issue-41.cover.jpg",
        "magazines/2026-05-15-issue-42.cbz",
    ] {
        assert!(entry(&target, name).1, "{name} is stored");
    }
    assert_eq!(entry(&target, "magazines/2026-04-issue-41.pdf").0, pdf(6));
    assert_eq!(
        entry(&target, "magazines/2026-04-issue-41.cover.jpg").0,
        JPEG
    );
    // The text is in the format of a guidebook's.
    let (text, _) = entry(&target, "magazines/2026-04-issue-41.text.json");
    let text = parse_guidebook_text(&text, 6).unwrap();
    assert_eq!(
        text.pages.iter().map(|page| page.page).collect::<Vec<_>>(),
        [1, 3]
    );
    // The pack reads as a pack folder too.
    let (manifest, _) = entry(&target, "manifest.json");
    let manifest: Value = serde_json::from_slice(&manifest).unwrap();
    assert_eq!(manifest["gameId"], "magazines-marsh-monthly");
    assert_eq!(manifest["title"], "Marsh Monthly");
    assert_eq!(manifest["writtenBy"]["name"], "Fen Shelf");
    let (list, _) = entry(&target, "magazines.json");
    let list: MagazineList = serde_json::from_slice(&list).unwrap();
    assert_eq!(list.format, MAGAZINE_LIST_FORMAT);
    assert_eq!(list.version, 1);
    assert_eq!(list.issues, header.issues);
    assert_eq!(Some(list.magazine), header.magazine);
    // As JSON, a game's pages are runs of [first, last].
    let raw: Value = serde_json::from_slice(&entry(&target, "magazines.json").0).unwrap();
    assert_eq!(
        raw["issues"][0]["games"][0],
        json!({"title": "Hollow Marsh", "system": "Desk", "gameId": "hollow-marsh",
               "pages": [[3, 4], [6, 6]]})
    );
}

#[test]
fn refuses_what_cannot_be_an_issue_and_writes_nothing() {
    let dir = TempDir::new("refuse");
    let file = dir.path("issue.pdf");
    fs::write(&file, pdf(4)).unwrap();
    let target = dir.path("out.atlaspack");
    let refused = |pack: MagazinePack| {
        let error = write_magazine_pack(&pack, &target).unwrap_err();
        assert!(!target.exists());
        assert!(!dir.path("out.atlaspack.part").exists());
        error.to_string()
    };
    let one = || issue(&file, "Issue 1", "2026-01");

    let mut wrong_id = pack(vec![one()]);
    wrong_id.pack_id = "marsh-monthly".into();
    assert!(refused(wrong_id).contains("magazines-"));
    let mut bare_id = pack(vec![one()]);
    bare_id.pack_id = "magazines-".into();
    assert!(refused(bare_id).contains("magazines-"));
    let mut untitled = pack(vec![one()]);
    untitled.magazine.title = "  ".into();
    assert!(refused(untitled).contains("title of the magazine"));
    assert!(refused(pack(Vec::new())).contains("at least one issue"));

    let cbr = dir.path("issue.cbr");
    fs::write(&cbr, b"Rar!\x1a\x07\x00").unwrap();
    assert!(refused(pack(vec![issue(&cbr, "Issue 2", "2026-02")])).contains("CBR"));
    let mut bad_date = one();
    bad_date.date = Some("May 2026".into());
    assert!(refused(pack(vec![bad_date])).contains("YYYY-MM"));
    let mut outside = one();
    outside.games = vec![game("Reed Runner", &[[3, 5]])];
    assert!(refused(pack(vec![outside])).contains("Reed Runner"));
    let mut backwards = one();
    backwards.games = vec![game("Reed Runner", &[[3, 2]])];
    assert!(refused(pack(vec![backwards])).contains("[first, last]"));
    let mut no_pages = one();
    no_pages.games = vec![game("Reed Runner", &[])];
    assert!(refused(pack(vec![no_pages])).contains("Reed Runner"));
    let mut bad_id = one();
    bad_id.games = vec![IssueGame {
        game_id: Some("Not An Id".into()),
        ..game("Reed Runner", &[[1, 1]])
    }];
    assert!(refused(pack(vec![bad_id])).contains("game id"));

    let png = dir.path("cover.png");
    fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
    let mut not_jpeg = one();
    not_jpeg.cover = Some(png);
    assert!(refused(pack(vec![not_jpeg])).contains("not a JPEG"));
    let large = dir.path("large.jpg");
    let mut big = JPEG.to_vec();
    big.resize(MAX_COVER_BYTES as usize + 1, 0);
    fs::write(&large, big).unwrap();
    let mut too_large = one();
    too_large.cover = Some(large);
    assert!(refused(pack(vec![too_large])).contains("512 KB"));
    let mut long_text = one();
    long_text.text = Some(vec![String::new(); 5]);
    assert!(refused(pack(vec![long_text])).contains("more pages than the issue"));
}

#[test]
fn names_issues_by_date_and_title_each_once() {
    let first = issue_name_of("Issue 42", Some("2026-05"), GuidebookType::Pdf, &[]);
    assert_eq!(first, "magazines/2026-05-issue-42.pdf");
    let again = issue_name_of(
        "Issue 42",
        Some("2026-05"),
        GuidebookType::Cbz,
        &[first.as_str()],
    );
    assert_eq!(again, "magazines/2026-05-issue-42-2.cbz");
    assert_eq!(
        issue_name_of("???", None, GuidebookType::Pdf, &[]),
        "magazines/issue.pdf"
    );
    assert!(is_issue_name("magazines/2026-05-issue-42.pdf"));
    assert!(!is_issue_name("magazines/Issue.pdf"));
    assert!(!is_issue_name("guidebooks/issue.pdf"));
    assert!(!is_issue_name("magazines/issue.cbr"));
    assert_eq!(
        issue_cover_name("magazines/a.cbz").as_deref(),
        Some("magazines/a.cover.jpg")
    );
    assert_eq!(
        issue_text_name("magazines/a.cbz").as_deref(),
        Some("magazines/a.text.json")
    );
    for good in ["2026-05", "2026-12-31", "1987-01-01"] {
        assert!(is_issue_date(good), "{good}");
    }
    for bad in [
        "2026",
        "2026-13",
        "2026-00",
        "2026-5",
        "2026-05-32",
        "26-05",
        "2026-05-01-1",
    ] {
        assert!(!is_issue_date(bad), "{bad}");
    }
}

/// A version 10 description of a pack of magazines.
fn header_v10() -> Value {
    json!({
        "format": "atlas-pack-file",
        "version": 10,
        "packId": "magazines-marsh-monthly",
        "title": "Marsh Monthly",
        "packVersion": "1.0.0",
        "detail": "full",
        "exportedAt": "2026-11-21T09:30:00Z",
        "exportedBy": {"name": "Fen Shelf", "version": "3.1.0", "platform": "windows"},
        "pack": {"id": "magazines-marsh-monthly", "title": "Marsh Monthly", "packVersion": "1.0.0",
                 "schemaVersion": 2, "attribution": []},
        "source": {"kind": "unknown", "origin": "magazines"},
        "contents": {},
        "images": {"detail": "full", "files": 0, "bytes": 0, "layers": []},
        "files": 4,
        "bytes": 48_000_100,
        "magazine": {"title": "Marsh Monthly"},
        "issues": [
            {"file": "magazines/2026-04-issue-41.pdf", "title": "Issue 41", "date": "2026-04",
             "pages": 84, "bytes": 48_000_000, "text": "magazines/2026-04-issue-41.text.json",
             "cover": "magazines/2026-04-issue-41.cover.jpg",
             "games": [{"title": "Hollow Marsh", "pages": [[12, 15]]}]}
        ]
    })
}

fn parsed(value: &Value) -> Result<Header, Refusal> {
    parse_header(&value.to_string())
}

#[test]
fn checks_the_issues_a_description_lists() {
    let header = parsed(&header_v10()).unwrap();
    assert_eq!(header.issues.len(), 1);
    assert_eq!(header.issues[0].games[0].pages, vec![[12, 15]]);
    assert_eq!(header.magazine.as_ref().unwrap().title, "Marsh Monthly");

    let damaged = |change: &dyn Fn(&mut Value)| {
        let mut value = header_v10();
        change(&mut value);
        let refusal = parsed(&value).unwrap_err();
        assert_eq!(refusal.kind(), RefusalKind::Damaged, "{refusal}");
        refusal.to_string()
    };
    let unusable = "This pack file lists an issue that cannot be used.";
    assert_eq!(
        damaged(&|v| v["issues"][0]["file"] = json!("magazines/../escape.pdf")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["issues"][0]["text"] = json!("magazines/other.text.json")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["issues"][0]["cover"] = json!("preview/other.jpg")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["issues"][0]["date"] = json!("April")),
        unusable
    );
    assert_eq!(
        damaged(&|v| v["issues"][0]["games"][0]["pages"] = json!([[80, 90]])),
        unusable
    );
    assert_eq!(
        damaged(&|v| {
            let copy = v["issues"][0].clone();
            v["issues"].as_array_mut().unwrap().push(copy);
        }),
        unusable
    );
    assert_eq!(
        damaged(&|v| v
            .as_object_mut()
            .unwrap()
            .remove("magazine")
            .map(drop)
            .unwrap()),
        "This pack file lists issues of no magazine."
    );
    // A run of pages that is not two numbers is not read.
    assert!(damaged(&|v| v["issues"][0]["games"][0]["pages"] = json!([12])).contains("damaged"));

    // A description of an older version carries no magazines.
    let mut older = header_v10();
    older["version"] = json!(9);
    let header = parsed(&older).unwrap();
    assert!(header.issues.is_empty());
    assert_eq!(header.magazine, None);
}

/// What an app that reads an older version does with a pack of
/// magazines: the same check this crate makes of a file newer than it
/// reads, so Atlas 0.25 to 0.27 (versions 8 and 9) refuse it as a file in
/// a newer format, naming the app that made it, and list it with that
/// reason instead of failing.
#[test]
fn a_reader_refuses_a_newer_pack_of_magazines_as_newer() {
    let mut newer = header_v10();
    newer["version"] = json!(VERSION + 1);
    let refusal = parsed(&newer).unwrap_err();
    assert_eq!(refusal.kind(), RefusalKind::NewerFormat);
    assert_eq!(
        refusal.to_string(),
        format!(
            "This pack file was made by Fen Shelf 3.1.0 in a newer format (version {}). A newer version of this app is needed to open it.",
            VERSION + 1
        )
    );
}
