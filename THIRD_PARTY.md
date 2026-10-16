# Third-party code

| What | Used for | License |
| --- | --- | --- |
| [Preact](https://github.com/preactjs/preact) 10.27.2 @ `0dbe636` | Interface components (`web/vendor/preact`; copied by `tools/vendor_preact.py`, only import paths changed) | MIT |
| [htm](https://github.com/developit/htm) @ `d62dcfd` | JSX-like templates without a build step (`web/vendor/htm`) | Apache-2.0 |
| [SQLite](https://sqlite.org) through [rusqlite](https://crates.io/crates/rusqlite) (bundled) | The index of each library, kept on this computer | SQLite: public domain; rusqlite: MIT |
| [serde](https://crates.io/crates/serde), [serde_json](https://crates.io/crates/serde_json), [anyhow](https://crates.io/crates/anyhow), [tokio](https://crates.io/crates/tokio), [sha2](https://crates.io/crates/sha2), [hex](https://crates.io/crates/hex) | The core's plumbing: JSON files, errors, background jobs, ids | MIT or Apache-2.0 (tokio: MIT) |
| [Archivo](https://github.com/google/fonts/tree/main/ofl/archivo) | Interface font of the Classic skin (bundled into the app by `tools/build_desktop.py --fetch-fonts`) | SIL OFL 1.1 |
| [Overpass](https://github.com/google/fonts/tree/main/ofl/overpass) and [Overpass Mono](https://github.com/google/fonts/tree/main/ofl/overpassmono) | Fonts of the Drafting Sheet skin (bundled the same way) | SIL OFL 1.1 |
| [Tauri](https://tauri.app) 2 and its dialog and opener plugins | The desktop app's window | MIT or Apache-2.0 |

The app's shell, interface pieces, tests and build tools started as copies from
[Model Library](https://github.com/DrFlGd/Model-Library) (same owner, MIT).
