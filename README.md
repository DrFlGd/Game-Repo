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
cd desktop && cargo test -p gamerepo-core          # the core and its tests
python3 tools/build_desktop.py --out build/desktop # the front end
cd desktop && npx @tauri-apps/cli@2 build          # the app (needs Node and WebView2)
```

`CHANGELOG.md` has the map of the code for whoever works on it next.

## Repository layout

```
docs/PLAN.md         project plan, decisions, phase designs and notes
web/                 the front end (no build step)
desktop/core/        Rust core and gamerepo-cli
desktop/src-tauri/   the Tauri 2 app
tools/, tests/       build and test scripts; synthetic fixtures only, never real game files
```

## Licence

MIT (see [LICENSE](LICENSE)). Third-party code and fonts are listed in
[THIRD_PARTY.md](THIRD_PARTY.md).
