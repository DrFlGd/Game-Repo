# Test files

Synthetic files for the tests only: no real game data, ROMs or DATs from
No-Intro, Redump or anyone else are kept here.

- `make_fixtures.py <dir>` writes a small made-up ROM set (correct, misnamed,
  bad-dump, unknown, zipped, duplicate and headered files), DATs describing it
  (Logiqx, a zipped ClrMamePro one, and one whose platform isn't recognised)
  and `expected.json`, what the Game Boy audit should find. `desktop_page.py`
  uses it; it's also the quickest way to try the CLI by hand.
- `torrentzip.py` writes the reference TorrentZip the Rust test
  `torrentzip::the_same_bytes_as_torrentzip` compares against (Python's zlib, the
  same deflate as TorrentZip's own tools) and prints its SHA1, which the test keeps.
- `version.rar` is a test file of the [unrar](https://crates.io/crates/unrar)
  crate (MIT or Apache-2.0), as Model Library uses it: one file, `VERSION`,
  holding `unrar-0.4.0`. RAR can't be written by free tools, so it's committed.
