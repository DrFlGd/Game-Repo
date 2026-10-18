#!/usr/bin/env python3
"""Acceptance test: the desktop app's page against its real backend
(`gamerepo-cli serve`, the same command table as the Tauri app), in Chromium.

    python3 tests/desktop_page.py --cli desktop/target/release/gamerepo-cli --ui build/desktop/ui \\
        --home /tmp/home --out shots [--chromium <path>]

Checks (Phase 0): the first start asks where the library goes and makes only
its _library/ folder there; the menu's places, the pages still to come; the
index is made on this computer (not in the library) and checked as a job;
(Phase 1) adding a folder with games, scanning it, importing DATs (one zipped,
one whose platform is asked), Home's platform bars, every Audit list against
tests/fixtures' expected counts, the fixdat and saved reports, a rescan after a
rename and a deletion, a DAT made from a set, the scan settings, and starter
DATs (from a local stand-in for libretro's copy); (Phase 2a) a file added outside
the app is read by the folder watcher, Rename these with its preview and Undo, Set
aside duplicates, Organise into TorrentZips in another folder, Naming, and the
History with Undo and emptying what was set aside; (Phase 2b) a cue/bin set and an
ISO audited against disc DATs, Organise keeping disc images whole, Convert… to CHD
in place, Extract to cue/bin into a folder, Undo of both, Preferred formats, and
Settings › Tools with a stand-in for dolphin-tool; (Phase 3) collections, their
properties, DATs and exports; (Phase 4) PC games imported from an inbox and identified
from stand-ins for Steam and GOG, unpacked, added to a stand-in Steam, a PC game's page
with its PC section, a detail, picture and document, and Add a game…;
(Phase 5) a source's key, a scrape from a stand-in for TheGamesDB, a game's panel,
documents imported from a folder with a pairing changed, searched and read, and an
export carrying pictures and documents; (Phase 5b) BIOS files from a stand-in for
libretro's list, audited and gathered, the layout editor with a change saved as a new
layout, and an export to a card in it with the BIOS files; (Phase 6) the Library
with thumbnails, a filter with counts kept in the address, the search, selecting,
the table sorted and grouped, a game's page with a detail edited and a file dropped
on it, Next, and the filters saved as a collection; (Phase 6b) each source's answer kept, a box from
another source chosen and a detail used from it on a game's page, Scrape again…,
Delete metadata… with Undo, and Settings › Metadata's sizes;
renaming the library, with Undo; opening a folder that already holds games
leaves them as they were, and switching back from the recent list; the theme
and skin follow the settings and are kept; a library from a newer app opens
read-only; the menu is a drawer in a narrow window; and the library and its
index are the same after the library is moved (library-info, index-info).
Writes <out>/library-info.json and <out>/library.zip for the cross-platform check.
"""
import argparse
import asyncio
import functools
import hashlib
import http.server
import json
import shutil
import struct
import subprocess
import sys
import threading
import time
import urllib.parse
import urllib.request
import zipfile
import zlib
from pathlib import Path

from playwright.async_api import async_playwright

sys.path.insert(0, str(Path(__file__).with_name("fixtures")))
import make_fixtures  # noqa: E402
import make_discs  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--cli", required=True)
ap.add_argument("--ui", required=True)
ap.add_argument("--home", required=True)
ap.add_argument("--out", default="shots")
ap.add_argument("--port", type=int, default=8790)
ap.add_argument("--chromium", help="a Chromium to use instead of Playwright's own")
a = ap.parse_args()
out = Path(a.out)
out.mkdir(parents=True, exist_ok=True)
home = Path(a.home).resolve()
shutil.rmtree(home, ignore_errors=True)
library = home / "My Games"
B = f"http://127.0.0.1:{a.port}/"
SHIM = Path(__file__).with_name("tauri_shim.js")
results, errors = [], []


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(f"{'PASS' if ok else 'FAIL'} {name}{' :: ' + str(detail) if detail != '' else ''}", flush=True)


def starter_source():
    """A local stand-in for libretro's copy of the No-Intro DATs: two systems' DATs."""
    d = home / "starter-source" / "no-intro"
    d.mkdir(parents=True)
    for name, ext, seed in (("Sega - Game Gear", "gg", 21), ("GCE - Vectrex", "vec", 22)):
        (d / f"{name}.dat").write_text(make_fixtures.logiqx(name, "20261001", [
            (f"Starter {ext} (World)", None, [make_fixtures.rom_xml(f"Starter {ext} (World).{ext}", make_fixtures.rom(seed, 512))])]))
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(d.parent))
    handler.log_message = lambda *args: None
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    (home / "config").mkdir(parents=True, exist_ok=True)
    (home / "config/config.json").write_text(json.dumps({"dat_source": f"http://127.0.0.1:{srv.server_port}/no-intro/"}))


def png(w, h, rgb):
    """A real PNG of one colour (the sources' artwork in the tests)."""
    chunk = lambda kind, data: struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + bytes(rgb) * w for _ in range(h))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


COVER, WIDE = png(60, 90, (40, 90, 160)), png(184, 69, (200, 140, 40))


TGDB = [{"id": 501, "game_title": "Alpha", "release_date": "1990-04-21", "overview": "Alpha's overview, from the stand-in.", "players": 2, "developers": [7], "genres": [1]},
        {"id": 502, "game_title": "Beta", "release_date": "1991-06-01", "overview": "Beta's overview.", "players": 1, "developers": [7], "genres": [1]}]


def sources_stand_in():
    """Local stand-ins for Steam's store and GOG's catalogue (Identify's sources), with artwork."""
    steam_games = [{"type": "app", "name": "Alpha Strike", "id": 4000}, {"type": "app", "name": "Zeta Quest", "id": 4001}]
    answers = {
        "/api/appdetails": lambda q: {q["appids"]: {"success": True, "data": {"type": "game", "steam_appid": int(q["appids"]),
            "name": next(g["name"] for g in steam_games if str(g["id"]) == q["appids"]), "developers": ["Stand-in Soft"],
            "short_description": "A game the test made up.", "release_date": {"date": "Mar 3, 2001" if q["appids"] == "4000" else "Feb 24, 1999"}}}},
        "/v1/catalog": lambda q: {"products": [{"id": "1207658930", "slug": "zeta_quest", "title": "Zeta Quest", "releaseDate": "1999.02.24", "coverVertical": f"{base}/img/zeta.jpg"}]
            if "zeta" in q.get("query", "").lower() else []},
        # TheGamesDB (Phase 5's scrape): Alpha and Beta are known, by name for Game Boy
        "/tgdb/Games/ByGameName": lambda q: {"data": {"games": [g for g in TGDB if g["game_title"].lower() == q.get("name", "").lower() and q.get("filter[platform]") == "4"]},
            "include": {"boxart": {"base_url": {"large": f"{base}/tgdb-img/"}, "data": {str(g["id"]): [{"type": "boxart", "side": "front", "filename": f"front/{g['id']}.png"}] for g in TGDB}}}},
        "/tgdb/Games/ByGameID": lambda q: {"data": {"games": [g for g in TGDB if str(g["id"]) == q.get("id")]}},
        "/tgdb/Games/Images": lambda q: {"data": {"base_url": {"large": f"{base}/tgdb-img/"}, "images": {q["games_id"]: [{"type": "boxart", "side": "front", "filename": f"front/{q['games_id']}.png"},
            {"type": "screenshot", "filename": f"shot/{q['games_id']}.png"}]}}},
        "/tgdb/Genres": lambda q: {"data": {"genres": {"1": {"id": 1, "name": "Action"}}}},
        "/tgdb/Developers": lambda q: {"data": {"developers": {"7": {"id": 7, "name": "Stand-in Soft"}}}},
        "/tgdb/Publishers": lambda q: {"data": {"publishers": {}}},
        "/products/1207658930": lambda q: {"id": 1207658930, "title": "Zeta Quest", "slug": "zeta_quest", "release_date": "1999-02-24T00:00:00+0200",
            "images": {"background": f"{base}/img/zeta-bg.jpg", "logo": f"{base}/img/zeta-logo.png"}, "description": {"lead": "<p>Quest <b>on</b>.</p>"}},
    }

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass

        def do_GET(self):
            path, _, query = self.path.partition("?")
            q = dict(urllib.parse.parse_qsl(query))
            if path == "/api/storesearch/":
                words = q.get("term", "").lower().split()
                body = json.dumps({"items": [g for g in steam_games if all(w in g["name"].lower() for w in words)]}).encode()
            elif path in answers:
                body = json.dumps(answers[path](q)).encode()
            elif path.endswith(".png") or path.endswith(".jpg"):
                body = COVER if "600x900" in path or path.endswith("zeta.jpg") else WIDE
            else:
                self.send_response(404)
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("content-length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    base = f"http://127.0.0.1:{srv.server_port}"
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return {"steam_store": base, "steam_api": base, "steam_cdn": f"{base}/cdn", "gog_catalog": base, "gog_api": base,
            "launchbox_zip": f"{base}/Metadata.zip", "launchbox_images": f"{base}/lb", "thegamesdb": f"{base}/tgdb"}


def start_server():
    starter_source()
    cfg = json.loads((home / "config/config.json").read_text())
    cfg["source_urls"] = sources_stand_in()
    (home / "config/config.json").write_text(json.dumps(cfg))
    log = open(out / "serve.log", "w")
    p = subprocess.Popen([a.cli, "serve", "--ui", a.ui, "--home", str(home), "--port", str(a.port)], stdout=log, stderr=log)
    for _ in range(100):
        try:
            urllib.request.urlopen(B, timeout=1)
            return p
        except Exception:
            time.sleep(0.2)
    raise SystemExit("the server didn't start; see serve.log")


def pick(path):
    """The answer to the next folder dialog."""
    req = urllib.request.Request(B + "test/pick", data=json.dumps({"path": str(path)}).encode(), method="POST", headers={"content-type": "application/json"})
    urllib.request.urlopen(req, timeout=5)


def cli_json(*args):
    r = subprocess.run([a.cli, *args], capture_output=True, text=True)
    if r.returncode:
        print(r.stderr)
    return json.loads(r.stdout)


def prefs():
    return json.loads((home / "config/prefs.json").read_text()).get("gr-ui", {})


async def toast_text(pg, pattern, timeout=15000):
    await pg.wait_for_function("(p) => { const t = document.querySelector('#toast'); return t && !t.hidden && new RegExp(p).test(t.textContent); }", arg=pattern, timeout=timeout)
    return await pg.inner_text("#toast")


async def main():
    server = start_server()
    try:
        async with async_playwright() as p:
            b = await p.chromium.launch(executable_path=a.chromium)
            ctx = await b.new_context(viewport={"width": 1400, "height": 900})
            await ctx.add_init_script(path=str(SHIM))
            pg = await ctx.new_page()
            pg.on("pageerror", lambda e: errors.append(str(e)))
            pg.on("dialog", lambda d: asyncio.ensure_future(d.accept()))
            pg.on("console", lambda m: errors.append(f"{m.text} ({m.location.get('url', '')})") if m.type == "error" and "net::ERR_" not in m.text else None)
            await pg.goto(B)
            await pg.wait_for_selector("#first-run", timeout=30000)

            # 1. the first start asks where the library goes, then makes only _library/ there
            asked = await pg.inner_text("#first-run h1")
            coming_off = await pg.locator(".sidebar #coming-pages a").count()
            await pg.screenshot(path=str(out / "00-first-run.png"))
            pick(library)
            await pg.click("#choose-library")
            await pg.wait_for_function("() => document.querySelector('#home-head h1')?.textContent === 'My Games'")
            made = (library / "_library/library.json").is_file() and sorted(x.name for x in library.iterdir()) == ["_library"]
            status = await pg.get_attribute(".status-library", "title")
            check("the first start asks where the library goes and makes only _library/ there",
                  "Where" in asked and coming_off == 0 and made and str(library) == status, (asked, made, status))

            # 2. the index: made on this computer, under the data folder, not in the library (Settings › Library)
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector("#settings-index #index-tables", state="attached")
            tables = await pg.text_content("#index-tables")
            await pg.click('.sidebar a[href="#/"]')
            await pg.wait_for_selector("#home-platforms")
            lib_id = json.loads((library / "_library/library.json").read_text())["id"]
            index_files = sorted(x.name for x in (home / "data/index").iterdir()) if (home / "data/index").is_dir() else []
            in_library = [str(x) for x in library.rglob("*.sqlite*")]
            check("the library's index is made on this computer, not in the library",
                  "index_meta" in tables and f"{lib_id}.sqlite" in index_files and not in_library, (tables, index_files, in_library))
            await pg.screenshot(path=str(out / "00-home.png"))

            # 3. the menu's places, and the pages still to come
            labels = await pg.eval_on_selector_all(".sidebar .nav-label", "els => els.map(e => e.textContent.trim())")
            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-empty")
            audit = await pg.inner_text(".cmdbar h1")
            active = await pg.inner_text(".sidebar .nav-link.active .nav-label")
            lead = await pg.inner_text("#audit-empty")
            check("the menu's places open, with the pages still to come saying what they'll do",
                  labels == ["Game Repo", "Home", "Library", "PC games", "Magazines", "Import", "Audit", "BIOS files", "Metadata", "Documents", "Collections", "Export layouts", "History", "Settings", "Help", "About"]
                  and audit == "Audit" and active == "Audit" and "DAT" in lead and "folders" in lead.lower(), (labels, audit, active, lead[:120]))
            await pg.screenshot(path=str(out / "01-coming.png"))

            # Help (D54): the words the app uses, and a section for every page in the menu
            await pg.click('.sidebar a[href="#/help"]')
            await pg.wait_for_selector("#help-terms dl")
            menu = await pg.eval_on_selector_all(".sidebar a.nav-link[href]", "els => els.map(e => e.getAttribute('href').replace(/^#\\/?/, '') || 'home').filter(r => r !== 'help')")
            sections = await pg.eval_on_selector_all("#help-pages .help-section", "els => els.map(e => e.dataset.route)")
            terms = await pg.eval_on_selector_all("#help-terms [data-term]", "els => els.map(e => e.dataset.term)")
            await pg.screenshot(path=str(out / "01b-help.png"), full_page=True)
            await pg.fill("#help-search", "set aside")
            await pg.wait_for_function("() => document.querySelectorAll('#help-terms [data-term]').length < 10")
            found_terms = await pg.eval_on_selector_all("#help-terms [data-term]", "els => els.map(e => e.dataset.term)")
            check("Help explains the words the app uses and has a section for every page in the menu, and its search narrows both",
                  set(menu) <= set(sections) and {"full-set", "collection", "set-aside", "change", "folders-with-games"} <= set(terms) and "set-aside" in found_terms,
                  (menu, sections, found_terms))
            await pg.fill("#help-search", "")
            # the page's parts sit at its left edge, not centred (D69)
            gap = await pg.evaluate("() => { const p = document.querySelector('#help-page'); return document.querySelector('#help-page > .page-lead').getBoundingClientRect().left - p.getBoundingClientRect().left - parseFloat(getComputedStyle(p).paddingLeft); }")
            check("a page's content is left-justified, not centred", abs(gap) < 1, gap)

            # 4. checking the index from Home's Library menu runs as a job
            await pg.click('.sidebar a[href="#/"]')
            await pg.click("#library-menu")
            await pg.click('#context-menu [data-action="check-index"]')
            said = await toast_text(pg, "index is sound")
            check("checking the index runs and says it's sound", "sound" in said, said)

            # 5. renaming the library, and Undo
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector("#library-name")
            index_path = await pg.inner_text("#index-path")
            await pg.fill("#library-name", "Retro")
            await pg.click(".inline-form button[type=submit]")
            await pg.wait_for_function("() => document.querySelector('.statusbar').textContent.includes('Retro')")
            stored = json.loads((library / "_library/library.json").read_text())["name"]
            await toast_text(pg, "Renamed the library Retro")
            await pg.click("#toast .toast-action")
            await pg.wait_for_function("() => document.querySelector('.status-library').textContent === 'My Games'")
            back = json.loads((library / "_library/library.json").read_text())["name"]
            check("the library is renamed, and Undo puts the name back", stored == "Retro" and back == "My Games", (stored, back))
            check("Settings shows where the index is on this computer", index_path.endswith(f"{lib_id}.sqlite") and str(home / "data") in index_path, index_path)
            await pg.fill("#library-name", "Retro")
            await pg.click(".inline-form button[type=submit]")
            await pg.wait_for_function("() => document.querySelector('.statusbar').textContent.includes('Retro')")
            await pg.screenshot(path=str(out / "02-settings.png"), full_page=True)

            # 6. opening another library (a folder that already holds games), then switching back
            other = home / "Old ROMs"
            (other / "Nintendo - SNES").mkdir(parents=True)
            rom = other / "Nintendo - SNES/Chrono Trigger (USA).sfc"
            rom.write_bytes(b"\x00rom data\xff")
            pick(other)
            await pg.click("#open-library")
            await pg.wait_for_function("() => document.querySelector('#library-path')?.textContent.includes('Old ROMs')")
            kept = rom.read_bytes() == b"\x00rom data\xff"
            top = sorted(x.name for x in other.iterdir())
            check("opening a folder of games adds only the app's folder", kept and top == ["Nintendo - SNES", "_library"], top)
            await pg.wait_for_function("(p) => { const t = document.querySelector('#index-path')?.textContent; return t && t.includes('.sqlite') && t !== p; }", arg=index_path)
            other_index = await pg.inner_text("#index-path")
            await pg.click("#recent-libraries button")
            await pg.wait_for_function("() => document.querySelector('#library-path')?.textContent.endsWith('My Games')")
            name = await pg.input_value("#library-name")
            await pg.wait_for_function("(p) => document.querySelector('#index-path')?.textContent === p", arg=index_path)
            check("a library opened before opens from the list, each with its own index", name == "Retro" and other_index != index_path, (name, other_index))

            # Phase 1. folders with games, scanning, DATs and the audit
            fx = home / "fx"
            make_fixtures.main(fx)
            expected = json.loads((fx / "expected.json").read_text())
            pick(fx / "Roms")
            # (games kept where they are: Import › Folders with games, D73)
            await pg.click('.sidebar a[href="#/import"]')
            await pg.click("#add-root")
            await pg.wait_for_selector("#roots li")
            roots_json = json.loads((library / "_library/library.json").read_text()).get("roots", [])
            check("a folder with games is added and kept in the library", len(roots_json) == 1 and Path(roots_json[0]["path"]) == fx / "Roms", roots_json)
            await pg.goto(B + "#/")
            await pg.click("#home-scan-button")
            said = await toast_text(pg, "Scanned 7 files")
            check("Scan reads every file once, inside ZIPs too", "7 new" in said and "read 7 files" in said, said)
            for f in ("gb.dat", "nes.zip", "odd.dat"):
                pick(fx / "dats" / f)
            await pg.click("#home-import")
            await pg.wait_for_selector("#dat-platform-dialog")
            asked = await pg.inner_text("#dat-platform-dialog")
            # a new system, named here (its id follows from the name)
            await pg.select_option("#dat-platform-0", "__new")
            await pg.fill("#dat-platform-0-name", "Satellaview")
            new_id = await pg.input_value("#dat-platform-0-id")
            await pg.click("#dat-platform-dialog button[type=submit]")
            await toast_text(pg, "Imported Nintendo - Satellaview")
            dat_files = sorted(str(x.relative_to(library / "_library/dats")).replace("\\", "/") for x in (library / "_library/dats").rglob("*.dat"))
            meta = json.loads((library / "_library/library.json").read_text())
            remembered = meta.get("dat_platforms", {})
            systems = {x["id"]: x for x in meta.get("systems", [])}
            check("DATs are imported (a zipped one too) and copied into the library; an unknown one is asked about, its new system added and remembered",
                  "Satellaview" in asked and len(dat_files) == 3 and dat_files[0].startswith("gb/") and remembered.get("Nintendo - Satellaview") == "satellaview"
                  and new_id == "satellaview" and systems.get("satellaview", {}).get("dat_names") == ["Nintendo - Satellaview"], (dat_files, remembered, new_id, systems))
            await pg.wait_for_selector('#home-platforms [data-platform="gb"]')
            bar = await pg.inner_text('#home-platforms [data-platform="gb"]')
            check("Home shows each system against its DAT, with one per title (1G1R)", "3 of 6" in bar and "One per title 3 of 5" in bar, bar)
            await pg.screenshot(path=str(out / "10-home-platforms.png"), full_page=True)

            async def tab_names(tab):
                await pg.click(f'#audit-tabs [data-tab="{tab}"]')
                await pg.wait_for_function("(t) => { const l = document.querySelector('#audit-list'); return l && l.dataset.tab === t && l.getAttribute('aria-busy') === 'false'; }", arg=tab)
                return sorted(await pg.eval_on_selector_all("#audit-list .audit-row", "els => els.map(e => e.dataset.name)"))

            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-page")
            platform_now = await pg.get_attribute("#audit-pick", "data-platform")
            totals = await pg.inner_text("#audit-totals")
            lists = {t: await tab_names(t) for t in ("have", "partial", "missing", "matched", "misnamed", "baddump", "unknown", "duplicates")}
            want = {"have": expected["have_list"], "partial": ["Epsilon (USA)"], "missing": expected["missing_list"], "matched": expected["matched"],
                    "misnamed": expected["misnamed"], "baddump": expected["baddump"], "unknown": expected["unknown"], "duplicates": expected["duplicates"]}
            check("the audit lists what you have, what's missing, misnamed, bad, unknown and duplicated",
                  platform_now == "gb" and "3 of 6" in totals and lists == want, {k: v for k, v in lists.items() if v != want[k]} or totals)
            await pg.click('#audit-tabs [data-tab="misnamed"]')
            await pg.wait_for_selector('#audit-list .audit-row[data-name="beta.gb"]')
            await pg.fill("#audit-search", "beta")
            await pg.wait_for_function("() => document.querySelectorAll('#audit-list .audit-row').length === 1")
            row = await pg.inner_text("#audit-list .audit-row")
            check("a list is searched, and a misnamed file shows the name the DAT gives it", "Beta (Europe).gb" in row, row)
            await pg.screenshot(path=str(out / "11-audit.png"), full_page=True)
            await pg.fill("#audit-search", "")
            # the audit's views (D69): a games list narrowed to a region, sorted, as a table;
            # the systems in the ribbon, and All systems' table of every system's counts
            have_all = await tab_names("have")
            await pg.wait_for_selector("#audit-region")
            regions = await pg.eval_on_selector_all("#audit-region option", "els => els.map(e => e.value).filter(Boolean)")
            await pg.select_option("#audit-region", regions[0])
            await pg.wait_for_selector(f'#audit-list[data-by="{regions[0]}|name"][aria-busy="false"]')
            await pg.wait_for_selector("#audit-list-table tr.audit-row")
            cells = await pg.eval_on_selector_all("#audit-list-table tr.audit-row td:nth-child(2)", "els => els.map(e => e.textContent)")
            await pg.screenshot(path=str(out / "11b-audit-table.png"))
            check("a games list is narrowed to a region, in a table with a column for it",
                  cells and all(regions[0] in c for c in cells) and len(cells) < len(have_all), (regions, cells, have_all))
            await pg.select_option("#audit-region", "")
            await pg.click("#audit-sort")
            await pg.click('#context-menu [data-action="sort-region"]')
            await pg.wait_for_selector('#audit-list[data-by="|region"][aria-busy="false"] #audit-list-table tr.audit-row')
            by_region = await pg.eval_on_selector_all("#audit-list-table tr.audit-row td:nth-child(2)", "els => els.map(e => e.textContent)")
            check("a games list is sorted by region", by_region == sorted(by_region), by_region)
            await pg.click("#audit-sort")
            await pg.click('#context-menu [data-action="sort-name"]')
            side = await pg.eval_on_selector_all("#audit-panel-list .nav-link", "els => els.map(e => e.dataset.id)")
            await pg.click('#audit-panel-list a[data-id=""]')
            await pg.wait_for_selector('#audit-overview tr[data-platform="gb"]')
            row = await pg.inner_text('#audit-overview tr[data-platform="gb"]')
            await pg.screenshot(path=str(out / "11c-audit-all.png"))
            check("the systems are in the ribbon beside the menu, and All systems is a table of each one's counts",
                  side[0] == "" and "gb" in side and "Game Boy" in row and str(len(have_all)) in row and "#/audit/all" in await pg.evaluate("location.hash"), (side, row))
            await pg.click('#audit-overview tr[data-platform="gb"] td[data-tab="misnamed"] a')
            await pg.wait_for_selector('#audit-list[data-tab="misnamed"]')
            check("a count in the table opens that system at that list", "#/audit/gb" in await pg.evaluate("location.hash"))
            await pg.click("#audit-menu")
            await pg.click('#context-menu [data-action="export-fixdat"]')
            said = await toast_text(pg, "fixdat of 3 games")
            await pg.click("#audit-menu")
            await pg.click('#context-menu [data-action="save-report-csv"]')
            await toast_text(pg, "Saved the report")
            fixdats = list((library / "_library/fixdats").glob("*.dat"))
            reports = list((library / "_library/reports").glob("*.csv"))
            csv = reports[0].read_text() if reports else ""
            check("Export fixdat writes what's missing as a DAT, and the report is saved as CSV",
                  len(fixdats) == 1 and "Alpha (Europe)" in fixdats[0].read_text() and "unknown,My Hack.gb" in csv, (said, [x.name for x in fixdats], csv[:200]))

            # a rename and a deletion: the next scan reads only what changed
            gbdir = fx / "Roms/Game Boy"
            (gbdir / "beta.gb").rename(gbdir / "Beta (Europe).gb")
            (gbdir / "Copies/Alpha copy.gb").unlink()
            await pg.click("#audit-scan")
            said = await toast_text(pg, "Scanned 6 files")
            misnamed_after = await tab_names("misnamed")
            dupes_after = await tab_names("duplicates")
            check("a rescan reads only what changed, and the audit follows", "read 1 file" in said and "2 gone" in said and not misnamed_after and not dupes_after,
                  (said, misnamed_after, dupes_after))

            # a DAT made from a set
            await pg.click("#audit-menu")
            await pg.click('#context-menu [data-action="make-dat"]')
            await pg.wait_for_selector("#make-dat-dialog")
            await pg.fill("#make-dat-folder", "Game Boy")
            await pg.fill("#make-dat-name", "My GB set")
            await pg.select_option("#make-dat-platform", "gb")
            await pg.click("#make-dat-dialog button[type=submit]")
            said = await toast_text(pg, "Wrote My GB set")
            made = list((library / "_library/made-dats").glob("My GB set*.dat"))
            check("a DAT is made from a scanned set (ZIPs read for SHA1s) and imported to audit against",
                  len(made) == 1 and "5 games" in said and "imported for Game Boy" in said and made[0].read_text().count("sha1=") == 5, said)
            await pg.select_option("#audit-dat", label="My GB set (" + made[0].stem.split("(")[-1].rstrip(")") + ")")
            await pg.wait_for_function("() => document.querySelector('#audit-have')?.textContent === '5'")
            chosen = json.loads((library / "_library/library.json").read_text()).get("audit_dats", {})
            check("the DAT chosen for a platform is kept for it", chosen.get("gb") == "My GB set", chosen)

            # Phase 2a. the folder watcher, organising with a preview, Undo, naming and the history
            await pg.select_option("#audit-dat", label="Nintendo - Game Boy (Parent-Clone) (20261001-000000)")
            await pg.wait_for_function("() => document.querySelector('#audit-have')?.textContent === '3'")
            (gbdir / "Extra").mkdir()
            shutil.copy(gbdir / "Alpha (USA).gb", gbdir / "Extra/alpha again.gb")
            (gbdir / "Extra/gamma.gb").write_bytes(make_fixtures.rom(3, 32768))
            said = await toast_text(pg, "Scanned what changed", timeout=30000)
            misnamed = await tab_names("misnamed")
            check("files added outside the app are read without a scan (the folder watcher)", "2 new" in said and misnamed == ["alpha again.gb", "gamma.gb"], (said, misnamed))

            await pg.click("#audit-rename")
            await pg.wait_for_selector('#organise-list li[data-op="rename"]')
            step = await pg.inner_text("#organise-list")
            tabs = await pg.inner_text("#organise-tabs")
            await pg.screenshot(path=str(out / "13-rename-preview.png"))
            await pg.click("#organise-dialog button[type=submit]")
            said = await toast_text(pg, "Renamed the misnamed Game Boy files")
            renamed = (gbdir / "Extra/Gamma (Japan).gb").is_file() and not (gbdir / "Extra/gamma.gb").exists()
            misnamed = await tab_names("misnamed")
            await pg.click("#toast .toast-action")
            undone = await toast_text(pg, "Undone")
            back = (gbdir / "Extra/gamma.gb").is_file() and not (gbdir / "Extra/Gamma (Japan).gb").exists()
            check("Rename these shows a preview (a copy of a file named right is left alone), renames in place, and Undo in the message puts it back",
                  "Gamma (Japan).gb" in step and "Left alone 1" in " ".join(tabs.split()) and "1 of 1 step done" in said and renamed and misnamed == ["alpha again.gb"] and back,
                  (step, tabs, said, undone, misnamed))

            await pg.click('#audit-tabs [data-tab="duplicates"]')
            await pg.click("#audit-aside")
            await pg.wait_for_selector('#organise-list li[data-op="aside"]')
            await pg.click("#organise-dialog button[type=submit]")
            said = await toast_text(pg, "Set aside Game Boy duplicates")
            held = [x for x in (fx / "Roms/.gamerepo-set-aside").rglob("*") if x.is_file()]
            dupes = await tab_names("duplicates")
            check("Set aside duplicates keeps the copy that's named right and moves the other out of the way",
                  (gbdir / "Alpha (USA).gb").is_file() and not (gbdir / "Extra/alpha again.gb").exists() and [x.name for x in held] == ["alpha again.gb"] and dupes == [],
                  (said, held, dupes))

            sorted_dir = home / "Sorted"
            pick(sorted_dir)
            await pg.click("#audit-organise")
            await pg.wait_for_selector("#organise-dialog")
            await pg.check('input[name="organise-mode"][value="copy"]')
            await pg.click("#organise-pick")
            await pg.wait_for_function("(p) => document.querySelector('#organise-out')?.value === p", arg=str(sorted_dir))
            await pg.select_option("#organise-format", "zip")
            await pg.screenshot(path=str(out / "14-organise.png"))
            await pg.click("#organise-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            totals = await pg.inner_text("#organise-totals")
            await pg.screenshot(path=str(out / "15-organise-preview.png"))
            await pg.click("#organise-dialog button[type=submit]")
            said = await toast_text(pg, "Sorted Game Boy into")
            zips = sorted(x.name for x in sorted_dir.glob("*.zip")) if sorted_dir.is_dir() else []
            tz = all(b"TORRENTZIPPED-" in (sorted_dir / z).read_bytes()[-40:] for z in zips)
            check("Organise copies a set into TorrentZips in another folder, after a preview, and leaves the originals",
                  "Write ZIP: 4 files" in totals and zips == ["Alpha (USA).zip", "Beta (Europe).zip", "Delta (World).zip", "Gamma (Japan).zip"] and tz and (gbdir / "Delta (World).zip").is_file(),
                  (totals, said, zips))

            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector("#naming-template")
            await pg.fill("#naming-template", "{title} ({region})")
            await pg.wait_for_function("() => document.querySelector('#naming-examples code')?.textContent === 'Super Game (USA, Europe)'")
            await pg.click("#naming-save")
            await toast_text(pg, "Saved how Game Boy games are named")
            naming = json.loads((library / "_library/library.json").read_text()).get("naming", {})
            check("Settings › Naming shows how names come out and keeps the platform's template in the library",
                  naming.get("gb", {}).get("template") == "{title} ({region})", naming)

            # History, a page of its own (D71)
            await pg.click('.sidebar a[href="#/history"]')
            await pg.wait_for_selector("#history li")
            states = await pg.eval_on_selector_all("#history li", "els => els.map(e => e.dataset.state)")
            await pg.screenshot(path=str(out / "16-history.png"), full_page=True)
            await pg.click('#history li:first-child [data-action="undo"]')
            await toast_text(pg, "Undone")
            gone = not sorted_dir.exists() or not any(sorted_dir.iterdir())
            await pg.click("#history-empty")
            await pg.click("#confirm-dialog button[type=submit]")
            await toast_text(pg, "Freed")
            emptied = not any(x.is_file() for x in (fx / "Roms").rglob("*") if ".gamerepo-set-aside" in x.parts)
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.uncheck("#scan-watch")
            await pg.wait_for_timeout(300)
            watch = json.loads((home / "config/config.json").read_text()).get("scan", {}).get("watch")
            check("History lists the changes; the newest is undone there, what was set aside is emptied, and watching can be turned off",
                  states == ["done", "done", "undone"] and gone and emptied and watch is False, (states, gone, emptied, watch))
            await pg.check("#scan-watch")

            # Import: the DATs; Settings: how scans read ZIPs (kept on this computer)
            await pg.click('.sidebar a[href="#/import"]')
            await pg.wait_for_selector("#settings-dats .dat-group")
            groups = await pg.eval_on_selector_all("#settings-dats .dat-group h3", "els => els.map(e => e.textContent)")
            await pg.screenshot(path=str(out / "12b-import.png"), full_page=True)
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.select_option("#scan-zips", "full")
            await pg.wait_for_timeout(300)
            cfg = json.loads((home / "config/config.json").read_text())
            check("Import lists the DATs by system, and Settings keeps the scan settings on this computer",
                  groups == ["Game Boy", "Nintendo Entertainment System", "Satellaview"] and cfg.get("scan", {}).get("full") is True, (groups, cfg.get("scan")))
            await pg.screenshot(path=str(out / "12-settings-games.png"), full_page=True)
            await pg.click('.sidebar a[href="#/import"]')
            await pg.click("#starter-dats")
            await pg.click("#ask-dialog button[type=submit]")
            said = await toast_text(pg, "starter DATs for")
            starters = sorted(x.parent.name for x in (library / "_library/dats").rglob("*.dat") if x.parent.name in ("gamegear", "vectrex"))
            check("Get starter DATs downloads the DATs of the systems that have none", "2 systems" in said and "2 already had one" in said and starters == ["gamegear", "vectrex"], (said, starters))

            # Phase 2b. disc systems: a cue/bin set and an ISO, converted, extracted and put back
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.uncheck("#scan-watch")
            await pg.wait_for_timeout(300)
            made = make_discs.write(home / "disc-src")
            src = home / "disc-src"
            (fx / "Roms/PlayStation").mkdir()
            (fx / "Roms/PSP").mkdir()
            for f in ("Disc Game (USA).cue", "Disc Game (USA) (Track 1).bin", "Disc Game (USA) (Track 2).bin"):
                shutil.copy(src / f, fx / "Roms/PlayStation" / f)
            shutil.copy(src / "Dvd Game (Europe).iso", fx / "Roms/PSP/Dvd Game (Europe).iso")
            ddir = home / "disc-dats"
            ddir.mkdir()
            cd_roms = [make_fixtures.rom_xml(f, (src / f).read_bytes()) for f in ("Disc Game (USA).cue", "Disc Game (USA) (Track 1).bin", "Disc Game (USA) (Track 2).bin")]
            (ddir / "psx.dat").write_text(make_fixtures.logiqx("Sony - PlayStation", "20261001", [("Disc Game (USA)", None, cd_roms)]))
            (ddir / "psp.dat").write_text(make_fixtures.logiqx("Sony - PlayStation Portable", "20261001",
                                                                [("Dvd Game (Europe)", None, [make_fixtures.rom_xml("Dvd Game (Europe).iso", (src / "Dvd Game (Europe).iso").read_bytes())])]))
            for f in ("psx.dat", "psp.dat"):
                pick(ddir / f)
            await pg.click('.sidebar a[href="#/import"]')
            await pg.click("#import-dats")
            await toast_text(pg, "Imported")
            await pg.goto(B + "#/")
            await pg.click("#home-scan-button")
            said = await toast_text(pg, "Scanned")
            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-page")
            await pg.click('#audit-panel-list a[data-id="psx"]')
            await pg.wait_for_function("() => document.querySelector('#audit-have')?.textContent === '1' && document.querySelector('#audit-games')?.textContent === '1'")
            await pg.click("#audit-organise")
            await pg.wait_for_selector("#organise-dialog")
            disc_note = await pg.locator("#organise-disc-note").count()
            no_format = await pg.locator("#organise-format").count() == 0
            await pg.keyboard.press("Escape")
            check("a cue/bin set is audited against its disc DAT, and Organise keeps disc images as they are", disc_note == 1 and no_format, (said, disc_note, no_format))

            await pg.click("#audit-convert")
            await pg.wait_for_selector("#convert-dialog")
            to = await pg.input_value("#convert-to")
            await pg.check('input[name="convert-place"][value="here"]')
            await pg.screenshot(path=str(out / "17-convert.png"))
            await pg.click("#convert-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            totals = await pg.inner_text("#organise-totals")
            await pg.screenshot(path=str(out / "18-convert-preview.png"))
            await pg.click("#convert-dialog button[type=submit]")
            said = await toast_text(pg, "Converted PlayStation to CHD")
            psx = fx / "Roms/PlayStation"
            chd = psx / "Disc Game (USA).chd"
            left = sorted(x.name for x in psx.iterdir() if x.is_file())
            await pg.wait_for_function("() => document.querySelector('#audit-have')?.textContent === '1'")
            check("Convert… turns a cue/bin set into a CHD where it is, sets the originals aside, and the game is still had",
                  to == "chd" and "Convert: 1 file" in totals and chd.is_file() and chd.read_bytes()[:8] == b"MComprHD" and left == ["Disc Game (USA).chd"],
                  (to, totals, said, left))

            extracted = home / "Extracted"
            pick(extracted)
            await pg.click('#audit-tabs [data-tab="have"]')
            await pg.click('#audit-list .audit-row[data-name="Disc Game (USA)"]', button="right")
            await pg.click('#context-menu [data-action="audit-convert-one"]')
            await pg.wait_for_selector("#convert-dialog")
            await pg.select_option("#convert-to", "extract")
            await pg.check('input[name="convert-place"][value="folder"]')
            await pg.click("#convert-pick")
            await pg.wait_for_function("(p) => document.querySelector('#convert-out')?.value === p", arg=str(extracted))
            await pg.click("#convert-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#convert-dialog button[type=submit]")
            said = await toast_text(pg, "Extracted PlayStation into")
            out_files = {x.name: make_discs.hashes(x.read_bytes())["sha1"] for x in extracted.rglob("*") if x.is_file()} if extracted.is_dir() else {}
            want = {k: v["sha1"] for k, v in made.items() if k.startswith("Disc Game")}
            check("Extract writes a CHD back out as the cue/bin Redump lists, byte for byte", out_files == want, (said, out_files))

            await pg.click('.sidebar a[href="#/history"]')
            await pg.wait_for_selector("#history li")
            await pg.click('#history li:first-child [data-action="undo"]')
            await toast_text(pg, "Undone")
            await pg.wait_for_selector('#history li:first-child[data-state="undone"]')
            await pg.click('#history li[data-state="done"] [data-action="undo"]')
            await toast_text(pg, "Undone")
            # (the first Undo's message may still be showing: wait for the second change itself)
            await pg.wait_for_selector('#history li:nth-child(2)[data-state="undone"]')
            back = sorted(x.name for x in psx.iterdir() if x.is_file())
            gone = not extracted.exists() or not any(x.is_file() for x in extracted.rglob("*"))
            check("Undo puts the extracted files and the CHD's originals back as they were",
                  back == ["Disc Game (USA) (Track 1).bin", "Disc Game (USA) (Track 2).bin", "Disc Game (USA).cue"] and gone, (back, gone))

            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector('#formats li[data-platform="psp"] select')
            await pg.select_option('#formats li[data-platform="psp"] select', "zso")
            said = await toast_text(pg, "PlayStation Portable games convert to ZSO")
            fmts = json.loads((library / "_library/library.json").read_text()).get("formats", {})
            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-page")
            await pg.click('#audit-panel-list a[data-id="psp"]')
            await pg.wait_for_function("() => document.querySelector('#audit-have')?.textContent === '1'")
            await pg.click("#audit-convert")
            await pg.wait_for_selector("#convert-dialog")
            await pg.wait_for_function("() => document.querySelector('#convert-to')?.value === 'zso'", timeout=5000)
            offered = await pg.eval_on_selector_all("#convert-to option", "els => els.map(e => e.value)")
            await pg.keyboard.press("Escape")
            check("Settings › Preferred formats keeps the platform's choice in the library, and Convert… offers it first",
                  fmts.get("psp") == "zso" and offered == ["cso", "zso", "chd", "extract"], (fmts, offered))

            fake = Path(__file__).with_name("fixtures") / "fake_dolphin_tool.py"
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector('#tools li[data-tool="dolphin-tool"]')
            pick(fake.resolve())
            await pg.click('#tools li[data-tool="dolphin-tool"] [data-action="choose"]')
            await pg.wait_for_selector('#tools li[data-tool="dolphin-tool"][data-found="yes"]')
            where = await pg.inner_text('#tools li[data-tool="dolphin-tool"] .tool-where')
            kept = json.loads((home / "config/config.json").read_text()).get("tools", {})
            await pg.screenshot(path=str(out / "19-tools.png"), full_page=True)
            await pg.click('#tools li[data-tool="dolphin-tool"] [data-action="clear"]')
            await pg.wait_for_selector('#tools li[data-tool="dolphin-tool"] [data-action="clear"]', state="detached")
            cleared = "dolphin-tool" not in json.loads((home / "config/config.json").read_text()).get("tools", {})
            check("Settings › Tools keeps where dolphin-tool is on this computer, shows what it found, and clears it",
                  Path(kept.get("dolphin-tool", "")) == fake.resolve() and str(fake.name) in where and cleared, (kept, where, cleared))
            await pg.check("#scan-watch")

            # Phase 3. Settings › Systems, the DAT question with a system added there, a collection made,
            # changed by hand, saved as DATs, exported to ES-DE, updated and checked
            await pg.wait_for_selector("#settings-systems")
            await pg.click("#system-add")
            await pg.fill("#system-name", "Acme Gizmo")
            await pg.fill("#system-dats", "Acme - Gizmo")
            await pg.fill("#system-exts", "gzm")
            await pg.select_option("#system-media", "cd")
            await pg.click("#system-dialog button[type=submit]")
            await pg.wait_for_selector('#systems-added li[data-system="acmegizmo"]')
            row = await pg.inner_text('#systems-added li[data-system="acmegizmo"]')
            meta = json.loads((library / "_library/library.json").read_text())
            gizmo = next((x for x in meta.get("systems", []) if x["id"] == "acmegizmo"), {})
            check("Settings › Systems adds a system to the library, beside the built-in ones",
                  "Acme Gizmo" in row and gizmo.get("exts") == ["gzm"] and gizmo.get("media") == "cd" and "Satellaview" in await pg.inner_text("#systems-added"), (row, gizmo))
            await pg.screenshot(path=str(out / "20-systems.png"), full_page=True)
            widgets = home / "widgets.dat"
            widgets.write_text('<?xml version="1.0"?>\n<datafile><header><name>Homebrew - Widgets</name><version>1</version></header>\n'
                               '<game name="Widget"><description>Widget</description><rom name="widget.gzm" size="3" crc="12345678"/></game>\n</datafile>\n')
            pick(widgets)
            await pg.click('.sidebar a[href="#/import"]')
            await pg.click("#import-dats")
            await pg.wait_for_selector("#dat-platform-dialog")
            offered = await pg.eval_on_selector_all("#dat-platform-0 option", "els => els.map(e => e.value)")
            await pg.select_option("#dat-platform-0", "acmegizmo")
            await pg.click("#dat-platform-dialog button[type=submit]")
            await toast_text(pg, "Imported Homebrew - Widgets for Acme Gizmo")
            remembered = json.loads((library / "_library/library.json").read_text()).get("dat_platforms", {})
            check("the DAT question offers every system, added ones too, or a new one",
                  "acmegizmo" in offered and "gb" in offered and offered[-1] == "__new" and remembered.get("Homebrew - Widgets") == "acmegizmo", (offered[-3:], remembered))

            # the collection's games are the Library's tiles (D69): its DAT names from their keys
            async def coll_names():
                await pg.wait_for_function("() => document.querySelector('#collection-result')?.getAttribute('aria-busy') === 'false' && document.querySelector('#lib-games')")
                await pg.wait_for_timeout(300)
                return await pg.eval_on_selector_all("#lib-games [data-key]", "els => els.map(e => e.dataset.key.split('/').slice(1).join('/'))")

            async def leave_out(name):
                await pg.click(f'#lib-games [data-key="gb/{name}"]')
                await pg.wait_for_selector("#bar-leave-out")
                await pg.click("#bar-leave-out")

            async def sort_by(what):
                await pg.click("#collection-sort")
                await pg.click(f'#context-menu [data-action="sort-{what}"]')

            await pg.click('.sidebar a[href="#/collections"]')
            await pg.wait_for_selector("#collections-empty")
            await pg.click("#collections-first")
            ways = await pg.eval_on_selector_all("#collection-way .way", "els => els.map(e => e.dataset.way)")
            await pg.fill("#collection-name", "Best Of")
            await pg.click("#collection-new-dialog button[type=submit]")
            await pg.wait_for_selector("#collection-rules")
            check("New collection offers by rules, from folders or from a DAT, and one by rules opens on its rules", ways == ["rules", "folders", "dat"], ways)
            await pg.check('#rule-platforms input[value="gb"]')
            await pg.wait_for_function("() => +document.querySelector('#collection-games')?.textContent > 0")
            await pg.click('#collection-view [data-view="games"]')
            await pg.wait_for_function("() => document.querySelector('#collection-systems') === null && document.querySelector('#lib-games')?.dataset.total > 0")
            names = sorted(await coll_names())
            await pg.click('#collection-view [data-view="rules"]')
            games_now = lambda: pg.evaluate("() => +document.querySelector('#collection-games').textContent")
            await pg.select_option("#rule-include", "all")
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent > n", arg=len(names))
            one_each = await games_now()
            await pg.uncheck("#rule-1g1r")
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent > n", arg=one_each)
            every = await games_now()
            stored = [json.loads(f.read_text()) for f in (library / "_library/collections").glob("*.json")]
            check("a collection is made from rules (the games you have or every game, one per title unless unticked) and kept in the library",
                  names and len(names) < one_each < every and len(stored) == 1 and stored[0]["rules"]["platforms"] == ["gb"]
                  and stored[0]["rules"]["one_per_title"] is False and stored[0]["rules"]["include"] == "all", (names, one_each, every, stored))
            await pg.check("#rule-1g1r")
            await pg.select_option("#rule-include", "have")
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n", arg=len(names))
            await pg.click('#collection-view [data-view="games"]')
            await pg.wait_for_selector(f'#lib-games [data-key="gb/{names[0]}"]')
            await leave_out(names[0])
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n - 1", arg=len(names))
            await pg.click('#collection-tabs [data-tab="out"]')
            await pg.wait_for_selector(f'#collection-out .audit-row[data-name="{names[0]}"] [data-action="put-back"]')
            stored = json.loads(next((library / "_library/collections").glob("*.json")).read_text())
            check("a game is left out by hand, and listed with Put back (changes made quickly are all kept)",
                  stored["removed"] == [f"gb/{names[0]}"] and stored["rules"]["one_per_title"] is True and stored["rules"]["include"] == "have", (stored["removed"], stored["rules"]))
            # its history (D67): each change a step, newest first; the leave-out undone, then made again
            await pg.click('#collection-view [data-view="history"]')
            await pg.wait_for_selector("#collection-history-list li")
            hows = await pg.eval_on_selector_all("#collection-history-list li", "els => els.map(e => e.dataset.how)")
            first_text = await pg.inner_text("#collection-history-list li:first-child")
            await pg.screenshot(path=str(out / "21c-collection-history.png"))
            await pg.click('#collection-history-list li[data-how="hand"] [data-action="undo"]')
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n", arg=len(names))
            await pg.wait_for_selector('#collection-history-list li:first-child[data-how="undo"]')
            undone = await pg.get_attribute('#collection-history-list li[data-how="hand"]', "data-undone")
            check("a collection's History lists each change newest first (by hand, its rules, made), and Undo puts the game left out back",
                  hows[0] == "hand" and "rules" in hows and hows[-1] == "made" and names[0] in first_text and undone == "yes", (hows, first_text, undone))
            await pg.click('#collection-view [data-view="rules"]')
            check("a collection takes its games' updates and DLC by default", await pg.is_checked("#rule-addons"))
            await pg.click('#collection-view [data-view="games"]')
            await pg.click('#collection-tabs [data-tab="in"]')
            await pg.wait_for_selector(f'#lib-games [data-key="gb/{names[0]}"]')
            await leave_out(names[0])
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n - 1", arg=len(names))
            await pg.screenshot(path=str(out / "21-collection.png"), full_page=True)
            kept = names[1:]

            # its properties: games, systems, size on disk; a size limit; the biggest games first
            # as a table, as the Library's: each game's size
            await pg.click('#collection-look [data-view="table"]')
            await pg.wait_for_function("(n) => document.querySelectorAll('#lib-games .lib-row[data-key]').length === n", arg=len(kept))
            each = await pg.eval_on_selector_all("#lib-games .lib-row[data-key]", "els => els.map(e => +e.dataset.bytes)")
            await pg.click('#collection-view [data-view="size"]')
            await pg.wait_for_selector("#collection-props")
            disk = await pg.evaluate("() => +document.querySelector('#prop-size').dataset.bytes")
            props = await pg.inner_text("#collection-props")
            check("Properties shows the games, systems and size on disk (the sum of its games')",
                  disk > 0 and disk == sum(each) and str(len(kept)) in props and "Systems" in props, (disk, each, props))
            await pg.select_option("#collection-limit", "other")
            await pg.fill("#collection-limit-gb", "0.0000001")
            await pg.press("#collection-limit-gb", "Tab")
            await pg.wait_for_selector('#collection-fit[data-over="true"]')
            stored = json.loads(next((library / "_library/collections").glob("*.json")).read_text())
            check("a size limit is kept with the collection, and one too small says by how much it's over",
                  stored.get("size_limit") == 100 and "Over by" in await pg.inner_text("#collection-fit"), stored.get("size_limit"))
            await pg.screenshot(path=str(out / "21b-collection-properties.png"))
            over_line = await pg.inner_text("#collection-fit-line")
            await pg.click('#collection-view [data-view="games"]')
            await sort_by("size")
            await pg.wait_for_selector('#collection-sort[data-sort="size"]')
            await pg.wait_for_timeout(400)
            by_size = await pg.eval_on_selector_all("#lib-games .lib-row[data-key]", "els => els.map(e => +e.dataset.bytes)")
            check("the games can be listed biggest first, and the line above says it's over its limit",
                  by_size == sorted(by_size, reverse=True) and len(by_size) == len(kept) and "over its" in over_line, (by_size, over_line))
            await sort_by("title")
            await pg.click('#collection-look [data-view="grid"]')

            await pg.click("#collection-dats")
            await pg.click("#save-dats-dialog button[type=submit]")
            await toast_text(pg, "Saved 1 DAT of Best Of")
            dat = library / "_library/collection-dats/Best Of/Best Of - Game Boy.dat"
            text = dat.read_text() if dat.exists() else ""
            check("Save as DATs writes one Logiqx DAT per system with the collection's games",
                  text.count("<game ") == len(kept) and all(f'name="{n}"' in text for n in kept) and "<author>Game Repo</author>" in text, text[:300])

            roms, esde = home / "ROMs", home / "ES-DE"
            esde.mkdir(exist_ok=True)
            await pg.click("#collection-export")
            await pg.wait_for_selector("#export-dialog")
            await pg.wait_for_selector('#export-layout option[value="knulli"]', state="attached")
            layout_first = await pg.input_value("#export-layout")
            offered = await pg.eval_on_selector_all("#export-layout option", "els => els.map(e => e.value)")
            check("Export offers RetroArch, Pegasus, LaunchBox and Playnite beside ES-DE, the devices and a plain folder",
                  all(x in offered for x in ["esde", "knulli", "muos", "retroarch", "pegasus", "launchbox", "playnite", "folder"]), offered)
            await pg.fill("#export-place-roms", str(roms))
            await pg.fill("#export-place-esde", str(esde))
            over = await pg.query_selector("#export-over")
            check("Export says when the collection is over its size limit, and starts from the ES-DE layout", over is not None and "more than its size limit" in await over.inner_text() and layout_first == "esde", layout_first)
            await pg.click("#export-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            totals = await pg.inner_text("#organise-totals")
            await pg.screenshot(path=str(out / "22-export-preview.png"))
            await pg.click("#export-dialog button[type=submit]")
            said = await toast_text(pg, "Exported Best Of")
            exported = sorted(x.name for x in (roms / "gb").iterdir()) if (roms / "gb").is_dir() else []
            gl_path = esde / "gamelists/gb/gamelist.xml"
            gamelist = gl_path.read_text() if gl_path.exists() else ""
            cfg = (esde / "collections/custom-Best Of.cfg").read_text() if (esde / "collections/custom-Best Of.cfg").exists() else ""
            check("Export puts the collection in ES-DE's ROMs folder by system, with a gamelist and a custom collection, after a preview",
                  f"{len(kept)} games" in totals and len(exported) == len(kept) and gamelist.count("<game>") == len(kept) and len(cfg.splitlines()) == len(kept),
                  (totals, said, exported, gamelist[:200], cfg))

            await pg.click('#collection-tabs [data-tab="out"]')
            await pg.click(f'#collection-out .audit-row[data-name="{names[0]}"] [data-action="put-back"]')
            await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n", arg=len(names))
            await pg.click('#collection-tabs [data-tab="in"]')
            await pg.click('#collection-view [data-view="exports"]')
            await pg.wait_for_selector('#exports li[data-target="esde"]')
            await pg.click('#exports li[data-target="esde"] [data-action="update"]')
            await pg.wait_for_selector("#organise-totals")
            update = await pg.inner_text("#organise-totals")
            await pg.click("#export-dialog button[type=submit]")
            said = await toast_text(pg, "Updated the export of Best Of")
            exported_after = sorted(x.name for x in (roms / "gb").iterdir())
            check("Update adds the game put back and leaves those there already",
                  f"{len(kept)} already in place" in update and set(exported) < set(exported_after) and len(exported_after) == len(names)
                  and gl_path.read_text().count("<game>") == len(names), (update, said, exported_after))
            await pg.click('#exports li[data-target="esde"] [data-action="check"]')
            said = await toast_text(pg, "as they should be")
            check("Check reads the export back against the DATs", f"All {len(exported_after)} files are as they should be" in said, said)
            await pg.click('#collection-view [data-view="size"]')
            await pg.select_option("#collection-limit", "32")
            await pg.wait_for_selector('#collection-fit[data-over="false"]')
            stored = json.loads(next((library / "_library/collections").glob("*.json")).read_text())
            check("a card's size is its label's (32 GB card: 32,000,000,000 bytes), and the collection fits",
                  stored.get("size_limit") == 32_000_000_000 and "to spare" in await pg.inner_text("#collection-fit"), stored.get("size_limit"))
            await pg.click('#collection-view [data-view="exports"]')
            await pg.screenshot(path=str(out / "23-exports.png"), full_page=True)

            # the Collections rework (D49, D50): a collection from a folder of a subset kept
            # elsewhere, matched by hash; from another tool's DAT; Export these…
            subset = home / "Subsets" / "Handheld picks"
            (subset / "more").mkdir(parents=True)
            seen, picks_ = set(), []
            for f in sorted((fx / "Roms").rglob("*.gb")):
                if ".gamerepo-set-aside" in f.parts or f.name == "alpha-eu.gb":
                    continue
                h = hashlib.sha1(f.read_bytes()).hexdigest()
                if h not in seen:
                    seen.add(h)
                    picks_.append(f)
                if len(picks_) == 2:
                    break
            shutil.copy(picks_[0], subset / "first pick.gb")
            shutil.copy(picks_[1], subset / "more" / "second.gb")
            (subset / "readme.txt").write_text("my picks")
            await pg.click("#collection-panel-new")
            await pg.check('#collection-way input[value="folders"]')
            pick(subset)
            await pg.click("#new-add-folder")
            await pg.wait_for_selector("#new-folders li")
            await pg.click("#collection-new-dialog button[type=submit]")
            await pg.wait_for_selector("#subset-review", timeout=30000)
            found = await pg.eval_on_selector_all("#subset-systems li", "els => els.map(e => [e.dataset.name, e.dataset.status])")
            unknown = await pg.eval_on_selector_all("#subset-unknown li", "els => els.map(e => e.dataset.path)")
            named = await pg.input_value("#collection-name")
            await pg.screenshot(path=str(out / "23b-collection-from-folders.png"))
            check("From folders reads a folder kept elsewhere and finds its games in the full set by hash, whatever the files are called",
                  len(found) == 2 and all(st == "have" for _, st in found) and unknown == ["Handheld picks/readme.txt"] and named == "Handheld picks", (found, unknown, named))
            await pg.click("#subset-review button[type=submit]")
            await toast_text(pg, "Made Handheld picks")
            await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Handheld picks' && +document.querySelector('#collection-games').textContent === 2")
            made = next(x for x in (json.loads(f.read_text()) for f in (library / "_library/collections").glob("*.json")) if x["name"] == "Handheld picks")
            own = library / made["rules"]["from_files"][0] if made["rules"].get("from_files") else None
            # the ribbon beside the menu is read again just after the collection opens (D69)
            await pg.wait_for_function("() => [...document.querySelectorAll('#collection-panel-list .panel-link')].some(e => e.innerText.includes('Handheld picks'))")
            side = await pg.eval_on_selector_all("#collection-panel-list .panel-link", "els => els.map(e => e.innerText)")
            current = await pg.get_attribute('#collection-panel-list .panel-link[aria-current="page"]', "data-id")
            check("it's kept as its own DAT in the library, listed in the ribbon on the left with its source, and its address names it",
                  current == made["id"] and f"#/collections/{made['id']}" in await pg.evaluate("location.hash") and
                  own is not None and own.is_file() and own.read_text().count("<game ") == 2 and made["made_from"]["kind"] == "folders"
                  and any("Handheld picks" in x and "from Handheld picks" in x for x in side), (made.get("rules"), side))
            await pg.screenshot(path=str(out / "23c-collection-made.png"), full_page=True)
            # Export these: one game picked, to a plain folder
            first = found[0][0]
            await pg.click(f'#lib-games [data-key="gb/{first}"]')
            await pg.wait_for_selector("#bar-count")
            picked_n = await pg.inner_text("#bar-count")
            await pg.click("#bar-export-these")
            await pg.wait_for_selector("#export-part")
            await pg.select_option("#export-layout", "folder")
            part = home / "Part"
            await pg.fill("#export-place-out", str(part))
            await pg.click("#export-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#export-dialog button[type=submit]")
            await toast_text(pg, "Exported Handheld picks")
            got = sorted(x.name for x in part.rglob("*") if x.is_file()) if part.is_dir() else []
            rec = next((x for x in (json.loads(f.read_text()) for f in (library / "_library/exports").glob("*.json")) if x.get("name") == "Handheld picks"), {})
            check("Export these… exports only the games picked, from the full set, and the export remembers them",
                  picked_n == "1 selected" and len(got) == 1 and got[0].startswith(first) and rec.get("only") == [f"gb/{first}"], (picked_n, got, rec.get("only")))
            await pg.keyboard.press("Escape")
            # from a DAT another tool made, with names of its own
            gb2 = hashlib.sha1(picks_[1].read_bytes()).hexdigest()
            crc2 = f"{zlib.crc32(picks_[1].read_bytes()):08x}"
            mine = home / "my list.dat"
            mine.write_text(f'<?xml version="1.0"?>\n<datafile><header><name>My list</name></header>'
                            f'<game name="the second one"><rom name="x.gb" size="{picks_[1].stat().st_size}" crc="{crc2}" sha1="{gb2}"/></game>'
                            f'<game name="nobody knows"><rom name="y.gb" size="3" crc="01020304"/></game></datafile>\n')
            await pg.click("#collection-panel-new")
            await pg.check('#collection-way input[value="dat"]')
            await pg.fill("#new-dat-path", str(mine))
            await pg.click("#collection-new-dialog button[type=submit]")
            await pg.wait_for_selector("#subset-review")
            found_dat = await pg.eval_on_selector_all("#subset-systems li", "els => els.map(e => e.dataset.name)")
            unknown_dat = await pg.eval_on_selector_all("#subset-unknown li", "els => els.map(e => e.dataset.path)")
            await pg.click("#subset-review button[type=submit]")
            await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'my list'")
            audit_dats = [x.read_text() for x in (library / "_library/dats").rglob("*.dat")]
            check("From a DAT reads it in place and finds its games by hash, not by name, and it isn't made an audit DAT",
                  len(found_dat) == 1 and found_dat[0] in [n for n, _ in found] and unknown_dat == ["nobody knows"]
                  and not any("<name>My list</name>" in t for t in audit_dats), (found_dat, unknown_dat))
            await pg.click('#collection-view [data-view="rules"]')
            await pg.wait_for_selector("#collection-source")
            source_tab = await pg.inner_text('#collection-view [data-view="rules"]')
            check("a collection made from a DAT says so under Source (not \"made from\")", source_tab.strip() == "Source" and "Source" in await pg.inner_text("#collection-source h2"), source_tab)
            await pg.click("#collection-read-again")
            await pg.wait_for_selector("#subset-review")
            await pg.click("#subset-review button[type=submit]")
            await toast_text(pg, "Read my list again")
            check("Read it again… reads the DAT again and keeps the collection", (await pg.inner_text("#collection-games")) == "1")
            # a game the full set lacks, brought into the library from the folder, named as the DAT says
            wanted = home / "Subsets" / "Wanted"
            wanted.mkdir(parents=True)
            (wanted / "europe one.gb").write_bytes(make_fixtures.rom(11, 32768))
            await pg.click("#collection-panel-new")
            await pg.check('#collection-way input[value="folders"]')
            pick(wanted)
            await pg.click("#new-add-folder")
            await pg.wait_for_selector("#new-folders li")
            await pg.click("#collection-new-dialog button[type=submit]")
            await pg.wait_for_selector("#subset-review", timeout=30000)
            lacking = await pg.eval_on_selector_all("#subset-systems li", "els => els.map(e => [e.dataset.name, e.dataset.status])")
            pointed = await pg.inner_text("#subset-lacking")
            await pg.click("#subset-review button[type=submit]")
            await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Wanted'")
            # collecting copies nothing (D71): the game the library lacks comes in from Import
            await pg.click('.sidebar a[href="#/import"]')
            await pg.click("#import-games")
            pick(wanted)
            await pg.click("#import-add-folder")
            await pg.wait_for_selector("#import-folders li")
            await pg.click("#import-games-dialog button[type=submit]")
            await pg.wait_for_selector("#import-games-review", timeout=30000)
            offered = await pg.eval_on_selector_all("#import-games-systems li", "els => els.map(e => [e.dataset.name, e.dataset.status])")
            await pg.click("#import-games-review button[type=submit]")
            await pg.wait_for_selector("#bring-dialog #organise-totals")
            await pg.screenshot(path=str(out / "23d-bring-in.png"))
            await pg.click("#bring-dialog button[type=submit]")
            said = await toast_text(pg, "Imported 1 game into the library")
            brought = [x for x in (fx / "Roms").rglob("Alpha (Europe).gb") if ".gamerepo-set-aside" not in x.parts]
            await pg.click('.sidebar a[href="#/collections"]')
            await pg.click('#collection-panel-list .panel-link:has-text("Wanted")')
            await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Wanted' && document.querySelector('#collection-have')?.textContent === '1'", timeout=30000)
            check("a collection from folders points at Import for the games the library lacks; Import copies one into its folder with games, named as the DAT says, and scans it",
                  lacking == [["Alpha (Europe)", "missing"]] and "Import" in pointed and offered == [["Alpha (Europe)", "missing"]] and len(brought) == 1 and (wanted / "europe one.gb").is_file(), (lacking, pointed, offered, said, brought))
            await pg.click('#collection-panel-list .panel-link:has-text("Best Of")')
            await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Best Of'")

            # Phase 4: PC games. The inbox (set in Settings), Import… with its review list
            # (one found by its goggame file, one by a search of the sources), the preview,
            # each game's folder, artwork, Unpack and its program, Identify, Add to Steam, and a
            # PC game's page: the shared game page with its PC section (D55)
            inbox = home / "Inbox"
            zeta = inbox / "Zeta Quest (1999)"
            (zeta / "bin").mkdir(parents=True)
            (zeta / "bin/zeta.exe").write_bytes(b"MZ zeta")
            (zeta / "goggame-1207658930.info").write_text(json.dumps({"gameId": "1207658930", "rootGameId": "1207658930", "name": "Zeta Quest",
                "playTasks": [{"category": "game", "isPrimary": True, "path": "bin\\zeta.exe", "type": "FileTask"}]}))
            with zipfile.ZipFile(inbox / "Alpha Strike.zip", "w") as z:
                z.writestr("Alpha Strike/alpha.exe", b"MZ alpha")
                z.writestr("Alpha Strike/data/level1.dat", b"level one")
            steam = home / "Steam"
            (steam / "userdata/22222/config").mkdir(parents=True)
            (steam / "config").mkdir(parents=True)
            (steam / "config/loginusers.vdf").write_text('"users"\n{\n\t"76561197960287950"\n\t{\n\t\t"AccountName"\t\t"tester"\n\t\t"PersonaName"\t\t"Tester"\n\t\t"Timestamp"\t\t"1790000000"\n\t}\n}\n')
            await pg.click("#nav-pc")
            await pg.wait_for_selector("#pc-empty")
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector("#settings-pc-inbox")
            pick(inbox)
            await pg.click('#settings-pc-inbox [data-action="choose"]')
            await pg.wait_for_function("(p) => document.querySelector('#pc-inbox-path')?.textContent === p", arg=str(inbox))
            pick(steam)
            await pg.click("#steam-choose")
            await pg.wait_for_function("(p) => document.querySelector('#steam-path')?.textContent === p", arg=str(steam))
            check("Settings keeps the PC games inbox and Steam's folder, and lists Identify's sources",
                  await pg.eval_on_selector_all("#sources > li", "els => els.map(e => e.dataset.source)") == ["steam", "gog", "screenscraper", "igdb", "thegamesdb", "launchbox"] and str(library / "PC") in await pg.inner_text("#pc-folder-path"), await pg.inner_text("#settings-sources"))
            await pg.click("#nav-pc")
            await pg.click("#pc-first")
            await pg.wait_for_selector("#pc-items li", timeout=30000)
            items = await pg.eval_on_selector_all("#pc-items li", "els => els.map(e => [e.querySelector('.pc-item-title').value, e.dataset.status, e.querySelector('.pc-item-meta').textContent])")
            check("Import… reads the inbox: one game found by its goggame file, the other by a search of the sources",
                  sorted(i[0] for i in items) == ["Alpha Strike", "Zeta Quest"] and all(i[1] == "ready" for i in items)
                  and any("goggame" in i[2] and "gog 1207658930" in i[2] for i in items) and any("steam 4000" in i[2] and "search" in i[2] for i in items), items)
            await pg.screenshot(path=str(out / "24-pc-import.png"))
            await pg.click("#pc-import-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#pc-import-dialog button[type=submit]")
            said = await toast_text(pg, "Import 2 PC games", timeout=30000)
            await pg.wait_for_selector("#pc-list li:nth-child(2)")
            made = sorted(x.name for x in (library / "PC").iterdir()) if (library / "PC").is_dir() else []
            alpha_dir = library / "PC/Alpha Strike (2001)"
            alpha = json.loads((alpha_dir / "game.json").read_text()) if (alpha_dir / "game.json").exists() else {}
            art = list((library / "_library/games/pc").glob("*/media/cover.*"))
            check("each PC game gets its own folder with a game.json, and the artwork the sources offered",
                  made == ["Alpha Strike (2001)", "Zeta Quest (1999)"] and alpha.get("ids") == {"steam": "4000"} and (alpha_dir / "install/Alpha Strike.zip").is_file()
                  and (library / "PC/Zeta Quest (1999)/extracted/bin/zeta.exe").is_file() and len(art) == 1 and (inbox / "Alpha Strike.zip").is_file(), (said, made, alpha, art))
            await pg.click('#pc-list .pc-row:has-text("Alpha Strike")')
            await pg.wait_for_selector('#pc-unpack-section [data-action="unpack"]')
            await pg.click('#pc-unpack-section [data-action="unpack"]')
            await toast_text(pg, "Unpacked")
            await pg.wait_for_selector("#pc-program")
            programs = await pg.eval_on_selector_all("#pc-program option", "els => els.map(e => e.value)")
            await pg.select_option("#pc-program", "extracted/Alpha Strike/alpha.exe")
            await pg.click("#pc-program-form button[type=submit]")
            await toast_text(pg, "Saved the program")
            alpha = json.loads((alpha_dir / "game.json").read_text())
            check("Unpack puts a game's archive in its extracted folder, and its program is chosen from what's there",
                  "extracted/Alpha Strike/alpha.exe" in programs and (alpha_dir / "extracted/Alpha Strike/data/level1.dat").is_file()
                  and alpha["launch"]["program"] == "extracted/Alpha Strike/alpha.exe", (programs, alpha.get("launch")))
            await pg.screenshot(path=str(out / "25-pc-game.png"))
            await pg.click('#pc-list .pc-row:has-text("Zeta Quest")')
            await pg.click("#pc-identify")
            await pg.wait_for_selector('#identify-results li[data-source="gog"]')
            sources = await pg.eval_on_selector_all("#identify-results li", "els => els.map(e => e.dataset.source)")
            await pg.screenshot(path=str(out / "26-identify.png"))
            await pg.click('#identify-results li[data-source="gog"] [data-action="pick"]')
            await pg.wait_for_selector("#identify-dialog", state="detached")
            zeta_json = json.loads((library / "PC/Zeta Quest (1999)/game.json").read_text())
            zeta_meta = library / f"_library/games/pc/{zeta_json['id']}"
            zeta_shared, zeta_gog = {}, {}
            for _ in range(50):
                zeta_shared = json.loads((zeta_meta / "game.json").read_text()) if (zeta_meta / "game.json").exists() else {}
                if zeta_shared.get("identified_by") == "gog":
                    break
                await asyncio.sleep(0.2)
            zeta_json = json.loads((library / "PC/Zeta Quest (1999)/game.json").read_text())
            zeta_gog = json.loads((zeta_meta / "sources/gog.json").read_text()) if (zeta_meta / "sources/gog.json").exists() else {}
            zeta_media = sorted(x.name for x in (zeta_meta / "media").iterdir())
            check("Identify searches the sources in order; the pick keeps its id with the game, its answer kept as the source's, and its artwork",
                  sources == ["steam", "gog"] and zeta_json["ids"].get("gog") == "1207658930" and "description" not in zeta_json.get("details", {})
                  and zeta_gog.get("details", {}).get("description", "").startswith("Quest") and zeta_shared.get("details", {}).get("description", "").startswith("Quest")
                  and zeta_shared.get("identified_by") == "gog" and "cover.png" in zeta_media and "hero.png" in zeta_media,
                  (sources, zeta_json.get("details"), zeta_shared.get("details"), zeta_media))
            await pg.click("#pc-steam")
            await pg.wait_for_selector("#steam-preview")
            preview = await pg.inner_text("#steam-preview")
            await pg.screenshot(path=str(out / "26b-steam.png"))
            await pg.click("#steam-dialog button[type=submit]")
            said = await toast_text(pg, "Wrote 2 shortcuts")
            vdf = (steam / "userdata/22222/config/shortcuts.vdf").read_bytes() if (steam / "userdata/22222/config/shortcuts.vdf").exists() else b""
            grid = sorted(x.name for x in (steam / "userdata/22222/config/grid").iterdir()) if (steam / "userdata/22222/config/grid").is_dir() else []
            check("Add to Steam writes the PC games as non-Steam shortcuts, tagged as the app's, with their artwork in Steam's grid",
                  "2 shortcuts to add" in preview and b"Zeta Quest" in vdf and b"Alpha Strike" in vdf and b"Game Repo" in vdf and any(g.endswith("p.png") for g in grid), (preview, said, grid))
            # Zeta's page: the shared game page, with the PC section; a detail edited, Steam's
            # answer added by Identify…, a picture and a detail taken from another source, and
            # a document dropped on it
            zeta_key = f"pc/{zeta_json['id']}"
            await pg.click('#pc-list .pc-row:has-text("Zeta Quest")')
            await pg.click("#pc-open")
            await pg.wait_for_selector(f'#game-page[data-key="{zeta_key}"][data-kind="pc"] #game-pc #pc-parts')
            parts = await pg.evaluate("() => [!!document.querySelector('#game-pc #pc-program, #game-pc #pc-no-program'), !!document.querySelector('#game-pc #pc-reveal'), document.querySelector('#game-pc-line')?.textContent || '', !!document.querySelector('#game-files')]")
            check("a PC game's page is the game page, with a PC section for its program, its folder and files",
                  parts[0] and parts[1] and "GOG" in parts[2] and not parts[3] and await pg.locator("#game-developer").count() == 1, parts)
            await pg.fill("#game-developer", "Zeta Folk")
            await pg.press("#game-developer", "Tab")
            await pg.wait_for_selector('.game-detail[data-field="developer"][data-edited="yes"]')
            await pg.click("#game-identify-pc")
            await pg.wait_for_selector('#identify-results li[data-source="steam"] [data-action="pick"]')
            await pg.click('#identify-results li[data-source="steam"] [data-action="pick"]')
            await pg.wait_for_selector("#identify-dialog", state="detached")
            await pg.wait_for_selector('#game-variants[data-kind="cover"] button[data-name="steam"][aria-pressed="true"]')
            await pg.click('#game-variants button[data-name="gog"]')
            await pg.wait_for_selector('#game-variants button[data-name="gog"][aria-pressed="true"]')
            row = '.game-detail[data-field="description"]'
            await pg.click(f"{row} .game-detail-others summary")
            await pg.click(f'{row} .game-detail-others [data-source="gog"] [data-action="use"]')
            await pg.wait_for_function("() => document.querySelector('#game-description')?.value.startsWith('Quest')")
            zeta_shared = json.loads((zeta_meta / "game.json").read_text())
            zeta_json = json.loads((library / "PC/Zeta Quest (1999)/game.json").read_text())
            covers = sorted(x.name for x in (zeta_meta / "media/cover").iterdir()) if (zeta_meta / "media/cover").is_dir() else []
            check("on its page a PC game's details and pictures are kept as any game's: each source's answer, a picture from each, Use this, and your edits over them",
                  zeta_shared.get("details", {}).get("developer") == "Zeta Folk" and "developer" in zeta_shared.get("edited", []) and zeta_shared.get("pinned", {}).get("description") == "gog"
                  and zeta_shared.get("chosen", {}).get("cover") == "gog" and len(covers) == 1 and covers[0].startswith("steam.") and (zeta_meta / "sources/steam.json").is_file()
                  and zeta_json["ids"] == {"gog": "1207658930", "steam": "4001"} and zeta_shared.get("identified_by") == "steam",
                  (zeta_shared.get("details"), zeta_shared.get("chosen"), zeta_shared.get("pinned"), covers, zeta_json.get("ids")))
            dropped = home / "PC guides" / "Zeta Map.txt"
            dropped.parent.mkdir(exist_ok=True)
            dropped.write_text("The map of Zeta's first world.")
            await pg.evaluate("(p) => window.__shimEmit('tauri://drag-drop', { paths: [p], position: { x: 400, y: 400 } })", str(dropped))
            await pg.wait_for_selector("#docs-import-dialog #doc-items li")
            await pg.click("#docs-import-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#docs-import-dialog button[type=submit]")
            said = await toast_text(pg, "Imported 1 document")
            await pg.wait_for_function("() => document.querySelector('#game-docs')?.textContent.includes('Zeta Map')")
            check("a file dropped on a PC game's page is attached to it as a document, kept with its details", (zeta_meta / "docs/Zeta Map.txt").is_file(), said)
            await pg.screenshot(path=str(out / "26c-pc-game-page.png"), full_page=True)
            await pg.locator("#game-pc").scroll_into_view_if_needed()
            await pg.screenshot(path=str(out / "26d-pc-game-section.png"))

            # Add a game… (a system's ROM from outside its folders), scanned and checked against its DAT
            downloads = home / "Downloads"
            downloads.mkdir()
            (downloads / "alpha-eu.gb").write_bytes(make_fixtures.rom(11, 32768))
            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-menu")
            await pg.click("#audit-menu")
            await pg.click('#context-menu [data-action="add-game"]')
            await pg.wait_for_selector("#add-game-dialog")
            await pg.select_option("#add-platform", "gb")
            pick(downloads / "alpha-eu.gb")
            await pg.click("#add-pick-files")
            await pg.wait_for_function("() => document.querySelector('#add-files')?.textContent.includes('alpha-eu.gb') && document.querySelector('#add-into')?.value")
            await pg.click("#add-game-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#add-game-dialog button[type=submit]")
            await pg.wait_for_selector("#add-results li", timeout=30000)
            row = await pg.inner_text("#add-results li")
            check("Add a game… copies a ROM into a folder with games, scans it and says which DAT game it is",
                  "Alpha (Europe)" in row and (fx / "Roms/alpha-eu.gb").is_file() and (downloads / "alpha-eu.gb").is_file(), row)
            await pg.screenshot(path=str(out / "27-add-game.png"))
            await pg.click("#add-results-dialog button[type=submit]")

            # Phase 5: a source's key (Settings › Sources, kept on this computer), a scrape of a
            # system's games by title (TheGamesDB's stand-in), a game's panel, documents imported
            # from a folder with their pairings changed, searched by their words and read, and an
            # export that carries the pictures and documents
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector('#sources li[data-source="thegamesdb"] .source-keys')
            await pg.fill('#sources li[data-source="thegamesdb"] input[name="apikey"]', "test-key-123")
            await pg.click('#sources li[data-source="thegamesdb"] [data-action="save-keys"]')
            await pg.wait_for_selector('#sources li[data-source="thegamesdb"] .source-said')
            said = await pg.inner_text('#sources li[data-source="thegamesdb"] .source-said')
            shown = await pg.get_attribute('#sources li[data-source="thegamesdb"] input[name="apikey"]', "placeholder")
            cfg = json.loads((home / "config/config.json").read_text())
            in_library = "test-key-123" in (library / "_library/library.json").read_text()
            check("Settings › Sources keeps a source's key on this computer (never shown again, never in the library) and Check asks the source",
                  cfg.get("source_keys", {}).get("thegamesdb", {}).get("apikey") == "test-key-123" and "answered" in said and "test-key" not in (shown or "") and not in_library,
                  (said, shown, cfg.get("source_keys")))
            await pg.screenshot(path=str(out / "30-sources.png"))
            await pg.click('.sidebar a[href="#/metadata"]')
            await pg.wait_for_selector("#meta-page")
            await pg.click('#meta-systems-list a[data-id="gb"]')
            await pg.wait_for_selector('#meta-list .pc-row[data-name="Alpha (USA)"]')
            counts = await pg.inner_text("#meta-counts")
            await pg.click("#meta-scrape")
            await pg.wait_for_selector("#scrape-dialog #scrape-sources")
            off = await pg.eval_on_selector_all("#scrape-sources li", "els => els.filter(e => e.querySelector('input').disabled).map(e => e.dataset.source)")
            await pg.screenshot(path=str(out / "31-scrape.png"))
            await pg.click("#scrape-dialog button[type=submit]")
            said = await toast_text(pg, "Scraped", timeout=60000)
            alpha = library / "_library/games/gb/Alpha (USA)"
            alpha_json = json.loads((alpha / "game.json").read_text()) if (alpha / "game.json").is_file() else {}
            alpha_media = sorted(x.name for x in (alpha / "media").iterdir()) if (alpha / "media").is_dir() else []
            check("Scrape… asks the ready sources for a system's games, by title, and keeps the details and pictures in each game's folder",
                  "found" in said and sorted(off) == ["igdb", "launchbox", "screenscraper"] and alpha_json.get("details", {}).get("developer") == "Stand-in Soft"
                  and alpha_json.get("ids", {}).get("thegamesdb") == "501" and "cover.png" in alpha_media and "screenshot.png" in alpha_media,
                  (counts, said, off, alpha_json.get("details"), alpha_media))
            row = '#meta-list .pc-row[data-name="Epsilon (USA)"]'
            await pg.wait_for_function("(r) => document.querySelector(r)?.textContent.includes('nothing yet')", arg=row, timeout=10000)
            epsilon = await pg.inner_text(row)
            check("a game sharing a ROM with another (Epsilon's second track is Beta) isn't taken for it", "Beta" not in epsilon and "Epsilon (USA)" in epsilon, epsilon)
            await pg.click('#meta-list .pc-row[data-name="Alpha (USA)"]')
            await pg.wait_for_selector("#meta-panel #meta-description")
            panel = await pg.inner_text("#meta-panel")
            pics = await pg.eval_on_selector_all("#meta-media figure", "els => els.map(e => e.dataset.kind)")
            check("a game's panel shows its details, where they came from, and its pictures",
                  "Alpha's overview" in panel and "Stand-in Soft" in panel and "1990" in panel and sorted(pics) == ["cover", "screenshot"], (panel[:300], pics))
            await pg.screenshot(path=str(out / "32-metadata.png"))

            guides = home / "Guides" / "Game Boy"
            guides.mkdir(parents=True)
            (guides / "Alpha Manual.txt").write_text("How to play Alpha. Press the zebrafish button to jump.")
            (guides / "beta_strategy_guide.md").write_text("# Beta strategy\n\nAlways go **left** at the second fork.")
            (guides / "Unrelated Thing.txt").write_text("Delta's secret level is behind the waterfall.")
            pick(guides)
            await pg.click("#meta-menu")
            await pg.click('#context-menu [data-action="meta-import-docs"]')
            await pg.wait_for_selector("#doc-items li", timeout=30000)
            pairings = await pg.eval_on_selector_all("#doc-items li", "els => Object.fromEntries(els.map(e => [e.dataset.name, e.dataset.pairing]))")
            row = '#doc-items li[data-name="Unrelated Thing.txt"]'
            await pg.click(f'{row} [data-action="change-game"]')
            await pg.fill(f"{row} .game-pick-search", "delta")
            await pg.wait_for_selector(f'{row} .game-pick-list [data-action="pick-game"]')
            await pg.click(f'{row} .game-pick-list [data-action="pick-game"]')
            await pg.wait_for_selector(f'{row}[data-pairing="you"]')
            await pg.select_option(f"{row} .doc-item-kind", "guide")
            await pg.screenshot(path=str(out / "33-docs-import.png"))
            await pg.click("#docs-import-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#docs-import-dialog button[type=submit]")
            said = await toast_text(pg, "Imported 3 documents")
            alpha_docs = json.loads((alpha / "docs/docs.json").read_text()) if (alpha / "docs/docs.json").is_file() else {}
            delta_docs = sorted(x.name for x in (library / "_library/games/gb/Delta (World)/docs").iterdir()) if (library / "_library/games/gb/Delta (World)/docs").is_dir() else []
            check("Import documents… pairs each file with its game by name, a pairing is changed by hand, and they're brought into the games' folders (the originals stay)",
                  pairings.get("Alpha Manual.txt") == "sure" and pairings.get("beta_strategy_guide.md") in ("sure", "likely") and pairings.get("Unrelated Thing.txt") == "none"
                  and alpha_docs.get("files", {}).get("Alpha Manual.txt", {}).get("kind") == "manual" and "Unrelated Thing.txt" in delta_docs and (guides / "Alpha Manual.txt").is_file(),
                  (pairings, said, alpha_docs, delta_docs))
            await pg.click('.sidebar a[href="#/documents"]')
            await pg.wait_for_selector("#docs-page #docs-list .pc-row")
            # the three brought in, and the map dropped on Zeta Quest's page in Phase 4
            listed = await pg.eval_on_selector_all("#docs-list .pc-row", "els => els.length")
            zeta_doc = await pg.locator('#docs-list .pc-row:has-text("Zeta Map")').inner_text() if await pg.locator('#docs-list .pc-row:has-text("Zeta Map")').count() else ""
            await pg.fill("#docs-search", "zebrafish")
            await pg.wait_for_function("() => document.querySelectorAll('#docs-list .pc-row').length === 1 && document.querySelector('#docs-list mark')")
            hit = await pg.inner_text("#docs-list .pc-row")
            await pg.click("#docs-list .pc-row")
            await pg.wait_for_selector("#doc-view #doc-text")
            text = await pg.inner_text("#doc-view #doc-text")
            await pg.screenshot(path=str(out / "34-documents.png"))
            await pg.fill("#docs-search", "")
            await pg.select_option("#docs-kind", "guide")
            await pg.wait_for_function("() => document.querySelectorAll('#docs-list .pc-row').length === 2")
            await pg.click('#docs-list .pc-row:has-text("Beta")')
            await pg.wait_for_selector("#doc-view #doc-text strong")
            await pg.select_option("#docs-kind", "")
            cli = cli_json("docs", "--home", str(home), "--search", "waterfall", "--json")
            check("Documents lists every document, searches their words with the words marked, and shows one (Markdown made into text); the command line searches them too",
                  listed == 4 and "Zeta Quest · PC" in zeta_doc and "Alpha Manual" in hit and "zebrafish" in hit.lower() and "zebrafish button" in text and cli.get("total") == 1 and cli["items"][0]["kind"] == "guide",
                  (listed, zeta_doc, hit, text[:100], cli.get("total")))

            # the export carries the pictures and the documents (a plain folder)
            flat = home / "Flat"
            await pg.click('.sidebar a[href="#/collections"]')
            await pg.wait_for_selector("#collection-export:not([disabled])")
            await pg.click("#collection-export")
            await pg.wait_for_selector("#export-dialog")
            await pg.wait_for_selector('#export-layout option[value="folder"]', state="attached")
            await pg.select_option("#export-layout", "folder")
            await pg.fill("#export-place-out", str(flat))
            await pg.click("#export-carry summary")
            await pg.check('#carry-media [data-id="cover"] input')
            await pg.check("#carry-docs")
            await pg.select_option("#carry-size", "320")
            await pg.click("#export-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#export-dialog button[type=submit]")
            said = await toast_text(pg, "Exported Best Of")
            carried = sorted(str(x.relative_to(flat)).replace("\\", "/") for x in flat.rglob("*") if x.is_file() and ".gamerepo" not in str(x))
            check("an export to a folder carries the chosen pictures (made smaller) and the documents beside each game",
                  any(c.endswith("/cover.png") for c in carried) and any(c.endswith("docs/Alpha Manual.txt") for c in carried), (said, carried))

            # Phase 5b. BIOS files: libretro's list (a stand-in), audited by system, gathered into the library
            gb_bios = bytes((i * 7) % 256 for i in range(256))
            (fx / "Roms" / "gb_bios.bin").write_bytes(gb_bios)
            line = lambda name, b: f"\trom ( name {name} size {len(b)} crc {zlib.crc32(b):08x} sha1 {hashlib.sha1(b).hexdigest()} )\n"
            (home / "starter-source/dat").mkdir(exist_ok=True)
            (home / "starter-source/dat/System.dat").write_text(
                'clrmamepro (\n\tname "System"\n\tdescription "System"\n\tversion 1\n\tauthor "libretro"\n)\n\ngame (\n\tname "System"\n\tcomment "System"\n\n'
                f'\tcomment "Nintendo - Gameboy"\n{line("gb_bios.bin", gb_bios)}{line("sgb_bios.bin", bytes([1, 2, 3]))}\n\tcomment "Sega - Dreamcast"\n{line("dc/dc_boot.bin", bytes(512))})\n')
            await pg.click('.sidebar a[href="#/"]')
            await pg.click("#home-scan-button")
            await toast_text(pg, "Scanned")
            await pg.click('.sidebar a[href="#/bios"]')
            await pg.wait_for_selector("#bios-empty")
            await pg.click("#bios-get")
            await pg.click("#ask-dialog button[type=submit]")
            said = await toast_text(pg, "starter DATs for 1 system")
            gb_row = '#bios-list .bios-system[data-system="Nintendo - Gameboy"]'
            await pg.wait_for_selector(gb_row)
            shown = await pg.eval_on_selector_all("#bios-list .bios-system", "els => els.map(e => [e.dataset.system, +e.dataset.have, +e.dataset.missing])")
            totals = await pg.inner_text("#bios-totals")
            await pg.fill("#bios-search", "dreamcast")
            await pg.wait_for_function("() => document.querySelectorAll('#bios-list .bios-system').length === 1")
            searched = await pg.eval_on_selector_all("#bios-list .bios-system", "els => els.map(e => e.dataset.system)")
            await pg.fill("#bios-search", "")
            check("BIOS files: libretro's list is got from the BIOS page and every system audited, the ones you have games for first, with a search and no picker (D69)",
                  shown == [["Nintendo - Gameboy", 1, 1], ["Sega - Dreamcast", 0, 1]] and searched == ["Sega - Dreamcast"] and "0 of 1 with every BIOS file" in totals
                  and await pg.query_selector("#bios-show") is None, (said, shown, searched, totals))
            await pg.screenshot(path=str(out / "40-bios.png"), full_page=True)
            await pg.click("#bios-gather")
            await pg.wait_for_selector("#bios-gather-dialog #organise-totals")
            await pg.click("#bios-gather-dialog button[type=submit]")
            said = await toast_text(pg, "Imported the BIOS files")
            gathered = library / "BIOS/Nintendo - Gameboy/gb_bios.bin"
            check("Import copies the BIOS files the library has into its BIOS folder, a folder for each system",
                  gathered.is_file() and gathered.read_bytes() == gb_bios and (fx / "Roms/gb_bios.bin").is_file(), said)

            # the layout editor: Knulli's layout as a tree, a system's folder changed, saved as a new layout
            await pg.click('.sidebar a[href="#/layouts"]')
            await pg.wait_for_selector('#layout-list [data-id="knulli"]')
            await pg.click('#layout-list [data-id="knulli"]')
            await pg.wait_for_selector('#lt-tree .lt-node[data-path="card/roms/gb"]')
            builtin_label = await pg.inner_text("#layout-save")
            await pg.click('#lt-tree .lt-node[data-path="card/roms/gb"] > .lt-row .lt-name')
            await pg.wait_for_selector('#lt-inspect[data-at="systems.gb"]')
            await pg.fill("#lt-system-folder", "gameboy")
            await pg.wait_for_selector('#lt-tree .lt-node[data-path="card/roms/gameboy"]')
            game = await pg.get_attribute('#lt-tree .lt-node[data-at="games.path"][data-path^="card/roms/gameboy/"]', "data-path")
            await pg.click(f'#lt-tree .lt-node[data-path="{game}"] > .lt-row .lt-name')
            await pg.wait_for_selector('#lt-inspect[data-at="games.path"]')
            await pg.fill("#lt-path", "roms/{folder}/{file}")
            await pg.wait_for_selector("#lt-problems")
            blocked = await pg.is_disabled("#layout-save")
            await pg.fill("#lt-path", "{card}/roms/{folder}/{file}")
            await pg.wait_for_selector("#lt-problems", state="detached")
            await pg.select_option("#lt-sample", label="Best Of's first games")
            await pg.wait_for_function("(n) => document.querySelector('#lt-tree')?.textContent.includes(n)", arg=names[0])
            await pg.screenshot(path=str(out / "41-layout-editor.png"))
            await pg.click("#lt-tab-json")
            text = await pg.input_value("#lt-json")
            await pg.click("#lt-tab-tree")
            await pg.click("#layout-save")
            await pg.fill("#layout-save-name", "Knulli (mine)")
            await pg.click("#layout-save-dialog button[type=submit]")
            await toast_text(pg, "Saved the layout Knulli")
            await pg.wait_for_selector('#layout-list [data-id="knulli-mine"].active')
            saved_file = library / "_library/layouts/knulli-mine.json"
            mine = json.loads(saved_file.read_text()) if saved_file.exists() else {}
            check("the layout editor shows Knulli's layout as a tree, follows a change at once, catches a path that breaks it, and saves the change as a new layout",
                  builtin_label == "Save as new…" and blocked and '"gb": "gameboy"' in text and mine.get("systems", {}).get("gb") == "gameboy"
                  and (mine.get("copied_from") or {}).get("id") == "knulli" and not mine.get("builtin"), (builtin_label, blocked, mine.get("systems"), mine.get("copied_from")))

            # an export to an SD card in the new layout, with the BIOS files
            card = home / "Card"
            await pg.click('.sidebar a[href="#/collections"]')
            await pg.wait_for_selector("#collection-export:not([disabled])")
            await pg.click("#collection-export")
            await pg.wait_for_selector('#export-layout option[value="knulli-mine"]', state="attached")
            await pg.select_option("#export-layout", "knulli-mine")
            await pg.fill("#export-place-card", str(card))
            await pg.evaluate("() => { document.querySelector('#export-carry').open = true; }")
            await pg.check("#carry-bios")
            await pg.click("#export-dialog button[type=submit]")
            await pg.wait_for_selector("#export-layout-used")
            await pg.click("#export-dialog button[type=submit]")
            said = await toast_text(pg, "Exported Best Of")
            on_card = sorted(str(x.relative_to(card)).replace("\\", "/") for x in card.rglob("*") if x.is_file() and ".gamerepo" not in str(x)) if card.is_dir() else []
            gl = (card / "roms/gameboy/gamelist.xml").read_text() if (card / "roms/gameboy/gamelist.xml").exists() else ""
            await pg.click('#collection-view [data-view="exports"]')
            await pg.wait_for_selector('#exports li[data-target="knulli-mine"]')
            listed = await pg.inner_text('#exports li[data-target="knulli-mine"]')
            check("an export to a card in the new layout puts the games in its folders, with a gamelist, small pictures and the BIOS files",
                  all(f"roms/gameboy/{n}" in " ".join(on_card) for n in names) and "bios/gb_bios.bin" in on_card and gl.count("<game>") == len(names)
                  and any(c.startswith("roms/gameboy/images/") and c.endswith("-thumb.png") for c in on_card) and "Knulli (mine)" in listed, (said, on_card, gl[:300], listed))
            # Phase 6. The Library: every game across the systems, by its pictures, filtered with counts
            await pg.evaluate("() => window.__gamerepo.platform.store.prefs.set('gr-library-view', 'grid')")
            await pg.click('.sidebar a[href="#/library"]')
            await pg.wait_for_selector('#lib-games .lib-tile[data-key="gb/Alpha (USA)"]')
            tiles = await pg.eval_on_selector_all("#lib-games .lib-tile", "els => els.map(e => e.dataset.key)")
            total = await pg.get_attribute("#lib-games", "data-total")
            await pg.wait_for_function("() => { const i = document.querySelector('.lib-tile[data-key=\"gb/Alpha (USA)\"] img'); return i && i.complete && i.naturalWidth > 0; }", timeout=15000)
            src = await pg.get_attribute('.lib-tile[data-key="gb/Alpha (USA)"] img', "src")
            thumbs = [x for x in (home / "data/thumbs").rglob("*.jpg")] if (home / "data/thumbs").is_dir() else []
            systems = await pg.eval_on_selector_all("#filter-platform [data-value]", "els => els.map(e => e.dataset.value)")
            check("the Library shows every game you have across the systems (PC games too), with thumbnails made on this computer",
                  "gb/Alpha (USA)" in tiles and "nes/Echo (USA)" in tiles and any(t.startswith("pc/") for t in tiles) and int(total) == len(tiles)
                  and "~thumb/256/" in (src or "") and thumbs and not list(library.rglob("*.jpg")) and {"gb", "nes", "pc"} <= set(systems), (tiles, total, src, len(thumbs), systems))
            await pg.screenshot(path=str(out / "50-library.png"))
            action = await pg.inner_text('#filter-genre [data-value="Action"] .lib-count')
            await pg.click('#filter-genre [data-value="Action"] input')
            await pg.wait_for_selector('#lib-chips [data-chip="genre-Action"]')
            await pg.wait_for_function("(n) => document.querySelectorAll('#lib-games .lib-tile').length === n", arg=int(action))
            shown = await pg.eval_on_selector_all("#lib-games .lib-tile", "els => els.map(e => e.dataset.key)")
            address = await pg.evaluate("location.hash")
            gb_count = await pg.inner_text('#filter-platform [data-value="gb"] .lib-count')
            await pg.go_back()
            await pg.wait_for_selector('#lib-chips [data-chip="genre-Action"]', state="detached")
            await pg.wait_for_function("(n) => document.querySelectorAll('#lib-games .lib-tile').length === n", arg=len(tiles), timeout=10000)
            back = await pg.eval_on_selector_all("#lib-games .lib-tile", "els => els.length")
            radios = await pg.eval_on_selector_all("#lib-filters input[type=radio]", "els => els.length")
            drops = await pg.eval_on_selector_all("#lib-filters .lib-filter-select select", "els => els.map(e => e.id)")
            check("Players and Rating are drop-downs in the filters, not lists of buttons", radios == 0 and len(drops) >= 1, (radios, drops))
            check("a filter's values have counts, its chip shows above the games, it's kept in the address and Back takes it off",
                  int(action) >= 2 and "gb/Alpha (USA)" in shown and "gb/Beta (Europe)" in shown and "nes/Echo (USA)" not in shown and "genre=Action" in address
                  and int(gb_count) == int(action) and back == len(tiles), (action, shown, address, gb_count, back))
            await pg.fill("#lib-search", "stand-in")
            await pg.wait_for_function("() => location.hash.includes('q=stand-in') && !document.querySelector('.lib-tile[data-key=\"nes/Echo (USA)\"]')")
            found = await pg.eval_on_selector_all("#lib-games .lib-tile", "els => els.map(e => e.dataset.key)")
            await pg.fill("#lib-search", "")
            await pg.wait_for_selector('.lib-tile[data-key="nes/Echo (USA)"]')
            check("the search finds games by their developer, description or title", "gb/Alpha (USA)" in found and "nes/Echo (USA)" not in found, found)
            await pg.click('.lib-tile[data-key="gb/Alpha (USA)"]')
            await pg.wait_for_selector('#lib-panel[data-key="gb/Alpha (USA)"] #lib-panel-title')
            panel = await pg.inner_text("#lib-panel")
            await pg.click('.lib-tile[data-key="gb/Beta (Europe)"]', modifiers=["Control"])
            await pg.wait_for_selector('#lib-panel[data-count="2"]')
            bar = await pg.inner_text("#bar-count")
            await pg.keyboard.press("Control+a")
            await pg.wait_for_function("(n) => document.querySelector('#bar-count')?.textContent.startsWith(String(n))", arg=len(tiles))
            await pg.keyboard.press("Escape")
            await pg.wait_for_selector("#lib-head:not(.selecting)")
            check("picking a game shows it in the panel; Ctrl adds to the selection, Ctrl+A takes every game shown and Esc clears it",
                  "Alpha" in panel and "Stand-in Soft" in panel and bar.startswith("2"), (panel[:200], bar))
            await pg.click("#lib-view [data-view=table]")
            await pg.wait_for_selector('#lib-games .lib-row[data-key="gb/Alpha (USA)"]')
            await pg.click('.lib-thead [data-sort="year"]')
            await pg.wait_for_function("() => location.hash.includes('sort=year')")
            await pg.wait_for_function("() => document.querySelector('#lib-games .lib-row .lib-col-year')?.textContent === '1990'")
            await pg.click("#lib-group")
            await pg.click('#context-menu [data-action="group-system"]')
            await pg.wait_for_selector('#lib-games .lib-group[data-group="gb"]')
            groups = await pg.eval_on_selector_all("#lib-games .lib-group", "els => els.map(e => e.dataset.group)")
            await pg.screenshot(path=str(out / "51-library-table.png"))
            check("the table sorts by a column, and the games can be grouped by system", "gb" in groups and "nes" in groups and "pc" in groups, groups)
            await pg.click("#lib-view [data-view=grid]")
            await pg.click("#lib-group")
            await pg.click('#context-menu [data-action="group-none"]')
            # a game's page: its details edited by hand (kept over scrapes), a file dropped on it attached
            await pg.dblclick('.lib-tile[data-key="gb/Alpha (USA)"]')
            await pg.wait_for_selector('#game-page[data-key="gb/Alpha (USA)"] #game-developer')
            await pg.fill("#game-developer", "My Own Studio")
            await pg.press("#game-developer", "Tab")
            await pg.wait_for_selector('.game-detail[data-field="developer"][data-edited="yes"]')
            alpha_json = json.loads((alpha / "game.json").read_text())
            pos = await pg.inner_text(".game-pos")
            gallery = await pg.eval_on_selector_all("#game-strip [data-kind], .game-strip [data-kind]", "els => els.map(e => e.dataset.kind)")
            check("a game's page shows its pictures and details; a detail changed there is kept in game.json, marked as yours",
                  alpha_json["details"]["developer"] == "My Own Studio" and "developer" in alpha_json.get("edited", []) and alpha_json["scraped"]["developer"] == "you"
                  and "of" in pos and sorted(gallery) == ["cover", "screenshot"], (alpha_json.get("details"), alpha_json.get("edited"), pos, gallery))
            await pg.screenshot(path=str(out / "52-game.png"), full_page=True)
            # the picture sits in one square box: picking another doesn't change its size
            box = "() => { const r = document.querySelector('#game-gallery .game-hero').getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; }"
            sizes = []
            for k in ["cover", "screenshot", "cover"]:
                await pg.click(f'#game-strip [data-kind="{k}"]')
                await pg.wait_for_function(f"() => document.querySelector('#game-strip [data-kind=\"{k}\"]').getAttribute('aria-selected') === 'true' && document.querySelector('#game-gallery .game-hero').complete")
                sizes.append(await pg.evaluate(box))
            check("a game's picture keeps one square size whichever picture is picked", sizes[0][0] > 100 and all(s == [sizes[0][0]] * 2 for s in sizes), sizes)
            dropped = home / "Guides" / "Alpha Map.txt"
            dropped.write_text("The map of Alpha's second world.")
            await pg.evaluate("(p) => window.__shimEmit('tauri://drag-drop', { paths: [p], position: { x: 400, y: 400 } })", str(dropped))
            await pg.wait_for_selector("#docs-import-dialog #doc-items li")
            await pg.click("#docs-import-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            await pg.click("#docs-import-dialog button[type=submit]")
            said = await toast_text(pg, "Imported 1 document")
            await pg.wait_for_function("() => document.querySelector('#game-docs')?.textContent.includes('Alpha Map')")
            check("a file dropped on a game's page is attached to it as a document", (alpha / "docs/Alpha Map.txt").is_file(), said)
            await pg.click("#game-next")
            await pg.wait_for_function("() => document.querySelector('#game-page')?.dataset.key && document.querySelector('#game-page').dataset.key !== 'gb/Alpha (USA)'")
            next_key = await pg.get_attribute("#game-page", "data-key")
            await pg.click("#game-back")
            await pg.wait_for_selector("#lib-page")
            await pg.click('.lib-tile[data-key="gb/Alpha (USA)"]')
            await pg.wait_for_selector('#lib-panel[data-key="gb/Alpha (USA)"]')
            panel = await pg.inner_text("#lib-panel")
            check("Next goes to the next game in the list it was opened from, and the Library follows the edit", next_key and next_key != "gb/Alpha (USA)" and "My Own Studio" in panel, (next_key, panel[:200]))
            # the filters saved as a collection, whose rules keep up with the details
            await pg.click('#filter-genre [data-value="Action"] input')
            await pg.wait_for_selector('#lib-chips [data-chip="genre-Action"]')
            await pg.click("#lib-menu")
            await pg.click('#context-menu [data-action="lib-save"]')
            await pg.fill("#lib-save-name", "Action games")
            await pg.click("#lib-save-dialog button[type=submit]")
            await toast_text(pg, "Saved Action games")
            saved = [json.loads(x.read_text()) for x in (library / "_library/collections").glob("*.json")]
            mine = next((c for c in saved if c.get("name") == "Action games"), {})
            cli = cli_json("library", "--home", str(home), "--genre", "Action", "--json")
            check("Save as a collection… makes the filters a collection's rules (by its games' details); the command line filters the same",
                  mine.get("rules", {}).get("details", {}).get("genre") == ["Action"] and cli.get("total") == int(action), (mine.get("rules"), cli.get("total")))
            await pg.click('#lib-chips [data-chip="genre-Action"] button')

            # Phase 6b: each source's answer kept apart, one picture from each source to choose
            # from, Delete metadata… with Undo, Scrape again… and the sizes in Settings. A second
            # source's answer and box are put in Alpha's folder as an earlier scrape would have.
            (alpha / "sources/igdb.json").write_text(json.dumps({"source": "igdb", "id": "55", "title": "Alpha", "details": {"description": "Alpha from IGDB."}}))
            (alpha / "media/cover").mkdir(exist_ok=True)
            (alpha / "media/cover/igdb.png").write_bytes(png(90, 60, (10, 160, 60)))
            kept = json.loads((alpha / "sources/thegamesdb.json").read_text())
            await pg.evaluate("() => { location.hash = '#/game/gb/' + encodeURIComponent('Alpha (USA)'); }")
            await pg.wait_for_selector('#game-variants button[data-name="igdb"]')
            await pg.click('#game-variants button[data-name="igdb"]')
            await pg.wait_for_selector('#game-variants button[data-name="igdb"][aria-pressed="true"]')
            hero = await pg.get_attribute("#game-gallery .game-hero", "src")
            alpha_json = json.loads((alpha / "game.json").read_text())
            check("a scrape keeps each source's answer apart, and a game's page shows each source's box to choose from (one each), the chosen one where exports look",
                  kept.get("details", {}).get("developer") == "Stand-in Soft" and alpha_json.get("chosen", {}).get("cover") == "igdb"
                  and (alpha / "media/cover.png").read_bytes() == png(90, 60, (10, 160, 60)) and (alpha / "media/cover/thegamesdb.png").is_file() and hero.endswith("v=igdb"),
                  (kept, alpha_json.get("chosen"), hero, sorted(x.name for x in (alpha / "media").rglob("*"))))
            await pg.screenshot(path=str(out / "53-game-variants.png"), full_page=True)
            # the Media ribbon on the right (D69): every kind, each source's, larger, Use this; ] folds it.
            # It starts folded (UI pass 3, finding 1), so the rail opens it
            started_folded = await pg.evaluate("() => !document.querySelector('#pane-media') && !!document.querySelector('#rail-media')")
            await pg.click("#rail-media")
            await pg.wait_for_selector('#inspector[data-tab="media"] #game-media .media-kind[data-kind="cover"] .media-variant[data-name="igdb"].chosen')
            in_ribbon = await pg.eval_on_selector_all('#game-media .media-kind[data-kind="cover"] .media-variant', "els => els.map(e => e.dataset.name)")
            await pg.click('#game-media .media-kind[data-kind="cover"] .media-variant[data-name="thegamesdb"] .media-thumb')
            await pg.wait_for_selector("#media-view img")
            await pg.screenshot(path=str(out / "53b-game-media.png"))
            await pg.click('#game-media .media-kind[data-kind="cover"] .media-variant[data-name="thegamesdb"] [data-action="use"]')
            await pg.wait_for_selector('#game-media .media-variant[data-name="thegamesdb"].chosen')
            chosen_now = json.loads((alpha / "game.json").read_text()).get("chosen", {}).get("cover")
            await pg.keyboard.press("]")
            await pg.wait_for_function("() => !document.querySelector('#pane-media') && document.querySelector('#rail-media')")
            await pg.click("#rail-media")
            await pg.wait_for_selector("#pane-media #game-media")
            check("a game's page has a ribbon on the right, folded at first, with every source's pictures by kind, larger on a click, Use this, folded with ]",
                  started_folded and sorted(in_ribbon) == ["igdb", "thegamesdb"] and chosen_now == "thegamesdb", (started_folded, in_ribbon, chosen_now))
            await pg.click('#game-variants button[data-name="igdb"]')
            await pg.wait_for_selector('#game-variants button[data-name="igdb"][aria-pressed="true"]')
            row = '.game-detail[data-field="description"]'
            await pg.click(f"{row} .game-detail-others summary")
            await pg.click(f'{row} .game-detail-others [data-source="igdb"] [data-action="use"]')
            await pg.wait_for_function("() => document.querySelector('#game-description')?.value === 'Alpha from IGDB.'")
            alpha_json = json.loads((alpha / "game.json").read_text())
            check("a detail lists what the other sources said, and Use this takes it from one (kept over the source order)",
                  alpha_json.get("pinned", {}).get("description") == "igdb" and alpha_json["details"]["description"] == "Alpha from IGDB.", alpha_json.get("pinned"))
            await pg.click("#game-menu")
            await pg.click('#context-menu [data-action="game-scrape-again"]')
            await pg.wait_for_selector("#scrape-dialog #scrape-variants")
            again = await pg.evaluate("() => [document.querySelector('input[name=scrape-existing][value=replace]').checked, document.querySelector('#scrape-fresh').checked, document.querySelector('#scrape-dialog h2, #scrape-dialog .dialog-title')?.textContent || '']")
            check("Scrape again… is the Scrape dialog set to replace and ask the sources afresh", again[0] and again[1] and "again" in again[2], again)
            await pg.keyboard.press("Escape")
            await pg.wait_for_selector("#scrape-dialog", state="detached")
            await pg.click("#game-menu")
            await pg.click('#context-menu [data-action="game-delete-metadata"]')
            await pg.wait_for_selector("#meta-delete-dialog #meta-delete-what")
            await pg.click("#meta-delete-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-dialog #organise-totals")
            await pg.screenshot(path=str(out / "54-delete-metadata.png"))
            await pg.click("#organise-dialog button[type=submit]")
            said = await toast_text(pg, "Deleted")
            alpha_json = json.loads((alpha / "game.json").read_text())
            gone = not (alpha / "sources").exists() and not (alpha / "media/cover.png").exists() and not (alpha / "media/cover").exists()
            check("Delete metadata… sets a game's details from sources and pictures aside; what you wrote and your own documents stay",
                  gone and alpha_json["details"].get("developer") == "My Own Studio" and "description" not in alpha_json["details"] and (alpha / "docs/Alpha Map.txt").is_file(),
                  (said, alpha_json.get("details"), sorted(x.name for x in alpha.rglob("*"))))
            await pg.click("#toast .toast-action")
            await toast_text(pg, "Undone")
            alpha_json = json.loads((alpha / "game.json").read_text())
            check("its Undo puts them back and the details are made again", (alpha / "media/cover.png").is_file() and alpha_json["details"].get("description") == "Alpha from IGDB.", alpha_json.get("details"))
            sizes = cli_json("games", "sizes", "--home", str(home), "--json")
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.click("#meta-sizes-work")
            await pg.wait_for_selector('#meta-sizes tr[data-platform="gb"]')
            shown = await pg.inner_text("#settings-metadata")
            check("Settings › Metadata shows what the metadata takes, by system and kind, and which source each detail comes from",
                  "Game Boy" in shown and "Box front" in shown and sizes.get("kinds", {}).get("cover", 0) > 0 and await pg.locator('#details-from select[data-field="description"]').count() == 1, (shown[:300], sizes.get("kinds")))
            await pg.screenshot(path=str(out / "55-settings-metadata.png"), full_page=True)

            # Phase 7: an arcade set (MAME's -listxml), with its BIOS set, a device it lacks, and its set type
            arc = fx / "Roms" / "MAME"
            arc.mkdir(parents=True, exist_ok=True)
            ab = lambda n: make_fixtures.rom(2000 + n, 64)

            def zipped(path, members):
                with zipfile.ZipFile(path, "w") as z:
                    for name, data in members:
                        z.writestr(name, data)
            zipped(arc / "alpha.zip", [("a.1", ab(1)), ("shared.2", ab(2))])
            zipped(arc / "alphaj.zip", [("aj.1", ab(3)), ("shared.2", ab(2))])
            zipped(arc / "bios1.zip", [("bios.bin", ab(4))])
            zipped(arc / "beta.zip", [("b.1", ab(5))])
            rx = lambda n, x, merge=None: make_fixtures.rom_xml(n, ab(x)).replace("<rom ", f'<rom merge="{merge}" ' if merge else "<rom ")
            mame = home / "mame.xml"
            mame.write_text('<?xml version="1.0"?>\n<mame build="0.271 (mame0271)">\n'
                            f'<machine name="bios1" isbios="yes"><description>Test BIOS</description>{rx("bios.bin", 4)}</machine>\n'
                            f'<machine name="dev1" isdevice="yes" runnable="no"><description>Test device</description>{rx("dev.bin", 6)}</machine>\n'
                            f'<machine name="alpha"><description>Alpha Fighter (World)</description><year>1991</year><manufacturer>Acme</manufacturer>{rx("a.1", 1)}{rx("shared.2", 2)}</machine>\n'
                            f'<machine name="alphaj" cloneof="alpha" romof="alpha"><description>Alpha Fighter (Japan)</description>{rx("aj.1", 3)}{rx("shared.2", 2, "shared.2")}</machine>\n'
                            f'<machine name="beta" romof="bios1"><description>Beta Quest</description>{rx("bios.bin", 4, "bios.bin")}{rx("b.1", 5)}<device_ref name="dev1"/></machine>\n'
                            '</mame>\n')
            pick(mame)
            await pg.click('.sidebar a[href="#/import"]')
            await pg.click("#import-dats")
            await toast_text(pg, "Imported")
            await pg.goto(B + "#/")
            await pg.click("#home-scan-button")
            await toast_text(pg, "Scanned")
            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-page")
            await pg.click('#audit-panel-list a[data-id="mame"]')
            await pg.wait_for_selector("#audit-playable")
            totals = await pg.inner_text("#audit-totals")
            await pg.click('#audit-tabs [data-tab="have"]')
            await pg.wait_for_selector('#audit-list[data-tab="have"] tr[data-name="beta"] .audit-needs')
            needs = await pg.inner_text('#audit-list tr[data-name="beta"] .audit-needs')
            alpha_row = await pg.inner_text('#audit-list tr[data-name="alpha"]')
            await pg.screenshot(path=str(out / "56-arcade-audit.png"), full_page=True)
            await pg.click('#audit-tabs [data-tab="sets"]')
            await pg.wait_for_selector('#audit-list[data-tab="sets"] tr[data-name="bios1"]')
            sets = await pg.inner_text("#audit-list")
            check("an arcade DAT (MAME's -listxml) is audited: BIOS sets and devices apart from the games, what's playable, and what a game needs",
                  "3 of 3 games" in totals.replace("\n", " ") and "2 playable" in totals and "1 of 2" in totals
                  and "dev1" in needs and "Alpha Fighter" in alpha_row and "1991" in alpha_row and "bios1" in sets and "dev1" in sets and "missing" in sets.lower(),
                  (totals, needs, alpha_row, sets))
            await pg.select_option("#audit-set-type", "split")
            await toast_text(pg, "split set")
            await pg.wait_for_function("() => document.querySelector('#audit-tabs [data-tab=\"misnamed\"] .tab-count')?.textContent === '1'", timeout=15000)
            kept = json.loads((library / "_library/library.json").read_text()).get("arcade_sets", {})
            await pg.click('.sidebar a[href="#/"]')
            await pg.wait_for_function("() => document.querySelector('#home-next')?.textContent.includes('Arcade (MAME) to put right')", timeout=15000)
            nxt = await pg.inner_text("#home-next")
            await pg.screenshot(path=str(out / "57-home-next.png"))
            await pg.click('#home-next li[data-kind="misnamed"] a:has-text("Arcade")')
            await pg.wait_for_selector('#audit-list[data-tab="misnamed"]')
            check("Home's Next strip says what to do, and its link opens the page to do it on",
                  "1 file in Arcade (MAME) to put right" in nxt and "#/audit" in await pg.evaluate("location.hash"), nxt)
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector('#arcade-set-types li[data-system="mame"] select')
            in_settings = await pg.input_value('#arcade-set-types li[data-system="mame"] select')
            await pg.select_option('#arcade-set-types li[data-system="mame"] select', "non-merged")
            await toast_text(pg, "non-merged set")
            back_to = json.loads((library / "_library/library.json").read_text()).get("arcade_sets", {})
            check("an arcade system's set type is kept in the library: split lists a clone's copy of its parent's ROM as misnamed, and Settings › Systems shows and changes it",
                  kept.get("mame") == "split" and in_settings == "split" and back_to.get("mame") == "non-merged", (kept, in_settings, back_to))

            # magazines (D68): a folder of issues read, put right and brought in; covers,
            # a comic book a page at a time, a list to check them against
            mags = home / "Magazines in"
            (mags / "Retro Gamer").mkdir(parents=True)
            with zipfile.ZipFile(mags / "Game Fan 001 (1992-10).cbz", "w") as z:
                for n in ("p1.png", "p2.png", "p10.png"):
                    z.writestr(n, COVER)
            (mags / "Game Fan 002 (1992-11).pdf").write_bytes(b"%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [] /Count 0 >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n")
            with zipfile.ZipFile(mags / "Retro Gamer" / "100.cbz", "w") as z:
                z.writestr("cover.png", WIDE)
            await pg.click('.sidebar a[href="#/magazines"]')
            await pg.wait_for_selector("#magazines-page #mag-empty")
            pick(mags)
            await pg.click("#mag-import")
            await pg.wait_for_selector("#mag-items li", timeout=30000)
            read = await pg.eval_on_selector_all("#mag-items li", "els => Object.fromEntries(els.map(e => [e.dataset.name, [e.querySelector('.mag-item-title').value, e.querySelector('.mag-item-number').value, e.querySelector('.mag-item-date').value]]))")
            await pg.fill('#mag-items li[data-name="100.cbz"] .mag-item-date', "2012-06")
            await pg.screenshot(path=str(out / "60-magazines-import.png"))
            await pg.click("#mag-import-dialog button[type=submit]")
            await pg.wait_for_selector("#organise-totals")
            totals = await pg.inner_text("#organise-totals")
            await pg.click("#mag-import-dialog button[type=submit]")
            said = await toast_text(pg, "Imported 3 magazine issues")
            await pg.wait_for_selector('#mag-panel-list a[data-id="Game Fan"]')
            fan = library / "Magazines" / "Game Fan"
            fan_json = json.loads((fan / "magazine.json").read_text()) if (fan / "magazine.json").is_file() else {}
            check("Import magazines… reads each file's magazine, number and date from its name (or its folder's), they're put right, and brought into Magazines/<title>/ (the originals stay)",
                  read.get("Game Fan 001 (1992-10).cbz") == ["Game Fan", "1", "1992-10"] and read.get("100.cbz") == ["Retro Gamer", "100", ""]
                  and "3 issues" in totals and [i.get("number") for i in fan_json.get("issues", [])] == ["1", "2"]
                  and (library / "Magazines/Retro Gamer/magazine.json").is_file() and (mags / "Game Fan 001 (1992-10).cbz").is_file(),
                  (read, totals, said, fan_json))
            await pg.click('#mag-panel-list a[data-id="Game Fan"]')
            await pg.wait_for_selector('#mag-issues li[data-number="1"] img.mag-cover')
            await pg.wait_for_function("() => document.querySelector('#mag-issues li[data-number=\"1\"] img.mag-cover')?.naturalWidth > 0", timeout=15000)
            await pg.wait_for_function("() => document.querySelector('#mag-issues li[data-number=\"2\"] img.mag-cover')?.naturalWidth > 0", timeout=15000)
            plain = await pg.evaluate("async () => { const r = await fetch(document.querySelector('#mag-issues li[data-number=\"2\"] img.mag-cover').src); return [r.headers.get('content-type'), await r.text()]; }")
            await pg.screenshot(path=str(out / "61-magazines.png"))
            check("a magazine's issues show by their covers: a comic book's first page, and a PDF without a picture a plain one with its number",
                  "svg" in (plain[0] or "") and "No. 2" in plain[1] and "Game Fan" in plain[1], (plain[0], plain[1][:200]))
            await pg.click('#mag-issues li[data-number="1"] .mag-open')
            await pg.wait_for_selector("#doc-dialog #comic-view")
            first = await pg.inner_text("#comic-page")
            await pg.click("#comic-next")
            await pg.wait_for_function("() => document.querySelector('#comic-page').textContent.includes('Page 2')")
            await pg.wait_for_function("() => document.querySelector('#comic-view .comic-page img')?.naturalWidth > 0", timeout=15000)
            locked = await pg.is_disabled("#doc-edit-title")
            await pg.screenshot(path=str(out / "62-comic.png"))
            await pg.click('#doc-dialog button[aria-label="Close"]')
            await pg.wait_for_selector("#doc-dialog", state="detached")
            check("an issue opens in the document viewer, a comic book a page at a time (its pages in order), named on the Magazines page only",
                  first == "Page 1 of 3" and locked, (first, locked))
            listed = home / "game-fan.csv"
            listed.write_text("magazine,number,date\nGame Fan,1,1992-10\nGame Fan,2,1992-11\nGame Fan,3,1992-12\n")
            pick(listed)
            await pg.click("#mag-add-list-button")
            await toast_text(pg, "Imported the list")
            await pg.wait_for_selector("#mag-missing li")
            missing = await pg.inner_text("#mag-missing")
            totals = await pg.inner_text("#mag-totals")
            await pg.click("#mag-save-missing-one")
            said = await toast_text(pg, "Saved 1 missing issue")
            saved = list((library / "_library/reports").glob("Magazines missing*.csv"))
            check("a list of issues (a CSV) says which are missing, and they're saved as a list of their own",
                  "Game Fan 003 (1992-12)" in missing and "Game Fan 001" not in missing and "2 of the 3 listed" in totals and len(saved) == 1 and "Game Fan,3" in saved[0].read_text(),
                  (missing, totals, said, [x.name for x in saved]))

            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector("#theme-select")

            # 7. the theme
            await pg.select_option("#theme-select", "night")
            theme = await pg.evaluate("() => document.documentElement.dataset.theme")
            await pg.wait_for_timeout(500)  # preferences are saved after a short pause
            check("the theme follows the setting and is kept", theme == "night" and prefs().get("theme") == "night", (theme, prefs()))
            await pg.screenshot(path=str(out / "03-night.png"))
            await pg.select_option("#theme-select", "system")
            check("the theme is chosen in one place, Settings › Appearance (the menu has no theme button)",
                  await pg.query_selector("#theme-toggle") is None and await pg.query_selector("#settings-appearance #theme-select") is not None)

            # 8. the skin: Drafting Sheet unless chosen, changed at once (icons too) and kept
            first = await pg.evaluate("() => document.documentElement.dataset.skin")
            drawn = lambda: pg.evaluate("() => [...document.querySelectorAll('.nav-link[title=Home] svg g[data-sk]')].find((g) => getComputedStyle(g).display !== 'none')?.dataset.sk")
            icon_before = await drawn()
            options = await pg.eval_on_selector_all("#skin-select option", "els => els.map(e => e.value)")
            await pg.select_option("#skin-select", "classic")
            skin = await pg.evaluate("() => document.documentElement.dataset.skin")
            icon_after = await drawn()
            await pg.wait_for_timeout(500)
            await pg.screenshot(path=str(out / "03-classic.png"))
            saved = prefs().get("skin")
            await pg.select_option("#skin-select", "drafting")
            check("the skin is Drafting Sheet unless chosen; Classic changes the look and icons at once and is kept",
                  first == "drafting" and icon_before == "drafting" and options == ["drafting", "classic"]
                  and skin == "classic" and icon_after == "classic" and saved == "classic", (first, icon_before, options, skin, icon_after, saved))

            # 9. About: versions, what's coming and the layout
            await pg.click('.sidebar a[href="#/about"]')
            await pg.wait_for_selector("#about-versions")
            about = await pg.inner_text("#about-page")
            check("About shows the version, what's coming and how a library is laid out",
                  "Game Repo" in about and "Phase 9" in about and "_library/" in about and "game.json" in about, about[:200])
            await pg.screenshot(path=str(out / "04-about.png"), full_page=True)

            # 10. a library made by a newer app opens read-only
            future = home / "Future"
            (future / "_library").mkdir(parents=True)
            (future / "_library/library.json").write_text(json.dumps({"format": 99, "id": "lib-future", "name": "Future"}))
            await pg.click('.sidebar a[href="#/settings"]')
            pick(future)
            await pg.click("#open-library")
            await pg.wait_for_function("() => document.querySelector('#library-path')?.textContent.endsWith('Future')")
            disabled = await pg.is_disabled("#library-name")
            await pg.goto(B + "#/")
            await pg.wait_for_selector(".home .warn-note")
            note = await pg.inner_text(".home .warn-note")
            check("a newer library opens read-only", disabled and "read-only" in note and sorted(x.name for x in future.iterdir()) == ["_library"]
                  and not (future / "_library/README.txt").exists(), note)
            await pg.screenshot(path=str(out / "05-read-only.png"))

            # 11. narrow window: the menu becomes a drawer
            await pg.set_viewport_size({"width": 700, "height": 800})
            await pg.wait_for_timeout(300)  # the drawer slides
            hidden = await pg.evaluate("() => document.querySelector('.sidebar').getBoundingClientRect().right <= 0")
            await pg.click(".cmdbar .bar-burger")
            await pg.wait_for_selector(".sidebar.open")
            await pg.wait_for_timeout(300)
            shown = await pg.evaluate("() => document.querySelector('.sidebar').getBoundingClientRect().left >= 0")
            check("the menu is a drawer in a narrow window", hidden and shown, (hidden, shown))
            await pg.screenshot(path=str(out / "06-narrow.png"))
            await b.close()
    finally:
        server.terminate()
        server.wait(timeout=20)

    # 12. portable: the library opens the same after moving it, and keeps its index here
    moved = home / "moved" / "My Games"
    shutil.copytree(library, moved)
    i1, i2 = cli_json("library-info", "--library", str(library)), cli_json("library-info", "--library", str(moved))
    same = {k: i1[k] for k in ("id", "name", "format")} == {k: i2[k] for k in ("id", "name", "format")}
    x1 = cli_json("index-info", "--library", str(library), "--data", str(home / "data"))
    x2 = cli_json("index-info", "--library", str(moved), "--data", str(home / "data"))
    check("the library opens the same after moving, with the same index", same and x1["path"] == x2["path"] and x2["set_aside"] is None,
          (i1["name"], i2["name"], x1["path"], x2["path"]))
    (out / "library-info.json").write_text(json.dumps({k: i1[k] for k in ("id", "name", "format")}, indent=1))
    shutil.make_archive(str(out / "library"), "zip", library)

    for e in errors:
        print("PAGE ERROR:", e)
    failed = [n for n, ok in results if not ok]
    print(f"{len(results) - len(failed)} of {len(results)} checks passed")
    return 1 if failed or errors else 0


sys.exit(asyncio.run(main()))
