#!/usr/bin/env python3
"""Acceptance test: the desktop app's page against its real backend
(`gamerepo-cli serve`, the same command table as the Tauri app), in Chromium.

    python3 tests/desktop_page.py --cli desktop/target/release/gamerepo-cli --ui build/desktop/ui \\
        --home /tmp/home --out shots [--chromium <path>]

Checks (Phase 0): the first start asks where the library goes and makes only
its _library/ folder there; the menu's places, the pages still to come; the
index is made on this computer (not in the library) and checked as a job;
renaming the library, with Undo; opening a folder that already holds games
leaves them as they were, and switching back from the recent list; the theme
and skin follow the settings and are kept; a library from a newer app opens
read-only; the menu is a drawer in a narrow window; and the library and its
index are the same after the library is moved (library-info, index-info).
Writes <out>/library-info.json and <out>/library.zip for the cross-platform check.
"""
import argparse
import asyncio
import json
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from playwright.async_api import async_playwright

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


def start_server():
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
                  "Where" in asked and coming_off == 4 and made and str(library) in status, (asked, made, status))

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
            await pg.wait_for_selector("#coming-audit")
            audit = await pg.inner_text(".cmdbar h1")
            active = await pg.inner_text(".sidebar .nav-link.active .nav-label")
            lead = await pg.inner_text("#coming-audit")
            check("the menu's places open, with the pages still to come saying what they'll do",
                  labels == ["Game Repo", "Home", "Audit", "Collections", "PC games", "Metadata", "Settings", "About", "Theme: System"]
                  and audit == "Audit" and active == "Audit" and "DAT" in lead and "phase 1" in lead.lower(), (labels, audit, active))
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
                  "Game Repo" in about and "Phase 1" in about and "_library/" in about and "game.json" in about, about[:200])
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
