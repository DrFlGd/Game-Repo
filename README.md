# Game Repo

A Windows desktop app for organising a personal video game library: audit ROM sets
against `.dat` files, build and export curated collections for ES-DE, import PC games
into a common structure and hand them to Steam, and scrape metadata and media for
everything.

**Start with the plan: [docs/PLAN.md](docs/PLAN.md).** It describes the goals, feature
modules, architecture, the stack (Rust core + Tauri 2 + Preact, seeded from
[Model Library](https://github.com/DrFlGd/Model-Library)), the phases, and the decisions
taken and still open.

## Status

Released and in use (v0.5.54, October 2026). Game Repo audits ROM sets against No-Intro,
Redump and MAME DATs (several per system, with a custom DAT for variants), organises and
converts them (ZIP, CHD with a playlist for a game on several discs, RVZ, CSO) with Undo,
builds collections and exports them to ES-DE, RetroArch, Pegasus, LaunchBox, Playnite or
a handheld's SD card, imports PC games and adds them to Steam, scrapes details, media and
manuals, and keeps documents, magazines and videos. Game Repo Server serves the library
on the home network to Game Repo Go on Windows, Android and the Steam Deck, which
downloads games for ES-DE and installs PC games. All three update themselves.

What's next is in [docs/PLAN.md](docs/PLAN.md) (section 12, "Next steps") and in the
app's About page. Releases (the Windows installers, Game Repo Go's Android APK and Steam
Deck AppImage) are on the [releases page](https://github.com/DrFlGd/Game-Repo/releases).

## Building

The Windows app is built by CI (`.github/workflows/desktop.yml`). Locally:

```
cd desktop && cargo test -p gamerepo-core -p gamerepo-wire -p gamerepo-serve -p gamerepo-device
                                                   # the core, Game Repo Server and their tests
python3 tools/build_desktop.py --out build/desktop # the front end (Game Repo's and the server's)
cd desktop/server && npx @tauri-apps/cli@2 build   # Game Repo Server and its own installer
mkdir -p desktop/src-tauri/binaries && cp desktop/target/release/gamerepo-server.exe desktop/src-tauri/binaries/gamerepo-server-x86_64-pc-windows-msvc.exe
cd desktop/src-tauri && npx @tauri-apps/cli@2 build # the app, with the server beside it (needs Node and WebView2)
cd desktop/client && npx @tauri-apps/cli@2 build   # Game Repo Go for Windows (Android and the Deck build in CI)
```

`CHANGELOG.md` has the map of the code for whoever works on it next.

## Repository layout

```
docs/PLAN.md         project plan, decisions, phase designs and notes
web/                 the front end (no build step)
desktop/core/        Rust core
desktop/wire/        what Game Repo Server and a device say to each other
desktop/serve/       Game Repo Server (serves the library to devices)
desktop/device/      a device's side: Game Repo Go's backend
desktop/steam/       Steam's own files (shortcuts, Proton), for Game Repo and Game Repo Go
desktop/update/      the in-app updates of all three programs
desktop/cli/         gamerepo-cli: the backend, the server and Go's pages, without a window
desktop/src-tauri/   the Tauri 2 app
desktop/server/      Game Repo Server's own program: tray icon, window, installer
desktop/client/      Game Repo Go: Windows, Android and the Steam Deck
desktop/installer/   what both installers add (the server's firewall rules)
tools/, tests/       build and test scripts; synthetic fixtures only, never real game files
```

## Licence

MIT (see [LICENSE](LICENSE)). Third-party code and fonts are listed in
[THIRD_PARTY.md](THIRD_PARTY.md).
