#!/usr/bin/env python3
"""Game Repo Go's Steam helper as Restart Steam starts it (docs/PLAN.md, D109), against a
stand-in Steam: it closes Steam, writes the shortcuts waiting into shortcuts.vdf and their
Proton into config.vdf, and starts Steam again.

    python3 tests/steam_helper.py --program "Game Repo Go_0.5.33_amd64.AppImage" --out reports
    python3 tests/steam_helper.py --program desktop/target/release/gamerepo-cli --cli --out reports

The stand-in Steam is a folder like Steam's (userdata/<user>/config), a `steam` on PATH that
closes the running one when asked (`steam -shutdown`) and otherwise notes it was started,
and a running program named `steam` (a copy of sleep). Linux only.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("--program", required=True, help="Game Repo Go (its AppImage), or gamerepo-cli with --cli")
ap.add_argument("--cli", action="store_true", help="the program is gamerepo-cli (device steam-apply)")
ap.add_argument("--out", default="steam-helper")
a = ap.parse_args()

program = str(Path(a.program).resolve())
out = Path(a.out).resolve()
out.mkdir(parents=True, exist_ok=True)
work = Path(tempfile.mkdtemp(prefix="gamerepo-test-steam-"))
results = []


def check(what, ok, detail=""):
    results.append((what, bool(ok)))
    print(("PASS " if ok else "FAIL ") + what + ("" if ok else f": {detail}"), flush=True)


steam, data, bin_ = work / "Steam", work / "data", work / "bin"
config = steam / "userdata" / "22202" / "config"
config.mkdir(parents=True)
data.mkdir()
(bin_ / "run").mkdir(parents=True)
(steam / "config").mkdir()
# Steam's own config.vdf, with something of its own the helper must keep
(steam / "config" / "config.vdf").write_text('"InstallConfigStore"\n{\n\t"Software"\n\t{\n\t\t"Valve"\n\t\t{\n\t\t\t"Steam"\n\t\t\t{\n\t\t\t\t"AutoUpdateWindowEnabled"\t\t"0"\n\t\t\t}\n\t\t}\n\t}\n}\n')
fake = bin_ / "steam"
fake.write_text(f"""#!/bin/sh
if [ "$1" = "-shutdown" ]; then kill "$(cat '{work}/running.pid')"; else date > '{work}/started'; fi
""")
fake.chmod(0o755)
shutil.copy("/bin/sleep", bin_ / "run" / "steam")
running = subprocess.Popen([str(bin_ / "run" / "steam"), "600"])
(work / "running.pid").write_text(str(running.pid))

(data / "settings.json").write_text(json.dumps({"steam": str(steam)}))
games = work / "Games" / "Zeta Quest (1999)"
games.mkdir(parents=True)
(work / "cover.png").write_bytes(b"\x89PNG\r\n\x1a\n")
(data / "steam.json").write_text(json.dumps({"wanted": [
    {"key": "go", "name": "Game Repo Go", "program": program, "start_dir": str(Path(program).parent)},
    {"key": "pcgames/zeta", "name": "Zeta Quest", "program": str(games / "zeta.exe"), "start_dir": str(games),
     "proton": True, "art": {"cover": str(work / "cover.png")}},
]}))

env = {k: v for k, v in os.environ.items() if k not in ("SteamGamepadUI", "GAMESCOPE_WAYLAND_DISPLAY", "XDG_CURRENT_DESKTOP")}
env["PATH"] = f"{bin_}{os.pathsep}{env.get('PATH', '')}"
cmd = [program, "device", "steam-apply", "--data", str(data)] if a.cli else [program, "--steam-apply", str(data)]
try:
    started = time.time()
    r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=180)
    took = time.time() - started
    log = (data / "steam-restart.log").read_text() if (data / "steam-restart.log").exists() else ""
    (out / "steam-helper.log").write_text(f"$ {' '.join(cmd)}\nexit {r.returncode} after {took:.1f} s\n{r.stdout}\n{r.stderr}\n--- steam-restart.log\n{log}")
    check("the helper finishes without an error", r.returncode == 0, (r.returncode, r.stdout[-400:], r.stderr[-400:], log))
    running.wait(timeout=5) if running.poll() is None else None
    check("it closes the running Steam first", running.poll() is not None)
    vdf = (config / "shortcuts.vdf").read_bytes() if (config / "shortcuts.vdf").exists() else b""
    check("the waiting shortcuts are written into Steam's shortcuts.vdf, tagged as Game Repo Go's",
          b"Game Repo Go" in vdf and b"Zeta Quest" in vdf and f'"{games / "zeta.exe"}"'.encode() in vdf, vdf[:300])
    cfg = (steam / "config" / "config.vdf").read_text()
    check("the Windows game is given Proton in config.vdf, and Steam's own settings are kept",
          "CompatToolMapping" in cfg and '"proton_experimental"' in cfg and '"AutoUpdateWindowEnabled"' in cfg, cfg[-600:])
    grid = list((config / "grid").glob("*p.png")) if (config / "grid").exists() else []
    check("the game's box front goes into Steam's grid", len(grid) == 1, grid)
    st = json.loads((data / "steam.json").read_text())
    check("what was written is noted, so nothing waits any more", sorted(st.get("written", {}).get("tools", {}).values()) == ["proton_experimental"]
          and len(st.get("written", {}).get("appids", [])) == 2, st.get("written"))
    end = time.time() + 10
    while time.time() < end and not (work / "started").exists():
        time.sleep(0.2)
    check("Steam is started again (outside Game Mode)", (work / "started").exists(), log)
finally:
    if running.poll() is None:
        running.kill()
    shutil.rmtree(work, ignore_errors=True)
    (out / "steam-helper.json").write_text(json.dumps(results, indent=1))

passed = sum(ok for _, ok in results)
print(f"\n{passed}/{len(results)} passed")
sys.exit(0 if results and passed == len(results) else 1)
