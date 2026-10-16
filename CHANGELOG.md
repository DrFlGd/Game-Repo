# Changelog

What changed in each release of Game Repo, newest first. The **Unreleased**
section becomes the next release's notes, so it's written for the people using
the app. The **For agents** section is the map for whoever works on the code next.

---

## For agents

### What this is

A Windows desktop app (Tauri 2) for organising large video game collections: auditing ROM sets against DAT files, building and exporting collections (ES-DE, Steam), importing PC games into a common layout, and scraping each game's details, media and manuals, with the user's own documents attached. The owner ("DrFlGd", Jerred) steers by phases; the plan, every decision taken (D1–D14) and the phases are in `docs/PLAN.md`.

The code started as a trimmed copy of [Model Library](https://github.com/DrFlGd/Model-Library) (same owner, same stack): its Tauri shell, its one-command-table backend, its library folder, its jobs and its Preact interface. Much more of it is meant to come across as the phases need it (the browser views, the sorting workspace for the PC inbox, journals and Undo, ZIP/7z reading, the side viewer for documents): look there before writing something from scratch. `docs/PLAN.md` section 5 lists what carries over.

### Where things are

| Path | What |
| --- | --- |
| `web/` | The front end (no build step). `app.js`: start-up and routing. `web/ui/`: Preact + htm islands. `shell.js` mounts them; `sidebar.js` the menu; `chrome.js` the menu's head (the cartridge mark, drawn per skin), the Theme button and the status bar; `home.js` Home (the index card, what's coming) and the first-start screen; `coming.js` the pages still to come (Audit, Collections, PC games, Metadata), each replaced by the real page in its phase; `settings.js`; `about.js` (versions, the changelog read from `CHANGELOG.md` beside the page, the roadmap, the library layout, licences); `layout.js` the command bar, menu button, view switcher, sort menu, search note and edit box (from Model Library, ready for Phase 1's pages); `actions.js` the menu and the window's keys; `dialogs.js` the dialog shell, Ask and Confirm; `library.js` opening and renaming the library, the index, messages with Undo, following jobs; `state.js` shared state, preferences (`gr-ui`), skins and themes; `icons.js` inline icons. `platform.js` / `platform-desktop.js`: the seam between the page and the backend. |
| `desktop/core/` | Rust, no GUI (`gamerepo-core`): `api.rs` (the command table, `App::call`; jobs the page polls with `job`/`jobs`/`job_cancel`; `library://` file serving with ranges), `library.rs` (the library folder: `_library/library.json` with format and id, read-only for newer formats), `index.rs` (the SQLite index per library on this computer, `<data dir>/index/<library id>.sqlite`, its migrations), `config.rs` (what stays on this computer: the open library, recent ones, prefs; `write_json` with sorted keys). `src/bin/gamerepo-cli.rs`: `library-info`, `index-info`, `serve` (the backend over HTTP, for the tests). |
| `desktop/src-tauri/` | The Tauri app (`game-repo`): a thin layer over `api.rs`, plus dialogs, the file manager and the `library://` protocol. Version in `tauri.conf.json`. Icons in `icons/` (the cartridge mark). |
| `tools/` | `build_desktop.py` (web/ to `build/desktop/ui`, fonts, and `CHANGELOG.md`, `THIRD_PARTY.md` and `build-info.json` for About), `set_version.py`, `vendor_preact.py`. |
| `tests/` | `desktop_page.py` (acceptance test: the page and the real backend through `gamerepo-cli serve` and `tauri_shim.js`, in Chromium), `desktop_ui.py` (WebDriver on the built app), `fixtures/` (synthetic files only). |
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
- **Build and test only the core locally:** `cd desktop && cargo test -p gamerepo-core && cargo build --release -p gamerepo-core`. The Tauri app needs WebView libraries the session doesn't have; it builds only in CI.
- **Acceptance test locally:** `python3 tools/build_desktop.py --out build/desktop`, then `python3 tests/desktop_page.py --cli desktop/target/release/gamerepo-cli --ui build/desktop/ui --home <tmp> --out <shots> --chromium /opt/pw-browsers/chromium` (`pip install playwright`; use the pre-installed Chromium, don't download one).
- **GitHub from the session:** GraphQL may be blocked, so open PRs through the REST API (`gh api repos/DrFlGd/Game-Repo/pulls --method POST`). CI reports land on the `desktop-ci-windows` branch (logs, acceptance screenshots in `page/`); fetch it to read results.

### Known limitations now

- Phase 0 is the shell: a library can be opened, renamed and moved, and its index is made, but nothing scans games yet. Audit, Collections, PC games and Metadata are pages saying what they'll do.
- The index has one table (`index_meta`); Phase 1 adds files, DATs and games.
- Two skins (Drafting Sheet, the default, and Classic). Model Library's Parts Bin and Layer Line were 3D-printing themed and weren't brought across.
- `web/styles.css` still holds Model Library's rules for views not built here yet (the browser grid, the workspace, the inspector); they're kept for the phases that bring those views, and pruned once those pages exist.
- Installers aren't code-signed, and there's no auto-update.

### Next (roadmap)

Phase 1 (`docs/PLAN.md`, section 7): scanning library roots and hashing files (inside ZIPs too, headered ROMs), DAT import, and the audit of cartridge systems, verified end to end on a No-Intro set; Home and Audit pages. Write "Phase 1 design" into the plan first, with `docs/dat-formats.md` and `docs/platforms.md`.

---

## Unreleased

The first version of Game Repo: the app itself, with its library and settings. The features come in the next phases; the menu shows what each brings.

- On first start the app asks where your library should go. The library is an ordinary folder: the app adds only a `_library` folder to it, so a folder already holding your games can be used as it is, and nothing in it is changed.
- The app keeps an index of each library on this computer (not in the library), ready for the hundreds of thousands of files a full set has. Settings shows where it is, and Home's Library menu checks it.
- Rename the library (with Undo), open another one, or go back to one opened before from Settings.
- Two looks, Drafting Sheet and Classic, each in light, dark and night themes.
- About shows the version, what's changed, what's coming in each phase, how a library is laid out and the licences.
