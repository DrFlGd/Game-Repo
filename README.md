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

Planning. No code yet.

## Repository layout (planned)

```
docs/PLAN.md         project plan, decisions, phase designs and notes
web/                 the front end (no build step)
desktop/core/        Rust core and gamerepo-cli
desktop/src-tauri/   the Tauri 2 app
tools/, tests/       build and test scripts; synthetic fixtures only, never real game files
```
