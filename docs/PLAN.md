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
  | 7 | MobyGames | every system | an API key; no longer free | 8 |

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
- A dedicated client app on the device is not planned; existing frontends and file
  managers are the clients unless a device needs one (decision D18).

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
| **8** | More targets and providers | RetroArch playlists, Pegasus, Playnite, LaunchBox exports; MobyGames; automatic DAT updates where allowed; Linux build if wanted; magazines (3.8, D31). |
| **9** | Serve to devices | The app as a server on the home network (3.10): browse and download from a browser, a read-only WebDAV view, device profiles for Android, Linux and handheld firmwares, and sync of a collection to a device with verification. Needs Phase 3's collections and export; can move ahead of 4–8. |

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
- **Skins:** Drafting Sheet (default) and Classic. Model Library's Parts Bin and Layer
  Line were themed on 3D printing and weren't brought across; a game-themed skin can be
  added later.
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

- Rescanning 100,000 unchanged files: under 5 s on CI's runner.
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
    (muOS). Other kinds (Pegasus, RetroArch playlists) come with Phase 8's exports.
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
- **D51 isn't built yet.** The subset folders are outside the folders with games, and
  setting aside, Undo and the journal work on the index's files; it needs them to take
  files the index doesn't hold. Until then the old folders can be deleted by hand once the
  collection is made (nothing in it depends on them).

- **D52 (asked by Jerred 2026-10-18, for a later phase).** A collection keeps a history:
  every game added or left out (by hand, by a rules change, or by *Read them again…*) is
  recorded with when and how, shown in a history view on the collection, and each step
  can be undone there.
- **D53 (agreed 2026-10-18).** *Bring these into the library*: in the review of a
  collection from folders, the games the full set doesn't have at all can be brought in:
  each one's files copied (hardlinked on the same drive) into the first folder with games
  for its system, named as the audit DAT names the game (an archive or image as the game,
  a loose file of a one-ROM game as its ROM; a multi-file game keeps its files' names
  for Organise to put right), byte-identical copies once, as a change of kind `bring` with
  the preview and Undo, scanned after it runs. The folders read are never changed (D5).
  Games the full set has in part aren't brought in (Organise or Add a game… does that).
- **D54 (agreed 2026-10-18).** A **Help** page in the menu: the words the app uses, each
  defined once (`TERMS` in `web/ui/help.js`), and what each page in the menu is for with
  its main actions (`PAGES`), with a search; the page test checks every page in the menu
  has a section. Pages use the words as Help defines them: a game's folder under
  `_library/games/` is *its library folder* (not to be confused with a PC game's folder),
  a system's audit DAT is its *full set*.

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

## 8. Working in a cloud session

- crates.io was reachable on 2026-10-16 and the workspace built online. It has been
  blocked in many cloud sessions, so `vendor.yml` pushes the core's dependencies to a
  `crates-vendor` branch whenever `desktop/core/Cargo.toml` changes, and a session can
  build offline from it (CHANGELOG.md, "For agents").
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
- **D15 Folders with games:** kept in the library (`library.json`), re-pointed on a
  computer where a folder's path differs (for now; per-computer lists may follow).
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
  them).
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
  TheGamesDB moved up from Phase 8); MobyGames stays in Phase 8.

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

D42–D45 are in "Phase 6 design", followed as built ("Phase 6 notes"). D46–D48 are proposed in "Phase 6b design", followed as built ("Phase 6b notes"). New ones get a number here as they come up (next: D55). D49–D54 are in "Collections rework design".

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
    2026-10-17); built ("Phase 6b notes", D46–D48), to be merged on Jerred's word.
