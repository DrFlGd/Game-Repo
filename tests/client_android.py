#!/usr/bin/env python3
"""Game Repo Go's APK on an Android emulator against a real Game Repo Server (docs/PLAN.md,
"Phase 9b design", Tests): the app installed with adb, its page driven through the web
view's debugging socket (the Chrome DevTools Protocol), files checked with adb.

    python3 tests/client_android.py --apk app-x86_64-debug.apk \\
        --cli desktop/target/release/gamerepo-cli --home /tmp/go-android --out shots \\
        [--update "Game Repo Go_9.9.9_x86_64.apk"]

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
device; saves are synced by WorkManager's job in the background (D186), which goes once
it's turned off in Settings; with --update (the same APK signed as 9.9.9 with the key the debug build was made
with, its .sig beside it, D139), the check as it starts finds nothing newer, Settings › Check
for updates finds 9.9.9, an APK changed after signing is refused before Android sees it, the
right one brings Android's installer to the front and backing out of it says the update
wasn't installed; Back on the Library sends the app to the background; no page errors.

    python3 tests/client_android.py --smoke --apk "Game Repo Go_<v>_arm64.apk" --out shots

The release APK (minified, its web view not open to debugging), where the emulator can run
arm64 apps: it starts on the pairing page with the access note, which needs the Kotlin
plugin's commands to have come through minifying, doesn't crash, and sets its saves' job
up through WorkManager without an error.
"""
import argparse
import base64
import http.server
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
ap.add_argument("--update", help="the update to offer: this APK signed as 9.9.9 (its .sig beside it)")
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
    crashed = re.search(rf"FATAL EXCEPTION[^\n]*\n[^\n]*Process: {re.escape(PKG)}[^\n]*(?:\n[^\n]*AndroidRuntime[^\n]*){{0,40}}", log)
    check("it doesn't crash", sh(f"pidof {PKG}", ok=False).strip() and not crashed, crashed.group(0) if crashed else "not running")
    # the saves' job (D186) set up or taken away as it starts, through WorkManager
    jobs = [l for l in log.splitlines() if "GameRepoGoSaves" in l or ("WM-" in l and " E " in l)]
    check("its saves' job is set up through WorkManager after minifying", not any(" E " in l or " W " in l for l in jobs), jobs[-20:])
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
# Alpha's record (its details), which its pictures go with
alpha = next(l.split()[0] for l in run("games", "gb", *lh).stdout.splitlines() if l.split()[:1] and l.split()[0].isdigit() and "Alpha (USA)" in l)
run("game", alpha, "--edit", "description", "Alpha is a game made for this test.", *lh)
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

# ---- the update the test offers (D139), where the debug build looks for it --------------
UPDATE_FILE = "/latest-game-repo-go.json"


class Offered(http.server.BaseHTTPRequestHandler):
    files, hits = {}, {}

    def do_GET(self):
        Offered.hits[self.path] = Offered.hits.get(self.path, 0) + 1
        body = Offered.files.get(self.path)
        if body is None:
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def offer(version="9.9.9", changed=False):
    """The update file announcing `version`, and the APK (`changed`: a byte added after it was signed)."""
    Offered.files["/go.apk"] = Path(a.update).read_bytes() + (b"!" if changed else b"")
    sig = Path(a.update + ".sig").read_text(encoding="ascii").strip()
    Offered.files[UPDATE_FILE] = json.dumps({"version": version, "notes": f"## {version} (2026-10-22)\n\n- What's new in it.", "pub_date": "2026-10-22T00:00:00Z",
                                             "platforms": {"android-x86_64": {"url": "http://10.0.2.2:8797/go.apk", "signature": sig}}}).encode()


if a.update:
    offer("0.0.1")
    threading.Thread(target=http.server.ThreadingHTTPServer(("127.0.0.1", 8797), Offered).serve_forever, daemon=True).start()

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

    def wait(self, js, timeout=60, what="", soft=False):
        """Until `js` is true; with `soft`, False when it never is (a check of its own, and
        the steps after it go on), else a TimeoutError that ends the test."""
        end = time.time() + timeout
        while time.time() < end:
            try:
                if self.eval(f"!!({js})"):
                    return True
            except Exception:
                pass
            time.sleep(0.3)
        shot(f"timeout-{len(results):02d}")
        if soft:
            return False
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


def hung():
    """The emulator's System UI is slow at times, and Android puts its "isn't responding"
    dialog in front, where it takes the keys meant for what's under it."""
    return "Not Responding" in sh("dumpsys window | grep mCurrentFocus", ok=False)


def unhang():
    """That dialog closed with its Wait (Back doesn't close it)."""
    for _ in range(3):
        if not hung():
            return
        sh("uiautomator dump /sdcard/window.xml", ok=False)
        m = re.search(r'(?:aerr_wait"|text="Wait")[^>]*?bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', sh("cat /sdcard/window.xml", ok=False))
        if m:
            x1, y1, x2, y2 = map(int, m.groups())
            sh(f"input tap {(x1 + x2) // 2} {(y1 + y2) // 2}")
        else:
            sh("am broadcast -a android.intent.action.CLOSE_SYSTEM_DIALOGS", ok=False)
        time.sleep(1)


def key(name):
    unhang()
    sh(f"input keyevent {name}")
    time.sleep(0.4)


def back_to_go(timeout=30):
    """Back from one of Android's screens to Game Repo Go."""
    key("KEYCODE_BACK")
    end = time.time() + timeout
    while time.time() < end and resumed() != PKG:
        time.sleep(1)
        if hung():  # it came up and took the Back
            key("KEYCODE_BACK")
    if resumed() != PKG:
        print(f"note: after Back, {resumed() or 'nothing'} is in front ({sh('dumpsys window | grep mCurrentFocus', ok=False).strip()})", flush=True)


def notification():
    return sh("dumpsys notification --noredact", ok=False)


def app_log(lines=60):
    """What Game Repo Go's process wrote to the log lately: its Rust, Tauri's and the web view's."""
    pid = sh(f"pidof {PKG}", ok=False).strip().split()
    log = adb("logcat", "-d", *(["--pid", pid[0]] if pid else []), ok=False, timeout=60).splitlines()
    return "\n".join(log[-lines:])


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
    # (between the two, no activity is in front for a moment)
    asked, end = "", time.time() + 30
    while time.time() < end:
        asked = resumed()
        if asked not in (PKG, ""):
            break
        time.sleep(0.5)
    shot("02-android-access-screen")
    sh(f"appops set --uid {PKG} MANAGE_EXTERNAL_STORAGE allow")
    back_to_go()
    pg.wait("!document.querySelector('#go-storage')", 30, "the access note to go")
    st = pg.api("status")
    check("Allow access opens Android's screen for it, and once allowed ES-DE's folder is found",
          asked and asked != PKG and (st.get("device") or {}).get("esde") == f"{SD}/ES-DE", (asked, st.get("device")))

    # 3. paired by address with the code from Game Repo; the library is read
    pg.fill("#pair-address", f"10.0.2.2:{port}")
    pg.fill("#pair-code", code())
    pg.click("#pair-go")
    # (on Android the big-screen view is the usual one, D113: the Library as shelves)
    pg.wait("document.querySelectorAll('#go-shelves .go-tile[data-key]').length >= 3", 90, "the library's games")
    keys = pg.eval("[...document.querySelectorAll('#go-shelves .go-tile[data-key]')].map(e => e.dataset.key)")
    check("the code from Game Repo pairs it by address, and the library's games are shown, on shelves in the big-screen view",
          sorted(keys) == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"] and pg.eval("document.documentElement.dataset.view") == "big", keys)
    shown = pg.wait("document.querySelector('.go-tile[data-key=\"gb/Alpha (USA)\"] img')?.naturalWidth > 0", 30, "Alpha's box front", soft=True)
    src = pg.eval("document.querySelector('.go-tile[data-key=\"gb/Alpha (USA)\"] img')?.src || ''")
    why = (src, info)
    if not shown:
        # the protocol asked straight, the backend's own picture, and the app's log
        url = info.get("picture_url", "") + "gb/Alpha%20(USA)/cover?size=256"
        asked = pg.eval(f"fetch({json.dumps(url)}).then(async r => r.status + ' ' + r.headers.get('content-type') + ' ' + (await r.arrayBuffer()).byteLength, e => 'failed: ' + e)")
        try:
            own = str(pg.api("picture", {"key": "gb/Alpha (USA)", "kind": "cover"}))[:60]
        except Exception as e:
            own = repr(e)
        card = pg.eval("document.querySelector('.go-tile[data-key=\"gb/Alpha (USA)\"]')?.outerHTML.slice(0, 500)")
        why = f"{url} -> {asked}; the picture command: {own}; the card: {card}\n{app_log()}"
    check("a box front comes through the app's picture protocol", shown and src.startswith(info.get("picture_url", "?")), why)
    shot("03-library")

    # 4. the pairing kept encrypted with a key the Android Keystore holds
    prefs = sh(f"run-as {PKG} cat shared_prefs/gamerepo-go-secrets.xml", ok=False)
    files = sh(f"run-as {PKG} find . -name secrets.json", ok=False)
    sealed = re.search(r'name="server">([^<]+)<', prefs)
    readable = sealed and b"token" in base64.b64decode(sealed.group(1) + "==")
    check("the pairing is kept encrypted with a Keystore key, with no token in the app's files", sealed and not readable and "token" not in prefs and not files.strip(), (prefs[:300], files))

    # 5. a controller, in the big-screen view (D113): the D-pad moves to the nearest game, X
    # adds one to the queue, A opens it and B comes back to it, R1 and L1 change the page,
    # Start opens Options, Y searches; then Options › Switch to the classic view for the rest
    # (the web view having Android's focus, as a handheld's first press gives it)
    key("KEYCODE_SHIFT_LEFT")
    focused = "document.activeElement?.dataset.key || document.activeElement?.id || ''"
    pg.eval("document.querySelector('#go-shelves .go-tile[data-key=\"gb/Alpha (USA)\"]').focus()")
    key("KEYCODE_DPAD_RIGHT")
    moved = pg.eval(focused)
    key("KEYCODE_BUTTON_X")
    end, queued = time.time() + 10, []
    while time.time() < end and not queued:
        queued = [i for i in pg.api("queue")["items"] if i["state"] == "queued"]
        time.sleep(0.3)
    for i in queued:
        pg.api("cancel", {"ids": [i["id"]]})
    key("KEYCODE_BUTTON_A")
    opened = pg.wait("document.querySelector('.go-big-game #go-game-title')", 15, "a game's page", soft=True)
    key("KEYCODE_BUTTON_B")
    back_again = pg.wait("document.activeElement?.dataset.key === 'gb/Beta (Europe)'", 15, "the same game on the Library again", soft=True)
    check("a controller's D-pad moves to the nearest game, X adds it to the queue, A opens it and B comes back to it",
          moved == "gb/Beta (Europe)" and [i["key"] for i in queued] == ["gb/Beta (Europe)"] and opened and back_again,
          (moved, queued, opened, back_again, pg.eval("location.hash"), sh("dumpsys window | grep -E 'mCurrentFocus|mInTouchMode'", ok=False).strip()))
    shot("03b-big-library")
    key("KEYCODE_BUTTON_R1")
    right = pg.wait("location.hash === '#queue'", 10, "Downloads", soft=True)
    key("KEYCODE_BUTTON_L1")
    left = pg.wait("location.hash === '#library'", 10, "the Library", soft=True)
    key("KEYCODE_BUTTON_START")
    menu = pg.wait("document.querySelector('#options-dialog')", 10, "Options", soft=True)
    shot("03c-big-options")
    key("KEYCODE_BUTTON_B")
    shut = pg.wait("!document.querySelector('#options-dialog')", 10, "Options closed", soft=True)
    key("KEYCODE_BUTTON_Y")
    found = pg.wait("document.activeElement?.id === 'go-search'", 10, "the search", soft=True)
    check("R1 and L1 change the page, Start opens Options and B closes it, Y goes to the search", right and left and menu and shut and found,
          (right, left, menu, shut, found, pg.eval("location.hash")))
    pg.eval("document.activeElement?.blur(); location.hash = '#library'")
    pg.wait("document.querySelector('#go-shelves')", 15, "the Library")
    # (Options as Start opens it, should Start not have reached the page above)
    pg.eval("dispatchEvent(new KeyboardEvent('keydown', { key: 'ContextMenu', bubbles: true, cancelable: true }))")
    pg.wait("document.querySelector('#options-classic')", 10, "Options")
    pg.click("#options-classic")
    pg.wait("document.querySelector('#go-games') && document.documentElement.dataset.view === 'classic'", 15, "the classic view")

    # 6. a game added comes into ES-DE's folders and gamelist, with its box front
    # (the classic view reads its games a page at a time, so its cards come after #go-games)
    pg.wait("document.querySelector('.go-card[data-key=\"gb/Alpha (USA)\"] .go-tick')", 15, "Alpha's card")
    pg.click('.go-card[data-key="gb/Alpha (USA)"] .go-tick')
    pg.wait("document.querySelector('#go-add')", 10, "Download…")
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

    # 9. saves synced with Game Repo Go closed (D186): WorkManager's job, set up once it's paired;
    # turned off in Settings, the job goes, and turned on again it runs at once (a periodic
    # job's first run waits for nothing, and WorkManager puts off one forced early) and syncs
    # Beta's RetroArch save to the library through the Rust library
    def saves_job():
        # its line in dumpsys (`JOB #u0a190/7: …` before Android 14, `JOB
        # androidx.work.systemjobscheduler:u0a192/1: …` with its namespace since)
        for line in sh(f"dumpsys jobscheduler {PKG}", ok=False).splitlines():
            m = re.search(r"JOB \S*?/(-?\d+): ", line)
            if m and "SystemJobService" in line and PKG in line:
                return m.group(1)
        return None

    def why_not():
        # what WorkManager and JobScheduler said, for a failure's report
        sh(f"am broadcast -a androidx.work.diagnostics.REQUEST_DIAGNOSTICS -p {PKG}", ok=False)
        time.sleep(3)
        jobs = sh(f"dumpsys jobscheduler {PKG}", ok=False)
        log = adb("logcat", "-d", "-s", "WM-WorkerWrapper", "WM-DiagnosticsWrkr", "WM-SystemJobScheduler", "WM-Schedulers", "WM-WorkManagerImpl", "GameRepoGoSaves", "AndroidRuntime", ok=False, timeout=60)
        (out / "jobscheduler.txt").write_text(jobs, encoding="utf-8")
        (out / "workmanager-log.txt").write_text(log, encoding="utf-8")
        return [l.strip() for l in jobs.splitlines() if "SystemJobService" in l or "Unsatisfied" in l][:8] + log.splitlines()[-25:]
    srm = f"{SD}/RetroArch/saves/Beta (Europe).srm"
    sh(f"mkdir -p {SD}/RetroArch/saves && echo beta-save > '{srm}'")
    # a save's sent once it's been still 30 s (STILL): this one was written yesterday
    sh(f"touch -t 202610300000.00 '{srm}'", ok=False)
    if time.time() - int(sh(f"stat -c %Y '{srm}'", ok=False).strip() or 0) < 40:
        time.sleep(31)
    end = time.time() + 30
    while time.time() < end and not saves_job():
        time.sleep(1)
    was = saves_job()
    pg.api("set_settings", {"background_saves": False})
    end = time.time() + 30
    while time.time() < end and saves_job():
        time.sleep(1)
    check("turned off in Settings, the job goes", was and not saves_job() and pg.api("saves")["background"]["set"] is False, (was, pg.api("saves").get("background")))
    said_before = len(adb("logcat", "-d", "-s", "GameRepoGoSaves", ok=False, timeout=60).splitlines())
    pg.api("set_settings", {"background_saves": True})
    in_lib = lib / "Saves" / "gb" / "Beta (Europe)" / "Save (retroarch)" / "Beta (Europe).srm"
    end, said = time.time() + 90, []
    while time.time() < end and not (in_lib.is_file() and any("synced:" in l for l in said)):
        time.sleep(1)
        said = adb("logcat", "-d", "-s", "GameRepoGoSaves", ok=False, timeout=60).splitlines()[said_before:]
    job = saves_job()
    check("saves are synced in the background by WorkManager's job, through the Rust library",
          job and in_lib.is_file() and in_lib.read_text().strip() == "beta-save" and any("synced: {" in l for l in said),
          (job, in_lib.is_file(), said[-5:], pg.api("saves").get("background"), None if job and in_lib.is_file() and said else why_not()))

    # 10. updates (D139): Settings › Check for updates finds 9.9.9; an APK changed after it
    # was signed is refused before Android sees it; the right one brings Android's installer
    # to the front, and backed out of there, the page says it wasn't installed
    if a.update:
        check("it checked for updates as it started", Offered.hits.get(UPDATE_FILE, 0) >= 1, Offered.hits)
        offer(changed=True)
        pg.eval("location.hash = '#settings'")
        pg.wait("document.querySelector('#go-settings #update-check')", 30, "Settings › About and updates")
        pg.click("#go-settings #update-check")
        pg.wait("document.querySelector('#update-panel[data-state=available] #update-install')", 60, "Game Repo Go 9.9.9 found")
        said = pg.eval("document.querySelector('#update-panel').innerText")
        check("Settings › Check for updates finds Game Repo Go 9.9.9, to download and install", "Game Repo Go 9.9.9 is out" in said and "Download and install" in said, said)
        pg.click("#update-install")
        refused = pg.wait("document.querySelector('#update-error')?.innerText.includes(\"doesn't match its signature\")", 120, soft=True)
        check("an APK changed after it was signed is refused", refused and resumed() == PKG, pg.eval("document.querySelector('#update-panel').innerText"))
        offer()
        # allowed to install apps, so Android's own question is the one shown
        sh(f"appops set {PKG} REQUEST_INSTALL_PACKAGES allow", ok=False)
        pg.click("#update-install")
        end = time.time() + 120
        while time.time() < end and "packageinstaller" not in resumed():
            time.sleep(1)
        asking = pg.wait("document.querySelector('#update-state')?.innerText.includes('Android is asking to install Game Repo Go 9.9.9')", 15, soft=True)
        shot("06-update")
        check("the update downloaded and checked brings Android's installer to the front", "packageinstaller" in resumed() and asking,
              (resumed(), pg.eval("document.querySelector('#update-panel').innerText")))
        back_to_go()
        kept = pg.wait("document.querySelector('#update-error')?.innerText.includes(\"The update wasn't installed.\")", 30, soft=True)
        check("backed out of Android's installer, the page says the update wasn't installed", kept and resumed() == PKG
              and pg.api("update_state")["version"] != "9.9.9", (resumed(), pg.eval("document.querySelector('#update-panel').innerText")))

    # 11. Back on the Library sends Game Repo Go to the background
    pg.eval("location.hash = '#library'")
    pg.wait("document.querySelector('#go-games')", 15, "the Library")
    key("KEYCODE_BACK")
    time.sleep(1.5)
    check("Back on the Library sends the app to the background", resumed() not in (PKG, ""), resumed())
    shot("07-left")
    errors = pg.eval("(window.__errors || []).slice(0, 20)")
    check("no page errors", not errors, errors)
except Exception as e:  # a step that timed out is a failure of its own
    try:
        e = f"{e!r}\n{app_log(80)}"
    except Exception:
        pass
    check("the test ran to the end", False, e)
finally:
    try:
        (out / "logcat.txt").write_text(adb("logcat", "-d", ok=False, timeout=120), encoding="utf-8")
    except Exception:
        pass
    if server and server.poll() is None:
        stop_server()
    (out / "client-android.json").write_text(json.dumps(results, indent=1), encoding="utf-8")

finish()
