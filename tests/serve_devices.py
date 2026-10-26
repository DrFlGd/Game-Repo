#!/usr/bin/env python3
"""Game Repo Server and the stand-in device client as two programs on the network
(docs/PLAN.md, "Phase 9a design"): `gamerepo-cli serve-devices` serving a library made
from tests/fixtures, and `gamerepo-cli device …` standing in for a phone with ES-DE.

    python3 tests/serve_devices.py --cli desktop/target/release/gamerepo-cli --home /tmp/sd \\
        --out ci-out/serve-devices [--server desktop/target/release/gamerepo-server]

With --server, the server is that program instead of `gamerepo-cli serve-devices`: Game
Repo Server's own (desktop/server, with its tray icon), given the library and Game Repo's
index as Game Repo gives them.

Checks: the server starts with a copy of Game Repo's index and announces itself (the
device finds it by DNS-SD); a pairing code from the control channel pairs the device;
the device lists the systems and games, adds two games into ES-DE's folders (each file
as the library has it), the server hears what it holds; one game is removed with its
files and entry; Game Repo Go's backend (`device app`) pairs a second device, reads the
library, adds a game through its queue, finds a game from elsewhere and removes one; a
device removed from the server is refused; the library and the
folders with games are as they were, byte for byte; the server stops when asked and
says so. Writes <out>/serve-devices.json with what each step answered.
"""
import argparse
import hashlib
import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).with_name("fixtures")))
import make_fixtures  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--cli", required=True)
ap.add_argument("--home", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--no-find", action="store_true", help="don't look for the server by DNS-SD (a network without multicast)")
ap.add_argument("--server", help="Game Repo Server's own program to run instead of `gamerepo-cli serve-devices`")
a = ap.parse_args()

cli = str(Path(a.cli).resolve())
home = Path(a.home).resolve()
out = Path(a.out)
out.mkdir(parents=True, exist_ok=True)
if home.exists():
    import shutil
    shutil.rmtree(home)
home.mkdir(parents=True)
lib = home / "Library"
fixtures = home / "fixtures"
data = home / "server"
phone = home / "phone"
answers = {}
failed = []


def check(what, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + what + (f": {detail}" if detail and not ok else ""), flush=True)
    if not ok:
        failed.append(what)


def run(*args, ok=True):
    r = subprocess.run([cli, *map(str, args)], capture_output=True, text=True, encoding="utf-8", timeout=300)
    if ok and r.returncode:
        raise SystemExit(f"gamerepo-cli {' '.join(map(str, args))} failed ({r.returncode}):\n{r.stdout}\n{r.stderr}")
    return r


def run_json(*args):
    v = json.loads(run(*args, "--json").stdout)
    answers[" ".join(map(str, args[:2]))] = v
    return v


def snapshot(*dirs):
    """Every file under the folders, with its bytes' SHA-1 and its modified time, but those
    Game Repo Server writes in the library itself (D181): Saves/ and the devices' records."""
    files = {}
    for d in dirs:
        for p in sorted(Path(d).rglob("*")):
            r = p.relative_to(d).parts
            if Path(d) == lib and (r[:1] == ("Saves",) or r[:2] == ("_library", "devices")):
                continue
            if p.is_file():
                st = p.stat()
                files[str(p.relative_to(home))] = (hashlib.sha1(p.read_bytes()).hexdigest(), st.st_mtime_ns, st.st_size)
    return files


def sha1(p):
    return hashlib.sha1(Path(p).read_bytes()).hexdigest()


# A library: the fixtures' Game Boy and NES folders with their DATs, in Game Repo's index
make_fixtures.main(fixtures)
lh = ("--home", home / "app", "--library", lib)
run("roots", "add", fixtures / "Roms" / "Game Boy", "--platform", "gb", *lh)
run("roots", "add", fixtures / "Roms" / "NES", "--platform", "nes", *lh)
run("dat", "import", fixtures / "dats" / "gb.dat", fixtures / "dats" / "nes.zip", *lh)
run("scan", *lh)
before = snapshot(lib, fixtures / "Roms")

# Device: ES-DE's folder with its settings and a game of its own in a gamelist
(phone / "ES-DE" / "settings").mkdir(parents=True)
(phone / "ES-DE" / "settings" / "es_settings.xml").write_text('<?xml version="1.0"?>\n<string name="ROMDirectory" value="~/ROMs" />\n', encoding="utf-8")
(phone / "ES-DE" / "gamelists" / "gb").mkdir(parents=True)
(phone / "ES-DE" / "gamelists" / "gb" / "gamelist.xml").write_text(
    '<?xml version="1.0"?>\n<gameList>\n\t<game>\n\t\t<path>./Mine.gb</path>\n\t\t<name>Mine</name>\n\t</game>\n</gameList>\n', encoding="utf-8")

# The server, as Game Repo starts it: its own data folder, the index copied from Game Repo's
lines = queue.Queue()
if a.server:
    lib_id = json.loads((lib / "_library" / "library.json").read_text(encoding="utf-8"))["id"]
    command = [str(Path(a.server).resolve()), "--library", str(lib), "--data", str(data), "--port", "0", "--index-from", str(home / "app" / "data" / "index" / f"{lib_id}.sqlite")]
else:
    command = [cli, "serve-devices", "--data", str(data), "--port", "0", "--no-watch", "--home", str(home / "app"), "--library", str(lib)]
server = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8")
threading.Thread(target=lambda: [lines.put(l.rstrip("\n")) for l in server.stdout], daemon=True).start()
said = []


def wait_for(prefix, secs=120):
    until = time.time() + secs
    while time.time() < until:
        try:
            line = lines.get(timeout=0.5)
        except queue.Empty:
            if server.poll() is not None:
                break
            continue
        said.append(line)
        print("  server: " + line, flush=True)
        if line.startswith(prefix):
            return line
    raise SystemExit(f"the server didn't say '{prefix}'; it said:\n" + "\n".join(said))


try:
    at = wait_for("at ")
    port = int(at.split()[1].split(",")[0].rsplit(":", 1)[1])
    wait_for("ready")
    check("the server's first index is a copy of Game Repo's", (data / "index").is_dir() and any((data / "index").iterdir()))
    status = run_json("serve-devices", "status", "--data", data)
    check("the server is ready and announced", status["ready"] and status["announced"], json.dumps(status))
    address = f"127.0.0.1:{port}"

    if not a.no_find:
        found = run_json("device", "find", "--wait", "8")
        check("the device finds the server on the network (DNS-SD)", any(f["server"] == status["server"] and f["address"].endswith(f":{port}") for f in found), json.dumps(found))

    # pairing: a code from Game Repo (the control channel), entered on the device
    code = run_json("serve-devices", "pair", "--data", data)["code"]
    wrong = run("device", "pair", address, "000000" if code != "000000" else "000001", "--to", phone, ok=False)
    check("a wrong code is refused", wrong.returncode != 0 and "isn't the code" in wrong.stderr, wrong.stderr)
    run("device", "pair", address, code, "--to", phone, "--name", "Stand-in phone")
    devices = run_json("serve-devices", "devices", "--data", data)
    check("the server lists the device by its name", [d["name"] for d in devices] == ["Stand-in phone"], json.dumps(devices))

    # browsing
    systems = run_json("device", "systems", "--to", phone)
    check("the systems with games", [(s["id"], s["games"]) for s in systems] == [("gb", 3), ("nes", 1)], json.dumps(systems))
    page = run_json("device", "games", "--to", phone, "--system", "gb")
    keys = [g["key"] for g in page["items"]]
    check("a system's games", keys == ["gb/Alpha (USA)", "gb/Beta (Europe)", "gb/Delta (World)"], json.dumps(keys))
    found = run_json("device", "games", "--to", phone, "--search", "echo")
    check("a search", [g["key"] for g in found["items"]] == ["nes/Echo (USA)"], json.dumps(found))

    # adding two games: each file as the library has it, an entry each in the gamelist
    added = run_json("device", "add", "gb/Beta (Europe)", "nes/Echo (USA)", "--to", phone, "--media", "cover", "--collection", "Stand-in")
    check("two games added", added["games"] == 2 and added["files"] >= 2, json.dumps(added))
    holdings = run_json("device", "holdings", "--to", phone)
    roms = phone / "ROMs"
    beta = roms / "gb" / "Beta (Europe).gb"
    check("the Game Boy game is in ES-DE's folder, named by its DAT", beta.is_file() and sha1(beta) == sha1(fixtures / "Roms" / "Game Boy" / "beta.gb"), str(sorted(map(str, roms.rglob("*")))))
    echo = [p for p in (roms / "nes").glob("Echo (USA).*")]
    check("the NES game is in ES-DE's folder", len(echo) == 1 and echo[0].stat().st_size > 0, str(echo))
    gl = (phone / "ES-DE" / "gamelists" / "gb" / "gamelist.xml").read_text(encoding="utf-8")
    check("the gamelist keeps the device's own entry and has the game's", "<name>Mine</name>" in gl and "./Beta (Europe).gb" in gl, gl)
    check("the record lists both games", sorted(g["key"] for g in holdings["games"]) == ["gb/Beta (Europe)", "nes/Echo (USA)"], json.dumps(holdings))
    for g in holdings["games"]:
        for f in g["files"]:
            p = phone / {"roms": "ROMs", "esde": "ES-DE", "media": "ES-DE/downloaded_media"}[f["place"]] / f["path"]
            check(f"{f['path']} is on the device as the record says", p.is_file() and p.stat().st_size == f["size"] and sha1(p) == f["sha1"], str(p))
    devices = run_json("serve-devices", "devices", "--data", data)
    check("the server hears what the device holds", devices[0]["holding"]["games"] == 2, json.dumps(devices))
    again = run_json("device", "add", "gb/Beta (Europe)", "--to", phone)
    check("a game added again fetches nothing", again["files"] == 0, json.dumps(again))

    # removing one: its file and entry go, the other game stays
    run("device", "remove", "gb/Beta (Europe)", "--to", phone)
    gl = (phone / "ES-DE" / "gamelists" / "gb" / "gamelist.xml").read_text(encoding="utf-8")
    check("a game removed with its file and entry", not beta.exists() and "Beta" not in gl and "<name>Mine</name>" in gl, gl)
    check("the other game stays", len(list((roms / "nes").glob("Echo (USA).*"))) == 1)

    # Game Repo Go's backend, as its pages call it, on a second device
    go = home / "go-phone"
    go_data = home / "go-data"
    (go / "ES-DE" / "settings").mkdir(parents=True)
    (go / "ES-DE" / "settings" / "es_settings.xml").write_text('<?xml version="1.0"?>\n<string name="ROMDirectory" value="" />\n', encoding="utf-8")
    for f in ("gb", "nes"):
        (go / "ROMs" / f).mkdir(parents=True)
    (go / "ROMs" / "gb" / "My Alpha.gb").write_bytes((fixtures / "Roms" / "Game Boy" / "Alpha (USA).gb").read_bytes())

    def app(cmd, args=None, wait=False):
        r = run("device", "app", cmd, json.dumps(args or {}), "--to", go, "--data", go_data, *(["--wait"] if wait else []))
        v = json.loads(r.stdout)
        answers["app " + cmd] = v
        return v

    code = run_json("serve-devices", "pair", "--data", data)["code"]
    st = app("pair", {"address": address, "code": code}, wait=True)["status"]
    check("Game Repo Go reads the library onto the device", st["paired"] and st["catalogue"]["games"] == 4 and st["sync"]["error"] == "", json.dumps(st))
    found = app("games", {"q": "echo"})
    check("Game Repo Go searches its own copy", [g["key"] for g in found["items"]] == ["nes/Echo (USA)"], json.dumps(found))
    done = app("add", {"keys": ["gb/Delta (World)"]}, wait=True)
    delta = list((go / "ROMs" / "gb").glob("Delta (World).*"))
    check("Game Repo Go's queue adds a game", done["queue"]["items"][0]["state"] == "done" and len(delta) == 1 and not (go / "ROMs" / ".gamerepo-coming").exists(), json.dumps(done["queue"]))
    app("on_device", wait=True)
    rows = {r["id"]: r for r in app("on_device")["games"]}
    delta_id = next((i for i, r in rows.items() if r.get("key") == "gb/Delta (World)"), None)
    check("On this device: the game added and one from elsewhere the library knows",
          delta_id and rows[delta_id]["from"] == "library" and rows.get("gb/My Alpha.gb", {}).get("from") == "known" and rows["gb/My Alpha.gb"]["key"] == "gb/Alpha (USA)", json.dumps(rows))
    if delta_id:
        app("remove", {"asks": [{"id": delta_id}]})
    check("Game Repo Go removes a game", delta and not delta[0].exists() and not (go / "ROMs" / ".gamerepo-removing").exists() and (go / "ROMs" / "gb" / "My Alpha.gb").is_file())
    if not a.no_find:
        # the library somewhere else (its computer given another address): found again on
        # the network by its id and answering with the certificate pinned, the pairing kept
        kept = json.loads((go_data / "secrets.json").read_text(encoding="utf-8"))
        paired = json.loads(kept["server"])
        kept["server"] = json.dumps({**paired, "address": "127.0.0.1:1"})
        (go_data / "secrets.json").write_text(json.dumps(kept), encoding="utf-8")
        st = app("sync", wait=True)["status"]
        check("Game Repo Go finds the library again at another address, still paired",
              st["paired"] and st["sync"]["error"] == "" and st["library"]["address"] not in ("127.0.0.1:1", "") and st["library"]["address"].endswith(f":{port}")
              and json.loads(json.loads((go_data / "secrets.json").read_text(encoding="utf-8"))["server"])["token"] == paired["token"], json.dumps(st))

    # a device removed from the server is refused, and told to pair again
    run("serve-devices", "remove", devices[0]["id"], "--data", data)
    refused = run("device", "systems", "--to", phone, ok=False)
    check("a removed device is refused", refused.returncode != 0 and "pair it again" in refused.stderr, refused.stderr)

    # the library as it was
    after = snapshot(lib, fixtures / "Roms")
    check("the library and its folders with games are as they were", after == before,
          sorted(k for k in set(after) | set(before) if after.get(k) != before.get(k)))
finally:
    run("serve-devices", "stop", "--data", data, ok=False)
    try:
        server.wait(timeout=30)
    except subprocess.TimeoutExpired:
        server.kill()
        failed.append("the server stops when asked")
    time.sleep(0.5)
    while not lines.empty():
        said.append(lines.get())
    (out / "serve-devices.json").write_text(json.dumps({"server said": said, "answers": answers, "failed": failed}, indent=2, default=str), encoding="utf-8")

check("the server stopped and said so", server.returncode == 0 and "stopped" in said and not (data / "server.json").exists(), "\n".join(said))
print(f"{'FAILED: ' + ', '.join(failed) if failed else 'all passed'}")
sys.exit(1 if failed else 0)
