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
History with Undo and emptying what was set aside;
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
import http.server
import json
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

from playwright.async_api import async_playwright

sys.path.insert(0, str(Path(__file__).with_name("fixtures")))
import make_fixtures  # noqa: E402

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


def start_server():
    starter_source()
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
            status = await pg.inner_text(".statusbar")
            check("the first start asks where the library goes and makes only _library/ there",
                  "Where" in asked and coming_off == 3 and made and str(library) in status, (asked, made, status))

            # 2. the index: made on this computer, under the data folder, not in the library
            await pg.wait_for_selector("#home-index #index-tables")
            tables = await pg.inner_text("#index-tables")
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
                  labels == ["Game Repo", "Home", "Audit", "Collections", "PC games", "Metadata", "Settings", "About", "Theme: System"]
                  and audit == "Audit" and active == "Audit" and "DAT" in lead and "folders" in lead.lower(), (labels, audit, active, lead[:120]))
            await pg.screenshot(path=str(out / "01-coming.png"))

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
            await pg.wait_for_function("() => document.querySelector('.statusbar').textContent.includes('My Games ·')")
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
            await pg.wait_for_function("() => document.querySelector('#index-path')?.textContent.includes('.sqlite')")
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
            await pg.select_option("#dat-platform-0", "__other")
            await pg.fill("#dat-platform-0-other", "satellaview")
            await pg.click("#dat-platform-dialog button[type=submit]")
            await toast_text(pg, "Imported Nintendo - Satellaview")
            dat_files = sorted(str(x.relative_to(library / "_library/dats")).replace("\\", "/") for x in (library / "_library/dats").rglob("*.dat"))
            remembered = json.loads((library / "_library/library.json").read_text()).get("dat_platforms", {})
            check("DATs are imported (a zipped one too) and copied into the library; an unknown one is asked about and remembered",
                  "Satellaview" in asked and len(dat_files) == 3 and dat_files[0].startswith("gb/") and remembered.get("Nintendo - Satellaview") == "satellaview",
                  (dat_files, remembered))
            await pg.wait_for_selector('#home-platforms [data-platform="gb"]')
            bar = await pg.inner_text('#home-platforms [data-platform="gb"]')
            check("Home shows each platform against its DAT, with 1G1R", "3 of 6" in bar and "1G1R 3 of 5" in bar, bar)
            await pg.screenshot(path=str(out / "10-home-platforms.png"), full_page=True)

            async def tab_names(tab):
                await pg.click(f'#audit-tabs [data-tab="{tab}"]')
                await pg.wait_for_function("(t) => { const l = document.querySelector('#audit-list'); return l && l.dataset.tab === t && l.getAttribute('aria-busy') === 'false'; }", arg=tab)
                return sorted(await pg.eval_on_selector_all("#audit-list .audit-row", "els => els.map(e => e.dataset.name)"))

            await pg.click('.sidebar a[href="#/audit"]')
            await pg.wait_for_selector("#audit-page")
            platform_now = await pg.input_value("#audit-platform")
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
            said = await toast_text(pg, "Read what changed", timeout=30000)
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
            await pg.uncheck("#scan-watch")
            await pg.wait_for_timeout(300)
            watch = json.loads((home / "config/config.json").read_text()).get("scan", {}).get("watch")
            check("Settings › History lists the changes; the newest is undone there, what was set aside is emptied, and watching can be turned off",
                  states == ["done", "done", "undone"] and gone and emptied and watch is False, (states, gone, emptied, watch))
            await pg.check("#scan-watch")

            # Settings: the DATs, and how scans read ZIPs (kept on this computer)
            await pg.click('.sidebar a[href="#/settings"]')
            await pg.wait_for_selector("#settings-dats .dat-group")
            groups = await pg.eval_on_selector_all("#settings-dats .dat-group h3", "els => els.map(e => e.textContent)")
            await pg.select_option("#scan-zips", "full")
            await pg.wait_for_timeout(300)
            cfg = json.loads((home / "config/config.json").read_text())
            check("Settings lists the DATs by platform and keeps the scan settings on this computer",
                  groups == ["Game Boy", "Nintendo Entertainment System", "satellaview"] and cfg.get("scan", {}).get("full") is True, (groups, cfg.get("scan")))
            await pg.screenshot(path=str(out / "12-settings-games.png"), full_page=True)
            await pg.click("#starter-dats")
            await pg.click("#ask-dialog button[type=submit]")
            said = await toast_text(pg, "starter DATs for")
            starters = sorted(x.parent.name for x in (library / "_library/dats").rglob("*.dat") if x.parent.name in ("gamegear", "vectrex"))
            check("Get starter DATs downloads the DATs of the systems that have none", "2 systems" in said and "2 already had one" in said and starters == ["gamegear", "vectrex"], (said, starters))

            # 7. the theme
            await pg.select_option("#theme-select", "night")
            theme = await pg.evaluate("() => document.documentElement.dataset.theme")
            await pg.wait_for_timeout(500)  # preferences are saved after a short pause
            check("the theme follows the setting and is kept", theme == "night" and prefs().get("theme") == "night", (theme, prefs()))
            await pg.screenshot(path=str(out / "03-night.png"))
            await pg.click("#theme-toggle")
            cycled = await pg.evaluate("() => document.documentElement.dataset.theme")
            await pg.select_option("#theme-select", "system")
            check("the menu's theme button goes on to the next choice", cycled in ("light", "dark"), cycled)

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
                  "Game Repo" in about and "Phase 2" in about and "_library/" in about and "game.json" in about, about[:200])
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
