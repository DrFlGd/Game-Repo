# Changelog

What changed in each release of Game Repo, newest first. The **Unreleased**
section becomes the next release's notes, so it's written for the people using
the app. The **For agents** section is the map for whoever works on the code next.

---

## For agents

### What this is

A Windows desktop app (Tauri 2) for organising large video game collections: auditing ROM sets against DAT files, building and exporting collections (ES-DE, Steam), importing PC games into a common layout, and scraping each game's details, media and manuals, with the user's own documents attached. The owner ("DrFlGd", Jerred) steers by phases; the plan, every decision taken (D1–D17; D18 open; D19–D20 proposed and followed) and the phases are in `docs/PLAN.md`.

The code started as a trimmed copy of [Model Library](https://github.com/DrFlGd/Model-Library) (same owner, same stack): its Tauri shell, its one-command-table backend, its library folder, its jobs and its Preact interface. Much more of it is meant to come across as the phases need it (the browser views, the sorting workspace for the PC inbox, journals and Undo, ZIP/7z reading, the side viewer for documents): look there before writing something from scratch. `docs/PLAN.md` section 5 lists what carries over.

### Where things are

| Path | What |
| --- | --- |
| `web/` | The front end (no build step). `app.js`: start-up and routing (and following the scans the folder watcher starts). `web/ui/`: Preact + htm islands. `shell.js` mounts them; `sidebar.js` the menu; `chrome.js` the menu's head (the cartridge mark, drawn per skin), the Theme button and the status bar; `home.js` Home (Scan, the platform bars, folders and what needs attention, the index card, what's coming) and the first-start screen; `audit.js` the Audit page (platform and DAT pickers, the counts as tabs, paged lists with search, fixdat, reports); `games.js` the Phase 1 actions (folders with games, scanning, importing DATs and asking for a platform, making a DAT from a set; `followAutoScans` shows the watcher's scans); `organise.js` the Organise… dialog and its preview, Rename these, Set aside duplicates, and running, undoing and finishing changes; `coming.js` the pages still to come (Collections, PC games, Metadata), each replaced by the real page in its phase; `settings.js` (also Folders with games, how scans read ZIPs, Watch for changes, DAT files, Naming, History); `about.js` (versions, the changelog read from `CHANGELOG.md` beside the page, the roadmap, the library layout, licences); `layout.js` the command bar, menu button, view switcher, sort menu, search note and edit box (from Model Library, ready for Phase 1's pages); `actions.js` the menu and the window's keys; `dialogs.js` the dialog shell, Ask, Confirm, "which platform is this DAT for?", Make a DAT and Organise; `library.js` opening and renaming the library, the index, messages with Undo, following jobs; `state.js` shared state, preferences (`gr-ui`), skins and themes; `icons.js` inline icons. `platform.js` / `platform-desktop.js`: the seam between the page and the backend. |
| `desktop/core/` | Rust, no GUI (`gamerepo-core`): `api.rs` (the command table, `App::call`; jobs the page polls with `job`/`jobs`/`job_cancel`, `auto` for the watcher's scans; one change at a time, never during a scan; `organise_*`, `change_*` and `naming*` in `organise_call`; the folder watcher kept in step with the roots by `ensure_watch`; `library://` file serving with ranges), `library.rs` (the library folder: `_library/library.json` with format and id, read-only for newer formats), `index.rs` (the SQLite index per library on this computer, `<data dir>/index/<library id>.sqlite`, its migrations), `config.rs` (what stays on this computer: the open library, recent ones, prefs, scan settings; `write_json` with sorted keys), `roots.rs` (folders with games, in library.json), `platforms.rs` (the platform table, kept in step with `docs/platforms.md`; how DATs and files find their platform), `hash.rs` (CRC32+MD5+SHA1 in one read; headers and N64 byte orders), `scan.rs` (walk, diff, moves, parallel hashing, archives quick/full, damaged ZIPs; `Options.only` scans just the places the watcher reports), `archive.rs` (ZIP, 7z and RAR: listing with their CRCs, reading in full, locked ones), `torrentzip.rs` (writing ZIPs TorrentZip's way; the test compares bytes with `tests/fixtures/torrentzip.py`), `organise.rs` (Phase 2a: planning a change from the index, the preview, running it with every write checked, the journal in `_library/journal/<id>.json` + `.log`, Undo, finishing an interrupted change, set-aside files in `.gamerepo-set-aside/<id>/`, naming templates in library.json `naming`), `watch.rs` (the folder watcher, from Model Library's; network drives left out), `dat.rs` (Logiqx and ClrMamePro, name fields, import into `_library/dats/<platform>/`, versions, `sync_library` rebuilding the index from the library's DATs), `audit.rs` (matching in SQL, summaries cached in `index_meta`, the paged lists, fixdat, reports, Home; the end-to-end and timing tests), `makedat.rs` (a DAT from a scanned set), `starter.rs` (starter DATs from libretro's copy of No-Intro's, D17; its address is `dat_source` in config.json). `src/bin/gamerepo-cli.rs`: `library-info`, `index-info`, `serve` (the backend over HTTP, for the tests), and the library commands `roots`, `scan`, `dat import|list|starter`, `audit`, `make-dat`, `platforms`, `organise`, `duplicates`, `history`, `undo`, `finish`, `set-aside empty` (on the app's own library and index unless `--library`/`--home`). |
| `desktop/src-tauri/` | The Tauri app (`game-repo`): a thin layer over `api.rs`, plus dialogs, the file manager and the `library://` protocol. Version in `tauri.conf.json`. Icons in `icons/` (the cartridge mark). |
| `tools/` | `build_desktop.py` (web/ to `build/desktop/ui`, fonts, and `CHANGELOG.md`, `THIRD_PARTY.md` and `build-info.json` for About), `set_version.py`, `vendor_preact.py`. |
| `tests/` | `desktop_page.py` (acceptance test: the page and the real backend through `gamerepo-cli serve` and `tauri_shim.js`, in Chromium), `desktop_ui.py` (WebDriver on the built app), `fixtures/` (synthetic files only; `make_fixtures.py` writes a small ROM set, DATs for it and the expected audit). |
| `.github/workflows/` | `desktop.yml` (the Windows app, tests and release; Linux only on request), `vendor.yml` (vendored crates for offline builds). |

### Conventions

- **Phases and releases.** The repository keeps `0.<phase>.0` in `desktop/src-tauri/tauri.conf.json` and `desktop/Cargo.toml` (`tools/set_version.py 0.1.0` stamps them; bump it when a phase starts). CI picks the next free `0.<phase>.<n>` from the release tags and publishes `v<version>` from `main` only when the Windows app, the Rust tests and both acceptance tests pass. The release notes are the **Unreleased** section below, so write it for the user.
- **Windows only for now** (D3). Keep code portable all the same; the Linux build runs only from *Run workflow* with "Linux too" ticked.
- **Plan first.** For a new phase or a big feature, write the design into `docs/PLAN.md` before code ("Phase N design"), and afterwards a "Phase N notes" section recording what was built and where it differs.
- **Performance on large sets** is the first requirement (D1): hundreds of thousands of files. Put lookups and joins in SQLite with indexes, stream rather than load, hash each file once and remember it by size and modified time, and run anything long as a job with progress and Stop.
- **The library folder is the source of truth** and must make sense without the app. Never write a database into it (the index is per computer and rebuilt from the files and DATs); keep unknown keys when rewriting its JSON; write JSON with `config::write_json` (sorted keys).
- **The index's schema** grows by appending steps to `index::MIGRATIONS`; a step that has shipped is never edited. An index from a newer app, or another library's, is set aside as `<id>.old.sqlite` and made again.
- **Originals are never touched by default** (D5): renaming in place and moving into the library are explicit, journalled choices with Undo.
- **Scraped data, media and documents are keyed by game id** (ROM hash → game; PC games by their own id), never by file name (D13).
- **External tools** (chdman, dolphin-tool, maxcso, 7z, innoextract) are found in the app's `tools/` folder, on PATH or at a path set in Settings; nothing is downloaded (D14).
- **Text:** plain words, no jargon in the interface; British spellings in the UI (organise, colour), as in Model Library.
- **Tests grow with each phase.** Add checks to `desktop_page.py` for every user-visible feature, Rust tests for the core, and `desktop_ui.py` when it needs the real app. Never commit real ROMs or DATs; make synthetic ones.
- **Commits:** a subject saying what changed for the user, a body with the details. Add a line under **Unreleased** for every user-visible change.

### Working in a cloud session

- **Rust:** crates.io was reachable on 16 October 2026 and the whole workspace resolved online (`Cargo.lock` is committed). If it's blocked, `vendor.yml` pushes `gamerepo-core`'s dependencies to the `crates-vendor` branch whenever `desktop/core/Cargo.toml` changes: build offline from it with a workspace holding only `core` and `cargo <cmd> --offline --config <branch>/vendor-config.toml`.
- **Build and test only the core locally:** `cd desktop && cargo test -p gamerepo-core && cargo build --release -p gamerepo-core`. The timing checks are ignored by default: `cargo test --release -p gamerepo-core -- --ignored timing --nocapture` (CI runs them).
- **Try the CLI on synthetic data:** `python3 tests/fixtures/make_fixtures.py <dir>`, then `gamerepo-cli roots add <dir>/Roms --home <h> --library <lib>`, `scan`, `dat import <dir>/dats/gb.dat`, `audit gb` with the same `--home`/`--library`. The Tauri app needs WebView libraries the session doesn't have; it builds only in CI.
- **Acceptance test locally:** `python3 tools/build_desktop.py --out build/desktop`, then `python3 tests/desktop_page.py --cli desktop/target/release/gamerepo-cli --ui build/desktop/ui --home <tmp> --out <shots> --chromium /opt/pw-browsers/chromium` (`pip install playwright`; use the pre-installed Chromium, don't download one).
- **GitHub from the session:** GraphQL may be blocked, so open PRs through the REST API (`gh api repos/DrFlGd/Game-Repo/pulls --method POST`). CI reports land on the `desktop-ci-windows` branch (logs, acceptance screenshots in `page/`); fetch it to read results.

### Known limitations now

- Cartridge systems only: disc systems, CHD, RVZ and CSO come in Phase 2b. RAR is read, never written; 7z is read and not written yet.
- ZIPs the app writes are at most 4 GB a file and 65,535 files (no ZIP64), plenty for cartridge games.
- Only the newest change can be undone (then the one before it, and so on); emptying what a change set aside ends its Undo.
- Renamed files are read again on the next scan (a move to another folder keeps its hashes; the rename can't be told apart from a different file safely).
- DATs whose platform isn't in `platforms.rs` (TOSEC's names, for one) are asked about once per DAT name; the answer is kept in library.json.
- A DAT ROM with no CRC32 isn't matched (every Logiqx and ClrMamePro DAT seen so far gives one).
- Two skins (Drafting Sheet, the default, and Classic). Model Library's Parts Bin and Layer Line were 3D-printing themed and weren't brought across.
- `web/styles.css` still holds Model Library's rules for views not built here yet (the browser grid, the workspace, the inspector); they're kept for the phases that bring those views, and pruned once those pages exist.
- Installers aren't code-signed, and there's no auto-update.

### Next (roadmap)

Phase 1 is released (v0.1.0); what's left of it is Jerred's check on a real No-Intro set. Phase 2 is designed in `docs/PLAN.md` ("Phase 2 design") and built in two halves (D19): 2a (built; "Phase 2a notes" in the plan), organising sets by DAT name in the three modes of D5 with a preview, a journal and Undo, plus 7z/RAR reading and a folder watcher; next 2b, disc systems (Redump; CHD, RVZ and CSO audited as the games they hold) and conversion with verification. Phase 9 (added 2026-10-16) serves games to Android, Linux and handheld devices on the home network (`docs/PLAN.md` 3.10); D18 asks which devices come first.

---

## Unreleased

- **Disc systems**: PlayStation, PlayStation 2, PSP, Saturn, Sega CD, Dreamcast, PC Engine CD, Neo Geo CD, 3DO, PC-FX, Jaguar CD, CD-i, GameCube, Wii and Xbox, audited against Redump's DATs from cue/bin, gdi and iso sets. A cue sheet that isn't Redump's but names the right tracks counts (*cue differs*), and renaming the set rewrites it to name the new files.
- **CHD, CSO, ZSO and RVZ** are audited as the games they hold: a CHD's tracks as the bins Redump lists, a CSO's or ZSO's ISO, and an RVZ's disc through dolphin-tool (until it's found, RVZ files wait to be checked).
- **Convert…** a game, a selection or a whole platform to its preferred format (CHD for CD and DVD systems, RVZ for GameCube and Wii, CSO or ZSO for PSP, ZIP for cartridges), into a folder or in place, and **Extract** back to cue/bin or ISO. Every result is read back and compared with the files it came from before anything is replaced, and Undo puts it all back. CHDs are written by the app itself, the same data chdman writes, so no tool is needed for them.
- **Get starter DATs** now also fetches libretro's copies of Redump's DATs for the disc systems. They list one track of each disc, so import Redump's own DATs for a full check.
- **Command line**: `convert <platform> [--to <format>] [--out <folder>] [--game <name>]… [--dry-run]`, `extract <platform>`, `formats [<platform> <format>]` and `tools [<tool> <path> | --clear <tool>]`.

## 0.2.0 (2026-10-16)

- **Plans**: Phase 2 is designed: organising sets by their DAT names with a preview and Undo, then disc systems and converting to CHD, RVZ and CSO.
- **Organise…** (Audit): sort a platform's set by its DAT in one of three ways: copy it to another folder (hardlinked on the same drive, so it takes no space), rename the files where they are, or move them into a folder for the platform in the library. Choose to keep files as they are, make a ZIP for each game, or unpack them; a preview shows every step, what's in the way and whether there's space, and nothing changes until you press Run. Every file written is read back and checked.
- **Undo**: the message after a change has Undo, and *Settings › History* lists the changes, undoes the newest, and finishes or puts back one that was interrupted (say, by a power cut). Files a change replaces are never deleted: they're set aside in a hidden `.gamerepo-set-aside` folder until you empty it there.
- **Rename these** on the Misnamed list and **Set aside duplicates** on the Duplicates list, each with the same preview and Undo.
- **TorrentZip**: ZIPs the app writes are byte-for-byte the same as other collectors' TorrentZip tools make, so they match torrents and other people's sets.
- **Naming** (Settings): how organised games are named for each platform: the DAT's name, or a shorter one from its title, region, languages and revision, and optionally a folder for each game.
- **7z and RAR**: scans and audits look inside them as they do ZIPs; password-protected ones are listed and skipped.
- **Watch for changes** (Settings, on by default): while the app is open, files added, changed or removed in your folders with games are read a few seconds later, without a scan. Network drives are still read when you scan.
- **Command line**: `organise`, `duplicates`, `history`, `undo`, `finish` and `set-aside empty`.

## 0.1.0 (2026-10-16)

- **Folders with games** (Settings): add the folders your ROMs are in, anywhere on this computer or a network drive, optionally marked with their platform. They're kept in the library; one that's somewhere else on another computer can be pointed there. Nothing in them is ever changed.
- **Scan** (Home or Audit): every file is read once and hashed (CRC32, MD5 and SHA1 together), files inside ZIPs too, and a second scan reads only what's new or changed. ROMs with headers (iNES, FDS, Atari 7800, Lynx, SNES copier) and N64 ROMs in other byte orders are matched as the DATs list them. ZIPs are matched by the checksums they record unless you choose *Unpack and hash every file*; how many files are read at once is a setting too.
- **DAT files** (Settings): import No-Intro, Redump, TOSEC or any Logiqx or ClrMamePro DAT, zipped or not. The platform is worked out from the DAT's name, or asked once and remembered. Each DAT is copied into the library; importing a newer version keeps the older one, and you can choose which is in use.
- **Audit**: for each platform, the games you have, partly have and are missing, with 1G1R completeness, and the files that are named right, misnamed (with the right name), bad dumps, unknown or duplicates, each searchable and with *Show in folder*. A platform can have several DATs; the one you pick stays its DAT.
- **Reports and fixdats**: *Export fixdat* writes what's missing as a DAT; *Copy report* and *Save report* (text or CSV) write the audit out.
- **Get starter DATs**: one click downloads the No-Intro DATs for the cartridge systems from libretro's public copy and imports them for the systems that have none yet. They can be a little behind No-Intro's newest; a newer DAT you import becomes the version in use.
- **Make a DAT from a set**: write a DAT of a folder you've scanned, and import it to audit against.
- **Home** shows a bar per platform, the last scan and what needs attention (folders not found here, files without a DAT or platform).
- **Faster rescans**: scanning folders where nothing changed no longer writes to the index.
- **Roadmap** (About): serving games from the app to Android, Linux and handheld devices on the home network is planned as Phase 9.
- **Command line**: `gamerepo-cli roots`, `scan [--full]`, `dat import|list`, `audit <platform> [--list …] [--fixdat …] [--report txt|csv]` and `make-dat` work on the app's library.

## 0.0.0 (2026-10-16)

The first version of Game Repo: the app itself, with its library and settings. The features come in the next phases; the menu shows what each brings.

- On first start the app asks where your library should go. The library is an ordinary folder: the app adds only a `_library` folder to it, so a folder already holding your games can be used as it is, and nothing in it is changed.
- The app keeps an index of each library on this computer (not in the library), ready for the hundreds of thousands of files a full set has. Settings shows where it is, and Home's Library menu checks it.
- Rename the library (with Undo), open another one, or go back to one opened before from Settings.
- Two looks, Drafting Sheet and Classic, each in light, dark and night themes.
- About shows the version, what's changed, what's coming in each phase, how a library is laid out and the licences.
