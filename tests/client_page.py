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
ticked, their size against the free space; Add to queue keeping them queued, not started,
and Start downloads… with their size on disc and the free space; Download… with the media
kinds and their sizes, one unticked; Downloads watched to the end and "Restart ES-DE"; the games in ES-DE's
folders and gamelists with the kinds chosen and not the other; On this device with the
games from the library, the copy recognised and the game from elsewhere, by system and in
one list; Make room ticking
games; a removal shown and done; a game's media removed alone; Settings (Show games on this
device, the look, each skin, About and updates with the CLI's stand-in for the updater); the Library and a game's page kept with the library away; every page
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
import urllib.request
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
# Steam on this "device" (D109): a user's folder, and on Linux a `steam` that closes a
# stand-in Steam (a program named steam) when told -shutdown, and notes when it's started
steam_dir, steam_bin = home / "steam" / "Steam", home / "steam" / "bin"
(steam_dir / "userdata" / "22202" / "config").mkdir(parents=True)
(steam_dir / "config").mkdir(parents=True)
(steam_dir / "config" / "loginusers.vdf").write_text('"users"\n{\n\t"76561197960287930"\n\t{\n\t\t"AccountName"\t\t"deck"\n\t\t"PersonaName"\t\t"Deck"\n\t\t"MostRecent"\t\t"1"\n\t}\n}\n')
steam_bin.mkdir(parents=True)
(steam_bin / "steam").write_text(f"""#!/bin/sh
if [ "$1" = "-shutdown" ]; then kill "$(cat '{steam_bin}/running.pid')"; else date > '{steam_bin}/started'; fi
""")
(steam_bin / "steam").chmod(0o755)
go_env = {**os.environ, "PATH": f"{steam_bin}{os.pathsep}{os.environ.get('PATH', '')}"}
if sys.platform == "win32":  # PC games' Start menu shortcuts (D112) go in the test's own
    go_env["APPDATA"] = str(home / "appdata")
for k in ("SteamGamepadUI", "GAMESCOPE_WAYLAND_DISPLAY"):
    go_env.pop(k, None)
go = subprocess.Popen([cli, "client-ui", "--ui", str(ui), "--home", str(phone), "--data", str(go_data), "--port", str(ui_port), "--kind", "windows"],
                      stdout=subprocess.DEVNULL, stderr=open(out / "client-ui.log", "w", encoding="utf-8"), env=go_env)
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


def shell(**changes):
    """What client-ui's shell says it is (kind, All files access), and how often Back left."""
    req = urllib.request.Request(B + "test/shell", data=json.dumps(changes).encode(), method="POST")
    return json.loads(urllib.request.urlopen(req, timeout=10).read())


def update(**to):
    """The CLI's stand-in for the updater (desktop/cli/src/updates.rs): what a check finds and
    how this copy stands, and what the page asked for."""
    req = urllib.request.Request(B + "test/update", data=json.dumps(to).encode() if to else None, method="POST" if to else "GET")
    return json.loads(urllib.request.urlopen(req, timeout=10).read())


async def settled(pg, js, timeout=5000):
    """Once the page shows it (a render comes after the key that asked for it), or on."""
    try:
        await pg.wait_for_function(js, timeout=timeout)
    except Exception:
        pass


async def focus_order(pg, presses, key="ArrowDown"):
    """What each press of `key` focuses: its id, else its text."""
    seen = []
    for _ in range(presses):
        await pg.keyboard.press(key)
        seen.append(await pg.evaluate("() => { const e = document.activeElement; return e?.id || e?.dataset?.index && 'card ' + e.dataset.index"
                                      " || (e?.parentElement?.closest('[id]')?.id || '') + ':' + (e?.name || (e?.textContent || '').trim().slice(0, 40)); }"))
    return seen


def add_pc_game():
    """A PC game added to the library (D110): Zeta, only its install archive, which the server
    unpacks for a device (D111)."""
    import zipfile
    game = lib / "PC" / "Zeta (1999)"
    (game / "install").mkdir(parents=True)
    with zipfile.ZipFile(game / "install" / "Zeta.zip", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("Zeta/zeta.exe", b"MZ zeta")
        z.writestr("Zeta/data/level1.dat", bytes(range(256)) * 256)
    size = (game / "install" / "Zeta.zip").stat().st_size
    (game / "game.json").write_text(json.dumps({"id": "zeta", "title": "Zeta", "year": 1999, "source": "other",
                                                "files": [{"path": "install/Zeta.zip", "kind": "install", "size": size, "crc32": "", "added": ""}]}), encoding="utf-8")
    run("serve-devices", "changed", "--data", data)


async def main():
    global before
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

            # 3b. Add to queue: queued, not started, until Start downloads…, which shows what they take first
            await pg.click("#go-enqueue")
            said = await toast(pg, "added to the queue")
            await pg.click("#go-nav-queue")
            await pg.wait_for_selector("#queue-queued li")
            queued = await pg.eval_on_selector_all("#queue-queued li", "els => els.map(e => e.dataset.key)")
            await pg.click("#queue-start")
            await pg.wait_for_selector("#start-games li")
            summary = await pg.inner_text("#start-dialog")
            await shot(pg, "03b-phone-start")
            await pg.click("#start-dialog button:has-text('Not yet')")
            check("Add to queue keeps games queued, not started, and Start downloads… shows their size on disc and the free space first",
                  sorted(queued) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"] and "3 games" in said and "Size on disc" in summary
                  and "Free on this device" in summary and "Start 3 downloads (" in summary and not (phone / "ROMs" / "gb" / "Alpha (USA).gb").exists(), (queued, said, summary))
            await pg.click("#go-nav-library")
            for k in ("gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"):
                await pg.click(f'.go-card[data-key="{k}"] .go-tick')

            # 4. Download…: the kinds with their sizes, Screenshot unticked
            await pg.click("#go-add")
            await pg.wait_for_selector("#add-kinds input[name=cover]")
            kinds = await pg.inner_text("#add-kinds")
            ticked = await pg.eval_on_selector_all("#add-kinds input", "els => els.filter(e => e.checked).map(e => e.name)")
            check("Download… shows the media kinds the library has, with sizes, ticked as Settings says", "Box front" in kinds and "Screenshot" in kinds and "KB" in kinds and "cover" in ticked and "screenshot" in ticked, (kinds, ticked))
            await pg.uncheck("#add-kinds input[name=screenshot]")
            await shot(pg, "04-phone-add")
            label = await pg.inner_text("#add-dialog button[type=submit]")
            await pg.click("#add-dialog button[type=submit]")
            said = await toast(pg, "Downloading 3 games")
            check("Download 3 games starts them, the queued ones with the kinds chosen now", label.startswith("Download 3 games ("), (label, said))
            check("the games added come off the ticks", await pg.query_selector("#go-picked") is None)

            # 5. Downloads to the end, then Restart ES-DE
            await pg.click("#go-nav-queue")
            await pg.wait_for_selector("#go-queue")
            await shot(pg, "05-phone-queue")
            await pg.wait_for_selector("#queue-restart", timeout=120000)
            done = await pg.eval_on_selector_all("#queue-done li", "els => els.map(e => e.dataset.key)")
            check("Downloads are watched to the end, and say to restart ES-DE", sorted(done) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"] and "Restart ES-DE" in await pg.inner_text("#queue-restart"), done)
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
            rows = await pg.eval_on_selector_all("#device-list .go-row", "els => els.map(e => [e.dataset.id, e.querySelector('.go-from').textContent])")
            froms = dict(rows)
            systems = await pg.eval_on_selector_all("#device-list > li.go-sys", "els => els.map(e => [e.dataset.system, e.querySelector('.go-sys-open').textContent])")
            check("On this device lists every game ES-DE has by system, with where each came from",
                  froms.get("gb/Alpha (USA).gb") == "From the library" and froms.get("gb/My Alpha.gb") == "The library has it" and froms.get("gb/Homebrew.gb") == "From elsewhere"
                  and len(systems) == 1 and systems[0][0] == "gb" and "5 games" in systems[0][1] and "free" in await pg.inner_text("#device-sum"), (rows, systems))
            await shot(pg, "07-phone-device")
            await pg.click("#device-list .go-sys-remove")
            await pg.wait_for_selector("#remove-files li")
            listed = await pg.inner_text("#remove-dialog")
            await pg.click("#remove-dialog button:has-text('Cancel')")
            check("a system's Remove all… shows every one of its games going first", "5 games go." in listed and "Homebrew.gb" in listed and "Alpha (USA).gb" in listed, listed)

            # 7. Make room ticks the games played longest ago first
            await pg.click("#device-room")
            await pg.fill("#room-gb", "0.00001")
            await pg.click("#room-dialog button[type=submit]")
            await pg.wait_for_selector("#device-room-note")
            ticked = await pg.eval_on_selector_all("#device-list .go-row.is-ticked", "els => els.map(e => e.dataset.id)")
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

            # 9b. a game whose files aren't where Game Repo Go put them (its folder changed, say)
            alpha = phone / "ROMs" / "gb" / "Alpha (USA).gb"
            alpha.rename(alpha.with_suffix(".moved"))
            await pg.click("#go-nav-library")
            await pg.click("#go-nav-device")
            await pg.wait_for_selector('[data-id="gb/Alpha (USA).gb"] .go-from-missing')
            row = await pg.inner_text('[data-id="gb/Alpha (USA).gb"]')
            await pg.click('[data-id="gb/Alpha (USA).gb"] .go-row-open')
            again = await pg.query_selector('[data-id="gb/Alpha (USA).gb"] .go-row-again') is not None
            await pg.click('[data-id="gb/Alpha (USA).gb"] .go-row-open')
            alpha.with_suffix(".moved").rename(alpha)
            check("a game whose files aren't where Game Repo Go put them is marked, not counted as there, with Download again…",
                  "Files not found" in row and "its files aren't on this device" in row and again, (row, again))

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

            # Settings › About and updates (D134): a newer Game Repo Go, what's new in it, Update and
            # restart; found by the check as it starts, a notice says so
            started = update()["checks"]
            update(found={"version": "9.9.9", "notes": "## 9.9.9 (2026-10-22)\n\n- Something new.\n\n## 0.0.1 (2020-01-01)\n\n- Something old."})
            await pg.click("#go-settings #update-check")
            await pg.wait_for_selector("#go-settings #update-panel[data-state=available] #update-install")
            state, new = await pg.inner_text("#update-state"), await pg.inner_text("#update-new")
            await shot(pg, "10b-phone-update")
            await pg.click("#update-install")
            await pg.wait_for_selector("#update-panel[data-state=busy]")
            check("Settings › About and updates finds a newer Game Repo Go, with what's new since this one, and Update and restart installs it",
                  started == 1 and "Game Repo Go 9.9.9 is out" in state and "Something new" in new and "Something old" not in new and update()["installs"] == 1, (started, state, new))
            update(checked=None, available=None, restarting=False)
            await pg.goto(B + "#library")
            await pg.reload()
            said = await toast(pg, "Game Repo Go 9.9.9 is out")
            await pg.click("#go-toast button")
            await pg.wait_for_selector("#go-settings #update-panel[data-state=available]")
            check("the check as it starts finds it, and a notice says so and opens Settings", "update it in Settings" in said and "Open Settings" in said
                  and update()["checks"] == 3 and not await pg.query_selector("#go-toast"), said)
            update(found=None, available=None)
            await pg.click("#go-settings #update-check")
            await pg.wait_for_selector("#go-settings #update-panel[data-state=newest]")

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

            # Game Repo's skins reach Game Repo Go's pages (a screenshot of the Library in each)
            skins = await pg.evaluate("async () => (await import('./ui/state.js')).SKINS.map(([v]) => v)")
            shown = []
            for v in skins:
                await pg.goto(B + "#settings")
                await pg.select_option("#set-skin", v)
                await pg.goto(B + "#library")
                await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 4")
                shown.append((v, await pg.evaluate("() => [document.documentElement.dataset.skin, getComputedStyle(document.querySelector('.go-nav')).backgroundImage + getComputedStyle(document.body).fontFamily]")))
                await shot(pg, f"12-pc-skin-{v}")
            check("each of Game Repo's skins reaches Game Repo Go's pages, and is kept", len(skins) >= 2 and all(v == got[0] for v, got in shown)
                  and len({got[1] for _, got in shown}) == len(shown), shown)
            await pg.goto(B + "#settings")
            await pg.select_option("#set-skin", skins[0])

            # 12. every page moved through with the keys alone
            await pg.goto(B + "#library")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 4")
            await pg.evaluate("() => document.activeElement?.blur()")
            order = await focus_order(pg, 12)
            check("the arrow keys go through the menu, the systems, the search and the order to the games", order[1:5] == ["go-nav-library", "go-nav-queue", "go-nav-device", "go-nav-settings"]
                  and "go-search" in order and "go-sort" in order and "card 0" in order, order)
            cols = await pg.evaluate("() => { const c = [...document.querySelectorAll('#go-games .go-card')].map(e => e.offsetTop); return c.filter(t => t === c[0]).length; }")
            await pg.keyboard.press("ArrowRight")
            await settled(pg, "() => document.activeElement?.dataset.index === '1'")
            right = await pg.evaluate("() => document.activeElement?.dataset.index")
            await pg.keyboard.press("ArrowLeft")
            await pg.keyboard.press("x")
            await settled(pg, "() => document.querySelectorAll('.go-card.is-ticked').length")
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

            # 13. on Android: games wait for All files access (D87), asked for from a note on
            # every page; Back closes, goes back, and from the Library leaves the app
            shell(kind="android", granted=False)
            await pg.set_viewport_size(PHONE)
            await pg.goto(B + "#library")
            await pg.reload()
            await pg.wait_for_selector("#go-storage")
            note = await pg.inner_text("#go-storage")
            await shot(pg, "15-phone-android-access")
            await pg.click("#go-storage-allow")
            await pg.wait_for_selector("#go-storage", state="detached")
            check("on Android, games wait for All files access, asked for from the note", "access to all files" in note and shell()["granted"] is True, note)
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-version")
            await pg.goto(B + "#game/" + "gb%2FAlpha%20(USA)")
            await pg.wait_for_selector("#go-game-title")
            await pg.evaluate("() => window.__shimBack()")
            await pg.wait_for_selector("#set-version")
            from_game = await pg.evaluate("() => location.hash")
            await pg.goto(B + "#library")
            await pg.wait_for_selector("#go-shelves .go-tile[data-key]")
            phone_big = await pg.evaluate("""() => { const t = document.querySelector('.go-tabs').getBoundingClientRect();
                return { view: document.documentElement.dataset.view, tabs_low: t.bottom >= innerHeight - 1 && t.top > innerHeight / 2, hints: !!document.querySelector('#go-hints')?.getClientRects().length }; }""")
            await shot(pg, "15b-phone-big-library")
            check("on Android the big-screen view is the usual one, on a phone upright with the pages along the bottom and no button hints",
                  phone_big == {"view": "big", "tabs_low": True, "hints": False}, phone_big)
            left_before = shell()["left"]
            await pg.evaluate("() => window.__shimBack()")
            for _ in range(50):
                if shell()["left"] > left_before:
                    break
                await asyncio.sleep(0.1)
            check("on Android, Back goes back a page, and from the Library leaves the app", from_game == "#settings" and shell()["left"] == left_before + 1
                  and await pg.evaluate("() => location.hash") == "#library", (from_game, shell()))
            # the big screen's size (D132): Smaller draws it all smaller, kept on the device,
            # with the pages still along the bottom
            def tile():
                return pg.evaluate("() => Math.round(document.querySelector('#go-shelves .go-tile[data-key]').getBoundingClientRect().height)")
            await pg.wait_for_selector("#go-shelves .go-tile[data-key]")
            usual = await tile()
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-size")
            sizes = await pg.eval_on_selector_all("#set-size option", "els => els.map(e => e.value)")
            await pg.select_option("#set-size", "smaller")
            await pg.goto(B + "#library")
            await pg.wait_for_selector("#go-shelves .go-tile[data-key]")
            smaller = await tile()
            frame = await pg.evaluate("""() => { const t = document.querySelector('.go-tabs').getBoundingClientRect();
                return [document.documentElement.dataset.size, t.bottom >= innerHeight - 1, JSON.parse(localStorage.getItem('go-look')).size]; }""")
            await shot(pg, "15c-phone-big-smaller")
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-size")
            await pg.select_option("#set-size", "")
            check("Settings › Look › Size draws the big screen smaller, kept on the device, the pages still along the bottom",
                  sizes == ["smaller", "", "larger"] and abs(smaller - usual * .8) <= 2 and frame == ["smaller", True, "smaller"], (sizes, usual, smaller, frame))
            shell(kind="windows")

            # 13b. on Linux and the Steam Deck (D107, D109): Settings › Steam adds Game Repo Go
            # to Steam's library, written at once while Steam's closed; while it runs the change
            # waits, and Restart Steam closes it, writes it and starts it again; in Game Mode A
            # on a text box brings up Steam's keyboard
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-version")
            not_on_windows = await pg.query_selector("#set-steam") is None
            shell(kind="linux", game_mode=True)
            await pg.set_viewport_size({"width": 1280, "height": 800})
            await pg.reload()
            await pg.wait_for_selector("#set-steam-none")
            # full screen (D127): in Game Mode by default, or always or never, kept as chosen
            full = await pg.eval_on_selector_all("#set-full-screen option", "els => els.map(e => [e.value, e.textContent])")
            await pg.select_option("#set-full-screen", "never")
            kept = ""
            for _ in range(30):
                kept = (await pg.evaluate("() => window.__TAURI__.core.invoke('api', { cmd: 'settings', args: {} })"))["full_screen"]
                if kept == "never":
                    break
                await asyncio.sleep(0.1)
            await pg.select_option("#set-full-screen", "usual")
            check("on Linux, Settings › This device › Full screen is in Steam's Game Mode by default, or always or never, and kept",
                  [v for v, _ in full] == ["usual", "always", "never"] and "Game Mode" in full[0][1] and kept == "never", (full, kept))
            await pg.fill("#set-steam-folder input", str(steam_dir))
            await pg.click("#set-steam-folder button:has-text('Save')")
            await pg.wait_for_selector("#set-steam-self")
            await pg.click("#set-steam-self")
            await pg.wait_for_selector("#set-steam-done")
            vdf = (steam_dir / "userdata" / "22202" / "config" / "shortcuts.vdf").read_bytes()
            mine = b"Game Repo Go" in vdf and Path(cli).name.encode() in vdf
            await shot(pg, "16-pc-linux-steam")
            check("on Linux, Settings adds Game Repo Go to Steam's library, written while Steam's closed",
                  not_on_windows and mine, (not_on_windows, vdf[:200]))
            # A on a text box, from a controller, in Game Mode: Steam's keyboard
            before_kb = shell()["keyboard"]
            await pg.evaluate("""() => {
                const pad = { id: "Steam Virtual Gamepad", connected: true, axes: [0, 0, 0, 0], buttons: Array.from({ length: 17 }, () => ({ pressed: false, value: 0 })) };
                window.__pad = pad;
                Object.defineProperty(navigator, "getGamepads", { value: () => [pad], configurable: true });
                document.querySelector("#set-name").focus();
                dispatchEvent(new Event("gamepadconnected"));
            }""")
            await asyncio.sleep(0.3)
            await pg.evaluate("() => { window.__pad.buttons[0].pressed = true; }")
            await asyncio.sleep(0.3)
            await pg.evaluate("() => { window.__pad.buttons[0].pressed = false; }")
            for _ in range(30):
                if shell()["keyboard"] > before_kb:
                    break
                await asyncio.sleep(0.1)
            check("in Game Mode, A on a text box brings up Steam's on-screen keyboard", shell()["keyboard"] == before_kb + 1, shell())
            if sys.platform.startswith("linux"):
                # Steam running: a program named steam
                (steam_bin / "run").mkdir(exist_ok=True)
                shutil.copy("/bin/sleep", steam_bin / "run" / "steam")
                fake = subprocess.Popen([str(steam_bin / "run" / "steam"), "600"])
                (steam_bin / "running.pid").write_text(str(fake.pid))
                await pg.reload()
                await pg.wait_for_selector("#set-steam-self")
                await pg.click("#set-steam-self")
                await pg.wait_for_selector("#set-steam-waiting")
                waiting = await pg.inner_text("#set-steam-waiting")
                still = b"Game Repo Go" in (steam_dir / "userdata" / "22202" / "config" / "shortcuts.vdf").read_bytes()
                await pg.click("#set-steam-restart")
                await pg.wait_for_selector("#set-steam-sure")
                await shot(pg, "17-pc-linux-steam-restart")
                await pg.click("#set-steam-restart-now")
                log = go_data / "steam-restart.log"
                for _ in range(300):
                    if log.exists() and "written" in log.read_text() and (steam_bin / "started").exists():
                        break
                    await asyncio.sleep(0.1)
                fake.wait(timeout=10)
                gone = b"Game Repo Go" not in (steam_dir / "userdata" / "22202" / "config" / "shortcuts.vdf").read_bytes()
                await pg.wait_for_selector("#set-steam-waiting", state="detached", timeout=10000)
                check("while Steam runs a change waits, and Restart Steam closes it, writes the change and starts it again",
                      "Steam is running" in waiting and still and gone and (steam_bin / "started").exists(),
                      (waiting, still, gone, log.read_text() if log.exists() else "no log"))

            # 13d. the big-screen view (D113), as in Game Mode on the Steam Deck: the pages as
            # tabs, changed with L1 and R1 (Page Up and Down); the Library as shelves, one per
            # system, the arrows going to the nearest game; X adds one to the queue; A opens its
            # page and B comes back to it; Start downloads… and ☰'s Options as panels from the
            # right; a list as a panel to choose from; Y searching; the classic view again
            async def at():
                return await pg.evaluate("() => { const e = document.activeElement; return [e?.closest('[data-shelf]')?.dataset.shelf || '', e?.dataset.key || e?.id || (e?.textContent || '').trim().slice(0, 30)]; }")

            def api(cmd, args=None):
                return pg.evaluate("([cmd, args]) => window.__TAURI__.core.invoke('api', { cmd, args })", [cmd, args or {}])

            # a key first, as a controller's press is (after a click or a touch nothing's focused)
            await pg.keyboard.press("Shift")
            await pg.goto(B + "#library")
            await pg.wait_for_selector("#go-shelves .go-tile[data-key]")
            await settled(pg, "() => document.activeElement?.classList.contains('go-tile')")
            frame = await pg.evaluate("""() => ({ view: document.documentElement.dataset.view, tabs: [...document.querySelectorAll('.go-tab')].map(e => e.textContent.trim()),
                shelves: [...document.querySelectorAll('#go-shelves [data-shelf]')].map(e => e.dataset.shelf), hints: document.querySelector('#go-hints')?.innerText || '',
                focused: document.activeElement?.dataset.key || '' })""")
            await shot(pg, "18-big-library")
            check("in Game Mode the big-screen view: the pages as tabs, a shelf for each system, the first game focused and the buttons' hints",
                  frame["view"] == "big" and [t[:9] for t in frame["tabs"]] == ["Library", "Downloads", "On this d", "Settings"] and frame["shelves"] == ["gb", "nes", "places"]
                  and frame["focused"].startswith("gb/") and "Open" in frame["hints"] and "Options" in frame["hints"] and "Search" in frame["hints"], frame)
            first = await at()
            await pg.keyboard.press("ArrowRight")
            right = await at()
            await pg.keyboard.press("ArrowDown")
            down = await at()
            await pg.keyboard.press("ArrowUp")
            up = await at()
            check("the arrows go to the nearest: Right along the shelf, Down to what's below on the next system's, Up back to the same game",
                  first[0] == "gb" and right[0] == "gb" and right[1] != first[1] and down[0] == "nes" and up[1] == right[1], (first, right, down, up))
            target = await pg.evaluate("() => { const t = document.querySelector('#go-shelves .go-tile[data-x]'); t?.focus(); return t?.dataset.key || ''; }")
            hint_x = await pg.inner_text("#go-hints")
            await pg.keyboard.press("x")
            queued_said = await toast(pg, "added to the queue")
            queued = [i for i in (await api("queue"))["items"] if i["state"] == "queued"]
            check("X adds the focused game to the queue, not started, as its hint says",
                  target and "Add to queue" in hint_x and [i["key"] for i in queued] == [target], (target, hint_x, queued_said, queued))
            await pg.evaluate("(k) => document.querySelector('#go-shelves .go-tile[data-key=\"' + CSS.escape(k) + '\"]').focus()", target)
            await pg.keyboard.press("Enter")
            await pg.wait_for_selector(".go-big-game #go-game-title")
            await settled(pg, "() => document.activeElement?.id === 'go-game-add'")
            game_focus = await pg.evaluate("() => document.activeElement?.id")
            await shot(pg, "19-big-game")
            await pg.keyboard.press("Escape")
            await pg.wait_for_selector("#go-shelves")
            await pg.wait_for_function("(k) => document.activeElement?.dataset.key === k", arg=target, timeout=5000)
            check("A opens a game's page with Download… focused, and B comes back to the same game", game_focus == "go-game-add", game_focus)
            hashes = []
            for k in ("PageDown", "PageUp", "twice", "PageUp"):
                if k == "twice":   # two presses before the page has changed (R1 twice, quickly)
                    await pg.evaluate("""() => { for (let i = 0; i < 2; i++) (document.activeElement || document.body).dispatchEvent(
                        new KeyboardEvent('keydown', { key: 'PageDown', bubbles: true, cancelable: true })); }""")
                else:
                    await pg.keyboard.press(k)
                await settled(pg, "() => document.querySelector('.go-tab[aria-current=page]')?.getAttribute('href') === location.hash", 2000)
                hashes.append(await pg.evaluate("() => location.hash"))
            check("R1 and L1 (Page Down and Up) go to the page after and the one before, pressed twice quickly too",
                  hashes == ["#queue", "#library", "#device", "#queue"], hashes)
            await pg.wait_for_selector("#queue-start")
            await pg.focus("#queue-start")
            await pg.keyboard.press("Enter")
            await pg.wait_for_selector("#start-sum")
            # (its button is focused a render after the summary shows)
            await settled(pg, "() => !!document.activeElement?.closest('#start-dialog')")
            panel = await pg.evaluate("""() => { const r = document.querySelector('#start-dialog').getBoundingClientRect(), bar = document.querySelector('#go-big-bar').getBoundingClientRect(), hints = document.querySelector('#go-hints').getBoundingClientRect();
                return { right: Math.round(innerWidth - r.right), left_half: r.left > innerWidth / 2, top: Math.round(r.top - bar.bottom), bottom: Math.round(hints.top - r.bottom),
                  focused: document.activeElement?.closest('#start-dialog') ? (document.activeElement.textContent || '').trim() : '' }; }""")
            await shot(pg, "20-big-start")
            await pg.keyboard.press("Escape")
            await pg.wait_for_selector("#start-dialog", state="detached")
            await settled(pg, "() => document.activeElement?.id === 'queue-start'")
            back_to = await pg.evaluate("() => document.activeElement?.id")
            check("Start downloads… opens as a panel from the right between the bar and the hints, its button focused; B closes it and goes back to what opened it",
                  panel["right"] == 0 and panel["left_half"] and panel["top"] == 0 and panel["bottom"] == 0 and panel["focused"].startswith("Start 1 download") and back_to == "queue-start", (panel, back_to))
            await pg.click("#queue-queued button:has-text('Remove from queue')")
            await pg.wait_for_selector("#queue-queued", state="detached")
            await pg.keyboard.press("PageDown")
            await pg.wait_for_selector("#device-sort")
            await pg.keyboard.press("ContextMenu")
            await pg.wait_for_selector("#options-dialog")
            options_text = await pg.inner_text("#options-dialog")
            await shot(pg, "21-big-options")
            await pg.click("#options-dialog .go-option:has-text('Largest')")
            await pg.wait_for_selector("#options-dialog", state="detached")
            sorted_by = await pg.input_value("#device-sort")
            await pg.keyboard.press("ContextMenu")
            await pg.click("#options-dialog .go-option:has-text('By system')")
            check("☰ opens Options for the page (its orders, Make room…, the classic view), and an order chosen there is the page's",
                  sorted_by == "size" and "Make room…" in options_text and "Switch to the classic view" in options_text, (sorted_by, options_text))
            await pg.keyboard.press("PageDown")
            await pg.wait_for_selector("#set-theme")
            theme = await pg.input_value("#set-theme")
            await pg.focus("#set-theme")
            await pg.keyboard.press("Enter")
            await pg.wait_for_selector("#choose-dialog")
            chooser = await pg.evaluate("() => ({ n: document.querySelectorAll('#choose-dialog .go-option').length, focused: document.activeElement?.getAttribute('aria-pressed') })")
            await pg.keyboard.press("ArrowDown")
            await pg.keyboard.press("Enter")
            await pg.wait_for_selector("#choose-dialog", state="detached")
            chosen = await pg.input_value("#set-theme")
            check("A on a list opens its choices as a panel, the one chosen focused, and the one picked there is the list's",
                  chooser["n"] >= 3 and chooser["focused"] == "true" and chosen != theme, (chooser, theme, chosen))
            await pg.select_option("#set-theme", theme)
            await pg.goto(B + "#library")
            await pg.wait_for_selector("#go-shelves .go-tile[data-key]")
            await settled(pg, "() => document.activeElement?.classList.contains('go-tile')")
            await pg.keyboard.press("y")
            await pg.wait_for_function("() => document.activeElement?.id === 'go-search'", timeout=10000)
            await pg.keyboard.type("Alpha")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 1", timeout=10000)
            await pg.keyboard.press("ArrowDown")
            found = await pg.evaluate("() => document.activeElement?.closest('.go-card')?.dataset.key")
            await shot(pg, "22-big-search")
            check("Y searches every game, and Down goes to what's found", found == "gb/Alpha (USA)", found)
            await pg.keyboard.press("ContextMenu")
            await pg.click("#options-classic")
            await pg.wait_for_selector(".go-nav")
            classic = await pg.evaluate("() => [document.documentElement.dataset.view, !!document.querySelector('#go-hints')]")
            check("Options › Switch to the classic view brings back the classic pages", classic == ["classic", False], classic)
            # the usual view again for the steps after
            await pg.evaluate("() => { const l = JSON.parse(localStorage.getItem('go-look') || '{}'); delete l.view; localStorage.setItem('go-look', JSON.stringify(l)); }")

            shell(kind="windows", game_mode=False)
            await pg.set_viewport_size(PHONE)

            # 13c. PC games (D110-D112), on Windows and Linux: a system of their own, put in the
            # PC games folder Settings names, each game in its folder with a shortcut (in the
            # Start menu on Windows, in Steam on Linux); removed, a save it made stays
            add_pc_game()
            before = snapshot(lib, fixtures, phone / "Saves", phone / "ES-DE" / "settings")
            await pg.goto(B + "#settings")
            await pg.wait_for_selector("#set-pc-folder")
            usual = await pg.input_value("#set-pc-folder input")
            pc_games = home / "pc-games"
            await pg.fill("#set-pc-folder input", str(pc_games))
            await pg.click("#set-pc-folder button:has-text('Save')")
            await pg.wait_for_selector("#set-pc-usual")
            check("Settings shows the PC games folder, the usual one until another is chosen", usual.endswith("Games"), usual)
            # the library read again once the server has the game
            listed = False
            for _ in range(60):
                listed = await pg.evaluate("""async () => { const b = await import('./client/backend.js'); await b.call('sync');
                    for (let i = 0; i < 20 && (await b.call('status')).sync.running; i++) await new Promise((r) => setTimeout(r, 100));
                    return (await b.call('systems')).some((x) => x.id === 'pcgames'); }""")
                if listed:
                    break
                await asyncio.sleep(0.5)
            await pg.goto(B + "#library")
            await pg.reload()
            await pg.wait_for_selector("#go-place-pick option[value='s:pcgames']", state="attached", timeout=20000)
            await pg.select_option("#go-place-pick", "s:pcgames")
            # Zeta is in every system's list too, so wait for the system's own to be shown
            await settled(pg, "() => document.querySelector('#go-place-name')?.textContent.includes('PC games')"
                              " && [...document.querySelectorAll('#go-games .go-card[data-key]')].map(e => e.dataset.key).join() === 'pcgames/zeta'", 20000)
            shown = await cards(pg)
            check("PC games are a system of their own in the Library", listed and shown == ["pcgames/zeta"], (listed, shown))
            await pg.click('.go-card[data-key="pcgames/zeta"] .go-tick')
            await pg.click("#go-add")
            await pg.wait_for_selector("#add-pc")
            said = await pg.inner_text("#add-dialog")
            await shot(pg, "18-phone-add-pc-game")
            check("Download… says where a PC game goes, and that the library unpacks it first", str(pc_games) in said and "being unpacked on the library first" in said, said)
            fake = None
            if sys.platform.startswith("linux"):
                # Steam running while it downloads (D125): its shortcut waits for Steam to close
                fake = subprocess.Popen([str(steam_bin / "run" / "steam"), "600"])
                (steam_bin / "running.pid").write_text(str(fake.pid))
                (steam_bin / "started").unlink(missing_ok=True)
            await pg.click("#add-dialog button[type=submit]")
            await toast(pg, "Downloading 1 game")
            await pg.click("#go-nav-queue")
            await pg.wait_for_selector('#queue-done li[data-key="pcgames/zeta"]', timeout=120000)
            restart = await pg.query_selector("#queue-restart")
            if fake:
                # …and Downloads offers Restart Steam, which writes it
                shell(kind="linux")
                await pg.reload()
                await pg.wait_for_selector("#queue-steam", timeout=10000)
                note = await pg.inner_text("#queue-steam")
                await pg.click("#queue-steam-restart")
                await pg.wait_for_selector("#queue-steam-now")
                await shot(pg, "18-linux-queue-steam")
                await pg.click("#queue-steam-now")
                for _ in range(300):
                    if (steam_bin / "started").exists():
                        break
                    await asyncio.sleep(0.1)
                fake.wait(timeout=10)
                await pg.wait_for_selector("#queue-steam", state="detached", timeout=10000)
                check("on Linux, a PC game in while Steam runs: Downloads offers Restart Steam, which closes it and starts it again",
                      "Restart Steam" in note and (steam_bin / "started").exists(), note)
                shell(kind="windows")
            folder = pc_games / "Zeta (1999)"
            files = sorted(str(x.relative_to(folder)).replace("\\", "/") for x in folder.rglob("*") if x.is_file()) if folder.is_dir() else []
            if sys.platform == "win32":
                shortcut = (home / "appdata" / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Game Repo Go" / "Zeta.lnk").is_file()
            else:
                shortcut = b"Zeta" in (steam_dir / "userdata" / "22202" / "config" / "shortcuts.vdf").read_bytes()
            # (zipped as its folder, Zeta: that folder isn't made again inside the game's, D124)
            check("a PC game is unpacked into its own folder of the PC games folder, with a shortcut, and no ES-DE restart asked for",
                  files == ["data/level1.dat", "zeta.exe"] and shortcut and restart is None, (files, shortcut, restart is None))
            (folder / "save1.sav").write_bytes(b"a save")
            await pg.click("#go-nav-device")
            await pg.click('#device-list [data-system="pcgames"] .go-sys-open')
            await pg.wait_for_selector('[data-id="pc:Zeta (1999)"]')
            await pg.check('[data-id="pc:Zeta (1999)"] .go-row-tick')
            await pg.click("#device-remove")
            await pg.wait_for_selector("#remove-pc")
            listed = await pg.inner_text("#remove-dialog")
            await pg.click("#remove-dialog button[type=submit]")
            said = await toast(pg, "Removed 1 game")
            if sys.platform == "win32":
                shortcut = (home / "appdata" / "Microsoft" / "Windows" / "Start Menu" / "Programs" / "Game Repo Go" / "Zeta.lnk").exists()
            else:
                shortcut = b"Zeta" in (steam_dir / "userdata" / "22202" / "config" / "shortcuts.vdf").read_bytes()
            left = sorted(str(x.relative_to(folder)).replace("\\", "/") for x in folder.rglob("*") if x.is_file()) if folder.is_dir() else []
            check("removing a PC game takes its files and shortcut, and keeps a save it made, said so",
                  "Files it made itself" in listed and "stay in Zeta (1999)" in said and left == ["save1.sav"] and not shortcut, (listed[-300:], said, left, shortcut))

            # 14. away from home: the library's list and a game's details are kept on the device
            server.terminate()
            server.wait(timeout=30)
            await pg.goto(B + "#library")
            await pg.select_option("#go-place-pick", "")
            await pg.wait_for_function("() => document.querySelectorAll('#go-games .go-card[data-key]').length === 5", timeout=20000)
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
