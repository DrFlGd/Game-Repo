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
Delete metadata… with Undo, and Settings › Details and pictures' sizes;
renaming the library, with Undo; opening a folder that already holds games
leaves them as they were, and switching back from the recent list; the theme
and skin follow the settings and are kept; a library from a newer app opens
read-only; the menu is a drawer in a narrow window; and the library and its
index are the same after the library is moved (library-info, index-info).
(Phase 8a) a collection exported to RetroArch, Pegasus, LaunchBox and Playnite, each
read back where that frontend looks. In homes of their own: (Phase 7) an arcade set
with its BIOS set, a device and its set type; (Phase 8c) magazines; (D77) games of two
systems moved in from a folder, one to a new folder named for its system, and Undo;
(Phase 9a) Settings › Serve to devices starting Game Repo Server, pairing the stand-in
device and removing it, and the server stopping with the app; (D134) updates, with the
CLI's stand-in for the updater: the check as it starts, About, the status bar, Settings ›
Updates, Update and restart, and Game Repo Server's window.
Writes <out>/library-info.json and <out>/library.zip for the cross-platform check.

Each part runs as a `Section` (review M44): a timeout or error in it is a FAIL of its
own, with the test's line, a screenshot `failed-<part>.png`, and the page back at Home
for the next part, so one stuck check doesn't hide the rest. A part that needs nothing
from the others has a home, library and backend of its own (`OWN_HOMES`, `own_home`).
"""
import argparse
import asyncio
import base64
import functools
import hashlib
import http.server
import json
import os
import shutil
import struct
import subprocess
import sys
import threading
import time
import traceback
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


def text_pdf(pages):
    """A real PDF whose pages are a line of text each (a magazine with its own text, D145)."""
    objs = [b"<< /Type /Catalog /Pages 2 0 R >>", None, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]
    kids = []
    for t in pages:
        stream = f"BT /F1 14 Tf 30 400 Td ({t}) Tj ET".encode()
        objs.append(b"<< /Length %d >>\nstream\n" % len(stream) + stream + b"\nendstream")
        objs.append(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 420 595] /Contents %d 0 R /Resources << /Font << /F1 3 0 R >> >> >>" % len(objs))
        kids.append(len(objs))
    objs[1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (b" ".join(b"%d 0 R" % k for k in kids), len(kids))
    out, at = bytearray(b"%PDF-1.4\n"), []
    for i, o in enumerate(objs, 1):
        at.append(len(out))
        out += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1) + b"".join(b"%010d 00000 n \n" % x for x in at)
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)
    return bytes(out)


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


def start_server(at=None, port=None):
    """The backend for the test's home (with the stand-ins for the sources), or for a part
    with a home of its own (review M44)."""
    at, port = at or home, port or a.port
    if at == home:
        starter_source()
        cfg = json.loads((home / "config/config.json").read_text())
        cfg["source_urls"] = sources_stand_in()
        (home / "config/config.json").write_text(json.dumps(cfg))
    log = open(out / ("serve.log" if at == home else f"serve-{at.name}.log"), "w")
    p = subprocess.Popen([a.cli, "serve", "--ui", a.ui, "--home", str(at), "--port", str(port)], stdout=log, stderr=log)
    for _ in range(100):
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1)
            return p
        except Exception:
            time.sleep(0.2)
    raise SystemExit(f"the server didn't start; see {log.name}")


def pick(path, base=None):
    """The answer to the next folder dialog (of the test's home, or a part's at `base`)."""
    req = urllib.request.Request((base or B) + "test/pick", data=json.dumps({"path": str(path)}).encode(), method="POST", headers={"content-type": "application/json"})
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


async def back_home(pg, base):
    """The page loaded again at Home, whatever was open."""
    await pg.goto(base)
    await pg.wait_for_selector("#home-head, #first-run", timeout=30000)


class Section:
    """A part of the test (review M44): when a wait times out or anything else goes wrong
    in it, that's a FAIL of its own, the page goes back to Home and the next part runs,
    so one stuck check doesn't hide the rest."""

    def __init__(self, pg, base, name):
        self.pg, self.base, self.name = pg, base, name

    async def __aenter__(self):
        self.started = time.time()
        return self

    async def __aexit__(self, kind, e, tb):
        took = f"{time.time() - self.started:.0f} s"
        if e is None:
            print(f"-- {self.name}: {took}", flush=True)
            return False
        if not isinstance(e, Exception):
            return False
        here = [f for f in traceback.extract_tb(tb) if f.filename == __file__]
        at = f" (line {here[-1].lineno})" if here else ""
        first = str(e).strip().splitlines()[0] if str(e).strip() else ""
        check(f"{self.name}: its checks ran to the end", False, f"{kind.__name__}{at}: {first}")
        try:
            slug = "-".join("".join(c if c.isalnum() else " " for c in self.name.lower()).split()[:5])
            await self.pg.screenshot(path=str(out / f"failed-{slug}.png"))
            await back_home(self.pg, self.base)
        except Exception as again:
            print(f"   (couldn't go back to Home: {again})", flush=True)
        print(f"-- {self.name}: stopped after {took}", flush=True)
        return True


async def own_home(b, name, port, part):
    """A part that needs nothing from the others runs in a home, a library and a backend of
    its own (review M44), from the first start on."""
    at = Path(f"{home}-{name}")
    shutil.rmtree(at, ignore_errors=True)
    at.mkdir(parents=True)
    try:
        server = start_server(at, port)
    except SystemExit as e:
        check(f"{name}: its backend started", False, e)
        return
    base = f"http://127.0.0.1:{port}/"
    try:
        ctx = await b.new_context(viewport={"width": 1400, "height": 900})
        await ctx.add_init_script(path=str(SHIM))
        pg = await ctx.new_page()
        pg.on("pageerror", lambda e: errors.append(f"{e} ({name})"))
        pg.on("dialog", lambda d: asyncio.ensure_future(d.accept()))
        pg.on("console", lambda m: errors.append(f"{m.text} ({m.location.get('url', '')}, {name})") if m.type == "error" and "net::ERR_" not in m.text else None)
        async with Section(pg, base, f"{name} (a home of its own)"):
            await pg.goto(base)
            await pg.wait_for_selector("#first-run", timeout=30000)
            pick(at / "My Games", base)
            await pg.click("#choose-library")
            await pg.wait_for_function("() => document.querySelector('#home-head h1')?.textContent === 'My Games'")
            await part(pg, base, at, at / "My Games")
        await ctx.close()
    finally:
        server.terminate()
        server.wait(timeout=20)


async def until(test, timeout=15.0):
    """Wait for something outside the page (a file written) to be so."""
    end = time.time() + timeout
    while not test():
        if time.time() > end:
            raise TimeoutError("waited too long")
        await asyncio.sleep(0.2)


async def arcade(pg, base, home, library):
    """Phase 7: an arcade set (MAME's -listxml), with its BIOS set, a device it lacks, and its set type."""
    arc = home / "Roms" / "MAME"
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
    pick(home / "Roms", base)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#add-root")
    await pg.wait_for_selector("#roots li")
    rx = lambda n, x, merge=None: make_fixtures.rom_xml(n, ab(x)).replace("<rom ", f'<rom merge="{merge}" ' if merge else "<rom ")
    mame = home / "mame.xml"
    mame.write_text('<?xml version="1.0"?>\n<mame build="0.271 (mame0271)">\n'
                    f'<machine name="bios1" isbios="yes"><description>Test BIOS</description>{rx("bios.bin", 4)}</machine>\n'
                    f'<machine name="dev1" isdevice="yes" runnable="no"><description>Test device</description>{rx("dev.bin", 6)}</machine>\n'
                    f'<machine name="alpha"><description>Alpha Fighter (World)</description><year>1991</year><manufacturer>Acme</manufacturer>{rx("a.1", 1)}{rx("shared.2", 2)}</machine>\n'
                    f'<machine name="alphaj" cloneof="alpha" romof="alpha"><description>Alpha Fighter (Japan)</description>{rx("aj.1", 3)}{rx("shared.2", 2, "shared.2")}</machine>\n'
                    f'<machine name="beta" romof="bios1"><description>Beta Quest</description>{rx("bios.bin", 4, "bios.bin")}{rx("b.1", 5)}<device_ref name="dev1"/></machine>\n'
                    '</mame>\n')
    pick(mame, base)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#import-dats")
    await toast_text(pg, "Imported")
    await pg.goto(base + "#/")
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
    await pg.wait_for_function("() => document.querySelector('#home-next')?.textContent.includes('Arcade (MAME) needs review')", timeout=15000)
    nxt = await pg.inner_text("#home-next")
    await pg.screenshot(path=str(out / "57-home-next.png"))
    await pg.click('#home-next li[data-kind="misnamed"] a:has-text("Arcade")')
    await pg.wait_for_selector('#audit-list[data-tab="misnamed"]')
    check("Home's Next strip says what to do, and its link opens the page to do it on",
          "1 file in Arcade (MAME) needs review" in nxt and "#/audit" in await pg.evaluate("location.hash"), nxt)
    await pg.click('.sidebar a[href="#/settings"]')
    await pg.wait_for_selector('#arcade-set-types li[data-system="mame"] select')
    in_settings = await pg.input_value('#arcade-set-types li[data-system="mame"] select')
    await pg.select_option('#arcade-set-types li[data-system="mame"] select', "non-merged")
    await toast_text(pg, "non-merged set")
    back_to = json.loads((library / "_library/library.json").read_text()).get("arcade_sets", {})
    check("an arcade system's set type is kept in the library: split lists a clone's copy of its parent's ROM as misnamed, and Settings › Systems shows and changes it",
          kept.get("mame") == "split" and in_settings == "split" and back_to.get("mame") == "non-merged", (kept, in_settings, back_to))


async def magazines(pg, base, home, library):
    """Magazines (D68): a folder of issues read, put right and brought in; covers, a comic
    book a page at a time, a list to check them against."""
    mags = home / "Magazines in"
    (mags / "Retro Gamer").mkdir(parents=True)
    with zipfile.ZipFile(mags / "Game Fan 001 (1992-10).cbz", "w") as z:
        for n in ("p1.png", "p2.png", "p10.png"):
            z.writestr(n, COVER)
    # a PDF with its own text, and no picture on its first page
    (mags / "Game Fan 002 (1992-11).pdf").write_bytes(text_pdf(["Letters to the editor, and what is coming next month", "Sonic the Hedgehog 2 reviewed: the fastest game there is"]))
    with zipfile.ZipFile(mags / "Retro Gamer" / "100.cbz", "w") as z:
        z.writestr("cover.png", WIDE)
    await pg.click('.sidebar a[href="#/magazines"]')
    await pg.wait_for_selector("#mag-empty")
    pick(mags, base)
    await pg.click("#mag-import")
    await pg.wait_for_selector("#mag-items li", timeout=30000)
    read = await pg.eval_on_selector_all("#mag-items li", "els => Object.fromEntries(els.map(e => [e.dataset.name, [e.querySelector('.mag-item-title').value, e.querySelector('.mag-item-number').value, e.querySelector('.mag-item-date').value]]))")
    await pg.fill('#mag-items li[data-name="100.cbz"] .mag-item-date', "2012-06")
    await pg.screenshot(path=str(out / "60-magazines-import.png"))
    await pg.click("#mag-import-dialog button[type=submit]")
    await pg.wait_for_selector("#organise-totals")
    totals = await pg.inner_text("#organise-totals")
    mag_head = await pg.evaluate("() => [document.querySelector('#mag-import-dialog h2').textContent, location.hash]")
    await pg.click("#mag-import-dialog button[type=submit]")
    said = await toast_text(pg, "Imported 3 magazine issues")
    check("Magazines' import is the Import page's, its preview in the present tense (M40, D78)", mag_head == ["Import 3 magazine issues", "#/import/import-mags"], mag_head)
    await pg.click('.sidebar a[href="#/magazines"]')
    await pg.wait_for_selector('#mags-items section[data-group="Game Fan"] .lib-tile')
    fan = library / "Magazines" / "Game Fan"
    fan_json = json.loads((fan / "magazine.json").read_text()) if (fan / "magazine.json").is_file() else {}
    check("Import magazines… reads each file's magazine, number and date from its name (or its folder's), they're put right, and brought into Magazines/<title>/ (the originals stay)",
          read.get("Game Fan 001 (1992-10).cbz") == ["Game Fan", "1", "1992-10"] and read.get("100.cbz") == ["Retro Gamer", "100", ""]
          and "3 issues" in totals and [i.get("number") for i in fan_json.get("issues", [])] == ["1", "2"]
          and (library / "Magazines/Retro Gamer/magazine.json").is_file() and (mags / "Game Fan 001 (1992-10).cbz").is_file(),
          (read, totals, said, fan_json))
    one = '#mags-items section[data-group="Game Fan"] .lib-tile[data-id^="Game Fan/Game Fan 001"]'
    two = '#mags-items section[data-group="Game Fan"] .lib-tile[data-id^="Game Fan/Game Fan 002"]'
    await pg.wait_for_function(f"() => document.querySelector('{one} img.lib-cover')?.naturalWidth > 0", timeout=15000)
    await pg.wait_for_function(f"() => document.querySelector('{two} img.lib-cover')?.naturalWidth > 0", timeout=15000)
    plain = await pg.evaluate(f"async () => {{ const r = await fetch(document.querySelector('{two} img.lib-cover').src); return [r.headers.get('content-type'), await r.text()]; }}")
    shelf = await pg.evaluate("() => [[...document.querySelectorAll('#mags-items .shelf-group-head h2')].map(e => e.textContent), [...document.querySelectorAll('#mags-items section[data-group=\"Game Fan\"] .lib-tile-title')].map(e => e.textContent), [...document.querySelectorAll('#filter-mags-magazine [data-value]')].map(e => [e.dataset.value, e.querySelector('.lib-count').textContent])]")
    await pg.screenshot(path=str(out / "61-magazines.png"))
    check("Magazines is laid out as the Library (D154): every issue a tile by its cover (a comic book's first page, a PDF without a picture a plain one with its number), grouped by magazine, with a Magazine filter and its counts",
          "svg" in (plain[0] or "") and "No. 2" in plain[1] and "Game Fan" in plain[1] and shelf[0] == ["Game Fan", "Retro Gamer"] and shelf[1] == ["No. 1", "No. 2"]
          and ["Game Fan", "2"] in shelf[2] and ["Retro Gamer", "1"] in shelf[2], (plain[0], plain[1][:200], shelf))
    await pg.dblclick(one)
    await pg.wait_for_selector("#doc-dialog #comic-view")
    first = await pg.inner_text("#comic-page")
    await pg.click("#comic-next")
    await pg.wait_for_function("() => document.querySelector('#comic-page').textContent.includes('Page 2')")
    await pg.wait_for_function("() => document.querySelector('#comic-view .comic-page img')?.naturalWidth > 0", timeout=15000)
    locked = await pg.is_disabled("#doc-edit-title")
    await pg.screenshot(path=str(out / "62-comic.png"))
    # Esc closes the reader, and so does going to another page (review M36)
    await pg.keyboard.press("Escape")
    await pg.wait_for_selector("#doc-dialog", state="detached")
    await pg.dblclick(one)
    await pg.wait_for_selector("#doc-dialog #comic-view")
    await pg.evaluate("() => { location.hash = '#/home'; }")
    await pg.wait_for_selector("#doc-dialog", state="detached")
    await pg.evaluate("() => { location.hash = '#/magazines'; }")
    await pg.wait_for_selector(one)
    check("an issue opens in the document viewer (a double click), a comic book a page at a time (its pages in order), named on the Magazines page only; Esc or another page closes it",
          first == "Page 1 of 3" and locked, (first, locked))
    listed = home / "game-fan.csv"
    listed.write_text("magazine,number,date\nGame Fan,1,1992-10\nGame Fan,2,1992-11\nGame Fan,3,1992-12\n")
    pick(listed, base)
    await pg.click("#mags-menu")
    await pg.click('#context-menu [data-action="mag-lists"]')
    await pg.click("#mag-add-list-button")
    await toast_text(pg, "Imported the list")
    await pg.wait_for_selector('#mag-list-names li[data-name="game-fan.csv"]')
    await pg.click("#mag-lists button[type=submit]")
    await pg.wait_for_selector("#mag-lists", state="detached")
    await pg.click("#mag-missing-too")
    ghost = '#mags-items section[data-group="Game Fan"] .lib-tile.lib-missing'
    await pg.wait_for_selector(ghost)
    ghosts = await pg.eval_on_selector_all(ghost, "els => els.map(e => e.querySelector('.lib-tile-title').textContent)")
    await pg.click(ghost)
    await pg.wait_for_selector('#mags-panel #mag-issue[data-missing="1"]')
    missing = await pg.inner_text("#mags-panel")
    await pg.click("#mag-save-missing-one")
    said = await toast_text(pg, "Saved 1 missing issue")
    saved = list((library / "_library/reports").glob("Magazines missing*.csv"))
    await pg.click("#mag-missing-too")
    await pg.wait_for_selector(ghost, state="detached")
    check("a list of issues (a CSV, in ⋯ › Lists…) says which are missing: Missing issues too shows them as faded tiles, and they're saved as a list of their own",
          ghosts == ["No. 3"] and "Game Fan 003 (1992-12)" in missing and len(saved) == 1 and "Game Fan,3" in saved[0].read_text(),
          (ghosts, missing, said, [x.name for x in saved]))
    # D145: the issues' text read (the PDF's own; the comic's pages by text recognition where
    # there's a reader, else left unread), kept beside them, searched, and shown in the viewer;
    # D154: from the magazine's heading menu
    before = await pg.inner_text("#mag-unread")
    await pg.click('#mags-items section[data-group="Game Fan"] .shelf-group-more')
    await pg.click('#context-menu [data-action="mag-read"]')
    said = await toast_text(pg, "Read 2 issues")
    await pg.click(two)
    await pg.wait_for_function("() => document.querySelector('#mags-panel #mag-text')?.dataset.state === 'read'", timeout=30000)
    after = await pg.inner_text("#mag-text")
    left = await pg.inner_text("#mag-unread")
    kept = json.loads((fan / ".text" / "Game Fan 002 (1992-11).pdf.json").read_text()) if (fan / ".text" / "Game Fan 002 (1992-11).pdf.json").is_file() else {}
    check("Read its text (a magazine heading's menu) reads its issues page by page and keeps the text beside them",
          "3 issues' text isn't read yet" in before and "1 issue's text isn't read yet" in left and "Its text is read" in after and "5 pages" in said and "Hedgehog" in "".join(kept.get("pages", [])[1:2]),
          (before, left, after, said, kept))
    games = await pg.inner_text("#mag-games")
    check("an issue's games are found in its text (none here: this library has no DATs), shown in the panel", "No game's title is in its text" in games and "Games in No. 2" in games, games)
    # D154: a tag added in the panel, then filtered by; an issue moved to another magazine, with Undo
    await pg.fill("#mag-tags-add", "Reviews")
    await pg.press("#mag-tags-add", "Enter")
    await pg.wait_for_selector('#filter-mags-tag [data-value="Reviews"]')
    tagged = [i.get("tags") for i in json.loads((fan / "magazine.json").read_text()).get("issues", [])]
    await pg.click('#filter-mags-tag [data-value="Reviews"] input')
    await pg.wait_for_selector('#mags-chips [data-chip="tag-Reviews"]')
    shown = await pg.eval_on_selector_all("#mags-items .lib-tile", "els => els.map(e => e.dataset.id)")
    await pg.click("#mags-clear")
    check("a tag added to an issue in the panel is kept in its magazine.json, and filters the issues (Tag)",
          [t or [] for t in tagged] == [[], ["Reviews"]] and len(shown) == 1 and shown[0].startswith("Game Fan/Game Fan 002"), (tagged, shown))
    retro = '#mags-items section[data-group="Retro Gamer"] .lib-tile'
    await pg.click(retro, button="right")
    await pg.click('#context-menu [data-action="move"]')
    await pg.fill("#shelf-ask-value", "Game Fan")
    await pg.click("#shelf-ask button[type=submit]")
    await pg.wait_for_selector("#organise-dialog #organise-totals")
    move_head = await pg.inner_text("#organise-dialog h2")
    await pg.click("#organise-dialog button[type=submit]")
    await toast_text(pg, "Moved 1 issue to Game Fan")
    await pg.wait_for_selector(retro, state="detached", timeout=15000)
    moved = sorted(x.name for x in fan.iterdir() if x.is_file() and x.suffix == ".cbz")
    fan_issues = [i.get("number") for i in json.loads((fan / "magazine.json").read_text()).get("issues", [])]
    await pg.click("#toast button:has-text('Undo')")
    await toast_text(pg, "Undone")
    await pg.wait_for_selector(retro, timeout=15000)
    check("Move to another magazine… moves an issue's file into that magazine's folder, named for it, as a change with the preview, and Undo puts it back",
          move_head == "Move 1 issue to Game Fan" and len(moved) == 2 and fan_issues == ["1", "2", "100"] and (library / "Magazines/Retro Gamer").is_dir()
          and any(x.suffix == ".cbz" for x in (library / "Magazines/Retro Gamer").iterdir()), (move_head, moved, fan_issues))
    # D152: the shelf keeps its view and grouping when another page is shown
    await pg.click("#mags-view [data-view=list]")
    await pg.evaluate("() => { location.hash = '#/home'; }")
    await pg.click('.sidebar a[href="#/magazines"]')
    await pg.wait_for_selector("#mags-page[data-view=list] .lib-line")
    await pg.click("#mags-view [data-view=table]")
    await pg.wait_for_selector("#mags-page .shelf-row")
    cols = await pg.eval_on_selector_all("#mags-page .shelf-thead [role=columnheader]", "els => els.map(e => e.textContent)")
    await pg.click("#mags-view [data-view=grid]")
    check("a shelf keeps its view while another page is shown; its table has its own columns", cols[:3] == ["Magazine ▴", "Number", "Date"], cols)
    await pg.click('.sidebar a[href="#/documents"]')
    await pg.fill("#docs-search", "hedgehog")
    await pg.wait_for_selector('#docs-list .pc-row:has-text("Game Fan 2")', timeout=15000)
    await pg.click('#docs-list .pc-row:has-text("Game Fan 2")')
    await pg.wait_for_selector("#doc-view #doc-page-text mark")
    marked = await pg.inner_text("#doc-page-text mark")
    at = await pg.input_value("#doc-view .pdf-page-label input")
    await pg.screenshot(path=str(out / "63-magazine-text.png"))
    await pg.fill("#docs-search", "")
    check("Documents' search finds an issue by its text; the viewer opens at the page with it, its text shown with the word marked",
          marked == "Hedgehog" and at == "2", (marked, at))


def mp4_bytes(seconds):
    """An MP4's header saying how long it is (there's nothing in it to play)."""
    box = lambda name, body: struct.pack(">I", len(body) + 8) + name + body
    mvhd = bytearray(100)
    mvhd[12:16] = struct.pack(">I", 1000)
    mvhd[16:20] = struct.pack(">I", seconds * 1000)
    return box(b"ftyp", b"isom\0\0\x02\0isomiso2") + box(b"moov", box(b"mvhd", bytes(mvhd)))


async def videos(pg, base, home, library):
    """Videos (D146): a folder of them read, put right and brought in by kind; one watched,
    the list filtered, a link added and changed, and a file removed with Undo."""
    vids = home / "Videos in"
    (vids / "Reviews").mkdir(parents=True)
    (vids / "Super Metroid - Longplay [dQw4w9WgXcQ].mp4").write_bytes(mp4_bytes(5400))
    (vids / "Reviews" / "20190412 Mega Man 2.mp4").write_bytes(mp4_bytes(600))
    await pg.click('.sidebar a[href="#/videos"]')
    await pg.wait_for_selector("#video-empty")
    pick(vids, base)
    await pg.click("#video-import")
    await pg.wait_for_selector("#video-items li", timeout=30000)
    read = await pg.eval_on_selector_all("#video-items li", "els => Object.fromEntries(els.map(e => [e.dataset.name, [e.querySelector('.video-item-title').value, e.querySelector('.video-item-kind').value, e.querySelector('.video-item-date').value, e.querySelector('.pc-item-name .muted').textContent]]))")
    await pg.fill('#video-items li[data-name="20190412 Mega Man 2.mp4"] .video-item-title', "Mega Man 2, reviewed")
    await pg.screenshot(path=str(out / "64-videos-import.png"))
    await pg.click("#video-import-dialog button[type=submit]")
    await pg.wait_for_selector("#organise-totals")
    totals = await pg.inner_text("#organise-totals")
    head = await pg.evaluate("() => [document.querySelector('#video-import-dialog h2').textContent, location.hash]")
    await pg.click("#video-import-dialog button[type=submit]")
    await toast_text(pg, "Imported 2 videos")
    await pg.click('.sidebar a[href="#/videos"]')
    await pg.wait_for_selector("#videos-items .lib-tile", timeout=15000)
    tiles = "els => els.map(e => [e.querySelector('.lib-tile-title').textContent, e.querySelector('.lib-badge')?.textContent, e.querySelector('.lib-tile-meta').textContent])"
    listed = await pg.eval_on_selector_all("#videos-items .lib-tile", tiles)
    on_disk = sorted(p.relative_to(library).as_posix() for p in (library / "Videos").rglob("*.mp4"))
    metroid = read.get("Super Metroid - Longplay [dQw4w9WgXcQ].mp4", ["", "", "", ""])
    check("Import videos… reads each one's title, kind and date from its name (or its folder's), its length from its header; they're put right and brought into Videos/<kind>/ (the originals stay); the page shows them as tiles, as the Library does (D153)",
          metroid[:3] == ["Super Metroid - Longplay", "walkthrough", ""] and "1:30:00" in metroid[3]
          and read.get("20190412 Mega Man 2.mp4", [None])[:3] == ["Mega Man 2", "review", "2019-04-12"]
          and head == ["Import 2 videos", "#/import/import-videos"] and "2 videos" in totals
          and on_disk == ["Videos/Reviews/Mega Man 2, reviewed.mp4", "Videos/Walkthroughs/Super Metroid - Longplay.mp4"]
          and (vids / "Super Metroid - Longplay [dQw4w9WgXcQ].mp4").is_file()
          and [x[0] for x in listed] == ["Mega Man 2, reviewed", "Super Metroid - Longplay"] and listed[1][1] == "Walkthrough" and "YouTube" in listed[1][2] and "1:30:00" in listed[1][2],
          (read, head, totals, on_disk, listed))
    await pg.dblclick('#videos-items .lib-tile:has-text("Super Metroid")')
    await pg.wait_for_selector("#video-player")
    playing = await pg.evaluate("() => [location.hash.startsWith('#/videos/'), !!document.querySelector('#video-player video, #video-player [data-video=\"cant-play\"]'), document.querySelector('#video-player h2').textContent]")
    await pg.click('#filter-videos-kind [data-value="review"] input')
    await pg.wait_for_selector('#videos-chips [data-chip="kind-review"]')
    reviews = await pg.eval_on_selector_all("#videos-items .lib-tile", "els => els.map(e => e.querySelector('.lib-badge').textContent)")
    await pg.click('#filter-videos-kind [data-value="review"] input')
    await pg.fill("#videos-search", "metroid")
    found = await pg.eval_on_selector_all("#videos-items .lib-tile-title", "els => els.map(e => e.textContent)")
    await pg.fill("#videos-search", "")
    await pg.screenshot(path=str(out / "65-videos.png"))
    await pg.click("#video-player-close")
    await pg.wait_for_selector("#video-player", state="detached")
    check("a video file plays on the Videos page (#/videos/<id>, a double click), and the tiles filter by kind (the filters down the left) and by words",
          playing == [True, True, "Super Metroid - Longplay"] and reviews == ["Review"] and found == ["Super Metroid - Longplay"], (playing, reviews, found))
    await pg.click("#videos-menu")
    await pg.click('#context-menu [data-action="video-add-link"]')
    await pg.wait_for_selector("#video-link-dialog")
    await pg.fill("#video-link-url", "https://www.youtube.com/watch?v=abcdefghijk")
    await pg.fill("#video-link-title", "The History of Metroid")
    await pg.click("#video-link-dialog button[type=submit]")
    await toast_text(pg, "Added The History of Metroid")
    history = '#videos-items .lib-tile:has-text("The History of Metroid")'
    await pg.click(history)
    await pg.wait_for_selector("#videos-panel #video-edit")
    await pg.fill("#video-edit-date", "2021-03")
    await pg.press("#video-edit-date", "Enter")
    await pg.wait_for_function("() => document.querySelector('#video-edit > p.muted')?.textContent.includes('2021-03')")
    link = await pg.inner_text("#video-edit > p.muted")
    kept = json.loads((library / "Videos" / "videos.json").read_text())
    link_kept = next((v for v in kept.get("videos", []) if v.get("url", "").endswith("abcdefghijk")), {})
    check("Add a link… keeps a web video (its kind from its title, its source from its address), and its date is changed in the panel",
          "Retrospective" in link and "YouTube" in link and "a link" in link and link_kept.get("date") == "2021-03" and "file" not in link_kept, (link, link_kept))
    # D153: two picked (Ctrl and a click), their series set and a tag added together; grouped by series
    await pg.click('#videos-items .lib-tile:has-text("Super Metroid")', modifiers=["Control"])
    await pg.wait_for_selector('#videos-panel[data-count="2"]')
    await pg.click('#videos-items .lib-tile:has-text("Super Metroid")', button="right")
    await pg.click('#context-menu [data-action="series"]')
    await pg.fill("#shelf-ask-value", "Metroid")
    await pg.click("#shelf-ask button[type=submit]")
    await pg.wait_for_selector("#shelf-ask", state="detached")
    await pg.click('#videos-items .lib-tile:has-text("Super Metroid")', button="right")
    await pg.click('#context-menu [data-action="tag-add"]')
    await pg.fill("#shelf-ask-value", "Favourites")
    await pg.click("#shelf-ask button[type=submit]")
    await pg.wait_for_selector('#filter-videos-tag [data-value="Favourites"]')
    await pg.click("#videos-group")
    await pg.click('#context-menu [data-action="videos-group-series"]')
    await pg.wait_for_selector('#videos-items section[data-group="Metroid"]')
    groups = await pg.eval_on_selector_all("#videos-items .shelf-group-head", "els => els.map(e => [e.querySelector('h2').textContent, e.parentElement.querySelectorAll('.lib-tile').length])")
    kept = {v["title"]: v for v in json.loads((library / "Videos" / "videos.json").read_text()).get("videos", [])}
    await pg.click("#videos-group")
    await pg.click('#context-menu [data-action="videos-group-none"]')
    await pg.keyboard.press("Escape")
    check("several videos picked are changed together: their series set and a tag added (kept in videos.json), and grouped by series",
          groups == [["Metroid", 2], ["In no series", 1]] and all(kept.get(t, {}).get("series") == "Metroid" and kept.get(t, {}).get("tags") == ["Favourites"] for t in ("Super Metroid - Longplay", "The History of Metroid")),
          (groups, kept))
    await pg.click('#videos-items .lib-tile:has-text("Mega Man 2")', button="right")
    await pg.click('#context-menu [data-action="remove"]')
    await pg.wait_for_selector("#organise-dialog #organise-totals")
    remove_head = await pg.inner_text("#organise-dialog h2")
    await pg.click("#organise-dialog button[type=submit]")
    said = await toast_text(pg, "Removed the video")
    await pg.wait_for_selector('#videos-items .lib-tile:has-text("Mega Man 2")', state="detached", timeout=15000)
    gone = not (library / "Videos/Reviews/Mega Man 2, reviewed.mp4").exists()
    await pg.click("#toast button:has-text('Undo')")
    await toast_text(pg, "Undone")
    await pg.wait_for_selector('#videos-items .lib-tile:has-text("Mega Man 2")', timeout=15000)
    check("Remove… sets a video's file aside as a change, with the preview, and Undo brings it back",
          remove_head == "Remove the video Mega Man 2, reviewed" and gone and (library / "Videos/Reviews/Mega Man 2, reviewed.mp4").is_file(), (remove_head, said, gone))


async def import_by_system(pg, base, home, library):
    """D77's moves and new folders (review M44: the test only looked for the choices): games
    of two systems moved in from a folder, one to its system's folder with games, the other
    to a new folder named for its system beside it, which becomes a folder with games of
    its own; Undo puts them back and takes the new folder away."""
    fx = home / "fx"
    make_fixtures.main(fx)
    gb = fx / "Roms" / "Game Boy"
    pick(gb, base)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#add-root")
    await pg.wait_for_selector("#roots li")
    gear = make_fixtures.rom(31, 512)
    gg_dat = home / "gg.dat"
    gg_dat.write_text(make_fixtures.logiqx("Sega - Game Gear", "20261001", [("Gear One (World)", None, [make_fixtures.rom_xml("Gear One (World).gg", gear)])]))
    for f in (fx / "dats" / "gb.dat", gg_dat):
        pick(f, base)
    await pg.click("#import-dats")
    await toast_text(pg, "Imported")
    await back_home(pg, base)
    await pg.click("#home-scan-button")
    await toast_text(pg, "Scanned")
    incoming = home / "Incoming"
    incoming.mkdir()
    (incoming / "europe one.gb").write_bytes(make_fixtures.rom(11, 32768))
    (incoming / "gear.gg").write_bytes(gear)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#import-games")
    pick(incoming, base)
    await pg.click("#import-add-folder")
    await pg.wait_for_selector("#import-folders li")
    await pg.check('#import-mode input[value="move"]')
    await pg.click("#import-games-dialog button[type=submit]")
    await pg.wait_for_selector("#import-games-review", timeout=30000)
    for p in ("gb", "gamegear"):
        await pg.wait_for_selector(f'#import-places li[data-platform="{p}"][data-dir]:not([data-dir=""])')
    to_gb = await pg.get_attribute('#import-places li[data-platform="gb"]', "data-dir")
    to_gg = await pg.get_attribute('#import-places li[data-platform="gamegear"]', "data-dir")
    why_gg = await pg.inner_text('#import-places li[data-platform="gamegear"] small')
    said_move = await pg.inner_text("#import-games-totals")
    await pg.click("#import-games-review button[type=submit]")
    await pg.wait_for_selector("#bring-dialog #organise-totals")
    preview = await pg.inner_text("#bring-dialog")
    await pg.screenshot(path=str(out / "23f-import-move.png"))
    await pg.click("#bring-dialog button[type=submit]")
    said = await toast_text(pg, "Imported 2 games into the library", timeout=30000)
    new_dir = fx / "Roms" / "Sega Game Gear"
    moved = [(gb / "Alpha (Europe).gb").is_file(), (new_dir / "Gear One (World).gg").is_file(), not any(incoming.iterdir())]
    roots = {Path(r["path"]): r.get("platform") for r in json.loads((library / "_library/library.json").read_text()).get("roots", [])}
    check("Import moves each system's games: to the folder with games named for it, and for a system with none to a new folder named for it beside it, which becomes its folder with games (D77)",
          Path(to_gb) == gb and Path(to_gg) == new_dir and "new folder" in why_gg.lower() and "moved" in said_move.lower() and "A new folder becomes a folder with games" in preview
          and all(moved) and roots.get(new_dir) == "gamegear", (to_gb, to_gg, why_gg, said_move, preview[-300:], said, moved, roots))
    await back_home(pg, base)
    await pg.wait_for_selector('#home-platforms [data-platform="gamegear"]')
    bar = await pg.inner_text('#home-platforms [data-platform="gamegear"]')
    check("what's moved in is scanned and matched: Game Gear has its one game", "1 of 1" in bar, bar)
    await pg.click('.sidebar a[href="#/history"]')
    await pg.wait_for_selector('#history li:first-child [data-action="undo"]')
    await pg.click('#history li:first-child [data-action="undo"]')
    await toast_text(pg, "Undone")
    roots = [Path(r["path"]) for r in json.loads((library / "_library/library.json").read_text()).get("roots", [])]
    back = [(incoming / "europe one.gb").is_file(), (incoming / "gear.gg").is_file(), not (gb / "Alpha (Europe).gb").exists(), not new_dir.exists(), new_dir not in roots]
    check("Undo moves them back, and takes away the new folder and its place in the library", all(back), (back, roots))

    # Jerred, 2026-10-20: games can be left where they are, and moved into the library
    # later all at once; the status bar shows how far what's running has got
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#import-games")
    pick(incoming, base)
    await pg.click("#import-add-folder")
    await pg.wait_for_selector("#import-folders li")
    await pg.check('#import-mode input[value="leave"]')
    label = await pg.inner_text("#import-games-dialog button[type=submit]")
    await pg.click("#import-games-dialog button[type=submit]")
    said = await toast_text(pg, "Scanned", timeout=30000)
    roots = [Path(r["path"]) for r in json.loads((library / "_library/library.json").read_text()).get("roots", [])]
    stayed = [(incoming / "europe one.gb").is_file(), (incoming / "gear.gg").is_file()]
    check("Leave them where they are adds the folder to Folders with games and scans it, copying nothing",
          label == "Add the folders" and incoming in roots and all(stayed), (label, said, roots, stayed))
    await pg.wait_for_selector("#import-gather")
    await pg.click("#import-gather")
    await pg.wait_for_selector("#organise-dialog #organise-totals")
    preview = await pg.inner_text("#organise-dialog")
    await pg.screenshot(path=str(out / "23g-import-gather.png"))
    await pg.click("#organise-dialog button[type=submit]")
    said = await toast_text(pg, "Moved the games outside the library into it", timeout=30000)
    inside = sorted(x.relative_to(library).as_posix() for x in library.rglob("*") if x.suffix in (".gb", ".gg", ".zip") and "_library" not in x.parts)
    check("Move into the library… takes every game outside the library folder into it, each system's into a folder named for it there, as one change",
          "Move the games outside the library into it" in preview and "Game Boy/Alpha (Europe).gb" in inside and "Game Boy/Delta (World).zip" in inside
          and "Sega Game Gear/Gear One (World).gg" in inside and not (incoming / "europe one.gb").exists() and not (incoming / "gear.gg").exists()
          and not (gb / "Delta (World).zip").exists(), (preview[-300:], said, inside))
    await pg.click('.sidebar a[href="#/history"]')
    await pg.wait_for_selector('#history li:first-child [data-action="undo"]')
    await pg.click('#history li:first-child [data-action="undo"]')
    await toast_text(pg, "Undone")
    back = [(incoming / "europe one.gb").is_file(), (incoming / "gear.gg").is_file(), (gb / "Delta (World).zip").is_file()]
    bar = await pg.evaluate("""async (b) => { const { addJob } = await import(b + 'ui/state.js'); const done = addJob('Testing', null); done.update('Testing: 1 of 4', 0.25);
        await new Promise((r) => setTimeout(r, 100)); const p = document.querySelector('#status-progress'); const v = p && p.value; done(); return v; }""", base)
    check("Undo puts them back where they were; and the status bar shows how far running work has got", all(back) and bar == 0.25, (back, bar))


async def serve_to_devices(pg, base, home, library):
    """Phase 9a: Settings › Serve to devices starts Game Repo Server (`gamerepo-cli
    serve-devices` standing in for the program installed beside the app) on a port of its
    own, shows a code, the stand-in device pairs with it and shows in the list, Remove takes
    it off and the stand-in is refused, and the switch stops it. Started again, it stops
    when the app's backend ends (checked after this part)."""
    import socket
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    data, phone = home / "server", home / "phone"
    (phone / "ES-DE" / "settings").mkdir(parents=True)
    (phone / "ES-DE" / "settings" / "es_settings.xml").write_text('<?xml version="1.0"?>\n<string name="ROMDirectory" value="~/ROMs" />\n', encoding="utf-8")
    device = lambda *args: subprocess.run([a.cli, "device", *map(str, args), "--to", str(phone)], capture_output=True, text=True, encoding="utf-8", timeout=120)
    await pg.click('.sidebar a[href="#/settings"]')
    await pg.wait_for_selector("#settings-serve #serve-on")
    state = await pg.inner_text("#serve-state")
    check("Settings › Serve to devices: not serving at first", "Not serving" in state and not await pg.is_checked("#serve-on"), state)
    await pg.click("#settings-serve .serve-more > summary")
    await pg.fill("#serve-port", str(port))
    await pg.click("#serve-port-set")
    await pg.wait_for_selector("#serve-port-set[disabled]")
    kept = json.loads((home / "config/config.json").read_text()).get("serve", {})
    check("its port is kept in the app's settings", kept.get("port") == port and kept.get("on") is False, kept)
    await pg.check("#serve-on")
    await pg.wait_for_selector('#settings-serve[data-running="yes"] #serve-address', timeout=60000)
    address = await pg.inner_text("#serve-address")
    running = json.loads((data / "server.json").read_text())
    check("the switch starts the server with the open library, on that port", address.endswith(f":{port}") and running["port"] == port and Path(running["library"]) == library
          and json.loads((home / "config/config.json").read_text())["serve"]["on"] is True, (address, running))
    await pg.wait_for_function("() => /Paired devices can browse it/.test(document.querySelector('#serve-state')?.textContent || '')", timeout=60000)
    await pg.click("#serve-pair")
    await pg.wait_for_selector("#serve-code")
    code = (await pg.inner_text("#serve-code")).replace(" ", "")
    left = await pg.inner_text("#serve-code-left")
    check("Pair a device shows a six-digit code and how long it works", len(code) == 6 and code.isdigit() and left.startswith(("for 9:", "for 10:")), (code, left))
    paired = device("pair", f"127.0.0.1:{port}", code, "--name", "Page phone")
    await pg.wait_for_selector('#serve-devices li:has-text("Page phone")', timeout=15000)
    said = await pg.inner_text("#settings-serve")
    check("the stand-in device pairs with the code and shows in the list", paired.returncode == 0 and "Paired with Page phone." in said
          and await pg.query_selector("#serve-code") is None, (paired.stderr, said))
    await pg.screenshot(path=str(out / "60-serve-to-devices.png"), full_page=True)
    systems = device("systems")
    check("the paired device browses the library", systems.returncode == 0, systems.stderr)
    await pg.click('#serve-devices li:has-text("Page phone") [data-action="remove"]')
    await pg.click('#serve-devices li:has-text("Page phone") [data-action="remove-yes"]')
    await pg.wait_for_selector("#serve-devices-none")
    refused = device("systems")
    check("Remove takes the device off and it's refused", refused.returncode != 0 and "pair it again" in refused.stderr, refused.stderr)
    await pg.uncheck("#serve-on")
    await pg.wait_for_function("() => /Not serving/.test(document.querySelector('#settings-serve[data-running=\"no\"] #serve-state')?.textContent || '')", timeout=30000)
    await pg.wait_for_timeout(2500)  # and an answer asked for before it stopped doesn't say otherwise
    state = await pg.inner_text("#serve-state")
    check("the switch stops the server", not (data / "server.json").exists() and "Not serving" in state, ((data / "server.json").exists(), state))
    await pg.check("#serve-on")
    await pg.wait_for_selector('#settings-serve[data-running="yes"]', timeout=60000)


# the parts with homes of their own, run after the rest
async def variants(pg, base, home, library):
    """Phase 10 (D119–D123): files no DAT knows are on Audit's Unknown tab, which Home's Next
    opens, each with the game it's likely a variant of and its tags; Add puts one in the
    system's custom DAT as that game's variant, which the Library shows under its own name
    and tags, its page linking its original; Edit… names one and gives it a new tag from the
    drop-down, and its file is renamed to it as one change; Remove puts a file back on
    Unknown, and Undo puts the custom DAT back."""
    gba = home / "GBA"
    gba.mkdir()
    jp, tr, proto = make_fixtures.rom(41, 4096), make_fixtures.rom(42, 4096), make_fixtures.rom(43, 2048)
    (gba / "Dragon Quest Monsters - Caravan Heart (Japan).gba").write_bytes(jp)
    translated = "Dragon Quest Monsters - Caravan Heart (English Translated).gba"
    (gba / translated).write_bytes(tr)
    (gba / "Dark Eden (Prototype).gba").write_bytes(proto)
    dat = home / "gba.dat"
    dat.write_text(make_fixtures.logiqx("Nintendo - Game Boy Advance", "20261001", [
        ("Dragon Quest Monsters - Caravan Heart (Japan)", None, [make_fixtures.rom_xml("Dragon Quest Monsters - Caravan Heart (Japan).gba", jp)]),
        ("Golden Sun (USA)", None, [make_fixtures.rom_xml("Golden Sun (USA).gba", make_fixtures.rom(44, 1024))]),
    ]))
    pick(gba, base)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#add-root")
    await pg.wait_for_selector("#roots li")
    pick(dat, base)
    await pg.click("#import-dats")
    await toast_text(pg, "Imported")
    await back_home(pg, base)
    await pg.click("#home-scan-button")
    await toast_text(pg, "Scanned")
    await pg.wait_for_selector('#home-next li[data-kind="unknown"]', timeout=15000)
    nxt = await pg.inner_text('#home-next li[data-kind="unknown"]')
    await pg.click('#home-next li[data-kind="unknown"] a')
    row = f'#audit-list-table tr[data-name="{translated}"]'
    await pg.wait_for_selector(row, timeout=15000)
    dq = await pg.inner_text(row)
    eden = await pg.inner_text('#audit-list-table tr[data-name="Dark Eden (Prototype).gba"]')
    await pg.screenshot(path=str(out / "58-audit-unknown-variants.png"))
    check("Home's Next opens Audit's Unknown tab, each file no DAT knows with the game it's likely a variant of and its tags (D120, D123)",
          "2 files in Game Boy Advance no DAT knows need review" in nxt and "Dragon Quest Monsters - Caravan Heart (Japan)" in dq and "same title" in dq
          and "English Translated" in dq and "No game found" in eden and "Prototype" in eden, (nxt, dq, eden))
    await pg.click(f"{row} .unknown-add")
    await toast_text(pg, "Added")
    await pg.wait_for_function("() => document.querySelector('#audit-tabs [data-tab=\"variants\"] .tab-count')?.textContent === '1'", timeout=15000)
    custom = list((library / "_library" / "dats" / "gba").glob("*(custom).dat"))
    text = custom[0].read_text() if custom else ""
    check("Add puts it in the system's custom DAT, a variant of the game suggested (D119)",
          len(custom) == 1 and 'name="Dragon Quest Monsters - Caravan Heart (English Translated)"' in text
          and 'cloneof="Dragon Quest Monsters - Caravan Heart (Japan)"' in text and await pg.inner_text('#audit-tabs [data-tab="unknown"] .tab-count') == "1", (custom, text[-600:]))
    # a game of its own, named with a new tag from the drop-down
    await pg.click('#audit-list-table tr[data-name="Dark Eden (Prototype).gba"] .unknown-edit')
    await pg.wait_for_selector("#variant-dialog")
    await pg.select_option("#variant-tag-add", "__new")
    await pg.fill("#variant-new-tag", "Unreleased")
    await pg.press("#variant-new-tag", "Enter")
    await pg.wait_for_function("() => document.querySelector('#variant-name')?.textContent.includes('Dark Eden (Prototype) (Unreleased)')")
    own = await pg.input_value("#variant-original")
    await pg.click("#variant-dialog button[type=submit]")
    await toast_text(pg, "Added Dark Eden")
    tags = json.loads((library / "_library/library.json").read_text()).get("variant_tags", [])
    await pg.click('#audit-tabs [data-tab="variants"]')
    await pg.wait_for_selector('#variants-table tr[data-name="Dark Eden (Prototype) (Unreleased)"]', timeout=15000)
    listed = await pg.inner_text("#variants-table")
    await pg.screenshot(path=str(out / "58b-audit-variants.png"))
    check("Edit… names it with a tag added from the drop-down (New tag… keeps it in the list), a game of its own; the Variants tab lists both (D121)",
          own == "" and "Unreleased" in tags and "Prototype" in tags and "A game of its own" in listed and "Dragon Quest Monsters - Caravan Heart (Japan)" in listed
          and "its file is named otherwise" in listed, (own, tags, listed))
    await pg.click("#variants-rename")
    await pg.wait_for_selector("#organise-dialog #organise-totals")
    await pg.click("#organise-dialog button[type=submit]")
    await toast_text(pg, "of")
    await until(lambda: (gba / "Dark Eden (Prototype) (Unreleased).gba").is_file())
    await pg.wait_for_function("() => !document.querySelector('#variants-table')?.textContent.includes('named otherwise')", timeout=15000)
    check("Rename to their names renames its file to its name in the custom DAT", (gba / "Dark Eden (Prototype) (Unreleased).gba").is_file()
          and not (gba / "Dark Eden (Prototype).gba").exists(), sorted(x.name for x in gba.iterdir()))
    # in the Library under its own name, with its tag to filter by; its page links its original
    await pg.click('#nav-library')
    await pg.wait_for_selector('#filter-tag label[data-value="English Translated"]', timeout=15000)
    await pg.goto(base + "#/game/gba/" + urllib.parse.quote("Dragon Quest Monsters - Caravan Heart (English Translated)"))
    await pg.wait_for_selector("#game-variant-of", timeout=15000)
    of = await pg.inner_text("#game-variant-of")
    head = await pg.inner_text("#game-head")
    await pg.click("#game-variant-of a")
    await pg.wait_for_selector("#game-variant-list", timeout=15000)
    theirs = await pg.inner_text("#game-variant-list")
    check("the Library shows the variant under its own name with its tags, and its page says what it's a variant of, whose page lists it (D122)",
          "(English Translated)" in head and "A variant of Dragon Quest Monsters - Caravan Heart (Japan)" in of and "English Translated" in of
          and "Caravan Heart (English Translated)" in theirs, (head, of, theirs))
    # taken out, its file is on Unknown again; Undo puts the custom DAT back
    await pg.goto(base + "#/audit/gba")
    await pg.click('#audit-tabs [data-tab="variants"]')
    await pg.click('#variants-table tr[data-name="Dragon Quest Monsters - Caravan Heart (English Translated)"] .variant-remove')
    await toast_text(pg, "Took")
    await pg.wait_for_function("() => document.querySelector('#audit-tabs [data-tab=\"unknown\"] .tab-count')?.textContent === '1'", timeout=15000)
    await pg.click('#toast button:has-text("Undo")')
    await pg.wait_for_function("() => document.querySelector('#audit-tabs [data-tab=\"variants\"] .tab-count')?.textContent === '2'", timeout=15000)
    check("Remove puts its file back on Unknown, and Undo puts the custom DAT back as it was",
          await pg.inner_text('#audit-tabs [data-tab="unknown"] .tab-count') == "0", await pg.inner_text("#audit-tabs"))
    # D118: a second DAT for the system, ticked Include in Audit in DATs…, knows a file the
    # main one doesn't: it's off Unknown, its counts are under the totals with Show its lists
    mother = make_fixtures.rom(45, 2048)
    (gba / "Mother 3 (English Translated).gba").write_bytes(mother)
    tr_dat = home / "translations.dat"
    tr_dat.write_text(make_fixtures.logiqx("Translations", "2024-05", [("Mother 3 (English Translated)", None, [make_fixtures.rom_xml("Mother 3 (English Translated).gba", mother)])]))
    await pg.click("#audit-scan")
    await pg.wait_for_function("() => document.querySelector('#audit-tabs [data-tab=\"unknown\"] .tab-count')?.textContent === '1'", timeout=30000)
    pick(tr_dat, base)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#import-dats")
    await pg.wait_for_selector("#dat-platform-dialog")
    await pg.select_option("#dat-platform-0", "gba")
    await pg.click("#dat-platform-dialog button[type=submit]")
    await toast_text(pg, "Imported Translations")
    await pg.goto(base + "#/audit/gba")
    await pg.click("#audit-dats")
    await pg.wait_for_selector('#audit-dats-list li[data-dat="Translations"]')
    listed = await pg.inner_text("#audit-dats-list")
    await pg.check('#audit-dats-list li[data-dat="Translations"] .audit-also')
    await pg.click("#audit-dats-dialog button[type=submit]")
    await pg.wait_for_selector("#audit-also", timeout=15000)
    also = await pg.inner_text("#audit-also")
    unknown = await pg.inner_text('#audit-tabs [data-tab="unknown"] .tab-count')
    kept = json.loads((library / "_library/library.json").read_text()).get("also_dats", {})
    await pg.screenshot(path=str(out / "58c-audit-also.png"))
    await pg.click("#audit-also .audit-also-view")
    await pg.wait_for_selector("#audit-viewing")
    viewing = await pg.inner_text("#audit-viewing")
    have = await pg.inner_text("#audit-have")
    tabs = await pg.eval_on_selector_all("#audit-tabs [data-tab]", "els => els.map(e => e.dataset.tab)")
    await pg.click("#audit-viewing-back")
    await pg.wait_for_selector("#audit-also")
    check("DATs… ticks a second DAT to audit with too (kept by name): the file it knows is off Unknown, its counts are under the totals, and Show its lists shows its own (D118)",
          "Custom - Game Boy Advance" in listed and "Nintendo - Game Boy Advance" in listed and kept == {"gba": ["Translations"]} and unknown == "0"
          and "Translations (2024-05) 1 of 1" in also and "Translations" in viewing and have == "1" and "unknown" not in tabs and "variants" not in tabs,
          (listed, kept, unknown, also, viewing, have, tabs))
    # D131: the DAT files list says which is the system's main DAT and which it's audited with
    # too, and makes another the main one, as Audit's DATs… does
    await pg.goto(base + "#/import/settings-dats")
    # (read again as it's shown, so it has what Audit's DATs… just changed)
    await pg.wait_for_selector('#settings-dats .dat-group[data-platform="gba"] li[data-name="Translations"] .dat-also', timeout=15000)
    roles = await pg.evaluate("""() => [...document.querySelectorAll('#settings-dats .dat-group[data-platform="gba"] li')].map((l) => [l.dataset.name,
        l.querySelector('.dat-main') ? 'main' : l.querySelector('.dat-also') ? 'also' : '', !!l.querySelector('[data-action="main-dat"]')])""")
    await pg.screenshot(path=str(out / "58d-dat-files-main.png"))
    await pg.click('#settings-dats .dat-group[data-platform="gba"] li[data-name="Translations"] [data-action="main-dat"]')
    said = await toast_text(pg, "main DAT now")
    await pg.wait_for_selector('#settings-dats li[data-name="Translations"] .dat-main', timeout=15000)
    meta = json.loads((library / "_library/library.json").read_text())
    await pg.goto(base + "#/audit/gba")
    await pg.wait_for_selector("#audit-dat")
    picked = await pg.evaluate("() => document.querySelector('#audit-dat').selectedOptions[0]?.textContent || ''")
    check("the DAT files list marks the main DAT and the one audited with too, and Make it the main DAT changes it, as Audit shows (D131)",
          sorted(roles) == sorted([["Custom - Game Boy Advance", "", False], ["Nintendo - Game Boy Advance", "main", False], ["Translations", "also", True]])
          and meta.get("audit_dats", {}).get("gba") == "Translations" and "gba" not in meta.get("also_dats", {}) and picked.startswith("Translations"),
          (roles, said, meta.get("audit_dats"), meta.get("also_dats"), picked))


def stand_in_update(base, to=None):
    """The CLI's stand-in for the updater (desktop/cli/src/updates.rs): what a check finds
    and how this copy stands (`to`), and what the page asked for."""
    req = urllib.request.Request(base + "test/update", data=None if to is None else json.dumps(to).encode(),
                                 method="GET" if to is None else "POST", headers={"content-type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=5).read())


async def updates_in_the_app(pg, base, home, library):
    """In-app updates (D134), with the stand-in for the updater: the check as the app starts,
    About, the status bar, Settings › Updates, Update and restart, and the server's window."""
    # 1. the check as it started found nothing newer
    await pg.click('.sidebar a[href="#/about"]')
    await pg.wait_for_selector("#about-versions #update-panel[data-state=newest]")
    newest = await pg.inner_text("#update-state")
    started = stand_in_update(base)
    check("the app checks for updates as it starts; About says this version is the newest",
          started["checks"] == 1 and "is the newest version" in newest and not await pg.query_selector("#status-update"), (started["checks"], newest))

    # 2. Check for updates finds a newer one: what's new since this version, and the status bar says so
    stand_in_update(base, {"found": {"version": "9.9.9", "date": "2026-10-22T00:00:00Z",
                                     "notes": "## 9.9.9 (2026-10-22)\n\n- Something **new**.\n\n## 0.0.1 (2020-01-01)\n\n- Something old."}})
    await pg.click("#update-check")
    await pg.wait_for_selector("#update-panel[data-state=available] #update-install")
    state, new = await pg.inner_text("#update-state"), await pg.inner_text("#update-new")
    await pg.wait_for_selector("#status-update")
    bar = await pg.inner_text("#status-update")
    check("Check for updates finds a newer version, with what's new since this one, and the status bar says it's out",
          "Game Repo 9.9.9 is out" in state and "Something new" in new and "Something old" not in new and "9.9.9 is out" in bar, (state, new, bar))
    await pg.screenshot(path=str(out / "07-update-found.png"), full_page=True)

    # 3. the status bar opens Settings › Updates; its tick is saved
    await pg.click("#status-update")
    await pg.wait_for_selector("#settings-updates #update-at-start")
    ticked = await pg.is_checked("#settings-updates #update-at-start")
    await pg.click("#settings-updates #update-at-start")
    await until(lambda: stand_in_update(base)["check_at_start"] is False)
    check("the status bar opens Settings › Updates, and \"Check for updates when Game Repo starts\" is saved", ticked and not await pg.is_checked("#settings-updates #update-at-start"))

    # 4. Update and restart
    await pg.click("#settings-updates #update-install")
    await pg.wait_for_selector("#settings-updates #update-panel[data-state=busy]")
    restarting = await pg.inner_text("#settings-updates #update-state")
    check("Update and restart installs it", stand_in_update(base)["installs"] == 1 and "Starting Game Repo 9.9.9" in restarting
          and not await pg.query_selector("#status-update"), restarting)

    # 5. not checked as it starts when the tick is off; checked again when it's on
    stand_in_update(base, {"checked": None, "available": None, "restarting": False})
    await back_home(pg, base)
    await pg.wait_for_timeout(500)
    quiet = await pg.query_selector("#status-update") is None and stand_in_update(base)["checks"] == 2
    stand_in_update(base, {"check_at_start": True})
    await back_home(pg, base)
    await pg.wait_for_selector("#status-update")
    check("with the tick off it doesn't check as it starts; with it on, the status bar says a newer version is out", quiet and stand_in_update(base)["checks"] == 3)

    # 6. a copy that can't update itself says why, with no buttons
    why = "This copy of Game Repo wasn't put here by its installer, so it can't update itself."
    stand_in_update(base, {"can": False, "why_not": why, "available": None})
    await pg.goto(base + "#/about")
    await pg.wait_for_selector("#update-panel[data-state=off]")
    check("a copy that can't update itself says why", why in await pg.inner_text("#update-why") and not await pg.query_selector("#update-check"))

    # 7. Game Repo Server's window has the panel too, with its tick
    stand_in_update(base, {"can": True, "why_not": None, "checked": None})
    await pg.goto(base + "server.html")
    await pg.wait_for_selector("#server-updates #update-panel #update-check")
    check("Game Repo Server's window has Updates, with Check for updates and the tick", await pg.query_selector("#server-updates #update-at-start") is not None)
    stand_in_update(base, {"found": None, "available": None})


async def addons_by_name(pg, base, home, library):
    """D160–D163: a 3DS update and DLC no DAT lists, named as sets name them (a title ID, a
    [DLC] tag), are on Audit's Unknown tab as an update and a DLC of their game; added, the
    Library counts them with it, lists them as games of their own when Updates and DLC with
    their game is unticked, and an update's page says which game it belongs to."""
    n3ds = home / "3DS"
    n3ds.mkdir()
    alpha = make_fixtures.rom(61, 4096)
    (n3ds / "Alpha (USA).3ds").write_bytes(alpha)
    update = "Alpha (USA) [0004000E00055D00][v1024].cia"
    dlc = "Alpha - Bonus Pack (USA) [DLC].cia"
    (n3ds / update).write_bytes(make_fixtures.rom(62, 2048))
    (n3ds / dlc).write_bytes(make_fixtures.rom(63, 1024))
    dat = home / "3ds.dat"
    dat.write_text(make_fixtures.logiqx("Nintendo - Nintendo 3DS", "20261001", [
        ("Alpha (USA)", None, [make_fixtures.rom_xml("Alpha (USA).3ds", alpha)]),
        ("Beta (Europe)", None, [make_fixtures.rom_xml("Beta (Europe).3ds", make_fixtures.rom(64, 1024))]),
    ]))
    pick(n3ds, base)
    await pg.click('.sidebar a[href="#/import"]')
    await pg.click("#add-root")
    await pg.wait_for_selector("#roots li")
    pick(dat, base)
    await pg.click("#import-dats")
    await toast_text(pg, "Imported")
    await back_home(pg, base)
    await pg.click("#home-scan-button")
    await toast_text(pg, "Scanned")
    await pg.goto(base + "#/audit/n3ds")
    await pg.click('#audit-tabs [data-tab="unknown"]')
    await pg.wait_for_selector(f'#audit-list-table tr[data-name="{update}"]', timeout=15000)
    upd = await pg.inner_text(f'#audit-list-table tr[data-name="{update}"]')
    bonus = await pg.inner_text(f'#audit-list-table tr[data-name="{dlc}"]')
    kinds = await pg.eval_on_selector_all("#audit-list-table tr[data-addon]", "els => els.map(e => e.dataset.addon).sort()")
    await pg.screenshot(path=str(out / "58e-audit-unknown-addons.png"))
    check("Audit's Unknown tab says a file named as an update or DLC (a 3DS title ID, a [DLC] tag) is one, of the game its name says (D160–D162)",
          "an update of Alpha (USA)" in " ".join(upd.split()) and "a DLC of Alpha (USA)" in " ".join(bonus.split()) and kinds == ["dlc", "update"], (upd, bonus, kinds))
    await pg.check('#audit-list-table th.tick input')
    await pg.click("#unknown-add-ticked")
    await toast_text(pg, "Added")
    await pg.wait_for_function("() => document.querySelector('#audit-tabs [data-tab=\"unknown\"] .tab-count')?.textContent === '0'", timeout=15000)
    # the Library: counted with their game, or games of their own beside it
    await pg.goto(base + "#/library")
    await pg.wait_for_selector("#lib-addons-with", timeout=15000)
    ticked = await pg.is_checked("#lib-addons-with")
    counted = await pg.inner_text('label:has(#lib-addons-with)')
    alone = await pg.eval_on_selector_all(".lib-tile[data-key]", "els => els.map(e => e.dataset.key)")
    await pg.uncheck("#lib-addons-with")
    await pg.wait_for_selector('.lib-tile[data-addon="update"]', timeout=15000)
    apart = await pg.eval_on_selector_all(".lib-tile[data-key]", "els => els.map(e => [e.dataset.key, e.dataset.addon || '', e.querySelector('.lib-badge-addon')?.textContent || ''])")
    await pg.screenshot(path=str(out / "58f-library-addons-apart.png"))
    check("the Library counts updates and DLC with their game, and unticked lists each beside it with what it is, the choice in the address (D163)",
          ticked and "2" in counted and alone == ["n3ds/Alpha (USA)"]
          and apart == [["n3ds/Alpha (USA)", "", ""], ["n3ds/Alpha (USA) [0004000E00055D00][v1024]", "update", "Update v1024"], ["n3ds/Alpha - Bonus Pack (USA) [DLC]", "dlc", "DLC"]]
          and "apart=1" in pg.url, (ticked, counted, alone, apart, pg.url))
    # (a double-click would open the panel, which moves the tiles under the second click)
    await pg.goto(base + "#/game/n3ds/" + urllib.parse.quote("Alpha (USA) [0004000E00055D00][v1024]"))
    await pg.wait_for_selector("#game-addon-of", timeout=15000)
    of = " ".join((await pg.inner_text("#game-addon-of")).split())
    await pg.click("#game-addon-of a")
    await pg.wait_for_selector("#game-addons", timeout=15000)
    theirs = await pg.inner_text("#game-addons")
    check("an update's page says what it is and links its game, whose page lists it (D163)",
          of.startswith("Update v1024 of Alpha") and "Link to another game" in of and "Alpha (USA) [0004000E00055D00][v1024]" in theirs
          and "Alpha - Bonus Pack (USA) [DLC]" in theirs and "You have 2 of 2" in theirs, (of, theirs))


async def add_systems(pg, base, home, library):
    """D165–D167: Add a system… makes PS5's DAT from a folder of its games (the folder added
    for it and scanned), imports a DAT file for Switch whatever its name says, and downloads
    PS2's from libretro, whose one-track copy Audit offers to swap for Redump's own; Get
    Redump's DATs… brings in Redump's (used before libretro's copies) and says which it
    couldn't get."""
    # stand-ins for libretro's copies and redump.org (whose /datfile/<system>/ is a folder's page here)
    web = home / "dat-web"
    (web / "metadat/no-intro").mkdir(parents=True)
    (web / "metadat/redump").mkdir(parents=True)
    t1, t2 = make_fixtures.rom(71, 2048), make_fixtures.rom(72, 1024)
    (web / "metadat/redump/Sony - PlayStation 2.dat").write_text(make_fixtures.logiqx("Sony - PlayStation 2", "2026.10.07", [
        ("Gamma (USA)", None, [make_fixtures.rom_xml("Gamma (USA) (Track 1).bin", t1)])]))
    for slug in ("ps2", "ps3", "psx"):
        (web / "datfile" / slug).mkdir(parents=True)
    with zipfile.ZipFile(web / "datfile/ps2/index.html", "w") as z:
        z.writestr("Sony - PlayStation 2 - Datfile (1) (2026-10-01 01-02-03).dat", make_fixtures.logiqx("Sony - PlayStation 2", "2026-10-01 01-02-03", [
            ("Gamma (USA)", None, [make_fixtures.rom_xml("Gamma (USA) (Track 1).bin", t1), make_fixtures.rom_xml("Gamma (USA) (Track 2).bin", t2)])]))
    (web / "datfile/ps3/index.html").write_text("<!DOCTYPE html><html><body>Please sign in</body></html>")
    (web / "datfile/psx/index.html").write_text(make_fixtures.logiqx("Sony - PlayStation", "2026-10-02 01-02-03", [
        ("Theta (Europe)", None, [make_fixtures.rom_xml("Theta (Europe).bin", make_fixtures.rom(76, 1024))])]))
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(web))
    handler.log_message = lambda *args: None
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    cfg_path = home / "config/config.json"
    cfg = json.loads(cfg_path.read_text()) if cfg_path.exists() else {}
    cfg.update({"dat_source": f"http://127.0.0.1:{srv.server_port}/metadat/no-intro/", "redump_source": f"http://127.0.0.1:{srv.server_port}/datfile/"})
    cfg_path.write_text(json.dumps(cfg))
    try:
        # PS5, from a folder of its games
        ps5 = home / "PS5 dumps"
        ps5.mkdir()
        (ps5 / "Delta (USA).iso").write_bytes(make_fixtures.rom(73, 4096))
        (ps5 / "Epsilon (Japan).iso").write_bytes(make_fixtures.rom(74, 2048))
        await pg.click("#home-add-system")
        await pg.wait_for_selector("#add-system-dialog")
        offered = await pg.eval_on_selector_all('#add-system-pick optgroup[label="No DAT yet"] option', "els => els.map(e => e.value)")
        await pg.select_option("#add-system-pick", "ps5")
        ways = await pg.eval_on_selector_all("#add-system-way .way", "els => els.map(e => e.dataset.way)")
        await pg.check('#add-system-way input[value="folder"]')
        await pg.fill("#add-system-folder", str(ps5))
        dat_name = await pg.input_value("#add-system-dat-name")
        await pg.screenshot(path=str(out / "70-add-system.png"))
        await pg.click("#add-system-dialog button[type=submit]")
        await pg.wait_for_selector('#audit-pick[data-platform="ps5"]', timeout=30000)
        counts = (await pg.inner_text("#audit-have"), await pg.inner_text("#audit-games"))
        roots = json.loads((library / "_library/library.json").read_text()).get("roots", [])
        made = sorted(x.name for x in (library / "_library/made-dats").glob("*.dat"))
        check("Add a system… lists the systems without a DAT (PS3, PS4, PS5 and Xbox 360 among them) and makes PS5's DAT from a folder of its games, added for it and scanned (D165, D166)",
              {"ps3", "ps4", "ps5", "xbox360", "switch"} <= set(offered) and ways == ["download", "file", "folder"] and dat_name == "My PlayStation 5 games"
              and counts == ("2", "2") and any(r.get("platform") == "ps5" for r in roots) and made and made[0].startswith("My PlayStation 5 games"), (offered[:5], ways, dat_name, counts, roots, made))

        # Switch, from a DAT file whose name says nothing of it
        sw = home / "switch.dat"
        sw.write_text(make_fixtures.logiqx("Someone - Handheld Dumps", "1", [("Zeta (World)", None, [make_fixtures.rom_xml("Zeta (World).nsp", make_fixtures.rom(75, 512))])]))
        await pg.click("#audit-add-system")
        await pg.select_option("#add-system-pick", "switch")
        await pg.check('#add-system-way input[value="file"]')
        pick(sw, base)
        await pg.click("#add-system-dialog button[type=submit]")
        await pg.wait_for_selector('#audit-pick[data-platform="switch"]', timeout=30000)
        games = await pg.inner_text("#audit-games")
        check("Add a system… imports a DAT file for the system chosen, whatever the DAT's name says (D166)",
              games == "1" and any((library / "_library/dats/switch").glob("Someone - Handheld Dumps*.dat")), games)

        # PS2, downloaded from libretro; its one-track copy swapped for Redump's own on Audit
        await pg.click("#audit-add-system")
        await pg.select_option("#add-system-pick", "ps2")
        sources = await pg.eval_on_selector_all("#add-system-source option", "els => els.map(e => e.value)")
        await pg.click("#add-system-dialog button[type=submit]")
        await pg.wait_for_selector('#audit-pick[data-platform="ps2"]', timeout=30000)
        await pg.wait_for_selector("#audit-libretro")
        await pg.click("#audit-get-redump")
        await pg.wait_for_selector("#redump-dialog")
        ticked = await pg.eval_on_selector_all("#redump-systems input:checked", "els => els.map(e => e.closest('[data-platform]').dataset.platform)")
        await pg.screenshot(path=str(out / "71-redump.png"))
        await pg.click("#redump-dialog button[type=submit]")
        said = await toast_text(pg, "Redump's DATs for")
        await pg.wait_for_selector("#audit-libretro", state="detached", timeout=15000)
        head = await pg.inner_text("#audit-head")
        check("Add a system… downloads libretro's copy (Redump's own offered beside it), and Audit offers Redump's DAT for it, then used before libretro's copy though dated earlier (D167)",
              sources == ["libretro", "redump"] and ticked == ["ps2"] and "PlayStation 2" in said and "2026-10-01 01-02-03" in head, (sources, ticked, said, head))

        # Import › DAT files: Get Redump's DATs…, one it couldn't get
        await pg.click('.sidebar a[href="#/import"]')
        await pg.click("#redump-dats")
        await pg.wait_for_selector("#redump-dialog")
        await pg.click("#redump-none")
        await pg.check('#redump-systems [data-platform="psx"] input')
        await pg.check('#redump-systems [data-platform="ps3"] input')
        await pg.click("#redump-dialog button[type=submit]")
        said = await toast_text(pg, "Couldn't get")
        await pg.wait_for_selector('#settings-dats .dat-group[data-platform="psx"]')
        ps2 = " ".join((await pg.inner_text('#settings-dats .dat-group[data-platform="ps2"]')).split())
        check("Get Redump's DATs… imports those it's given and says which it couldn't get, and the DAT list says which is libretro's copy",
              "Imported Redump's DATs for PlayStation" in said and "PlayStation 3" in said and "signed in" in said and ps2.count("libretro's copy") == 1, (said, ps2))
    finally:
        srv.shutdown()


OWN_HOMES = [("arcade", arcade), ("magazines", magazines), ("import", import_by_system), ("variants", variants), ("serve", serve_to_devices),
             ("updates", updates_in_the_app), ("videos", videos), ("addons", addons_by_name), ("systems", add_systems)]


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

            async with Section(pg, B, "the first start, the menu, Help and the library"):
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
                      labels == ["Game Repo", "Home", "Library", "PC games", "Magazines", "Videos", "Import", "Audit", "BIOS files", "Metadata", "Documents", "Collections", "Backlog", "Export layouts", "History", "Settings", "Help", "About"]
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

            async with Section(pg, B, "Phase 1: folders with games, scanning, DATs and the audit"):
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
                # review M41 (D78): the import is the Import page's, opened at its DAT files
                home_import_at = await pg.evaluate("() => [location.hash, !!document.querySelector('#import-page[data-at=\"settings-dats\"] #settings-dats')]")
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
                check("Home's Import DATs… opens the Import page at its DAT files and imports there (review M41, D78)", home_import_at == ["#/import/settings-dats", True], home_import_at)
                # a file dropped where no page takes it goes to the Import page, asked what it is (D78)
                await pg.goto(B + "#/")
                await pg.wait_for_selector("#home-platforms")
                await pg.evaluate("(p) => window.__shimEmit('tauri://drag-drop', { paths: [p], position: { x: 400, y: 400 } })", str(fx / "dats" / "gb.dat"))
                await pg.wait_for_selector("#import-drop-dialog")
                dropped_as = await pg.evaluate("() => [location.hash, document.querySelector('#import-drop-kind input:checked')?.value]")
                await pg.click("#import-drop-dialog-cancel")
                check("a DAT dropped on Home goes to the Import page, taken for a DAT (D78)", dropped_as == ["#/import/import-dropped", "dats"], dropped_as)
                await pg.goto(B + "#/")
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
                said = await toast_text(pg, "list of 3 missing games")
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

            async with Section(pg, B, "Phase 2a: the watcher, organising, Undo, naming and History"):
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
                # one line says what it does; the rest is folded under More about this (D147)
                more = pg.locator("#organise-dialog #organise-more")
                folded = [await more.count(), await more.evaluate("m => m.open"), await pg.locator("#organise-more > p").first.is_visible()]
                await pg.click("#organise-more > summary")
                opened = [await more.evaluate("m => m.open"), await pg.locator("#organise-more > p", has_text="TorrentZip").is_visible()]
                check("Organise… keeps what it needn't say at once under More about this, which opens on a click (D147)",
                      folded == [1, False, False] and opened == [True, True], (folded, opened))
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
                days = await pg.eval_on_selector_all("#history .day-group", "els => els.map(e => [e.open, e.querySelector('summary').textContent])")
                await pg.click('#history li:first-child [data-action="steps"]')
                await pg.wait_for_selector("#history li:first-child .change-step")
                steps_shown = await pg.eval_on_selector_all("#history li:first-child .change-step", "els => els.map(e => e.dataset.op)")
                await pg.screenshot(path=str(out / "16-history.png"), full_page=True)
                await pg.click('#history li:first-child [data-action="steps"]')
                check("History is grouped by day, the newest open, and a change opens to the steps it took (D78)",
                      len(days) == 1 and days[0][0] and days[0][1].startswith("Today") and "3 changes" in days[0][1] and len(steps_shown) > 0, (days, steps_shown))
                await pg.click('#history li:first-child [data-action="undo"]')
                await toast_text(pg, "Undone")
                gone = not sorted_dir.exists() or not any(sorted_dir.iterdir())
                # what's set aside, looked through (D168): under its change, and on the Set aside tab
                await pg.click('#history li[data-kind="aside"] [data-action="aside"]')
                await pg.wait_for_selector('#history li[data-kind="aside"] .aside-file')
                under = await pg.eval_on_selector_all('#history li[data-kind="aside"] .aside-file', "els => els.map(e => e.textContent)")
                await pg.click('#history-tabs [data-tab="aside"]')
                await pg.wait_for_selector("#aside-changes .aside-file")
                aside_tab = await pg.eval_on_selector_all("#aside-changes .aside-change", "els => els.map(e => [e.querySelector('h3').textContent, e.querySelectorAll('.aside-file').length])")
                await pg.locator("#aside-changes .aside-file input").first.check()
                await pg.click('#aside-changes [data-action="aside-delete"]')
                ask = await pg.inner_text("#confirm-dialog")
                await pg.screenshot(path=str(out / "16b-history-aside.png"))
                await pg.click("#confirm-dialog-cancel")
                await pg.click('#history-tabs [data-tab="changes"]')
                check("History shows what a change set aside, under it and on the Set aside tab, each file to delete for good (D168)",
                      len(under) == 1 and "alpha again.gb" in under[0] and "from" in under[0] and aside_tab and aside_tab[0][1] == 1
                      and "duplicates" in aside_tab[0][0].lower() and "can't be undone" in ask, (under, aside_tab, ask))
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

            async with Section(pg, B, "Phase 2b: disc images converted and extracted"):
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
                # PlayStation, shown before, has one game too: wait for the PSP's own audit
                await pg.wait_for_selector('#audit-pick[data-platform="psp"]', state="attached")
                await pg.wait_for_function("() => document.querySelector('#audit-have')?.textContent === '1'")
                await pg.click("#audit-convert")
                await pg.wait_for_selector("#convert-dialog")
                try:
                    await pg.wait_for_function("() => document.querySelector('#convert-to')?.value === 'zso'", timeout=5000)
                except Exception:
                    # what the dialog and the backend said, to see why (a FAIL below, not a crash)
                    seen = await pg.evaluate("""async () => ({ value: document.querySelector('#convert-to')?.value,
                      formats: await window.__TAURI__.core.invoke('api', { cmd: 'formats', args: {} }).then((all) => all.find((x) => x.platform === 'psp')).catch((e) => String(e?.message || e)),
                      library: await window.__TAURI__.core.invoke('api', { cmd: 'app_info', args: {} }).then((i) => i.library).catch((e) => String(e?.message || e)) })""")
                    print("convert dialog:", seen, "library.json formats:", json.loads((library / "_library/library.json").read_text()).get("formats"))
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

            async with Section(pg, B, "Phase 3: systems, collections, exports and Import"):
                # Phase 3. Settings › Systems, the DAT question with a system added there, a collection made,
                # changed by hand, saved as DATs, exported to ES-DE, updated and checked
                await pg.wait_for_selector("#settings-systems")
                # (Add a system…, D166: one Game Repo doesn't know, with nothing yet)
                await pg.click("#system-add")
                await pg.wait_for_selector("#add-system-new")
                await pg.fill("#add-system-name", "Acme Gizmo")
                await pg.click("#add-system-details summary")
                await pg.fill("#add-system-dat-names", "Acme - Gizmo")
                await pg.fill("#add-system-exts", "gzm")
                await pg.select_option("#add-system-media", "cd")
                await pg.click("#add-system-dialog button[type=submit]")
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
                    await pg.wait_for_selector("#bar-remove-from")
                    await pg.click("#bar-remove-from")

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
                check("New collection offers by rules, from folders, from a DAT or empty, and one by rules opens on its rules", ways == ["rules", "folders", "dat", "empty"], ways)
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
                # In it / Removed by hand: a filter of the one games view (finding 6)
                await pg.select_option("#collection-show", "out")
                await pg.click(f'#collection-result[data-show="out"] #lib-games [data-key="gb/{names[0]}"]')
                await pg.wait_for_selector("#bar-put-back")
                shown_out = await pg.eval_on_selector_all("#lib-games [data-key]", "els => els.map(e => e.dataset.key)")
                await pg.keyboard.press("Escape")
                stored = json.loads(next((library / "_library/collections").glob("*.json")).read_text())
                check("a game is removed from a collection by hand (Remove from collection), and listed with Put back (changes made quickly are all kept)",
                      stored["removed"] == [f"gb/{names[0]}"] and stored["rules"]["one_per_title"] is True and stored["rules"]["include"] == "have" and shown_out == [f"gb/{names[0]}"],
                      (stored["removed"], stored["rules"], shown_out))
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
                check("a collection's History lists each change newest first (by hand, its rules, made), and Undo puts the game removed back",
                      hows[0] == "hand" and "rules" in hows and hows[-1] == "made" and names[0] in first_text and undone == "yes", (hows, first_text, undone))
                await pg.click('#collection-view [data-view="rules"]')
                check("a collection takes its games' updates and DLC by default", await pg.is_checked("#rule-addons"))
                await pg.click('#collection-view [data-view="games"]')
                await pg.select_option("#collection-show", "in")
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

                await pg.select_option("#collection-show", "out")
                await pg.click(f'#collection-result[data-show="out"] #lib-games [data-key="gb/{names[0]}"]')
                await pg.click("#bar-put-back")
                await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n", arg=len(names))
                await pg.select_option("#collection-show", "in")
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
                # RetroArch, Pegasus, LaunchBox and Playnite (Phase 8a), each written where that
                # frontend looks, not only offered (review M44)
                fe = home / "Frontends"
                for layout, name, places in (("retroarch", "RetroArch", {"roms": fe / "RA games", "retroarch": fe / "RetroArch"}),
                                             ("pegasus", "Pegasus", {"roms": fe / "Pegasus"}),
                                             ("launchbox", "LaunchBox", {"launchbox": fe / "LaunchBox"}),
                                             ("playnite", "Playnite", {"roms": fe / "Playnite"})):
                    await pg.click("#collection-export")
                    await pg.wait_for_selector(f'#export-layout option[value="{layout}"]', state="attached")
                    await pg.select_option("#export-layout", layout)
                    for k, v in places.items():
                        await pg.fill(f"#export-place-{k}", str(v))
                    await pg.click("#export-dialog button[type=submit]")
                    await pg.wait_for_selector("#organise-totals")
                    await pg.click("#export-dialog button[type=submit]")
                    await toast_text(pg, f"Exported Best Of to {name}", timeout=30000)
                written = sorted(str(x.relative_to(fe)).replace("\\", "/") for x in fe.rglob("*") if x.is_file())
                n = len(names)
                lpl = fe / "RetroArch/playlists/Nintendo - Game Boy.lpl"
                items = json.loads(lpl.read_text())["items"] if lpl.is_file() else []
                ra_games = [x for x in (fe / "RA games/gb").iterdir()] if (fe / "RA games/gb").is_dir() else []
                peg = fe / "Pegasus/gb/metadata.pegasus.txt"
                peg_text = peg.read_text() if peg.is_file() else ""
                lb = fe / "LaunchBox/Data/Platforms/Nintendo Game Boy.xml"
                lb_text = lb.read_text() if lb.is_file() else ""
                lb_games = list((fe / "LaunchBox/Games/Nintendo Game Boy").iterdir()) if (fe / "LaunchBox/Games/Nintendo Game Boy").is_dir() else []
                pn_games = list((fe / "Playnite/Nintendo Game Boy").iterdir()) if (fe / "Playnite/Nintendo Game Boy").is_dir() else []
                check("Export to RetroArch writes the games and a playlist named as RetroArch's database names the system, each game's path in it",
                      len(ra_games) == n and len(items) == n and all(Path(i["path"]).parent == fe / "RA games/gb" and i["db_name"] == "Nintendo - Game Boy.lpl" for i in items),
                      (written, items[:2]))
                check("Export to Pegasus writes the games with a metadata.pegasus.txt beside them, a game: entry for each",
                      len([x for x in (fe / "Pegasus/gb").iterdir() if x.is_file() and x.name != "metadata.pegasus.txt"]) == n and peg_text.count("\ngame: ") + peg_text.startswith("game: ") == n
                      and "collection:" in peg_text, (written, peg_text[:300]))
                check("Export to LaunchBox puts the games in its Games folder and lists them in its platform's file in Data/Platforms",
                      len(lb_games) == n and lb_text.count("<Game>") == n and "Nintendo Game Boy" in lb_text, (written, lb_text[:300]))
                check("Export to Playnite puts the games in a folder named as Playnite names the platform, for its scanner",
                      len(pn_games) == n and all(x.is_file() for x in pn_games), written)
                await pg.click('#collection-view [data-view="size"]')
                await pg.select_option("#collection-limit", "32")
                await pg.wait_for_selector('#collection-fit[data-over="false"]')
                stored = json.loads(next((library / "_library/collections").glob("*.json")).read_text())
                check("a card's size is its label's (32 GB card: 32,000,000,000 bytes), and the collection fits",
                      stored.get("size_limit") == 32_000_000_000 and "to spare" in await pg.inner_text("#collection-fit"), stored.get("size_limit"))
                await pg.click('#collection-view [data-view="exports"]')
                await pg.screenshot(path=str(out / "23-exports.png"), full_page=True)

                # collection groups (D75): a group added from the ⋯ menu, by rules, after the first
                coll_file = next((library / "_library/collections").glob("*.json"))
                await pg.click("#collection-menu")
                await pg.click('#context-menu [data-action="collection-add-group"]')
                await pg.wait_for_selector("#collection-new-group")
                group_ways = await pg.eval_on_selector_all("#collection-way .way", "els => els.map(e => e.dataset.way)")
                submit_label = await pg.inner_text("#collection-new-dialog button[type=submit]")
                await pg.fill("#collection-name", "Every one")
                await pg.click("#collection-new-dialog button[type=submit]")
                await pg.wait_for_selector("#collection-group-1 #rule-1-include")
                await pg.select_option("#rule-1-include", "all")
                await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent > n", arg=len(names))
                total = int(await pg.inner_text("#collection-games"))
                stored = json.loads(coll_file.read_text())
                heads = await pg.eval_on_selector_all("#collection-main .collection-group h2", "els => els.map(e => e.textContent)")
                check("Add a group… offers the New collection dialog's ways, and the group by rules is kept after the first, with its own rules",
                      group_ways == ["rules", "folders", "dat", "empty"] and submit_label == "Add the group" and heads == ["Best Of", "Every one"]
                      and [g["name"] for g in stored.get("groups", [])] == ["Every one"] and stored["groups"][0]["rules"]["include"] == "all"
                      and stored["groups"][0]["rules"]["platforms"] == ["gb"] and stored["rules"]["include"] == "have", (group_ways, submit_label, heads, stored.get("groups")))
                # the games view: a section for each group, with its name and count, none twice
                await pg.click('#collection-view [data-view="games"]')
                await pg.wait_for_function("() => document.querySelectorAll('#lib-games .lib-group').length === 2")
                await pg.wait_for_timeout(300)
                sections = await pg.eval_on_selector_all("#lib-games .lib-group", "els => els.map(e => [e.querySelector('h2').textContent, parseInt(e.querySelector('.muted').textContent)])")
                keys = await coll_names()
                check("a collection's games are a section for each group, with its name and count, and no game is in two",
                      [n for n, _ in sections] == ["Best Of", "Every one"] and sections[0][1] == len(names) and sum(n for _, n in sections) == total
                      and len(keys) == len(set(keys)) == total, (sections, total, len(keys)))
                await pg.screenshot(path=str(out / "23e-collection-groups.png"))
                # under the collection in the ribbon, each group with its count; one opens its section
                children = await pg.eval_on_selector_all("#collection-panel-list .panel-child", "els => els.map(e => e.innerText.replace(/\\s+/g, ' ').trim())")
                await pg.click('#collection-panel-list .panel-child:has-text("Every one")')
                await pg.wait_for_selector('#lib-games .lib-group[data-group="1"]')
                check("the ribbon lists the current collection's groups under it, with their counts",
                      children == [f"Best Of {len(names)}", f"Every one {total - len(names)}"] and await pg.text_content("#collection-group-count") == "2 groups", children)
                # Export offers where the groups go (ES-DE: collections, subfolders or systems of their own)
                await pg.click("#collection-export")
                await pg.wait_for_selector('#export-layout option[value="esde"]', state="attached")
                await pg.select_option("#export-layout", "esde")
                await pg.wait_for_selector("#export-groups")
                placings = await pg.eval_on_selector_all("#export-groups [data-placing]", "els => els.map(e => e.dataset.placing)")
                chosen_placing = await pg.eval_on_selector("#export-groups input:checked", "e => e.value")
                # D81: Knulli has collections and folders for groups; ROCKNIX only folders
                shown = "els => els.map(e => e.textContent)"
                await pg.select_option("#export-layout", "rocknix")
                await pg.wait_for_function("() => document.querySelector('#export-groups [data-placing] b')?.textContent === 'Together'")
                rocknix = await pg.eval_on_selector_all("#export-groups [data-placing] b", shown)
                await pg.select_option("#export-layout", "knulli")
                await pg.wait_for_function("() => document.querySelector('#export-groups [data-placing] b')?.textContent === 'Collections only'")
                knulli = await pg.eval_on_selector_all("#export-groups [data-placing] b", shown)
                await pg.click("#export-dialog-cancel")
                check("Export asks where a collection's groups go when the layout can place them, collections only by default",
                      placings == ["collections", "subfolders", "systems"] and chosen_placing == "collections", (placings, chosen_placing))
                check("Knulli offers its groups as collections or folders, ROCKNIX in folders or together (D81)",
                      knulli == ["Collections only", "Subfolders"] and rocknix == ["Together", "Subfolders"], (knulli, rocknix))
                # renamed, then removed: the collection as it was
                await pg.click('#collection-view [data-view="rules"]')
                await pg.click('#collection-group-1 [data-action="group-rename"]')
                await pg.fill("#collection-name", "All the rest")
                await pg.click("#collection-name-dialog button[type=submit]")
                await pg.wait_for_function("() => document.querySelector('#collection-group-1 h2')?.textContent === 'All the rest'")
                renamed = [g["name"] for g in json.loads(coll_file.read_text()).get("groups", [])]
                await pg.click('#collection-group-1 [data-action="group-remove"]')
                await pg.click("#confirm-dialog button[type=submit]")
                await pg.wait_for_function("(n) => +document.querySelector('#collection-games').textContent === n && !document.querySelector('#collection-group-1')", arg=len(names))
                gone = json.loads(coll_file.read_text())
                check("a group can be renamed and removed, and the collection is as it was without it",
                      renamed == ["All the rest"] and "groups" not in gone and gone["name"] == "Best Of" and await pg.query_selector("#collection-groups-none") is not None, (renamed, gone.get("groups")))

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
                # review H16: after a search, Ctrl+A and Export these… take only the games it found
                second = found[1][0]
                await pg.fill("#collection-search", second)
                await pg.wait_for_function("(k) => [...document.querySelectorAll('#lib-games [data-key]')].map(e => e.dataset.key).join() === k", arg=f"gb/{second}")
                await pg.click(f'#lib-games [data-key="gb/{second}"]')
                await pg.keyboard.press("Control+a")
                await pg.wait_for_selector("#bar-count")
                await pg.click("#bar-export-these")
                await pg.wait_for_selector("#export-part")
                await pg.select_option("#export-layout", "folder")
                part2 = home / "Part2"
                await pg.fill("#export-place-out", str(part2))
                await pg.click("#export-dialog button[type=submit]")
                await pg.wait_for_selector("#organise-totals")
                totals2 = await pg.inner_text("#organise-totals")
                await pg.click("#export-dialog button[type=submit]")
                await toast_text(pg, "Exported Handheld picks")
                got2 = sorted(x.name for x in part2.rglob("*") if x.is_file()) if part2.is_dir() else []
                check("Export these… after a search and Ctrl+A exports only the games the search found",
                      len(got2) == 1 and got2[0].startswith(second) and "1 game" in totals2, (got2, totals2))
                await pg.keyboard.press("Escape")
                await pg.fill("#collection-search", "")
                # an empty collection (D141): a name and no games, its page says how to add them,
                # and Add games… fills it; deleted again after
                await pg.click("#collection-panel-new")
                await pg.check('#collection-way input[value="empty"]')
                await pg.fill("#collection-name", "To fill")
                await pg.click("#collection-new-dialog button[type=submit]")
                await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'To fill'")
                await pg.wait_for_selector('#collection-none[data-empty="hand"] #collection-to-library')
                empty_rules = next(x for x in (json.loads(f.read_text()) for f in (library / "_library/collections").glob("*.json")) if x["name"] == "To fill")["rules"]
                await pg.screenshot(path=str(out / "23c2-empty-collection.png"))
                await pg.click("#collection-add")
                await pg.wait_for_selector("#add-games-list li input")
                await pg.check("#add-games-list li input >> nth=0")
                await pg.click("#add-games-dialog button[type=submit]")
                await pg.wait_for_function("() => +document.querySelector('#collection-games')?.textContent === 1")
                check("New collection › Empty makes one with no games (only those added by hand), its page says how to add them, and Add games… adds one",
                      empty_rules.get("include") == "none", empty_rules)
                await pg.click("#collection-menu")
                await pg.click('#context-menu [data-action="collection-delete"]')
                await pg.click("#confirm-dialog button[type=submit]")
                await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent !== 'To fill'")
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
                # with a file no DAT knows beside it, taken as its folder's system; copied in full
                (wanted / "homebrew.gb").write_bytes(b"a homebrew game nobody listed")
                await pg.click('.sidebar a[href="#/import"]')
                await pg.click("#import-games")
                pick(wanted)
                await pg.click("#import-add-folder")
                await pg.wait_for_selector("#import-folders li")
                systems_offered = await pg.eval_on_selector_all("#import-system option", "els => els.map(e => e.textContent)")
                await pg.check('#import-mode input[value="always"]')
                # into a new collection too (D140), named here (the folder's name is taken)
                await pg.select_option("#import-into-pick", "new")
                into_default = await pg.get_attribute("#import-into-name", "placeholder")
                await pg.fill("#import-into-name", "Imported picks")
                await pg.click("#import-games-dialog button[type=submit]")
                await pg.wait_for_selector("#import-games-review", timeout=30000)
                into_line = await pg.inner_text("#import-into-line")
                offered = await pg.eval_on_selector_all("#import-games-systems li", "els => els.map(e => [e.dataset.name, e.dataset.status])")
                unknown_as = await pg.input_value('#import-games-unknown li[data-folder="Wanted"] select')
                await pg.wait_for_selector('#import-places li[data-platform="gb"][data-dir]:not([data-dir=""])')
                place = await pg.get_attribute('#import-places li[data-platform="gb"]', "data-dir")
                games_first = await pg.evaluate("() => !!(document.querySelector('#import-games-systems').compareDocumentPosition(document.querySelector('#import-places')) & Node.DOCUMENT_POSITION_FOLLOWING)")
                await pg.screenshot(path=str(out / "23d-import-review.png"))
                # the window stays when the page behind it is clicked; closed, it comes back where it was
                await pg.mouse.click(5, 5)
                stayed = await pg.query_selector("#import-games-review") is not None
                await pg.click('#import-games-review .dialog-head button[aria-label="Close"]')
                await pg.wait_for_selector("#import-games-review", state="detached")
                await pg.click("#import-games")
                await pg.wait_for_selector("#import-games-review")
                kept = await pg.input_value('#import-games-unknown li[data-folder="Wanted"] select')
                await pg.click("#import-games-review button[type=submit]")
                await pg.wait_for_selector("#bring-dialog #organise-totals")
                await pg.screenshot(path=str(out / "23d-bring-in.png"))
                bring_head = await pg.evaluate("() => [document.querySelector('#bring-dialog h2').textContent, document.querySelector('#organise-totals p').textContent, document.querySelector('#bring-dialog button[type=submit]').textContent]")
                await pg.click("#bring-dialog button[type=submit]")
                said = await toast_text(pg, "Imported 2 games into the library")
                brought = [x for x in (fx / "Roms").rglob("Alpha (Europe).gb") if ".gamerepo-set-aside" not in x.parts]
                picks = next((x for x in (json.loads(f.read_text()) for f in (library / "_library/collections").glob("*.json")) if x["name"] == "Imported picks"), {})
                check("Import can put the games in a new collection too: the review says so, the collection is made from them, and the file no DAT knows waits on it (D140)",
                      into_default == "Wanted" and "Imported picks" in into_line and "1 game" in into_line and "1 file" in into_line and "Imported picks is a collection of them" in said
                      and picks.get("made_from", {}).get("kind") == "import" and [w["name"] for w in picks.get("waiting", [])] == ["homebrew.gb"] and picks.get("waiting", [{}])[0].get("platform") == "gb",
                      (into_default, into_line, said, picks))
                await pg.click('.sidebar a[href="#/collections"]')
                await pg.click('#collection-panel-list .panel-link:has-text("Imported picks")')
                await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Imported picks' && document.querySelector('#collection-games')?.textContent === '1'", timeout=30000)
                await pg.wait_for_selector("#collection-waiting")
                waits = await pg.eval_on_selector_all("#collection-waiting li", "els => els.map(e => e.dataset.name)")
                await pg.click("#collection-waiting summary")
                await pg.screenshot(path=str(out / "23f-collection-waiting.png"))
                await pg.click('#collection-waiting li[data-name="homebrew.gb"] [data-action="waiting-leave"]')
                await pg.wait_for_selector("#collection-waiting", state="detached")
                left = json.loads(next(f for f in (library / "_library/collections").glob("*.json") if json.loads(f.read_text())["name"] == "Imported picks").read_text())
                check("the collection's page lists the file waiting on Audit › Unknown, and Leave it stops waiting for it",
                      waits == ["homebrew.gb"] and not left.get("waiting"), (waits, left.get("waiting")))
                await pg.click('.sidebar a[href="#/import"]')
                await pg.wait_for_selector('#import-recent-list li[data-kind="bring"]')
                await pg.wait_for_selector('#import-recent-list li[data-kind="bring"]')
                recent = await pg.eval_on_selector_all("#import-recent-list li", "els => els.map(e => e.dataset.kind)")
                await pg.locator("#import-recent").scroll_into_view_if_needed()
                await pg.screenshot(path=str(out / "23e-recent-imports.png"))
                check("the import review lists the games before where each system's go (review M42); its preview says what it will do with one word for it (M40); Recent imports lists it (D78)",
                      games_first and bring_head == ["Import 2 games into the library", "2 games to import.", "Import"] and recent and recent[0] == "bring" and all(k in ("bring", "add", "import", "docs", "magazines", "videos", "bios") for k in recent),
                      (games_first, bring_head, recent))
                check("Import offers every system or one, and copy, always copy or move; its window stays put and comes back where it was; each system's games go to the folder its games are in, and a file no DAT knows to its folder's system",
                      systems_offered[0].startswith("Every system") and "Game Boy" in systems_offered and unknown_as == "gb" and stayed and kept == "gb" and len(brought) == 1
                      and brought[0].parent == Path(place) and (Path(place) / "homebrew.gb").is_file() and os.stat(brought[0]).st_nlink == 1,
                      (systems_offered[:3], unknown_as, stayed, kept, place, brought))
                await pg.click('.sidebar a[href="#/collections"]')
                # (by its id: "Imported picks" is imported from Wanted)
                wanted_id = next(x["id"] for x in (json.loads(f.read_text()) for f in (library / "_library/collections").glob("*.json")) if x["name"] == "Wanted")
                await pg.click(f'#collection-panel-list .panel-link[data-id="{wanted_id}"]')
                await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Wanted' && document.querySelector('#collection-have')?.textContent === '1'", timeout=30000)
                check("a collection from folders points at Import for the games the library lacks; Import copies one into its folder with games, named as the DAT says, and scans it",
                      lacking == [["Alpha (Europe)", "missing"]] and "Import" in pointed and offered == [["Alpha (Europe)", "missing"]] and len(brought) == 1 and (wanted / "europe one.gb").is_file(), (lacking, pointed, offered, said, brought))
                await pg.click('#collection-panel-list .panel-link:has-text("Best Of")')
                await pg.wait_for_function("() => document.querySelector('#collection-title')?.textContent === 'Best Of'")

            async with Section(pg, B, "Phase 4: PC games"):
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
                      await pg.eval_on_selector_all("#sources > li", "els => els.map(e => e.dataset.source)") == ["screenscraper", "steam", "gog", "igdb", "thegamesdb", "launchbox"] and str(library / "PC") in await pg.inner_text("#pc-folder-path"), await pg.inner_text("#settings-sources"))
                await pg.click("#nav-pc")
                await pg.click("#pc-first")
                await pg.wait_for_selector("#pc-items li", timeout=30000)
                pc_at = await pg.evaluate("() => [location.hash, !!document.querySelector('#import-pc.import-here')]")
                # review M35: with what it read shown, a stray click or Esc leaves the window up
                await pg.mouse.click(5, 5)
                await pg.keyboard.press("Escape")
                pc_stayed = await pg.locator("#pc-items li").count()
                check("PC games' Import… opens the Import page at PC games (D78), and its list stays through a stray click and Esc (M35)",
                      pc_at == ["#/import/import-pc", True] and pc_stayed == 2, (pc_at, pc_stayed))
                items = await pg.eval_on_selector_all("#pc-items li", "els => els.map(e => [e.querySelector('.pc-item-title').value, e.dataset.status, e.querySelector('.pc-item-meta').textContent])")
                check("Import… reads the inbox: one game found by its goggame file, the other by a search of the sources",
                      sorted(i[0] for i in items) == ["Alpha Strike", "Zeta Quest"] and all(i[1] == "ready" for i in items)
                      and any("goggame" in i[2] and "GOG game 1207658930" in i[2] for i in items) and any("Steam app 4000" in i[2] and "search" in i[2] for i in items), items)
                await pg.screenshot(path=str(out / "24-pc-import.png"))
                pc_how = await pg.eval_on_selector_all("#pc-import-mode input", "els => els.map(e => e.value)")
                await pg.click("#pc-import-dialog button[type=submit]")
                await pg.wait_for_selector("#organise-totals")
                pc_title = await pg.inner_text("#pc-import-dialog h2")
                await pg.click("#pc-import-dialog button[type=submit]")
                said = await toast_text(pg, "Imported 2 PC games", timeout=30000)
                check("the PC import has the Import page's three ways in (D79) and a present-tense preview (M40)", pc_how == ["copy", "always", "move"] and pc_title == "Import 2 PC games", (pc_how, pc_title))
                await pg.click("#nav-pc")
                await pg.wait_for_function("() => document.querySelectorAll('#pcgames-items [data-id]').length === 2")
                made = sorted(x.name for x in (library / "PC").iterdir()) if (library / "PC").is_dir() else []
                alpha_dir = library / "PC/Alpha Strike (2001)"
                alpha = json.loads((alpha_dir / "game.json").read_text()) if (alpha_dir / "game.json").exists() else {}
                art = list((library / "_library/games/pc").glob("*/media/cover.*"))
                check("each PC game gets its own folder with a game.json, and the artwork the sources offered",
                      made == ["Alpha Strike (2001)", "Zeta Quest (1999)"] and alpha.get("ids") == {"steam": "4000"} and (alpha_dir / "install/Alpha Strike.zip").is_file()
                      and (library / "PC/Zeta Quest (1999)/extracted/bin/zeta.exe").is_file() and len(art) == 1 and (inbox / "Alpha Strike.zip").is_file(), (said, made, alpha, art))
                await pg.click('#pcgames-items [data-id]:has-text("Alpha Strike")')
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
                # laid out as the Library is (D169): filters by year, store and the rest, Grid, List and Table, groups
                years = await pg.eval_on_selector_all("#filter-pcgames-year [data-value]", "els => els.map(e => e.dataset.value)")
                stores = await pg.eval_on_selector_all("#filter-pcgames-store [data-value]", "els => els.map(e => e.dataset.value)")
                await pg.click('#filter-pcgames-year [data-value="1999"] input')
                await pg.wait_for_function("() => document.querySelectorAll('#pcgames-items [data-id]').length === 1")
                only_1999 = await pg.inner_text("#pcgames-items [data-id]")
                pc_chips = await pg.eval_on_selector_all("#pcgames-chips [data-chip]", "els => els.map(e => e.dataset.chip)")
                await pg.click("#pcgames-clear")
                await pg.wait_for_function("() => document.querySelectorAll('#pcgames-items [data-id]').length === 2")
                await pg.click('#pcgames-view [data-view="table"]')
                await pg.wait_for_selector('#pcgames-items.shelf-tables [role="row"][data-id]')
                pc_cols = await pg.eval_on_selector_all('.shelf-thead [role="columnheader"]', "els => els.map(e => e.textContent.replace(/[▴▾]/g, '').trim())")
                await pg.click("#pcgames-group")
                await pg.click('#context-menu [data-action="pcgames-group-store"]')
                await pg.wait_for_selector('#pcgames-items .shelf-group-head[data-group="gog"]')
                pc_groups = await pg.eval_on_selector_all("#pcgames-items .shelf-group-head h2", "els => els.map(e => e.textContent)")
                await pg.click('#pcgames-view [data-view="list"]')
                await pg.wait_for_selector("#pcgames-items.shelf-lists .lib-line")
                await pg.screenshot(path=str(out / "25b-pc-shelf.png"))
                await pg.click("#pcgames-group")
                await pg.click('#context-menu [data-action="pcgames-group-none"]')
                await pg.click('#pcgames-view [data-view="grid"]')
                await pg.wait_for_selector("#pcgames-items.shelf-grids .lib-tile")
                check("PC games are laid out as the Library (D169): filters by year and store, Grid, List and Table with their details, and groups",
                      years == ["2001", "1999"] and "gog" in stores and "Zeta Quest" in only_1999 and pc_chips == ["year-1999"]
                      and {"Title", "Year", "Developer", "Publisher", "Genres", "Store"} <= set(pc_cols) and "GOG" in pc_groups, (years, stores, only_1999, pc_chips, pc_cols, pc_groups))
                await pg.click('#pcgames-items [data-id]:has-text("Zeta Quest")')
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
                # one source at a time (D138): the choice beside the year searches just it
                await pg.click("#pc-identify")
                await pg.wait_for_selector('#identify-results li[data-source="gog"]')
                await pg.select_option("#identify-source", "gog")
                await pg.wait_for_function("() => [...document.querySelectorAll('#identify-results li')].every(e => e.dataset.source === 'gog')")
                only = await pg.eval_on_selector_all("#identify-results li", "els => els.map(e => e.dataset.source)")
                await pg.click("#identify-dialog-cancel")
                await pg.wait_for_selector("#identify-dialog", state="detached")
                check("Identify can search one source, chosen beside the year", only == ["gog"], only)
                # (a game picked, the bar is the selection's, as the Library's is)
                await pg.click("#bar-clear")
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
                await pg.click('#pcgames-items [data-id]:has-text("Zeta Quest")')
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

                # Jerred's tryout (D128–D130): a newer zip of Alpha Strike goes into it and replaces
                # its installer; a game no store has, which a search took for Zeta Quest, is kept as
                # a game of its own with No match; then Remove from the library…, with Undo
                with zipfile.ZipFile(inbox / "Alpha Strike (2001).zip", "w") as z:
                    z.writestr("Alpha Strike/alpha.exe", b"MZ alpha, a newer one")
                with zipfile.ZipFile(inbox / "Zeta Quest (1999).zip", "w") as z:
                    z.writestr("zq.exe", b"MZ a game no store has")
                await pg.click("#nav-pc")
                await pg.click("#pc-import")
                await pg.wait_for_selector('#pc-items li[data-status="update"]', timeout=30000)
                rows_js = """els => els.map(e => ({ key: e.dataset.key, name: e.querySelector('.pc-item-name').textContent, status: e.dataset.status, title: e.querySelector('.pc-item-title').value,
                    year: e.querySelector('.pc-item-year').value, edit: !e.querySelector('.pc-item-title').disabled, meta: e.querySelector('.pc-item-meta').textContent,
                    replaces: e.querySelector('.pc-item-replaces')?.textContent || '', unmatch: !!e.querySelector('[data-action=no-match]') }))"""
                rows = {r["name"]: r for r in await pg.eval_on_selector_all("#pc-items li", rows_js)}
                newer, lost = rows.get("Alpha Strike (2001).zip", {}), rows.get("Zeta Quest (1999).zip", {})
                check("the review list says an installer that goes into a game you have replaces its own (D130), and offers No match on what a search matched",
                      newer.get("status") == "update" and newer.get("replaces") == "Replaces its installer" and newer.get("unmatch")
                      and lost.get("status") == "update" and "Zeta Quest" in lost.get("meta", "") and not lost.get("replaces") and lost.get("unmatch")
                      and all(not r["unmatch"] for r in rows.values() if r["status"] == "have"), rows)
                await pg.click(f'#pc-items li[data-key="{lost.get("key")}"] [data-action="no-match"]')
                await pg.wait_for_selector(f'#pc-items li[data-key="{lost.get("key")}"][data-status="own"]')
                own = {r["name"]: r for r in await pg.eval_on_selector_all("#pc-items li", rows_js)}.get("Zeta Quest (1999).zip", {})
                badge = await pg.inner_text(f'#pc-items li[data-key="{lost.get("key")}"] .pc-badge')
                check("No match keeps it as a game of its own (D128): the search's id and match go, its title and year are what its name said, and can be changed",
                      badge == "Its own game" and own.get("title") == "Zeta Quest" and own.get("year") == "1999" and own.get("edit") and "Steam app" not in own.get("meta", "")
                      and "search" not in own.get("meta", "") and not own.get("unmatch"), (badge, own))
                await pg.fill(f'#pc-items li[data-key="{lost.get("key")}"] .pc-item-title', "Zeta Quest Lost")
                await pg.screenshot(path=str(out / "26e-pc-no-match.png"))
                await pg.click("#pc-import-dialog button[type=submit]")
                await pg.wait_for_selector("#organise-list li")
                aside_rows = await pg.eval_on_selector_all('#organise-list li[data-op="aside"]', "els => els.map(e => e.textContent)")
                await pg.click("#pc-import-dialog button[type=submit]")
                said = await toast_text(pg, "Imported 2 PC games", timeout=30000)
                alpha = json.loads((alpha_dir / "game.json").read_text())
                held = list((library / "PC/.gamerepo-set-aside").glob("*/Alpha Strike (2001)/install/Alpha Strike.zip"))
                lost_dir = library / "PC/Zeta Quest Lost (1999)"
                lost_json = json.loads((lost_dir / "game.json").read_text()) if (lost_dir / "game.json").exists() else {}
                zeta_now = json.loads((library / "PC/Zeta Quest (1999)/game.json").read_text())
                check("the import sets the installer it replaces aside, shown in its preview, and the unmatched game comes in under its own title with no ids",
                      len(aside_rows) == 1 and "Alpha Strike (2001)/install/Alpha Strike.zip" in aside_rows[0] and "its installer, replaced by Alpha Strike (2001).zip" in aside_rows[0]
                      and [f["path"] for f in alpha["files"] if f["kind"] == "install"] == ["install/Alpha Strike (2001).zip"] and len(held) == 1 and not (alpha_dir / "install/Alpha Strike.zip").exists()
                      and lost_json.get("title") == "Zeta Quest Lost" and not lost_json.get("ids") and (lost_dir / "install/Zeta Quest (1999).zip").is_file()
                      and not any(f["kind"] == "install" for f in zeta_now["files"]), (said, aside_rows, alpha["files"], held, lost_json, zeta_now["files"]))

                # the installer it replaced, deleted for good from History's Set aside tab (D168)
                await pg.click('.sidebar a[href="#/history"]')
                await pg.click('#history-tabs [data-tab="aside"]')
                await pg.wait_for_selector('#aside-changes .aside-file[data-held$="Alpha Strike.zip"]')
                await pg.check('#aside-changes .aside-file[data-held$="Alpha Strike.zip"] input')
                await pg.click('#aside-changes .aside-change:has(.aside-file[data-held$="Alpha Strike.zip"]) [data-action="aside-delete"]')
                await pg.click("#confirm-dialog button[type=submit]")
                said = await toast_text(pg, "Deleted 1 file")
                await pg.wait_for_function("() => !document.querySelector('#aside-changes .aside-file[data-held$=\"Alpha Strike.zip\"]')")
                check("a file deleted from Set aside is gone for good, and no longer listed (D168)",
                      held and not held[0].exists() and (alpha_dir / "install/Alpha Strike (2001).zip").is_file(), (said, held))
                await pg.click('#history-tabs [data-tab="changes"]')

                # Remove from the library… on Alpha Strike's panel: the preview, gone, then Undo
                alpha_meta = library / f"_library/games/pc/{alpha['id']}"
                tree = lambda d: {str(p.relative_to(d)): p.read_bytes() for p in d.rglob("*") if p.is_file() and not any(x.startswith(".gamerepo") for x in p.relative_to(d).parts)} if d.is_dir() else {}
                alpha_before, alpha_meta_before = tree(alpha_dir), tree(alpha_meta)
                pc_keys = """async () => { const inv = (cmd, args = {}) => window.__TAURI__.core.invoke('api', { cmd, args }); await inv('browse');
                    return (await inv('browse_query', { query: { platform: ['pc'], limit: 50 } })).items.map((i) => i.key); }"""
                await pg.click("#nav-pc")
                await pg.click('#pcgames-items [data-id]:has-text("Alpha Strike")')
                await pg.wait_for_selector("#pc-remove")
                await pg.click("#pc-remove")
                await pg.wait_for_selector("#organise-dialog #organise-list li")
                removing = await pg.evaluate("""() => [document.querySelector('#organise-dialog h2')?.textContent || '', document.querySelector('#organise-list')?.dataset.total,
                    [...document.querySelectorAll('#organise-list li')].map((e) => e.dataset.op), [...document.querySelectorAll('#organise-list li .op-to')].pop()?.textContent || '',
                    document.querySelector('#pc-remove-steam')?.textContent || '', document.querySelector('#organise-dialog button[type=submit]')?.textContent || '']""")
                await pg.screenshot(path=str(out / "26f-pc-remove.png"))
                await pg.click("#organise-dialog button[type=submit]")
                said = await toast_text(pg, "Removed Alpha Strike from the library", timeout=30000)
                await pg.wait_for_function("() => document.querySelector('#pcgames-items') && ![...document.querySelectorAll('#pcgames-items [data-id]')].some((e) => e.textContent.includes('Alpha Strike'))")
                keys = await pg.evaluate(pc_keys)
                check("Remove from the library… (D129) previews every file of its folder and its details set aside, game.json last, and says its Steam shortcut stays",
                      removing[0] == "Remove Alpha Strike from the library" and int(removing[1] or 0) == len(alpha_before) + len(alpha_meta_before) and set(removing[2]) == {"aside"}
                      and removing[3] == "Alpha Strike (2001)/game.json" and "Add to Steam" in removing[4] and removing[5] == "Remove it", (removing, len(alpha_before), len(alpha_meta_before)))
                check("once it's run the game is gone from PC games, the Library and its folders, set aside",
                      not alpha_dir.exists() and not alpha_meta.exists() and f"pc/{alpha['id']}" not in keys and f"pc/{zeta_json['id']}" in keys
                      and list((library / "PC/.gamerepo-set-aside").glob("*/Alpha Strike (2001)/game.json")), (said, keys))
                await pg.click("#toast .toast-action")
                await toast_text(pg, "Undone", timeout=30000)
                await pg.wait_for_selector('#pcgames-items [data-id]:has-text("Alpha Strike")')
                keys = await pg.evaluate(pc_keys)
                check("Undo brings the removed game back as it was, in PC games and the Library",
                      tree(alpha_dir) == alpha_before and tree(alpha_meta) == alpha_meta_before and f"pc/{alpha['id']}" in keys, (sorted(tree(alpha_dir)), keys))
                # Identify…'s None of these on a game you have (D128): its ids go, its title and details stay
                await pg.click('#pcgames-items [data-id]:has-text("Alpha Strike")')
                await pg.click("#pc-identify")
                await pg.wait_for_selector("#identify-results li")
                await pg.click("#identify-none-of-these")
                said = await toast_text(pg, "is matched to no source now")
                await pg.wait_for_selector("#identify-dialog", state="detached")
                alpha = json.loads((alpha_dir / "game.json").read_text())
                answers = {p.stem: json.loads(p.read_text()) for p in (alpha_meta / "sources").glob("*.json")}
                shared = json.loads((alpha_meta / "game.json").read_text())
                check("Identify…'s None of these on a game you have takes away its ids at the sources, and keeps its title and what they said",
                      alpha.get("ids") == {} and alpha.get("title") == "Alpha Strike" and answers and all("id" not in a for a in answers.values())
                      and "identified_by" not in shared and "Delete metadata" in said and (alpha_meta / "media").is_dir(), (said, alpha.get("ids"), answers, shared.get("identified_by")))
                # Find details… (D138) asks about the games matched to no source, and leaves alone
                # one said to be none of theirs
                await pg.click("#bar-clear")
                await pg.click("#pcgames-menu")
                await pg.click('#context-menu [data-action="pc-find-details"]')
                await pg.wait_for_selector("#pc-found-dialog")
                listed = await pg.eval_on_selector_all("#pc-found-dialog li b", "els => els.map(e => e.textContent)")
                found_text = await pg.inner_text("#pc-found-dialog")
                await pg.screenshot(path=str(out / "26c-find-details.png"))
                await pg.click("#pc-found-dialog button[type=submit]")
                await pg.wait_for_selector("#pc-found-dialog", state="detached")
                check("Find details… leaves a game said to be no source's alone",
                      "Alpha Strike" not in listed and json.loads((alpha_dir / "game.json").read_text()).get("ids") == {}, (listed, found_text))
                # and on a game's page, in its ⋯ menu; once it's run, back to PC games
                await pg.click('#pcgames-items [data-id]:has-text("Zeta Quest Lost")')
                await pg.click("#pc-open")
                await pg.wait_for_selector(f'#game-page[data-key="pc/{lost_json.get("id")}"]')
                await pg.click("#game-menu")
                await pg.click('#context-menu [data-action="game-pc-remove"]')
                await pg.wait_for_selector("#organise-dialog #organise-list li")
                from_page = await pg.evaluate("() => [document.querySelector('#organise-dialog h2')?.textContent || '', !!document.querySelector('#pc-remove-steam')]")
                await pg.click("#organise-dialog button[type=submit]")
                said = await toast_text(pg, "Removed Zeta Quest Lost from the library", timeout=30000)
                await pg.wait_for_function("() => location.hash.startsWith('#/pc')")
                check("a PC game's page has Remove from the library… in its menu, and goes to PC games once it's removed",
                      from_page == ["Remove Zeta Quest Lost from the library", False] and not lost_dir.exists(), (from_page, said))

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
                check("Add a game… copies a ROM into the folder its system's games are in, scans it and says which DAT game it is",
                      "Alpha (Europe)" in row and (fx / "Roms/Game Boy/alpha-eu.gb").is_file() and (downloads / "alpha-eu.gb").is_file(), row)
                await pg.screenshot(path=str(out / "27-add-game.png"))
                await pg.click("#add-results-dialog button[type=submit]")

            async with Section(pg, B, "Phase 5: sources, scraping, documents and exports that carry them"):
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
                # ScreenScraper's threads, before its account is read (D142)
                threads_shown = await pg.get_attribute('#sources li[data-source="screenscraper"] input[name="threads"]', "placeholder")
                cfg = json.loads((home / "config/config.json").read_text())
                in_library = "test-key-123" in (library / "_library/library.json").read_text()
                check("Settings › Sources keeps a source's key on this computer (never shown again, never in the library) and Check asks the source",
                      cfg.get("source_keys", {}).get("thegamesdb", {}).get("apikey") == "test-key-123" and "answered" in said and "test-key" not in (shown or "") and not in_library
                      and threads_shown == "As many as your account allows",
                      (said, shown, threads_shown, cfg.get("source_keys")))
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
                # grouped by game (D155): the game is the group's heading
                zeta_doc = await pg.evaluate("() => { const r = [...document.querySelectorAll('#docs-list .pc-row')].find(e => e.textContent.includes('Zeta Map')); return r ? `${r.closest('section').querySelector('h2')?.textContent} | ${r.textContent}` : ''; }")
                await pg.fill("#docs-search", "zebrafish")
                await pg.wait_for_function("() => document.querySelectorAll('#docs-list .pc-row').length === 1 && document.querySelector('#docs-list mark')")
                hit = await pg.inner_text("#docs-list .pc-row")
                await pg.click("#docs-list .pc-row")
                await pg.wait_for_selector("#doc-view #doc-text")
                text = await pg.inner_text("#doc-view #doc-text")
                await pg.screenshot(path=str(out / "34-documents.png"))
                await pg.fill("#docs-search", "")
                await pg.click('#filter-docs-kind [data-value="guide"] input')
                await pg.wait_for_function("() => document.querySelectorAll('#docs-list .pc-row').length === 2")
                await pg.click('#docs-list .pc-row:has-text("Beta")')
                await pg.wait_for_selector("#doc-view #doc-text strong")
                await pg.click('#filter-docs-kind [data-value="guide"] input')
                # the grid (D170): a game's documents beside the next game's, not a row each
                await pg.wait_for_selector("#doc-view", state="detached")
                await pg.click('#docs-view [data-view="grid"]')
                await pg.wait_for_function("() => document.querySelectorAll('#docs-list.shelf-grids .doc-tile').length === 4")
                doc_rows = await pg.evaluate("() => [...document.querySelectorAll('#docs-list .shelf-section')].map((s) => Math.round(s.getBoundingClientRect().top))")
                await pg.screenshot(path=str(out / "34b-documents-grid.png"))
                await pg.click('#docs-view [data-view="list"]')
                await pg.wait_for_selector("#docs-list.shelf-lists .pc-row")
                check("Documents' grid puts each game's documents beside the next game's (D170), not one game to a row",
                      len(doc_rows) >= 3 and len(set(doc_rows)) < len(doc_rows), doc_rows)
                cli = cli_json("docs", "--home", str(home), "--search", "waterfall", "--json")
                check("Documents lists every document, searches their words with the words marked, and shows one (Markdown made into text); the command line searches them too",
                      listed == 4 and zeta_doc.startswith("Zeta Quest | ") and "PC" in zeta_doc and "Alpha Manual" in hit and "zebrafish" in hit.lower() and "zebrafish button" in text and cli.get("total") == 1 and cli["items"][0]["kind"] == "guide",
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

            async with Section(pg, B, "Phase 5b: BIOS files and export layouts"):
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
                # (the list is got from Import › BIOS files, D78: back to the BIOS page to see it)
                bios_list_at = await pg.evaluate("() => location.hash")
                await pg.click('.sidebar a[href="#/bios"]')
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
                      and await pg.query_selector("#bios-show") is None and bios_list_at == "#/import/import-bios", (said, shown, searched, totals, bios_list_at))
                await pg.screenshot(path=str(out / "40-bios.png"), full_page=True)
                await pg.click("#bios-gather")
                await pg.wait_for_selector("#bios-gather-dialog #organise-totals")
                bios_how = await pg.eval_on_selector_all("#bios-gather-mode input", "els => els.map(e => e.value)")
                bios_at = await pg.evaluate("() => location.hash")
                await pg.click("#bios-gather-dialog button[type=submit]")
                said = await toast_text(pg, "Imported the BIOS files")
                gathered = library / "BIOS/Nintendo - Gameboy/gb_bios.bin"
                check("Import copies the BIOS files the library has into its BIOS folder, a folder for each system, from the Import page with its three ways in (D78, D79)",
                      gathered.is_file() and gathered.read_bytes() == gb_bios and (fx / "Roms/gb_bios.bin").is_file() and bios_how == ["copy", "always", "move"] and bios_at == "#/import/import-bios", (said, bios_how, bios_at))

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
            async with Section(pg, B, "Phase 6: the Library and a game's page"):
                # Phase 6. The Library: every game across the systems, by its pictures, filtered with counts
                await pg.evaluate("() => window.__gamerepo.platform.store.prefs.set('gr-library-view', 'grid')")
                await pg.click('#nav-library')
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
                await pg.select_option("#filter-players-select", "2")
                await pg.wait_for_function("() => location.hash.includes('players=2') && !document.querySelector('.lib-tile[data-key=\"gb/Beta (Europe)\"]')")
                two = await pg.eval_on_selector_all("#lib-games .lib-tile", "els => els.map(e => e.dataset.key)")
                await pg.select_option("#filter-players-select", "0")
                await pg.wait_for_function("(n) => document.querySelectorAll('#lib-games .lib-tile').length === n", arg=len(tiles), timeout=10000)
                check("Players and Rating are drop-downs in the filters, not lists of buttons, and 2 or more shows only the games for two",
                      radios == 0 and "filter-players-select" in drops and "gb/Alpha (USA)" in two and "gb/Beta (Europe)" not in two, (radios, drops, two))
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
                # D151: Open in the panel keeps the list too; Previous and Next are labelled, with where it is in which list
                await pg.click("#lib-panel-open")
                await pg.wait_for_selector('#game-page[data-key="gb/Alpha (USA)"]')
                await pg.wait_for_selector("#game-next")
                steps = await pg.evaluate("() => [document.querySelector('#game-prev')?.textContent.trim(), document.querySelector('#game-next')?.textContent.trim(), document.querySelector('.game-pos')?.textContent]")
                await pg.click("#game-back")
                await pg.wait_for_selector('#lib-panel[data-key="gb/Alpha (USA)"]')
                check("Open in the Library's panel keeps the list: the game's page has ‹ Previous and Next › and says where it is (\"N of M in the Library\"), and Back picks it again",
                      steps[0] == "‹ Previous" and steps[1] == "Next ›" and " of " in (steps[2] or "") and (steps[2] or "").endswith("in the Library"), steps)
                # the filters saved as a collection, whose rules keep up with the details
                await pg.click('#filter-genre [data-value="Action"] input')
                await pg.wait_for_selector('#lib-chips [data-chip="genre-Action"]')
                # D150: the Library keeps its filters and view when another page is shown, and comes back to them
                await pg.click("#lib-view [data-view=list]")
                await pg.click('.sidebar a[href="#/collections"]')
                await pg.wait_for_selector("#collections-head")
                await pg.click("#nav-library")
                await pg.wait_for_selector('#lib-chips [data-chip="genre-Action"]')
                kept_place = await pg.evaluate("() => [location.hash, document.querySelector('#lib-page')?.dataset.view]")
                await pg.click("#lib-view [data-view=grid]")
                check("the Library keeps its filters and view while another page is shown: the menu's Library goes back to them (D150)",
                      kept_place[0].startswith("#/library?") and "genre=Action" in kept_place[0] and kept_place[1] == "list", kept_place)
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
                # a collection of two systems (D171): each under its name in the ribbon, a click to just its games
                two = await pg.evaluate("""async () => { const inv = (cmd, args = {}) => window.__TAURI__.core.invoke('api', { cmd, args });
                    const r = await inv('collection_save', { collection: { id: '', name: 'Two systems', rules: { platforms: ['gb', 'nes'], include: 'have' } } }); return r.collection.id; }""")
                await pg.evaluate("(id) => { location.hash = '#/collections/' + id; }", two)
                await pg.wait_for_selector('#collection-panel-list .panel-child[data-id="system-nes"]')
                sys_rows = await pg.eval_on_selector_all("#collection-panel-list .panel-child", "els => els.map(e => [e.dataset.id, e.innerText.replace(/\\s+/g, ' ').trim()])")
                await pg.click('#collection-panel-list .panel-child[data-id="system-nes"]')
                await pg.wait_for_function("() => document.querySelector('#collection-systems')?.value === 'nes' && document.querySelectorAll('#lib-games [data-key]').length === 1")
                nes_only = await pg.eval_on_selector_all("#lib-games [data-key]", "els => els.map(e => e.dataset.key)")
                active = await pg.get_attribute('#collection-panel-list .panel-child.active', "data-id")
                await pg.screenshot(path=str(out / "52-collection-systems.png"))
                await pg.click('#collection-panel-list .panel-link[aria-current="page"]')
                await pg.wait_for_function("() => document.querySelector('#collection-systems')?.value === '' && document.querySelectorAll('#lib-games [data-key]').length > 1")
                check("a collection's systems are listed under it in the ribbon, with their counts; one shows just its games, and the collection all of them again (D171)",
                      [r[0] for r in sys_rows] == ["system-gb", "system-nes"] and sys_rows[1][1].endswith("1") and nes_only == ["nes/Echo (USA)"] and active == "system-nes", (sys_rows, nes_only, active))
                await pg.evaluate("(id) => window.__TAURI__.core.invoke('api', { cmd: 'collection_delete', args: { id } })", two)

            async with Section(pg, B, "The backlog (D157-D159)"):
                # D157-D159: the backlog, under Collections: games added from the Library (a PC game
                # too), ranked by dragging, In progress highlighted, Completed beside, each with Undo
                saved_order = lambda: [g["key"] for g in json.loads((library / "_library/backlog.json").read_text())["games"]]
                order = lambda: pg.eval_on_selector_all("#backlog-todo-list .bl-row", "els => els.map(e => e.dataset.key)")

                async def drag(src, x, y):
                    b = await pg.locator(src).bounding_box()
                    await pg.mouse.move(b["x"] + 30, b["y"] + b["height"] / 2)
                    await pg.mouse.down()
                    await pg.mouse.move(b["x"] + 40, b["y"] + b["height"] / 2 + 10, steps=3)
                    await pg.mouse.move(x, y, steps=10)
                    await pg.mouse.up()

                await pg.wait_for_selector('#lib-games .lib-tile[data-key="nes/Echo (USA)"]')
                pc_key = await pg.get_attribute('#lib-games .lib-tile[data-key^="pc/"]', "data-key")
                for i, k in enumerate(["gb/Alpha (USA)", pc_key, "nes/Echo (USA)"]):
                    await pg.click(f'.lib-tile[data-key="{k}"]', modifiers=["Control"] if i else [])
                await pg.wait_for_selector('#lib-panel[data-count="3"]')
                await pg.click('.lib-tile[data-key="nes/Echo (USA)"]', button="right")
                await pg.click('#context-menu [data-action="backlog"]')
                said = await toast_text(pg, "Added 3 games to the backlog")
                await pg.keyboard.press("Escape")
                await pg.click("#nav-backlog")
                await pg.wait_for_function("() => document.querySelectorAll('#backlog-todo-list .bl-row').length === 3")
                first = await order()
                ribbon = await pg.evaluate("() => [document.querySelector('#panel-backlog')?.getAttribute('aria-current'), document.querySelectorAll('#collection-panel-list .panel-link').length, document.querySelector('#nav-backlog')?.closest('li')?.classList.contains('nav-child')]")
                check("Add to the backlog in the Library puts the games picked (a PC game too) in the backlog, which is under Collections in the menu and at the top of their ribbon (D157)",
                      first == ["gb/Alpha (USA)", pc_key, "nes/Echo (USA)"] and saved_order() == first and ribbon == ["page", ribbon[1], True] and ribbon[1] > 0, (said, first, ribbon))
                # dragged to the top
                top = await pg.locator("#backlog-todo-list .bl-row >> nth=0").bounding_box()
                await drag('#backlog-todo-list .bl-row[data-key="nes/Echo (USA)"]', top["x"] + 120, top["y"] + 6)
                await pg.wait_for_function("() => document.querySelector('#backlog-todo-list .bl-row')?.dataset.key === 'nes/Echo (USA)'")
                dragged = await order()
                check("a game dragged to the top of To play takes the first place, and the others move down (D158)",
                      dragged == ["nes/Echo (USA)", "gb/Alpha (USA)", pc_key] and saved_order() == dragged, dragged)
                # In progress: highlighted, with its label, and counted
                await pg.click('#backlog-todo-list .bl-row[data-key="gb/Alpha (USA)"] [data-action="playing"]')
                await pg.wait_for_selector('#backlog-todo-list .bl-row[data-key="gb/Alpha (USA)"][data-playing] .bl-badge')
                counted = await pg.inner_text("#backlog-head .page-count")
                tint = await pg.evaluate("() => [getComputedStyle(document.querySelector('.bl-row[data-playing]')).borderLeftColor, getComputedStyle(document.querySelector('.bl-row:not([data-playing])')).borderLeftColor]")
                started = json.loads((library / "_library/backlog.json").read_text())["games"][1].get("started")
                check("In progress highlights a game with its label, and the bar counts it (D159)", "1 in progress" in counted and started and tint[0] != tint[1], (counted, started, tint))
                # dragged to Completed: completed, with the date; Undo puts it back where it was
                box = await pg.locator("#backlog-done-list").bounding_box()
                await drag(f'#backlog-todo-list .bl-row[data-key="{pc_key}"]', box["x"] + 80, box["y"] + 30)
                await pg.wait_for_selector(f'#backlog-done-list .bl-row[data-key="{pc_key}"]')
                done_text = await pg.inner_text(f'#backlog-done-list .bl-row[data-key="{pc_key}"]')
                said = await toast_text(pg, "Marked .* completed")
                left = await order()
                await pg.click("#toast .toast-action")
                await pg.wait_for_selector(f'#backlog-todo-list .bl-row[data-key="{pc_key}"]')
                check("a game dragged to Completed is in the list beside with the date it was completed; Undo puts it back in its place",
                      "completed" in done_text and len(left) == 2 and await order() == dragged and saved_order() == dragged, (done_text, said, left, await order()))
                await pg.click(f'#backlog-todo-list .bl-row[data-key="{pc_key}"] [data-action="complete"]')
                await pg.wait_for_selector(f'#backlog-done-list .bl-row[data-key="{pc_key}"]')
                # Alt+↓ a place down; Move to place… from its menu
                await pg.click('#backlog-todo-list .bl-row[data-key="nes/Echo (USA)"] .bl-title')
                await pg.keyboard.press("Alt+ArrowDown")
                await pg.wait_for_function("() => document.querySelector('#backlog-todo-list .bl-row')?.dataset.key === 'gb/Alpha (USA)'")
                stepped = await order()
                await pg.click('#backlog-todo-list .bl-row[data-key="nes/Echo (USA)"]', button="right")
                await pg.click('#context-menu [data-action="bl-place"]')
                await pg.fill("#shelf-ask-value", "1")
                await pg.click("#shelf-ask button[type=submit]")
                await pg.wait_for_function("() => document.querySelector('#backlog-todo-list .bl-row')?.dataset.key === 'nes/Echo (USA)'")
                check("Alt+↓ moves the game picked a place down, and Move to place… puts it where it's asked (D158)",
                      stepped == ["gb/Alpha (USA)", "nes/Echo (USA)"] and saved_order() == ["nes/Echo (USA)", "gb/Alpha (USA)"], (stepped, saved_order()))
                # a game opened from the backlog: its place on its page, In progress there, Previous and Next through the backlog
                await pg.dblclick('#backlog-todo-list .bl-row[data-key="nes/Echo (USA)"] .bl-title')
                await pg.wait_for_selector('#game-page[data-key="nes/Echo (USA)"] #game-backlog-line')
                line = await pg.inner_text("#game-backlog-line")
                pos = await pg.inner_text(".game-pos")
                await pg.click("#game-backlog-playing")
                await pg.wait_for_selector('#game-backlog-playing[aria-pressed="true"]')
                await pg.click("#game-next")
                await pg.wait_for_selector('#game-page[data-key="gb/Alpha (USA)"] #game-backlog-line')
                line2 = await pg.inner_text("#game-backlog-line")
                await pg.click("#game-back")
                await pg.wait_for_selector('#backlog-todo-list .bl-row.selected[data-key="gb/Alpha (USA)"]')
                playing = await pg.eval_on_selector_all("#backlog-todo-list .bl-row[data-playing]", "els => els.map(e => e.dataset.key)")
                check("a game opened from the backlog shows its place there and In progress on its page; Previous and Next go through the backlog, and Back picks it again",
                      line.startswith("1 of 2 to play") and pos.endswith("in the backlog") and line2.startswith("2 of 2 to play") and "In progress" in line2
                      and sorted(playing) == ["gb/Alpha (USA)", "nes/Echo (USA)"], (line, pos, line2, playing))
                # Add games… by title, to the top; Back to the backlog from Completed (at the bottom)
                await pg.keyboard.press("Escape")
                await pg.click("#backlog-add")
                await pg.fill("#backlog-add-search", "Beta")
                await pg.wait_for_selector("#backlog-add-list li input")
                await pg.click("#backlog-add-list li >> nth=0 >> input")
                await pg.click("#backlog-add-top")
                await pg.click("#backlog-add-dialog button[type=submit]")
                said = await toast_text(pg, "Added .* to the backlog")
                await pg.wait_for_function("() => document.querySelector('#backlog-todo-list .bl-row')?.dataset.key.startsWith('gb/Beta')")
                await pg.click(f'#backlog-done-list .bl-row[data-key="{pc_key}"] [data-action="again"]')
                await pg.wait_for_function("(k) => [...document.querySelectorAll('#backlog-todo-list .bl-row')].pop()?.dataset.key === k", arg=pc_key)
                await pg.mouse.move(5, 5)
                await pg.screenshot(path=str(out / "52b-backlog.png"))
                now = await order()
                check("Add games… finds games by title and adds them at the top when asked; Back to the backlog puts a completed game at the bottom of To play",
                      len(now) == 4 and now[0].startswith("gb/Beta") and now[-1] == pc_key and not await pg.query_selector("#backlog-done-list .bl-row"), (said, now))
                # Delete takes one out; Ctrl+Z puts it back
                await pg.click(f'#backlog-todo-list .bl-row[data-key="{pc_key}"] .bl-title')
                await pg.keyboard.press("Delete")
                await toast_text(pg, "Removed .* from the backlog")
                gone = await order()
                await pg.keyboard.press("Control+z")
                await pg.wait_for_function("(k) => !!document.querySelector(`#backlog-todo-list .bl-row[data-key=\"${k}\"]`)", arg=pc_key)
                check("Delete takes a game out of the backlog, and Ctrl+Z puts it back", pc_key not in gone and await order() == now, (gone, await order()))

            async with Section(pg, B, "Phase 6b: each source's answer, Delete metadata and videos"):
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
                # a game's video (batch 4, Jerred: its sound played twice): a small one in the ribbon
                # is an icon, not a player; one video starting pauses the other; one the app can't
                # play offers the system's own player. A short WebM is recorded by the page itself.
                webm = await pg.evaluate("""async () => {
                  const c = document.createElement('canvas'); c.width = 64; c.height = 48;
                  const g = c.getContext('2d'); let i = 0;
                  const rec = new MediaRecorder(c.captureStream(15), { mimeType: 'video/webm' });
                  const parts = []; rec.ondataavailable = (e) => parts.push(e.data);
                  const done = new Promise((r) => { rec.onstop = r; });
                  const t = setInterval(() => { g.fillStyle = `hsl(${i++ * 40},70%,50%)`; g.fillRect(0, 0, 64, 48); }, 50);
                  rec.start(); await new Promise((r) => setTimeout(r, 1200)); rec.stop(); await done; clearInterval(t);
                  const b = new Uint8Array(await new Blob(parts).arrayBuffer()); let s = '';
                  for (const x of b) s += String.fromCharCode(x);
                  return btoa(s);
                }""")
                (alpha / "media/video.webm").write_bytes(base64.b64decode(webm))
                await pg.evaluate("() => { location.hash = '#/library'; }")
                await pg.wait_for_selector("#lib-page")
                await pg.evaluate("() => { location.hash = '#/game/gb/' + encodeURIComponent('Alpha (USA)'); }")
                await pg.wait_for_selector('#game-strip [data-kind="video"] .game-strip-video')
                await pg.click('#game-strip [data-kind="video"]')
                await pg.wait_for_selector("#game-gallery video.game-hero")
                await pg.wait_for_selector('#game-media .media-kind[data-kind="video"] .media-thumb')
                small_players = await pg.eval_on_selector_all('#game-media .media-kind[data-kind="video"] video', "els => els.length")
                await pg.click('#game-media .media-kind[data-kind="video"] .media-thumb')
                await pg.wait_for_selector("#media-view video")
                await pg.wait_for_function("() => [...document.querySelectorAll('#game-gallery video, #media-view video')].every((v) => v.readyState >= 1)")
                playing = await pg.evaluate("""async () => {
                  const [hero, big] = [document.querySelector('#game-gallery video'), document.querySelector('#media-view video')];
                  for (const v of [hero, big]) v.muted = true;
                  await hero.play(); await big.play();
                  await new Promise((r) => setTimeout(r, 200));
                  return [hero.paused, big.paused, document.querySelectorAll('video').length];
                }""")
                await pg.click("#media-view-close")
                (alpha / "media/video.webm").unlink()
                (alpha / "media/video.avi").write_bytes(b"RIFF\x00\x00\x00\x00AVI LIST" + bytes(64))
                await pg.evaluate("() => { location.hash = '#/library'; }")
                await pg.wait_for_selector("#lib-page")
                await pg.evaluate("() => { location.hash = '#/game/gb/' + encodeURIComponent('Alpha (USA)'); }")
                await pg.click('#game-strip [data-kind="video"]')
                await pg.wait_for_selector('#game-gallery [data-video="cant-play"]')
                cant = await pg.inner_text('#game-gallery [data-video="cant-play"]')
                check("a game's video: a small one is an icon, not a player; one starting pauses the other; one the app can't play offers its own player",
                      small_players == 0 and playing == [True, False, 2] and "can't play" in cant, (small_players, playing, cant))
                (alpha / "media/video.avi").unlink()
                await pg.evaluate("() => { location.hash = '#/library'; }")
                await pg.wait_for_selector("#lib-page")
                await pg.evaluate("() => { location.hash = '#/game/gb/' + encodeURIComponent('Alpha (USA)'); }")
                await pg.wait_for_selector('#game-page[data-key="gb/Alpha (USA)"] #game-variants')
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
                await pg.wait_for_selector("#scrape-dialog #scrape-sources")
                # review M42, D80: what's scraped is a line, its choices behind Change what's scraped…
                short = await pg.evaluate("() => { const d = document.querySelector('#scrape-dialog'); return [!document.querySelector('#scrape-media'), d.scrollHeight <= d.clientHeight + 2, document.querySelector('#scrape-what-text')?.textContent || ''] }")
                await pg.screenshot(path=str(out / "56b-scrape-short.png"))
                await pg.click("#scrape-change")
                await pg.wait_for_selector("#scrape-dialog #scrape-variants")
                check("the Scrape window fits without scrolling, what's scraped in one line, its choices behind Change what's scraped… (D80)", short[0] and short[1] and "detail" in short[2], short)
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
                check("Settings › Details and pictures shows the space they take, by system and kind, and which source each detail comes from",
                      "Game Boy" in shown and "Box front" in shown and sizes.get("kinds", {}).get("cover", 0) > 0 and await pg.locator('#details-from select[data-field="description"]').count() == 1, (shown[:300], sizes.get("kinds")))
                await pg.screenshot(path=str(out / "55-settings-metadata.png"), full_page=True)

            async with Section(pg, B, "the theme and the skin"):
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

                # 8. the skin: Classic unless chosen, changed at once and kept
                first = await pg.evaluate("() => document.documentElement.dataset.skin")
                drawn = lambda: pg.evaluate("() => [...document.querySelectorAll('.nav-link[title=Home] svg g[data-sk]')].find((g) => getComputedStyle(g).display !== 'none')?.dataset.sk")
                icon_before = await drawn()
                options = await pg.eval_on_selector_all("#skin-select option", "els => els.map(e => e.value)")
                await pg.select_option("#skin-select", "tome")
                skin = await pg.evaluate("() => document.documentElement.dataset.skin")
                await pg.wait_for_timeout(500)
                saved = prefs().get("skin")
                await pg.select_option("#skin-select", "classic")
                check("the skin is Classic unless chosen; another changes the look at once and is kept",
                      first == "classic" and icon_before == "classic" and options[0] == "classic" and "drafting" not in options
                      and skin == "tome" and saved == "tome", (first, icon_before, options, skin, saved))
                await pg.screenshot(path=str(out / "03-classic.png"))
                # the skins of skins.css: each changes the colours and lettering, says what it looks like,
                # and draws Classic's icons; a screenshot of Home in each
                looks, abouts = {}, set()
                for sk in options[1:]:
                    await pg.select_option("#skin-select", sk)
                    looks[sk] = await pg.evaluate("() => { const st = getComputedStyle(document.body); return [st.backgroundColor, st.fontFamily.split(',')[0], getComputedStyle(document.documentElement).getPropertyValue('--tool').trim()].join(' | '); }")
                    abouts.add(await pg.inner_text("#skin-about"))
                    icons = await drawn()
                    if icons != "classic":
                        looks[sk] += f" | icons {icons}"
                    await pg.click('.sidebar a[href="#/"]')
                    await pg.wait_for_selector("#home-platforms")
                    await pg.wait_for_timeout(300)
                    await pg.screenshot(path=str(out / f"03-skin-{sk}.png"))
                    await pg.click('.sidebar a[href="#/settings"]')
                    await pg.wait_for_selector("#skin-select")
                await pg.select_option("#skin-select", "classic")
                check("each further skin has its own colours, lettering and words in Settings, with Classic's icons",
                      {"neon", "terminal", "tome", "pixel", "cosy"} <= set(looks) and len(set(looks.values())) == len(looks)
                      and not any("icons" in v for v in looks.values()) and len(abouts) == len(looks), looks)

            async with Section(pg, B, "batch 4: narrow windows, Previous and Next, Enter, Metadata's system"):
                # batch 4: the Audit bar fits a 1000 px window (review H22); a game opened from
                # elsewhere has no Previous and Next (M33); Enter opens a game's tile (M38); the
                # filters' counts show at 1280 px (M43); a remembered system with no DAT doesn't
                # leave Metadata on "To start" (H23)
                await pg.set_viewport_size({"width": 1000, "height": 800})
                await pg.evaluate("() => { location.hash = '#/audit/nes'; }")
                await pg.wait_for_selector('#audit-pick[data-platform="nes"]', state="attached")
                await pg.wait_for_selector("#audit-convert")
                bar = await pg.evaluate("() => [document.scrollingElement.scrollWidth, innerWidth, Math.round(document.querySelector('#audit-head .more-btn-bar, #audit-head [id$=menu]')?.getBoundingClientRect().right || 0)]")
                await pg.set_viewport_size({"width": 1280, "height": 900})
                await pg.evaluate("() => { location.hash = '#/game/gb/' + encodeURIComponent('Delta (World)'); }")
                await pg.wait_for_selector('#game-page[data-key="gb/Delta (World)"]')
                walk = await pg.query_selector("#game-prev")
                await pg.evaluate("() => { location.hash = '#/library'; }")
                await pg.wait_for_selector('.lib-tile[data-key="gb/Delta (World)"]')
                filters = await pg.evaluate("() => { const f = document.querySelector('.lib-filters'); return [f.scrollWidth, f.clientWidth]; }")
                await pg.focus('.lib-tile[data-key="gb/Delta (World)"]')
                await pg.keyboard.press("Enter")
                await pg.wait_for_selector('#game-page[data-key="gb/Delta (World)"]')
                inv = "(a) => window.__TAURI__.core.invoke('api', a)"
                kept = await pg.evaluate(f"async () => ({inv})({{ cmd: 'prefs_get', args: {{}} }})")
                kept["gr-meta"] = {**kept.get("gr-meta", {}), "platform": "snes"}
                await pg.evaluate(f"async (p) => ({inv})({{ cmd: 'prefs_set', args: {{ prefs: p }} }})", kept)
                await pg.reload()
                await pg.evaluate("() => { location.hash = '#/metadata'; }")
                await pg.wait_for_selector("#meta-scrape")
                check("the Audit bar fits a narrow window; a game opened from elsewhere has no Previous and Next; Enter opens a game; the filters fit; Metadata picks a system with a DAT",
                      bar[0] <= bar[1] and 0 < bar[2] <= bar[1] and walk is None and filters[0] <= filters[1], (bar, walk, filters))
                await pg.set_viewport_size({"width": 1400, "height": 900})

            async with Section(pg, B, "About, a newer library and a narrow window"):
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
            for i, (name, part) in enumerate(OWN_HOMES, 1):
                await own_home(b, name, a.port + i, part)
            await b.close()
        # the server Serve to devices started again stops with the backend that started it
        # (its input closes), as it does with the app unless it's kept
        served = Path(f"{home}-serve") / "server" / "server.json"
        try:
            await until(lambda: not served.exists(), 30)
            stopped = True
        except TimeoutError:
            stopped = False
        check("Game Repo Server stops when the app that started it ends", stopped)
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
