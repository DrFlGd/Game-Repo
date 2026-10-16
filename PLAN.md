# Game Repo — Project Plan

> Status: **proposal, v0.1** (2026-10-16). Nothing here is final. Section 9 lists the
> decisions that need an owner's call before implementation starts.

## 1. What we are building

A desktop/home-server application for organising a personal video game library:

1. **Audit** large ROM sets against reference `.dat` files (No-Intro, Redump, TOSEC, MAME,
   FBNeo…) and report what is present, missing, misnamed, duplicated or unknown.
2. **Build collections** from those sets using rules (region, language, 1G1R, categories,
   hand-picked lists) and **export** them into the folder layouts that emulator frontends
   expect.
3. **Import PC games** (zipped games, GOG/other installers, ISOs, loose folders) into a
   single, predictable file structure with a manifest per game.
4. **Scrape and store metadata and media** (box art, screenshots, descriptions, release
   data…) from online sources such as ScreenScraper (the service behind Skraper),
   LaunchBox GamesDB, IGDB and others, and write it out in the formats frontends consume.

Everything is driven from one local database that indexes the user's files by hash, so
each feature builds on the same core: *know every file you have, what it is, and what it
should be called.*

### Non-goals (for now)

- Not an emulator or a game launcher. We organise; frontends (ES-DE, RetroArch, Batocera,
  Playnite, LaunchBox…) launch.
- Never downloads game content. DAT files and metadata only.
- No multi-user/cloud sync. Single library, single owner.

---

## 2. Core concepts

| Concept | Meaning |
|---|---|
| **Library root** | A folder the app indexes (e.g. `D:\ROMs`, `/mnt/nas/games`). Many roots allowed. |
| **File** | Any file under a root. Stored with path, size, mtime and hashes (CRC32, MD5, SHA1; SHA256 optional). Archive members (`game.zip/rom.bin`) are files too, with a parent reference. |
| **Platform** | A system such as `snes`, `psx`, `pc`. One canonical id per platform, plus aliases used by DAT groups and frontends (`Nintendo - Super Nintendo Entertainment System`, `snes`, `SNES`…). |
| **DAT** | A reference list for one platform from one source (e.g. *No-Intro SNES 2026-10-01*). Parsed into **DAT games** and **DAT ROMs** (expected name, size, hashes, parent/clone, region/language flags). |
| **Match** | A File ↔ DAT ROM link established by hash. The basis of audit, rename and export. |
| **Game** | The user-facing entity: a title on a platform, possibly with several ROM/disc files, plus metadata and media. A DAT game becomes a Game when matched; PC imports are Games without a DAT. |
| **Collection** | A saved rule set (or manual list) that selects Games; can be exported to a target layout. |
| **Provider** | A metadata/media source with its own matching and rate-limit rules. |
| **Job** | Any long-running operation (scan, audit, export, scrape). Runs in the background, reports progress, can be cancelled and resumed. |

---

## 3. Feature modules

### 3.1 Library scanner & hasher

- Walk every library root; detect new/changed/deleted files by `(path, size, mtime)` so
  rescans are incremental.
- Hash files in parallel with a bounded worker pool (disk-bound, so pool size ≈ 2–4 per
  physical drive, configurable).
- **Archives**: look inside `.zip`, `.7z`, `.rar`. Zip central directories already store
  CRC32 + uncompressed size, so a *quick* audit can match zips against DATs without
  decompressing; *deep* mode decompresses to compute MD5/SHA1 and verify.
- **Headered ROMs**: No-Intro hashes are usually of the *headerless* data. Detect common
  headers (iNES/NES 16 B, FDS 16 B, Atari 7800 A78 128 B, Lynx LNX 64 B, SNES copier
  512 B…) using No-Intro's header definition XMLs and record both headered and headerless
  hashes.
- **Disc images**: `.cue/.bin`, `.iso`, `.gdi`, `.chd`. Redump DATs hash per track; CHDs
  are verified through `chdman` (external tool, optional) or by the SHA1 stored in the
  CHD header.
- Store results in SQLite. The hash cache is keyed on content, so moving/renaming a file
  does not force a rehash.

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
- **Rename / rebuild** to DAT names into a *sorted* output folder. Non-destructive by
  default (copy or hardlink; originals untouched). A "move" mode exists behind an
  explicit confirmation and a dry-run preview.
- Optional canonical zipping of sorted output (TorrentZip-style deterministic archives) so
  files hash identically to other collectors' sets.
- Arcade sets (MAME/FBNeo) need merged/split/non-merged logic, BIOS/device dependencies
  and CHD disks. This is scoped as a later milestone (see §7 and decision D4).

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
- **Export targets** (templates, user-editable):
  - plain folder tree `<Platform>/<Game Name>.<ext>` (DAT names);
  - **ES-DE / EmulationStation / Batocera / RetroBat**: `roms/<system>/…` plus
    `gamelist.xml` and media folders;
  - **RetroArch** playlists (`.lpl`);
  - **Pegasus** `metadata.pegasus.txt`;
  - **Playnite / LaunchBox** import files for PC games.
- Export modes: copy, hardlink (same volume, zero extra space), symlink, or zip/unzip on
  the fly. Every export is verified by re-hashing the written files and recorded so it can
  be diffed and re-synced later.

### 3.5 PC game import

- An **inbox** folder (or drag-and-drop in the UI). Each item is classified:
  archive (`.zip/.7z/.rar`), installer (`setup.exe`, InnoSetup, GOG `setup_*.exe` +
  `.bin` parts, MSI), disc image (`.iso`), or loose folder.
- Identification, best effort in this order:
  1. GOG installers → `innoextract --gog-game-id` gives the GOG product id → GOG product
     API gives title/year/slug;
  2. Steam-style `steam_appid.txt` / depot manifests if present;
  3. filename parsing (`Title (Year) [Source] v1.2.3`) with a user confirmation step;
  4. manual entry.
- Organised into a common structure (names are a proposal, see D6):

  ```
  PC/
    <Title> (<Year>)/
      install/     installer(s) or the game archive, untouched
      extracted/   optional unpacked copy (off by default)
      patches/     updates, DLC, fixes
      extras/      manuals, soundtracks, wallpapers
      media/       box art, screenshots (from the scraper)
      game.json    manifest: title, year, source (GOG/Steam/retail), version,
                   files with sizes + SHA1, provider ids, import date
  ```

- Duplicate detection by hash across the whole library; version bumps (a newer GOG
  installer for an existing game) go into the existing folder and the manifest records
  both.

### 3.6 Metadata & media scraping

- **Providers** behind one interface (`identify(game) → candidates`, `fetch(candidate) →
  metadata + media URLs`), each with its own credentials, rate limiter and cache:
  - **LaunchBox GamesDB** — offline `Metadata.zip` (XML). No account, no rate limit; good
    first provider and a solid fallback.
  - **ScreenScraper.fr** — the service Skraper uses. Supports lookup by ROM hash
    (CRC/MD5/SHA1), which makes it the most accurate for ROM sets. Needs a user account
    and per-application developer credentials; thread limits depend on account level.
  - **IGDB** (Twitch developer app), **TheGamesDB** (API key), **MobyGames** (API key),
    **GOG / Steam store** for PC titles.
- Matching strategy: hash lookup when a provider supports it → exact name match within
  the platform → fuzzy name match with a confidence score. Low-confidence matches queue
  for review instead of being written silently.
- Stored locally: structured metadata (title, alt titles, description, developer,
  publisher, release date, genres, players, rating, provider ids) and media files
  (box front/back/3D, screenshot, title screen, marquee, wheel/logo, video, manual) under
  `media/<platform>/<game>/`. Everything is cached so re-exports never re-download.
- Written out through the export templates above (`gamelist.xml`, Pegasus, ES-DE media
  folders, Playnite/LaunchBox).

### 3.7 User interface

Screens (web UI, see §5):

- **Dashboard** — per-platform completeness bars, recent jobs, inbox count.
- **Library** — searchable/filterable table of Games with media thumbnails.
- **Audit** — per-platform report with Have/Missing/Unknown tabs, fixdat and rename
  actions.
- **Collections** — rule builder with live counts, export wizard.
- **Import inbox** — PC game identification review.
- **Scraper** — provider setup, queue, review of low-confidence matches.
- **Jobs** — progress, logs, cancel/resume.
- **Settings** — roots, platforms/aliases, external tools (`7z`, `chdman`, `innoextract`),
  credentials.

Everything the UI can do, the CLI can do (`gamerepo scan`, `audit`, `export`, `import`,
`scrape`, `serve`), which also keeps the core testable.

---

## 4. Architecture

```
┌────────────────────────────────────────────────────────────┐
│  UI (web app in the browser)        CLI (gamerepo …)       │
├────────────────────────────────────────────────────────────┤
│  API layer  (HTTP + WebSocket for job progress)            │
├────────────────────────────────────────────────────────────┤
│  Core engine                                               │
│   scanner · hasher · archive readers · header detection    │
│   dat parsers · matcher/audit · collections · exporters    │
│   pc-import · providers (metadata) · job runner            │
├────────────────────────────────────────────────────────────┤
│  Storage: SQLite (library.db) + media/ + cache/            │
│  External tools (optional): 7z, chdman, innoextract        │
└────────────────────────────────────────────────────────────┘
```

Principles:

- **Core is a library.** No UI code in it; CLI and API are thin. This makes every feature
  scriptable and unit-testable with fixture files.
- **Hash-first.** All identity flows through content hashes; names are derived, never
  trusted.
- **Non-destructive by default.** The app writes into its own output folders; touching
  the user's originals requires an explicit mode and a dry run.
- **Resumable jobs.** Every long operation checkpoints into the DB so a crash or a closed
  laptop lid doesn't restart a 2 TB hash run.
- **Providers and exporters are plugins** with a small interface, so adding TheGamesDB or
  a new frontend layout is one file.

### Proposed repository layout

```
Game-Repo/
  PLAN.md            this document
  README.md
  docs/              design notes, DAT format notes, provider notes
  packages/
    core/            engine library
    cli/             command-line interface (also hosts `serve`)
    web/             browser UI
  fixtures/          tiny synthetic ROMs/DATs/archives for tests (no real game data)
```

### Data model (first cut)

`library_roots`, `files` (path, size, mtime, crc32, md5, sha1, sha256?, headerless_sha1,
parent_file_id for archive members), `platforms`, `platform_aliases`, `dats`, `dat_games`,
`dat_roms`, `matches`, `games`, `game_files`, `collections`, `collection_rules`,
`collection_items`, `exports`, `export_files`, `metadata` (per game per provider),
`media`, `provider_cache`, `jobs`, `job_log`.

---

## 5. Proposed tech stack

**Recommendation: TypeScript on Node.js**, as a small monorepo.

| Layer | Choice | Why |
|---|---|---|
| Language/runtime | TypeScript, Node.js 22 LTS | One language for engine, API and UI. Strong streaming + worker-thread story for hashing. The best existing open-source ROM manager (Igir) is built on this stack, which proves it copes with multi-TB sets, archives, headers and 1G1R. |
| Database | SQLite via `better-sqlite3` (+ a light migration tool) | Single file, zero setup, fast enough for millions of file rows, easy to back up. |
| Hashing | Node `crypto` + `zlib` CRC32 in worker threads | Native speed without a C toolchain for users. |
| Archives | `yauzl`/`node-stream-zip` (zip), `7zip-bin` + `node-7z` (7z, rar read), `chdman` external (CHD) | Mirrors what Igir/RomVault users already rely on; rar/7z stay optional dependencies. |
| DAT parsing | Streaming XML (`sax`/`fast-xml-parser`) + a small ClrMamePro grammar | MAME listxml is 300 MB+; must stream. |
| API | Fastify + WebSocket | Small, typed, fast. |
| UI | React + Vite + TanStack Table/Query, Tailwind | Large virtualised tables are the main UI problem; this stack handles them well. |
| Packaging | `pnpm` workspace; `gamerepo serve` opens the browser; Docker image for NAS use; optional Electron/Tauri shell later | Runs where the files are (often a NAS or always-on PC), no 200 MB desktop bundle required to start. |
| Tests | Vitest, fixtures of synthetic ROMs and DATs | Real game data never enters the repo. |
| Lint/format | ESLint + Prettier, strict TS | — |

**Alternatives considered**

- *Python + PySide6*: fastest to prototype, `hashlib` is quick, but archive libraries are
  weaker (7z/rar mostly shell out), packaging a Qt app is heavier, and a web UI would mean
  a second language anyway.
- *Rust + Tauri*: best raw performance and a small native app; slower to build features in
  and FFI needed for CHD/rar. Worth revisiting if hashing throughput ever becomes the
  bottleneck (it's disk-bound in practice).
- *C# / .NET + Avalonia*: RomVault's territory, very capable, but least convenient for a
  NAS/Docker deployment and a web UI.

See **D1** and **D2** in §9.

---

## 6. External tools, services and legal notes

- Optional binaries detected at runtime, with install hints in Settings: `7z` (7z/rar),
  `chdman` (from MAME, for CHD), `innoextract` (GOG/InnoSetup installers), `unrar`.
- Credentials (ScreenScraper, IGDB, TheGamesDB, MobyGames) are stored locally in the
  user's config directory, never in the repo.
- The repository will contain **no ROMs, no installers and no copyrighted media**. Test
  fixtures are synthetic (random bytes with known hashes and hand-written DATs).
- DAT files are redistributable from their respective groups' terms; we ship none by
  default and let the user drop them in.

---

## 7. Milestones

Each milestone ends with something usable from the CLI and covered by tests.

| # | Milestone | Deliverable / acceptance |
|---|---|---|
| **M0** | Scaffold | Monorepo, CI (lint + test), SQLite migrations, `gamerepo --help`, this plan merged. |
| **M1** | Scan + DAT + Audit (cartridge systems) | `gamerepo scan <root>` hashes files incl. zip members and headered ROMs; `gamerepo dat import`; `gamerepo audit <platform>` prints Have/Missing/Unknown/Misnamed and writes a fixdat. Verified on a No-Intro set. |
| **M2** | Rename/rebuild + disc systems | Sorted output by DAT name (copy/hardlink/move with dry run); Redump cue/bin and CHD support; 7z/rar reading. |
| **M3** | Collections + export | Rule-based collections with 1G1R; export to plain tree and ES-DE/Batocera layout; export verification and re-sync. |
| **M4** | PC game import | Inbox classification, GOG id detection, common structure + `game.json`, duplicate detection. |
| **M5** | Metadata & media | LaunchBox offline provider, then ScreenScraper (hash lookup); media storage; `gamelist.xml`/Pegasus output; review queue for weak matches. |
| **M6** | Web UI | Dashboard, Library, Audit, Collections, Inbox, Scraper, Jobs, Settings over the API. |
| **M7** | Arcade (MAME/FBNeo) | Merged/split/non-merged audits, BIOS/device deps, CHD disks, sample sets. |
| **M8** | Packaging & polish | Docker image, one-command install, RetroArch/Playnite/LaunchBox exports, more providers (IGDB, TheGamesDB, MobyGames), automatic DAT updates where allowed. |

M1–M3 are the heart of the tool and should be solid before UI work starts; M4 and M5 are
independent of each other and can be reordered.

---

## 8. Risks

- **Scale**: multi-TB libraries and 100k+ file platforms. Mitigated by incremental scans,
  zip CRC fast path, resumable jobs, and virtualised UI tables.
- **Provider terms and rate limits**: ScreenScraper throttles aggressively; IGDB requires
  a Twitch app. Mitigated by caching every response, offline LaunchBox DB as fallback,
  and per-provider limiters.
- **Destructive operations**: a bad rename over a curated set is painful. Mitigated by
  non-destructive default, dry runs, and recording every move so it can be reverted.
- **Format edge cases**: headered dumps, multi-track discs, nested archives, MAME
  merge modes. Mitigated by fixtures per edge case and by leaning on how Igir/RomVault
  handle them.
- **PC game identification** is inherently fuzzy. Mitigated by the review step and the
  manifest keeping the original filename.

---

## 9. Decisions needed

Please weigh in on these; the plan assumes the **bold** default if you don't.

- **D1 — Language/stack**: **TypeScript/Node** (recommended) vs Python vs Rust. (§5)
- **D2 — UI form**: **local web app** (`gamerepo serve`, works on a NAS, wrap in
  Electron/Tauri later) vs native desktop app from day one vs CLI-only until M3.
- **D3 — Where it runs**: **Windows desktop** vs Linux server/NAS (Docker) vs both.
  Affects path handling defaults, hardlink/symlink behaviour and packaging priority.
- **D4 — Arcade scope**: **MAME/FBNeo in M7** (after everything else) vs earlier. Arcade
  adds the most complexity.
- **D5 — Touching originals**: **never by default** (sorted copies/hardlinks in an output
  folder) vs an in-place rename mode as a first-class feature.
- **D6 — PC game folder convention**: `PC/<Title> (<Year>)/install|patches|extras|media`
  as proposed, or your own existing layout to match.
- **D7 — Frontends to target first**: **ES-DE and RetroArch**, then Batocera/RetroBat,
  Pegasus, Playnite, LaunchBox. Tell me what you actually use.
- **D8 — Metadata providers / accounts**: **LaunchBox offline first, ScreenScraper
  second**. Do you already have a ScreenScraper account (and are you OK registering for
  developer credentials), IGDB/Twitch app, or TheGamesDB/MobyGames keys?
- **D9 — License**: **MIT** vs GPL vs unlicensed/private.
- **D10 — Name**: "Game Repo" as the repo name; CLI binary name proposed as `gamerepo`.

---

## 10. Next steps once decisions are in

1. Lock D1–D3, create the monorepo scaffold (M0) with CI.
2. Write `docs/dat-formats.md` and `docs/platforms.md` (alias table) while building the
   DAT importer.
3. Build the scanner/hasher and audit against one real No-Intro set end to end (M1).
