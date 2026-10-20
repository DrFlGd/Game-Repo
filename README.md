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

Phase 0: the app's shell. A library can be made, opened, renamed and moved, and its
SQLite index is kept on each computer; the pages for auditing, collections, PC games and
metadata say what their phase will bring. Releases (Windows installer) are on the
[releases page](https://github.com/DrFlGd/Game-Repo/releases).

## Building

The Windows app is built by CI (`.github/workflows/desktop.yml`). Locally:

```
cd desktop && cargo test -p gamerepo-core -p gamerepo-wire -p gamerepo-serve -p gamerepo-device
                                                   # the core, Game Repo Server and their tests
python3 tools/build_desktop.py --out build/desktop # the front end (Game Repo's and the server's)
cd desktop/server && npx @tauri-apps/cli@2 build   # Game Repo Server and its own installer
mkdir -p desktop/src-tauri/binaries && cp desktop/target/release/gamerepo-server.exe desktop/src-tauri/binaries/gamerepo-server-x86_64-pc-windows-msvc.exe
cd desktop/src-tauri && npx @tauri-apps/cli@2 build # the app, with the server beside it (needs Node and WebView2)
```

`CHANGELOG.md` has the map of the code for whoever works on it next.

## Repository layout

```
docs/PLAN.md         project plan, decisions, phase designs and notes
web/                 the front end (no build step)
desktop/core/        Rust core
desktop/wire/        what Game Repo Server and a device say to each other
desktop/serve/       Game Repo Server (serves the library to devices)
desktop/device/      a device's side (the stand-in client)
desktop/cli/         gamerepo-cli: the backend, the server and the stand-in, without a window
desktop/src-tauri/   the Tauri 2 app
desktop/server/      Game Repo Server's own program: tray icon, window, installer
desktop/installer/   what both installers add (the server's firewall rules)
tools/, tests/       build and test scripts; synthetic fixtures only, never real game files
```

## Licence

MIT (see [LICENSE](LICENSE)). Third-party code and fonts are listed in
[THIRD_PARTY.md](THIRD_PARTY.md).
