#!/usr/bin/env python3
"""Acceptance test: Game Repo Go's pages against its real backend (`gamerepo-cli
client-ui`, the command table of the app) and a real Game Repo Server, in Chromium at a
phone's size and a PC's (docs/PLAN.md, "Phase 9b design", Tests).

    python3 tests/client_page.py --cli desktop/target/release/gamerepo-cli --ui build/desktop/ui \\
        --home /tmp/go-page --out shots [--chromium <path>]

The library is made from tests/fixtures (Game Boy and NES, some games with details and
pictures) and served by `gamerepo-cli serve-devices`; the device is a folder with ES-DE's
settings, its folders for Game Boy and NES, and two games of its own (a copy of a game
the library has, renamed, and one it doesn't).

Checks: Game Repo Go asks to be paired, finds nothing wrong with an address typed in, and
a wrong code is refused; the code from Game Repo pairs it and the library is read onto
the device; a system browsed from the picker; a search across every system; three games
ticked, their size against the free space; Add to device… with the media kinds and their
sizes, one unticked; the queue watched to the end and "Restart ES-DE"; the games in ES-DE's
folders and gamelists with the kinds chosen and not the other; On this device with the
games from the library, the copy recognised and the game from elsewhere; Make room ticking
games; a removal shown and done; a game's media removed alone; Settings (Show games on this
device, the look); the Library and a game's page kept with the library away; every page
moved through with the keys alone, and the games by rows and columns; no page errors; and
the library and everything outside ES-DE's folders as they were. Screenshots for each.
"""
import argparse
import asyncio
import hashlib
import json
import os
import queue
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import zlib
from pathlib import Path

from playwright.async_api import async_playwright

sys.path.insert(0, str(Path(__file__).with_name("fixtures")))
import make_fixtures  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--cli", required=True)
ap.add_argument("--ui", required=True)
ap.add_argument("--home", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--chromium", default=None)
a = ap.parse_args()

cli = str(Path(a.cli).resolve())
ui = Path(a.ui).resolve()
home = Path(a.home).resolve()
out = Path(a.out).resolve()
out.mkdir(parents=True, exist_ok=True)
SHIM = Path(__file__).with_name("tauri_shim.js")
if home.exists():
    shutil.rmtree(home)
home.mkdir(parents=True)
lib, fixtures, data, phone, go_data = home / "Library", home / "fixtures", home / "server", home / "phone", home / "go-data"
results, errors = [], []


def check(what, ok, detail=""):
    results.append((what, bool(ok)))
    print(("PASS " if ok else "FAIL ") + what + ("" if ok else f": {detail}"), flush=True)


def run(*args, ok=True):
    r = subprocess.run([cli, *map(str, args)], capture_output=True, text=True, encoding="utf-8", timeout=300)
    if ok and r.returncode != 0:
        raise SystemExit(f"gamerepo-cli {' '.join(map(str, args))} failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def snapshot(*dirs):
    """Every file with its bytes' hash and modified time."""
    snap = {}
    for d in dirs:
        for p in sorted(Path(d).rglob("*")):
            if p.is_file():
                snap[str(p)] = (hashlib.sha1(p.read_bytes()).hexdigest(), p.stat().st_mtime_ns)
    return snap


# ---- the library: the fixtures' Game Boy and NES games, some with details and pictures ----
make_fixtures.main(fixtures)
lh = ("--home", home / "app", "--library", lib)
run("roots", "add", fixtures / "Roms" / "Game Boy", "--platform", "gb", *lh)
run("roots", "add", fixtures / "Roms" / "NES", "--platform", "nes", *lh)
run("dat", "import", fixtures / "dats" / "gb.dat", fixtures / "dats" / "nes.zip", *lh)
run("scan", *lh)
ids = {}
for line in run("games", "gb", *lh).stdout.splitlines() + run("games", "nes", *lh).stdout.splitlines():
    parts = line.split()
    if parts and parts[0].isdigit():
        ids[" ".join(parts[1:]).split("  [")[0].split(" [")[0]] = parts[0]
pictures = {"Alpha (USA)": (200, 60, 60), "Beta (Europe)": (60, 160, 90), "Echo (USA)": (70, 90, 200)}
for name, rgb in pictures.items():
    run("game", ids[name], "--edit", "description", f"{name.split(' (')[0]} is a game made for this test.", *lh)
    media = lib / "_library" / "games" / ("nes" if name.startswith("Echo") else "gb") / name / "media"
    media.mkdir(parents=True, exist_ok=True)
    (media / "cover.png").write_bytes(png(300, 400, rgb))
    (media / "screenshot.png").write_bytes(png(320, 288, tuple(255 - c for c in rgb)))
run("game", ids["Alpha (USA)"], "--edit", "players", "2", *lh)
run("library", *lh)
lib_id = json.loads((lib / "_library" / "library.json").read_text(encoding="utf-8"))["id"]

# ---- the device: ES-DE's settings, its folders for each system, two games of its own ----
(phone / "ES-DE" / "settings").mkdir(parents=True)
(phone / "ES-DE" / "settings" / "es_settings.xml").write_text('<?xml version="1.0"?>\n<string name="ROMDirectory" value="~/ROMs" />\n', encoding="utf-8")
(phone / "ES-DE" / "gamelists" / "gb").mkdir(parents=True)
(phone / "ES-DE" / "gamelists" / "gb" / "gamelist.xml").write_text(
    '<?xml version="1.0"?>\n<gameList>\n\t<game>\n\t\t<path>./My Alpha.gb</path>\n\t\t<name>My Alpha</name>\n\t\t<lastplayed>20240101T120000</lastplayed>\n\t</game>\n</gameList>\n', encoding="utf-8")
for system in ("gb", "nes"):
    (phone / "ROMs" / system).mkdir(parents=True)
shutil.copy2(fixtures / "Roms" / "Game Boy" / "Alpha (USA).gb", phone / "ROMs" / "gb" / "My Alpha.gb")
(phone / "ROMs" / "gb" / "Homebrew.gb").write_bytes(bytes(range(256)) * 64)
(phone / "Saves").mkdir()
(phone / "Saves" / "Alpha (USA).srm").write_bytes(b"a save")
before = snapshot(lib, fixtures, phone / "Saves", phone / "ES-DE" / "settings")

# ---- the server, as Game Repo starts it, and Game Repo Go's backend --------------------
lines = queue.Queue()
server = subprocess.Popen([cli, "serve-devices", "--data", str(data), "--port", "0", "--no-watch", "--home", str(home / "app"), "--library", str(lib),
                           "--index-from", str(home / "app" / "data" / "index" / f"{lib_id}.sqlite")],
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
threading.Thread(target=lambda: [lines.put(l.rstrip("\n")) for l in server.stdout], daemon=True).start()


def wait_for(prefix, secs=120):
    until = time.time() + secs
    said = []
    while time.time() < until:
        try:
            line = lines.get(timeout=0.5)
        except queue.Empty:
            if server.poll() is not None:
                break
            continue
        said.append(line)
        if line.startswith(prefix):
            return line
    raise SystemExit(f"the server didn't say '{prefix}':\n" + "\n".join(said))


port = int(wait_for("at ").split()[1].split(",")[0].rsplit(":", 1)[1])
wait_for("ready")
address = f"127.0.0.1:{port}"
ui_port = free_port()
go = subprocess.Popen([cli, "client-ui", "--ui", str(ui), "--home", str(phone), "--data", str(go_data), "--port", str(ui_port), "--kind", "windows"],
                      stdout=subprocess.DEVNULL, stderr=open(out / "client-ui.log", "w", encoding="utf-8"))
B = f"http://127.0.0.1:{ui_port}/"
for _ in range(100):
    try:
        with socket.create_connection(("127.0.0.1", ui_port), timeout=0.2):
            break
    except OSError:
        time.sleep(0.1)


def code():
    return json.loads(run("serve-devices", "pair", "--data", data, "--json").stdout)["code"]


PHONE = {"width": 390, "height": 844}
PC = {"width": 1280, "height": 800}


async def shot(pg, name):
    await pg.screenshot(path=str(out / f"{name}.png"))


async def toast(pg, text, timeout=20000):
    await pg.wait_for_function("(t) => document.querySelector('#go-toast')?.textContent.includes(t)", arg=text, timeout=timeout)
    return await pg.inner_text("#go-toast")


async def cards(pg):
    return await pg.eval_on_selector_all("#go-games .go-card[data-key]", "els => els.map(e => e.dataset.key)")


async def focus_order(pg, presses, key="ArrowDown"):
    """What each press of `key` focuses: its id, else its text."""
    seen = []
    for _ in range(presses):
        await pg.keyboard.press(key)
        seen.append(await pg.evaluate("() => { const e = document.activeElement; return e?.id || e?.dataset?.index && 'card ' + e.dataset.index"
                                      " || (e?.parentElement?.closest('[id]')?.id || '') + ':' + (e?.name || (e?.textContent || '').trim().slice(0, 40)); }"))
    return seen


async def main():
    async with async_playwright() as p:
        b = await p.chromium.launch(executable_path=a.chromium)
        ctx = await b.new_context(viewport=PHONE, device_scale_factor=2, has_touch=True)
        await ctx.add_init_script(path=str(SHIM))
        pg = await ctx.new_page()
        pg.on("pageerror", lambda e: errors.append(str(e)))
        pg.on("console", lambda m: errors.append(f"{m.text} ({m.location.get('url', '')})") if m.type == "error" else None)
        try:
            await pg.goto(B)

            # 1. pairing: the first page until it's paired
            await pg.wait_for_selector("#pair-page")
            await pg.wait_for_selector("#pair-none, .go-found-item", timeout=20000)
            check("Game Repo Go asks to be paired with a library first", "Pair with your library" in await pg.inner_text("#pair-page"))
            await shot(pg, "01-phone-pair")
            await pg.fill("#pair-address", address)
            await pg.fill("#pair-code", "000000" if code() != "000000" else "000001")
            await pg.click("#pair-go")
            await pg.wait_for_selector("#pair-error")
            check("a wrong code is refused, and said so", await pg.inner_text("#pair-error") != "", await pg.inner_text("#pair-error"))
            await pg.fill("#pair-code", code())
            await pg.click("#pair-go")
            said = await toast(pg, "Paired with")
            await pg.wait_for_function("() => (document.querySelector('#go-games-total')?.dataset.total | 0) >= 4", timeout=60000)
            st = json.loads((go_data / "settings.json").read_text(encoding="utf-8"))
            check("the code from Game Repo pairs it, and the library is read onto the device", "Paired with" in said and st["library"] and (go_data / "catalogue.sqlite").exists() or any(go_data.glob("*.sqlite")), (said, st))
            keys = await cards(pg)
            check("the Library shows every game of the systems ES-DE has folders for", sorted(keys) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)", "nes/Echo (USA)"], keys)
            await pg.wait_for_function("() => document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] img')?.naturalWidth > 0", timeout=20000)
            check("box fronts come through the picture protocol (from the library, then the device's cache)", True)
            await shot(pg, "02-phone-library")

            # 2. a system from the picker, then a search across every system
            await pg.select_option("#go-place-pick", "s:nes")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 1")
            check("a system picked shows its games", await cards(pg) == ["nes/Echo (USA)"], await cards(pg))
            await pg.fill("#go-search", "be")
            await pg.wait_for_function("() => [...document.querySelectorAll('#go-games .go-card[data-key]')].map(e => e.dataset.key).join() === 'gb/Beta (Europe)'")
            check("a search finds games across every system", await pg.inner_text("#go-place-name") == "Search")
            await pg.fill("#go-search", "")
            await pg.select_option("#go-place-pick", "")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 4")

            # 3. three games ticked: their size against the free space
            for k in ("gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"):
                await pg.click(f'.go-card[data-key="{k}"] .go-tick')
            bar = await pg.inner_text("#go-picked")
            check("three games ticked show their size against the free space", "3 games" in bar and "free" in bar, bar)
            await shot(pg, "03-phone-ticked")

            # 4. Add to device…: the kinds with their sizes, Screenshot unticked
            await pg.click("#go-add")
            await pg.wait_for_selector("#add-kinds input[name=cover]")
            kinds = await pg.inner_text("#add-kinds")
            ticked = await pg.eval_on_selector_all("#add-kinds input", "els => els.filter(e => e.checked).map(e => e.name)")
            check("Add to device… shows the media kinds the library has, with sizes, ticked as Settings says", "Box front" in kinds and "Screenshot" in kinds and "KB" in kinds and "cover" in ticked and "screenshot" in ticked, (kinds, ticked))
            await pg.uncheck("#add-kinds input[name=screenshot]")
            await shot(pg, "04-phone-add")
            label = await pg.inner_text("#add-dialog button[type=submit]")
            await pg.click("#add-dialog button[type=submit]")
            said = await toast(pg, "added to the queue")
            check("Add 3 games puts them in the queue", label.startswith("Add 3 games (") and "3 games" in said, (label, said))
            check("the games added come off the ticks", await pg.query_selector("#go-picked") is None)

            # 5. the queue to the end, then Restart ES-DE
            await pg.click("#go-nav-queue")
            await pg.wait_for_selector("#go-queue")
            await shot(pg, "05-phone-queue")
            await pg.wait_for_selector("#queue-restart", timeout=120000)
            done = await pg.eval_on_selector_all("#queue-done li", "els => els.map(e => e.dataset.key)")
            check("the queue is watched to the end, and says to restart ES-DE", sorted(done) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"] and "Restart ES-DE" in await pg.inner_text("#queue-restart"), done)
            await shot(pg, "06-phone-queue-done")
            roms = sorted(x.name for x in (phone / "ROMs" / "gb").iterdir())
            gamelist = (phone / "ES-DE" / "gamelists" / "gb" / "gamelist.xml").read_text(encoding="utf-8")
            media = phone / "ES-DE" / "downloaded_media" / "gb"
            covers = sorted(x.name for x in (media / "covers").iterdir()) if (media / "covers").is_dir() else []
            check("the games are in ES-DE's folder and gamelist, with the box fronts and not the screenshots",
                  {"Alpha (USA).gb", "Beta (Europe).gb"} <= set(roms) and any(r.startswith("Delta (World)") for r in roms)
                  and "./Alpha (USA).gb" in gamelist and "./My Alpha.gb" in gamelist and covers == ["Alpha (USA).png", "Beta (Europe).png"]
                  and not (media / "screenshots").exists(), (roms, covers, gamelist))
            await pg.click("#queue-restart button")
            await pg.wait_for_selector("#queue-restart", state="detached")

            # 6. On this device: the library's games, the copy recognised, the game from elsewhere
            await pg.click("#go-nav-device")
            await pg.wait_for_selector("#device-list")
            await pg.wait_for_function("() => !document.querySelector('#device-checking') && document.querySelector('[data-id=\"gb/My Alpha.gb\"] .go-from-known')", timeout=60000)
            rows = await pg.eval_on_selector_all("#device-list > li", "els => els.map(e => [e.dataset.id, e.querySelector('.go-from').textContent])")
            froms = dict(rows)
            check("On this device lists every game ES-DE has, with where each came from",
                  froms.get("gb/Alpha (USA).gb") == "From the library" and froms.get("gb/My Alpha.gb") == "The library has it" and froms.get("gb/Homebrew.gb") == "From elsewhere"
                  and "free" in await pg.inner_text("#device-sum"), rows)
            await shot(pg, "07-phone-device")

            # 7. Make room ticks the games played longest ago first
            await pg.click("#device-room")
            await pg.fill("#room-gb", "0.00001")
            await pg.click("#room-dialog button[type=submit]")
            await pg.wait_for_selector("#device-room-note")
            ticked = await pg.eval_on_selector_all("#device-list > li.is-ticked", "els => els.map(e => e.dataset.id)")
            check("Make room ticks the game played longest ago first", ticked == ["gb/My Alpha.gb"], (ticked, await pg.inner_text("#device-room-note")))
            await pg.click("#device-picked button:has-text('Clear')")

            # 8. a removal: what goes and the room it frees, then gone (the rest as it was)
            await pg.check('[data-id="gb/Homebrew.gb"] .go-row-tick')
            await pg.click("#device-remove")
            await pg.wait_for_selector("#remove-files li")
            listed = await pg.inner_text("#remove-dialog")
            check("Remove… shows each file and the room it frees first", "Homebrew.gb" in listed and "1 game goes. This frees 16 KB." in listed, listed)
            await shot(pg, "08-phone-remove")
            await pg.click("#remove-dialog button[type=submit]")
            said = await toast(pg, "Removed 1 game")
            await pg.wait_for_selector('[data-id="gb/Homebrew.gb"]', state="detached")
            check("the game from elsewhere is removed, and nothing is left set aside",
                  not (phone / "ROMs" / "gb" / "Homebrew.gb").exists() and (phone / "ROMs" / "gb" / "My Alpha.gb").exists()
                  and not any(phone.rglob(".gamerepo-removing")) and not (phone / "ES-DE" / "gamerepo" / "removing.json").exists(), said)

            # 9. a game's media on its own: Alpha's box front removed, the game kept
            await pg.click('[data-id="gb/Alpha (USA).gb"] .go-row-open')
            await pg.click('[data-id="gb/Alpha (USA).gb"] .go-sizes button:has-text("Remove")')
            await pg.wait_for_selector("#remove-files li")
            await pg.click("#remove-dialog button[type=submit]")
            await toast(pg, "Removed, freeing")
            check("a game's box front is removed on its own, and the game stays",
                  not (media / "covers" / "Alpha (USA).png").exists() and (phone / "ROMs" / "gb" / "Alpha (USA).gb").exists() and (media / "covers" / "Beta (Europe).png").exists())

            # 10. Settings: Show games on this device, and the look
            await pg.click("#go-nav-settings")
            await pg.wait_for_selector("#go-settings")
            await shot(pg, "09-phone-settings")
            await pg.check("#set-on-device")
            await pg.select_option("#set-theme", "night")
            theme = await pg.evaluate("() => document.documentElement.dataset.theme")
            await pg.click("#go-nav-library")
            await pg.wait_for_function("() => document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] .go-on-device')", timeout=20000)
            check("Show games on this device shows them, marked; the theme follows Settings", theme == "night" and len(await cards(pg)) == 4, theme)
            await shot(pg, "10-phone-library-night")
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-theme")
            check("the look is kept", await pg.evaluate("() => document.documentElement.dataset.theme") == "night")
            await pg.select_option("#set-theme", "system")

            # 11. a PC's size: the menu down the side, the systems beside the games, a game's page
            await pg.set_viewport_size(PC)
            await pg.goto(B + "#library")
            await pg.wait_for_selector("#go-places")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 4")
            side = await pg.evaluate("() => getComputedStyle(document.querySelector('.go-nav')).flexDirection")
            check("at a PC's size the pages are down the side, the systems beside the games", side == "column" and await pg.is_visible("#go-places") and not await pg.is_visible("#go-place-pick"))
            await shot(pg, "11-pc-library")
            await pg.click('.go-card[data-key="gb/Alpha (USA)"] .go-card-open')
            await pg.wait_for_selector("#go-game-title")
            text = await pg.inner_text("#go-game")
            check("a game's page shows its details, pictures and that it's on the device", "Alpha is a game made for this test." in text and "Players" in text and await pg.is_visible("#go-game-on"), text[:300])
            await shot(pg, "12-pc-game")

            # 12. every page moved through with the keys alone
            await pg.goto(B + "#library")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 4")
            await pg.evaluate("() => document.activeElement?.blur()")
            order = await focus_order(pg, 12)
            check("the arrow keys go through the menu, the systems, the search and the order to the games", order[1:5] == ["go-nav-library", "go-nav-queue", "go-nav-device", "go-nav-settings"]
                  and "go-search" in order and "go-sort" in order and "card 0" in order, order)
            cols = await pg.evaluate("() => { const c = [...document.querySelectorAll('#go-games .go-card')].map(e => e.offsetTop); return c.filter(t => t === c[0]).length; }")
            await pg.keyboard.press("ArrowRight")
            right = await pg.evaluate("() => document.activeElement?.dataset.index")
            await pg.keyboard.press("ArrowLeft")
            await pg.keyboard.press("x")
            ticked = await pg.eval_on_selector_all(".go-card.is-ticked", "els => els.map(e => e.dataset.key)")
            await pg.keyboard.press("Enter")
            await pg.wait_for_selector("#go-game-title")
            opened = await pg.inner_text("#go-game-title")
            await pg.keyboard.press("Escape")
            await pg.wait_for_selector("#go-games")
            check("in the games Right moves a card, X ticks it and Enter opens it, Esc comes back", right == "1" and len(ticked) == 1 and opened and cols >= 2, (right, ticked, opened, cols))
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-version")
            await pg.evaluate("() => document.activeElement?.blur()")
            stops = await pg.evaluate("() => [...document.querySelectorAll('#go-root button, #go-root input, #go-root select, #go-root a[href]')].filter(e => e.getClientRects().length).length")
            order = await focus_order(pg, stops + 2)
            check("Settings: Down reaches every control in turn", "set-theme" in order and "set-on-device" in order and "set-unpair" in order and len(set(order)) >= stops - 1, (stops, order))
            await shot(pg, "13-pc-settings")

            # 13. away from home: the library's list and a game's details are kept on the device
            server.terminate()
            server.wait(timeout=30)
            await pg.goto(B + "#library")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 4", timeout=20000)
            await pg.goto(B + "#game/" + "gb%2FAlpha%20(USA)")
            await pg.wait_for_selector("#go-game-title")
            check("with the library away the list and a game's details are still here", "Alpha is a game made for this test." in await pg.inner_text("#go-game"))
            await shot(pg, "14-pc-away")
        except Exception:  # what the page showed when a step failed
            try:
                await shot(pg, "failed")
                print("page says:", (await pg.inner_text("body"))[:1500], flush=True)
            except Exception:
                pass
            raise
        await b.close()


try:
    asyncio.run(main())
except Exception as e:  # a step that timed out is a failure of its own, with what the page said
    import traceback
    traceback.print_exc()
    check("the test ran to the end", False, repr(e))
finally:
    for proc in (go, server):
        if proc.poll() is None:
            proc.terminate()
            proc.wait(timeout=30)

after = snapshot(lib, fixtures, phone / "Saves", phone / "ES-DE" / "settings")
check("the library, the folders with games, saves and ES-DE's settings are as they were", before == after,
      [k for k in set(before) | set(after) if before.get(k) != after.get(k)][:5])
for e in errors:
    print("PAGE ERROR:", e)
failed = [n for n, ok in results if not ok]
print(f"{len(results) - len(failed)} of {len(results)} checks passed")
sys.exit(1 if failed or errors else 0)
