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
- No multi-user/cloud sync. Single library, single owner. (Serving games to the
  owner's own devices on the home network is in scope: 3.10.)
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
  4. the **Identify** dialog (below), searching online sources;
  5. manual entry.
- **Add a game… and Identify (D24, Jerred 2026-10-17).** One game can be added on its own,
  from the window or `gamerepo-cli add <path>`: a ROM, a disc (any of its files, or its
  CHD/RVZ/CSO), or a PC archive, installer or folder. A ROM or disc is hashed and matched
  against the DATs; a match goes into the library the way Organise places games (copies
  or hardlinks by default, D5). A PC game goes through the import above. A game nothing
  recognises opens **Identify**: a search box filled in from the file name, the system
  (an existing one or a new one, as in the DAT question), and results from the sources
  in their order, each with title, year, system, cover and where it came from. Picking
  one stores the provider's id in `game.json` and pulls its details (and, from Phase 5,
  its media); "None of these" falls back to manual entry. The same dialog is how any
  game, including one already in the library, is re-identified, and in Phase 5 it is the
  review queue for weak matches. Sources come in by how easy they are to integrate:

  | Order | Source | Covers | Needs | Phase |
  |---|---|---|---|---|
  | 1 | Steam store search | PC | nothing | 4 |
  | 2 | GOG catalogue | PC | nothing | 4 |
  | 3 | LaunchBox GamesDB (offline `Metadata.zip`) | every system | nothing; one download, searched locally | 4 |
  | 4 | ScreenScraper (Skraper's source) | every system, by ROM hash first | the user's account; the app's developer id, which ScreenScraper grants on request | 5 |
  | 5 | IGDB | every system | the user's own Twitch developer app (free) | 5 (from 8) |
  | 6 | TheGamesDB | every system | an API key (free, on request) | 5 (from 8) |
  | 7 | MobyGames | every system | an API key; no longer free | dropped (Jerred, 2026-10-18) |

  The order a user sees is a Settings choice (D8); a source without its key is offered
  greyed out with a link to Settings.
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
  - Every provider also answers the **Identify** dialog's searches (3.5, D24), in the
    order of the table there.
- Matching strategy: hash lookup when a provider supports it → exact name match within
  the platform → fuzzy name match with a confidence score. Low-confidence matches queue
  for review instead of being written silently.
- Stored locally: structured metadata (title, alt titles, description, developer,
  publisher, release date, genres, players, rating, provider ids) and media files
  (box front/back/3D, screenshot, title screen, marquee, wheel/logo, video, manual) under
  `_library/games/<platform>/<game>/media/` (D13). Everything is cached so re-exports
  never re-download.
- Written out through the exporters above (`gamelist.xml` + ES-DE media folders, Steam
  grid artwork).
- Providers that offer manuals (ScreenScraper does) save them as documents of kind
  *manual* (3.8), marked as scraped so a user's own copy is never overwritten.
- **The library holds everything; an export takes a subset** (Jerred, 2026-10-17). The
  library is meant to be the complete set of metadata and media, and an export picks
  what it carries:
  - **What is scraped** is chosen per scrape and saved as the default: which fields,
    which media types (and from which provider each, in priority order), which regions
    and languages to prefer, and whether to replace, fill gaps only, or skip games that
    already have something.
  - **What is scraped for**: the games in the library, a platform, a collection, or a
    **whole DAT**, including the games it lists that aren't in the library yet.
    ScreenScraper looks games up by the hashes a DAT records, so a DAT's media can be
    gathered before the ROMs are, and it's there when they arrive.
  - **What an export writes** is chosen in the export: which media types, which fields
    in the gamelist, whether to include manuals and other documents, and image sizes
    (a handheld's SD card wants small images). The choice is remembered per export, and
    device profiles (3.10) start from defaults that suit the device.

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
  menu; or **bulk import** a folder (Jerred, 2026-10-17), which is how collectors'
  packs of manuals, guides and scans arrive:
  1. choose the folder, and the platform if the pack is for one;
  2. a review list shows every file with the game Game Repo thinks it belongs to (by
     DAT name, the way scrapers match: exact, then fuzzy, with how sure it is), and the
     kind it guessed;
  3. change any pairing (search for another game, or leave the file out) or any kind,
     then preview and import. It runs as an Organise change (copy by default, or move),
     journalled with Undo, like the PC games import (3.5).
- **Viewing**: PDFs, images, text and markdown in the side viewer (Model Library's
  `docs.rs` and `document-viewer.js` carry over); audio plays in the page; anything else
  opens externally. Documents inside archives are listed and viewed in place, as Model
  Library does for its archives.
- **Export**: manuals go to ES-DE's `manuals/`; posters and maps can be exported as
  ES-DE fanart; the rest stays in the library. A collection export can include a game's
  documents beside its files when the target is a plain folder tree.
- **Scraped manuals** (3.6) are documents of kind manual with `source: screenscraper`;
  a user's own file of the same kind is preferred on export.
- **Documents on their own** (Jerred, 2026-10-17): guides, FAQs, manuals and the rest are
  searchable apart from games. A **Documents** page lists every document in the library,
  with search across titles, file names and the text inside PDFs, text and markdown
  files, and filters by kind, platform and source. Opening one shows it in the side
  viewer, with a link to its game. Documents stay stored with their game (D13); the page
  is a view over the index, and a document can also belong to no game (a general FAQ or
  a platform's guide), kept in `_library/docs/`.
- **Magazines** (Jerred, 2026-10-17; a later phase, D31): a magazine collection treated
  like games: magazines listed by title and issue, with their own folder and metadata
  (publisher, date, issue number, cover), imported, audited against a list and exported
  the way games are, and searchable with the documents.

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

### 3.10 Serving games to other devices

(Added by Jerred, 2026-10-16.) The app can run as a server on the home network, so an
Android phone or handheld, a Linux PC or a dedicated Linux handheld (Anbernic-style
devices running muOS, Knulli, ROCKNIX, ArkOS or Batocera) can browse the library and
take games from it, without copying an SD card by hand.

- **One server, built on the core.** `App::call` is already one command table (section
  4), so the server is a thin HTTP layer over it, started from the desktop app (a
  *Serve to devices* switch) or headless from `gamerepo-cli serve-devices`. It serves
  read-only by default; nothing a device does changes the library.
- **What a device sees**: the games by platform, or a collection (3.4), with names from
  the DAT, scraped media and manuals (3.6, 3.8). Files go out in the format the device
  wants, converting on the way where needed (3.9), the same as an export.
- **How a device gets them**, in the order they'd be built:
  1. a small web page and JSON API, so any browser on the device can browse and download;
  2. a read-only WebDAV view of the export layout, which Android file managers, Linux
     (`davfs2`, `rclone`) and the handheld firmwares that can mount a network share use
     directly, so the device's own frontend plays from it or copies from it;
  3. **Sync to a device**: a device profile (its firmware's folder names and formats,
     the collection it carries, how much space it has) and a sync job that brings the
     device to match, sending only what changed and verifying what was written. This is
     the export of 3.4 aimed at a device instead of a folder.
- **Device profiles** know each firmware's ROM and media folder names, as the ES-DE
  export knows ES-DE's. The ones Jerred uses come first (D18): **ES-DE on Android or
  Linux** (the Phase 3 ES-DE export's layout, with the device's ROM folder as the
  target), **Knulli** and **ROCKNIX** (a folder per system with its own `gamelist.xml`
  and media beside it), and **muOS** (its ROM folders plus its own catalogue of box art,
  previews and descriptions). ArkOS and Batocera follow if asked for. Each profile's
  folder names and media layout are checked against that firmware's documentation when
  it is built.
- **Layouts the user can change (D36)**: each profile's folders, game lists and media
  names are a JSON layout the user can copy and edit, by hand or in a visual editor that
  shows the export folder by folder ("Phase 5b design"), so a new firmware or frontend,
  or a change to one, needs no new version of the app.
- **SD cards first, then the network.** A device profile is first an export target: a
  collection exported to an SD card in that firmware's layout, within the card's size
  limit (Phase 3's *Size limit*). Serving and syncing over the network reuse the same
  profiles.
- **Safety on the network**: off until switched on, listening on the home network only,
  paired with a code shown in the app, and never exposed to the internet by the app.
- **A device client (D84)** for Windows and Android, then the Steam Deck: a small app
  that browses the library, adds a handful of games where ES-DE finds them (with the
  media the user picks) and removes them to make room. It talks to **Game Repo
  Server** (D85), a small app of its own that Game Repo starts and stops. muOS, Knulli
  and ROCKNIX handhelds come later with a design of their own; until then D18 holds
  for them. Requirements: "Device client requirements" below.

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
| **2** | Rename/rebuild + disc systems + conversion (two releases, D19: 2a sort and rebuild, 2b discs and conversion) | Sorted output by DAT name (copy/hardlink/move with dry run and Undo); Redump cue/bin; 7z/rar reading; CHD, RVZ and CSO audited as the games they hold; Convert and Extract with verification and Undo, deterministic zip and 7z writing, preferred format per platform. |
| **3** | Collections + ES-DE export | Rule-based collections with 1G1R; export to ES-DE (ROMs, gamelist.xml, custom collections) and a plain tree; export verification and re-sync. |
| **4** | PC game import + Steam | Inbox classification, GOG id detection, common structure + `game.json`, duplicate detection; Steam non-Steam shortcuts with artwork; Add a game… for one ROM, disc or PC game, and Identify with Steam, GOG and LaunchBox (D24). |
| **5** | Metadata, media & documents | Provider settings; LaunchBox offline provider, then ScreenScraper (hash lookup, manuals), IGDB and TheGamesDB, all in Identify (D24); media storage; the game folder with `docs/`, attach and bulk attach, the side viewer for PDFs/images/text/audio; ES-DE media and manuals folders and Steam grid output; review queue for weak matches. Choosing what is scraped and for what (a whole DAT included) and what each export writes (3.6); bulk import of documents with a review of the pairings, and the Documents page with search (3.8). |
| **5b** | Device profiles | Export a collection to an SD card in the layout of muOS, Knulli, ROCKNIX, or ES-DE on Android or Linux (3.10, D18, D29): each firmware's ROM folders, gamelists and media, within the card's size limit. Layouts are JSON files the user can copy and change, in a visual editor (D36). Each system's BIOS files as a set of their own, audited and exported with the games (D40). Built on Phase 3's export and Phase 5's media. |
| **6** | Library polish | Library page with media thumbnails, search, filters by scraped metadata; Scraper and Collections pages complete. |
| **7** | Arcade (MAME/FBNeo) | Merged/split/non-merged audits, BIOS/device deps, CHD disks, sample sets. |
| **8** | More targets, add-ons, magazines | RetroArch playlists, Pegasus, Playnite and LaunchBox exports (8a); updates, DLC and patches with their base game (D56) and a collection's history (D52) (8b); magazines (3.8, D31) (8c). MobyGames dropped (Jerred, 2026-10-18). |
| **9** | Device client | 9a Game Repo Server on Windows, started from Game Repo (D85; "Phase 9a design", D89–D95). 9b the client for Windows and Android, ES-DE first (D84, D86–D88; "Phase 9b design", D96–D104). 9c the Steam Deck: the client built for Linux, usable with the Deck's controls in Game Mode. 9d later: the server on a NAS (Unraid, Synology), access from outside the home network (parked), other frontends, the read-only WebDAV view, muOS/Knulli/ROCKNIX handhelds. Needs Phase 5b's layouts. |
| **Later** | Magazine text | OCR of scanned magazines, so their text is searchable, and tagging an issue with the games it covers (Jerred, 2026-10-18). It reads every format Magazines imports: a PDF's pages and a CBZ's or CBR's images alike. |
| **Later** | Videos | Walkthroughs, reviews and retrospectives kept as a section of their own, as magazines are, not as one game's metadata: each video can be linked to the games it covers, and shows on those games' pages (Jerred, 2026-10-20). Not designed yet. |

Phases 1–3 are the heart of the tool; 4 and 5 are independent of each other and can be
reordered.

### Phase 0 notes (built 2026-10-16)

What was built, and where it differs from the table above:

- **Seeded from Model Library** (0.5.20): the Tauri shell (`main.rs`: dialogs, file
  manager, `library://` with ranges, links opening in the browser), the command table and
  jobs (`api.rs`), `config.rs`, the library folder (`library.rs`, trimmed to `_library/`
  with `library.json` and a README), `gamerepo-cli serve` for the tests, the page's
  islands (menu, command bar, status bar, menu and keys, dialogs, messages with Undo,
  About with the changelog), the build tools and the CI workflow. The 3D pieces (viewer,
  meshes, previews, categories, the sorting workspace, archives) were left behind; the
  sorting workspace, journals and archive reading come back in Phases 2 and 4.
- **Opening a folder adds only `_library/`.** Model Library also made `Unsorted/` and
  `schemas/`; here a folder of ROMs is left exactly as it was (D5).
- **Index:** `index.rs`, one SQLite file per library at
  `<data dir>/index/<library id>.sqlite` (WAL, `foreign_keys`, 5 s busy timeout). Its
  schema is `MIGRATIONS`, applied in order and counted in `user_version`; Phase 0 has one
  step (`index_meta`). An index from a newer app or another library is set aside as
  `<id>.old.sqlite` and made again. A moved library keeps its index, found by id.
  `index_info` and `index_check` (a job running `PRAGMA integrity_check`) are the
  commands; `gamerepo-cli index-info` the CLI.
- **Pages:** Home (the index, what's coming), Settings (name with Undo, folder, the index
  and Check, other libraries, skin, theme), About, and one page for each of Audit,
  Collections, PC games and Metadata saying what its phase brings.
- **Skins:** Drafting Sheet (the first default) and Classic. Model Library's Parts Bin and
  Layer Line were themed on 3D printing and weren't brought across. Since 2026-10-20
  (Jerred, PR #46): Classic is the default, Drafting Sheet is dropped, and five themed
  skins join it (Neon City, Wasteland Terminal, Adventurer's Tome, Pixel Arcade, Cosy
  Pastel), with textures on parts of the interface where they don't get in the way.
- **Icons:** a cartridge mark of its own (menu, favicon, app icons).
- **`Cargo.lock` is committed**: crates.io was reachable from the session and the whole
  workspace, Tauri included, resolved.
- **Tests:** 13 Rust tests (library, index, commands) and 15 page checks in
  `tests/desktop_page.py`; `tests/desktop_ui.py` drives the built app in CI.

### Phase 1 design (written 2026-10-16, for review before code)

Phase 1 makes the app useful on cartridge sets: tell it where your ROMs are, give it the
DATs, and it says what you have, what's missing and what's wrong, without changing a
file. Renaming, rebuilding, discs, 7z/RAR and conversion are Phase 2.

**What the user can do at the end of Phase 1**

1. Add the folders their ROMs are in (*Settings › Folders with games*), each optionally
   marked with its platform.
2. Run **Scan**: every file is read once and hashed; ZIPs are looked inside; headered
   ROMs get both hashes. A second scan reads only what changed.
3. **Import DATs** (No-Intro's `.dat`, or the `.zip` it downloads as; any Logiqx XML or
   ClrMamePro DAT): the app works out the platform from the DAT's name and asks when it
   can't.
4. Open **Audit** for a platform: completeness, then lists of *Have*, *Missing*,
   *Misnamed*, *Unknown*, *Bad dump* and *Duplicates*, searchable, with each file's
   path, and *Show in folder*. **Export fixdat** writes the missing games as a DAT.
5. **Home** shows a bar per platform (have / total, and 1G1R complete), the last scan
   and anything that needs attention (a folder that isn't there on this computer, a DAT
   whose platform wasn't recognised).
6. The same from the CLI: `roots add|list|remove`, `scan [--full]`, `dat import|list`,
   `audit <platform> [--list missing|…] [--fixdat <file>] [--json]`.

**Folders with games (library roots)**

- Kept in `library.json` (`roots: [{id, path, platform?}]`), because the index is rebuilt
  from the library and its roots. A root inside the library is stored relative to it; one
  outside, absolute. On another computer a root that isn't there shows as *Not found on
  this computer* with *Point it elsewhere…*, which keeps its id (and so everything matched
  under it). **(D15, taken 2026-10-16.)**
- A root's platform is a hint for files no DAT recognises; matched files take their
  platform from their DAT. Otherwise a folder's name is matched against the platform
  aliases (`SNES`, `Super Nintendo`, `Nintendo - Super Nintendo Entertainment System`…)
  and, last, the extension when it belongs to one platform only (`.sfc`, `.gba`…).
- The library's own `_library/` is never scanned; hidden and system files are skipped;
  symlinks and junctions are not followed (so a loop can't hang a scan).

**Scanning and hashing**

- A scan is a job (progress *n of m files, x GB*, Stop). Stopped or crashed, the next scan
  continues where it was: hashes are committed in batches of a few hundred files.
- Step 1, *walk*: list every file with size and modified time (fast: no file is opened).
  A file whose path, size and modified time are unchanged keeps its hashes. A file that
  vanished and one that appeared with the same name, size and time in the same scan is a
  move: its hashes go with it, no reread. (A rename is read again: by size and time alone
  it can't be told apart from another file safely.)
- Step 2, *hash* what's new or changed, in parallel: a pool of readers (default 2, set in
  Settings: more for SSDs and NAS, 1 for a single slow disk), each file read once in 1 MB
  blocks feeding CRC32, MD5 and SHA1 together. SHA256 is not computed in Phase 1 (no DAT
  needs it to match); it is a later option.
- **ZIPs**: each member is a file of its own (with the ZIP as its parent). The *quick*
  scan (the default, **D16**) trusts the CRC32 and size the ZIP already records, so a ZIP
  is matched without being decompressed; those matches are shown as *matched by CRC*. The
  *full* scan, chosen in Settings (*Scan ZIPs: Trust their checksums / Unpack and hash
  every file*) or with `scan --full`, decompresses each member and hashes it, and turns
  those matches into SHA1 matches. A ZIP whose central directory is damaged is hashed in
  full whatever the setting.
- **Headers**: while hashing, the first bytes are checked against built-in rules (iNES and
  NES 2.0 `NES\x1A` 16 bytes; FDS `FDS\x1A` 16 bytes; Atari 7800 `ATARI7800` at offset 1,
  128 bytes; Lynx `LYNX` 64 bytes; SNES copier headers when the size is 512 more than a
  multiple of 1 KB). A headered file gets a second set of hashes of the data after the
  header, computed in the same read. N64 ROMs in byte-swapped (`.v64`) or little-endian
  (`.n64`) order are hashed as big-endian too, which is what No-Intro lists. Other
  header rules are added when a platform needs them; No-Intro's header XMLs are not
  needed.
- Hashes live in the index only (`blobs`); nothing is written next to the files.

**DATs**

- Import reads the file (or the one DAT inside a `.zip`), stores a copy in
  `_library/dats/<platform>/<DAT name> (<version>).dat` (the library is the source of
  truth: a rebuilt index reads its DATs from here) and loads it into the index.
- Formats: Logiqx XML (No-Intro, Redump, TOSEC, FBNeo; streamed with `quick-xml`) and
  ClrMamePro text. MAME `-listxml` comes with arcade in Phase 7. `docs/dat-formats.md`
  describes what is read from each.
- The platform comes from the DAT's header name, matched against `docs/platforms.md`'s
  table after dropping the group's suffixes (`(Headered)`, `(Parent-Clone)`, a date…).
  Unknown: the import asks which platform it is for and remembers the answer for that
  DAT name (in `library.json`, so the library keeps it).
- Versions: importing a newer version of a DAT keeps the older one; each platform has one
  *current* DAT (the newest by default; *Use this version* picks another). A platform can
  have DATs from several groups (No-Intro and a TOSEC set); the audit shows one at a
  time, chosen at the top of the page.
- DAT entries marked `nodump` aren't counted as missing; `baddump` entries are matched
  but flagged.
- Names are parsed into fields at import (title, regions, languages, revision, version,
  flags such as Beta, Proto, Demo, Unl, BIOS, Pirate), for display now and for
  collection rules in Phase 3.

**Matching and the audit**

- A file matches a DAT ROM by SHA1, else MD5, else CRC32 and size (the last only for
  quick-scanned ZIP members, and marked). Headerless hashes are tried for headered files.
  Matches are worked out after each scan and each DAT import, for what changed only.
- A **DAT game** is *Have* when every ROM it lists (other than nodumps) is matched,
  *Partial* when some are, *Missing* when none are.
- A **file** under a platform is:
  - *Matched* and correctly named: the file name is the DAT ROM name; for a ZIP, the ZIP
    is named for the game and each member for its ROM;
  - *Misnamed*: matched, but named otherwise (the rename that would fix it is shown; doing
    it is Phase 2);
  - *Bad dump*: no hash match, but named as one of the DAT's ROMs (a likely corrupt,
    trimmed or patched copy);
  - *Unknown*: no match at all (a hack, a translation, a file for another platform);
  - *Duplicate*: one of several files matching the same ROM (each is listed with its
    path, to choose which to keep in Phase 2).
- Completeness: *have / total* games, and **1G1R complete**: parent/clone groups with
  at least one member present, out of all groups (so owning *Super Mario World (USA)*
  counts the group whatever the other regions).
- **Fixdat**: the Missing (and Partial) games as a Logiqx DAT named
  `<DAT name> (fixdat <date>)`, saved to `_library/fixdats/` and openable from the page.

**Index tables (Phase 1 migration steps)**

```
roots        id TEXT PK, path TEXT, platform TEXT
files        id INTEGER PK, root_id, parent_id (the ZIP for a member), rel_path, name,
             size, mtime, blob_id, platform, seen (scan number), missing_since
             UNIQUE(root_id, rel_path) · INDEX(blob_id) · INDEX(platform)
blobs        id INTEGER PK, size, crc32, md5, sha1, header, hl_crc32, hl_md5, hl_sha1,
             quick (1 when only CRC32+size are known)
             INDEX(sha1) · INDEX(md5) · INDEX(crc32, size) · INDEX(hl_sha1)
dats         id INTEGER PK, platform, name, version, source_path, imported, current
dat_games    id INTEGER PK, dat_id, name, description, parent_id, title, regions,
             languages, revision, flags
dat_roms     id INTEGER PK, game_id, name, size, crc32, md5, sha1, status
             INDEX(sha1) · INDEX(md5) · INDEX(crc32, size)
matches      file_id, rom_id, method (sha1|md5|crc), PRIMARY KEY(file_id, rom_id)
scans        id, started, finished, files_seen, files_hashed, bytes_hashed, stopped
```

Each table is its own `MIGRATIONS` step, appended after Phase 0's. Hashes are stored as
lowercase hex text (the DATs' own form, and easy to read in a database tool); if the
100k-file check below shows that costs too much, they become blobs in a later step.

**Performance targets (checked by tests on generated data)**

- Rescanning 100,000 unchanged files: under 5 s on CI's runner. Scanning them the first
  time: under 30 s (review M45; it was 188 s on Windows).
- Importing a 50,000-ROM DAT: under 5 s. Auditing it: under 1 s.
- Hashing runs at disk speed: one read per file, whatever the number of hashes.
- Audit lists page through SQL (`LIMIT/OFFSET` with the search in the query), so a list
  of 40,000 missing games scrolls without loading them all.

**Pages**

- *Settings › Folders with games*: add (folder picker), remove (asks; nothing on disk is
  touched), platform hint, *Not found on this computer › Point it elsewhere…*; *Files
  read at once*; *Scan ZIPs: Trust their checksums (default) / Unpack and hash every file*.
- *Settings › DAT files*: Import…, the list by platform with version and date, *Use this
  version*, *Remove*.
- *Home*: Scan (with its job in the status bar), the platform bars, what needs attention.
- *Audit*: platform and DAT at the top, the counts as tabs, the list with search, Show in
  folder, Export fixdat, Copy report.
- Audit, Home and Settings replace their Phase 0 placeholders; Collections, PC games and
  Metadata stay as they are.

**Tests**

- Rust: header detection and headerless hashes, N64 byte orders, ZIP members (full and
  quick), incremental rescans and moved files, both DAT formats, platform detection,
  every audit category, fixdat round-trip (export it, import it, same games), and the
  timing checks on generated data.
- `tests/fixtures/`: a script makes synthetic ROMs (random bytes, some with iNES or
  copier headers, some zipped, some misnamed, some corrupt) and a hand-written DAT
  describing them; no real ROMs or DATs are committed.
- `desktop_page.py`: add a folder, scan, import the DAT, read every Audit tab, export a
  fixdat, rescan after a file is renamed and one deleted.
- Real-set check: Jerred runs the Phase 1 release on a No-Intro set he has and compares
  the counts with the tool he uses now; differences become bugs.

**Not in Phase 1** (later phases): renaming, rebuilding and moving files (2), 7z and RAR
(2), discs and CHD/RVZ/CSO (2), a folder watcher (2; Phase 1 rescans on demand), arcade
(7), automatic DAT downloads (8).

**Decisions taken for Phase 1**

- **D15** Folders with games are kept in the library (`library.json`), re-pointed per
  computer when their path differs. *(Jerred, 2026-10-16: "for now".)*
- **D16** ZIPs are trusted by default: matched by the CRC32 and size they record. A
  setting turns on the full scan (members decompressed and hashed). *(Jerred,
  2026-10-16.)*

**Added to Phase 1 at Jerred's request (2026-10-16)**

- *A DAT per system*: the DAT picked for a platform on the Audit page stays its DAT (for
  Home too), kept in `library.json` as `audit_dats: {platform: DAT name}`. Without a pick,
  the platform's current No-Intro DAT, else its first by name.
- *Audit reports*: besides Copy report and the fixdat, *Save report* writes a text report
  (the counts and what's missing) or a CSV (every list, a row per game or file) to
  `_library/reports/`.
- *A DAT from a set*: a scanned folder (or a folder inside one) written as a Logiqx DAT,
  one game per file or ZIP, to `_library/made-dats/`; ZIP members known only by CRC are
  read so every ROM has its SHA1. Optionally imported for a platform to audit against.
- *A starter set of DATs* (**D17**): *Get starter DATs* downloads the No-Intro cartridge
  DATs from libretro's public copy on GitHub
  (`libretro-database/metadat/no-intro/<DAT name>.dat`, each of a platform's DAT names
  tried in turn) and imports them for the platforms that have no DAT yet; anything newer
  is imported by hand. The address is `dat_source` in config.json on each computer, in
  case libretro moves them. CI downloads them for real (a reported step, not a gate).

### Phase 1 notes (built 2026-10-16)

- **Core** (`desktop/core/src`): `platforms.rs`, `hash.rs`, `roots.rs`, `scan.rs`,
  `dat.rs`, `audit.rs`, `makedat.rs`; the index gained six steps (roots, blobs, files,
  dats with games and ROMs, matches, scans), as designed, plus `files.kind`
  (file/zip/member), `files.hashed` (0, 1 by a ZIP's CRCs, 2 read) and `files.matched`
  (0 until matched), and `blobs.hl_size`. Hashes are hex text; the 100k-file check
  passed without needing blobs.
- **Jobs get their own connection** to the index (WAL), so a scan or import doesn't hold
  up the page's reads; hashes are committed 250 files at a time.
- **Matching** is SQL joined on CRC32 and size, then checked by SHA1 and MD5 where both
  sides have them; method `sha1|md5|crc`, with `-hl` when matched without the header.
  Matches are made for files hashed since (`files.matched = 0`) after a scan, and for a
  new DAT's ROMs after an import. Summaries are cached in `index_meta` and dropped
  whenever matches change.
- **The index follows the library's DATs**: `dat::sync_library` loads DAT files in
  `_library/dats/<platform>/` the index doesn't have and drops rows for ones that are
  gone; the chosen version of each DAT is `dat_current` in library.json.
- **Timings** (Linux, release, this session): rescan of 100,000 unchanged files 0.3 s
  (first scan of them 8 s); import of a 50,000-ROM DAT 0.6 s; its audit and a page of a
  list 0.2 s. CI runs the same checks on Windows (`--ignored timing`), where the rescan
  took 4.7 s and 6.7 s on two runs of the same code: a rescan of unchanged files now
  writes nothing to the index (the per-file `seen` stamp is set only when a file is
  added or moved), and the check takes the better of two rescans.
- **Differences from the design**: renames are read again (see Scanning); a damaged ZIP is
  read from its local headers, and a file named `.zip` that isn't one is hashed as a
  file; files are *unknown* to a DAT when no DAT of that platform or of another platform
  matches them (a file matched only by another DAT of the same platform counts as unknown
  to this one).
- **To check on a real set** (Jerred): the counts against the tool used now, the DAT
  names in `docs/platforms.md` against the current No-Intro release.


### Phase 2 design (written 2026-10-16, for review before code)

Phase 1 says what you have; Phase 2 puts it in order. It does that in two releases, so
the safer half reaches you first:

- **2a, Sort and rebuild (0.2.x)**: rename, rebuild and move cartridge sets by their DAT
  names in the three modes of D5, each with a preview and Undo; 7z and RAR read like
  ZIPs; a folder watcher.
- **2b, Discs and conversion (later 0.2.x)**: disc systems from Redump DATs;
  cue/bin, gdi and iso sets; CHD, RVZ and CSO audited as the games they hold; Convert and
  Extract with verification and Undo; deterministic ZIP and 7z writing; a preferred
  format per platform (D12).

Both are designed here; 2a is built first and released when it passes, then 2b. Each
half is a PR of its own.

**What the user can do at the end of Phase 2**

1. On the Audit page, **Organise…** a platform: choose a mode, see a preview of every
   file that would be copied, linked, renamed, moved or zipped (with the space it needs),
   then run it as a job. The Misnamed tab gets *Rename these* (mode 2, only the misnamed
   files).
2. **Undo** the newest change from the status bar's message or *Settings › History*; a
   change interrupted by a crash or Stop is offered to be finished or put back.
3. Scan 7z and RAR archives (each member matched like a ZIP member), and folders change
   without a rescan by hand (the watcher).
4. Import Redump DATs (or *Get starter DATs*, now with libretro's Redump copies) and audit
   PlayStation, Saturn, Dreamcast, GameCube and the other disc systems from cue/bin,
   gdi, iso, CHD, RVZ or CSO files.
5. **Convert** a game, a selection or a whole platform to its preferred format (CHD for
   CD/DVD systems, RVZ for GameCube/Wii, CSO for PSP, ZIP for cartridges), or to another
   format; **Extract** back to raw files. Every result is checked against the DAT before
   an original is set aside.
6. The same from the CLI: `organise <platform> --mode copy|link|rename|move [--out
   <folder>] [--zip] [--dry-run]`, `undo`, `history`, `convert`, `extract`, `tools`.

#### 2a. Sort and rebuild

**The three modes (D5)**, chosen each time in the *Organise* dialog, the first selected:

1. **Copy to a folder** (the default; originals untouched). Builds the sorted set in an
   output folder chosen per run (remembered per platform). A file whose format doesn't
   change is **hardlinked** when the output is on the same drive (no extra space) and
   copied otherwise; *Always copy* is a checkbox. Nothing under the folders with games
   changes.
2. **Rename where they are**. Each matched file (or ZIP) is renamed to its DAT name in
   its own folder. A ZIP's members are renamed by rewriting the ZIP (below).
3. **Move into the library**. Matched files go to `<Library>/<Platform folder>/` (or the
   output folder set for the platform in Settings), renamed on the way; a file on another
   drive is copied, read back and hashed, then its original is set aside (below).

**What is organised**: the games the DAT says you have (and, with *Include partly-there
games*, those you partly have) for the platform's DAT. Not organised, and reported in
the preview: unknown files and bad dumps (left where they are), and duplicates beyond
the one kept (below). Choosing a subset (1G1R, regions) is Phase 3's collections; Phase
2 organises the whole platform or the Misnamed list.

**Which copy is kept** when several files match the same ROM: one that is already
correctly named, else one already in the target format (a ZIP when zipping), else one on
the output drive, else the first by folder and path. The others are listed in the
preview and in the change's report; in modes 2 and 3 they stay where they are, and
*Set aside duplicates* (a separate action on the Duplicates tab) moves them out, with
Undo. (**D20** proposes this.)

**Naming** follows the platform's convention in *Settings › Naming*: DAT names as they
are (the default), or a template of DAT fields (`{name}`, `{title}`, `{region}`,
`{languages}`, `{revision}`, `{ext}`), with *A folder per game* on or off. Multi-file
games always get a folder. Names are made safe for Windows (`<>:"/\|?*` and trailing
dots and spaces replaced, reserved names like `CON` prefixed) and kept under the path
length limit by shortening the folder template first; a clash between two games adds
` (2)`, and the preview shows it.

**Formats on the way out**: *As they are* (the default: loose stays loose, a ZIP stays a
ZIP, renamed inside), or **one ZIP per game** (TorrentZip-style, below), or loose files
(unzipped). Reading a 7z or RAR member for output unpacks it once into the cache.

**Preview and running**

- *Plan* reads only the index and builds every operation (`link`, `copy`, `rename`,
  `move`, `zip`, `rezip`) with its source, target, size and expected SHA1, and checks:
  free space on each target drive, targets that already exist (identical: skipped;
  different: listed as a conflict and not overwritten), names too long, read-only
  folders. The preview is a paged list with the totals; nothing is written.
- *Run* writes the plan to a journal, then performs it as a job (progress, Stop). Each
  written file is read back and its hash compared with the expected one **before** its
  source is renamed or set aside; a mismatch stops that game, keeps its original and is
  reported. A rename on the same drive is one filesystem rename (no reread).
- The index is updated as each operation completes (the files keep their hashes: a rename
  or move is a path change), so no rescan is needed after.

**Journal and Undo** (Model Library's `relayout.rs` pattern)

- Every change in modes 2 and 3, *Set aside duplicates*, and every conversion is written
  to `_library/journal/<id>.json` before anything moves: the operations, their state
  (planned, done), what each set aside, and the change's state (running, done, undone).
- **Undo** reverses the newest change still undoable, newest operation first: renames are
  renamed back, moved files moved back (read back and hashed again when crossing drives),
  set-aside originals restored, files the change created removed. Older changes are
  undone in order. A change whose files have since been changed by hand undoes what it
  safely can and lists the rest.
- On start-up a journal left *running* is offered: *Finish it* or *Put it back*.
- Mode 1 changes are journalled too (so *Undo* removes the output it wrote), but they
  never touch an original.
- *Settings › History* lists changes with their reports; the newest twenty finished ones
  are kept (journals holding set-aside files are kept until those are emptied).

**Setting aside**: an original that a change replaces (a move across drives, a
conversion in place, a duplicate set aside) is not deleted. It is renamed into
`.gamerepo-set-aside/<journal id>/` at the top of its folder with games (same drive, so
instant; hidden, so scans skip it) or `_library/set-aside/` for files in the library.
*Settings › History* shows the space held and **Empty** per change or for all; emptying
is the only step that deletes, asks first, and ends that change's Undo. (**D20**.)

**Rebuilding ZIPs**

- A ZIP to be renamed inside, or a game zipped on the way out, is written to a temporary
  file next to its target, read back (every member's CRC32 and SHA1 checked), then
  renamed into place.
- Written **TorrentZip-style** so the same game always gives the same bytes: members
  sorted by lower-case name, stored with the fixed date TorrentZip uses
  (1996-12-24 23:32), no extra fields, deflate at level 9, and the
  `TORRENTZIPPED-<CRC32 of the central directory>` comment. Whether the bytes equal
  TorrentZip's exactly depends on the deflate implementation matching zlib's; a test
  compares with a ZIP written by Python's zlib in the fixtures, and the zlib-compatible
  backend is used if `miniz` differs.

**7z and RAR** (from Model Library's `archive.rs`)

- Scanned like ZIPs: each member is a file with the archive as its parent. 7z records
  each member's CRC32, so the quick scan trusts it like a ZIP's (D16); RAR records CRC32
  too. *Full* unpacks into the data folder's `archive-cache` (size-limited, emptied
  after the scan) and hashes the members.
- Read with `sevenz-rust2` and `unrar` (as Model Library does). Password-protected
  archives are listed as *locked* and skipped. RAR is read only.

**Folder watcher** (`watch.rs`, `notify`)

- Watches the folders with games on local drives while the app is open; files that
  change are rescanned a few seconds after the changes stop (only those paths). Network
  drives don't report changes reliably, so they rescan on demand as now, which the
  folder's row says. *Settings › Folders with games › Watch for changes* (on by default).

#### Phase 2a notes (built 2026-10-16)

- **Core**: `archive.rs` (ZIP, 7z, RAR; `files.kind` 'zip', '7z', 'rar' and the new
  `files.locked`), `torrentzip.rs`, `organise.rs` (plan, preview, run, journal, Undo,
  finish, set-aside, naming), `watch.rs`; `api.rs` gained `organise_plan`,
  `organise_duplicates`, `organise_preview`, `organise_run`, `change_history`,
  `change_undo`, `change_finish`, `change_empty`, `naming`, `naming_example`,
  `naming_set`, and `watch` in the scan settings. The command line has `organise`,
  `duplicates`, `history`, `undo`, `finish` and `set-aside empty`.
- **A plan** is built from the index alone and kept in the data folder
  (`<data>/plans/<id>.json`, a day) until it's run, so the preview pages through it
  without planning again. Running writes it to `_library/journal/<id>.json`, and each
  step done is a line in `<id>.log` (made durable every 64 steps and whenever a step set
  something aside); a step already done is recognised when a change is finished.
- **One change at a time**, never during a scan (the API refuses; the page says so).
  The index is updated step by step (paths, copies, new ZIPs with their members), so no
  rescan is needed; the watcher's scan after a change finds nothing new.
- **The folder watcher** watches the folders with games that are found and not on a
  network drive (`GetDriveTypeW` on Windows, `/proc/mounts` elsewhere), skips hidden
  folders and `_library/`, and after 3 s of quiet scans only the places that changed
  (`scan::Options.only`; over 2,000 places, everything). Its scans are jobs marked
  `auto`, which the page follows by polling `jobs` every 2 s.
- **Differences from the design**: hardlinks fall back to a copy when the drive refuses
  them; a 7z or RAR whose members need renaming is rebuilt as a ZIP (the app doesn't
  write them), which the preview notes; *Rename these* leaves a misnamed copy of a file
  that's named right elsewhere (it's a duplicate) and the preview says why; set-aside
  files in the library go to `.gamerepo-set-aside/` at the top of its folder like any
  other folder with games, not `_library/set-aside/`; the output folder for copies is
  remembered on this computer for all platforms, not per platform; a `{ext}` field
  isn't offered (the extension always follows the ROM or the format). An interrupted
  change is offered at start-up with *Finish it* and *Put it back*, and in History.
- **Timings** (Linux, release, this session): planning 50,000 matched files 0.5 s;
  renaming 10,000 files in place with the journal 1.0 s. CI runs both on Windows.
- **To check on a real set** (Jerred): organise a copy of a real set in mode 1 into
  ZIPs and compare with a TorrentZip tool's output; mode 2 on a copy, then Undo.

#### 2b. Discs and conversion

**Disc platforms** join the table (`platforms.rs` and `docs/platforms.md`) with their
Redump DAT names, ES-DE ids and preferred formats: PlayStation, PlayStation 2, PSP,
Saturn, Sega CD / Mega-CD, Dreamcast, PC Engine CD / TurboGrafx-CD, Neo Geo CD, 3DO,
PC-FX, Jaguar CD, CD-i, GameCube, Wii, Xbox (audit only). *Get starter DATs* also tries
libretro's Redump copies (`metadat/redump/`), for disc platforms without a DAT (D17).

**Disc sets and the DAT**

- A Redump game lists its files: a `.cue` and one `.bin` per track, a `.gdi` and its
  tracks, or one `.iso`. Each file is matched by its hashes as in Phase 1, and a game is
  *Have* when every track is.
- The **cue sheet**: Redump hashes its own cue, which a renamed set no longer matches. A
  cue that doesn't match but whose `FILE` lines name the game's matched tracks counts as
  *cue differs*, not a bad dump, and the game is still *Have*. Renaming rewrites only the
  `FILE` lines, which turns Redump's cue (renamed) back into an exact match.
- A disc game always gets a folder of its own when organised.

**Compressed images, audited as the game they hold**

- **CHD** (CD and DVD): read in the core with the `chd` crate (pure Rust, no tool
  needed). For a CD, each track's sectors (2,352 bytes, without CHD's subcode and
  padding) are hashed as the `.bin` Redump lists; for a DVD, the whole image as the
  `.iso`. The tracks are stored as members of the CHD (`kind = 'track'`), so matching is
  Phase 1's. A CHD has no cue, so a CHD game is *Have* when every track matches (the cue
  isn't needed). The CHD's own header SHA1 is kept too, so a rescan of an unchanged CHD
  is free.
- **RVZ** (GameCube, Wii): `dolphin-tool verify` reports the CRC32, MD5 and SHA1 of the
  disc the RVZ holds, which is what Redump lists. It reads the whole file, so it runs in
  the hash step like any other file, once. Without `dolphin-tool`, RVZ files are listed
  as *Needs dolphin-tool to check* (not unknown).
- **CSO / ZSO** (PSP): decompressed in a stream in the core (zlib for CSO, LZ4 for ZSO)
  and hashed as the `.iso`.
- **Plain ISO / bin**: hashed as files, as now.

**Tools** (D14): `chdman`, `dolphin-tool`, `maxcso`. Looked for in the app's `tools/`
folder, then on the PATH, then at a path set in *Settings › Tools*, which shows each
one's version and what it enables; a missing tool greys out the conversions it does and
nothing else. (During the build, `libchdman-rs`, MAME's own CHD code as a Rust crate, is
tried as a built-in CHD writer; if it builds on Windows CI and its output verifies,
`chdman` becomes optional.)

**Convert**

| From | To | How |
|---|---|---|
| cue/bin, gdi, iso (CD systems) | CHD | `chdman createcd` |
| iso (PS2, other DVD systems) | CHD | `chdman createdvd` |
| iso, gcm, wbfs | RVZ | `dolphin-tool convert -f rvz -c zstd -l 5 -b 131072` |
| iso (PSP) | CSO | `maxcso` |
| loose cartridge ROMs, 7z, RAR | ZIP | built in (TorrentZip-style) |
| anything (PC games, Phase 4) | 7z | built in (`sevenz-rust2`, deterministic settings) |

- A conversion is a job per game: convert into a temporary file beside the target, then
  **verify** by reading the result as the audit would (CHD tracks, RVZ through
  `dolphin-tool verify`, CSO in a stream, ZIP/7z members) and comparing with the DAT
  match of the source. Only a result whose every track or member matches is placed; the
  original is then set aside (journalled, Undo). A source with no DAT match is verified
  by converting the result back and comparing hashes with the original.
- **Where results go**: in mode 1 (the default) into an output folder; *Convert in place*
  replaces the originals, set aside as above. Lossless only: anything that would drop
  data (an ISO from a multi-track CD) is refused.
- *Compress this platform*: every game not in its preferred format, as one job, Stop and
  continue later. Space needed is shown first (the set-aside originals still use theirs
  until emptied).

**Extract**: CHD → cue/bin or gdi (`chdman extractcd`), CHD → iso (`extractdvd`), RVZ →
iso (`dolphin-tool convert -f iso`), CSO → iso (built in), ZIP/7z/RAR → files. Verified
the same way, into an output folder.

**Index (Phase 2 migration steps)**

```
files.kind     gains '7z', 'rar' (archives), 'chd', 'rvz', 'cso' (images) and
               'track' (a CHD track, member of its CHD, like a ZIP member)
files.locked   1 for a password-protected archive
blobs.inner    for an image: the hashes of what it holds when that's a single file
               (RVZ, CSO, DVD CHD), so a rescan of an unchanged image is free
dat_roms       unchanged; cue sheets are recognised by extension
platforms      disc rows in platforms.rs (not the index)
```

The journal lives in the library, not the index (the library is the source of truth); the
index is updated as operations complete and can be rebuilt by a scan.

**Performance targets (checked by tests on generated data)**

- Planning the organisation of 50,000 matched files: under 2 s; the preview pages
  through it.
- Renaming 10,000 files in place: under 30 s on CI's runner, including the journal.
- Hashing a CHD runs at disk speed or CPU speed, whichever is slower, one read per image.
- An unchanged CHD, RVZ or CSO costs nothing on a rescan.

**Pages**

- *Audit*: **Organise…** (the dialog: mode, output folder, format, *Include partly-there
  games*, *Always copy*; then the preview with totals, conflicts and space; Run).
  *Rename these* on the Misnamed tab; *Set aside duplicates* on the Duplicates tab;
  *Convert…* on a game's row and for the platform.
- *Settings › Naming* (per platform template and folder per game), *Settings › Tools*,
  *Settings › History* (changes, Undo, set-aside space, Empty), *Preferred formats* per
  platform (D12), *Watch for changes*.
- The status bar's message after a change has **Undo**.

**Tests**

- Rust: each mode on the fixture set, with Undo of each returning the folders to the
  exact files and hashes they had; a change stopped half-way, finished and put back;
  conflicts, clashes and Windows-unsafe names; duplicates choice; TorrentZip bytes equal
  to the Python-written reference; 7z quick and full scans (7z written by the test with
  `sevenz-rust2`) and RAR (the unrar crate's MIT-licensed test files, as Model Library
  does); CHD track hashing and CSO/ZSO streaming on small synthetic images committed to
  `tests/fixtures/` (random bytes made into a CHD by `chdman`, generated by the fixture
  script where `chdman` is present and committed, so tests don't need it); verification
  refusing a corrupted conversion; the timing checks.
- CI (Windows): converts the synthetic disc set with `chdman` and `maxcso` taken from
  their official release downloads in the workflow (CI only; the app never downloads
  tools, D14), and verifies the results; `dolphin-tool` the same if its download is
  practical, else RVZ is tested with a recorded `verify` output.
- `desktop_page.py`: Organise in each mode with the preview, Undo, Rename these, the
  History page; Convert with a fake tool script standing in for `chdman` (its output
  checked like the real one).
- Real-set check (Jerred): organise a copy of a real set in mode 1, then mode 2 on a
  copy, and Undo; convert a few real discs to CHD and compare with `chdman verify`.

#### Phase 2b notes (built 2026-10-17)

- **Core**: `cue.rs` (cue sheets and GDIs: tracks, file names, rewriting the names),
  `image.rs` (reading CHD, CSO, ZSO and RVZ as the files they hold; writing CHD and
  CSO/ZSO), `convert.rs` (Convert and Extract plans, run by `organise::run`, so the
  journal, Undo and *Finish* are Phase 2a's), `tools.rs` (D14: `tools/` beside the app,
  then PATH, a path set in Settings first). `api.rs` gained `convert_plan`, `formats`,
  `format_set`, `tools` and `tools_set`; the command line has `convert`, `extract`,
  `formats` and `tools`. `platforms.rs` has the 15 disc rows and each platform's media
  and formats.
- **No chdman or maxcso.** The `chd` crate (chd-rs, BSD-3-Clause) writes CHDs; with
  `chdman createcd`'s layout (2,448-byte frames, tracks padded to 4 frames, audio
  big-endian, CHT2 metadata; for a GDI, CHGD metadata with each track's gap to the next
  LBA as empty frames) and `createdvd`'s, its CHDs have the same data SHA1 and the same
  SHA1 as chdman 0.264's, checked by the tests against CHDs chdman made
  (`tests/fixtures/discs/`, made by `make_discs.py --chd`). CSO (v1, 2,048-byte blocks,
  deflate) and ZSO (LZ4, `lz4_flex`) are written in the core too, so maxcso was dropped.
  `dolphin-tool` (or `DolphinTool`) is the one tool, for RVZ; without it RVZ files are
  scanned with `locked = 2`, counted as *waiting for dolphin-tool* on Home and the
  Audit, and read on the next scan once it's found.
- **Images are containers.** `files.kind` gains 'chd', 'cso', 'zso', 'rvz' (no migration
  step was needed: the column is text). Their members are ordinary members
  (`kind = 'member'`, not 'track' as designed): a CD CHD's are `Track NN.bin`, a DVD
  CHD's, CSO's, ZSO's or RVZ's is `<stem>.iso`. `blobs.inner` wasn't needed: the member
  rows already make an unchanged image free on a rescan.
- **Cue sheets**: a game in a CHD is *Have* without its cue (or GDI), and a member is
  *named right* when the image is named for the game. A sheet that isn't Redump's but
  names the game's tracks is matched with method `cue`. Organise rewrites a cue or GDI
  to the new track names, and a disc game's images are always kept whole (Organise's
  format is *keep* for disc platforms; Convert changes it). A single-file image (a CHD,
  an ISO) isn't put in a folder of its own unless Naming says so; a multi-file set is.
- **Convert and Extract**: each result is written beside its target as
  `<name>.gamerepo-tmp` and checked before it takes its name: a CD CHD's tracks hashed
  per source file (`cd_file_hashes`), anything else read back whole and compared with
  the source's hashes. In place, the originals are set aside (D20). Extract writes
  Redump's files again: tracks named as the DAT names them and the sheet rewritten for
  them (so it's an exact match), a GD-ROM as a GDI with `.raw` audio. Cartridges'
  Convert (ZIP) and Extract (loose files) are Organise's copy or rename plans.
- **Preferred formats** (D12) are in library.json `formats`; CD and DVD systems have
  CHD only, GameCube and Wii RVZ, PSP CSO, ZSO or CHD; Xbox is audited only.
- **Starter DATs** for the disc systems come from libretro's `metadat/redump/` copies,
  which list one file of each disc (its first track or ISO), so a disc's other tracks
  and its sheet are unknown files against them; the app says so after importing them.
  Redump's own DATs (redump.org) give the full check.
- **CI**: the tests don't need chdman (the fixtures hold chdman's CHDs); the
  `chdman verify` test runs where chdman is installed (this session's Linux), not on
  Windows CI, where only MAME's full release has it. The RVZ round trip uses
  `tests/fixtures/fake_dolphin_tool.py`, on Linux only.
- **To check on a real set** (Jerred): convert a few real CD, GD-ROM and DVD games to
  CHD and compare with `chdman verify` and chdman's own CHD of the same disc (the
  `SHA1` line of `chdman info`); a PSP game to CSO in PPSSPP; with dolphin-tool, a
  GameCube game to RVZ and back.

**Not in Phase 2**: 1G1R and region choices (Phase 3 collections), export to ES-DE (3),
arcade merged/split sets (7), automatic DAT updates (8), lossy conversions, writing RAR.

**Decisions proposed for Phase 2** (defaults the build follows unless changed)

- **D19** Phase 2 ships in two halves: 2a *Sort and rebuild* (cartridges, 7z/RAR,
  watcher, Undo), then 2b *Discs and conversion*, each its own PR and release.
- **D20** Originals a change replaces, and duplicates set aside, are moved to a hidden
  `.gamerepo-set-aside/` folder on the same drive (instant, undoable) and deleted only
  when you empty it; not sent to the Recycle Bin (which may delete large files at once)
  and not deleted.

### Phase 3 design (written 2026-10-17; Jerred asked for it to be built straight away)

Phase 3 makes the library's games into **collections** and **exports** them. Jerred
added three things on 2026-10-17: a collection can be **saved as DATs**, a collection can
span **several systems**, and **systems can be added** in Settings. The "which system is
this DAT for?" question offers the known systems (built-in and added ones) or a new one.

**Systems (D22)**

- Systems added by hand are kept in library.json `systems`: `{id, name, dat_names,
  folders, exts, media}`. The id is the folder name exports use (ES-DE's system name for
  ES-DE), lower case; `media` is cartridge, cd, nintendo, umd or audit (which formats it
  converts to, as for the built-in systems). They join the built-in table when the
  library opens (`platforms::configure`), so DATs, folders and extensions find them like
  any other, and every page's system list shows them.
- An id a DAT was given before (Phase 1's "another platform…") is shown as a system
  named by its id until it's described.
- *Settings › Systems* lists the built-in systems (read-only) and the added ones: add,
  edit, remove (a system with DATs can't be removed until they are).
- *Which system is this DAT for?* lists every system, plus *A new system…*, which asks
  for its name (its id follows from the name and can be changed) and adds it with the
  DAT's name as its DAT name, so the next version is found on its own.

**Collections (D21)**

- A collection is a file in the library, `_library/collections/<id>.json`: its name, its
  rules and the games added or left out by hand. Nothing about it is in the index: it is
  worked out from the index when it's shown or used, so it follows the library.
- **Rules**: the systems it covers (none ticked: every system with a DAT, D23 for
  several); which games (*games you have*, the default, or *every game in the DAT*, for a
  list of what to look for); *1G1R* with a region order (USA, World, Europe, Japan by
  default) and *only those regions*; languages; leave out betas, prototypes, demos,
  samples, unlicensed, pirate, BIOS, programs, kiosk and debug builds (each a tick);
  a name search; and **from DATs**: only games in one or more other DATs (by name or
  by any ROM's hashes), so a DAT of your own (a "best of" list, a RetroAchievements
  list) picks the games.
- **1G1R**: in each parent/clone group, among the games that pass the rules (and, with
  *games you have*, that you have), the game whose first region comes earliest in the
  order, then the one with no flags, then the highest revision and version, then the
  parent. With *only those regions* a group with no game in them gives none.
- **By hand**: *Leave out* on any game in the result, *Add games…* from a system's DAT;
  both kept by `system/game name` and shown, with *Put back*.
- The page: the list of collections, and for the one chosen its rules, then the result
  (a count per system, the games a page at a time with a search). New, Rename,
  Duplicate, Delete. Every change is saved at once.

**Save as DATs (D23)**

- One Logiqx DAT per system, `<Collection> - <System>.dat`, with each game and its ROMs
  as its own DAT lists them (clone links kept when both are in), header author Game
  Repo. They go to a folder you choose (by default `_library/collection-dats/<name>/`).
  One DAT per system because RomVault, clrmamepro and igir expect a DAT to be one system.

**Export**

- Two targets: **ES-DE** (`<ROMs folder>/<system id>/`) and **a plain folder**
  (`<folder>/<DAT group name>/`, as Organise's *Move* names it, with Settings › Naming's
  folder per game). Files are hardlinked when the export is on the same drive (no space
  used), else copied, or always copied if you choose; every file written is checked
  (Phase 2a's copy, read back). It's a change like Organise's, with the preview, the
  journal and Undo.
- For ES-DE, a game of several files (a cue/bin, a GDI) goes into a folder named as its
  sheet, `Game (USA).cue/`, which ES-DE shows as one game and launches the file of the
  same name inside (its "directories interpreted as files"); a game of one file is the
  file. Names follow Settings › Naming; the files inside a game's folder keep the DAT's
  names, as the sheet names them.
- With the ES-DE folder given (`%USERPROFILE%/ES-DE` is offered when it exists), the
  export also writes `gamelists/<system>/gamelist.xml` (an entry with the game's short
  title or its full name for each exported game that has none; entries already there are
  kept as they are) and the collection as an ES-DE custom collection,
  `collections/custom-<Collection>.cfg`. A file it replaces is set aside, so Undo puts
  it back.
- **Re-sync**: each export is remembered in `_library/exports/<id>.json` (collection,
  target, folders, files it placed). *Update* plans it again: games already there with the
  same contents are left, new ones are added, and files the export placed that the
  collection no longer has are set aside in `.gamerepo-set-aside/` at the top of the
  export folder (D20). Files it didn't place are never touched.
- **Check**: reads every file of the export back and reports the ones missing, changed
  (their contents no longer match a DAT) or as they should be. A job, with Stop.
- An export folder inside a folder with games is refused: its copies would be counted
  as duplicates.
- **Properties** (Jerred, 2026-10-17): a collection shows its games, systems and size on
  disk, by system, and the games it lacks with their size as the DATs list them. It can
  have a **size limit** (an SD card's: a "64 GB card" is 64,000,000,000 bytes, which
  Windows shows as 59.6 GB), and says whether it fits or by how much it's over; the games
  can be listed biggest first to choose what to leave out, and Export… warns when it's
  over. On disk, a game counts the archive or image each ROM is in, else the file, once.

**Not in Phase 3**: converting on export (the library's format is exported; Convert
changes it first), `.m3u` files for multi-disc games, scraped details and media in the
gamelist (Phase 5), RetroArch, Pegasus and the rest (Phase 8).

**Tests**: Rust: systems added and found by DAT name, folder and extension; each rule and
1G1R on the fixture set; a collection's size on disk and what it lacks, and its size
limit; DATs as sources; collections saved as DATs and imported again
(the same games); ES-DE and plain exports, with a cue/bin game in its folder, the gamelist
merged, the custom collection, re-sync after a rule change and Undo; Check finding a
changed file. Page: Settings › Systems; the DAT question with an existing and a new
system; a collection made, changed by hand, sized against a limit and listed biggest
first, saved as DATs, exported to ES-DE (warned of the limit), updated and checked.

**Decisions taken for Phase 3** (Jerred, 2026-10-17, from Claude's proposal)

- **D21** Collections are saved queries in the library, worked out from the index when
  used; games added or left out by hand are kept by system and name.
- **D22** Systems can be added in Settings and from the DAT question; they're kept in
  library.json beside the built-in table.
- **D23** A collection can span systems; saved as DATs, it's one DAT per system.

### Phase 3 notes (built 2026-10-17)

- **Core**: `systems.rs` (D22: library.json `systems`, checked against the built-in ids
  and DAT names; an id with DATs can't change), `platforms.rs` (`configure` adds the
  library's systems to the table behind a lock, `all()` is the built-in and added rows;
  ids a DAT was given before become systems named by their id; `App::library` reads
  library.json's systems again when it changes), `collection.rs` (D21: rules, 1G1R,
  DATs as sources, by hand; the last result cached by the index's `generation` counter,
  which every index change bumps; Save as DATs), `export.rs` (the export plan, the record
  in `_library/exports/<id>.json`, the gamelist and custom collection, Check). `api.rs`
  gained `systems_set`, `systems_remove`, `collections`, `collection_get`,
  `collection_games`, `collection_save`, `collection_delete`, `collection_candidates`,
  `collection_dats`, `export_defaults`, `export_plan`, `exports`, `export_check` and
  `export_forget`; the command line has `systems`, `collections`, `collection`,
  `export` and `exports`.
- **An export is an Organise change.** `export::plan` makes one `organise::plan` (copy,
  format *keep*) per system into `<out>/<system id>/` (ES-DE) or `<out>/<DAT group
  name>/`, merges them, and adds `aside` steps for files the export placed before that
  the collection no longer has (set aside at `<out>/.gamerepo-set-aside/<change>/`, a
  `dir:` source root, since the export folder isn't a folder with games) and `text`
  steps for ES-DE's `gamelist.xml` and `collections/custom-<name>.cfg` (the file they
  replace is set aside first, so Undo puts it back). Running, the journal, Undo and
  *Finish* are Phase 2a's; `organise::run` and `undo` call `export::record`.
- **Organise's options** gained `esde` (a game of several files goes in a folder named
  as its sheet, ES-DE's "directories interpreted as files"; a game of one file is the
  file, whatever Naming's folder option says) and `known` (files the export placed,
  with their size and modified time: one still the same is *there already* without
  reading it). A plan's `present` lists targets there already with the right contents.
- **Re-sync**: the record keeps each file placed with its size, modified time, game,
  system and DAT. Files there already that the export didn't place are never recorded,
  so never set aside. Undo of an update restores the record from the files on disk;
  Undo of the first export forgets it.
- **Check** reads each recorded file (archives and images by their members) and
  compares it with its game's ROMs in the DAT it was exported from (or the system's
  current DAT of that name): *missing*, *changed* or as it should be. A cue sheet is
  checked by being there (the export may rewrite it).
- **The page**: Collections (rules beside the games, a page at a time with a search,
  Leave out and Put back, Add games…, Save as DATs…, Export…, the collection's exports
  with Update…, Check and Forget), Settings › Systems, and the DAT question (and Make a
  DAT) offering every system, added ones too, or *A new system…* named there. Rule
  changes are saved one after another, each on the collection as the last left it.
- **Not as designed**: the export folder can't hold a folder with games either (its
  copies would be found by scans); gamelist entries are added, never removed (ES-DE
  drops entries whose files are gone).
- **To check on a real set** (Jerred): export a collection to ES-DE's ROMs folder and
  open ES-DE: the systems, a cue/bin game shown once, the gamelist names and the custom
  collection; update it after a rule change.

---

### Phase 4 design (written 2026-10-17; Jerred asked for it to be built straight away)

Phase 4 brings **PC games** into the library (3.5), adds them to **Steam** as non-Steam
shortcuts with their artwork, and adds **Add a game…** and **Identify** for one game at a
time, PC or not (D24).

**The PC games folder (D25)**

- PC games live in one folder, `<library>/PC/` unless Settings › PC games chooses another
  (library.json `pc.folder`, relative to the library when inside it). Each game is
  `<Title> (<Year>)/` with `install/`, `patches/`, `extras/`, `dlc/`, an optional
  `extracted/` and `game.json` (D6). It is not a folder with games (no DAT knows PC
  games, and hashing every installer on each scan would be slow): the PC games page reads
  the `game.json` files, which hold everything the app knows about each game.
- `game.json`: `id` (made once, never changed), `title`, `year`, `source` (gog, steam,
  retail, other), `version`, `ids` (gog, steam, launchbox…), `details` (developer,
  publisher, released, genres, description), `files` (path in the game's folder, kind
  install/patch/extra/dlc, size, CRC32, SHA1, version), `launch` (program, arguments,
  working folder) and `installed` (a folder outside the library where it's installed).
  Unknown keys are kept when the app rewrites it.
- Scraped artwork for a PC game goes in `_library/games/pc/<id>/media/` (D6, D13): cover
  (portrait), banner (horizontal), hero, logo, icon.

**Import (3.5, D26)**

- *Import…* reads an **inbox** (a folder remembered on this computer, Downloads by
  default) or the files and folders chosen, as a job: each top-level file or folder is an
  item, except that a GOG installer's `setup_<slug>…exe` and its `.bin` parts are one
  item and its `patch_<slug>…` files join it as patches.
- Each item is classified (GOG installer, installer, MSI, archive, disc image, folder),
  hashed (CRC32 and SHA1, one read) and identified, best first:
  1. a `goggame-<id>.info` file (in a folder or an archive): GOG id, name and the
     program to start;
  2. `innoextract --gog-game-id` on a GOG installer, when innoextract is found (D14);
  3. `steam_appid.txt` or `appmanifest_<id>.acf`: Steam app id;
  4. the name: `setup_<slug>_<version>_(<build>).exe` gives GOG's slug and the version;
     `Title (Year) [Source] v1.2` gives those;
  5. a search of the sources (below) with that title, taking a result only when its title
     is the same (ignoring case and punctuation) and its year agrees, if there is one.
- Items that need a look say so; *Identify…* settles them. An item whose files are all in
  a PC game already (by SHA1) is *in the library already* and left out; one with the same
  GOG or Steam id as a game there goes into that game's folder (a newer installer, a
  patch) and its `game.json` lists both versions.
- The import is an Organise change (D26): files are copied (hardlinked on the same
  drive) by default, or moved if you choose (D5), each checked, with the preview, the
  journal and Undo; `game.json` is written by the change, and the one it replaces is set
  aside so Undo puts it back.
- **Unpack** puts a copy of the game in `extracted/`: ZIP and 7z by the app, a GOG
  installer by innoextract. The program to start is taken from `goggame-*.info` when
  there is one, else chosen from the `.exe` files found (*Choose the program…*).
  *Installed at…* points at an installed copy elsewhere instead. Removing the unpacked
  copy is a button.

**Identify and the sources (D24, D27)**

- One dialog for any game: a search box (filled from the name), the system, and results
  from each source in turn (title, year, system, cover, source). Picking one stores the
  source's id and details in `game.json` (for a ROM game without a DAT match,
  `_library/games/<system>/<id>/game.json`, keyed by its SHA1) and fetches its artwork.
- Phase 4's sources need no account: **Steam** (store search and app details), **GOG**
  (catalogue search and product details), both PC only, and **LaunchBox Games DB**,
  every system, from its `Metadata.zip`, fetched on request (*Settings › Sources › Get
  LaunchBox's database*) and kept on this computer as `<data dir>/launchbox.sqlite`,
  searched there. Their order and which are on are a Settings choice on this computer
  (config.json `sources`), as their keys will be when Phase 5 adds sources that need one.
- Every source's address can be changed in config.json (as `dat_source` can), which is
  how the tests serve their answers locally.

**Add a game… (D24)**

- For a PC game it's Import with one item. For any other system: choose the file (a
  ROM, an archive, a disc's sheet or image), the system (guessed from its extension;
  an existing system or a new one) and the folder with games it goes into (the system's,
  or one chosen), copied or moved. After the change the app scans the file: a DAT match
  is offered *Name it as the DAT does* (Organise for that game); no match opens Identify.

**Steam (3.4, D28)**

- *Add to Steam…* finds Steam (`HKCU\Software\Valve\Steam\SteamPath`, then the usual
  folder, or the folder set in Settings) and its users (`userdata/<id>/`, named from
  `config/loginusers.vdf`). Every PC game with a program to start becomes a non-Steam
  shortcut: its name, program, working folder, the tag "Game Repo", and its artwork
  copied to `userdata/<id>/config/grid/` as Steam names it (`<appid>p` portrait,
  `<appid>` banner, `<appid>_hero`, `<appid>_logo`, `<appid>_icon`). The shortcut's id is
  Steam's own rule (CRC32 of program and name, with the top bit set), so the same game
  keeps its id.
- `shortcuts.vdf` is binary; the app reads it, keeps every shortcut that isn't its own,
  replaces its own (tagged "Game Repo") with the current list and writes it back. Only
  while Steam is closed (the app checks and says so); the file is backed up first
  (`shortcuts.vdf.gamerepo-<time>`, the last five kept), and *Put back the shortcuts as
  they were* restores the latest backup and removes the artwork it added.
- Artwork: from Steam's CDN for a game with a Steam id, else GOG's images, else
  LaunchBox's; fetched once into the game's media folder.

**Not in Phase 4**: running installers (unpack or *Installed at…* instead), emulated
games in Steam (a launcher command per system, as Steam ROM Manager does), DLC and
version choice beyond keeping the files, sources that need an account (ScreenScraper,
IGDB, TheGamesDB: Phase 5), scraped media for ES-DE (Phase 5).

**Decisions proposed for Phase 4** (defaults the build follows unless changed)

- **D25** PC games are in their own folder (`<library>/PC/` by default), listed from
  their `game.json` files, not scanned as a folder with games.
- **D26** Import is an Organise change (copy or hardlink by default, move if chosen),
  with the preview, journal and Undo; duplicates are found by SHA1.
- **D27** Phase 4's sources are Steam, GOG and LaunchBox (no accounts); their order and
  LaunchBox's database are kept on this computer.
- **D28** Steam's shortcuts are written only while Steam is closed, backed up each time,
  and the app's own are tagged "Game Repo" so the rest are never touched.

### Phase 4 notes (built 2026-10-17)

- **Core**: `pcgames.rs` (D25: the PC games folder, library.json `pc.folder`; `game.json`
  per game as `PcGame`, unknown keys kept in `extra`; the inbox read into `Item`s: GOG
  installers grouped with their `.bin` parts and patches, discs with their tracks,
  folders walked; each hashed, looked inside (a folder's files, a ZIP's names and small
  files, other archives under 256 MB), then named, then searched; `plan_import` makes the
  Organise change, D26; `unpack` and `remove_unpacked`), `identify.rs` (D27: the sources,
  their order in config.json `sources`, every address in `source_urls`; Steam's store
  search and appdetails, GOG's catalogue and api.gog.com products, LaunchBox; `best` for
  the import, which checks the year after fetching details since Steam's search has
  none), `launchbox.rs` (Metadata.zip streamed into `<data dir>/launchbox.sqlite`: games,
  alternate names, images; scored by words), `steam.rs` (D28: binary VDF read and written
  back byte for byte, keys read in any case; the shortcut id; Steam's folder from the
  registry or the usual places; users from `userdata/` and `loginusers.vdf`, the latest
  sign-in first; shortcuts written with their grid art and a record in
  `<data dir>/steam/<user>.json`), `addgame.rs` (Add a game… for other systems).
  `api.rs` routes `pc_*`, `identify_*`, `sources*`, `launchbox_fetch`, `steam_*` and
  `add_*` to `pc_call`; `organise_run` fetches an import's artwork afterwards
  (`game.json` keeps the URLs as `art` until then) and scans an add's files, returning
  what each is (`added`). The command line has `pc`, `identify`, `sources`, `launchbox`,
  `steam` and `add`.
- **Checked against the sources' real shapes** (a research pass reading Steam ROM
  Manager, Heroic, Lutris, Playnite, RomM and innoextract, and the live Steam and GOG
  answers): Steam's store answers `null` when it's had too many requests, keys appdetails
  by another id now and then, and HTML-escapes descriptions; many games keep their library
  art under a hashed folder, which `IStoreBrowseService/GetItems` (no key) names, so that
  is asked first and the fixed names are the fallback. A `goggame-*.info` for a DLC sits
  beside the game's (only `gameId == rootGameId` counts) and paths may use `//`;
  `innoextract --gog-game-id` prints just the id with `--silent`. Steam stopped writing
  `MostRecent`, so the latest `Timestamp` is the user signed in last.
- **The page**: PC games (the list, a game's panel with its title, program, Unpack and
  Identify, Import… and its review list, Add to Steam…), the Identify dialog (which
  takes the review list's or Add's place and comes back to it), Add a game… on the Audit
  page's ⋯ menu, Settings › PC games, Sources and Steam, innoextract in Settings › Tools.
- **Not as designed**: *Installed at…* is `installed` in `game.json` and `pc set`'s
  program can be any path, but the page has no button for it yet; Steam's *Put back*
  removes the app's shortcuts and their art (keeping yours) instead of restoring the
  latest backup, which would lose shortcuts you added since; Add a game… doesn't guess
  the system from the extension (the Audit page's system is chosen) and its DAT match is
  named by Rename these on the Misnamed list rather than a button of its own.
- **To check on a real set** (Jerred): import a few GOG installers and zipped games from
  an inbox; Identify one; Unpack a GOG installer (with innoextract in the tools folder);
  Add to Steam with Steam closed, then open Steam and look at the shortcuts' artwork.

### Phase 5 design (written 2026-10-17; Jerred asked for it to be built straight away)

Phase 5 fills the library with each game's **details, pictures, videos and manuals**
(3.6), lets the user **attach their own documents** one at a time or in bulk and find
them on a **Documents** page (3.8), and has every export **choose what it carries**
(D30). Identify gains the sources that need an account (D24).

**A game's folder (D13, D32)**

- A DAT game's folder is `_library/games/<system>/<DAT game name>/` (the name made safe
  for a folder), holding `game.json`, `media/` and `docs/`. It is made the first time
  anything is kept for the game. A ROM no DAT knows keeps Phase 4's
  `_library/games/<system>/<SHA1>/`.
- `game.json`: `id` (made once), `platform`, `name` (the DAT game's name), `dat` (the
  DAT's name), `roms` (name, size, CRC32, SHA1 of each of the game's ROMs, from its DAT),
  `title`, `year`, `ids` (screenscraper, igdb, thegamesdb, launchbox…), `details`
  (description, developer, publisher, released, genres, players, rating), `scraped`
  (when, and which source gave each field and picture), `review` (the candidates when a
  scrape wasn't sure). Unknown keys are kept.
- The game is found by its ROMs' hashes, not its name (D13): the index keeps every
  `game.json`'s ROMs (`meta_games`, `meta_roms`), so a DAT game is joined to its folder
  by SHA1, or CRC32 and size. A DAT update that renames the game still finds the folder;
  the folder keeps its old name until the next scrape of that game, which renames it
  (Phase 6 makes that a journalled change).
- The index reads the game folders when the library opens and after every scrape,
  attach or import, by each `game.json`'s size and modified time, so a hundred thousand
  of them are read once.

**Scraping (3.6, D30)**

- *Scrape…* on the Metadata page, for: a system's games in the library, **every game in
  the system's DAT** (owned or not), or a collection. The dialog chooses:
  - **sources**, in order, from those set up in Settings › Sources (off ones can be
    turned on for this scrape);
  - **details** to take: description, developer, publisher, release date, genres,
    players, rating;
  - **pictures and more**: box front, box back, 3D box, screenshot, title screen,
    marquee (logo), fan art, cartridge or disc, video, manual; each from the first source
    in the order that has it, or from one source chosen for that kind;
  - **regions** preferred for names and pictures (from the DAT game's region first,
    then the list, `us, eu, wor, jp` by default);
  - **games that have something already**: fill in what's missing (the default),
    replace, or leave them.
  The choices are saved in library.json `scrape` as the next scrape's defaults.
- Each game is asked of the sources in order. **ScreenScraper** is asked by the ROMs'
  hashes (CRC32, MD5 when known, SHA1, size and name), which works for games the user
  doesn't have, since the DAT records the hashes; then **IGDB**, **TheGamesDB** and
  **LaunchBox** by the DAT game's title (its name without the tags) and system. A
  name-only answer is taken when its title is the same (ignoring case, punctuation and
  a leading "The") and its year agrees with the DAT's, where both have one; otherwise the
  best few are kept as the game's `review` and it is listed under **Needs a look**, where
  Identify settles it.
- A scrape is a job (progress, Stop). Each source has its own pace: ScreenScraper one
  request at a time (its free accounts allow one thread) and slowing down when it says
  it's busy, IGDB four a second, TheGamesDB and the rest one a second. Every answer is
  kept on this computer (`<data dir>/scrape-cache/`) for a week, so a stopped scrape
  starts again where it stopped.
- Pictures go in `media/` by kind (`cover`, `back`, `box3d`, `screenshot`,
  `titlescreen`, `marquee`, `fanart`, `physical`, `video`), as Phase 4's PC artwork does.
  A scraped manual is a document of kind *manual* in `docs/`, named
  `Manual (ScreenScraper).pdf`, and never replaces one of the user's.

**Sources that need an account (D8, D24, D33)**

- **ScreenScraper**: a ScreenScraper account (user name and password) and the app's
  developer id and password, which ScreenScraper gives each application on request.
  Game Repo has none yet, so Settings › Sources has a place for all four; the app
  works without them.
- **IGDB**: a Twitch developer application's client id and secret (free); the app
  fetches the access token and renews it.
- **TheGamesDB**: an API key (free, on request from TheGamesDB).
- The keys are kept on this computer in config.json `source_keys`, never in the library
  (D33). Settings › Sources says which sources are ready and which need a key, and
  *Check* tries them.
- Identify (Phase 4's dialog) searches these too, in the same order.

**The Metadata page**

- A system's games (owned, or every game in its DAT) with what each has: details,
  pictures, documents, and whether it needs a look. Search and paging as the Audit
  lists do. *Scrape…* above it.
- A game's panel: its title, year, details, pictures (the video plays), its documents
  (opened in the side viewer), *Attach documents…*, *Identify…* (the dialog, with the
  scrape's candidates first when it needs a look), *Scrape this game* and *Clear* (the
  game's scraped details and pictures go; the user's documents stay).
- **Needs a look**: the games a scrape wasn't sure about, each with its candidates.

**Documents (3.8)**

- A game's documents are the files in its `docs/` folder. `docs/docs.json` keeps each
  one's kind, title and source (missing ones are guessed from the name and format:
  manual, guide, FAQ, map, poster, box scan, cartridge or disc scan, soundtrack, readme,
  magazine, other). Documents of no game live in `_library/docs/` (by system, or
  General) with a `docs.json` of their own.
- **Attach documents…** (a game's panel, or files dropped on it) and **Import
  documents…** (the Documents page, for a folder) are one change: the files picked or
  the folder's (walked), each paired with a game (Import) and given a kind; a review
  list shows each file with its game, how sure the pairing is, and its kind, and lets
  each be changed (search for another game, *No game*, or *Leave out*); then the
  preview, and the change copies (or moves) the files into the games' `docs/` folders
  and writes their `docs.json`, journalled with Undo.
- Pairing: the file's name, without its extension, the kind words ("manual", "guide",
  "FAQ"…) and the tags, compared with the DAT games' titles of the system chosen (or
  every system with a DAT, the system's name in the path deciding when it can):
  the same title is *sure*, a close one (by words in common) *likely*, else none.
- **The Documents page** lists every document: title, kind, game, system, source, size.
  Search covers titles, file names and the **text inside** PDFs, text and Markdown
  files (read once into the index's full-text table on this computer, as a job), with
  filters by kind, system and source. A document opens in the side viewer (PDFs in the
  system's PDF viewer, pictures, text and Markdown as text, audio plays); *Open
  outside* uses the program Windows chooses. Model Library's viewer carries over.

**Exports choose what they carry (D30, D34)**

- The Export dialog gains **What it carries**: which details go in the gamelist, which
  pictures (and the video), manuals and other documents, and the **largest picture
  size** (as they are, or 1920, 1280, 640 or 320 pixels on the longest side, made
  smaller by the app as JPEG or PNG; a handheld's SD card wants small ones). The
  choices are kept with the export and used by *Update*.
- **ES-DE**: each game's gamelist entry gets the details chosen (name, desc, rating,
  releasedate, developer, publisher, genre, players), replacing those the export wrote
  before and keeping the user's other entries and fields; pictures go to
  `<ES-DE>/downloaded_media/<system>/<covers|backcovers|3dboxes|screenshots|
  titlescreens|marquees|fanart|physicalmedia|videos|manuals>/<the game's file name>.<ext>`.
- **A plain folder**: pictures and documents beside each game's file, in a folder named
  after it, when chosen.
- Pictures and documents are copied (hardlinked on the same drive) as the export's
  steps, checked, recorded, set aside by *Update* when they go, and undone with it.
- Steam's grid artwork (Phase 4) is unchanged.

**Not in Phase 5**: renaming a game's folder when its DAT renames it (Phase 6), the
Library page with thumbnails and filters by metadata (Phase 6), arcade metadata
(Phase 7), MobyGames (Phase 8), device profiles (Phase 5b), magazines (D31), reading text
from scanned PDFs (only PDFs with text are searched inside).

**Decisions proposed for Phase 5** (defaults the build follows unless changed)

- **D32** A DAT game's folder is named after it and found by its ROMs' hashes, recorded
  in its `game.json`; the index keeps them for joins.
- **D33** Sources' keys are kept on this computer (config.json `source_keys`), not in the
  library; the scrape's choices are the library's (library.json `scrape`).
- **D34** Documents are kept by kind in `docs.json` beside them; their text is searched
  from the index, read once.
- **D35** An export keeps what it carries with it; the app makes pictures smaller itself
  (no external tool).

### Phase 5 notes (built 2026-10-17)

- **Core**: `meta.rs` (D32: a DAT game's folder and its `game.json`; the index's
  `meta_games` and `meta_roms`, read from each `game.json` by size and modified time;
  the Metadata list, a game's panel, Clear), `scrapers.rs` (ScreenScraper's `jeuInfos`
  by hash and `jeuRecherche`, IGDB's Apicalypse queries with a Twitch token renewed on
  401, TheGamesDB's `ByGameName`, `ByGameID`, `Images` and its genre, developer and
  publisher names; each source paced, its answers kept a week in
  `<data dir>/scrape-cache/`, its errors said in plain words (ScreenScraper's are French
  text and status codes); `KEYS`, `needs`, `check`), `scrape.rs` (the scrape's
  `Options`, `ask` for each game in the sources' order, `apply`, `pick`), `docs.rs`
  (D34: the documents, `docs.json`, the index's `docs` and its FTS5 table `docs_text`,
  text from PDFs read with lopdf (capped at a megabyte a document), `search` with
  snippets, `to_html` with pulldown-cmark, the import's `read_items` and `plan`), and in
  `export.rs` (D35) `Carry`, the gamelist's details merged into the entries already
  there, pictures and manuals into ES-DE's media folder (its `MediaDirectory` setting
  when set) or beside each game, and the `image` step that makes pictures smaller with
  the image crate. `identify.rs` searches ScreenScraper, IGDB and TheGamesDB too, when
  their keys are set. New index migration: "the games' folders and their documents".
- **Checked against the sources' real shapes** by reading ES-DE's, Skyscraper's and
  RomM's clients and TheGamesDB's server code (the live services aren't reachable from
  the build session): ScreenScraper sends numbers as strings, its rating out of 20, and
  only media whose `parent` is `jeu` are the game's; its media region order is
  `wor, us, eu, jp` after the user's. The tests serve stand-ins for all three.
- **The page**: Metadata (system picker, the counts as tabs: in the library, every game,
  needs a look, nothing yet; a game's panel with its candidates, Scrape…, Identify…,
  Attach documents…, Clear), the Scrape… dialog, Documents (search, kind, system and
  source filters, the viewer with Model Library's PDF frame), Import documents… with its
  review list and game picker, the Export dialog's *What it carries*, Settings › Sources'
  keys and Check. Metadata and Documents replace the last of the pages still to come.
- **Not as designed**: a game's folder is found by holding exactly its ROMs (CRC32 and
  size), else by its DAT name on the same system, rather than by any one ROM's hash: a
  ROM two games share (Epsilon's track that is Beta's ROM, in the tests) mustn't decide
  it. The page's Scrape… offers a system's games or its whole DAT; a collection's games
  (and chosen ones) are scraped from the command line or a game's panel. The picture
  sizes offered are 320, 480, 640, 800 and 1080 pixels (handheld screens), not 1920 and
  1280. Files dropped on a game's panel aren't attached yet; Attach documents… picks
  them. The index reads the game folders when the Metadata or Documents page opens and
  after every change, rather than when the library opens.
- **To check on a real set** (Jerred): set a TheGamesDB key or an IGDB application in
  Settings › Sources and press Check; scrape a small system (Game Boy, say) and look at a
  few games' panels; import a folder of guides and look at the pairings before bringing
  them in; search the Documents page for a word inside one; export a collection to ES-DE
  with pictures and details ticked and open ES-DE. ScreenScraper needs a developer id
  and password for Game Repo, which ScreenScraper gives on request (its forum).

### Phase 5b design (written 2026-10-17; Jerred asked for layouts the user can change)

Phase 5b exports a collection to an **SD card in the layout a device's firmware reads**:
muOS, Knulli, ROCKNIX, or ES-DE on Android or Linux (3.10, D18, D29). Each card gets
the firmware's ROM folders, its game lists, and the pictures, descriptions and manuals it
shows, within the card's size limit. Jerred asked (2026-10-17, D36) that these layouts
not be fixed in the code: new firmwares and frontends come along and change, so each
layout is a **JSON file that can be copied and edited**, and the app has a **visual
editor** that shows what an export would write, folder by folder, lets the user change
it, and saves it as a new layout.

**A layout is a JSON file (D36, D37)**

- The **built-in layouts** ship with the app: *ES-DE* (Phase 3's export, on this
  computer), *ES-DE on Android or Linux*, *Knulli*, *ROCKNIX*, *muOS* and *Plain folder*.
  They can't be changed; a change to one is saved as a new layout.
- The user's own layouts are files in the library, `_library/layouts/<id>.json` (with
  its collections and exports, so they move with it). *Import a layout…* reads a `.json`
  file, *Save as file…* writes one to give to someone else, and *Open the folder* shows
  them, so a layout can be shared or edited in any text editor.
- A layout says where everything goes as **paths made of tokens**, starting from a
  **place** the Export dialog asks for:
  ```json
  {
    "format": 1,
    "id": "knulli",
    "name": "Knulli",
    "about": "Knulli's SHARE partition: a folder per system with its gamelist.xml and media beside it.",
    "places": [{ "id": "card", "label": "The SD card's SHARE partition" }],
    "systems": { "megadrive": "megadrive", "pce": "pcengine", "3do": null },
    "games": { "path": "{card}/roms/{folder}/{file}", "names": "dat" },
    "gamelist": {
      "kind": "es-xml", "path": "{card}/roms/{folder}/gamelist.xml",
      "fields": { "description": "desc", "developer": "developer", "publisher": "publisher",
                  "released": "releasedate", "genres": "genre", "players": "players", "rating": "rating" }
    },
    "media": {
      "cover":      { "path": "{card}/roms/{folder}/images/{stem}-thumb.{ext}",  "tag": "thumbnail" },
      "screenshot": { "path": "{card}/roms/{folder}/images/{stem}-image.{ext}",  "tag": "image" },
      "marquee":    { "path": "{card}/roms/{folder}/images/{stem}-marquee.{ext}", "tag": "marquee" },
      "video":      { "path": "{card}/roms/{folder}/videos/{stem}-video.{ext}",  "tag": "video" },
      "manual":     { "path": "{card}/roms/{folder}/manuals/{stem}-manual.pdf",  "tag": "manual" }
    },
    "text": {},
    "pictures": { "max": 640, "as": "keep" },
    "collection": null
  }
  ```
  - **Tokens**: `{<place>}` (where a place is on this computer), `{folder}` (the
    system's folder in this layout), `{system}` (the system's id), `{system_name}` (the
    layout's name for the system, else the app's), `{file}` (the game's file or folder,
    as exported), `{stem}` (that without its extension), `{title}` (the game's title),
    `{ext}` (the picture's or document's own extension).
  - **`systems`**: the layout's folder for each system; a system not listed uses its id
    (ES-DE's name, which most firmwares share). An entry can be an object,
    `{ "folder": "SFC", "name": "Nintendo SNES-SFC" }`, when the firmware also names the
    system (muOS's catalogue). `null`: the firmware doesn't run it, so its games are
    left out and the export says why.
  - **`gamelist`**: `es-xml` (EmulationStation's `gamelist.xml`, which ES-DE, Knulli,
    ROCKNIX and Batocera read), with the tag each detail goes in; a picture with a `tag`
    is written into the entry too, as a path relative to the gamelist (`./images/…`).
    ES-DE's own layout tags none, since ES-DE finds them by name. `null`: no gamelist
    (muOS). Phase 8a adds `pegasus`, `retroarch` and `launchbox` ("Phase 8a notes").
  - **`media`**: where each kind of picture (meta::MEDIA), the video and the manual go;
    a kind left out isn't offered. **`text`**: details written as text files (muOS's
    description, `{ "description": { "path": "…/text/{stem}.txt" } }`).
  - **`pictures`**: the size pictures are made for this device (the Export dialog can
    change it) and `as`: `keep`, or `png` for a firmware that only shows PNG (muOS).
  - **`collection`**: `esde-cfg` writes ES-DE's custom collection (Phase 3); `null`
    writes none.
  - `format` is the file's version: a layout from a newer Game Repo is refused in plain
    words, and keys the app doesn't know are kept. A copy keeps `copied_from` (the
    built-in's id and version), so the editor can say when the built-in has changed
    since.
- **Checked when it's saved or imported**, each problem said beside the line it's on:
  every path starts with a place; no `..`; every token is known; the games' path has
  `{file}`; a game's picture, text or manual path has `{stem}`, `{file}` or `{title}`;
  two kinds don't write the same file; a gamelist's tagged pictures are under the
  gamelist's folder.

**The built-in layouts** (each checked against its firmware's documentation as it's
built; what couldn't be confirmed there is said in the notes)

- **ES-DE** (this computer): places *ROMs folder* and *ES-DE's folder*; exactly what
  Phase 3 and 5 write today, custom collection included.
- **ES-DE on Android or Linux**: one place, the card. `{card}/ROMs/{system}/{file}`, and
  ES-DE's `gamelists/` and `downloaded_media/` in `{card}/ES-DE/`, to be copied to the
  device's ES-DE folder or used by pointing ES-DE's media folder at the card. ES-DE on
  Android keeps its own folder on internal storage, which a card can't reach.
- **Knulli**: `{card}/roms/{folder}/{file}` on the SHARE partition (Knulli's
  documentation: "only scans for games in the subfolders of the roms folder"; folder
  names are case-sensitive), with each system's `gamelist.xml`, `images/`, `videos/`
  and `manuals/` beside its games, as in the example above.
- **ROCKNIX**: `{card}/roms/{folder}/{file}` (ROCKNIX's documentation: the card's `roms`
  folder, `/storage/roms` on the device), with the same gamelist and media as Knulli.
- **muOS**: `{card}/ROMS/{folder}/{file}`, and its catalogue,
  `{card}/MUOS/info/catalogue/{system_name}/box/{stem}.png` (box front),
  `…/preview/{stem}.png` (screenshot) and `…/text/{stem}.txt` (description); no
  gamelist; pictures as PNG. The catalogue's system names are those of muOS's
  `info/assign/<system>.ini` (`catalogue=`), such as "Nintendo SNES-SFC" (muOS's
  documentation, *Artwork*).
- **Plain folder**: Phase 3 and 5's plain folder, pictures and documents beside each
  game.
- Pictures are made 640 pixels at most for the handhelds by default (the screens are
  640×480 or smaller), as they are now for Phase 5's exports (D35).

**The editor (Settings › Layouts)**

- On the left, the layouts: the built-in ones (marked *built in*) and the user's own,
  with *New from…*, *Import a layout…*, *Save as file…* and *Remove*.
- In the middle, **On the card**: a folder tree of what an export with this layout would
  write, worked out from a sample: the first few games of up to three systems of a
  collection chosen above it, with their own pictures and manuals; or, with nothing in
  the library yet, a stand-in game per system. Each file says what it is (the game, box
  front, gamelist, description…). Folders open and close; an empty kind (a game with no
  video) is drawn faintly, so it can still be chosen.
- On the right, whatever is chosen in the tree: a file's path, with its tokens as
  buttons to put in and what it comes to for the sample game shown under it; its
  gamelist tag; *Leave it out*. A system's folder: its folder name and the name the
  firmware gives it, or *Not on this device*. The gamelist: on or off, where it goes,
  and the tag for each detail. The pictures' size and format. Every change redraws the
  tree at once, and problems are listed under it.
- **Save**: a built-in layout is never changed, so the first change makes a draft and
  Save asks for its name and saves it as a new layout (*Knulli (mine)*). The user's own
  layout is saved in place; *Save as new…* copies it.
- **JSON**: the layout as text, editable, checked as it's typed, for copying to and from
  a forum post or another computer.
- *Compare with the built-in* shows what a copy changed, when it was copied from one.

**Exporting to a card**

- The Export dialog's targets are the layouts (ES-DE first, then the devices, then the
  user's own). It asks for the layout's places (a card's drive, `E:\`, say), and the rest
  is as before: what it carries (only kinds the layout has a place for), picture size,
  Update, Check, Undo.
- **One engine (D38)**: Phase 3's ES-DE and plain-folder exports become the built-in
  *ES-DE* and *Plain folder* layouts run through the same code; a test checks they write
  exactly what they wrote before. Exports made before keep working, as those layouts.
- An export remembers its layout's id. *Update* uses the layout as it is now, so after
  the layout is changed, files it no longer wants are set aside and the new ones written,
  as Phase 3 does when a collection changes.
- **Size limit**: the export adds up the games and everything it carries, and says so
  before it starts when that's more than the collection's size limit or the card's free
  space.
- Gamelists are merged as they are for ES-DE (D35): entries for the exported games are
  added or updated, and the device's own entries (play counts, favourites) kept.

**BIOS files (D40)**

Jerred asked (2026-10-17) for each system's BIOS files to be kept as a set of their
own: a separate category, audited for what each system is missing, and exported with a
set of games so another device has what they need.

- **What a system needs** comes from a **BIOS DAT**: libretro's `System.dat` (its public
  database, `dat/System.dat`, which lists every core's BIOS and firmware files with their
  sizes and hashes, grouped by system) is offered by *Get starter DATs* beside the
  game DATs, and any other BIOS DAT (a firmware's own list, one made by hand) can be
  imported as a system's BIOS DAT. A file a system needs is *required* or *optional* as
  the DAT says (libretro's marks some optional), and keeps its folder (`dc/dc_boot.bin`,
  `np2kai/font.bmp`), since cores look for it there.
- **The BIOS page** (in the nav after Audit): each system with a BIOS DAT, its files as
  *Have*, *Missing* or *Wrong* (a file of the right name whose hashes don't match, a bad
  dump or another region's), required ones first, and the systems missing a required
  file at the top. Found anywhere in the folders with games, by hash like any game
  (Phase 1's scan), so nothing has to be moved first. *Report* saves the list of what's
  missing, as Phase 1's reports do.
- **The BIOS set**: *Gather the BIOS files* puts a copy of each one found into the
  library's own `BIOS/<system>/` folder (named and nested as its DAT says; hardlinked on
  the same drive, D5), as an Organise change with a preview and Undo. That folder is a
  folder with games like the others, scanned with them, so the set stays audited.
- **Exported with the games**: the Export dialog's *What it carries* gains **BIOS files
  for these systems** (required ones, or optional ones too), for the systems the
  collection's games are on. Each layout says where they go, as `"bios": { "path": … }`,
  with the token `{bios_file}` (the file's name with its DAT folder): Knulli
  `{card}/bios/{bios_file}`, ROCKNIX `{card}/roms/bios/{bios_file}`, muOS `{card}/MUOS/bios/{bios_file}`, ES-DE on
  Android or Linux `{card}/BIOS/{bios_file}`, to point RetroArch's system folder at,
  and the plain folder `{out}/BIOS/{system_name}/{bios_file}`. The editor shows them in
  the tree like the rest. Each layout's BIOS folder is checked against its firmware's
  documentation when it's built.
- Game Repo never ships, downloads or names a source for BIOS files: it audits and
  copies the user's own (6, legal notes).

**Not in Phase 5b**: converting games on the way to the card (a format the device can't
play is the collection's to choose; 3.9's convert-on-export comes later), `.m3u` lists
for multi-disc games, writing to a device over the network (Phase 9), reading a card's
own layout to make a layout from it, ArkOS and Batocera (if asked for).

**Decisions proposed for Phase 5b** (defaults the build follows unless changed)

- **D37** Layouts are JSON files: built-in ones in the app, the user's own in the
  library (`_library/layouts/`); a built-in one is changed only by saving a copy.
- **D38** One export engine: ES-DE and the plain folder are built-in layouts like the
  devices' ones.
- **D39** The editor's tree is worked out from the user's own games (a collection's
  first few), not a fixed example, so it shows the names the card will really hold.
- **D41** A system's BIOS files are known from a BIOS DAT (libretro's `System.dat` by
  default) and gathered into the library's `BIOS/<system>/`, as copies.

### Phase 5b notes (built 2026-10-17)

- **Core**: `layout.rs` (D36, D37: the layout's JSON, unknown keys kept and a newer
  `format` refused; places, tokens, `expand`, `check`; the built-in layouts compiled in
  from `desktop/core/layouts/*.json`, the user's in `_library/layouts/<id>.json`, saved
  as a copy with `copied_from {id, version}`, imported and written as files), `bios.rs`
  (D40, D41: BIOS DATs kept as the system `bios`; libretro's System.dat, one game with a
  comment before each system's files, read as a game per system by `by_system`;
  `ALIASES` from libretro's system names to Game Repo's; the summary, the report,
  `gather_plan`, `found_for`), and `export.rs` (D38: `plan` works only from the layout;
  Phase 3's `target`, `out` and `esde` are mapped onto the built-in `esde` and `folder`
  layouts, so exports made before keep working, and the Phase 3 and 5 tests pass
  unchanged; text files for muOS; BIOS files; `sample` and `preview` for the editor).
  The commands are `layouts`, `layout_*`, `bios`, `bios_report`, `bios_gather_plan`;
  `export_plan` takes `layout` and `places`. A DAT import's answer `"*"` names the
  platform for every DAT in it (Import a BIOS DAT…).
- **The page**: *Export layouts* is a page of its own, in the menu after Collections and
  linked from Settings, rather than a Settings section: three columns didn't fit there.
  Its ⋯ menu has Import a layout…, Save as a file…, Open the layouts folder, Save as a
  new layout…, Undo the changes and Remove; there's no *New from…*, since changing any
  layout and saving makes the new one. *BIOS files* is in the menu after Audit, with
  Gather…, Get libretro's list…, Import a BIOS DAT… and the report. The Export dialog
  picks a layout, asks for its places, and offers in *What it carries* only what the
  layout has a place for, its picture size chosen from the layout's.
- **Not as designed**: libretro's System.dat marks no file required or optional, so the
  BIOS page and the export don't either: a system's files are listed as its DAT lists
  them, and the export carries every one the library has for the collection's systems.
  The BIOS page lists the systems you have games for first. Gathered files go in
  `BIOS/<the DAT's name for the system>/` (`BIOS/Nintendo - Gameboy/`), taken out of a
  ZIP when that's where it is; an export carries loose files only, so a BIOS file in a
  ZIP is exported once it's gathered. The plain
  folder's BIOS path is `{out}/BIOS/{dat_name}/{bios_file}`. A picture with a gamelist
  tag must be under the system's games folder, since gamelists give `./` paths. The
  export's size check is the drive's free space, as before; the collection's size limit
  is still said in the dialog from the games' size alone.
- **Tests**: the core's layout tests (the built-in layouts pass their check, paths made
  from places and tokens, the check's problems and where they are, a built-in changed by
  saving a copy and a newer format refused), a collection exported to Knulli's and muOS's layouts and again
  after a layout is changed, the editor's preview with a stand-in and a clash, and BIOS
  files audited, gathered and exported; the page test gets libretro's list from a
  stand-in, audits and gathers, edits Knulli's layout in the tree (a system's folder,
  a path that breaks it), saves it as *Knulli (mine)*, and exports to a card in it with
  the BIOS files.
- **To check on a real card** (Jerred): the layouts follow each firmware's documentation
  and Batocera's scraper, not a real card. Export a small collection to each device's
  card with pictures and BIOS files ticked and look at it on the device: Knulli's and
  ROCKNIX's picture names (`images/<game>-thumb.png`, `-image`, `-marquee`), muOS's
  catalogue folders (its names for systems other than Game Boy and SNES), Batocera's
  folder names (`megadrive`, `wswanc`, `gamecube`…) and each BIOS folder. A
  `gamelist.xml` from a device that scraped its own games would settle the picture
  names. Anything wrong can be put right in a copy of the layout, and the built-in one
  fixed in the next release.

### Phase 6 design (written 2026-10-17)

Phase 6 brings one page for every game the library holds, across every system, browsed
by its pictures and found by its details, and a page for each game. It also ties off
what Phases 3 and 5 left: renaming a game's folder when its DAT renames the game, and
scraping or attaching from the page instead of the command line.

**What the Library shows**

- **Every game you have**: each DAT game with a file that matches it, in each system's
  DAT in use (the audit's *Have*). Each PC game (`PC/<Title> (<Year>)/game.json`). Each
  game Add a game… kept that no DAT knows (`_library/games/<system>/<SHA1>/`). BIOS
  files stay on their own page.
- A switch shows **every game in the DATs** instead, so games you've scraped but don't
  have can be browsed too, drawn faintly.
- All regions are shown by default. *One per title* keeps one game of each title, in
  the collection rules' region order (Phase 3's 1G1R).

**The index (D42)**

- A new table, `library`, holds one row per game:
  - what it is: the kind (DAT, PC or added) and its key
  - its system, DAT name, title, and the name it sorts by (*The Legend of Zelda* under L)
  - its details: year, developer, publisher, players, rating, regions, languages
  - its state: whether you have it, its folder, the picture shown for it, which kinds
    of media it has, how many documents, and its size on disk
- Genres and other details with several values go in `library_values (game, field,
  value)`, indexed by field and value. Search uses an FTS5 table of the title, the DAT
  name, the description, the developer and the publisher.
- `meta_games` gains the details read from `game.json` (a new migration), so the
  library is built from the index without opening a folder.
- It's rebuilt **one system at a time**, in a transaction, when that system's audit
  generation or its game folders change. A scan that touches one system rebuilds only
  that one.
- Lists page through it with keyset paging (the last row's sort key). Each filter's
  counts come from one GROUP BY over the filtered games, with the top 50 values shown
  and the rest searchable.
- **Target**: 200,000 games, a page of the list in under 100 ms, the filter counts in
  under 300 ms, on a mid-range laptop. A timing test checks it, as Phase 1's do.

**Pictures in the grid (D43)**

- **Thumbnails** are made from the game's cover, else its 3D box, screenshot or title
  screen. A PC game uses its portrait cover. They are 256 pixels on the long side for
  the grid (512 for the large size), as JPEG.
- They're kept on this computer, never in the library: `<data dir>/thumbs/<library
  id>/`, keyed by the picture's path, size and modified time, so a new picture makes a
  new thumbnail. The newest 50,000 are kept.
- Each is made on first request, at most four at once, and served by `library://` like
  the pictures themselves. The page loads only the ones in view (Model Library's
  IntersectionObserver queue).
- A game with no picture shows its title's first letter on its system's colour.

**The page**

- **Command bar**: search (titles, names, descriptions, developers), the view switch
  (Model Library's `ViewSwitch`, with grid, list and table), the sort (title, system,
  year, rating, size, file date) and ⋯.
- **Filters**, on the left, each with counts: System, Genre, Year (by decade, opening to
  years), Developer, Publisher, Players, Rating (at least), Region, Language, *Has*
  (pictures, description, manual, documents, video), *Needs a look*, and *In a
  collection*. Values within one filter combine with OR; separate filters combine with
  AND. The filters in use show as chips above the games, each with ×, and *Clear*. They
  are kept in the page's address, so Back works and a view can be bookmarked.
- **Grid**: covers in three sizes, each with its title and the system as a badge. The
  grid is virtualised (only the rows in view are drawn) and pages are fetched as it
  scrolls, since Model Library's "Show more" won't do for 200,000 games. **List**: a
  row per game with a small cover. **Table**: title, system, year, developer, genre,
  players, rating, region, size and status, with sortable columns.
- **Group by** system, genre, decade or developer: a heading for each group, with its
  count, in any view.
- **Select**: click, Ctrl or Shift, and Ctrl+A for everything filtered, with Model
  Library's keys. What's selected can be:
  - *Added to a collection…*
  - *Scraped…*
  - *Converted…*
  - given *Attach documents…* (for one game)
  - *Shown in folder*
- **Save as a collection…**: the filters become a new collection's rules, so it keeps
  up as games are scraped. Collections gain rules for genre, year, developer,
  publisher, players and rating.
- **The panel** (Model Library's inspector) shows the chosen game's cover, details and
  actions, and *Open* goes to its page.

**A game's page** (`#/game/<id>`)

- Its pictures as a gallery, video included, and its details, each saying where it came
  from.
- **Edit** any detail by hand. It's kept in `game.json` `edited` and wins over later
  scrapes (D44).
- Its documents, in the side viewer.
- Its files: where each is, its format and size, the audit's word on it, *Convert…* and
  *Show in folder*.
- Its DAT's facts: region, languages, revision, and what it's a clone of.
- The collections it's in, to add it to or take it out of.
- *Scrape…*, *Identify…*, *Attach documents…*, and **files dropped on the page are
  attached** (also on the Metadata page's panel).
- Previous and next go through the list it was opened from.

**Loose ends**

- **A game's folder renamed when its DAT renames it (D45)**: after a DAT import, the
  games whose folders have their old names are offered as one change (*Rename 12
  games' folders to their new names*), journalled with a preview and Undo. Until then
  the folder is still found by its ROMs. The command line has `games rename-folders`.
- *Scrape…* for a collection or for the games selected on the Library page (until now
  only on the command line).
- The About page's library layout and the CHANGELOG map follow.

**Not in Phase 6**: starting games (the frontends do that), favourites and play counts
(those are the frontends' too; Phase 8 may read them back), arcade sets (Phase 7),
MobyGames and magazines (Phase 8).

**Decisions proposed for Phase 6** (defaults the build follows unless changed)

- **D42** The Library page reads a `library` table in the index on this computer, rebuilt
  one system at a time, never a file in the library.
- **D43** Thumbnails are a cache on this computer (`<data dir>/thumbs/`), made from the
  library's own pictures when first shown.
- **D44** A detail edited by hand is kept in `game.json` and wins over later scrapes,
  until it's cleared.
- **D45** A game's folder is renamed to its DAT's new name only as a change the user
  runs, with a preview and Undo, never by itself.

### Phase 6 notes (built 2026-10-17)

- **Core**: `browse.rs` (D42: the `library` table, `library_values` and the FTS5 table
  `library_text`, a new migration that also gives `meta_games` the details and picture
  from `game.json`; `refresh` rebuilds a system when its stamp changes, kept in
  `index_meta` as `library:<system>`, PC games being a system of their own; `query`,
  `facets`, `keys`, `overview`, `game` and `edit`; `matching` for collection rules;
  thumbnails and `prune`, D43). `meta.rs` gains `edit_value`/`edit_dir` (D44, the edit
  marked in `game.json` `edited` and `scraped[field] = "you"`, skipped by
  `merge_details`) and `rename_folders_plan` (D45, an Organise change of kind
  `folders`, run and undone like any other). `collection.rs` gains `Rules.details`
  (search, genre, decade, developer, publisher, players, rating, has, look) and the
  `include` value `none`, only the games added by hand. The commands are `browse`,
  `browse_query`, `browse_keys`, `browse_game`, `browse_edit`, `meta_edit` and
  `organise_rename_folders`; thumbnails are served at
  `library://~thumb/<128|256|512>/<path>`. The command line has `library [filters]
  [--facets]`, `games rename-folders [--dry-run]` and `game <id> --edit <field> <value>`
  / `--unmark <field>`.
- **The pages**: *Library* is in the menu after Home (`browse.js`); a game's page is
  `#/game/<system>/<DAT name>` (`gamepage.js`). The query is kept in the address
  (`#/library?genre=Action&sort=year`), a filter change being a new history entry and
  typing in the search replacing it. Previous and Next on a game's page replace the
  entry too, so Back goes to the list it came from.
- **Not as designed**:
  - Lists page by offset, not by key: each sort has a covering index (`year_sort`,
    `rating_sort`, `size_sort`, `date_sort`, with unknowns last) and a page fetches its
    ids first, which keeps a page at 120,000 under 15 ms, so keyset paging wasn't needed.
  - The filters' counts are added up in Rust over the filtered games, one pass for the
    filters not in use and one per filter in use (leaving it out), instead of a GROUP
    BY per filter, which took about a second at 200,000 games. Each filter sends its
    500 most-used values; its Find box searches those.
  - Year is by decade only, not opening to years.
  - Grouping by genre uses a game's first genre; games with no value for the grouping
    go under *Unknown*.
  - Thumbnails load with the browser's own lazy loading rather than an
    IntersectionObserver queue; the core makes at most four at once either way.
  - A game's page lists its files with their format and size but not the audit's word
    on each, and its DAT facts are the DAT, the name, the regions and the ROM count:
    languages, revision and clone-of aren't shown yet.
  - *Add to a collection…* with a new collection makes one with `include: none`, so it
    holds only the games chosen.
- **Timing** (the ignored test `timing_the_library_of_200k_games`, run in release on
  CI): a page in 7–15 ms at any depth and sort, the counts in about 190 ms, a search
  matching every game in about 230 ms (limits 100 ms and 300 ms).
- **Tests**: the core's (the list with 1G1R and refresh, edits kept over a scrape,
  search, filters and counts, a collection by details, a folder renamed to its new DAT
  name and undone, thumbnails made and pruned, the timing test); the page test opens
  the Library with thumbnails, filters by genre and goes Back, searches, picks a game
  and opens its page, edits the developer there, drops a file on the page to attach
  it, steps to the next game and back, and saves the filters as a collection, checked
  against `library --genre Action`.
- **To check with a real library** (Jerred): open the Library on a large scraped set
  and scroll the grid quickly; filter by genre and year; open a game, change a detail,
  scrape it again and see the change kept; import a newer DAT that renames games and
  run *Rename the folders…*.

### Phase 6b design (written 2026-10-17)

Jerred asked (2026-10-17) that each source's metadata be kept apart, that pictures and
video come in variants, that the user choose what is scraped, that the space it takes be
shown (no limit), that metadata be deleted for a system or a game, and that a system or a
game be scraped again. Phase 6b builds these before Phase 7. (An import of ES-DE's own
scrapes was offered and set aside by Jerred, 2026-10-17.)

**Each source's answer kept (D46)**

- A scrape keeps each source's answer in the game's folder, `sources/<source>.json`:
  its id, title, year and the details taken, and when. Only the details chosen in the
  Scrape dialog are kept. *Fill in* adds the fields a source's file lacks; *Replace*
  writes it again.
- `game.json`'s `details`, `title` and `year` are **made from those files**: for each
  field, the value of the game's own pick for it (`pinned`, set on its page), else the
  source chosen for that field in Settings › Sources (*Details from*), else the first
  source in the scrape's order that has it. A hand edit wins over all of them (D44).
- Changing *Details from* makes every game's details again from the files kept, without
  asking the sources.
- On a game's page each detail lists what the other sources said, with *Use this*.
- A folder scraped before this has no `sources/`: its details are split into source files
  by `scraped`, which already says where each came from, the first time it's touched.
- PC games keep their own `game.json` as Phase 4 made it.

**Pictures and video from each source (D47)**

- A kind of picture or video can come from each source that has one. A **variant** is
  one source's, and there is one per source (Jerred, 2026-10-17: a variant is a different
  source's of the same kind, never a second one from the same source).
- The **chosen** variant of a kind stays at `media/<kind>.<ext>`, where the grid, the game
  page, exports and frontends already look. The others are kept at
  `media/<kind>/<source>.<ext>`; `game.json` `chosen` names the chosen one's source.
- **Exports carry only the chosen one**, named as now (`cover.png`, the layout's names);
  the others never leave the library.
- The chosen one is the preferred source's for that kind (the Scrape dialog's *A picture
  from a source of its own*), else the first source in order that has one; each source
  picks its own by the game's region, then the region order, as now. On a game's page
  each kind shows its variants, and *Use this* swaps it in, kept over later scrapes.
- **What to scrape**: the Scrape dialog keeps its choice of details and kinds, and adds,
  for each kind, *from every source that has one* or the chosen one only. Every source
  is the default for pictures, the chosen one for video. There's no size limit.
- **Sizes**: Settings › Metadata shows what the library's metadata takes, for each system
  and each kind (details, each kind of picture, video, manuals, the user's documents).

**Deleting metadata, and scraping again (D48)**

- *Delete metadata…* on the Metadata page (a system) and on a game's page (a game): the
  details from sources, any of the kinds of picture and video, scraped manuals. It's an
  Organise change of kind `metadata`: the files are set aside, with a preview, journalled,
  and undone like any other; the details are made again after it and after its Undo.
  The user's documents and hand edits stay.
- *Scrape again…* (a system, or a game) is the Scrape dialog set to *Replace* and *Ask the
  sources again*, which skips their answers kept from the past week.

**Decisions proposed for Phase 6b** (defaults the build follows unless changed)

- **D46** Each source's answer is kept in the game's folder; the details are made from
  them by field (a game's pick, Settings › Sources, the scrape's order), hand edits first.
- **D47** Pictures and video are kept as one variant per source for each kind, the
  chosen one where it is now and the only one exported; the Scrape dialog chooses which
  kinds and whether every source's; no size limit, the sizes shown in Settings.
- **D48** Deleting metadata is an Organise change with Undo; scraping again replaces and
  asks the sources afresh.

### Phase 6b notes (built 2026-10-17)

- **Core**: `sources.rs` (D46–D48). `keep` writes a source's answer (only the fields
  chosen; *Replace* writes them again, *Fill in* adds what's missing); `make` builds
  `game.json`'s `details`, `title`, `year`, `ids` and `scraped` field by field from
  `pinned`, library.json `details_from` and the scrape's order, hand edits first, and
  marks the folder `by_source`; a folder from before is split by `scraped` first (what
  nothing attributes goes to the one source in `ids`, else to `earlier`, *An earlier
  scrape*). Pictures: `store`, `choose_in`/`choose` (`chosen`, `pinned_media`),
  `variants`. `delete_plan` is an Organise change of kind `metadata` whose `aside` steps
  have the root `dir:<_library/games>`, so they're set aside in
  `_library/games/.gamerepo-set-aside/<id>/`; `organise::run` and `undo` call `after`,
  which makes those games' details again. `scrape::apply` uses them; `Options` gains
  `variants` and `fresh` (`scrapers::FRESH` passes over the week's cache while a scrape
  runs). Add a game…'s Identify keeps its source's answer the same way. `meta::sync`
  skips dot-folders. Commands: `meta_pin`, `meta_choose`, `meta_delete`, `meta_sizes`,
  `meta_details_from`, `meta_set_details_from` (a job making every game's details
  again); `scrape_choices` lists what can be deleted.
- **The pages**: the Scrape dialog's *From every source that has one* and *Ask the
  sources again*; Scrape again… and Delete metadata… in the Metadata page's menu, a
  game's panel there and a game's page's menu; a game's page lists what the other
  sources said under each detail, with *Use this*, and each source's picture of the kind
  shown; Settings › Metadata has *Details from* and the sizes.
- **Not as designed**:
  - *Details from* is in Settings › Metadata, beside the sizes, rather than in
    Settings › Sources (which is about the sources' accounts and keys, on this
    computer, while *Details from* belongs to the library).
  - The Metadata page's *Clear what was scraped* became *Delete metadata…* with Undo;
    `meta_clear` and `game --clear` remain for the command line.
  - Taking the hand-edit mark off a detail brings the sources' value back at once,
    instead of waiting for the next scrape.
  - A picture's URL carries `?v=` (the chosen source on a game's page, the folder's
    stamp in the Library), which the servers leave out, so a picture changed under the
    same name isn't shown from the browser's cache.
- **Tests**: the core's (details by field from two sources, a pick, *Details from*,
  edits, fill and replace, none left; a folder from before split; one picture per
  source, a second from the same source replacing the first, chosen and swapped, the
  sizes, a system's details and boxes deleted and undone; the scrape test asks IGDB
  too, keeps its box beside ScreenScraper's, and scrapes a game again with a pick kept);
  the page test chooses another source's box and description on a game's page, opens
  Scrape again…, deletes the game's metadata and undoes it, and works out the sizes.
- **To check with a real library** (Jerred): scrape a system with two sources and
  *From every source* ticked; on a game's page choose another source's box and
  description, scrape it again and see both kept; export it and see only the chosen box;
  Delete metadata… for a system, look in Settings › Metadata, then Undo.

### UI pass notes (built 2026-10-17)

From the UI review (a Claude Doc, "Game Repo UI review"), which Jerred accepted as
proposed. One PR, in three steps:

- **Everywhere**: a plan has `title` (what it will do, for the preview) beside `label`
  (what it did, for History and the message); paths shown short with the whole path
  in a tooltip (`shortPath`/`baseName` in `library.js`, `organise::shown`/`tail`, and
  a preview's destination shown beside its source as a relative path); messages at the
  bottom right, hidden under dialogs, three lines at most; read-only libraries grey out
  the pages and buttons that would change them; Drafting Sheet headings in the heading
  face.
- **The pages used most**: Library (regions on tiles and lines, no System column when
  grouped by system, quieter covers); a game's page (details as text until pointed at,
  the main source named once); Settings' section index (`SectionIndex`, sticky, hidden
  under 1000px wide); the Scrape dialog's kinds as a table, the rest under *More
  choices*.
- **Tidying**: Home without the index card (now *What's in it* in Settings › The
  index); Metadata's panel with *Open its page*, *Scrape…* and a ⋯ menu; Collections'
  picker in the bar, the rarer rules under *More rules*, the exports under Properties;
  Identify's results first; Documents' reader at full height; BIOS systems by the app's
  names; Export layouts without the breadcrumb; About's build details folded; a dialog
  can have one button (`cancelLabel={null}`).
- **Left for later**: PC games on the shared game page (it means the game page reading
  `game.json` in `PC/<Title> (<Year>)/` and its Unpack/program parts; its own PR before
  Phase 7 or with it), Home's *Next* strip, the Theme control shown twice (menu and
  Settings), and the Library's Players filter as a list.

### Collections rework design (written 2026-10-18)

Jerred keeps subsets of his full sets as separate folders (a best-of, a handheld's
card, a friend's list) and wants them as collections instead: point at the folders,
capture the subset as a DAT, then review it and export part or all of it from the
full set, so only the full set needs keeping.

- **D49 (agreed 2026-10-18).** The Audit page's DATs are the complete lists (No-Intro,
  Redump, the full sets). A collection is always a subset of them, made one of three
  ways: **by rules** (as now), **from folders**, or **from a DAT**. A DAT read for a
  collection never becomes one of a system's audit DATs.
- **From folders.** *New collection › From folders…* takes one folder, a folder of
  folders, or several, from anywhere (they're not added as folders with games). Their
  files are read as a scan reads them (ZIP CRCs trusted as D16 says, 7z, RAR and disc
  images read in full) and matched by hash (SHA1, else CRC32 and size, with and without
  headers) against each system's audit DAT. The review lists, by system, the games
  matched (and whether the full set has them), and the files no audit DAT knows; the
  collection takes the matched games.
- **From a DAT.** *New collection › From a DAT…* reads a DAT file in place and matches
  its games' ROMs the same way, by hash, so a list made by another tool, with other
  names, still finds the games.
- **Kept as DATs.** A collection made from folders or a DAT is kept as DATs of its own,
  one per system, in `_library/collections/<id>/`, written with the full set's names
  and hashes (so RomVault, igir and clrmamepro can use them). Its rules point at them
  (`from_files`) with every game in the DAT, no 1G1R and nothing left out by flags, so
  it's worked out as any collection is and matched by name or hash, surviving renamed
  files and newer DATs. `made_from` remembers the folders or the DAT, for *Read them
  again…*, which replaces its DATs after the same review. Its rules can still be edited
  (say, 1G1R on top), and games added or left out by hand as before.
- **D50 (proposed, built).** **The Collections page**: the collections listed down the left
  (name, games, how it was made); the one chosen opens on its games, with a tab per
  system, its size against its limit in one line above, and tabs for *Rules* (or *Made
  from*, for a folder or DAT one) and *Exports*. Games and whole systems can be ticked:
  *Export these…* exports only those (`only` in the export, remembered so *Update*
  keeps to the same part), *Export…* the whole collection.
- **D51 (proposed; dropped 2026-10-18, Jerred: not needed, he deletes the old folders himself once the full set holds their games).** After a collection from folders is saved, *Set aside these
  folders…* offers to set the subset folders aside (D20's hidden folder on the same
  drive, a change with a preview and Undo), but only the files whose games the full set
  has: anything the full set lacks is kept where it is. Nothing is touched unless asked
  (D5).

**As built (2026-10-18).** D49 and D50 as written, with these choices:
- The page's tabs are *Games*, *Rules* (*Made from* with the rules under it, for a
  collection from folders or a DAT), *Size* (the properties and size limit, which needed a
  home of their own) and *Exports*. A ticked system is `<system>/*` in `only`; its games
  can't be unticked one by one while it is.
- A file whose ROM two games of the DAT share counts as the game it makes whole (a game
  and a set that also holds it: the game).
- Duplicating a collection from folders or a DAT copies its DATs, so each has its own.
- **D51 was dropped** (Jerred, 2026-10-18) and isn't built. The old folders can be
  deleted by hand once the collection is made (nothing in it depends on them).

- **D52 (asked by Jerred 2026-10-18, for a later phase).** A collection keeps a history:
  every game added or left out (by hand, by a rules change, or by *Read them again…*) is
  recorded with when and how, shown in a history view on the collection, and each step
  can be undone there. Built in Phase 8b as D67 ("Phase 8b notes").
- **D53 (agreed 2026-10-18).** *Bring these into the library*: in the review of a
  collection from folders, the games the full set doesn't have at all can be brought in:
  each one's files copied (hardlinked on the same drive) into the first folder with games
  for its system, named as the audit DAT names the game (an archive or image as the game,
  a loose file of a one-ROM game as its ROM; a multi-file game keeps its files' names
  for Organise to put right), byte-identical copies once, as a change of kind `bring` with
  the preview and Undo, scanned after it runs. The folders read are never changed (D5).
  Games the full set has in part aren't brought in (Organise or Add a game… does that).
  Built (v0.5.8); UI pass 3 moved it to the Import page (D71, D73), and since D77 each
  system's games go to that system's own folder ("Import by system").
- **D54 (agreed 2026-10-18).** A **Help** page in the menu: the words the app uses, each
  defined once (`TERMS` in `web/ui/help.js`), and what each page in the menu is for with
  its main actions (`PAGES`), with a search; the page test checks every page in the menu
  has a section. Pages use the words as Help defines them: a game's folder under
  `_library/games/` is *its library folder* (not to be confused with a PC game's folder),
  a system's audit DAT is its *full set*.

### PC game page design (written 2026-10-18)

Until now a PC game had a page of its own on the PC games page (its panel), with one
source's details and artwork kept in `PC/<Title> (<Year>)/game.json`, while every other
game's page kept each source's answer, a picture from each and the user's edits (D46–D48).
PC games get the same.

- **D55 (agreed 2026-10-17 by Jerred).** A PC game's page is the shared game page. What
  only a PC game has stays in `PC/<Title> (<Year>)/game.json` (D6): its id, source,
  version, ids, what's installed and its program. Its details, pictures and documents are
  kept as any game's, in `_library/games/pc/<id>/`:
  - `game.json`: the details made from the sources' answers, `edited`, `pinned`, `chosen`,
    `pinned_media`, `scraped` and `identified_by`;
  - `sources/<source>.json`: each source's answer, kept by Identify… and the import as a
    scrape keeps one (D46);
  - `media/<kind>.<ext>`: the chosen picture, with the other sources' at
    `media/<kind>/<source>.<ext>` (D47). PC games have banners, heroes, logos and icons as
    well as boxes;
  - `docs/`: documents attached to it.
- **Its title and year are its own.** They name its folder, so they aren't made from the
  sources. They're changed by hand (the folder is renamed with them), or by Identify… when
  a match is picked, or from a source's answer with *Use this*.
- **Its details** are made as a DAT game's are (D46). The order is the game's own pick
  (`pinned`), then the source it was last identified by, then the library's *Details from*
  and scrape order, then Steam's and GOG's. A hand edit wins over all of them.
- **The page** has everything a DAT game's page has: details with what the other sources
  said, the pictures from each source, edits, documents (attached or dropped on the page),
  Delete metadata… and Show its folder. *Identify…* takes the place of *Scrape…*, and a
  **PC game** section has what only a PC game has: the program and its arguments, Unpack
  (and removing what was unpacked), its folder and its files. *Add to Steam* stays on the
  PC games page: it writes all the PC games at once, not one game.
- **The PC games page** keeps its list. A game's panel there has its picture, title and
  facts, *Open its page*, *Identify…* and the same PC parts as the page.
- **Libraries from before** keep working. A PC game's details from before are read where
  they were, as the answer of the source that gave them (`identified_by`, else its one id's
  source, else "an earlier scrape"), until its metadata is first changed. They're then
  brought across: kept as that source's answer, its pictures marked as that source's,
  `_library/games/pc/<id>/game.json` written, and only then the details taken out of
  `PC/…/game.json`. Run again it finds nothing to do, and an interrupted run carries on
  from what was written. Nothing is lost.

- **D56 (requirement from Jerred, not scheduled).** Updates, DLC and patches for 3DS, Xbox
  and PS4 games, often in the same file format as the game itself, must be tied to the
  game they belong to. Not built. The DATs list them as separate games, so the audit
  already knows them by hash. What's missing is a "belongs to" link in the library: the
  update's or DLC's own entry pointing at its base game, kept beside the game's metadata
  (as a PC game's `patches/` already are, D6), so the game's page lists them, the Library
  shows the base game once, and exports and collections carry them with it. Scheduled in
  Phase 8 and built in Phase 8b as D66 ("Phase 8b notes").

**As built (2026-10-18).** D55 as written, with these choices:
- `pcmeta.rs` holds the layout; sources.rs, meta.rs, browse.rs and docs.rs treat a folder
  under `_library/games/pc/` as a PC game's. Delete metadata…, Undo, *Details from* and
  Settings › Metadata's sizes work on PC games as on any game.
- Bringing a game across is lazy, on its first change (an edit, *Use this*, a picture
  chosen, Identify…, Delete metadata…). Until then the Library, the page and the command
  line read the old place. Keys a game from before has that aren't details (`gog_slug`)
  stay in `PC/…/game.json`.
- An import plans its files and `PC/…/game.json` as before (a change with Undo). After it
  runs, each game's details are kept as the answer of the source that found it, and that
  source's artwork is fetched as its pictures, so Undo is unchanged.
- Identify… on a game already identified adds the new source's answer beside the others
  and makes its pictures the ones shown, unless the user picked another source's on the
  page. The game's `ids` in `PC/…/game.json` gain the new source's id, and the page shows
  them with the ids the answers have.
- The panel on the PC games page no longer edits the title or shows the description:
  those are on the page.

### Phase 7 design (written 2026-10-18; Jerred asked for it to be built and released for testing)

Arcade sets are kept the way MAME and FinalBurn Neo want them: a ZIP per machine named by
its short name (`sf2.zip`), clones leaning on their parents, BIOS sets (`neogeo.zip`) and
devices (`namco51.zip`) as ZIPs of their own, hard disks and laserdiscs as CHDs in a
folder named for the machine (`sfiii/sfiii.chd`), and sounds in `samples/<name>.zip`.
How a clone's ROMs are spread between its ZIP and its parent's is the *set type*.

- **D57 Two arcade systems.** `mame` (Arcade (MAME)) and `fbneo` (Arcade (FinalBurn
  Neo)), ES-DE's ids, of a new kind of system, *arcade* (Settings › Systems offers it for
  a system added by hand, such as Naomi or a MAME 2003-Plus set kept apart). A DAT whose
  name starts with "MAME" is for `mame`, one naming FinalBurn Neo or FBNeo is for
  `fbneo`. MAME's own `mame.exe -listxml` output, which has no header, is read as a DAT
  named "MAME" at its build's version; it's big (300 MB), so it's streamed.
- **D58 What an arcade DAT says is kept.** Besides each machine's ROMs: its BIOS set and
  parent (`romof`), samples (`sampleof`, `<sample>`), the devices it uses
  (`<device_ref>`), whether it's a BIOS set or a device, year and manufacturer; each ROM's
  `merge` (the parent's or BIOS's ROM it is); and its disks (`<disk name sha1>`), kept as
  files of the machine named `<disk>.chd`. A CHD's own SHA1 (in its header, as MAME's DATs
  give it) is read when it's scanned, and that's what a disk matches. A hard disk or
  laserdisc CHD isn't unpacked and hashed in full (it's only ever matched by that SHA1);
  CD and GD-ROM CHDs are read as before. BIOS sets and devices aren't games: they aren't
  counted with the system's games, listed in the Library or put in collections; the Audit
  page lists them on a tab of their own. Arcade games are titled by the DAT's description
  ("Street Fighter II: The World Warrior"), the short name staying the file's.
- **D59 Playable.** A machine is *had* when every ROM and disk it lists is somewhere in
  the folders with games, matched by hash wherever it is (so a merged, split or non-merged
  set audits the same). For arcade systems the audit adds **playable**: had, and so are its
  BIOS set, every device it uses that has ROMs, and (for a DAT that leaves a clone's
  parent's ROMs out, a split DAT) its parent. A game's line says what it's waiting for
  ("needs neogeo", "needs the disk sfiii"). Samples are listed when missing but don't stop
  a game being playable (MAME runs without its sounds).
- **D60 Set types.** Each arcade system has a set type, in Settings › Systems and the
  Organise dialog: **non-merged** (the default: every game's ZIP holds every ROM it needs
  apart from its BIOS's, so any ZIP can be copied alone), **split** (a clone's ZIP holds
  only the ROMs its parent doesn't) or **merged** (a parent's ZIP holds its clones' ROMs
  too; a clone ROM whose name the parent uses for other contents goes in as
  `<clone>/<rom>`, as clrmamepro does). BIOS sets and devices are always ZIPs of their own,
  and a disk always goes in the folder of the machine whose disk it is (a clone that uses
  its parent's disk finds it there). Organise builds ZIPs as the set type says, and a file
  in the wrong ZIP is *misnamed*, so Rename these… puts a set right after its type changes.
- **D61 Arcade exports are non-merged**, whatever the library's set type: each game is its
  own ZIP (what frontends and handhelds list), with its BIOS set's and devices' ZIPs and its
  disk folders beside it. BIOS sets and devices get no gamelist entry (ES-DE hides MAME's
  BIOS and device ZIPs by name). Samples aren't exported yet: their folder is the
  emulator's and isn't in the layouts.

Not in Phase 7: MAME's software lists, reading MAME's `-listxml` by running MAME (the file
is imported like any DAT), CHDs made or checked by chdman for arcade, catver/history files.

### Phase 7 notes (built 2026-10-18)

D57–D61 as written, with these choices:
- **Where it lives.** `arcade.rs`: `Machines` (a DAT's machines from the index), `origin`
  (the machine a shared ROM comes from: its `merge`, else the same name and contents in the
  set it leans on), `places` (where each ROM goes in a set type), `needs` (the BIOS sets,
  devices and, for a split DAT's clone, the parent a machine needs), `containers_for` (what
  an export or Organise of some games takes) and `needs` per DAT (playable, missing samples),
  kept with the summaries. The index's step "arcade sets" adds the columns and `dat_links`,
  `dat_places` (the set type's places, for the audit's *named*) and `files.chd_sha1`.
- **The audit.** `audit::GAMES` leaves BIOS sets and devices out (every page that lists
  games uses it); `audit::MACHINES` has them. An arcade DAT's *named* is
  `MATCHED_ARCADE`, against `dat_places`. A clone is taken as split-listed (needing its
  parent) when none of its ROMs comes from its parent.
- **Disks.** A scan reads each CHD's header once (`scan::chd_headers`; "" for one that
  can't be read); hard disk and laserdisc CHDs aren't unpacked. Disks match with method
  `chd`. The tracks of a CHD matched as a disk aren't *unknown*.
- **Organise.** For an arcade DAT, `organise::arcade_rows` turns the machines into the
  ZIPs the set type keeps (a member of a merged parent's ZIP that's a clone's is
  *optional*: the ZIP is complete without it), and `place_disks` puts each disk at
  `<machine>/<disk>.chd`. Machines are always named by their short names (no naming
  template, no folder per game). Set aside duplicates uses the same rows, so a ROM a
  non-merged clone shares with its parent is kept in both ZIPs. Convert doesn't apply.
- **Exports** pass `set_type: non-merged` to Organise with the collection's games, so the
  BIOS sets and devices they need are copied too and left out of the gamelist. muOS's
  layout lists `mame` and `fbneo` (catalogue "Arcade", unconfirmed); the others use the ids.
- **Starter DATs** skip the arcade systems (libretro's No-Intro and Redump copies have no
  arcade DATs). ScreenScraper's, IGDB's and TheGamesDB's arcade ids are set for scraping.
- **Not built:** samples aren't exported, merged sets' clones get no gamelist entry of their
  own (exports are non-merged, so it doesn't arise), and an arcade machine's CHD isn't
  checked with chdman.

### Phase 8 design (written 2026-10-18; Jerred agreed the scope)

Jerred's scope for Phase 8 (2026-10-18): the RetroArch, Pegasus, Playnite and LaunchBox
exports, the magazine collection (D31), updates, DLC and patches tied to their base game
(D56), a collection's history with Undo (D52), and the three items the UI pass left for
later. MobyGames is dropped. A UI review by Jerred and Claude follows Phase 8: Claude
writes its findings, Jerred adds his, and one UI pass is built from them.

It's built and released in three parts, each tested before the next: **8a** the exports
and the UI items, **8b** updates and DLC with the collection history, **8c** magazines.

**8a: more export targets**

Each is a built-in export layout (D36–D38), so it can be copied and changed like the
others; the layout format gains the kinds of list each one writes.

- **D62 RetroArch playlists.** Layout `retroarch`: each system's games in
  `{roms}/{folder}/`, a playlist per system in `{retroarch}/playlists/<playlist>.lpl`
  (RetroArch's JSON playlist), and pictures in
  `{retroarch}/thumbnails/<playlist>/Named_Boxarts|Named_Snaps|Named_Titles/<label>.png`.
  `<playlist>` is the No-Intro or Redump name of the system, the name RetroArch's own
  database and thumbnail server use ("Nintendo - Game Boy"), so its scanner, its
  achievements and its own thumbnail downloads agree. `<label>` is the game's title with
  `&*/:`<>?\|` written as `_`, the way RetroArch looks for it; pictures are written as PNG.
  Each entry's core is left to RetroArch (`DETECT`). The paths in a playlist are where the
  games will be when RetroArch runs: a layout place, *Where the games are on the device*,
  set for an SD card (`/storage/roms` and so on), else where they're written. Entries in a
  playlist that Game Repo didn't write are kept.
- **D63 Pegasus.** Layout `pegasus`: each system's games in `{roms}/{folder}/` with a
  `metadata.pegasus.txt` beside them: the collection (its name, short name and file
  extensions) and an entry per game (title, file, developer, publisher, genre, players,
  release, rating, description). Pictures go where Pegasus finds them by itself,
  `media/<file name without its extension>/boxFront.<ext>` (and `boxBack`, `screenshot`,
  `titlescreen`, `logo`, `marquee`, `background`, `video`). Entries in the file for games
  Game Repo didn't export are kept.
- **D64 LaunchBox.** Layout `launchbox`, written into LaunchBox's own folder: games in
  `Games/<platform>/`, each platform's `Data/Platforms/<platform>.xml` with a `<Game>` per
  game (title, its file as a path from LaunchBox's folder, details, its manual), the
  platform added to `Data/Platforms.xml` when it's new, and pictures in LaunchBox's names
  (`Images/<platform>/Box - Front/<title>-01.<ext>`, `Box - Back`, `Screenshot - Gameplay`,
  `Screenshot - Game Title`, `Clear Logo`, `Fanart - Background`, `Box - 3D`), videos in
  `Videos/<platform>/` and manuals in `Manuals/<platform>/`. `<platform>` is LaunchBox's
  name for the system ("Nintendo Game Boy"), listed in the layout's `systems`. As with
  Steam (D24), it's written only while LaunchBox and Big Box are closed, the XML files are
  backed up first, and a game already there (by its file) is updated, not added twice.
- **D65 Playnite** (asked of Jerred 2026-10-18). Playnite keeps its library in a database
  only Playnite writes, so no file Game Repo writes can add games to it with their details
  and pictures. The options are a folder laid out for Playnite's own *emulated games*
  scanner (Playnite then finds the games and fetches their details itself), a small
  Playnite add-on that reads a Game Repo export (a separate project, in C#), or leaving
  Playnite out. Proposed: the folder now (layout `playnite`, a folder per system named as
  Playnite's platforms are), the add-on later if wanted.
- **The UI items left from the UI pass.** Home gets a *Next* strip: the games that need a
  look, the systems not scraped yet, the BIOS files missing and the audits with misnamed
  files, each linking to its page, and nothing when there's nothing to do. The theme has
  one switch, in Settings › Appearance with the skin (the menu's theme button goes). The
  Library's Players filter becomes one drop-down.

**8b: updates and DLC, and a collection's history**

- **D66 Updates, DLC and patches** (D56, Jerred 2026-10-17). An *add-on* is a DAT game
  that's an update, DLC or patch: its DAT says so in its name (No-Intro's "(Updates and
  DLC)", or "Updates", "DLC", "Patches"), or its own name has an `(Update)`, `(vX.Y)` with
  update, `(DLC)` or `(Patch)` tag. Each add-on belongs to a base game: the game in the
  system's other DATs with the same title (the name without its tags) and region, then the
  same languages; one that matches none belongs to no game until it's linked by hand. Links
  made or broken by hand are kept in library.json `addons`. Add-ons are audited with their
  own DAT as now. They aren't games of their own anywhere else: the Library shows the base
  game once, with how many updates and DLC you have, and a filter for it; a game's page
  lists its add-ons with what you have, their versions, and *Link to another game…*; a
  collection takes its games' add-ons with them (a rule, *With their updates and DLC*, on
  by default); an export copies them beside their game (a layout's `addons` path, by
  default the games' own), without entries of their own in its list. A PC game's patches
  (`PC/<Title> (<Year>)/patches/`, D6) are shown under the same heading.
- **D67 A collection's history** (D52, Jerred 2026-10-18). Every change to what's in a
  collection is a step kept in `_library/collections/<id>.json` `history` (the newest
  500): when, how (by hand, a rules change, *Read them again…*, an undo), the games it
  added and left out, and, for a rules change, the rules before it; for *Read them
  again…*, the DATs before it are kept in `_library/collections/<id>/history/`. A
  **History** tab on the collection lists the steps, newest first, with what each did.
  *Undo* on a step puts it back: a step by hand puts back or leaves out the same games
  (any step can be undone, the newest first if they touch the same games), a rules change
  or *Read them again…* goes back to the rules or DATs before it, and only the newest of
  those can be undone. An undo is a step of its own, so it can be undone too.

**8c: magazines**

- **D68 Magazines** (D31, Jerred 2026-10-17). A **Magazines** page in the menu, kept like
  PC games: a folder per magazine, `Magazines/<Title>/`, with `magazine.json` (its
  publisher, the issues and each issue's number, date, file and cover) and its issues'
  files, PDF, CBZ or CBR, named `<Title> <number> (<YYYY-MM>).<ext>`.
  - **Import magazines…** reads a folder (or several), works out each file's magazine,
    number and date from its name ("Nintendo Power 012 (1990-05)", "EGM_045_Apr1993"),
    and shows them in a review list to put right, then copies them in (hardlinked on the
    same drive, D5), as an Organise change with Undo.
  - **Covers** are the first page: a CBZ's or CBR's first picture, or a PDF's first page
    when it's a scan (one picture to the page); otherwise none until a picture is dropped
    on the issue.
  - **Audited against a list**: a DAT whose games are issues (matched by hash, else by
    magazine and number), or a list of magazine, number and date as CSV, kept in
    `_library/dats/magazines/`; the page shows each magazine's issues you have and those
    missing, and the missing ones can be saved as a list.
  - **Read** in the document viewer: PDFs as now, CBZ and CBR a page at a time. Their text
    (PDFs that have it) is searched with the documents, and the Documents page can show
    only magazines.
  - **Exported** with the plain folder layout (a folder per magazine, its issues and
    covers), ticked as part of a collection's export.

Not in Phase 8: MobyGames (dropped by Jerred, 2026-10-18), DAT updates fetched by
themselves, and a Linux build (D3).

### Phase 8a notes (built 2026-10-18)

D62–D64 as written; D65 as Jerred chose it (2026-10-18, *Folder for its scanner*). The
choices made in the build:
- **The layout format** gains three kinds of list (`layout::GAMELISTS`: `es-xml`,
  `pegasus`, `retroarch`, `launchbox`), a place that's a path on the device
  (`"device": true`: not a folder here, nothing written under it, no *Choose…* in the
  Export dialog), and two tokens: `{playlist}` (the layout's name for a system when it
  gives one, else its No-Intro or Redump name) and `{label}` (the name the list gives the
  game, with the characters RetroArch or LaunchBox can't have in a picture's name as
  `_`). A picture's `tag` is its path from the games' folder for EmulationStation and
  Pegasus, from LaunchBox's folder (with `\`) for LaunchBox; a RetroArch layout has none.
- **Names.** The list names a game by its title or full name as the Export dialog's
  choice says; RetroArch starts with the full name, since libretro's thumbnails are named
  by it. The same name is the pictures' `{label}`, so the two always agree.
- **Merging.** Each writer keeps what isn't Game Repo's: RetroArch entries by path (a
  game's own entry keeps the core RetroArch learnt), Pegasus entries by `file:` (Game
  Repo's own written again; the file's collection header kept, or written when it's new),
  LaunchBox `<Game>`s by `ApplicationPath` (only the title, the details chosen and the
  tagged paths are set; a new one's `<ID>` is a GUID made from its platform and file, so
  it's the same each time). A list that changes is set aside first, so Undo puts it back.
- **LaunchBox** is refused while `LaunchBox.exe` or `BigBox.exe` runs (Windows'
  `tasklist`, as for Steam). Its platform names, Playnite's and the few RetroArch
  playlist names that differ from No-Intro's (in `retroarch.json`'s `systems`) follow
  their documentation as of October 2026 and are unconfirmed.
- **Home's Next strip** is `audit::home`'s `next`: misnamed and bad files per system, games
  needing a look (`meta_games.review`), systems you have games of with none scraped
  (arcade left out), and systems missing BIOS files (when a BIOS DAT is in). The theme
  button left the menu (Settings › Appearance has Theme beside Skin), and the Library's
  *at least* filters (Players, Rating) are one drop-down each.

### Phase 8b notes (built 2026-10-18)

D66 and D67 as written, with these choices made in the build:
- **Telling an add-on.** `addons.rs`: a DAT whose name has a group with "update", "DLC",
  "patch" or "add-on" in it holds add-ons; elsewhere a game is one by an `(Update)`,
  `(DLC)` or `(Patch)` tag. Its version is its `(vX.Y)` or `(Version X.Y)`. Its base game
  is in the system's audit DAT: the one with the same title and first group (No-Intro's
  region), else the only one with the same title. The DAT names' `(Digital)`, `(CDN)`,
  `(PSN)`, `(Content)`, `(Standard)`, `(Updates and DLC)`, `(Updates)`, `(DLC)` and
  `(Patches)` are suffixes, so those DATs find their system by themselves, and the audit
  DAT chosen by itself is never one of add-ons.
- **New systems,** audited only: Nintendo 3DS, Wii U and Switch, PlayStation 3, 4 and
  Vita, and Xbox 360 (`n3ds`, `wiiu`, `switch`, `ps3`, `ps4`, `psvita`, `xbox360`, as
  ES-DE names them), with their ScreenScraper, IGDB and TheGamesDB ids and LaunchBox and
  Playnite names (unconfirmed, as 8a's).
- **The Library** leaves an audit DAT's own add-ons out of its rows, and keeps on each
  base game how many add-ons you have (the index's `library.addons`, a migration);
  *Has › Updates and DLC* finds them. The stamp a system's rows are rebuilt by includes
  every current DAT of it and the hand links.
- **Links by hand** are library.json `addons: {system: {add-on: base game, or null for
  none}}`; *Undo the link* takes one out. The game page's *Updates and DLC* lists the
  add-ons with their kind, version and whether you have them, and those belonging to no
  game to link there; a PC game's patches and DLC (its `patches/` and `dlc/` files) are
  shown under the same heading.
- **Collections and exports.** `Rules.with_addons` (on by default, also for collections
  saved before) adds the add-ons you have (all or some of their files) to each system as
  `System.addons`; they count in its size and against its size limit. An export copies
  them from their own DAT into the layout's `games.addons` folder (a path ending in
  `/{file}`), by default the games' own, named as their DAT names them, with no entries in
  the list it writes. *Export these…* takes the add-ons of the games ticked.
- **History** (`history.rs`): a step is kept by every `collection_save` (by hand, or
  `rules` when the rules or what it's made from changed) and `collection_from` (`made`,
  or `read` for *Read them again…*). A change of rules or a reading again lists its first
  50 games each way (it's undone by what was before, not by its games); its `before` has
  the rules and what it was made from, and a reading again's DATs are copied to
  `_library/collections/<id>/history/<step>/` (they go with the step when it's one of the
  oldest past 500). Undoing it puts back only the rules (and DATs): games added or left
  out by hand since are steps of their own. Undoing a step by hand leaves out the games it
  added (off the added list, else on the left-out list) and puts back those it left out
  (off the left-out list, and added by hand only if the rules don't bring them in).
  Making a collection is a step that can't be undone.

### Phase 8c notes (built 2026-10-18)

D68 as written, with these choices made in the build:
- **Names.** `magazines.rs` reads a file's magazine, number and date from its name: a
  number (leading zeros dropped), then a date as `(YYYY-MM)`, `YYYY-MM` or a month and
  year (`Apr1993`). A name that's only a number or a date takes its folder's name as the
  magazine. An issue is named `<Title> <number to three places> (<YYYY-MM>).<ext>`; a file
  the library already has (by SHA1) is shown as *Have it* and left unticked.
- **Bringing in** is a plan of kind `magazines` (copy, or hardlink on the same drive),
  writing each magazine's `magazine.json` last, so Undo takes the files and the list
  back together. The issues are indexed as documents of the kind Magazine from their
  `magazine.json` (only the files it lists), so they're searched and filtered with the
  documents; their title and kind are changed on the Magazines page only.
- **Covers and pages.** A cover is a picture chosen for it (copied into the magazine's
  `covers/`), else a CBZ's or CBR's first picture in natural order, else a PDF's first
  page when that page is a single JPEG, else a plain cover drawn with its magazine's name,
  number and date; covers and comic pages are cached under the data
  folder's `thumbs/<library id>/` (never in the library) and served as `~cover/<path>`
  and `~page/<n>/<path>`. Dropping a picture on the page sets the focused issue's cover;
  dropping anything else imports it.
- **Lists** are kept in `_library/dats/magazines/`: a CSV of magazine, number, date (a
  header line is passed over), or a DAT whose games are issues, matched by SHA1, else by
  magazine and number. *Save as a list* writes the missing to
  `_library/reports/Magazines missing … .csv` in the same CSV shape.
- **Exports** carry them with a tick in *What it carries* (any layout): each magazine's
  issues and covers in `Magazines/<Title>/` in the export's folder.

### UI pass 2 notes (built 2026-10-18, D69)

Jerred's list after Phase 8, built as one PR before the joint UI review.

- **Ribbons, from Model Library's and SCAD Workshop's subcategory views** (`web/ui/panels.js`).
  `useSidePanel(spec)` puts a page's own list in a ribbon to the right of the menu
  (`SidePanel`, drawn by `sidebar.js` after the nav): a header with its name, count and
  actions, the rows (`a.panel-link` with a second line), an optional *All* entry above
  them; resizable from 180 to 420 px (`sidePanelWidth`), folded to a 34 px strip with its
  name written down it (`[`, `sidePanel`), folded by default at 1250 px or narrower. In a
  window under 900 px it isn't drawn: `DrawerRows` puts the rows under the page's link in
  the menu's drawer. Pages give their list an address: `#/<page>/<arg>` (`useRouteArg`,
  `argHash`), so Back returns to the one you were on. `Inspector` is the right-hand
  ribbon: a pane with tabs and a rail of tab names down the window's edge (`]` folds it;
  `inspector`, `inspectorTab`, `inspectorWidth` per page in `gr-ui`); under 900 px the
  pane covers the page when opened.
- **Collections:** the collections in the ribbon (`#/collections/<id>`, New collection… in
  its header and the ⋯ menu); the games drawn by the Library's own `Games` (grid, list,
  table; sorted by title, system, year or size) from a `browse_query` with `collection` and
  `all`, picked with the Library's `useSelection` (click, Ctrl, Shift, Ctrl+A) for
  *Export these…* and *Leave out* in the selection bar and the right-click menu. "Made
  from" is now *Source* (the tab and its section); *Export…* is only in the top bar; the
  system tabs became a drop-down; *Left out by hand* stays a list with Put back.
- **A game's page:** a *Media* tab in the `Inspector`: every kind of picture it has, each
  source's at 256 px, a click shows it larger at the top of the pane, *Use this*
  (`meta_choose`). The gallery on the page is unchanged.
- **Left-justified:** `.pages > *` keeps its max width without auto margins.
- **Audit:** the systems in the ribbon with *All systems* above them (`#/audit/all`): a
  table of every system's counts from `audit_overview` (`audit::overview`, each system's
  primary DAT's summary), each count a link to that system at that list. A system's games
  lists take a region (`audit_list`'s `region`, matched within `gs.regions`; the first
  page returns `regions` with their counts) and a sort (`name`, `region`, `year`), and a
  List or Table look (`gr-audit-look`); the platform drop-down is gone (the ribbon is the
  picker; Home's bars link to `#/audit/<system>`).
- **BIOS files:** every system the BIOS DATs list, those with games first (as before),
  with the search; the three-way picker is gone.
- **Magazines** moved their list into the ribbon too (`#/magazines/<title>`), for one way
  of doing it across the app.

**Claude's findings for the joint review** (not built; for Jerred to add to):

1. *Right-hand panels differ by page.* Library (`lib-panel`), Metadata, Documents, PC games
   and Export layouts each have a fixed panel on the right that doesn't fold or resize; the
   game page now has the shared `Inspector`. Moving them all onto it would give every page
   the same rail, `]` and widths.
2. *System pickers.* Metadata (`#meta-platform`) and Documents (`#docs-platform`) choose
   a system from a drop-down at the top; Audit and Collections now use the ribbon. Metadata
   in particular reads like Audit (a system's DAT games) and could share its ribbon.
3. *Library filters* (`lib-filters`) are a fixed column on the left of the page; they
   could be the Library's ribbon (folding with `[`), which would leave one left-hand
   pattern.
4. *Settings' section index* is its own column; it could be the ribbon as well.
5. *Two meanings of "Source".* Help's glossary uses *Source* for ScreenScraper, IGDB and
   the rest; Collections now uses it for what a collection was made from. Help explains
   both, but a reader may trip on it.
6. *The PC games page and the Library overlap:* PC games are in the Library with their own
   page already; the PC games page could become a filtered Library view with its
   Import/Unpack actions, leaving one list of games.
7. *Grid of tabs on a collection* (Games, Source, Size, Exported to, History) plus the
   In it / Left out tabs: two rows of tabs; Left out could be a filter of the games view.

### Scraping threads and the game picture (built 2026-10-18, D70)

Jerred asked during the joint review: his ScreenScraper account allows several threads, and
the game page's picture changed size with every picture picked from the strip under it.

- **ScreenScraper threads** (`scrapers.rs`, `scrape.rs`). Before a scrape that asks
  ScreenScraper, `ss_configure` reads the account's `ssuserInfos.php` (never from the
  cache): `maxthreads` (at most 16, fewer if Settings › Sources' *Threads at once* says
  so) and `maxrequestspermin`, which sets the gap between requests' starts (a tenth under
  the rate; else 1.2 s shared between the threads). `Slot` holds the count of requests in
  flight at the threads. The scrape asks that many games at once (scoped threads, each
  game's sources in their order), then keeps their answers one after another on the main
  thread, so progress, the review list and the index are written as before; a game's
  pictures are fetched that many at a time (`fetch_all`). 429s and "maximum threads"
  still back off. *Check* says how many threads the account allows.
- **The game's picture is one square box** (`.game-hero`): `aspect-ratio: 1` at the
  column's width (at most 520 px), the picture or video scaled inside it with
  `object-fit: contain`, so picking another doesn't move the page.

### UI pass 3 design (written 2026-10-18, D71–D76)

Jerred's review of v0.5.15 (2026-10-18), with his notes as examples for a review of every
page: the menu has overlapping pages whose purpose isn't clear; BIOS files, Metadata and
Documents could sit under Audit; it's unclear how games come into the library at all
(Collections can bring games in, the library itself can't); importing, organising,
auditing and collecting should each be clearly one thing. The Audit ribbon should put each
system's count beside its name; the Audit list view goes (the table says more in less
room); a game should open from the Audit. BIOS files says *Gather*, which should be
*Import*, and the wording across the app wants a review. Collections need groups: one
collection over several systems, each with groups of its own (translations, language
packs) kept apart. Later: OCR for magazines, and tagging an issue with the games it
covers.

**D71: five verbs, one place each.** Every action is one of these, and is reached from
the page of its verb (other pages link there, they don't repeat it):

| Verb | What it does | Where |
|---|---|---|
| **Import** | Brings files into the library: games for any system (PC games too), DATs, BIOS files, documents, magazines. | The new Import page (and drag and drop anywhere). |
| **Audit** | Checks what the library holds against the DATs: games, BIOS files, metadata and pictures, documents. Changes nothing. | Audit, with BIOS files, Metadata and Documents under it. |
| **Organise** | Changes files the library already holds: names, folders, formats, duplicates set aside. Always a preview, then Run, with Undo. | Started from Audit (a system) or the Library (picked games). |
| **Collect** | Picks games for an export. Never copies files into the library. | Collections. |
| **Export** | Writes a collection to a device, a frontend or a folder. | Collections (Export…) and Export layouts. |

So Collections' *Bring in the games the full set lacks* (D53) moves to Import (*Games
from a folder*, below), and Settings keeps only settings: *Folders with games* and *DAT
files* move to Import, and *History* becomes its own page.

**D72: the menu, nested.** Children are indented under their parent and always shown
(visual nesting, no expanding):

    Home
    Library
      PC games
      Magazines
    Import
    Audit
      BIOS files
      Metadata
      Documents
    Collections
    Export layouts
    History
    ─────
    Settings · Help · About

**D73: the Import page.** One card per kind of thing, each saying in a line where the
files end up, with its one button. A drop on any page opens Import with what was dropped
(as PC games' and Documents' drops do now).

- *Games already in a folder*: add the folder to *Folders with games*; its files stay
  where they are, are scanned and audited. (Today's Settings section, with its list.)
- *Games to copy into the library*: choose files or a folder; they're matched against the
  DATs and copied (hardlinked on the same drive) or moved into a folder with games, named
  as their DAT says; files no DAT knows can still be added to a system (today's *Add a
  game…* and D53's bring-in, as one dialog).
- *PC games*: installers, archives and game folders (today's PC games import).
- *DATs*: Import DATs…, Get starter DATs…, Make a DAT from a set…, with the list of DATs
  from Settings.
- *BIOS files*: Import BIOS files… (today's *Gather*) and Import a BIOS DAT….
- *Documents* and *Magazines*: their imports.
- *Recent imports*: the last few from History, each with Undo.

The Library page's empty state and its ⋯ menu get *Import…*; Home's *Import DATs…* and
*Add a folder…* open the Import page's cards.

**D74: Audit.** The Systems ribbon's rows are one line, the count right-aligned. The list
view goes: every tab is a table (Have, Partly there, Missing: name, region, year, flags,
ROMs; Named right, Misnamed, Bad dumps, Unknown, Duplicates: file, folder, its DAT game,
what it should be called). A row's double-click (or Enter) opens the game's page, a missing
game's too (its DAT entry, *not in the library*); *Convert…* moves from each row to the
right-click menu and the selection bar. The tabs are in two labelled groups, *Games* and
*Files*. BIOS files and Metadata use the same Systems ribbon (Metadata loses its system
drop-down).

**D75: collection groups.** A collection holds groups, each with a name and its own way of
choosing games (rules, a DAT, or folders, as a collection does now); one group can span
systems, and a system can have several groups (*SNES*, *SNES translations*, *SNES
language packs*). The page shows each group as a section of the games view, the ribbon
shows a collection's groups under it, and *Add a group…* is in the collection's menu. An
existing collection becomes one group, so nothing changes until a group is added. Where a
group's games go (Jerred, 2026-10-18): every export puts all of a system's games in its
system folder, and each group becomes a custom collection or playlist where the frontend has
them (ES-DE, Pegasus, RetroArch, LaunchBox, Playnite). An ES-DE export (the ES-DE layouts and
the devices that use ES-DE) asks which, at export: *collections only* (that default),
*subfolders* (the first group at the system's folder, each other group in a subfolder,
`roms/snes/Translations/`, which ES-DE shows as a folder) or *separate systems* (each group
its own system folder, `roms/snes-translations/`, set up in ES-DE as a system of its own).
Layouts get a `{group}` token for a layout of your own.

**D76: wording.** One word per idea across the app, from a review of every label: see "UI
pass 3 notes" for the table. *Import* for everything that brings files in (not *gather*,
*bring in*, *add*); *Systems* everywhere (Home says *Platforms*); jargon such as 1G1R and
fixdat gets a plain label with the term in its tooltip.

**Claude's findings from the review** (built in the same pass unless marked):

1. The game page shows its pictures twice (the strip under the picture and the Media
   ribbon): the ribbon starts folded.
2. The Library's right-hand panel only shows a hint until a game is picked: it starts
   folded and opens on a pick.
3. Metadata repeats the game page in a side panel: it becomes a table per system (game,
   details, pictures, documents, source) opening the game page.
4. Toasts with Undo stay long and cover the bottom right of the page (seen on most
   pages): shorter, and History holds the Undo after.
5. Dialogs explain at length (Organise, bring-in, PC import): one line, the rest behind
   *More about this*.
6. A collection has two rows of tabs and a long hint: *In it* / *Left out* becomes a
   filter of the games view, the hint a tooltip.
7. The Audit's DAT drop-down repeats the title's DAT name: it moves into the ⋯ menu when
   a system has one DAT.
8. BIOS files' count reads awkwardly ("0 of 1 systems you have games for have every BIOS
   file") and its search box touches the first card.
9. Export layouts says *built in* on every row: two headed lists, *Built in* and *Yours*.
10. Magazines edits each issue's date in a field under its cover: the date is text, edited
    from the issue's menu.
11. PC games overlaps the Library (UI pass 2 finding 6): it stays a page under Library for
    now, for its imports and Steam.

**Later (roadmap):** magazine OCR (text read from scanned issues, so they're searchable as
documents are) and tagging an issue with the games it covers, linked from each game's page.
The OCR reads every format an issue can be in: a PDF's pages, and the images in a CBZ or
CBR (Jerred, 2026-10-18). Videos too (Jerred, 2026-10-20): walkthroughs, reviews and
retrospectives as a section of their own like magazines, each linked to the games it covers.

### UI pass 3 notes (built 2026-10-18)

Built as designed above, with these differences:

- The menu, the Import page (`web/ui/importpage.js`) and the History page
  (`web/ui/history.js`) are as D72–D73 say. *Games to copy into the library* reads folders
  with `collection_read_folders` and copies with D53's `collection_bring_plan`; files no DAT
  knows can go on to *Add a game…*. Settings keeps the index, Scanning (ZIPs and the
  watcher), Systems, Naming, Preferred formats, Metadata, Sources and Tools.
- Finding 1: the Media ribbon starts folded; it still shows every picture by source.
- Finding 3 (Metadata) was built smaller: the systems moved into the ribbon (no drop-down);
  the game panel stays, since it's where a scrape's candidates are chosen.
- Finding 5 (shorter dialogs) is left for a later pass.
- Finding 10: an issue's date and a magazine's publisher are edit boxes (text until
  clicked), not a field under every cover.
- Wording (D76), the words that changed: *Gather* → *Import* (BIOS files); *Bring in* →
  *Import* (games); *Platform* → *System*; *fixdat* → *Save a list of what's missing*
  (fixdat in the tooltip); *1G1R* → *One per title*; *Put it back* → *Undo*; *Take out* →
  *Leave out*; *Reveal* → *Show in folder*; *Make a DAT from a set* → *Make a DAT from a
  folder*; *Rename to DAT names* → *Rename to the DAT's names*. Help's terms and pages
  are rewritten in the menu's order.

### Collection groups notes (built 2026-10-18, D75)

Built as D75 says, with these choices:

- **The data.** A collection keeps its own fields as its first group and gains `groups`
  (each `{ id, name, rules, added, removed, made_from }`) and `group_name` (the first
  group's name, when it isn't the collection's). An app from before reads a collection of
  groups as its first group; a collection from before is one group. Group names are unique
  within a collection; ids are made by `save`. `Rules.dats` names DATs a group takes its
  games from in place of each system's audit DAT (the Library gets rows for their games
  the audit DAT lacks by name). Groups are worked out in order and a game (by key) is in
  the first that picks it, or that left it out by hand, unless a later group has it added
  by hand. A group from folders or a DAT keeps its own DATs in
  `_library/collections/<id>/<group>/`.
- **Hand edits** go through `collection_hand` (`leave_out`, `put_back`, `add` with a
  `group`): left out goes on the list of the group that has the game, put back comes off
  every group's, and a game put back that no rule brings in is added by hand to its group.
  Each is a step of the history (D67), as is adding, changing or removing a group; Undo
  of a group's change keeps the hand edits made since.
- **Save as DATs** writes `<Collection> - <System>.dat` for the first group and
  `<Collection> - <Group> - <System>.dat` for each other.
- **Export.** Every export puts all of a system's games in its system's folder (the
  default, *Collections only*). Each group is an ES-DE custom collection
  (`custom-<Collection> - <Group>.cfg`, with ES-DE's `%ROMPATH%` when the export is to
  ES-DE's ROMs folder, else the games' folder), a Pegasus collection (merged with the
  same name across systems' files) or a RetroArch playlist (`<Collection> - <Group>.lpl`).
  LaunchBox and Playnite get no group lists yet. The ES-DE layouts (`esde`, `esde-card`,
  now version 2) also offer *Subfolders* (their games' and pictures' paths have
  `{group}`: nothing for the first group, the group's name for the others) and *Separate
  systems* (`groups.system`, `{folder}-{group}`: `roms/snes-translations/`, its gamelist
  and pictures beside, named "Super Nintendo - Translations"). For a separate system the
  export copies its system's `<system>` entry from ES-DE's `custom_systems/es_systems.xml`
  or the one ES-DE ships with into `custom_systems/es_systems.xml`, when the layout's new
  `es_de` says where ES-DE's settings are; when it can't, the preview's notes say what to
  set up. A layout's `placements` are what it can do; the Export dialog asks only when
  the collection has groups and the layout can do more than one, and the choice is kept
  in the export record, so Update places them the same way (`--groups` on the CLI's
  `export` and `export update`). Other built-in layouts are unchanged, so the device
  layouts offer collections only for now.
- **`{group}`** is a token of the system's (layouts.rs and the editor's token list): a
  token with an empty value is left out with its path part, so `{folder}/{group}/{file}`
  is `gb/Alpha.gb` for the first group.
- **The page.** Each group is a section of the games view (the Library's grouping, by
  `group: "collection"`), the ribbon lists the current collection's groups under it with
  their counts (`children` in `useSidePanel`; one jumps to its section), and the Rules tab
  has a section per group with Rename…, Remove… (removing the first makes the next the
  first), its source and its rules. *Add a group…* (the ⋯ menu and the Rules tab) uses the
  New collection dialog's three ways. *In it* / *Left out by hand* is a filter of the one
  games view (finding 6), with Put back in the selection bar, and the hint is a tooltip.
- **Wording** (collections only): *Add a folder…* → *Choose a folder…*; *Make it* → *Make
  the collection*; *Read* → *Read the DAT* or *Scan the folders*; the empty page's buttons
  say *New collection by rules…*, *from folders…*, *from a DAT…*; *your full sets* → *your
  audited games* or *the Audit page's DATs*; the export's *Run* → *Export*; *Rules on top*
  → *More rules*; *Forget* → *Forget…*. The tab stays *Source*.

### Import by system (built 2026-10-18, D77)

Jerred's findings on v0.5.16 (2026-10-18): Import put a whole read's games in the first
folder with games for every system (his Game Gear games went into a Game Boy Advance
collection's folder). He asked for an option to force a copy or a move, for each system's
games to go to its own folder, and for files no DAT knows to be asked about during the
import. He also asked for the system drop-down *One game…* has, with an *every system*
choice.

**D77: where an import's games go.** For each system, the best of `addgame::places`:
1. A folder with games for that system.
2. The folders its games are in already (from the index, the fullest first).
3. Its own folder inside a folder with games for every system. That folder must hold
   several systems' games, or none yet, or that system's games: a folder with games for
   every system that holds only another system's games is never offered.

   A folder with games for every system that is named for a system (`platforms::from_folder`,
   such as *SNES*) is that system's: only that system's games go in it.
4. When a system has none of these: a new folder named for the system (Jerred, v0.5.17:
   "the app should create a folder for each system based on the system name"). It goes
   beside the other systems' folders, in the folder most of them share (*Roms* for
   *Roms/GBA* and *Roms/GB/Champion Collection*), else in the library folder.

The review can always choose *Another folder…* instead. A folder outside
every folder with games becomes one for its system (`Plan::new_roots`), and Undo takes it
away again. The first step chooses *every system* or one system, and copy (hardlinked),
always copy, or move. Files no DAT knows are grouped by folder, each taken as the system
`platforms::guess` gives it, and they come in under their own names. *One game…* uses the
same places and gains *always copy*.

Jerred, later the same day: work that runs after Read or Preview must not be lost by
clicking away. The import's place is kept outside its window, so closing the window loses
nothing and *From a folder…* reopens it there. Checking the files is a job like reading
them. While either runs, the window shows its progress with *Stop* and *Run in the
background*. The status bar's entry for that job reopens the window, and a message with
*Show* says when the result is ready. The page behind the window doesn't close it
(`Dialog`'s `sticky`).

### Hardening after the v0.5.18 review (D78–D82)

Jerred, 2026-10-18: before any new features, make the last build air tight. The review of
v0.5.18 is in [review-2026-10-18.md](review-2026-10-18.md): how it was done, every finding
with its ID (C1–C5 Critical, H1–H24 High, M1–M47 Medium, then Low), where it is, how it
was checked and the fix. Work from that file; this section says what goes in which pull
request, in what order, and what has been done. Each pull request adds a test that
reproduces each finding it fixes (failing before the fix), a CHANGELOG line, and the
review file's ID in the commit body. Mark a batch done here, with its PR, when it merges.

Order (Jerred agreed the order, and asked for the docs batch to go with the first):

1. **Safe changes, Undo, and the docs** (C1–C4, H1, H2, H4, M1–M5, M7, the reserved names;
   the plan, CHANGELOG, About and Help text in "Docs (batch 1)"). Undo checks before it
   deletes and sets aside instead of removing; copy, link and extract refuse a target
   that appeared after the preview; a failed game keeps its archive; Undo never removes a
   folder with games or the folders above it, and re-indexes what it put back; an
   unreadable library.json or config.json is refused, never rewritten.
   **Status: done, merged with batch 2 (PR #32) and released in v0.5.19.** Fixed: C1–C4, H1, H2, H4, M1–M5,
   M7 and History's Undo label (from M43), and the "Docs (batch 1)" items. The Rust tests
   in `organise.rs`, `library.rs` and `addgame.rs` reproduce the findings and name them in
   a comment (M3's checks are in `names_are_safe_and_follow_the_template`); every finding
   has a test but M2 (an index error is hard to cause on purpose). As built:
   - Undo doesn't set a written file aside: it removes it only after checking every
     original it replaced can come back (C1), and only when its size and modified time
     are still what the journal recorded (`wrote`; a changed file is kept and reported
     with `kept`, C2). While a step can't be undone the change isn't logged as undone,
     so Undo can be tried again (`finished: false` in its answer).
   - The journal records each step's `wrote` stamp and the folders it `made`; Undo
     removes only those folders, never a folder with games or one above it (H1). Older
     journals keep the climb, with the same guard.
   - Undo stops before changing anything when a step's drive or folder with games isn't
     there, and removes a folder it added only when it's there and empty (M4). It scans
     the folders it touched when it's done (M7).
   - Each step is journalled before the index is updated; an index error becomes a
     warning (`warnings` in the run's answer) and those folders are scanned (M2). The
     steps of a failed game that are passed over are listed in `failed` with `skipped`.
   - library.json that can't be read opens the library read-only and `update_meta`
     refuses to write it; config.json that can't be read is kept as
     `config.unreadable.json` before a new one is written (H2,
     `config::try_read_json_object`).
   - Left for later batches as planned: C5 (batch 3), H3 (batch 6). M6, the space check
     counting hardlinks as free, was in no batch and goes in batch 2.
2. **CI permissions** (H24) on their own, then **scanning and audit** (H7–H11, M10–M14,
   the cue byte-order mark), and M6 (try one hardlink when planning).
   **Status: done, merged with batch 1 (PR #32) and released in v0.5.19.** The tests name
   the finding in a comment. As built:
   - H24: the workflow's token reads only (`permissions: contents: read`); checkouts keep
     no credentials; every action is pinned to a commit and every tool to a version
     (Tauri CLI, selenium, playwright, tauri-driver, msedgedriver-tool). The test jobs
     upload their reports as artifacts and a `reports` job (the only one that writes,
     with `release`) pushes the `desktop-ci-<os>` branches.
   - H7: a ZIP, 7z or RAR member that can have a copier header (`hash::may_have_header`)
     is read in full even in a quick scan, so its headerless hashes match. H8: a member
     that can't be read is passed over, not the whole archive; an archive that can't be
     opened for a reason other than its format (locked, unplugged) fails and is tried
     again on the next scan. The `zip` crate is built without AES or zstd but with
     deflate64, bzip2 and lzma. RAR scratch folders are one per call.
   - H9: hashing results are committed every 250 files, every second, and after half a
     second with none; an error recording one stops the readers and fails the scan.
   - H10: a CHD with a hunk size of 0 or over 64 MiB, and a CSO whose block index runs
     past the file, are refused; a GDI sector size of 0 counts as 2352. The cue byte-order
     mark is dropped when reading and rewriting a sheet.
   - H11: arcade duplicates are the files matched to one ROM more than once where
     neither is in its own place (`DUPLICATES_ARCADE`), so a non-merged set's clones
     aren't duplicates; the count and the list come from one query (`dm`).
   - M10: a folder's files are guessed again when its system hint or place changes, at
     once (`scan::reguess_root`, from `roots_update`) and at the next scan when
     library.json changed elsewhere. M11: `arcade::refresh_places` writes an arcade DAT's
     places when it has none or another set type's (`index_meta` `places:<dat id>`), and
     drops them from a DAT whose system isn't arcade any more; it runs at every scan and
     when a system is saved. M12: unmatched cue sheets beside newly hashed files are
     looked at again. M13: a scan that changed nothing keeps the audit's summaries and the
     collections' generation, and a watcher scan reads only the index rows under its
     places. M14: an undated DAT is older than any dated one. M6: planning tries one
     hardlink per drive; where it can't be made, links count as copies (`copied` in the
     plan's `space`) and the preview says so.
3. **Exports and collections** (C5, H12–H16, M17–M19, M21, M22, M24).
   **Status: done (PR #33).** As built:
   - C5: `export::plan` keeps the recorded files of a game still in the collection that
     the library can't place now (they're wanted again, carried ones too), and says so
     in `left`.
   - H12: `collection::Sources.names` holds (system, name); a source file's system comes
     from its header name, else from its ROMs' CRC and size in the index's DATs, else
     any system. H13: a game from a DAT of the system's own that `addons::kind_of` tags
     is not a game of the collection; an add-on DAT's are.
   - H14: a folder entry (`entry_dirs`) is started from the file it's named for inside
     it in RetroArch playlists, LaunchBox's ApplicationPath and Pegasus' `file:`; Pegasus'
     `extensions` on a disc system leave out bin, img, raw, wav, sub and ccd.
   - H15: `next_game` finds `<game …>` with attributes. H16: *Export these…* with every
     game picked and a search sends the search's games as `only`.
   - M17: Check groups a system's DAT ROMs by archive or not; an arcade ZIP is checked
     against the non-merged places in it (`arcade::places`), an arcade CHD by
     `image::chd_sha1`. M18: undoing a step that read DATs again restores them
     (`history::before` with "read"). M21: `merge_pegasus` keeps the user's block for an
     exported file and sets keys with `pegasus_set`.
   - M22: with short names, titles shared by two games of a system become their full
     names (LaunchBox and RetroArch name pictures by title). M24: `export::record`
     counts text ops as written, so carried text files are recorded and set aside.
   - M19: recorded files indexed by path and by game, carried files deduplicated with
     sets, folder entries from `entry_dirs`, playlists and LaunchBox lists merged by a
     path map. `timing_export_of_8k_games_with_pictures_and_its_update` (ignored, run
     in CI): 1.4 s to plan and 1.4 s for a no-op Update on Linux. Making the plan a job
     with Stop is left for batch 4's interface work if it's still wanted.
4. **Interface fixes** (H22, H23, M33, M34, M36–M39, M43, the Low wording and layout
   items), and video previews (Jerred, 2026-10-19): audio playing twice, as if a hidden
   copy autoplayed before Play was pressed (look for two `<video>` elements for one file,
   and add a page test that plays one); .mov and .m4v shown as broken pictures
   (`isVideo` in gamepage.js and metadata.js); .avi and .mkv may not play (`mime()` in
   main.rs has no mkv); the video thumbnail uses the picture icon.
   **Status: done (PR #34).** As built:
   - Video: `web/ui/video.js` has `isVideo` (mp4, m4v, mov, webm, mkv, avi) and `Video`,
     the one player used by the game page's hero, the Media ribbon's large view and the
     Metadata page's `Picture`. The cause of the double audio was a `<video>` in each
     Media ribbon thumbnail as well as the large view: thumbnails now show the video icon
     (`Icon.video`) and never a player, and a capturing `play` listener pauses every other
     playing video or audio. A video that fails shows *This video can't play in the app*
     with *Open it in its own player* (`openPath`). `mime()` in main.rs and the CLI's
     server send mkv as video/webm (WebView2 plays Matroska with VP8/VP9/AV1) and avi as
     video/x-msvideo (which falls to the message). The page test records a WebM in the
     page, plays both players and checks only one plays, then an AVI's message.
   - H22: the Audit bar's place shrinks and its count is cut with an ellipsis; the page
     test checks no sideways scroll at 1000 px. H23: `audit_summary` returns `platforms`
     with a null summary, and Metadata and Audit forget a remembered system that has no
     DAT now.
   - M33: `opened.key` is the game the list was opened at; Previous/Next show only on
     that game. M34: the game page's *Leave out* calls `collection_hand` with `leave_out`.
   - M36: the document dialog closes on Esc and when the page changes. M37: a layout's
     unsaved draft is kept per library while away from the page, the Library's scroll is
     kept per view and query, and Import resets when the library changes. M38: Enter on a
     Library tile or line opens it. M39: DAT imports end in one message (`failedLines`).
     M43: the filter panel's column is `minmax(0, 1fr)`.
   - Low items: the Library table's columns fit 1280 px (min-width 800), *Choose…* beside
     its box, `shortPath` for long folders, `ID_NAMES` for store ids, "system" for
     "platform", *Put back*, Close-only dialogs when there's nothing to do, BIOS left out
     of collection systems, the narrow breakpoint the same 900 px everywhere, the first
     start's command bar. Not done: making the export plan a job with Stop (batch 3's
     note); it plans 8,000 games in 1.4 s, so it waits until someone asks.
5. **Metadata, PC games and magazines** (H17–H21, M25–M32).
   **Status: done, with batch 6 (PR #35).** Jerred asked for batches 5 and 6 to go
   together. The tests name the finding in a comment. As built:
   - H17: a scrape, *Making details again* and Identify take the change's place with
     `begin_meta` (its name starts `meta:`, so a scan may run beside it but no change,
     other scrape or hand edit may); every hand edit asks `editable()` first.
   - H18: Identify writes `identified_by` into game.json and `sources::make_fields` puts
     that source first in the order, under a pinned field or a field's own source.
     M27: Identify sets only `identified_by` and the ids, and `sources::make` builds the
     rest, so hand edits stay.
   - H19: `scrapers::searches_system` says whether a source's name search is limited to
     the system; IGDB and TheGamesDB without the system's id give candidates only. The
     ids were filled in for 12 systems and 6 systems added (ColecoVision, Intellivision,
     Vectrex, Pokémon Mini, PC-FX, CD-i).
   - H20: a magazine's import finds the folder with its manifest by `organise::key` when
     the title isn't found. H21: dates and names are cut by characters. M32: a cover or
     issue file outside its magazine's folder (`magazines::inside`) isn't served or
     exported.
   - M25: `meta::sync` builds the PC games' title map once. M26: LaunchBox's search
     orders exact, then prefix matches, then length, before its 500-row limit. M28:
     Identify and Delete metadata's planning open their own index connection. M29: a
     source failing three games in a row is switched off for the run (`FAILS`), and
     ScreenScraper's account is read on a thread while Stop is polled. M30: Stop is
     heard between inbox items. M31: `pcgames::read_manifest` reads leniently.
6. **One Import page** (D78–D80, which take in H5, H6, M8, M35, M40–M42 and H3).
   **Status: done, with batch 5 (PR #35).** As built:
   - D78, M41: `importFrom(card, run)` (importpage.js) opens `#/import/<section>` and
     starts the import there; Home, Audit, BIOS files, PC games, Documents, Magazines and
     Metadata call it. `listenForDrops` (documents.js, started by `mountShell`) sends a
     drop to the shown page's `useDroppedFiles` handler (a game's page, the Metadata
     panel's game, a magazine's cover), which returns true when it took it; otherwise
     `dropToImport` opens the Import page and asks what the files are (`import-drop`,
     guessed from the extensions; a page can return a guess, as Magazines does). Metadata's
     and Collections' first screens link to Import › DAT files.
   - Recent imports, History and a collection's history: `DayGroups` (layout.js) groups a
     history by day, the newest open; `ChangeHistory` (settings.js) is History's list and,
     with `IMPORT_KINDS`, Recent imports'. *Steps* opens `ChangeSteps` (organise.js), a
     page at a time from `change_items`. A collection's step names up to five games, and
     more behind a `<details>`; `history::SHOWN` is 1000.
   - D79: `HowControl` and `howArgs` (organise.js) in every import: games, One game…, PC
     games, documents, magazines and BIOS files (`bios::gather_plan` and
     `magazines::plan` take the mode; BIOS plans are kind `bios`). Every last button
     says *Import*.
   - M40: plans' `title` is present tense ("Import 2 PC games"), `label` past tense for
     History; the preview counts the plan's own kind (`COUNTS`). One game… asks for the
     file first. M42: the import review lists the games before where they go. D80: the
     Scrape window shows one line of what's fetched and *Change what's fetched…*.
   - M35: a `sticky` dialog ignores clicks behind it and Esc unless `onDismiss` puts it
     away with nothing lost; the document, magazine, PC game and collection-from-folders
     windows are sticky once they've read something.
   - H3: a ZIP already named as its game, with the file inside it misnamed, is zipped
     again in place (`organise::rezips`), the old archive set aside first. H5: a sheet the DAT doesn't know whose
     tracks are a matched game's comes in with the game. M8: a game of several files comes
     in whole, in `<place>/<game>/`, or not at all; an unknown sheet is rewritten with the
     new names. H6: a bad copy where a game goes is set aside and replaced; a good one is
     left (`subset::holds`).
7. **The rest of CI and tests** (M44–M46): chdman fetched, a dolphin-tool stand-in on
   Windows, the page test split, page tests for D77's move and new-folder paths and the
   four frontend exports, and why Windows' first scan is 25 times slower than Linux's.
   **Status: done (PR #36).** As built:
   - M45: the scan's summary says where its time went (`took`: listing, comparing,
     hashing, and the readers' opening and reading against the one writer's recording).
     On Linux the first scan of the 100,000-file timing set was 18 s: 8.5 s comparing,
     each new file's system guessed by squashing every system's names
     (`platforms::from_folder` now keeps a map, made again by `configure`), and 8.8 s
     recording, a commit (and a WAL checkpoint) every 250 files. `scan::BATCH` is 5,000,
     still at most a second apart, and the insert is a cached statement: 5 s. Windows was
     then 30.7 s, 25.5 s of it recording: each SQLite read and write costs far more there
     (under strace on Linux, recording took 49 s). `Index::open` sets `cache_size` to
     64 MiB (filled only as used) and `wal_autocheckpoint` to 10,000 pages: 7 times fewer
     reads and a third of the writes, recording 15 s under strace and 3.5 s for the whole
     scan on Linux. The target: a first scan of 100,000 files under 30 s on CI's runner
     (the timing test).
   - M44: `desktop_page.py`'s parts are `Section`s: a timeout or error is a FAIL with its
     line and `failed-<part>.png`, the page goes back to Home and the next part runs.
     Phase 7's arcade set, the magazines and a new D77 part (games of two systems moved
     in, one to a new folder named for its system, which becomes a folder with games, and
     Undo) run in homes of their own (`OWN_HOMES`). The four frontends are exported and
     read back where each looks; the Players filter is used, not only looked for.
   - M46: CI fetches chdman (MAME 0.289's Windows build, its SHA-256 checked, cached;
     Ubuntu's `mame-tools` on Linux) and sets `GAMEREPO_TEST_CHDMAN`, so its test fails
     rather than passing unrun; `tests/fixtures/fake_dolphin_tool.cmd` runs the RVZ tests
     on Windows; the timing step counts its tests; the bundles steps fail on a missing
     bundle; a pull request's reports go to `desktop-ci-<os>-pr`; only a push to `main`
     publishes; and the release job gives the published Unreleased section its version's
     heading on `main`.
8. **Groups on devices** (D81). **Status: done (PR #38).** As built, after checking each
   firmware's documentation (2026-10-19):
   - Knulli: custom collections are Batocera's (knulli.org/configure/collections,
     wiki.batocera.org/game_collection_settings): `custom-<name>.cfg` in
     `/userdata/system/configs/emulationstation/collections/`, a game a line by its full
     path (`/userdata/roms/nes/Game.zip`). The SHARE partition is `/userdata`, so the
     layout's new `collection: "es-cfg"` writes one for each group under
     `{card}/system/configs/emulationstation/collections`, naming the games from
     `collections.device` (`/userdata`). Its games' path has `{group}`, so Subfolders is
     offered too; the pictures (written into the gamelist) get `{group}` as well, nothing
     for a first group. Separate systems isn't: Batocera's systems are defined in its
     read-only system image, so there's nothing to copy one from.
   - ROCKNIX: its EmulationStation has custom collections (rocknix.org/configure/collections),
     but keeps them in `~/.config/emulationstation` on its ext4 storage partition, which
     Windows can't write (rocknix.org/faqs), and the layout writes the games card. So it
     gets Subfolders only; "Collections only" is *Together* in the Export dialog for a
     layout with no collections, and the preview says so when the groups aren't kept apart.
   - muOS: it has collections (muos.dev/tour/modules/muxcollect, `MUOS/info/collection/`),
     but their `.cfg` format isn't documented, the 2508 Goose release changed it and broke
     older files, and its developers don't plan PC tools for it. Nothing is written for
     them; the preview says the groups are with the rest.
   - LaunchBox and Playnite (no group lists) get the same note. The layout editor's
     collection inspector chooses none, ES-DE's or the device's, and *Add to it* adds one.

**D78 One Import page** (Jerred, 2026-10-18). Every import goes through the Import page:
the buttons on Home, Audit, BIOS files, PC games, Documents and Magazines open its
matching section, files dropped anywhere go there, and empty pages point to it (finishes
D71 and D73). The Import page gains *Recent imports*. In Recent imports, History and a
collection's history, actions are grouped and expandable: "if I imported 5 games today
and 500 yesterday, those should by default be grouped and I can expand them to view all
changes in a group."

**D79 One copy or move control** (Jerred). Every import uses the Import page's three
choices (copy, hardlinked on the same drive; always copy; move) and one button word.
D78 resolves it.

**D80 A shorter Scrape window** (Jerred). The details and pictures to fetch go behind
*Change what's fetched…*; the window shows the sources and the systems or games.

**D81 Groups on devices** (Jerred). Collection groups go to the device layouts whose
firmware is confirmed to support custom collections, with the same choices ES-DE has.
muOS's collections are to be checked against its documentation first ("if muOS can show
custom collections, that's a good addition"); Knulli and ROCKNIX follow Batocera's.

**D82 Later** (Jerred). A lock so two computers can't run a change on one library at once
(M9) and streaming DAT import (M15) go in a later phase. Re-applying a new picture size to
pictures already exported (M20) is device-specific and goes in whichever phase suits.

**D83 A release from the tested build** (Jerred, 2026-10-19, after batch 7). A pull
request's run builds and tests the Windows app as if merged, and the push to `main` did
it all again (about 20 minutes) before publishing. Now a push whose git tree and version
a run of this repository's own already passed publishes that run's installer, so what
ships is the build that passed; anything else, such as another merge in between, builds
and tests as before (CHANGELOG.md, "For agents").

### Device client requirements (D84–D88)

Jerred asked (2026-10-19) for a download client for space-constrained devices: browse a
library far larger than the device holds, pick a few games, put them where the frontend
finds them, and take them off again. Full requirements, with numbered items C1–C23 and
S1–S7, are in the Claude Doc "Game Repo device client: requirements"
(https://claude.ai/code/artifact/884dfbd9-515b-49d1-9918-41cac8f888ba).

**Targets**: Windows PC or handheld and Android (ES-DE) first; the Steam Deck (ES-DE as
EmuDeck installs it) and Linux PCs next; muOS, Knulli, ROCKNIX later.

**Game Repo Server (S1–S7)**
- A small app built from the same core, reading the library folder Game Repo made, with
  its own index (built from the folder the first time, as on a new computer, D15, and
  kept up to date as Game Repo changes the library). It never writes to the library.
- Game Repo starts and stops it from Settings › *Serve to devices*; it also runs alone
  with a tray icon; `gamerepo-cli serve-devices` is the same server with no window.
- Announces itself on the home network; pairs a device with a six-digit code; a
  *Devices* list shows each paired device, when it was last seen and what it holds.
- JSON API: systems, collections, games with details and picture links, searchable and
  paged, with a change marker.
- A **plan** per set of games on a device: every file to write (games, discs, arcade
  dependencies, the chosen media kinds, pictures at the device's size), with place, path,
  size and hash, from the same layout engine as exports (D36, D38). The client never
  works out paths itself.
- Resumable downloads (HTTP ranges), several at once; pictures resized and cached.
- Later (9d) the same server as a NAS container, beside the library.

**The client (C1–C23)**
- Pages: Library, Queue, On this device, Settings. Finds the server on the home network
  or by address; pairs once; finds ES-DE's ES-DE folder and ROMs folder
  (`ES-DE/settings/es_settings.xml`); one library and one device.
- Browse by system and collection with box front, title, year, size; search; only
  systems ES-DE has a folder for; the last catalogue is kept for browsing away from home.
- A selection shows its total size against free space. Before downloading the user picks
  which media kinds come with the games, each with its size; remembered per device.
- Downloads run in the background, resume, pause and cancel; each file is written under
  a temporary name, checked against the library's hash, then renamed into place.
- A game brings what it needs: all discs and its `.m3u`, an arcade game's BIOS, device
  and parent sets. Formats are the library's own (D12); pictures sized for the screen.
- Placed by the built-in ES-DE layout: `<ROMs>/<system>/`, `ES-DE/downloaded_media/`,
  `ES-DE/gamelists/`; gamelists merged, keeping ES-DE's own entries (D35); optional ES-DE
  custom collection (D75, D81); says when ES-DE needs restarting.
- *On this device* lists every game ES-DE has there, with size, last played and whether
  it came from the library (by hash). Removing a game removes its files, media and
  gamelist entry after a confirmation; shared BIOS/parent sets stay while needed; saves
  never touched. Media can be removed or added on their own. *Make room* offers the
  least recently played games. A record of what it added: `ES-DE/gamerepo/device.json`.
- Rust + Tauri 2 + Preact (D1), one codebase for Windows and Android; released by CI on
  the same GitHub release as the desktop app (an `.apk` and the client's and server's
  Windows installers).

**Not in the first version**: frontends other than ES-DE; the Linux build; PC games;
BIOS outside the ROMs folder; converting to formats the library doesn't keep; save
files; changing the library from the client; more than one library per client; access
from outside the home network.

**Parked: reaching the library away from home.** Options besides a VPN such as
Tailscale: download before leaving (C8 already keeps the catalogue); the NAS's own
reverse proxy (likeliest once the server is on a NAS); a tunnel the user runs
(Cloudflare Tunnel, ngrok); port forwarding with HTTPS; a relay run for Game Repo (not
suited to private use). All but the first need HTTPS only, per-device tokens that
expire and can be revoked, and a limit on failed pairing attempts.

**D84 Device client** (Jerred, 2026-10-19). A small client app for Windows and Android
browses the library and adds and removes games, with the media chosen, for ES-DE.
Replaces D18's "no client app" for those; the Steam Deck is next; muOS, Knulli and
ROCKNIX come later with a design of their own.

**D85 Game Repo Server** (Jerred, 2026-10-19, on Claude's recommendation). A separate
small app reads the library folder and serves it read-only; Game Repo starts and stops
it; Windows first, a NAS container (Unraid, Synology) later.

**D86 Home network only** (Jerred, 2026-10-19). Paired devices on the home network;
access from outside is parked ("Device client requirements").

**D87 Android install** (Jerred, 2026-10-19). An APK on the GitHub release, with *All
files access*; the app is for private use.

**D88 What a client installs and removes** (Jerred, 2026-10-19). Scraped media is chosen
per kind when installing and removed with the game (or on its own); the client may
remove any game ES-DE lists on the device, with its media and gamelist entry, and
nothing outside ES-DE's folders.

### Phase 9a design: Game Repo Server (written 2026-10-19; Jerred agreed it the same day)

Jerred asked (2026-10-19) for the server and client milestone to start with Game Repo
Server (D85). 9a builds requirements S1–S6 on Windows and tests them with a stand-in
client, as the sources were tested with stand-ins; 9b builds the client's screens on the
stand-in's code. D89–D95 were proposed here and Jerred agreed them (2026-10-19).

**What's built**

- `desktop/wire` (`gamerepo-wire`): what the server and a client say to each other: the
  API's types, a plan, the device's record (`device.json`), and the gamelist and
  custom-collection merges, moved out of `export.rs`, which keeps using them. It's small
  and has no archive or disc code, so the Android client can link it without the core.
- `desktop/serve` (`gamerepo-serve`): the server as a library on the core: HTTPS,
  discovery, pairing, the API, plans and downloads.
- `desktop/server` (`gamerepo-server`, Tauri 2): Game Repo Server itself, a tray icon and
  a small window opened from it.
- `gamerepo-cli serve-devices` (the same server without a window) and `gamerepo-cli
  device …` (the stand-in client). The CLI moves to a crate of its own (`desktop/cli`,
  same binary name and place in `target/`) so it can use the server's crate; CI builds it
  with `-p gamerepo-cli`.
- Settings › *Serve to devices* in Game Repo, and the server's installer on the release.

**D89 One server program, started by Game Repo**

- `gamerepo-server.exe` is installed with Game Repo (a Tauri sidecar, beside the app) and
  is also on the release as an installer of its own, for a PC without Game Repo. One copy
  runs per computer: it writes `server.json` in its data folder (its process, ports, and a
  secret for the control channel below) and a second start finds the first.
- Game Repo starts it as a separate process with the open library (`--library <folder>`)
  and talks to it on a **control channel**: plain HTTP on `127.0.0.1` only, a port of its
  own, every request carrying the secret from `server.json`. Through it Game Repo shows
  whether it's running and where, the pairing code, the Devices list, and *Stop*; it also
  tells the server when it changed the library (D90).
- Settings › *Serve to devices*: a switch (start, stop), the address devices see, *Pair a
  device*, the Devices list, the port, and two choices that default to off: *Keep serving
  when Game Repo is closed* (the server stays with its tray icon) and *Start with Windows*.
  When Game Repo is closed and the server isn't kept, it stops with Game Repo.
- Alone, the tray icon's menu has *Pair a device…*, *Devices…*, *Open the library…*
  (another library folder) and *Stop*. Its window is the same Preact component as Game
  Repo's Settings panel, talking to the same control channel.
- The installers add a Windows Firewall rule for the server on **private** networks only
  (inbound TCP on its port, UDP 5353 for discovery). Windows often puts a new home Wi-Fi
  network down as public, which the rule then keeps out, so Settings says when the
  network is public (`Get-NetConnectionProfile`) and how to make it private.

**D90 Its own index, never writing to the library** (S2)

- The server opens the library read-only: a new `Library::open_read_only` creates nothing
  (no `_library/`, no README, no `library.json`), and the library's writes go through
  `Library::writable()` (72 places today), which then refuses; the build checks that none
  goes round it, and a test proves the folder unchanged (below). A library from a newer
  Game Repo is refused with "update the server". Its index, thumbnails, made files (D93)
  and devices are in its own data folder (`%LOCALAPPDATA%\io.github.drflgd.gamerepo.server\`, part 3), as a
  second computer's would be.
- **The first index** is a copy of Game Repo's when it's on the same computer and the same
  version (SQLite's online backup, safe while Game Repo writes), then checked by a quick
  scan (size and modified time, as now). Otherwise it's built from the library like a new
  computer's (D15): its folders with games, DATs and `game.json` files, then a scan.
- **Kept up to date**: Game Repo tells it over the control channel which folders and
  systems a finished job changed, and it scans those again by size and time; alone, it
  watches the folders with games and `_library/` with `watch.rs`, and folders on network
  drives (which don't report changes) get a quick scan at start and every 30 minutes. Each
  scan ends by rebuilding the changed systems' Library rows (`browse::refresh`), so the
  catalogue is the Library page's own `library` table.

**D91 Found on the home network, paired with a code** (S3)

- **Discovery**: DNS-SD over multicast DNS, service `_gamerepo._tcp`, with the library's
  name, the server's id, its version and the API version in the TXT record (the
  `mdns-sd` crate). A device can also be given an
  address (`192.168.1.20`, or with `:port`).
- **HTTPS with a pinned certificate**: the server makes itself a certificate the first
  time (ECDSA P-256, `rcgen`), served with `rustls`. A client accepts it at pairing and
  from then on talks only to a server with that certificate, so the token never crosses
  the network in the clear and later connections can't be listened to or answered by
  another machine. The gap left: someone actively intercepting on the home network at
  the moment of pairing. That's closed later, when outside access is designed, by
  pairing with a password-authenticated exchange (SPAKE2, as Android's wireless
  debugging pairs with its six-digit code). The alternative is plain HTTP with a token,
  as media servers often do at home: simpler, but anyone on the network could read the
  token and the library.
- **Pairing**: *Pair a device* shows a six-digit code for 10 minutes. The device sends
  the code with its name and kind (`POST /v1/pair`); the server answers with a token (32
  random bytes) and keeps only its SHA-256. Five wrong codes end the pairing; it's
  started again from Settings. Every other request carries `Authorization: Bearer
  <token>`; a removed device gets 401 and the client says it must pair again. The token
  is kept in the device's secure storage (Windows Credential Manager, Android Keystore).
- **Home network only** (D86): the server answers only addresses on the local network
  (private IPv4 ranges, link-local, IPv6 unique-local and link-local), whatever the
  firewall says, and never sets up port forwarding.
- **Devices** (S3): each paired device with its name, kind, when it was paired and last
  seen, its address, and what it holds (the client reports its record's games, their
  size and the free space when it connects, `PUT /v1/device`), with *Remove*. A device
  can unpair itself too (`DELETE /v1/device`, C2).

**D92 The API: a catalogue a client can keep** (S4)

JSON over HTTPS, versioned by path (`/v1/`); `GET /v1/info` (the one request without a
token) gives the server's id, the library's name, the version and the API version, and a
client refuses a newer API than it knows with "update the client".

| Request | What it gives |
|---|---|
| `GET /v1/systems` | each system with games: id, name, ES-DE folder, game count and size |
| `GET /v1/collections` | each collection with its groups, game count and size |
| `GET /v1/games?system=&collection=&group=&q=&sort=&offset=&limit=` | a page of games (at most 500): key, system, title, year, size, the media kinds it has, its picture's link; `total` and the catalogue's `rev` |
| `GET /v1/games/<key>` | one game's details (description, developer, publisher, genres, players, rating, regions, its files, discs and add-ons, each media kind with its size) |
| `GET /v1/changes?since=<rev>&offset=` | the games changed since `rev` (as the list gives them) and the keys gone, a page at a time, with the current `rev`; `410` when `rev` is too old, so the client fetches the whole list again |
| `GET /v1/pictures/<key>/<kind>?size=128\|256\|512` | a game's picture as a thumbnail (D43's, cached on the server), with an ETag |
| `POST /v1/identify` | for files on a device (name, size, and CRC32s: a ZIP's members' from its directory, or a CHD's SHA-1 from its header), the library's game for each (C18) |
| `POST /v1/plans` | a plan (D93) |
| `GET /v1/files/…` | a file of a plan (D95) |

- Paging, sorting and search are `browse::query`'s, in SQL with FTS5 (already timed at
  120,000 games); a game's key is the Library's (`<system>/<DAT name>`,
  `<system>/~<folder>`, `pc/<id>`).
- **The change marker**: the server keeps a hash of each game's list fields and a
  counter, `rev`. When a refresh changes a game's row, or adds or removes one, the counter
  goes up and the game is marked with it, so `changes?since=` is one indexed query. A
  client that keeps the catalogue (C8) fetches it once and then only what changed, and
  browses and searches its own copy, which makes scrolling instant away from home too;
  the server's paging and search are there for the first minutes and for small clients.

**D93 The plan: the export's layout engine with the device's places** (S5)

A client asks for a plan for some games (`POST /v1/plans` with their keys, the media
kinds wanted, whether manuals come, the device's longest screen side, and the layout,
the built-in `esde` for now). The server answers with every file to write:

- each with its **place** (`roms`, `esde` or `media`, the ES-DE layout's own places, which
  the client finds on the device from `es_settings.xml`, C3), its **path** in that place,
  its size, its SHA-1, its game, its role (game, disc, playlist, BIOS, parent set, device
  set, a media kind, manual) and, for a BIOS, parent or device set, every game in the plan
  that needs it (C12, C20);
- the **gamelist entries** for each system's `gamelists/<folder>/gamelist.xml`, as data
  (the entry's path and fields), which the client merges into the device's gamelist with
  the export's own merge (D35, now in `gamerepo-wire`), keeping ES-DE's entries;
- the lines for an ES-DE **custom collection** when one is asked for (C16, D75), in
  ES-DE's `%ROMPATH%` form, merged the same way;
- each **media kind's size** for the games, the kinds not asked for too, so the client can
  show them before the user picks (C13, D88), pictures as they'll be at the device's
  size;
- the **record** to add to `ES-DE/gamerepo/device.json` once it's all written (each game's
  key, title, system, files with their roles and hashes); the format is `gamerepo-wire`'s.

How it's worked out:

- `export::plan` is split in two. The first half, new as `export::lay_out`, takes the
  games, the layout and the carry choices and gives the files with their places, paths
  and sources, and the list entries; it reads nothing at the destination. The second
  half (what's there already, merging lists with what's on disk, the journal) stays the
  export's. Exports keep their tests and give the same plans; a test checks that a
  device plan's paths are those an ES-DE export of the same games writes, with its places
  as folders.
- The library's files go as they are (its formats, D12). Files that have to be made are
  made in the server's data folder when the plan is asked for and kept there by hash
  (the newest 20 GB, set in Settings): an arcade set rebuilt for the set type an export
  uses (D61, with TorrentZip, so the same set gives the same file), a ROM taken out of a
  larger archive, a picture shrunk to the device's size (`export::shrink`, as JPEG or PNG
  as it was). A plan with files to make answers at once with `ready: false` and the
  files as they're made; most plans are ready straight away.
- A file's SHA-1 comes from the index when it has the whole file's (loose ROMs). Archives,
  CHD, RVZ and CSO files are hashed by the server in the plan's order, alongside the
  downloads (the disk is quicker than the network), and remembered by path, size and
  modified time; the client checks the hash once its file is complete
  (`GET /v1/files/…/sha1`, which waits until it's known) (C11).
- A plan is only advice: it holds no lock and the server keeps nothing for it but its
  made files, so a client can ask again at any time, and a game in the library that
  changed since gives a new plan whose files no longer match.

**D94 A game on several discs is one game on the device** (C12)

The Library lists each disc of a Redump set as a game. On a device, discs of one game
(names the same once `(Disc N)` is taken out, same system and DAT) are one game in the
catalogue, with the discs' total size; a plan writes them into a folder
`<name>.m3u/` in the system's folder, with `<name>.m3u` inside listing the discs, which
ES-DE shows as one game (its *directories interpreted as files*), and one gamelist entry
and one set of media for it. Only for systems whose ES-DE extensions include `.m3u`
(`platforms.rs`); others keep a game per disc. The ES-DE export writes discs the same
way, in 9a, as the same code (Jerred, 2026-10-19).

**D95 Downloads that resume, several at once** (S6)

- `GET /v1/files/library/<path>` for a file in the library, `GET /v1/files/made/<sha1>`
  for a made one. Only files in the folders with games and the games' media and
  documents can be fetched (never `_library/`'s settings, journal or DATs, never outside
  the library). A link stays good while the file is unchanged: its ETag is its size and
  modified time, so a resume of a file that changed (`If-Range`) starts again.
- Single byte ranges (`Range: bytes=n-`), so a download picks up where it stopped (C10).
  Files are streamed from disk in 1 MiB pieces, never read into memory (the page test's
  `library_range` reads a range whole and stays as it is).
- Up to four downloads at once per device (the client runs three by default), any number
  of devices. The client writes each under a temporary name, checks its SHA-1 and renames
  it into place (C11).
- Thumbnails and pictures carry `Cache-Control` with their ETag, so a client asks again
  only when one changed.

**The stand-in client and the tests**

- `gamerepo-cli device find | pair <address> <code> | systems | games [--system …] [--q …]
  | add <keys…> --to <folder> [--media cover,manual] [--size 640] | remove <keys…> --to
  <folder> | holdings --to <folder>`, where `<folder>` stands for a device: a folder with
  `ES-DE/settings/es_settings.xml` and `ROMs/`. Its downloading, checking, gamelist
  merging, record keeping and removing (shared sets kept while needed, saves left alone,
  nothing outside ES-DE's folders) are a crate, `desktop/device`, that 9b's client is
  built on.
- Rust tests: pairing (wrong codes, the limit, a removed device's 401, a pinned
  certificate that changed); the library folder unchanged, byte for byte and time for
  time, after a server has indexed it, served it and made files; plans against the ES-DE
  export's paths; a multi-disc game's folder and `.m3u`; an arcade game's sets, with a
  set shared by two games kept until both are removed; a download cut at 40% and resumed;
  four at once; a file changed under a resume; `changes?since=` after a library change;
  the discovery record built and read back (multicast itself isn't on every test
  machine, so it's tried end to end on Windows CI).
- Timings (`--ignored timing`, as now): with 100,000 games, a system's first page under
  200 ms at the server (leaving the network the rest of the second), `changes?since=`
  under 100 ms, one download over loopback HTTPS at least 250 MB/s (twice gigabit).
- Page test: Settings › *Serve to devices* starts the server, shows a code, the stand-in
  pairs with it, the device shows in the list, *Remove* takes it off and the stand-in
  is refused.
- CI: Windows builds the server, its installer and the stand-in, and runs the stand-in
  against the server; the release adds `Game Repo Server_<version>_x64-setup.exe` to the
  same GitHub release (with D83's reuse of a tested pull request's files).

**Not in 9a**: the client's screens and the Android build (9b); the Linux build and the
Steam Deck (9c); the NAS container, outside access, pairing by SPAKE2, other frontends,
the read-only WebDAV view and handhelds (9d). 3.10's web page for any browser is left out
too: the client covers it.

**Notes for 9b, found in this design**: pictures reach the client's page through its own
protocol (as Game Repo's `library://`), since a web view won't trust the server's own
certificate; Android needs a multicast lock while it looks for the server.

**Order of the build**, each a pull request tested before the next: (1) the groundwork in
the core: `open_read_only`, `export::lay_out`, the discs rule and `gamerepo-wire`, with
exports unchanged; (2) the server crate and the stand-in, from the CLI; (3) Game Repo
Server's tray app, Settings › *Serve to devices*, the installers and the firewall rule.

### Phase 9a notes, part 1: the groundwork (built 2026-10-19)

The first of 9a's three pull requests: the core's side of the server, with exports giving
the same plans as before except for games on several discs (D94).

- **`desktop/wire` (`gamerepo-wire`)**: a crate of its own in the workspace, with only
  `serde` and `serde_json`. It holds the plan (`Plan`, `File`, `List`, `Entry`, `Lines`,
  `Left`), the gamelist merge moved out of `export.rs` (`gamelist::merge`, the old
  `merge_gamelist`, with `next_game`, `set_tag`, `set_tag_in` and the escapes, plus
  `merge_entries` for a plan's entries) and `merge_lines` for custom collections. The
  export uses it, so an export and a device merge a gamelist the same way. The API's
  types and the device's record (`device.json`) join it in part 2, with the server.
- **`Library::open_read_only`** (D90): refuses a folder without `_library/library.json`
  (and creates nothing in it, not even the README `open` writes) and a library from a newer
  Game Repo ("update the server"); every write goes through `writable()`, which refuses.
  The design's build check that no write goes round `writable()` is a test instead:
  `a_server_reading_the_library_leaves_it_as_it_was` takes the library folder and the
  folder with games byte for byte and time for time, runs what the server will (rebuilding
  an index of its own with `dat::sync_library`, `scan::scan`, `meta::sync` and
  `browse::refresh`, then a device plan with media, details and a manual) and finds both
  unchanged.
- **`export::lay_out`** (D93): the design split `export::plan` in two. Built instead with
  less risk to exports: `lay_out` runs the export's own plan (`plan_with`) for the asked
  games, as a collection made on the spot (`include: none` with the games added by hand),
  with the layout's places in a scratch folder that doesn't exist, so nothing at a
  destination is read and every file is a step. Each step then becomes a plan file: its
  place (the longest place folder it's under) and path, size, role (the media kind the
  export carried, playlist, manual, BIOS set or device from the DAT, else game),
  `needed_by` for a BIOS set or device (`arcade::containers_for`), `made` for anything but
  a plain copy, and a SHA-1 only for a loose file the index has whole. The gamelists and
  custom collection lines come back as data (`Lists`) rather than as text steps.
  `DeviceAsk` and `Laid` (the plan with each file's step, which part 2 runs to make a
  file). Media sizes for the kinds not asked for, the device's record and made files in
  the server's folder come with the server in part 2. A test checks the device plan's
  files, by place, against the tree an ES-DE export of the same games writes, its
  gamelist merge against the export's gamelist, and its custom collection lines.
- **Discs** (D94): `platforms::disc_of` reads Redump's `(Disc 1)`, `(Disc 2 of 3)` and
  `(Disc A)`; `platforms::m3u_folders` is the nine systems whose ES-DE entry takes `.m3u`
  (psx, ps2, saturn, segacd, dreamcast, pcenginecd, pcfx, gc, wii). For the ES-DE layout
  (not a `naming` style), `disc_sets` groups a system's discs of one game (two or more,
  in disc order), organise's `Options.disc_dirs` puts them in `<name>.m3u/`, `disc_start`
  picks each disc's file for the playlist (its cue, gdi, cdi, ccd or mds, else its only
  file), and the playlist `<name>.m3u` is written as a text step recorded as
  `carried: playlist`. The game is listed once, by the first disc's game with the name
  without the disc, with one set of media. A game with one disc of a set keeps its own
  file. Updating an export from before sets the loose discs aside, as for any file it no
  longer wants; their old gamelist entries stay, and ES-DE doesn't show entries whose
  file is gone.
- **CI**: the core tests run with `-p gamerepo-core -p gamerepo-wire`; `vendor.yml`
  copies `desktop/wire` into the vendoring workspace and runs when its `Cargo.toml`
  changes.

### Phase 9a notes, part 2: the server and the stand-in device (built 2026-10-19)

The second of 9a's three pull requests: Game Repo Server as a crate, run from the CLI, and
a stand-in device client, tested end to end. No window, tray icon or installer yet (part 3).

- **Crates**: `desktop/serve` (`gamerepo-serve`), `desktop/device` (`gamerepo-device`) and
  `desktop/cli` (`gamerepo-cli`, the binary moved out of the core, same name and place in
  `target/`). The server is plain threads, one per connection (at most 64), with HTTP/1.1
  written by hand over rustls (ring) rather than a web framework, so ranges, streaming in
  1 MiB pieces and the slots per device are in plain view; the client the same. New
  crates: rustls, ring, rcgen (the certificate) and mdns-sd (DNS-SD), all at least two
  weeks old; mdns-sd is pinned (`=0.21.4`).
- **D89**: `server.json` in the data folder (`%LOCALAPPDATA%\Game Repo Server\`, or
  `--data`) holds the process, the HTTPS port, the control port and secret, the library
  and the version; a second start is refused while the first answers. The control channel
  (`GET /status`, `POST`/`DELETE /pair`, `GET /devices`, `DELETE /devices/<id>`,
  `POST /changed`, `POST /stop`) is built now, so the CLI's `serve-devices status|pair|…`
  use it as Game Repo will.
- **D90**: the first index is a copy of Game Repo's (`VACUUM INTO`, only when its schema is
  this build's), else made from scratch; the server keeps it up to date itself (DATs,
  scan, metadata, the Library rows, then the catalogue) when told over the control
  channel, when a watched local folder changes, or every 30 minutes. Until the first pass
  is done the catalogue answers 503 ("still reading the library"). The library is opened
  with `open_read_only`; the tests take the library and the folders with games byte for
  byte and time for time.
- **D91**: the certificate is ECDSA P-256 for the name `gamerepo-server`; a device pins its
  SHA-256 at pairing and refuses any other ("pair it again"). Pairing codes are six
  digits, ten minutes, five wrong ones end the pairing; `devices.json` keeps each token's
  SHA-256 only. Connections from outside the home network's addresses (private,
  link-local, loopback, IPv6 unique-local) are closed unanswered. The TXT record carries
  `id`, `library`, `version` and `api`.
- **D92**: the catalogue is `serve_games` in the server's index, made from the Library's
  `library` rows of the DAT games had **whole**: a game had in part is left out, as an
  export leaves it out. For that the core's `library` table gained a `partial` column (a
  new index step, which makes every system's rows again once). PC games and files no DAT
  knows aren't in it yet. Each row has a hash; a refresh gives the rows that changed (and
  those gone, kept as `gone`) the next marker. The first marker is the time, so a
  catalogue made again never follows a marker from before (410). Pages sort by title,
  year or size, each with an index of its own (a deep page by size was 330 ms with
  100,000 games before them, 25 ms after). `GET /v1/games/<key>` gives the game's details
  and files as Game Repo's game page has them.
- **D93**: a plan is `export::lay_out` in a scratch folder under the data folder, as part 1
  built it. Each file gets the catalogue key of its game (`key`; empty for a set or BIOS
  file, whose `needed_by` lists the catalogue keys that need it) and a URL. The kinds'
  sizes come from a second `lay_out` with every media kind and manuals; pictures not
  asked for are estimated from their size and the device's screen (`estimated`). Made
  files are kept by **recipe** (a hash of the step, its sources' paths, sizes and times),
  not by their SHA-1, since a plan names them before they're made; a made file's URL is
  `/v1/files/made/<recipe>` and answers 202 until it's ready. The SHA-1 of any file is
  `GET <its URL>?sha1`, which waits up to 20 s, then 202. `PlanAsk` has `bios` too. The
  device builds its record from the plan (the server sends no separate record).
- **D95**: a file in a folder with games is `/v1/files/roots/<root id>/<path>` and one in
  the library folder `/v1/files/library/<path>` (only `_library/games/**/media|docs/**`,
  `_library/docs/**` and files outside `_library`), rather than one `library/<path>`, as
  the folders with games needn't be in the library. ETag `"<size>-<mtime ms>"`,
  `If-Range`, single ranges, 429 with `Retry-After: 1` past four downloads per device.
- **The device** (`gamerepo-device`): ES-DE's folders from `es_settings.xml`
  (`ROMDirectory`, `MediaDirectory`, `~` the device's home), files fetched three at a time
  to `<name>.gamerepo-part` with the ETag beside it, checked by SHA-1 and renamed into
  place; entries and collection lines merged with `gamerepo-wire`; the record
  `ES-DE/gamerepo/device.json` lists each game's files and its entries (collection lines
  as `line:…`), so `remove` deletes only what it added and keeps a file another held game
  lists. The stand-in keeps its pairing in `ES-DE/gamerepo/server.json`; 9b's client keeps
  the token in the system's secure storage.
- **Tests**: `serve/tests/devices.rs` (pairing, wrong codes and the limit, a removed
  device, a changed certificate; a game on two discs as one game and its `.m3u` folder;
  pictures made smaller; a shared BIOS set kept until both arcade games are gone; resume
  at 40%; a file changed under a resume; four at once and a fifth refused; `changes` after
  a game is added and one goes; identify; the library unchanged). Timings: a page of a
  100,000-game catalogue in every order under 200 ms (25–30 ms here), `changes` under
  100 ms (1 ms), a 512 MiB download sent at least 250 MB/s (870 MB/s here). The same
  download written to disk and synced is only reported, as that's the disk's speed more
  than the server's (320 MB/s here, 84 MB/s on Windows CI). `tests/serve_devices.py` runs the two programs on Windows CI: found by
  DNS-SD (it worked in the cloud session too), paired by a code from the control channel,
  games added and removed, a removed device refused, the library unchanged.
- **CI**: builds with `-p gamerepo-cli`, tests `-p gamerepo-core -p gamerepo-wire
  -p gamerepo-serve -p gamerepo-device`, timings with `-p gamerepo-core -p gamerepo-serve`;
  `vendor.yml` vendors the new crates too.

### Phase 9a notes, part 3: the tray app, Settings and the installers (built 2026-10-19)

The last of 9a's three pull requests: Game Repo starting and stopping the server, Settings ›
*Serve to devices*, Game Repo Server's own program with its tray icon and window, and the
installers.

- **Game Repo's side** (`core/src/serving.rs`, the `serve_*` commands in `api.rs`): the
  server is started as its own process with `--library`, `--data`, `--port`, `--parent`,
  Game Repo's index as `--index-from`, and `--keep` when ticked; its output goes to
  `server.log` in the server's data folder, whose last lines are the message when it
  doesn't start. Game Repo keeps the child's input open: with `--parent` the server reads
  it until it closes (Game Repo ended, however it ended) and then stops unless it's kept,
  so a crashed Game Repo doesn't leave a server behind it. *Keep serving* is told to a
  running server (`POST /keep`) as well as kept in Game Repo's settings (`serve`: `on`,
  `port`, `keep`). `on` brings the server back when Game Repo opens (`serve_resume`; why it
  didn't start shows in Settings). After a job or a command that changes the library
  (`TELLS_SERVER`), Game Repo tells a server serving that library (`POST /changed`).
- **Start with Windows** is HKCU's Run value "Game Repo Server" (the name Tauri's
  uninstaller already removes for the server's own install), pointing at the program;
  started with no library, the server serves the library and port it served last
  (`settings.json`). It's offered only where the program is installed (not for
  `gamerepo-cli serve-devices`).
- **The data folder** moved to `%LOCALAPPDATA%\io.github.drflgd.gamerepo.server\` (the
  server app's identifier, as Tauri names its folders): the server's own installer puts
  the program in `%LOCALAPPDATA%\Game Repo Server\`, which was to be the data folder, and
  its uninstaller's "delete the app's data" now removes the server's data.
- **Settings › Serve to devices** (`web/ui/serve.js`): the switch, the address, *Pair a
  device* with the code counting down (it goes when a device pairs, and "Paired with …"
  says which), the devices with what each holds and when it was seen, *Remove…* asked
  in the row, "serving another library" with *Serve this one instead*, the public-network
  note (`Get-NetConnectionProfile`) and, folded, the port, *Keep serving* and *Start with
  Windows*. It polls `serve_status` every 2 s (every second while a code shows).
- **Game Repo Server's program** (`desktop/server`, `gamerepo-server`): Tauri 2 with
  `tray-icon`; the same options as Game Repo gives `serve-devices`. The tray menu has *Pair
  a device…* (makes a code and opens the window), *Devices…*, *Open the library…* (serves
  another, on the same port), *Start with Windows* (a tick) and *Stop*; a click opens the
  window. The window is `server.html`, the same panel with *Open another library…* and
  *Stop serving* in place of the switch; closing it hides it. One per computer: a second
  start asks the first to show its window (`POST /show`) and ends. It ends when its
  server is asked to stop (Game Repo's *Stop*, or Game Repo ending when it isn't kept).
- **Installers** (`desktop/installer/hooks.nsh`, in both): the firewall rules are
  "Game Repo Server" for its own install and "Game Repo Server (with Game Repo)" for the
  copy beside the app, each for that program on private networks only: TCP (the port can
  change) and UDP 5353. Adding a rule needs an administrator, so it's asked for once
  (when the rule isn't there); if that's declined, Windows asks itself when the server
  first listens. Installing or removing stops that copy of the server first; removing
  (not updating) takes its rules away, and Start with Windows when it pointed there.
- **Tests and CI**: the page test's part "serve" (a home of its own) starts the server
  from Settings on a free port, pairs `gamerepo-cli device` with the code shown, sees it
  in the list, removes it and sees it refused, stops and starts it, and checks it stops
  when the backend that started it ends. `serve_devices.py --server` runs the stand-in
  against Game Repo Server's own program. Windows CI builds the server with its
  installer, runs the stand-in against it, copies it beside Game Repo and builds Game
  Repo's installer (checked to carry it), and the release adds
  `Game Repo Server_<version>_x64-setup.exe`.

### Phase 9b design: the device client for Windows and Android (written 2026-10-20; Jerred agreed it the same day)

Jerred asked (2026-10-20) for 9b to start: the client of the "Device client requirements"
(C1–C23) for Windows and Android, with ES-DE, on Game Repo Server as 9a built it. 9a's
stand-in already does the hard part of a device's side (`gamerepo-device`: pairing with the
pinned certificate, plans, downloads that resume and are checked by SHA-1, the gamelist
merge, the record, removing with shared sets kept); 9b puts screens on it, adds what a
device needs that a folder standing in for one doesn't, and builds it for both systems.
D96, the app's name, is Jerred's (*Game Repo Go*, 2026-10-20); D97–D104 were proposed here
and Jerred agreed them by merging the design (PR #44, 2026-10-20). What part 1 built, and
where it differs, is in "Phase 9b notes, part 1".

**What's built**

- `desktop/device` (`gamerepo-device`) grows into the client's whole backend, still
  without the core (it links `gamerepo-wire`, rustls, SQLite and nothing with archives or
  discs, so it builds for Android as it is): the catalogue kept on the device (D98), the
  queue (D99), *On this device* (D100), removing safely (D101), media on their own (D102),
  the client's settings, and one command table, `app::App::call(cmd, args)`, as the core's
  `api.rs` is for Game Repo.
- `desktop/client` (`gamerepo-client`, Tauri 2): the app itself, for Windows and Android,
  a thin shell over that command table, with pictures through a protocol of its own
  (`picture://`, from the device's cache or the server; a web view won't trust the
  server's own certificate, "Notes for 9b").
- `desktop/client/android` (`tauri-plugin-gamerepo-device`, Kotlin): what only Android can
  do (D97).
- `web/client.html` and `web/client/` (Preact): the four pages, Library, Queue, *On this
  device* and Settings, with the desktop app's look (its tokens, skins and themes) laid out
  for a phone first.
- `gamerepo-cli client-ui --home <folder>`: the client's pages and command table served
  over HTTP with `tests/tauri_shim.js`, as `gamerepo-cli serve` does for Game Repo, so the
  client is tried in Chromium at a phone's size and a PC's, against a real server.
- Game Repo Server: three small additions (below, "What the server adds").

**D96 One client app for both systems, named "Game Repo Go"** (Jerred, 2026-10-20)

- One app from one codebase: `Game Repo Go_<version>_x64-setup.exe` for Windows and
  `Game Repo Go_<version>_arm64.apk` for Android, beside Game Repo's and Game Repo
  Server's on the same release. Identifier `io.github.drflgd.gamerepo.go`.
- Jerred chose it from *Game Repo Client* (Claude's suggestion, beside *Game Repo Server*)
  and *Game Repo* on Android only: it fits under an Android icon and reads as the app for
  on the go. In the code and these notes it's still "the client" (`desktop/client`,
  `gamerepo-client`).

**D97 The Android parts in a plugin of the client's own**

Tauri 2 builds the Rust and the page for Android; a small Kotlin plugin does the rest:

- **All files access** (D87): asks for it on first start (Android's own *Allow access to
  manage all files* screen) and says plainly what it's for; without it the client can
  browse but not add. Android 10, which has no such permission, uses the older storage
  permission. ES-DE needs the same access, so devices that run ES-DE allow it.
- **The device's storage**: its volumes (internal storage and SD cards), so the client
  finds `ES-DE/settings/es_settings.xml` on any of them (C3), and the free space where the
  ROMs go (C9).
- **Downloads in the background** (C10): a foreground service while the queue runs, with
  Android's usual notification ("Downloading 3 games, 45%") and *Pause* and *Cancel* on
  it; a Wi-Fi lock so the connection doesn't sleep; a multicast lock while looking for the
  server ("Notes for 9b"). Android 15 lets such a service run six hours a day; past that
  the queue pauses and goes on when the client is next opened.
- **The pairing token** in the Android Keystore (an AES key kept by Android encrypts it
  in the app's own files); on Windows it's in Windows Credential Manager.
- **Controllers** (Retroid, AYN, Ayaneo): the D-pad already reaches the page as arrow
  keys; the plugin turns the left stick into them too, A into Enter, B into Back, X into
  ticking a game, and the shoulder buttons into a page up or down. On Windows the page reads controllers itself
  (the Gamepad API).
- The APK is built for arm64 (every handheld Jerred named), with an x86_64 build only
  for CI's emulator.

**D98 The catalogue is kept on the device** (C5, C8)

- The client keeps the catalogue in SQLite in its own data folder: the systems (with every
  system's ES-DE folder), the collections and their groups, and every game as the list
  gives it, with FTS5 for search. The first time it fetches the whole list a page at a
  time ("Reading the library: 12,000 of 100,000 games"), then only `changes?since=` each
  time it connects, and the whole list again on a `410`. Browsing, scrolling and search
  are its own copy's, so they're instant and work away from home; a game's details are
  fetched when it's opened and kept.
- **Pictures** (box fronts, at the screen's size) are fetched as they come into view and
  kept in a cache of their own (500 MB by default, set in Settings; the least recently
  shown go first). Away from home a game without a cached picture shows its title.
- **What's shown** (C6): the systems the device has an ES-DE folder for (ES-DE makes them
  all when it's set up, so in practice those it knows), with *Show every system*; games
  already on the device are hidden unless *Show games on this device* is on.

**D99 The queue: games, not files, kept until they're done** (C9–C13, C22)

- A selection shows its games' total size against the free space (C9), the size of the
  media kinds chosen included. *Add to device* first shows the media kinds the library has
  for those games, each with its size, from the plan (D93; C22), ticked as last time on
  this device, and *Add 3 games (2.4 GB)*. The choice is remembered per device.
- The queue (`queue.json` in the client's data folder) holds games, each with its plan
  and its state: waiting, downloading (bytes and speed), paused, waiting for the library,
  done, or what went wrong. Files go three at a time (Settings: 1–4) as the stand-in
  fetches them, each resumed, checked and renamed into place (C11); a game's gamelist
  entry and record are written when its last file is in, so ES-DE never lists half a game.
- When the server can't be reached the queue waits ("Waiting for the library") and goes
  on by itself once it's found again; a plan whose files changed in the library since is
  asked for again. Closing the client and opening it later goes on where it stopped.
- **Windows**: closing the window while games are downloading leaves an icon by the clock
  that shows the progress, and the client quits when the queue is done; Android's is the
  notification (D97).
- When it's done: "Restart ES-DE to see the new games" (C17), as ES-DE reads its gamelists
  when it starts.

**D100 *On this device* is every game ES-DE has there** (C18, C20, C21)

- The client looks through ES-DE's ROMs folder, system by system, for the files ES-DE
  would list (each system's extensions, from the server and kept with the catalogue;
  `<name>.m3u/` folders as one game, as ES-DE treats them), with each game's size including
  its media and manual (ES-DE's `downloaded_media/<system>/<kind>/<name>.*`) and when it
  was last played (the gamelist's `lastplayed`).
- **Where each came from**: a game the record lists came from the library; any other is
  identified by the server (`POST /v1/identify`, a ZIP's CRCs from its directory or a CHD's
  SHA-1 from its header, never the whole file), remembered by path, size and modified time,
  so it's asked once. Games the library doesn't know show as *from elsewhere*.
- **Sorting**: by last played (the default), size, name or system; *Make room* (C20), shown
  when a selection is bigger than the free space, ticks the least recently played games
  (never played ones by when they were added, oldest first) until there's room, for the
  user to change and confirm.

**D101 Removing a game safely** (C19, C21, the safety requirement)

- Removing shows what goes (each file, its media and manual, its gamelist entry) and the
  room it frees, and asks first. A BIOS, parent or device set another game on the device
  needs stays (the record's `needed_by`, as the stand-in does). Saves are never touched:
  ES-DE's emulators keep them elsewhere.
- A game *from elsewhere* goes with every file it's made of: a `.cue`'s or `.gdi`'s tracks,
  an `.m3u`'s discs, a `<name>.m3u/` folder, and its media by ES-DE's names. Nothing is
  removed outside ES-DE's folders (the ROMs folder, the ES-DE folder and the media folder);
  a path that leads out of them (`..`, a link) is refused.
- **Set aside, then deleted**: a removal first moves its files into `.gamerepo-removing/`
  in the same folder (the ROMs folder, the media folder; a rename on the same drive, so
  it's instant), writes `ES-DE/gamerepo/removing.json`, updates the gamelists and the
  record, and then deletes what it set aside. A removal cut short (the battery, a crash) is
  found on the next start and offered *Finish removing* or *Put back*. ES-DE looks only in
  each system's own folder, so it never lists what's set aside.

**D102 Media on their own** (C23)

- A game on the device shows its media kinds, each with its size: one can be removed on its
  own, keeping the game, and one the library has can be added later without downloading
  the game again (a plan asked for with `roms: false`, below).
- For a game *from elsewhere* that the library knows (D100), its media can be added the
  same way; it then shows as from the library and its entry is merged as the client's own.

**D103 One layout for touch, mouse, keys and controllers**

- Library: systems and collections down the side on a PC, a picker at the top on a phone;
  the games as box fronts with title, year and size, scrolling a virtual list (only what's
  in view is in the page, so 100,000 games scroll as 100), with a search across every
  system; a tick on each, and a bar at the bottom with the selection, its size against the
  free space and *Add to device*.
- A game's page (C7): its pictures, description, players, genre, its size on the device
  with the kinds chosen, and whether it's already there.
- Queue: each game with its progress, *Pause*, *Cancel*, and *Pause all*.
- *On this device*: the list of D100 with the free space at the top, ticks, *Remove…* and
  *Make room…*.
- Settings: the library (its name and address, *Pair with another library…*, *Unpair*),
  the ES-DE folder and ROMs folder found (each can be changed, C3; "ES-DE isn't installed
  on this device" when there's none), the media kinds ticked by default, downloads at
  once, the ES-DE custom collection games are put in (none, *On this device*, or the
  collection they came from, C16), the picture cache's size, and the version.
- Every control can be reached with the arrow keys and Enter, in a set order, so a
  controller (D97) moves through the Library and the Queue as a mouse would.

**D104 Releases**

- CI builds the Windows installer as it builds the server's, and the APK on Linux with
  the Android SDK and NDK the runners have. Both go on the same release with the same
  version (`set_version.py` bumps the client too; Android's version code follows it).
- **An APK has to be signed, always with the same key**, or Android refuses each update
  over the last one. The key is kept as two repository secrets (the key file and its
  password); this session can't set secrets, so Jerred adds them once, with a script in
  `tools/` that makes the key on his PC and opens the page to paste it into. Until then CI
  builds and tests the APK but doesn't publish it.
- Installing it on Android needs *Install unknown apps* for the browser or file manager
  that opens it, once; the release notes say so.

**What the server adds**

- `GET /v1/systems?all=1`: every system ES-DE knows (from `platforms.rs`), with its
  folder and extensions and the library's game count (none for most), so *On this device*
  knows what ES-DE lists in a folder the library has no games for.
- `PlanAsk.roms` (default `true`): `false` gives a plan of the media and manuals only, for
  games the device already has (D102).
- `limit` up to 5,000 for a client reading the whole list (D98); a page stays at 500
  otherwise.

**Tests**

- Rust tests in `gamerepo-device`: the catalogue read whole, then by changes, and again
  after a `410`; search and paging in its own copy; the queue kept across a restart and
  resumed, waiting while the server's away and going on when it's back, a game's entry
  written only when its last file is in; *On this device* with games from the library,
  from elsewhere (identified and not) and multi-file games (`.cue`/`.bin`, `.gdi`, `.m3u`
  and `<name>.m3u/` folders); a removal cut short at each step, then finished and put
  back; shared sets kept; nothing outside ES-DE's folders touched (every file outside them
  compared byte for byte and time for time); media removed and added on their own;
  *Make room*'s order.
- Timings: the whole catalogue of 100,000 games read over loopback in under 20 s; a page
  and a search of the device's copy under 50 ms.
- Page test (`tests/client_page.py`): `gamerepo-cli client-ui` with a real server on a
  synthetic library, at a phone's size and a PC's: found by address, paired with the code
  from the control channel, a system browsed, a search, three games ticked against the
  free space, the media kinds chosen, the queue watched to the end, the games in ES-DE's
  folders and gamelists, *On this device* with a game from elsewhere, *Make room*, a
  removal, Settings, and every page moved through with the keyboard alone. Screenshots
  for each.
- Windows CI: builds the client and its installer, runs the page test against the Windows
  server, and drives the real app with WebDriver, as Game Repo's is driven.
- Android CI: builds the APK and runs it on an emulator (x86_64, Android 14) with the
  server on the runner: All files access granted by `adb`, an ES-DE folder made on the
  emulator's storage, the page driven through the web view's debugging socket, a game
  added (its files and gamelist checked with `adb`), the notification seen while it
  downloads, and the game removed. A real device can't be in CI: the first one is
  Jerred's.

**Not in 9b**: the Linux build and the Steam Deck (9c); other frontends, the NAS server,
outside access and handhelds (9d); PC games; BIOS files outside the ROMs folder; save files;
a second library.

**Order of the build**, each a pull request tested before the next: (1) the client's
backend in `gamerepo-device` with the server's additions, tried with the stand-in from the
CLI; (2) the client's pages and the Windows app, with its installer on the release; (3)
the Android build: the plugin, the APK, the emulator test and the signing key.

### Phase 9b notes, part 1: Game Repo Go's backend (built 2026-10-20)

The first of 9b's three pull requests: everything Game Repo Go does below its pages, in
`gamerepo-device`, with the server's additions, tried through the stand-in CLI and tested
end to end against a real server. No pages, window or APK yet (parts 2 and 3).

- **Where**: `desktop/device/src/` gains `catalogue.rs` (D98), `queue.rs` (D99), `scan.rs`
  (D100), `removing.rs` (D101, D102) and `app.rs` (the settings, `Secrets`, and the command
  table `App::call(cmd, args)`). The stand-in's `device add`/`remove` run on the same code
  (`add_planned`, `removing::remove`), so the CLI and the client share one path. The CLI
  runs the client's backend as its pages will: `gamerepo-cli device app <command>
  [<args as JSON>] --to <device folder> --data <client's folder> [--wait]`.
- **The server's additions**: `GET /v1/systems?all=1` gives every ES-DE system but BIOS,
  with its folder and extensions (the platform's own, plus `zip` and `7z` for cartridges
  and arcade, `chd` for discs, `m3u` where a game on several discs gets a folder).
  `PlanAsk.media_only` stands for the design's `roms: false`, so the default stays a whole
  plan; `PlanAsk.named` (a game's key and its file on the device) names its media and
  gamelist entry after the device's file, for a game from elsewhere (D102). `limit` goes
  up to 5,000; a page is still 500 unless asked.
- **D98 catalogue**: SQLite (WAL) with one connection for reading and one for writing, so
  the pages aren't held up while a sync writes; FTS5 over titles and names. A sync reads
  the systems, collections and their members, then the games whole (5,000 at a time) the
  first time, for another library, or after a `410`, and otherwise only the changes. A
  whole read keeps the details and pictures of games still in the library. Pictures go in
  `pictures/` (500 MB by default; the least recently shown go first, down to nine tenths).
- **D99 queue**: one game at a time, its files three at once (1–4). Where it differs: a
  game's files are kept in `.gamerepo-coming/` in their place (the ROMs folder, the media
  folder) until every one is in and checked, then put in place together, because ES-DE
  lists files, not only entries: a `<name>.m3u/` folder with one disc of two would show.
  A file already whole there is only checked when the game goes on. Pause keeps what came;
  Cancel drops it; Resume also tries again a game that went wrong. A refused connection or
  a cut one is "Waiting for the library", tried again every 15 s; a file damaged twice, or
  gone from the library (404, 410, 416), asks for the plan again once; 401 or 403 says to
  pair again. `queue.json` keeps each game's plan and bytes; a game that was downloading
  when the client closed waits to go on. `added` counts games since the "Restart ES-DE"
  notice was cleared.
- **D100 On this device**: the scan follows ES-DE: each system's extensions (from the
  catalogue's systems), folders starting with a dot skipped, links not followed, what a
  `.cue`, `.gdi` or `.m3u` names part of its game, a `<name>.m3u/` folder one game with its
  playlist first; media from `downloaded_media/<system>/<kind>/` by name; names, plays and
  `lastplayed` from the gamelist. A BIOS or device set the record says games share isn't
  listed as a game. Games from elsewhere are checked in the background, a hundred at a
  time, each remembered by its path, size and modified time: a ZIP by its directory's
  CRCs, a CHD by its header's SHA-1 (versions 3 to 5), a disc by its first cue sheet, and
  a loose file up to 64 MiB read whole for its CRC (a cartridge has nothing smaller to go
  by); a 7z or a larger file isn't asked about and shows as from elsewhere.
- **D101 removing**: `removal` shows the files, those kept (needed by another game, or
  media another game of the same name has), the entries and lines, and the room freed.
  `remove` writes `ES-DE/gamerepo/removing.json` (with the entries as they were written and
  the record's games as they were), sets the files aside in
  `<place>/.gamerepo-removing/<id>/`, takes out empty folders, changes the gamelists,
  collections and record, then deletes what it set aside. One cut short is offered *Finish
  removing* or *Put back* (not once deleting began), and another removal waits for it.
  Every path is checked inside its place after following links. A game from elsewhere goes
  with its files, its media by ES-DE's names, its gamelist entry and any custom collection
  lines naming it.
- **D102 media on their own**: `add_media` queues a media-only plan named for the device's
  file (a game from elsewhere is then taken into the record with its own files);
  `remove_media` is a removal of those kinds only.
- **Settings**: the ES-DE and ROMs folders when they aren't found (C3), the media ticked
  (cover, screenshot and marquee at first, then as last chosen), manuals, files at once,
  the custom collection (none; one for every game the client adds, *Game Repo Go* unless
  renamed; or the collection they came from), the picture cache, *Show every system*,
  *Show games on this device*, the device's name and screen size. The pairing token is
  kept by `Secrets`: a file for the stand-in and the tests here, Windows Credential
  Manager and the Android Keystore in parts 2 and 3.
- **Commands**: `status`, `find`, `pair`, `unpair`, `sync`, `systems`, `collections`,
  `games`, `game`, `picture` (a data URL for now; part 2 may serve pictures itself),
  `prepare`, `add`, `queue`, `pause`, `resume`, `cancel`, `clear_done`, `on_device`,
  `make_room`, `removal`, `remove`, `pending_removal`, `finish_removal`, `put_back`,
  `add_media`, `remove_media`, `settings`, `set_settings`. The page is told `queue`,
  `sync`, `device` or `settings` when one changes.
- **Tests**: `serve/tests/client.rs` against a real server on the synthetic library
  (helpers shared with `devices.rs` in `serve/tests/common/`): the catalogue read whole,
  then its changes, then whole after a `410`, kept away from home; the queue with a game on
  two discs paused part way (nothing in the ROMs folder or gamelist), the client closed
  and opened, cancelled, waiting for the library and going on by itself, and a file gone
  from the library; *On this device* with games from the library, a game from elsewhere
  the library knows, two it doesn't, a removal cut short at each step then put back or
  finished, a BIOS set kept, media removed and added alone, and the library, saves and
  everything outside ES-DE's folders compared byte for byte and time for time. `scan.rs`
  has its own tests (the files a game is made of, ZIP and CHD reading, *Make room*'s
  order). `tests/serve_devices.py` drives a second device through `device app` (paired,
  the library read, a game added through the queue, a game from elsewhere recognised, one
  removed), so Windows CI tries the backend through the CLI too.
- **Timings** (release build, in the session): 100,006 games read over loopback in 4.2 s
  (the target is 20 s); a page at 90,000 of the systems on the device 17 ms, a system's
  page by size 1.4 ms, a search 2.5 ms (the target is 50 ms). A page of some systems first
  took 287 ms: SQLite read the system's index and sorted what it found, so the page now
  reads its order's index (each ends with the system) and passes over the other systems'
  games, and asks for no system at all when ES-DE has a folder for every one with games.

### Phase 9b notes, part 2: Game Repo Go's pages and Windows app (built 2026-10-20)

The second of 9b's pull requests: the pages, the Windows app around them and its installer
on the release. Android is part 3.

- **Where**: `desktop/client/` (`gamerepo-client`, Tauri 2): `main.rs` (the commands `api`,
  which runs `App::call` off the window's thread, `app_info` and `pick_folder`; the
  `picture` protocol; the icon by the clock; "go-changed" events) and `secrets.rs`. The
  pages are `web/client.html`, `client.js`, `client.css` and `web/client/` (`state.js`
  holds the store and the address, `#library`, `#game/<key>`, `#queue`, `#device`,
  `#settings`; `keys.js` the keys and controllers; one file per page or dialog). They take
  `lib/` and `ui/icons.js`/`ui/state.js` from Game Repo and nothing else, so part 3 reuses
  them as they are. `gamerepo-cli client-ui` serves them with the backend over HTTP.
- **Pictures**: the `picture` protocol (`http://picture.localhost/<key>/<kind>?size=` on
  Windows and Android, `picture://localhost/` elsewhere) answers from the device's cache or
  the library, at 256 or 512 pixels by the card's width and the screen, cached by the web
  view for a day. The page never holds data URLs, and the web view never has to trust the
  server's certificate. The `picture` command's data URL stays for the stand-in.
- **The look**: Game Repo's `styles.css` and its colour names only, no colours of its own;
  the menu is a `.sidebar`, the Library's bar a `.cmdbar` and the games `.card`s, so a
  skin's textures reach them. Skin and theme are chosen in Settings › *Look* and kept on
  the device. Phone first: under 760 px wide the menu runs along the bottom and the systems
  are a picker above the games; wider, both are down the side.
- **Library** (C5–C9): the games a page of 200 at a time, and only the cards in view (two
  rows either side) drawn, so a list of 100,000 scrolls as one of twenty. Places: every
  game, a system, a collection or one of its groups. A search covers every system (D103)
  and leaves the place; a place picked ends the search. Orders: title, year and size, each
  either way. Ticks stay while moving between places; the bar under the games shows their
  size against the free space, with *Make room…* when they don't fit.
- **Add to device…** (C10–C12): `prepare` for the ticked games, then the media kinds the
  library has for them with their sizes (ticked as last chosen), manuals, and what doesn't
  fit. A game's page offers *Add to device…* and *See what's on this device*.
- **Queue**: refreshed every 0.7 s while a game is coming (the backend's events say when one
  starts or ends); *Pause*, *Resume* or *Try again*, and *Cancel* for each game and all;
  *Restart ES-DE* once games are in, cleared with *OK*.
- **On this device** (C13–C20): every game ES-DE has, with where it came from (*From the
  library*, *The library has it*, *From elsewhere*, *Checking…*); ordered by played longest
  ago (*Make room*'s order), size, name or system; a game opened shows its media with
  *Remove* each and *Add media…*; *Make room* ticks games for the room asked; *Remove…*
  shows each file and the room it frees first. A removal cut short is a note on every page
  with *Finish removing* and *Put back*.
- **Keys and controllers**: the arrow keys move between controls in the order they're
  drawn, and in the games by card and row (Page Up and Down by a screen); Enter opens,
  Space or X ticks, Esc or Backspace goes back. On Windows the page reads controllers
  itself (the Gamepad API: the d-pad and left stick are the arrows, A Enter, B Back, X
  ticks, the shoulder buttons a page); on Android the plugin will (part 3).
- **The Windows app**: one copy at a time (a second start shows the first's window). The
  pairing is a generic credential "Game Repo Go/server" in Windows Credential Manager, for
  the signed-in user on this computer. Closing the window while games download hides it
  and shows an icon by the clock with their progress (D99): a click opens the window again,
  *Quit* stops them until Game Repo Go next opens, and Game Repo Go closes when they're
  done.
  The installer, `Game Repo Go_<version>_x64-setup.exe`, installs for the user without an
  administrator, so it adds no firewall rule: finding the library on the network may make
  Windows ask once to let Game Repo Go use private networks, and typing the library's
  address works either way.
- **The library at another address** (not in the design): when the library can't be
  reached, Game Repo Go looks for it on the network by its id (a 3 s look, at most once a
  minute, from the queue's wait or a sync) and, when it answers there with the certificate
  pinned at pairing, keeps the new address with the pairing unchanged. So a computer given
  another address by the router needs no new code; one that only announces itself at an
  address the device can't reach is never taken.
- **Tests**: `tests/client_page.py` (27 checks) runs the pages in Chromium at a phone's size
  and a PC's against `client-ui` and a real `serve-devices`: pairing (a wrong code
  refused), the Library with pictures, a system and a search, ticks against the free space,
  *Add to device…* with a kind unticked, the queue to the end and the files and gamelists,
  *On this device*'s sources, *Make room*'s order, removals of a game and of a picture,
  Settings and the look, the keys through every page, and the library away from home; the
  library, saves and ES-DE's settings compared byte for byte afterwards.
  `tests/client_ui.py` drives the built app with WebDriver: paired, a picture through the
  protocol, a download, the credential kept and gone after *Unpair*. Windows CI runs both,
  `cargo test -p gamerepo-client`, and checks the installer is made; the release carries
  it. `tests/serve_devices.py` checks the library found again at another address.

## 8. Working in a cloud session

- crates.io was reachable on 2026-10-16 and the workspace built online. It has been
  blocked in many cloud sessions, so `vendor.yml` pushes the dependencies of every crate
  but the window's to a `crates-vendor` branch whenever one of their `Cargo.toml`s
  changes, and a session can
  build offline from it (CHANGELOG.md, "For agents").
- The Tauri apps build in a cloud session once WebKitGTK is installed (CHANGELOG.md, "For
  agents"), and Game Repo Server's program runs there under Xvfb; the backend is tested
  with `gamerepo-cli serve` + `tests/tauri_shim.js` in the pre-installed Chromium. The
  installers build only in CI.
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
  between versions. Mitigated by doing what the app can itself (it writes CHD, CSO and
  ZSO, so only dolphin-tool and innoextract are used) and finding tools without
  downloading them (D14). CI doesn't run the real tools: the CHD tests compare with
  chdman's own CHDs committed in `tests/fixtures/discs/` and CI runs chdman's `verify`
  on the CHDs the app writes; RVZ is tested with a Python stand-in for dolphin-tool
  (`tests/fixtures/fake_dolphin_tool.py`, started by `fake_dolphin_tool.cmd` on Windows).
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
  As built, Convert writes ZIP (TorrentZip), CHD, CSO, ZSO and RVZ (`platforms::formats`);
  7z and RAR are read, not written, so PC games keep the format they came in.
- **D13 Where documents live:** with the scraped media in the library's own folder,
  `_library/games/<platform>/<Game Name>/docs/`. Scraped content is tied to the ROM's
  hash, and to the game id for PC games (section 4, "Game identity").
- **D14 External tools:** dropped into the app's `tools/` folder, or pointed at in
  Settings; not downloaded by the app.
- **D15 Folders with games:** kept in the library (`library.json`), re-pointed on a
  computer where a folder's path differs (for now; per-computer lists may follow). As
  built, a folder inside the library is stored relative to it, so it follows the library
  anywhere; re-pointing any other folder (`roots::update`) changes its path in
  library.json, so it changes for every computer that opens the library.
- **D16 Scanning ZIPs:** trusted by default (matched by the CRC32 and size they record);
  a Settings option unpacks and hashes every file in them.
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
- **D17 Starter DATs:** downloaded on request from libretro's public copy of the No-Intro
  cartridge DATs and imported for platforms without one; newer DATs imported by hand
  (not shipped in the installer: each group publishes its DATs on its own terms).

- **D18 Devices (taken 2026-10-17):** the devices are muOS, Knulli and ROCKNIX
  handhelds, and Android or Linux running ES-DE. Their device profiles come first
  (3.10); none of them needs a client app of its own (browser, WebDAV and sync cover
  them). For Windows, Android and the Steam Deck, replaced by D84 (2026-10-19).
- **D29 When device profiles come (taken 2026-10-17):** right after Phase 5, as export
  targets for SD cards (Phase 5b), since what they carry is Phase 5's metadata and
  media; serving and syncing over the network stay in Phase 9.

## 11. Open decisions

- **D19, D20** (Phase 2 design): Phase 2 in two halves; originals and duplicates set aside
  on the same drive until emptied. Proposed defaults, followed unless Jerred changes them.

- **D21–D23** (Phase 3 design): collections in the library, systems added by hand, one
  DAT per system when a collection is saved as DATs. Taken 2026-10-17.

- **D24 Add a game and Identify** (Jerred's suggestion, 2026-10-17): one game added on
  its own; one it isn't recognised is identified by searching online sources, brought in
  by ease of integration (3.5). Taken 2026-10-17: Add a game… and Identify with Steam,
  GOG and LaunchBox in Phase 4; ScreenScraper, IGDB and TheGamesDB in Phase 5 (IGDB and
  TheGamesDB moved up from Phase 8); MobyGames was to come in Phase 8 and was dropped by Jerred on 2026-10-18.

- **D25–D28** (Phase 4 design): the PC games folder, import as an Organise change,
  Phase 4's sources, Steam's shortcuts. Proposed defaults, followed in the build
  ("Phase 4 notes"); Jerred asked for Phase 4 to be built and released on 2026-10-17.

- **D30 Scraping and export choices** (Jerred, 2026-10-17): the library keeps the complete
  set of metadata and media, scraped for the library, a collection or a whole DAT; each
  export picks the subset it writes (3.6). Bulk import of documents with a review of the
  pairings, and documents searchable on their own page (3.8). Taken as asked; built in
  Phase 5.
- **D31 Magazines** (Jerred, 2026-10-17, "a later feature"): a magazine collection treated
  like games (3.8). Proposed for Phase 8, after arcade; it reuses the documents page and
  viewer from Phase 5.

- **D32–D35** (Phase 5 design): game folders found by their ROMs' hashes, sources' keys on
  this computer, documents kept by kind beside them, exports keeping what they carry.
  Proposed defaults, followed in the build; Jerred asked for Phase 5 to be built on
  2026-10-17.

- **D36 Layouts the user can change** (Jerred, 2026-10-17): device and frontend export
  layouts are user-configurable, so new firmwares and frontends can be followed: kept as
  JSON specs that can be copied and edited, with a visual editor that shows the expected
  export at folder level, lets the user adjust it and saves a new layout. Taken as asked;
  Phase 5b ("Phase 5b design").
- **D37–D39** (Phase 5b design): layouts as JSON files, built-in ones changed only by
  copying; one export engine; the editor's tree made from the user's own games.
  Proposed defaults, followed in the build unless changed.

- **D40 BIOS files** (Jerred, 2026-10-17): each system's BIOS files kept as a separate
  category, audited for systems missing them, and exported with a set of games for
  another device. Taken as asked; Phase 5b ("Phase 5b design", BIOS files), with D41
  proposed.

D42–D45 are in "Phase 6 design", followed as built ("Phase 6 notes"). D46–D48 are proposed in "Phase 6b design", followed as built ("Phase 6b notes"). New ones get a number here as they come up (next: D105). D96–D104 are in "Phase 9b design" (2026-10-20): D96, the client's name *Game Repo Go*, is Jerred's; D97–D104 he agreed with the design (PR #44), followed as built ("Phase 9b notes, part 1"). D89–D95 (Jerred, 2026-10-19) are in "Phase 9a design". D84–D88 (Jerred, 2026-10-19) are in "Device client requirements". D78–D82 (Jerred, 2026-10-18) are in "Hardening after the v0.5.18 review", and D83 (Jerred, 2026-10-19) after them. D77 (Jerred, 2026-10-18) is in "Import by system". D71–D76 are in "UI pass 3 design" (Jerred's review of 2026-10-18 and Claude's findings), followed as built ("UI pass 3 notes"); D75's export choice for ES-DE is Jerred's. D49–D54 are in "Collections rework design". D55–D56 are in "PC game page design", D55 followed as built and D56 built in Phase 8b as D66. D51 was dropped (Jerred, 2026-10-18); D52 was built as D67. D57–D61 are in "Phase 7 design", followed as built ("Phase 7 notes"). D62–D68 are in "Phase 8 design": D65 chosen by Jerred, the rest proposed with the scope he agreed; D62–D68 followed as built ("Phase 8a notes", "Phase 8b notes", "Phase 8c notes"). D69 is Jerred's list for the UI pass after Phase 8 (2026-10-18), built as "UI pass 2 notes": in Collections, "made from" becomes "source", the games are shown as the Library shows them, Export appears once (not on the page and again in the top bar), and the collections sit in a ribbon of their own on the left, as Model Library's and SCAD Workshop's subcategory views do; a game's page has a collapsible ribbon on the right showing its scraped media; the pages are left-aligned, not centred; Audit gets ways to view a system other than one long list; and BIOS files drops its choice of systems (those with games, with files missing, every system) for every system with a search field. D70 (Jerred, 2026-10-18): ScreenScraper is asked with as many threads as the account allows, and a game's picture sits in a fixed square box ("Scraping threads and the game picture").

---

## 12. Next steps

1. ~~Phase 0: seed the repository from Model Library, drop the 3D pieces, rename the crates,
   add the SQLite index and CI, write CHANGELOG's "For agents".~~ Done (Phase 0 notes, 7).
2. ~~Write `docs/dat-formats.md` and `docs/platforms.md`.~~ First versions written with the
   Phase 1 design; they grow as platforms are added.
3. Phase 1: scanner, DAT import and audit, verified end to end on one real No-Intro set.
   Built (Phase 1 notes); the real-set check is Jerred's.
4. ~~Merge Phase 1 and release v0.1.0.~~ Done 2026-10-16 (PR #4).
5. Phase 2: design written ("Phase 2 design"); 2a built ("Phase 2a notes"), merged and
   released as v0.2.0; 2b built ("Phase 2b notes").
6. Phase 3: design written ("Phase 3 design", D21–D23) and built ("Phase 3 notes").
7. Phase 4: design written ("Phase 4 design", D25–D28) and built ("Phase 4 notes"); 2b, 3
   and 4 merged and released together, as Jerred asked (2026-10-17) as v0.4.0.
8. Phase 5: design written ("Phase 5 design", D32–D35) and built ("Phase 5 notes"). Next:
   Phase 5b, device profiles for SD cards (D18, D29).
9. Phase 5 merged and released as v0.5.0 (2026-10-17). Phase 5b: design written
   ("Phase 5b design", D36–D41), with layouts the user can change (D36) and BIOS
   files (D40), and built ("Phase 5b notes"). Merged on Jerred's go-ahead.
10. Phase 6: design written ("Phase 6 design", D42–D45) and built ("Phase 6 notes"); released as v0.5.2.
11. Phase 6b: metadata by source, variants, delete and scrape again (Jerred,
    2026-10-17); built ("Phase 6b notes", D46–D48), released as v0.5.3.
12. The UI pass (v0.5.4), the Collections rework with bringing games in and Help (D49–D54,
    v0.5.7 and v0.5.8), PC games on the shared game page (D55, v0.5.9), Phase 7 (arcade,
    v0.5.10), Phase 8 (8a v0.5.11, 8b v0.5.12, 8c v0.5.13), UI pass 2 (D69, v0.5.14),
    UI pass 3 with collection groups (D71–D76, v0.5.16), import by system (D77, v0.5.17)
    and a new system's own folder (v0.5.18): all released.
13. Hardening after the v0.5.18 review (D78–D82): the eight batches in that section, in
    order, each a pull request merged on Jerred's word. Batches 1 and 2 are released
    (v0.5.19), batch 3, exports and collections, is v0.5.20; batch 4, the interface, is
    v0.5.21; batches 5 (metadata, PC games and magazines) and 6 (one Import page) are
    v0.5.22; batch 7, the rest of CI and the tests, is v0.5.23; batch 8, groups on devices
    (D81), is done, which finishes the review's batches.
14. Then Phase 9, the device client ("Device client requirements", D84–D88): 9a Game Repo
    Server on Windows, 9b the client for Windows and Android with ES-DE, 9c the Steam
    Deck, 9d later. 9a's design ("Phase 9a design", D89–D95) is agreed (Jerred,
    2026-10-19) and is built in three pull requests; the first, the groundwork in the core,
    is v0.5.27 ("Phase 9a notes, part 1"); the second, the server and the stand-in device
    from the CLI, is built ("Phase 9a notes, part 2"); the third, the tray app, Settings ›
    Serve to devices and the installers, is built ("Phase 9a notes, part 3"), all in
    v0.5.28. Next is 9b, the client for Windows and Android ("Phase 9b design", D96–D104,
    agreed 2026-10-20), in three pull requests: the first, Game Repo Go's backend and the
    server's additions, is built ("Phase 9b notes, part 1"); the second, the pages and the
    Windows app with its installer, is built ("Phase 9b notes, part 2"); Android comes
    next. Later: reading magazines' text
    (PDF, CBZ and CBR) and tagging each issue with the games it covers; videos
    (walkthroughs, reviews, retrospectives) as a section of their own, linked to games.
