# Third-party code

| What | Used for | License |
| --- | --- | --- |
| [Preact](https://github.com/preactjs/preact) 10.27.2 @ `0dbe636` | Interface components (`web/vendor/preact`; copied by `tools/vendor_preact.py`, only import paths changed) | MIT |
| [htm](https://github.com/developit/htm) @ `d62dcfd` | JSX-like templates without a build step (`web/vendor/htm`) | Apache-2.0 |
| [SQLite](https://sqlite.org) through [rusqlite](https://crates.io/crates/rusqlite) (bundled) | The index of each library, kept on this computer | SQLite: public domain; rusqlite: MIT |
| [serde](https://crates.io/crates/serde), [serde_json](https://crates.io/crates/serde_json), [anyhow](https://crates.io/crates/anyhow), [tokio](https://crates.io/crates/tokio), [sha2](https://crates.io/crates/sha2), [hex](https://crates.io/crates/hex) | The core's plumbing: JSON files, errors, background jobs, ids | MIT or Apache-2.0 (tokio: MIT) |
| [zip](https://crates.io/crates/zip), [quick-xml](https://crates.io/crates/quick-xml), [crc32fast](https://crates.io/crates/crc32fast), [md-5](https://crates.io/crates/md-5) and [sha1](https://crates.io/crates/sha1) | Looking inside ZIPs, reading DAT files, and hashing ROMs (CRC32, MD5, SHA1 in one read) | zip, quick-xml: MIT; crc32fast, md-5, sha1: MIT or Apache-2.0 |
| [flate2](https://crates.io/crates/flate2) with [zlib](https://zlib.net) | Writing TorrentZips (zlib's deflate at level 9, so the bytes match other TorrentZip tools) | flate2: MIT or Apache-2.0; zlib: zlib licence |
| [sevenz-rust2](https://crates.io/crates/sevenz-rust2) | Reading 7z archives | Apache-2.0 |
| [unrar](https://crates.io/crates/unrar) and the UnRAR source by Alexander Roshal (in unrar_sys) | Reading RAR archives (it can't make them) | unrar: MIT or Apache-2.0; UnRAR: its own freeware licence, which allows reading and extracting RAR archives but not re-creating the RAR compression algorithm |
| [chd](https://crates.io/crates/chd) (chd-rs), with [claxon](https://crates.io/crates/claxon), [flacenc](https://crates.io/crates/flacenc) and [lzma-rust2](https://crates.io/crates/lzma-rust2) | Reading CHD disc images and writing them (the same data, with the same SHA1, as MAME's chdman) | chd: BSD-3-Clause; claxon, flacenc, lzma-rust2: Apache-2.0 |
| [lz4_flex](https://crates.io/crates/lz4_flex) | Reading and writing ZSO (PSP disc images compressed with LZ4) | MIT |
| [notify](https://crates.io/crates/notify) | Noticing changes in the folders with games while the app is open | CC0-1.0, MIT or Apache-2.0 |
| [fs4](https://crates.io/crates/fs4) and, on Windows, [windows-sys](https://crates.io/crates/windows-sys) | Free space on a drive before a change; telling network drives apart | MIT or Apache-2.0 |
| [ureq](https://crates.io/crates/ureq) (with rustls and webpki-roots) | Downloading starter DATs when asked | MIT or Apache-2.0 (webpki-roots: CDLA-Permissive-2.0 / MPL-2.0 data) |
| [rustls](https://crates.io/crates/rustls) with [ring](https://crates.io/crates/ring), [rcgen](https://crates.io/crates/rcgen) and [mdns-sd](https://crates.io/crates/mdns-sd) | Game Repo Server: its HTTPS with a certificate of its own, and announcing it on the home network so devices find it | rustls: Apache-2.0, ISC or MIT; ring: Apache-2.0 and ISC; rcgen: MIT or Apache-2.0; mdns-sd: Apache-2.0 or MIT |
| [Archivo](https://github.com/google/fonts/tree/main/ofl/archivo) | Interface font of the Classic skin (bundled into the app by `tools/build_desktop.py --fetch-fonts`) | SIL OFL 1.1 |
| [Overpass](https://github.com/google/fonts/tree/main/ofl/overpass) and [Overpass Mono](https://github.com/google/fonts/tree/main/ofl/overpassmono) | Fonts of the Drafting Sheet skin (bundled the same way) | SIL OFL 1.1 |
| [Tauri](https://tauri.app) 2 and its dialog and opener plugins | The desktop app's window | MIT or Apache-2.0 |

The app's shell, interface pieces, tests and build tools started as copies from
[Model Library](https://github.com/DrFlGd/Model-Library) (same owner, MIT).
