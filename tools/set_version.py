#!/usr/bin/env python3
"""Release version numbers for the desktop app.

The repository keeps <major>.<minor>.0 (desktop/src-tauri/tauri.conf.json and the
Rust workspace, whose version Game Repo Server's desktop/server/tauri.conf.json and Game Repo Go's desktop/client/tauri.conf.json take); the minor number goes up with each roadmap phase. CI gives each
release the next number in that series, from the release tags already on GitHub:
v0.0.0, v0.0.1, v0.0.2, ... and v0.1.0 once the repository says 0.1.0.

    python tools/set_version.py --next            # print the next release number
    python tools/set_version.py 0.0.7             # stamp it into the app before building
"""
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONF = ROOT / "desktop/src-tauri/tauri.conf.json"
CARGO = ROOT / "desktop/Cargo.toml"
LOCK = ROOT / "desktop/Cargo.lock"
PACKAGES = ("game-repo", "gamerepo-core", "gamerepo-wire", "gamerepo-serve", "gamerepo-device", "gamerepo-steam", "gamerepo-cli", "gamerepo-server", "gamerepo-client", "tauri-plugin-gamerepo-device")


def series():
    """'0.0' from the repository's version."""
    major, minor = json.loads(CONF.read_text(encoding="utf-8"))["version"].split(".")[:2]
    return f"{major}.{minor}"


def next_version(remote="origin"):
    base = series()
    out = subprocess.run(["git", "ls-remote", "--tags", remote, f"refs/tags/v{base}.*"],
                         capture_output=True, text=True, check=True, cwd=ROOT).stdout
    taken = [int(m.group(1)) for m in re.finditer(rf"refs/tags/v{re.escape(base)}\.(\d+)$", out, re.M)]
    return f"{base}.{max(taken) + 1 if taken else 0}"


def sub(path, pattern, repl, count=1):
    # newline="" keeps the file's line endings (CRLF in a Windows checkout)
    with open(path, encoding="utf-8", newline="") as f:
        text = f.read()
    new, n = re.subn(pattern, repl, text, count=count, flags=re.M)
    if n != count:
        sys.exit(f"set_version: no version found in {path.relative_to(ROOT)}")
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(new)


def stamp(v):
    sub(CONF, r'^(\s*"version":\s*)"[^"]*"', rf'\g<1>"{v}"')
    sub(CARGO, r'(\[workspace\.package\]\r?\nversion = )"[^"]*"', rf'\g<1>"{v}"')
    if LOCK.exists():
        for name in PACKAGES:
            sub(LOCK, rf'(name = "{name}"\r?\nversion = )"[^"]*"', rf'\g<1>"{v}"')


if __name__ == "__main__":
    if sys.argv[1:] == ["--next"]:
        print(next_version())
    elif len(sys.argv) == 2 and re.fullmatch(r"\d+\.\d+\.\d+", sys.argv[1]):
        stamp(sys.argv[1])
        print(f"version {sys.argv[1]}")
    else:
        sys.exit(__doc__)
