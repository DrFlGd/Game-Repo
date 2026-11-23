//! Magazines made up here: a PDF with its text read, a CBZ whose first page is a large
//! picture, a CBR written byte by byte as RAR 4 (the guidebooks' fixtures), an empty file and
//! a PDF whose pages can't be counted; read back with Atlas's own crate.

use super::*;
use crate::guidebooks::tests::{cbz, pdf, rar};
use std::io::Read;

fn picture(w: u32, h: u32, format: ::image::ImageFormat) -> Vec<u8> {
    let img = ::image::RgbImage::from_fn(w, h, |x, y| ::image::Rgb([(x * 7 % 251) as u8, (y * 13 % 241) as u8, ((x ^ y) % 239) as u8]));
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, format).unwrap();
    out.into_inner()
}

fn millis(p: &Path) -> i64 {
    std::fs::metadata(p).unwrap().modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
}

/// The Game Boy fixture's library with a magazine of five issues: a PDF with a chosen cover,
/// its text read and two games (one ticked, linked to a pack), a CBZ whose first page is a
/// picture larger than a cover, a CBR with no number and a year only, an empty PDF and one that
/// isn't one.
fn library(name: &str) -> (PathBuf, Library, Index, PathBuf) {
    let (dir, lib, ix) = crate::collection::tests::fixture(name);
    let mag = magazines::folder(&lib).join("Marsh Monthly");
    pdf(&mag.join("Marsh Monthly 041 (2026-04).pdf"), &["Contents", "Delta reviewed", "Delta's maps"]);
    std::fs::create_dir_all(mag.join("covers")).unwrap();
    std::fs::write(mag.join("covers/041.png"), picture(800, 600, ::image::ImageFormat::Png)).unwrap();
    let text = crate::magtext::text_path(&mag.join("Marsh Monthly 041 (2026-04).pdf")).unwrap();
    std::fs::create_dir_all(text.parent().unwrap()).unwrap();
    let pdf_path = mag.join("Marsh Monthly 041 (2026-04).pdf");
    let kept = json!({ "size": std::fs::metadata(&pdf_path).unwrap().len(), "modified": millis(&pdf_path), "read": "2026-11-23", "pages": ["Contents", "Delta reviewed", "Delta's maps", "a page more than it has"] });
    std::fs::write(&text, kept.to_string()).unwrap();
    cbz(&mag.join("Marsh Monthly 042 (2026-05).cbz"), &[
        ("p01.png", picture(1024, 1536, ::image::ImageFormat::Png)),
        ("p02.jpg", picture(20, 30, ::image::ImageFormat::Jpeg)),
        ("p03.jpg", picture(20, 30, ::image::ImageFormat::Jpeg)),
        ("p10.jpg", picture(20, 30, ::image::ImageFormat::Jpeg)),
    ]);
    rar(&mag.join("Marsh Monthly Special (2026).cbr"), &[("b.jpg", crate::atlaspack::tests::jpeg(2)), ("a.jpg", crate::atlaspack::tests::jpeg(1))]);
    std::fs::write(mag.join("Marsh Monthly 043 (2026-06).pdf"), b"").unwrap();
    std::fs::write(mag.join("Marsh Monthly 044 (2026-07).pdf"), b"%PDF-1.4\nnot a page in it").unwrap();
    let game = |key: &str, title: &str, pages: &[u32], ticked: bool| json!({ "key": key, "title": title, "platform": "gb", "pages": pages, "ticked": ticked });
    let m = json!({ "title": "Marsh Monthly", "publisher": "Reed Press", "issues": [
        { "number": "41", "date": "2026-04", "file": "Marsh Monthly 041 (2026-04).pdf", "cover": "covers/041.png", "size": 1, "added": "2026-11-23",
          "games": [game("gb/Delta (World)", "Delta", &[3, 2], true), game("gb/Beta (Japan)", "Beta", &[1], false)] },
        { "number": "042", "date": "2026-05", "file": "Marsh Monthly 042 (2026-05).cbz", "size": 1, "added": "2026-11-23",
          "games": [game("gb/Alpha (USA)", "Alpha", &[1, 2, 4, 9], true), game("gb/Gamma (USA) (Beta)", "Gamma", &[], true)] },
        { "date": "2026", "file": "Marsh Monthly Special (2026).cbr", "size": 1, "added": "2026-11-23" },
        { "number": "43", "date": "2026-06", "file": "Marsh Monthly 043 (2026-06).pdf", "size": 0, "added": "2026-11-23" },
        { "number": "44", "date": "2026-07", "file": "Marsh Monthly 044 (2026-07).pdf", "size": 1, "added": "2026-11-23" },
    ] });
    crate::config::write_json(&mag.join(magazines::MANIFEST), &m).unwrap();
    // Delta has a pack of maps linked to it: its id is Delta's id in Atlas
    let packs = json!({ "packs": { "delta-example-maps": { "games": [{ "key": "gb/Delta (World)", "title": "Delta", "platform": "gb" }], "added": "2026-11-19" } } });
    std::fs::create_dir_all(maps::folder(&lib)).unwrap();
    crate::config::write_json(&maps::folder(&lib).join("packs.json"), &packs).unwrap();
    (dir, lib, ix, mag)
}

fn wanted_now(ix: &Index, lib: &Library) -> Wanted {
    let all = all(ix, lib).unwrap();
    assert_eq!(all.len(), 1);
    all.into_iter().next().unwrap()
}

/// An entry of a pack file, read with the zip crate.
fn entry(pack: &Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(pack).unwrap()).unwrap();
    let mut f = z.by_name(name).unwrap_or_else(|_| panic!("no {name} in the pack"));
    let mut out = vec![];
    f.read_to_end(&mut out).unwrap();
    out
}

fn names(pack: &Path) -> Vec<String> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(pack).unwrap()).unwrap();
    (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect()
}

#[test]
fn a_magazines_pack_is_made_once_made_again_on_a_change_and_read_back_by_atlass_crate() {
    let (dir, lib, ix, mag) = library("magpack-made");
    let w = wanted_now(&ix, &lib);
    assert_eq!((w.title.as_str(), w.publisher.as_deref(), w.pack_id.clone()), ("Marsh Monthly", Some("Reed Press"), pack_id_for("Marsh Monthly")));
    // read without opening the files: the empty one is left out now, the one that isn't a PDF
    // when its pages are counted
    assert_eq!(w.items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Marsh Monthly Special (2026)", "Issue 41", "Issue 42", "Issue 44"]);
    assert_eq!(w.left, [("Issue 43".to_string(), "It's empty, so it can't go to Atlas.".to_string())]);
    // the year alone isn't a date the contract takes; "042" is the number 42 (Issue 42)
    let special = &w.items[0];
    assert_eq!((special.date.clone(), special.number, special.kind.as_str()), (None, None, "cbr"));
    assert_eq!((w.items[2].date.as_deref(), w.items[2].number), (Some("2026-05"), Some(42)));
    // only the games ticked, each with its system's name and, for Delta, its pack's id
    let g = &w.items[1].games;
    assert_eq!(g.iter().map(|g| (g.title.as_str(), g.system.as_deref(), g.game_id.as_deref())).collect::<Vec<_>>(), [("Delta", Some("Game Boy"), Some("delta-example-maps"))]);

    let cache = Cache::new(dir.join("server").join(DIR));
    let no = AtomicBool::new(false);
    let made = make(&cache, &w, &no).unwrap();
    assert_eq!((made.items, made.pages), (3, 2 + 3 + 4));
    assert_eq!(made.left.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), ["Issue 43", "Issue 44"], "{:?}", made.left);
    assert!(made.left[1].1.ends_with("It can't go to Atlas."), "{:?}", made.left[1]);
    let file = cache.path(&w.pack_id, &made);
    assert!(file.starts_with(dir.join("server").join("magazines").join(&w.pack_id)), "never the library");

    // read back by Atlas's own crate: format 10, the magazine, its issues in order
    let h = atlas_pack::read_header(&file).unwrap();
    assert_eq!((h.version, h.pack_id.as_str(), h.origin()), (10, w.pack_id.as_str(), Some(atlas_pack::Origin::Magazines)));
    assert_eq!(h.magazine, Some(MagazineFacts { title: "Marsh Monthly".into(), publisher: Some("Reed Press".into()) }));
    assert_eq!((h.exported_by.name.as_str(), made.updated.clone()), (guidebooks::WRITER, h.pack.updated.clone().unwrap()));
    let said: Vec<(&str, Option<&str>, Option<u32>, u32, bool, bool)> = h.issues.iter().map(|i| (i.title.as_str(), i.date.as_deref(), i.number, i.pages, i.text.is_some(), i.cover.is_some())).collect();
    assert_eq!(said, [
        ("Marsh Monthly Special (2026)", None, None, 2, false, false),
        ("Issue 41", Some("2026-04"), Some(41), 3, true, true),
        ("Issue 42", Some("2026-05"), Some(42), 4, false, true),
    ]);
    // a CBR goes as a CBZ, its pages in order
    let special = &h.issues[0];
    assert!(special.file.ends_with(".cbz"), "{}", special.file);
    let inner = dir.join("special.cbz");
    std::fs::write(&inner, entry(&file, &special.file)).unwrap();
    assert_eq!(magazines::pages(&inner).unwrap(), ["0001.jpg", "0002.jpg"]);
    assert_eq!(magazines::page(&inner, 0).unwrap().0, crate::atlaspack::tests::jpeg(1));
    // the games: page runs within the issue, the game with no pages on all of them
    let games: Vec<(&str, Option<&str>, Option<&str>, Vec<[u32; 2]>)> = h.issues.iter().flat_map(|i| i.games.iter()).map(|g| (g.title.as_str(), g.system.as_deref(), g.game_id.as_deref(), g.pages.clone())).collect();
    assert_eq!(games, [
        ("Delta", Some("Game Boy"), Some("delta-example-maps"), vec![[2, 3]]),
        ("Alpha", Some("Game Boy"), None, vec![[1, 2], [4, 4]]),
        ("Gamma", Some("Game Boy"), None, vec![[1, 4]]),
    ]);
    // the text, cut to the issue's pages, in a guidebook's text format
    let text = atlas_pack::parse_guidebook_text(&entry(&file, h.issues[1].text.as_deref().unwrap()), 3).unwrap();
    assert_eq!(text.pages.iter().map(|p| (p.page, p.text.as_str())).collect::<Vec<_>>(), [(1, "Contents"), (2, "Delta reviewed"), (3, "Delta's maps")]);
    // the covers: JPEGs of at most 512 KB, 512 pixels on their longest side (the chosen one
    // for issue 41, the first page for 042)
    for (issue, size) in [(1, (512, 384)), (2, (341, 512))] {
        let bytes = entry(&file, h.issues[issue].cover.as_deref().unwrap());
        assert!(bytes.starts_with(&[0xFF, 0xD8, 0xFF]) && bytes.len() as u64 <= MAX_COVER_BYTES);
        let img = ::image::load_from_memory(&bytes).unwrap();
        assert_eq!((img.width(), img.height()), size, "issue {issue}");
    }
    // the entries in the contract's order: the description, manifest.json and
    // magazines.json, the covers, then each issue's text and file
    let order = names(&file);
    let covers: Vec<String> = h.issues.iter().filter_map(|i| i.cover.clone()).collect();
    let mut want = vec!["atlaspack.json".to_string(), "manifest.json".into(), "magazines.json".into()];
    want.extend(covers);
    for i in &h.issues {
        want.extend(i.text.clone());
        want.push(i.file.clone());
    }
    assert_eq!(order, want);
    let list: atlas_pack::MagazineList = serde_json::from_slice(&entry(&file, "magazines.json")).unwrap();
    assert_eq!((list.format.as_str(), list.version, list.issues.len()), ("atlas-magazines", 1, 3));

    // made once: asked again, the same file
    let again = make(&cache, &wanted_now(&ix, &lib), &no).unwrap();
    assert_eq!((again.file.clone(), again.sha1.clone()), (made.file.clone(), made.sha1.clone()));
    assert_eq!(cache.current_of(&w.pack_id, &w.recipe()), Some(made.clone()));

    // a change (a game ticked) makes it again, dated after the one before
    let mut m: Value = serde_json::from_slice(&std::fs::read(mag.join(magazines::MANIFEST)).unwrap()).unwrap();
    m["issues"][0]["games"][1]["ticked"] = json!(true);
    crate::config::write_json(&mag.join(magazines::MANIFEST), &m).unwrap();
    let w2 = wanted_now(&ix, &lib);
    assert_ne!(w2.recipe(), w.recipe());
    assert_eq!(cache.current_of(&w2.pack_id, &w2.recipe()), None);
    let second = make(&cache, &w2, &no).unwrap();
    assert_ne!(second.file, made.file);
    let instant = |s: &str| gamerepo_wire::packs::instant(s).unwrap();
    assert!(instant(&second.updated) > instant(&made.updated), "{} after {}", second.updated, made.updated);
    assert_eq!(atlas_pack::read_header(&cache.path(&w2.pack_id, &second)).unwrap().issues[1].games.len(), 2);
    // and a new title (its folder keeps its id), and only the newest two files are kept
    m["title"] = json!("Marsh Monthly Weekly");
    crate::config::write_json(&mag.join(magazines::MANIFEST), &m).unwrap();
    let w3 = wanted_now(&ix, &lib);
    assert_eq!((w3.pack_id.as_str(), w3.title.as_str()), (w.pack_id.as_str(), "Marsh Monthly Weekly"));
    let third = make(&cache, &w3, &no).unwrap();
    assert_eq!(atlas_pack::read_header(&cache.path(&w3.pack_id, &third)).unwrap().magazine.unwrap().title, "Marsh Monthly Weekly");
    let files: Vec<String> = std::fs::read_dir(dir.join("server/magazines").join(&w.pack_id)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".atlaspack")).collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(!cache.path(&w.pack_id, &made).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_magazines_page_says_its_pack_and_the_issues_that_cant_go() {
    let (dir, lib, ix, _) = library("magpack-page");
    let server = dir.join("server");
    let mut v = magazines::overview(&ix, &lib).unwrap();
    annotate(&ix, &lib, &server, &mut v).unwrap();
    let m = &v["magazines"][0];
    assert_eq!((m["atlas"]["pack_id"].as_str(), m["atlas"]["issues"].as_u64(), m["atlas"]["size"].clone()), (Some(pack_id_for("Marsh Monthly").as_str()), Some(4), Value::Null), "{m}");
    assert_eq!(m["atlas"]["file"], "Marsh Monthly - Magazines.atlaspack");
    let left = |m: &Value| m["issues"].as_array().unwrap().iter().filter(|i| i["atlas_left"].is_string()).map(|i| i["file"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(left(m), ["Marsh Monthly 043 (2026-06).pdf"]);
    // once Game Repo Server has made it: its size, and the issue whose pages can't be counted
    let made = make(&Cache::new(server.join(DIR)), &wanted_now(&ix, &lib), &AtomicBool::new(false)).unwrap();
    let mut v = magazines::overview(&ix, &lib).unwrap();
    annotate(&ix, &lib, &server, &mut v).unwrap();
    let m = &v["magazines"][0];
    assert_eq!((m["atlas"]["issues"].as_u64(), m["atlas"]["size"].as_u64()), (Some(3), Some(made.size)));
    assert_eq!(left(m), ["Marsh Monthly 043 (2026-06).pdf", "Marsh Monthly 044 (2026-07).pdf"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_contracts_bounds_leave_out_what_cant_go() {
    // runs of pages: sorted, joined, within the issue
    assert_eq!(runs(&[12, 13, 14, 15, 40, 3, 3, 0, 99], 50), [[3, 3], [12, 15], [40, 40]]);
    assert!(runs(&[], 10).is_empty());
    // at most 1,000 issues: the newest go, the oldest are left out and said
    let (dir, lib, _ix) = crate::collection::tests::fixture("magpack-bounds");
    let mag = magazines::folder(&lib).join("Big Mag");
    std::fs::create_dir_all(&mag).unwrap();
    let mut issues = vec![];
    for n in 1..=1002u32 {
        let file = format!("Big Mag {n:04}.pdf");
        std::fs::write(mag.join(&file), b"%PDF-1.4").unwrap();
        let games: Vec<Value> = if n == 3 { (0..250).map(|g| json!({ "key": format!("gb/G{g}"), "title": format!("Game {g}"), "platform": "gb", "pages": [1], "ticked": true })).collect() } else { vec![] };
        issues.push(json!({ "number": n.to_string(), "file": file, "size": 8, "added": "2026-11-23", "games": games }));
    }
    // a title too long is cut to the contract's 200 characters
    crate::config::write_json(&mag.join(magazines::MANIFEST), &json!({ "title": "B".repeat(300), "issues": issues })).unwrap();
    let (d, m) = magazines::list(&lib).into_iter().next().unwrap();
    let w = wanted(&d, &m, &GameIds::default()).unwrap();
    assert_eq!((w.items.len(), w.left.len(), w.title.chars().count()), (1000, 2, 200));
    assert_eq!((w.items[0].number, w.left[0].0.as_str()), (Some(3), "Issue 1"));
    assert!(w.left[0].1.contains("at most 1,000 issues"));
    // at most 200 games an issue
    assert_eq!(w.items[0].games.len(), 200);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_magazines_pack_id_is_safe_stable_and_its_own() {
    let a = pack_id_for("Marsh Monthly");
    assert!(a.starts_with("magazines-marsh-monthly-") && a.len() <= 64 && atlas_pack::is_pack_id(&a) && is_pack_id(&a), "{a}");
    assert_eq!(a, pack_id_for("Marsh Monthly"));
    assert_eq!(a, pack_id_for("MARSH MONTHLY"), "a folder's name without case, as Windows has it");
    assert_ne!(a, pack_id_for("Marsh Monthly 2"));
    assert_ne!(a, pack_id_for("Marsh-Monthly"), "two names that read alike have their own");
    let long = pack_id_for(&"Very Long Magazine Name ".repeat(10));
    assert!(long.len() <= 64 && is_pack_id(&long), "{long}");
    let odd = pack_id_for("Ünïcödé ★");
    assert!(is_pack_id(&odd) && odd.len() <= 64, "{odd}");
    assert!(!is_pack_id(&crate::guidebooks::pack_id_for("gb/Delta (World)")));
}
