# Game Repo — Project Plan

> Draft 2, 2026-10-16. Draft 1 proposed the features and asked ten questions; this draft
> records Jerred's answers (section 10) and the stack they led to (section 5). Open items
> are in section 11. As in Model Library, this file is the plan of record: each phase gets
> a design section before code and a notes section after.

## 1. What we are building

A Windows desktop application for organising a personal video game library:

1. **Audit** large ROM sets against reference `.dat` files (No-Intro, Redump, TOSEC, MAME,
   FBNeo…) and report what is present, missing, misnamed, duplicated or unknown.
2. **Build collections** from those sets using rules (region, language, 1G1R, categories,
   hand-picked lists) and **export** them into the layout ES-DE expects.
3. **Import PC games** (zipped games, GOG/other installers, ISOs, loose folders) into a
   single, predictable file structure with a manifest per game, and hand them to Steam.
4. **Scrape and store metadata and media** (box art, screenshots, descriptions, release
   data…) from configurable online sources such as ScreenScraper (the service behind
   Skraper), LaunchBox GamesDB and IGDB, and write it out where ES-DE and Steam read it.
5. **Attach documents** to a game: instruction books, game guides, posters, maps, box and
   cartridge scans, soundtracks, and anything else the user keeps with it; view them in
   the app and export manuals where ES-DE shows them. (Added by Jerred, 2026-10-16.)
6. **Compress, decompress and convert** game files between the common formats: zip, 7z
   and rar archives; CHD for disc images; RVZ for GameCube/Wii; CSO/ZSO for PSP; with
   every conversion verified against the DAT before an original is touched. (Added by
   Jerred, 2026-10-16.)

Everything is driven from one local index of the user's files by hash, so each feature
builds on the same core: *know every file you have, what it is, and what it should be
called.*

### Non-goals (for now)

- Not an emulator or a game launcher. We organise; ES-DE and Steam launch.
- Never downloads game content. DAT files and metadata only.
- No multi-user/cloud sync. Single library, single owner.
- Linux and server builds: keep the code portable, but nothing is built or tested there
  until Windows is done (decision D3).

---

## 2. Core concepts

| Concept | Meaning |
|---|---|
| **Library root** | A folder the app indexes (e.g. `D:\ROMs`, `\\nas\games`). Many roots allowed. |
| **File** | Any file under a root. Stored with path, size, mtime and hashes (CRC32, MD5, SHA1; SHA256 optional). Archive members (`game.zip/rom.bin`) are files too, with a parent reference. |
| **Platform** | A system such as `snes`, `psx`, `pc`. One canonical id per platform (ES-DE's system names), plus aliases used by DAT groups and scrapers (`Nintendo - Super Nintendo Entertainment System`, ScreenScraper id 4…). |
| **DAT** | A reference list for one platform from one source (e.g. *No-Intro SNES 2026-10-01*). Parsed into **DAT games** and **DAT ROMs** (expected name, size, hashes, parent/clone, region/language flags). |
| **Match** | A File ↔ DAT ROM link established by hash. The basis of audit, rename and export. |
| **Game** | The user-facing entity: a title on a platform, possibly with several ROM/disc files, plus metadata and media. A DAT game becomes a Game when matched; PC imports are Games without a DAT. |
| **Document** | A file the user keeps with a Game that isn't the game itself: a manual, guide, poster, map, scan, soundtrack, readme. Has a kind, a title, and lives in the Game's own folder (3.8). |
| **Storage format** | How a game's files are stored on disk for a platform: raw (`.bin/.cue`, `.iso`), an archive (`.zip`, `.7z`), or a compressed image (`.chd`, `.rvz`, `.cso`). Each platform has a preferred one, and conversion moves between them (3.9). |
| **Collection** | A saved rule set (or manual list) that selects Games; can be exported to a target layout. |
| **Provider** | A metadata/media source with its own matching and rate-limit rules. |
| **Job** | Any long-running operation (scan, audit, export, scrape). Runs in the background, reports progress, can be stopped and resumed. |

---

## 3. Feature modules

### 3.1 Library scanner & hasher

- Walk every library root; detect new/changed/deleted files by `(path, size, mtime)` so
  rescans are incremental. A folder watcher (`notify`, as in Model Library) picks up
  changes made outside the app.
- Hash files in parallel with a bounded worker pool (disk-bound, so pool size ≈ 2–4 per
  physical drive, configurable). **One read computes every hash** (CRC32, MD5, SHA1, and
  SHA256 when asked) so a 2 TB set is read once, not three times.
- **Archives**: look inside `.zip`, `.7z`, `.rar`. Zip central directories already store
  CRC32 + uncompressed size, so a *quick* audit can match zips against DATs without
  decompressing; *deep* mode decompresses to compute MD5/SHA1 and verify. 7z and RAR are
  read through the same crates Model Library uses (solid archives are unpacked once into
  the data folder's cache).
- **Headered ROMs**: No-Intro hashes are usually of the *headerless* data. Detect common
  headers (iNES/NES 16 B, FDS 16 B, Atari 7800 A78 128 B, Lynx LNX 64 B, SNES copier
  512 B…) using No-Intro's header definition XMLs and record both headered and headerless
  hashes.
- **Disc images**: `.cue/.bin`, `.iso`, `.gdi`, `.chd`, `.rvz`, `.cso`. Redump DATs hash
  the raw tracks, so a compressed image is matched by the hash of what it *decompresses
  to*: a CHD by the data SHA1 in its header (confirmed with `chdman verify` on demand),
  an RVZ by `dolphin-tool verify`, a CSO by decompressing it in a stream. The index
  records both the container's own hash and the decompressed hash, so audits treat a
  verified CHD as the Redump game it holds.
- Store results in the index (section 4). The hash cache is keyed on content, so moving or
  renaming a file does not force a rehash.

### 3.2 DAT management

- Import **Logiqx XML** (No-Intro, Redump, TOSEC, FBNeo), **ClrMamePro text** DATs and
  **MAME `-listxml`** output. Keep every imported version; audits reference a specific DAT
  version so results are reproducible.
- Parse parent/clone (`cloneof`), region, language, and the usual name tags (`(USA)`,
  `(Rev 1)`, `(Beta)`, `(Proto)`, `(Demo)`, `[BIOS]`, `(Unl)`…) into structured fields so
  collection rules don't need regexes.
- Map each DAT to a platform via a maintained alias table; prompt the user when a DAT's
  header name is unknown.
- Updates are manual file drops at first (No-Intro requires a login to download; Redump
  and FBNeo publish direct URLs that can be automated later).

### 3.3 Audit

- For each platform: match Files to DAT ROMs by hash (SHA1 → MD5 → CRC32+size, in that
  order of confidence). Produce the classic report:
  **Have / Missing / Unknown / Misnamed / Bad dump (wrong hash) / Duplicate**, plus a
  completeness percentage overall and per parent-only ("1G1R complete").
- **Fixdat export**: a DAT containing only the missing entries, usable with other tools.
- **Rename / rebuild** (decision D5). Three modes, chosen explicitly each time, each
  with a dry-run preview first:
  1. **Leave the originals** (the default): build the sorted set as copies or hardlinks
     in an output folder; nothing under the library roots changes.
  2. **Rename in place**: originals are renamed to the convention where they are.
  3. **Move into the library**: originals are moved into the common library folder,
     nested by platform (`<Library>/<Platform>/<Game Name>.<ext>`, or a folder per game
     for multi-file games), and renamed on the way.
  Modes 2 and 3 are journalled so every move can be undone (Model Library's journal
  pattern), and nothing is removed until the moved or renamed file has been read back
  and hashed.
- **The naming convention is a Settings option**, per platform: DAT names as they are
  (the default), or a template built from the DAT fields (`{name}`, `{region}`,
  `{languages}`, `{revision}`, `{ext}`…) with a choice of flat files or a folder per
  game, so a user who keeps `SNES/Chrono Trigger (USA)/` layouts can say so once.
- Optional canonical zipping of sorted output (TorrentZip-style deterministic archives) so
  files hash identically to other collectors' sets.
- Arcade sets (MAME/FBNeo) need merged/split/non-merged logic, BIOS/device dependencies
  and CHD disks. This is scoped as a later phase (see §7 and D4).

### 3.4 Collections & export

- A collection is a saved query over Games:
  - filters: platform(s), DAT source, region priority list, languages, exclude
    beta/proto/demo/unlicensed, include BIOS, categories (where the DAT provides them),
    name/regex, metadata fields (genre, year, rating, players) once scraped;
  - **1G1R**: pick one ROM per parent/clone group using region, language and revision
    priority;
  - manual additions/removals, and imported lists (a text file of names, another DAT, a
    RetroAchievements-supported list, etc.).
- Collections are live (re-evaluated against the current library) but can be frozen into
  a snapshot for a given export.
- **Export targets** (decision D7: ES-DE and Steam first):
  - **ES-DE**: `ROMs/<system>/<Game Name>.<ext>` with ES-DE's system folder names,
    `ES-DE/gamelists/<system>/gamelist.xml`, and media under
    `ES-DE/downloaded_media/<system>/<type>/` (covers, backcovers, 3dboxes, screenshots,
    titlescreens, marquees, fanart, videos, manuals, miximages…). A game's manual
    document goes to `manuals/`, a poster or map can be sent as `fanart`. Also ES-DE
    custom collections (`.cfg` lists) so a Game Repo collection shows up as one in ES-DE.
    The export can convert on the way out (3.9): a collection exported for an emulator
    that wants CHD gets CHD, whatever the library stores.
  - **Steam**: PC games (and, optionally, emulated games via a configurable launcher
    command, the way Steam ROM Manager does it) added as non-Steam shortcuts in
    `userdata/<id>/config/shortcuts.vdf`, with grid/hero/logo/icon artwork written to
    `userdata/<id>/config/grid/`. Written only while Steam is closed, with a backup of
    the previous `shortcuts.vdf`.
  - a plain folder tree with DAT names, for anything else.
  - Later (M8): RetroArch playlists, Pegasus, Playnite, LaunchBox.
- Export modes: copy, hardlink (same volume, zero extra space), symlink, or zip/unzip on
  the fly. Every export is verified by re-hashing the written files and recorded so it can
  be diffed and re-synced later.

### 3.5 PC game import

- An **inbox** folder (or drag-and-drop in the window). Each item is classified:
  archive (`.zip/.7z/.rar`), installer (`setup.exe`, InnoSetup, GOG `setup_*.exe` +
  `.bin` parts, MSI), disc image (`.iso`), or loose folder.
- Identification, best effort in this order:
  1. GOG installers → `innoextract --gog-game-id` gives the GOG product id → GOG product
     API gives title/year/slug;
  2. Steam-style `steam_appid.txt` / depot manifests if present;
  3. filename parsing (`Title (Year) [Source] v1.2.3`) with a user confirmation step;
  4. manual entry.
- Organised into a common structure (decision D6):

  ```
  PC/
    <Title> (<Year>)/
      install/     installer(s) or the game archive, untouched
      extracted/   optional unpacked copy (off by default)
      patches/     updates, DLC, fixes
      extras/      the user's own documents: manuals, soundtracks, wallpapers
      game.json    manifest: title, year, source (GOG/Steam/retail), version,
                   files with sizes + SHA1, provider ids, installed-at path,
                   import date
  ```

  Scraped media and manuals for a PC game live with every other game's in
  `_library/games/pc/<Title> (<Year>)/media/`, not in the game's folder (Jerred,
  2026-10-16), so the game folder holds only what the user put there.

- A game that has been installed (by the user, or later by the app running the installer)
  records its launch executable in `game.json`, which is what the Steam export needs.
- Duplicate detection by hash across the whole library; version bumps (a newer GOG
  installer for an existing game) go into the existing folder and the manifest records
  both.

### 3.6 Metadata & media scraping

- **Providers** behind one interface (`identify(game) → candidates`, `fetch(candidate) →
  metadata + media URLs`), each with its own credentials, rate limiter and cache. Decision
  D8: every provider is **configurable in Settings** (on/off, credentials, priority order,
  which media types to take from it); none is assumed, and the app works with none.
  - **LaunchBox GamesDB** — offline `Metadata.zip` (XML). No account, no rate limit; a
    good first provider and fallback.
  - **ScreenScraper.fr** — the service Skraper uses. Supports lookup by ROM hash
    (CRC/MD5/SHA1), which makes it the most accurate for ROM sets. Needs a user account
    and per-application developer credentials; thread limits depend on account level.
  - **IGDB** (Twitch developer app), **TheGamesDB** (API key), **MobyGames** (API key),
    **GOG / Steam store** for PC titles (no key needed for basic data).
- Matching strategy: hash lookup when a provider supports it → exact name match within
  the platform → fuzzy name match with a confidence score. Low-confidence matches queue
  for review instead of being written silently.
- Stored locally: structured metadata (title, alt titles, description, developer,
  publisher, release date, genres, players, rating, provider ids) and media files
  (box front/back/3D, screenshot, title screen, marquee, wheel/logo, video, manual) under
  `_library/media/<platform>/<game>/`. Everything is cached so re-exports never
  re-download.
- Written out through the exporters above (`gamelist.xml` + ES-DE media folders, Steam
  grid artwork).
- Providers that offer manuals (ScreenScraper does) save them as documents of kind
  *manual* (3.8), marked as scraped so a user's own copy is never overwritten.

### 3.7 User interface

Pages (Preact + htm islands, the Model Library shell; see §5):

- **Home** — per-platform completeness bars, recent jobs, inbox count.
- **Library** — searchable/filterable grid or list of Games with media thumbnails; the
  Model Library browser views.
- **Audit** — per-platform report with Have/Missing/Unknown tabs, fixdat and rename
  actions.
- **Collections** — rule builder with live counts, export wizard.
- **Import** — PC game identification review (the Model Library sorting workspace, with
  installers and archives instead of STLs).
- **Scraper** — provider setup, queue, review of low-confidence matches.
- **Jobs** in the status bar — progress, log, Stop; the Model Library job pattern.
- **Game page** — a game's details, media, its documents (viewed in the side viewer) and
  its files with their storage format and Convert actions.
- **Settings** — roots, platforms/aliases, preferred storage format per platform,
  external tools (`chdman`, `dolphin-tool`, `maxcso`, `innoextract`, `7z`), providers and
  their credentials, ES-DE and Steam folders.

Everything the UI can do, the CLI can do (`gamerepo-cli scan|dat|audit|export|import|
scrape|convert|docs|serve`), which also keeps the core testable without the window.

### 3.8 Documents attached to a game

A game can carry any number of documents: instruction books, strategy guides, posters,
maps, box and cartridge scans, soundtracks, patch notes, readmes.

- **Where they live** (D13). Every Game that has anything beyond its ROM gets a folder of
  its own, `_library/games/<platform>/<Game Name>/`, holding `game.json`, `media/`
  (scraped) and `docs/` (the user's). A PC game's own documents are its `extras/` folder
  (3.5); its scraped media and manuals are in `_library/games/pc/…` like everyone else's.
  The folder is plain files that make sense without the app; documents keep their names
  and are never renamed silently. The index records each document's kind, title, size,
  hash and source.
- **Kinds**: manual, guide, poster, map, box scan, cartridge/disc scan, soundtrack,
  readme, other. The kind is guessed from the name and format and can be changed.
- **Attaching**: drop files on a game or its page; *Attach documents…* from a game's
  menu; or **bulk attach** a folder of manuals/scans to a platform, matched to games by
  DAT name the way scrapers match (exact, then fuzzy with review), which is how
  collectors' manual packs arrive.
- **Viewing**: PDFs, images, text and markdown in the side viewer (Model Library's
  `docs.rs` and `document-viewer.js` carry over); audio plays in the page; anything else
  opens externally. Documents inside archives are listed and viewed in place, as Model
  Library does for its archives.
- **Export**: manuals go to ES-DE's `manuals/`; posters and maps can be exported as
  ES-DE fanart; the rest stays in the library. A collection export can include a game's
  documents beside its files when the target is a plain folder tree.
- **Scraped manuals** (3.6) are documents of kind manual with `source: screenscraper`;
  a user's own file of the same kind is preferred on export.

### 3.9 Compression, decompression and conversion

The library stores each platform in a **preferred storage format**, and the app converts
to and from it:

| Format | Platforms | Tool | Notes |
|---|---|---|---|
| `.zip` (TorrentZip-style, deterministic) | cartridge systems, arcade | built in (`zip` crate) | the format DAT groups and other managers expect |
| `.7z`, `.rar` | anything, PC game archives | built in (`sevenz-rust2`, `unrar`; 7z is also written) | rar is read only |
| `.chd` | PS1, PS2, Saturn, Dreamcast, Sega CD, PC Engine CD, Neo Geo CD, arcade disks… | `chdman` (external, from MAME) | `createcd` / `createdvd` / `extractcd` / `verify`; CD and DVD modes chosen per platform |
| `.rvz` | GameCube, Wii | `dolphin-tool` (external, ships with Dolphin) | `convert -f rvz` with zstd; `verify` gives the decompressed hashes |
| `.cso` / `.zso` | PSP (and PS2 for some emulators) | `maxcso` (external) | lossless; decompressed by streaming in the core |
| raw (`.bin/.cue`, `.iso`, `.gdi`) | any disc system | built in | what Redump DATs describe |

Rules:

- **Verified before anything is removed.** A conversion is a job: convert to a temporary
  file, hash the result's *decompressed* data (or ask the tool to verify), compare with
  the DAT match of the source, and only then place the result and journal the original
  for Undo (Model Library's checking and journal pattern). A conversion that can't be
  verified against a DAT is still checked by round-tripping (convert back and compare
  hashes) before the original goes.
- **Lossless only by default.** CHD, RVZ, CSO/ZSO, zip and 7z all round-trip exactly.
  Lossy conversions (dropping audio tracks, ISO from a multi-track CD) are refused unless
  explicitly asked for, and are marked in the index.
- **Where it's used**: a *Convert* action on one game or a selection; a *Compress this
  platform* job that brings a whole platform to its preferred format; on import (a PC
  game archive re-packed as 7z if wanted); on export (3.4) when the target wants a
  format the library doesn't store; and *Extract* for anything the user wants raw.
- **Audit understands all of them** (3.1): a CHD, RVZ or CSO is audited as the Redump game
  it holds, so compressing a verified set does not break its audit.
- Tools are found in the app's `tools/` folder (drop the binaries in), on the PATH, or at
  a path set in Settings, with install hints; the conversions a missing tool provides are
  greyed out, nothing else changes.

---

## 4. Architecture

```
web/ (Preact + htm, no build step) --platform.js--> platform-desktop.js --Tauri IPC--> desktop/src-tauri
                                                                                       └─> desktop/core (Rust, no GUI)
                                                                                           api.rs: the command table;
                                                                                           scanner, hashes, archives,
                                                                                           DATs, audit, collections,
                                                                                           exporters, PC import,
                                                                                           providers, jobs, index
```

Principles (Model Library's, applied here):

- **Core is a library.** `desktop/core` has no GUI; `src-tauri` and `gamerepo-cli` are
  thin. `App::call` is one command table, so a server build later costs nothing.
- **Hash-first.** All identity flows through content hashes; names are derived, never
  trusted.
- **The library folder is the source of truth** and must make sense without the app:
  games sit in ordinary folders, PC games carry `game.json`, and the index can be
  rebuilt from disk at any time. Nothing the user needs lives only in the index.
- **Non-destructive by default.** The app writes into its own output folders; renaming
  originals or moving them into the library is an explicit mode with a dry run (3.3),
  and every move is journalled for Undo.
- **Resumable jobs.** Every long operation checkpoints so a crash or a closed laptop lid
  doesn't restart a 2 TB hash run.
- **Providers and exporters are small traits**, so adding TheGamesDB or a new frontend
  layout is one file.

### On-disk layout

The **library folder** is one folder the app owns, portable like Model Library's
(`_library/library.json` carries a format number and the library id). ROMs can be
anywhere (library roots, 2.) or moved into it (3.3, mode 3); PC games imported through
the inbox land in it.

```text
<Library>/
  _library/                          the app's files, kept small and plain
    library.json                     format, library id, name, platform settings
    games/<platform>/<Game Name>/    one folder per game that has more than its ROM
      game.json                      id, platform, DAT match, ROM hashes, metadata
      media/                         scraped: covers, screenshots, videos, manuals…
      docs/                          the user's documents (3.8)
    dats/                            imported DAT files, by platform and version
    journal/                         moves and renames, newest first, for Undo
    fixdats/, exports/               reports the app writes
  Nintendo - SNES/                   platform folders (ES-DE system names), when ROMs
    Chrono Trigger (USA).zip         are moved in; named by the convention in Settings
  Sony - PlayStation/
    Final Fantasy VII (USA)/         a folder per multi-file game
      Final Fantasy VII (USA) (Disc 1).chd
  PC/
    <Title> (<Year>)/                3.5
```

The index (SQLite, below) lives in the app's data folder on this computer, not here,
and is rebuilt from these files and DATs when missing.

### Game identity: hashes, not file names

Scraped metadata, media and documents are tied to a **game id**, never to a file name
(decision of 2026-10-16):

- A **ROM game** is identified by content. A file's hash matches a DAT ROM, the DAT ROM
  belongs to a DAT game, and the Game is that DAT game (its parent/clone group), given a
  stable id (ULID) the first time it is seen. `game.json` in the game's folder records
  the id, the platform, the DAT it was matched against and the hashes of the ROMs that
  make it up. Rename the file, move it, re-zip it or convert it to CHD and it still
  resolves to the same Game, because the match is by hash; a DAT update that renames the
  game only renames its folder (journalled), the id and everything under it stay.
- A ROM with **no DAT match** is a Game keyed by its own content hash until a DAT claims
  it, and keeps its id when that happens.
- A **PC game** has no DAT, so its Game is its id in `game.json`, plus the provider ids
  the scraper found (GOG product id, Steam app id, IGDB id…) and the hashes of its
  installer or archive files for duplicate detection. Replacing the installer with a newer
  version does not change the game.
- Providers are asked by hash where they support it (ScreenScraper) and by name and
  platform otherwise; the answer is stored against the game id either way.

### Index: where this app departs from Model Library

Model Library keeps its index in memory and caches it as JSON per computer (10,000
models, about a second). A ROM library is two orders of magnitude bigger: a full
No-Intro + Redump set is hundreds of thousands of files, MAME alone is ~45,000 machines
with ~300,000 ROM entries, and the core operations are hash lookups and joins (file ↔
DAT ROM ↔ game). So the index here is **SQLite** (`rusqlite`, bundled, WAL mode), kept
in the app's data folder per computer, never inside the library, and rebuildable from
the files and DATs at any time. The "no database in the library" rule holds.

### Repository layout (mirrors Model Library)

```
Game-Repo/
  README.md
  CHANGELOG.md         newest first; "For agents" section with the code map and conventions
  CLAUDE.md            points agents at CHANGELOG.md and docs/PLAN.md
  docs/PLAN.md         this document
  web/                 the front end (no build step): app.js, ui/, lib/, platform*.js
  desktop/
    Cargo.toml         workspace: core, src-tauri
    core/              gamerepo-core (Rust library) + src/bin/gamerepo-cli.rs
    src-tauri/         the Tauri 2 app: window, dialogs, library:// protocol
  tools/               build_desktop.py, set_version.py, vendor_preact.py
  tests/               desktop_page.py (page + real backend), desktop_ui.py, tauri_shim.js,
                       fixtures/ (synthetic ROMs, DATs, archives; no real game data)
  .github/workflows/   desktop.yml (Windows build, tests, release), vendor.yml (crates for offline builds)
```

### Data model (first cut, SQLite)

`library_roots`, `files` (path, size, mtime, crc32, md5, sha1, sha256?, headerless_sha1,
parent_file_id for archive members), `platforms`, `platform_aliases`, `dats`, `dat_games`,
`dat_roms`, `matches`, `games`, `game_files`, `collections`, `collection_rules`,
`collection_items`, `exports`, `export_files`, `metadata` (per game per provider),
`media`, `documents` (game, kind, title, path, hash, source), `conversions` (source file,
result file, format, verified-by, journal entry), `provider_cache`, `jobs`, `job_log`.
`files` carries, for a compressed image, the decompressed data's hashes beside its own.

---

## 5. Tech stack (decision D1, taken 2026-10-16)

**Rust core + Tauri 2 + a no-build Preact/htm front end, seeded from
[Model Library](https://github.com/DrFlGd/Model-Library)**, the same way Model Library
was seeded from Grid Workshop.

Why this and not the TypeScript stack draft 1 proposed:

- **Performance on large sets.** Hashing, archive reading and DAT matching in Rust run at
  disk speed with one pass per file; no GC pauses, no worker-thread marshalling of
  gigabyte buffers. The Rust crates Model Library already uses (`crc32fast`, `sha2`, `zip`,
  `sevenz-rust2`, `unrar`, `notify`) cover most of 3.1 out of the box.
- **It's the owner's established pattern.** SCAD Workshop and Model Library share this
  shell, the one-command-table backend, the Preact interface, the Python build and test
  tools and the CI recipe (Windows app built and released from `main`). A third app in
  the family costs far less to build and maintain than a new stack.
- **Model Library is the same problem with different file types.** Large collections,
  archives read in place, duplicates by fingerprint then hash, a folder watcher, a
  sorting workspace for imports, journalled moves with Undo. Much of it carries over.

| Layer | Choice |
|---|---|
| Core | Rust (stable), `desktop/core` crate `gamerepo-core`; CLI `gamerepo-cli` with `serve` for tests |
| Hashes | `crc32fast`, `md-5`, `sha1`, `sha2`; one buffered read feeding all of them |
| Archives and images | `zip` (entries read in place, deterministic writing), `sevenz-rust2` (read and write), `unrar` (read; unpacked once into the data folder's cache, Model Library's `archive.rs`); external `chdman` (CHD), `dolphin-tool` (RVZ), `maxcso` (CSO/ZSO), driven as jobs with progress |
| DAT parsing | `quick-xml` streaming (MAME listxml is 300 MB+), a small ClrMamePro parser |
| Index | `rusqlite` (bundled SQLite, WAL), migrations in the crate |
| HTTP | `ureq` (as in SCAD Workshop) with per-provider rate limiting |
| Parallelism | `rayon` for hashing/matching, `tokio` for jobs and `serve` (as Model Library) |
| Shell | Tauri 2, `tauri-plugin-dialog`, `tauri-plugin-opener`, `library://` protocol with range requests |
| UI | Preact + htm from `web/lib`, no build step; Model Library's `ui/` islands, store, skins |
| Build & test | `tools/build_desktop.py`, `tools/set_version.py`; `tests/desktop_page.py` against `gamerepo-cli serve` + `tauri_shim.js` in Chromium; `tests/desktop_ui.py` on the built app |
| CI | `desktop.yml`: Windows build, Rust tests, acceptance test, numbered release from `main`; `vendor.yml` for offline crate builds |

### What carries over from Model Library

| Model Library piece | Use here |
|---|---|
| `desktop/src-tauri` (window, dialogs, opener, `library://`) | Keep. |
| `api.rs` command table, jobs with progress and Stop, `config.rs` | Keep the pattern exactly. |
| `library.rs` (portable library folder, `_library/`, format number, safe paths, trash) | Keep; the layout changes (platform folders, `PC/`). |
| `archive.rs` (ZIP in place; 7z/RAR unpacked once, cache limits, passwords, split sets) | Keep whole. Add the zip CRC fast path. |
| `dupes.rs` (fingerprint then SHA-256, set aside with undo) | Reuse for duplicate ROMs and PC installers; hashes move to the SQLite index. |
| `watch.rs`, `checking.rs` (read and hash before a change, with progress) | Keep. |
| `relayout.rs` / journal, `fileops.rs`, `delete.rs` | Reuse for rename/rebuild, export moves, and Undo. |
| `sort.rs` + `web/ui/sort.js` (the Import workspace) | Becomes the PC game inbox and the bulk document attach. |
| `docs.rs`, `document-viewer.js`, the media views in `parts.js`, `archive_ops.rs` (Compress to ZIP, Extract archive, read back and hashed first) | Keep: a game's documents (3.8) and the archive side of conversion (3.9). |
| `import.rs` (destinations from a schema, moving/copying with hash checks) | Reuse for the PC folder convention and export writing. |
| `index.rs` in-memory JSON cache | Replaced by SQLite (above); the search folding and query language stay. |
| `web/ui/` browser, details, inspector, dialogs, settings, skins, status bar | Keep. |
| `mesh.rs`, `thumb.rs`, `extract.rs` 3D parts, `schema.rs` categories | Drop or trim; games don't need them. Previews come from scraped media instead. |
| Tests, tools, CI, CHANGELOG conventions | Keep. |

Start as **a new repository seeded with those pieces** (this one), not a fork: the
products differ and Model Library has its own release phases.

---

## 6. External tools, services and legal notes

- Optional binaries, with install hints in Settings: `chdman` (from MAME, for CHD),
  `dolphin-tool` (from Dolphin, for RVZ), `maxcso` (CSO/ZSO), `innoextract` (GOG/InnoSetup
  installers), `7z` (fallback for odd archives). The app looks in its own `tools/` folder
  (`<data dir>/tools/`, opened from Settings so binaries can be dropped in), then on the
  PATH, then at a path set per tool in Settings (D14). Each is called as a child process
  with its output streamed into the job's progress; none is bundled, for licence and size
  reasons.
- Credentials (ScreenScraper, IGDB, TheGamesDB, MobyGames) are stored on this computer in
  the app's data folder, never in the library and never in the repository.
- The repository will contain **no ROMs, no installers and no copyrighted media**. Test
  fixtures are synthetic (random bytes with known hashes and hand-written DATs).
- DAT files are redistributable under their groups' terms; we ship none by default and
  let the user drop them in.

---

## 7. Phases

Each phase ends with something usable from the CLI, covered by tests, and (from Phase 1)
a Windows build published by CI. Versions follow Model Library's scheme: `0.<phase>.<n>`.

| # | Phase | Deliverable / acceptance |
|---|---|---|
| **0** | Scaffold | Repository seeded from Model Library with the game-library pieces kept and the 3D pieces dropped; `gamerepo-cli --help`; SQLite index with migrations; CI builds the Windows app and runs the (empty) tests; CHANGELOG "For agents" written. |
| **1** | Scan + DAT + Audit (cartridge systems) | `gamerepo-cli scan <root>` hashes files incl. zip members and headered ROMs; `dat import`; `audit <platform>` prints Have/Missing/Unknown/Misnamed and writes a fixdat. Home and Audit pages in the app. Verified on a No-Intro set. |
| **2** | Rename/rebuild + disc systems + conversion | Sorted output by DAT name (copy/hardlink/move with dry run and Undo); Redump cue/bin; 7z/rar reading; CHD, RVZ and CSO audited as the games they hold; Convert and Extract with verification and Undo, deterministic zip and 7z writing, preferred format per platform. |
| **3** | Collections + ES-DE export | Rule-based collections with 1G1R; export to ES-DE (ROMs, gamelist.xml, custom collections) and a plain tree; export verification and re-sync. |
| **4** | PC game import + Steam | Inbox classification, GOG id detection, common structure + `game.json`, duplicate detection; Steam non-Steam shortcuts with artwork. |
| **5** | Metadata, media & documents | Provider settings; LaunchBox offline provider, then ScreenScraper (hash lookup, manuals); media storage; the game folder with `docs/`, attach and bulk attach, the side viewer for PDFs/images/text/audio; ES-DE media and manuals folders and Steam grid output; review queue for weak matches. |
| **6** | Library polish | Library page with media thumbnails, search, filters by scraped metadata; Scraper and Collections pages complete. |
| **7** | Arcade (MAME/FBNeo) | Merged/split/non-merged audits, BIOS/device deps, CHD disks, sample sets. |
| **8** | More targets and providers | RetroArch playlists, Pegasus, Playnite, LaunchBox exports; IGDB, TheGamesDB, MobyGames; automatic DAT updates where allowed; Linux build if wanted. |

Phases 1–3 are the heart of the tool; 4 and 5 are independent of each other and can be
reordered.

---

## 8. Working in a cloud session (from Model Library's notes; to confirm here)

- crates.io has been blocked in most cloud sessions. Model Library's answer: `vendor.yml`
  pushes the core's dependencies to a `crates-vendor` branch whenever `Cargo.toml`
  changes, and the session builds offline from it. Phase 0 sets the same up here.
- The Tauri app itself can't be built in a cloud session; the backend is tested with
  `gamerepo-cli serve` + `tests/tauri_shim.js` in the pre-installed Chromium. The real app
  builds only in CI.
- CI reports and acceptance screenshots land on a `desktop-ci-windows` branch.

---

## 9. Risks

- **Scale**: multi-TB libraries and 100k+ file platforms. Mitigated by one-pass hashing,
  incremental scans, the zip CRC fast path, resumable jobs, SQLite, and virtualised lists
  in the UI.
- **Provider terms and rate limits**: ScreenScraper throttles aggressively; IGDB requires
  a Twitch app. Mitigated by caching every response, the offline LaunchBox DB as fallback,
  and per-provider limiters.
- **Destructive operations**: a bad rename over a curated set is painful. Mitigated by
  non-destructive default, dry runs, and the journal so every move can be undone.
- **Steam's files**: `shortcuts.vdf` is binary and Steam rewrites it. Mitigated by writing
  only while Steam is closed, backing up first, and keeping our shortcuts identifiable.
- **Conversions that lose data**: a CHD made with the wrong mode, an RVZ of a scrubbed
  ISO, a CD ripped to ISO without its audio tracks. Mitigated by verifying every result
  against the DAT or by round trip before an original is journalled, and refusing lossy
  conversions unless asked.
- **External tools** (`chdman`, `dolphin-tool`, `maxcso`) change their command lines
  between versions. Mitigated by probing the version at startup and pinning the known
  syntax per version, with fixtures that exercise each tool in CI.
- **Format edge cases**: headered dumps, multi-track discs, nested archives, MAME merge
  modes. Mitigated by fixtures per edge case and by leaning on how Igir/RomVault handle
  them.
- **PC game identification** is inherently fuzzy. Mitigated by the review step and the
  manifest keeping the original filename.

---

## 10. Decisions taken (Jerred, 2026-10-16)

- **D1 Stack:** deferred to Claude with the requirement that it be performant on large
  data sets, with SCAD Workshop and Model Library as reference. → Rust core + Tauri 2 +
  Preact, seeded from Model Library (section 5).
- **D2 UI form:** native desktop application.
- **D3 Platform:** Windows only for now. Code stays portable; CI builds and tests Windows.
- **D7 Frontends:** ES-DE and Steam are what's used. They are the export targets through
  Phase 5; others come in Phase 8.
- **D8 Metadata providers:** no accounts assumed; providers and credentials are
  configurable in Settings.
- **Requirements added (Jerred, 2026-10-16):** documents attached to a game (manuals,
  guides, posters…), section 3.8; compression, decompression and conversion (zip, CHD,
  RVZ…), section 3.9.
- **D12 Preferred storage formats:** cartridges → zip, CD/DVD systems → CHD,
  GameCube/Wii → RVZ, PSP → CSO, PC → 7z; each changeable per platform in Settings.
- **D13 Where documents live:** with the scraped media in the library's own folder,
  `_library/games/<platform>/<Game Name>/docs/`. Scraped content is tied to the ROM's
  hash, and to the game id for PC games (section 4, "Game identity").
- **D14 External tools:** dropped into the app's `tools/` folder, or pointed at in
  Settings; not downloaded by the app.
- **D4 Arcade scope:** MAME/FBNeo in Phase 7.
- **D5 Touching originals:** allowed, as an explicit choice: rename in place, or move
  into the common library folder nested by platform; otherwise originals are not
  touched. The naming convention is a Settings option (3.3).
- **D6 PC game folder:** `PC/<Title> (<Year>)/install|extracted|patches|extras` with
  `game.json`; scraped media goes in `_library/games/` with everything else's (3.5).
- **D9 License:** MIT.
- **D11 Index:** SQLite (section 4).
- **D10 Name:** **Game Repo**, as the repository is called (Game Library, Game Shelf and
  Game Vault were offered). Crates `gamerepo-core` and `gamerepo-cli`, app id
  `io.github.drflgd.gamerepo`, default library folder `~/Game Repo`.

## 11. Open decisions

None. Every decision in draft 1's list has been taken; new ones get a number here as
they come up.

---

## 12. Next steps

1. Phase 0: seed the repository from Model Library, drop the 3D pieces, rename the crates,
   add the SQLite index and CI, write CHANGELOG's "For agents".
2. Write `docs/dat-formats.md` and `docs/platforms.md` (ES-DE system names ↔ DAT names ↔
   provider ids) while building the DAT importer.
3. Phase 1: scanner, DAT import and audit, verified end to end on one real No-Intro set.
