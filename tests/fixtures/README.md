# Test files

Synthetic files for the tests only: no real game data, ROMs or DATs from
No-Intro, Redump or anyone else are kept here.

- `make_fixtures.py <dir>` writes a small made-up ROM set (correct, misnamed,
  bad-dump, unknown, zipped, duplicate and headered files), DATs describing it
  (Logiqx, a zipped ClrMamePro one, and one whose platform isn't recognised)
  and `expected.json`, what the Game Boy audit should find. `desktop_page.py`
  uses it; it's also the quickest way to try the CLI by hand.
