#!/usr/bin/env python3
"""Game Repo Go's APK on an Android emulator against a real Game Repo Server (docs/PLAN.md,
"Phase 9b design", Tests): the app installed with adb, its page driven through the web
view's debugging socket (the Chrome DevTools Protocol), files checked with adb.

    python3 tests/client_android.py --apk app-x86_64-debug.apk \\
        --cli desktop/target/release/gamerepo-cli --home /tmp/go-android --out shots

The emulator is running already (CI starts one, x86_64 with Android 14), with nothing of
Game Repo Go's on it. The library is the fixtures' Game Boy games, one with a box front,
served by `gamerepo-cli serve-devices` on this computer, which the emulator reaches at
10.0.2.2. ES-DE's folder is made on the emulator's storage as ES-DE makes it.

Checks, in the real app: it opens asking to be paired and, until All files access is
allowed, says games can't be added; Allow access opens Android's own screen, and once it's
allowed there ES-DE's folder is found; the code from Game Repo pairs it by address and the
library is read; a box front comes through the picture protocol; the pairing is kept
encrypted with a Keystore key (no token in the app's files); a controller's D-pad moves
between games, X ticks one, A opens it and B comes back; a game added is downloaded into
ES-DE's folders and gamelist; while the library can't be reached a game queued shows the
downloads' notification from its foreground service, its Pause pauses the queue, and once
the library's back the game comes and the notification goes; a game removed goes from the
device; Back on the Library sends the app to the background; no page errors.

    python3 tests/client_android.py --smoke --apk "Game Repo Go_<v>_arm64.apk" --out shots

The release APK (minified, its web view not open to debugging), where the emulator can run
arm64 apps: it starts on the pairing page with the access note, which needs the Kotlin
plugin's commands to have come through minifying, and doesn't crash.
"""
import argparse
import base64
import json
import queue
import re
import shutil
import struct
import subprocess
import sys
import threading
import time
import urllib.request
import zlib
from pathlib import Path

import websocket  # websocket-client

sys.path.insert(0, str(Path(__file__).with_name("fixtures")))
import make_fixtures  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--apk", required=True)
ap.add_argument("--cli")
ap.add_argument("--home", help="a scratch folder for the library and the server")
ap.add_argument("--out", default="shots")
ap.add_argument("--adb", default="adb")
ap.add_argument("--smoke", action="store_true", help="the release APK: it starts, and its plugin answers")
a = ap.parse_args()
if not a.smoke and not (a.cli and a.home):
    ap.error("--cli and --home are needed, except with --smoke")

PKG = "io.github.drflgd.gamerepo.go"
SD = "/storage/emulated/0"
out = Path(a.out).resolve()
out.mkdir(parents=True, exist_ok=True)
results = []


def check(what, ok, detail=""):
    results.append((what, bool(ok)))
    print(("PASS " if ok else "FAIL ") + what + ("" if ok else f": {detail}"), flush=True)


def run(*args):
    r = subprocess.run([cli, *map(str, args)], capture_output=True, text=True, encoding="utf-8", timeout=300)
    if r.returncode != 0:
        raise SystemExit(f"gamerepo-cli {' '.join(map(str, args))} failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r


def adb(*args, ok=True, timeout=120):
    r = subprocess.run([a.adb, *map(str, args)], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=timeout)
    if ok and r.returncode != 0:
        raise SystemExit(f"adb {' '.join(map(str, args))} failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r.stdout


def sh(cmd, ok=True):
    return adb("shell", cmd, ok=ok)


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def shot(name):
    (out / f"{name}.png").write_bytes(subprocess.run([a.adb, "exec-out", "screencap", "-p"], capture_output=True, timeout=60).stdout)


def there(path):
    return sh(f"[ -e '{path}' ] && echo yes || echo no", ok=False).strip() == "yes"


def finish():
    passed = sum(ok for _, ok in results)
    print(f"{passed} of {len(results)} checks passed")
    sys.exit(0 if results and passed == len(results) else 1)


if a.smoke:
    # what's on the screen, read through Android's accessibility tree (the web view's text
    # is in it), as the release build's page can't be reached any other way
    adb("uninstall", PKG, ok=False)
    adb("install", "-r", "-g", Path(a.apk).resolve(), timeout=300)
    sh(f"appops set --uid {PKG} MANAGE_EXTERNAL_STORAGE default", ok=False)
    sh("logcat -c", ok=False)
    sh(f"monkey -p {PKG} -c android.intent.category.LAUNCHER 1")
    screen, end = "", time.time() + 120
    while time.time() < end:
        screen = sh("uiautomator dump /sdcard/ui.xml >/dev/null && cat /sdcard/ui.xml", ok=False)
        if "Pair with your library" in screen and "access to all files" in screen:
            break
        time.sleep(2)
    shot("release-start")
    log = adb("logcat", "-d", ok=False, timeout=120)
    (out / "logcat-release.txt").write_text(log, encoding="utf-8")
    check("the release APK starts on the pairing page", "Pair with your library" in screen, screen[-1500:])
    check("its plugin answers after minifying (the access note needs it)", "access to all files" in screen, screen[-1500:])
    crashed = re.search(rf"FATAL EXCEPTION[^\n]*\n[^\n]*Process: {re.escape(PKG)}", log)
    check("it doesn't crash", sh(f"pidof {PKG}", ok=False).strip() and not crashed, crashed.group(0) if crashed else "not running")
    (out / "client-android-release.json").write_text(json.dumps(results, indent=1), encoding="utf-8")
    finish()

cli = str(Path(a.cli).resolve())
work = Path(a.home).resolve()
if work.exists():
    shutil.rmtree(work)
work.mkdir(parents=True)
lib, fixtures, data = work / "Library", work / "fixtures", work / "server"


# ---- the library (the fixtures' Game Boy games, Alpha with a box front) and the server ----
make_fixtures.main(fixtures)
lh = ("--home", work / "app", "--library", lib)
run("roots", "add", fixtures / "Roms" / "Game Boy", "--platform", "gb", *lh)
run("dat", "import", fixtures / "dats" / "gb.dat", *lh)
run("scan", *lh)
media = lib / "_library" / "games" / "gb" / "Alpha (USA)" / "media"
media.mkdir(parents=True, exist_ok=True)
(media / "cover.png").write_bytes(png(300, 400, (200, 60, 60)))
run("library", *lh)
lib_id = json.loads((lib / "_library" / "library.json").read_text(encoding="utf-8"))["id"]
index = work / "app" / "data" / "index" / f"{lib_id}.sqlite"
server, port = None, 0


def serve(at=0):
    """The server, on `at` (0: any port), told apart from the last one by its output."""
    global server, port
    lines = queue.Queue()
    server = subprocess.Popen([cli, "serve-devices", "--data", str(data), "--port", str(at), "--no-watch", *map(str, lh), "--index-from", str(index)],
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
    threading.Thread(target=lambda: [lines.put(l.rstrip("\n")) for l in server.stdout], daemon=True).start()
    until, said = time.time() + 120, []
    while time.time() < until:
        try:
            line = lines.get(timeout=0.5)
        except queue.Empty:
            if server.poll() is not None:
                break
            continue
        said.append(line)
        if line.startswith("at "):
            port = int(line.split()[1].split(",")[0].rsplit(":", 1)[1])
        if line.startswith("ready"):
            return
    raise SystemExit("the server didn't start:\n" + "\n".join(said))


def stop_server():
    server.terminate()
    try:
        server.wait(timeout=30)
    except subprocess.TimeoutExpired:
        server.kill()


def code():
    return json.loads(run("serve-devices", "pair", "--data", data, "--json").stdout)["code"]


serve()

# ---- the device: ES-DE's settings and its Game Boy folder, as ES-DE makes them -----------
settings = work / "es_settings.xml"
settings.write_text(f'<?xml version="1.0"?>\n<string name="ROMDirectory" value="{SD}/ROMs" />\n', encoding="utf-8")
sh(f"rm -rf {SD}/ES-DE {SD}/ROMs; mkdir -p {SD}/ES-DE/settings {SD}/ROMs/gb")
adb("push", settings, f"{SD}/ES-DE/settings/es_settings.xml")
adb("uninstall", PKG, ok=False)
adb("install", "-r", "-g", Path(a.apk).resolve(), timeout=300)
sh(f"appops set --uid {PKG} MANAGE_EXTERNAL_STORAGE default", ok=False)
sh("logcat -c", ok=False)
sh(f"monkey -p {PKG} -c android.intent.category.LAUNCHER 1")


# ---- the page, through the web view's debugging socket ----------------------------------
class Page:
    def __init__(self):
        self.ws, self.n = None, 0

    def connect(self, timeout=120):
        end = time.time() + timeout
        last = ""
        while time.time() < end:
            pid = sh(f"pidof {PKG}", ok=False).strip().split()
            if pid:
                adb("forward", "tcp:9333", f"localabstract:webview_devtools_remote_{pid[0]}", ok=False)
                try:
                    targets = json.loads(urllib.request.urlopen("http://127.0.0.1:9333/json", timeout=5).read())
                    page = next((t for t in targets if t.get("type") == "page" and "client.html" in t.get("url", "")), None)
                    if page:
                        self.ws = websocket.create_connection(page["webSocketDebuggerUrl"], timeout=60, suppress_origin=True)
                        return
                    last = json.dumps(targets)
                except Exception as e:
                    last = repr(e)
            time.sleep(1)
        raise SystemExit(f"Game Repo Go's page never showed on the debugging socket: {last}")

    def eval(self, js):
        self.n += 1
        self.ws.send(json.dumps({"id": self.n, "method": "Runtime.evaluate", "params": {"expression": js, "awaitPromise": True, "returnByValue": True}}))
        while True:
            m = json.loads(self.ws.recv())
            if m.get("id") == self.n:
                r = m.get("result", {})
                if "exceptionDetails" in r:
                    raise RuntimeError(f"{js[:80]}: {r['exceptionDetails'].get('exception', {}).get('description', r['exceptionDetails'])}")
                return r.get("result", {}).get("value")

    def wait(self, js, timeout=60, what=""):
        end = time.time() + timeout
        while time.time() < end:
            try:
                if self.eval(f"!!({js})"):
                    return True
            except Exception:
                pass
            time.sleep(0.3)
        shot(f"timeout-{len(results):02d}")
        raise TimeoutError(f"timed out waiting for {what or js}; the page says: " + str(self.eval("document.body.innerText.slice(0, 600)")))

    def click(self, sel):
        self.eval(f"document.querySelector({json.dumps(sel)}).click()")

    def fill(self, sel, value):
        self.eval(f"(() => {{ const e = document.querySelector({json.dumps(sel)}); e.value = {json.dumps(value)}; e.dispatchEvent(new Event('input', {{ bubbles: true }})); }})()")

    def api(self, cmd, args=None):
        return self.eval(f"window.__TAURI__.core.invoke('api', {{ cmd: {json.dumps(cmd)}, args: {json.dumps(args or {})} }})")


def resumed():
    """The app in front, by its package."""
    m = re.search(r"(?:mResumedActivity|topResumedActivity)[^\n]*?\s([\w.]+)/", sh("dumpsys activity activities", ok=False))
    return m.group(1) if m else ""


def key(name):
    sh(f"input keyevent {name}")
    time.sleep(0.4)


def notification():
    return sh("dumpsys notification --noredact", ok=False)


pg = Page()
try:
    pg.connect()

    # 1. it opens asking to be paired, and says games wait for All files access
    pg.wait("document.querySelector('#pair-page') && document.querySelector('#go-storage')", 90, "the pairing page and the access note")
    info = pg.eval("window.__TAURI__.core.invoke('app_info')")
    check("Game Repo Go opens asking to be paired, and says games wait for All files access",
          "Pair with your library" in pg.eval("document.querySelector('#pair-page').innerText") and "access to all files" in pg.eval("document.querySelector('#go-storage').innerText")
          and info.get("kind") == "android", info)
    shot("01-pair-access")

    # 2. Allow access opens Android's own screen; allowed there, ES-DE's folder is found
    pg.click("#go-storage-allow")
    end = time.time() + 30
    while time.time() < end and resumed() == PKG:
        time.sleep(0.5)
    asked = resumed()
    shot("02-android-access-screen")
    sh(f"appops set --uid {PKG} MANAGE_EXTERNAL_STORAGE allow")
    key("KEYCODE_BACK")
    pg.wait("!document.querySelector('#go-storage')", 30, "the access note to go")
    st = pg.api("status")
    check("Allow access opens Android's screen for it, and once allowed ES-DE's folder is found",
          asked and asked != PKG and (st.get("device") or {}).get("esde") == f"{SD}/ES-DE", (asked, st.get("device")))

    # 3. paired by address with the code from Game Repo; the library is read
    pg.fill("#pair-address", f"10.0.2.2:{port}")
    pg.fill("#pair-code", code())
    pg.click("#pair-go")
    pg.wait("(document.querySelector('#go-games-total')?.dataset.total | 0) >= 3", 90, "the library's games")
    keys = pg.eval("[...document.querySelectorAll('#go-games .go-card[data-key]')].map(e => e.dataset.key)")
    check("the code from Game Repo pairs it by address, and the library's games are shown", sorted(keys) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"], keys)
    pg.wait("document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] img')?.naturalWidth > 0", 30, "Alpha's box front")
    src = pg.eval("document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] img').src")
    check("a box front comes through the app's picture protocol", src.startswith(info.get("picture_url", "?")), (src, info))
    shot("03-library")

    # 4. the pairing kept encrypted with a key the Android Keystore holds
    prefs = sh(f"run-as {PKG} cat shared_prefs/gamerepo-go-secrets.xml", ok=False)
    files = sh(f"run-as {PKG} find . -name secrets.json", ok=False)
    sealed = re.search(r'name="server">([^<]+)<', prefs)
    readable = sealed and b"token" in base64.b64decode(sealed.group(1) + "==")
    check("the pairing is kept encrypted with a Keystore key, with no token in the app's files", sealed and not readable and "token" not in prefs and not files.strip(), (prefs[:300], files))

    # 5. a controller: the D-pad moves between games, X ticks, A opens, B comes back
    pg.eval("document.querySelector('#go-games .go-card-open[data-index=\"0\"]').focus()")
    key("KEYCODE_DPAD_RIGHT")
    moved = pg.eval("document.activeElement?.dataset.index")
    key("KEYCODE_BUTTON_X")
    ticked = pg.eval("document.querySelectorAll('.go-card.is-ticked').length")
    key("KEYCODE_BUTTON_X")
    key("KEYCODE_BUTTON_A")
    opened = pg.wait("document.querySelector('#go-game-title')", 15, "a game's page")
    key("KEYCODE_BUTTON_B")
    pg.wait("document.querySelector('#go-games')", 15, "the Library again")
    check("a controller's D-pad moves between games, X ticks one, A opens it and B comes back", moved == "1" and ticked == 1 and opened, (moved, ticked))

    # 6. a game added comes into ES-DE's folders and gamelist, with its box front
    pg.click('.go-card[data-key="gb/Alpha (USA)"] .go-tick')
    pg.wait("document.querySelector('#go-add')", 10, "Add to device…")
    pg.click("#go-add")
    pg.wait("document.querySelector('#add-kinds input[name=cover]')", 30, "the media kinds")
    shot("04-add")
    pg.click("#add-dialog button[type=submit]")
    rom = f"{SD}/ROMs/gb/Alpha (USA).gb"
    end = time.time() + 120
    while time.time() < end and not there(rom):
        time.sleep(1)
    gamelist = sh(f"cat {SD}/ES-DE/gamelists/gb/gamelist.xml", ok=False)
    check("a game added is downloaded into ES-DE's folder and gamelist, with its box front",
          there(rom) and "./Alpha (USA).gb" in gamelist and there(f"{SD}/ES-DE/downloaded_media/gb/covers/Alpha (USA).png"),
          sh(f"ls -R {SD}/ROMs {SD}/ES-DE", ok=False)[-800:])

    # 7. the downloads' notification: a game queued while the library's away
    stop_server()
    pg.api("add", {"keys": ["gb/Beta (Europe)"]})
    end = time.time() + 30
    while time.time() < end and "Waiting for the library" not in notification():
        time.sleep(1)
    services = sh(f"dumpsys activity services {PKG}", ok=False)
    check("a game queued while the library's away shows the downloads' notification, from a foreground service",
          "Waiting for the library" in notification() and "DownloadService" in services and "isForeground=true" in services, services[-800:])
    sh("cmd statusbar expand-notifications", ok=False)
    time.sleep(2)
    shot("05-notification")
    dump = sh("uiautomator dump /sdcard/ui.xml >/dev/null && cat /sdcard/ui.xml", ok=False)
    pause = re.search(r'text="Pause"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', dump, re.I)
    title = re.search(r'text="Waiting for the library"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', dump)
    if not pause and title:
        # a quiet notification can be shown folded, without its buttons: pulled down, it opens
        x1, y1, x2, y2 = map(int, title.groups())
        sh(f"input swipe {(x1 + x2) // 2} {y1} {(x1 + x2) // 2} {y1 + 500} 300")
        time.sleep(1.5)
        dump = sh("uiautomator dump /sdcard/ui.xml >/dev/null && cat /sdcard/ui.xml", ok=False)
        pause = re.search(r'text="Pause"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', dump, re.I)
    (out / "notification-shade.xml").write_text(dump, encoding="utf-8")
    if pause:
        x1, y1, x2, y2 = map(int, pause.groups())
        sh(f"input tap {(x1 + x2) // 2} {(y1 + y2) // 2}")
    end = time.time() + 20
    states = []
    while time.time() < end:
        states = [i["state"] for i in pg.api("queue")["items"] if i["key"] == "gb/Beta (Europe)"]
        if states == ["paused"]:
            break
        time.sleep(0.5)
    check("the notification's Pause pauses the queue", pause and states == ["paused"], (bool(pause), states))
    sh("cmd statusbar collapse", ok=False)
    serve(port)
    pg.api("resume")
    beta = f"{SD}/ROMs/gb/Beta (Europe).gb"
    end = time.time() + 120
    while time.time() < end and not there(beta):
        time.sleep(1)
    end = time.time() + 30
    while time.time() < end and "DownloadService" in sh(f"dumpsys activity services {PKG}", ok=False):
        time.sleep(1)
    check("once the library's back the game comes, and the notification goes", there(beta) and "DownloadService" not in sh(f"dumpsys activity services {PKG}", ok=False)
          and "Waiting for the library" not in notification(), sh(f"ls {SD}/ROMs/gb", ok=False))

    # 8. a game removed goes from the device
    rows = pg.api("on_device")["games"]
    alpha = next((r["id"] for r in rows if r.get("key") == "gb/Alpha (USA)"), None)
    pg.api("remove", {"asks": [{"id": alpha}]})
    check("a game removed goes from the device", alpha and not there(rom) and there(beta), rows)

    # 9. Back on the Library sends Game Repo Go to the background
    pg.eval("location.hash = '#library'")
    pg.wait("document.querySelector('#go-games')", 15, "the Library")
    key("KEYCODE_BACK")
    time.sleep(1.5)
    check("Back on the Library sends the app to the background", resumed() not in (PKG, ""), resumed())
    shot("06-left")
    errors = pg.eval("(window.__errors || []).slice(0, 20)")
    check("no page errors", not errors, errors)
except Exception as e:  # a step that timed out is a failure of its own
    check("the test ran to the end", False, repr(e))
finally:
    try:
        (out / "logcat.txt").write_text(adb("logcat", "-d", ok=False, timeout=120), encoding="utf-8")
    except Exception:
        pass
    if server and server.poll() is None:
        stop_server()
    (out / "client-android.json").write_text(json.dumps(results, indent=1), encoding="utf-8")

finish()
