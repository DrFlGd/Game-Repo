#!/usr/bin/env python3
"""Drive the built Game Repo Go app through tauri-driver (WebDriver) against a real Game
Repo Server, and save screenshots (docs/PLAN.md, "Phase 9b design", Tests).

    tauri-driver &                     # needs WebKitWebDriver (Linux) or msedgedriver (Windows)
    python3 tests/client_ui.py --app desktop/target/release/gamerepo-client \\
        --cli desktop/target/release/gamerepo-cli --home /tmp/go-ui --out shots

The library is the fixtures' Game Boy games, one with a box front, served by `gamerepo-cli
serve-devices`. The app looks for ES-DE in the home folder as it does on any computer, so
the test makes ~/ES-DE and ~/ROMs there and takes them away after; it refuses to run where
either is already, so it never touches a real ES-DE.

Checks, in the real app (its window, web view and `picture` protocol): it opens asking to
be paired; the code from Game Repo pairs it and the library is read onto the device; a box
front comes through the picture protocol; the pairing is kept in Windows Credential
Manager (a file elsewhere); a game added is downloaded into ES-DE's folder and gamelist;
Settings shows ES-DE's folder and the version; Unpair forgets the pairing (gone from
Credential Manager too) and asks to be paired again; no page errors. On Linux the pairing's
file is readable by its user alone (D108), and with --gamepad a virtual controller (an Xbox
360 pad made with uinput, as Steam Input's is on the Steam Deck) moves between the games,
opens one with A and comes back with B, through WebKitGTK's Gamepad API (D107).
"""
import argparse
import json
import os
import queue
import shutil
import struct
import subprocess
import sys
import threading
import time
import zlib
from pathlib import Path

from selenium import webdriver
from selenium.webdriver.common.options import ArgOptions

sys.path.insert(0, str(Path(__file__).with_name("fixtures")))
import make_fixtures  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--app", required=True)
ap.add_argument("--cli", required=True)
ap.add_argument("--home", required=True, help="a scratch folder for the library and the server")
ap.add_argument("--out", default="shots")
ap.add_argument("--driver", default="http://127.0.0.1:4444")
ap.add_argument("--gamepad", action="store_true", help="Linux: try a virtual controller (needs /dev/uinput and python-evdev)")
a = ap.parse_args()

cli = str(Path(a.cli).resolve())
work = Path(a.home).resolve()
out = Path(a.out).resolve()
out.mkdir(parents=True, exist_ok=True)
user = Path.home()
esde, roms = user / "ES-DE", user / "ROMs"
if esde.exists() or roms.exists():
    raise SystemExit(f"{esde} or {roms} is already here: this test makes its own and takes them away after, so it only runs where there's neither")
if work.exists():
    shutil.rmtree(work)
work.mkdir(parents=True)
lib, fixtures, data = work / "Library", work / "fixtures", work / "server"
WINDOWS = os.name == "nt"
results = []


def check(what, ok, detail=""):
    results.append((what, bool(ok)))
    print(("PASS " if ok else "FAIL ") + what + ("" if ok else f": {detail}"), flush=True)


def run(*args):
    r = subprocess.run([cli, *map(str, args)], capture_output=True, text=True, encoding="utf-8", timeout=300)
    if r.returncode != 0:
        raise SystemExit(f"gamerepo-cli {' '.join(map(str, args))} failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def credentials():
    """What Windows Credential Manager holds for Game Repo Go."""
    r = subprocess.run(["cmdkey", "/list"], capture_output=True, text=True)
    return [l.strip() for l in r.stdout.splitlines() if "Game Repo Go/" in l]


# ---- the library (the fixtures' Game Boy games, Alpha with a box front) and the server ----
make_fixtures.main(fixtures)
lh = ("--home", work / "app", "--library", lib)
run("roots", "add", fixtures / "Roms" / "Game Boy", "--platform", "gb", *lh)
run("dat", "import", fixtures / "dats" / "gb.dat", *lh)
run("scan", *lh)
media = lib / "_library" / "games" / "gb" / "Alpha (USA)" / "media"
alpha = next(l.split()[0] for l in run("games", "gb", *lh).stdout.splitlines() if "Alpha (USA)" in l and l.split()[0].isdigit())
run("game", alpha, "--edit", "description", "Alpha is a game made for this test.", *lh)
media.mkdir(parents=True, exist_ok=True)
(media / "cover.png").write_bytes(png(300, 400, (200, 60, 60)))
run("library", *lh)
lib_id = json.loads((lib / "_library" / "library.json").read_text(encoding="utf-8"))["id"]

lines = queue.Queue()
server = subprocess.Popen([cli, "serve-devices", "--data", str(data), "--port", "0", "--no-watch", *map(str, lh),
                           "--index-from", str(work / "app" / "data" / "index" / f"{lib_id}.sqlite")],
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
threading.Thread(target=lambda: [lines.put(l.rstrip("\n")) for l in server.stdout], daemon=True).start()


def wait_for(prefix, secs=120):
    until, said = time.time() + secs, []
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

# ---- the device: ES-DE's settings and its Game Boy folder, in the home folder ------------
(esde / "settings").mkdir(parents=True)
(esde / "settings" / "es_settings.xml").write_text('<?xml version="1.0"?>\n<string name="ROMDirectory" value="~/ROMs" />\n', encoding="utf-8")
(roms / "gb").mkdir(parents=True)

# ---- the app ------------------------------------------------------------------------------
opts = ArgOptions()
opts.set_capability("browserName", "wry")
opts.set_capability("tauri:options", {"application": str(Path(a.app).resolve())})
print("starting a session…", flush=True)
d = webdriver.Remote(command_executor=a.driver, options=opts)
d.set_script_timeout(30)
try:
    d.set_window_size(1180, 800)
except Exception as e:  # some drivers can't resize
    print("resize:", e, flush=True)


def shot(name):
    d.save_screenshot(str(out / f"{name}.png"))


def wait(js, timeout=60, what=""):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if d.execute_script(f"return !!({js})"):
                return True
        except Exception:
            pass
        time.sleep(0.25)
    shot(f"timeout-{len(results):02d}")
    raise TimeoutError(f"timed out waiting for {what or js}; the page says: " + str(d.execute_script("return document.body.innerText.slice(0, 600)")))


def fill(sel, value):
    d.execute_script("const e = document.querySelector(arguments[0]); e.value = arguments[1]; e.dispatchEvent(new Event('input', { bubbles: true }))", sel, value)


def click(sel):
    d.execute_script("document.querySelector(arguments[0]).click()", sel)


def code():
    return json.loads(run("serve-devices", "pair", "--data", data, "--json").stdout)["code"]


try:
    # 1. it opens asking to be paired, then the code pairs it and the library is read
    wait("document.querySelector('#pair-page')", 90, "the pairing page")
    check("Game Repo Go opens asking to be paired with a library", "Pair with your library" in d.execute_script("return document.querySelector('#pair-page').innerText"))
    info = d.execute_async_script("const done = arguments[arguments.length - 1]; window.__TAURI__.core.invoke('app_info').then(done, (e) => done({ error: String(e) }))")
    shot("01-pair")
    fill("#pair-address", address)
    fill("#pair-code", code())
    click("#pair-go")
    wait("(document.querySelector('#go-games-total')?.dataset.total | 0) >= 3", 60, "the library's games")
    keys = d.execute_script("return [...document.querySelectorAll('#go-games .go-card[data-key]')].map(e => e.dataset.key)")
    check("the code from Game Repo pairs it, and the library's games are shown", sorted(keys) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"], keys)
    wait("document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] img')?.naturalWidth > 0", 30, "Alpha's box front")
    src = d.execute_script("return document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] img').src")
    check("a box front comes through the app's picture protocol", src.startswith(info.get("picture_url", "?")) and info.get("kind") == ("windows" if WINDOWS else "linux"), (src, info))
    shot("02-library")
    if WINDOWS:
        check("the pairing is kept in Windows Credential Manager", credentials(), subprocess.run(["cmdkey", "/list"], capture_output=True, text=True).stdout[-800:])
    else:
        kept = list(user.glob(".local/share/io.github.drflgd.gamerepo.go/secrets.json"))
        mode = oct(kept[0].stat().st_mode & 0o777) if kept else None
        check("the pairing is kept in a file in the app's folder only its user can read", kept and "server" in json.loads(kept[0].read_text(encoding="utf-8")) and mode == "0o600", (kept, mode))

    if a.gamepad and not WINDOWS:
        # 1b. a controller, as Steam Input gives one to an app on the Steam Deck
        from evdev import AbsInfo, UInput, ecodes as e
        stick = AbsInfo(0, -32768, 32767, 16, 128, 0)
        pad = UInput({e.EV_KEY: [e.BTN_SOUTH, e.BTN_EAST, e.BTN_NORTH, e.BTN_WEST, e.BTN_TL, e.BTN_TR, e.BTN_SELECT, e.BTN_START, e.BTN_MODE, e.BTN_THUMBL, e.BTN_THUMBR],
                      e.EV_ABS: [(e.ABS_X, stick), (e.ABS_Y, stick), (e.ABS_RX, stick), (e.ABS_RY, stick), (e.ABS_Z, AbsInfo(0, 0, 255, 0, 0, 0)), (e.ABS_RZ, AbsInfo(0, 0, 255, 0, 0, 0)),
                                     (e.ABS_HAT0X, AbsInfo(0, -1, 1, 0, 0, 0)), (e.ABS_HAT0Y, AbsInfo(0, -1, 1, 0, 0, 0))]},
                     name="Microsoft X-Box 360 pad", vendor=0x045E, product=0x028E, version=0x110, bustype=e.BUS_USB)
        try:
            time.sleep(2)  # for udev and WebKitGTK to see it
            # the window has the keyboard's focus, as it would on the Deck
            subprocess.run(["xdotool", "search", "--name", "Game Repo Go", "windowfocus", "--sync"], timeout=20, capture_output=True)

            def tap(code, value=1, kind=e.EV_KEY):
                pad.write(kind, code, value)
                pad.syn()
                time.sleep(0.25)
                pad.write(kind, code, 0)
                pad.syn()
                time.sleep(0.5)

            # the first input makes the pad known to the page (gamepadconnected); then focus Alpha
            pad.write(e.EV_ABS, e.ABS_Z, 10)
            pad.syn()
            time.sleep(1)
            seen = d.execute_script("return navigator.getGamepads ? [...navigator.getGamepads()].filter(Boolean).map(p => p.id + ' ' + p.mapping) : 'no Gamepad API'")
            # a card's button takes the focus; the card round it has the game's key
            focused = "return document.activeElement?.closest('.go-card')?.dataset?.key || ''"
            d.execute_script("document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] .go-card-open').focus()")
            first = d.execute_script(focused)
            tap(e.ABS_HAT0X, 1, e.EV_ABS)
            moved = d.execute_script(focused)
            tap(e.BTN_SOUTH)
            opened = d.execute_script("return location.hash")
            tap(e.BTN_EAST)
            # back where it came from: the Library, at the address it started at (no #library)
            back = d.execute_script("return document.querySelector('#go-games') && !document.querySelector('#go-game-title') ? 'the Library' : location.hash")
            shot("02b-controller")
            check("a controller (as Steam Input gives one) moves between the games, opens one with A and comes back with B",
                  first and moved and moved != first and opened.startswith("#game") and back == "the Library", (seen, first, moved, opened, back))
        finally:
            pad.close()

    # 2. a game added is downloaded into ES-DE's folder and gamelist
    click('.go-card[data-key="gb/Alpha (USA)"] .go-tick')
    wait("document.querySelector('#go-add')", 10, "Add to queue…")
    click("#go-add")
    wait("document.querySelector('#add-kinds input[name=cover]')", 30, "the media kinds")
    shot("03-add")
    click("#add-dialog button[type=submit]")
    end = time.time() + 120
    gamelist = esde / "gamelists" / "gb" / "gamelist.xml"
    while time.time() < end and not ((roms / "gb" / "Alpha (USA).gb").exists() and gamelist.exists()):
        time.sleep(0.5)
    covers = esde / "downloaded_media" / "gb" / "covers"
    check("a game added is downloaded into ES-DE's folder and gamelist, with its box front",
          (roms / "gb" / "Alpha (USA).gb").exists() and gamelist.exists() and "./Alpha (USA).gb" in gamelist.read_text(encoding="utf-8")
          and (covers / "Alpha (USA).png").exists(), sorted(str(p.relative_to(user)) for p in (*roms.rglob("*"), *esde.rglob("*"))))
    d.execute_script("location.hash = '#queue'")
    wait("document.querySelector('#queue-restart')", 60, "the queue's end")
    shot("04-queue-done")

    # 3. Settings: ES-DE's folder and the version
    d.execute_script("location.hash = '#settings'")
    wait("document.querySelector('#set-version')", 20, "Settings")
    text = d.execute_script("return document.querySelector('#go-settings').innerText")
    check("Settings shows ES-DE's folder and the app's version", str(esde) in text and d.execute_script("return document.querySelector('#set-version').textContent") == info.get("version"), text[:600])
    shot("05-settings")

    # 4. Unpair: the pairing forgotten, and asked for again
    click("#set-unpair")
    wait("document.querySelector('#unpair-dialog button[type=submit]')", 10, "the unpair dialog")
    click("#unpair-dialog button[type=submit]")
    wait("!document.querySelector('#unpair-dialog')", 30, "the unpairing")
    d.execute_script("location.hash = '#library'")
    wait("document.querySelector('#pair-page')", 20, "the pairing page again")
    if WINDOWS:
        check("Unpair forgets the pairing, in Credential Manager too, and asks to be paired again", not credentials(), credentials())
    else:
        kept = list(user.glob(".local/share/io.github.drflgd.gamerepo.go/secrets.json"))
        check("Unpair forgets the pairing and asks to be paired again", not kept or "server" not in json.loads(kept[0].read_text(encoding="utf-8")), kept)
    check("the game downloaded stays on the device", (roms / "gb" / "Alpha (USA).gb").exists())
    shot("06-unpaired")
    errors = d.execute_script("return (window.__errors || []).slice(0, 20)")
    check("no page errors", not errors, errors)
except Exception as e:  # a step that timed out is a failure of its own
    check("the test ran to the end", False, repr(e))
finally:
    try:
        d.quit()
    except Exception:
        pass
    server.terminate()
    try:
        server.wait(timeout=30)
    except subprocess.TimeoutExpired:
        server.kill()
    shutil.rmtree(esde, ignore_errors=True)
    shutil.rmtree(roms, ignore_errors=True)
    (out / "client-ui.json").write_text(json.dumps(results, indent=1), encoding="utf-8")

passed = sum(ok for _, ok in results)
print(f"{passed} of {len(results)} checks passed")
sys.exit(0 if results and passed == len(results) else 1)
