#!/usr/bin/env python3
"""The update key helper, tools/updater_key.cmd, as Jerred runs it on his PC (docs/PLAN.md,
"In-app updates", D135), on Windows only, with the Game Repo just built.

It makes the key with `game-repo.exe --make-update-key` in a scratch folder: a private key,
a public key and a password of 32 letters and digits, with a README; the Tauri CLI signs an
installer with that private key and password (as CI's builds do), and the signature
checks against the public key with the version in it (tools/update_files.py, as the release
job and the updater check it); run again, it keeps the key it made rather than making
another; and a key without its password is refused, not replaced.

    python tests/updater_key.py --app desktop/target/release/game-repo.exe --out <folder>
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import update_files  # noqa: E402

ap = argparse.ArgumentParser()
ap.add_argument("--app", required=True, help="game-repo.exe")
ap.add_argument("--out", required=True, help="a scratch folder (emptied first)")
a = ap.parse_args()
if os.name != "nt":
    raise SystemExit("the helper is a Windows batch file; this test runs on Windows")

out = Path(a.out).resolve()
shutil.rmtree(out, ignore_errors=True)
out.mkdir(parents=True)
failures = []


def check(what, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f": {detail}"), flush=True)
    if not ok:
        failures.append(what)


def helper(folder):
    env = {**os.environ, "GAMEREPO_KEY_DIR": str(folder), "GAMEREPO_KEY_QUIET": "1", "GAMEREPO_UPDATE_KEY_PROGRAM": str(Path(a.app).resolve())}
    r = subprocess.run(["cmd", "/c", str(ROOT / "tools" / "updater_key.cmd")], env=env, capture_output=True, text=True, timeout=180)
    print("  helper: " + "\n  helper: ".join((r.stdout + r.stderr).strip().splitlines()), flush=True)
    return r


folder = out / "Game Repo update key"
first = helper(folder)
private, public, pw_file = folder / "private.key", folder / "public.key", folder / "password.txt"
check("the helper makes the key, its password and the README", first.returncode == 0 and private.is_file() and public.is_file() and pw_file.is_file() and (folder / "README.txt").is_file(), first.stdout + first.stderr)
if first.returncode == 0 and private.is_file():
    password = pw_file.read_text(encoding="ascii").strip()
    check("a password of 32 letters and digits", re.fullmatch(r"[A-Za-z0-9]{32}", password) is not None, repr(password))
    keynum, _ = update_files.public_key(public.read_text(encoding="ascii"))
    check("it says the key's ID", f"Its key ID: {keynum[::-1].hex().upper()}" in first.stdout, first.stdout)

    # an installer signed as CI's builds sign it: the Tauri CLI with the key and its password
    installer = out / "Game Repo_9.9.9_x64-setup.exe"
    installer.write_bytes(os.urandom(1 << 20))
    signed = subprocess.run(["npx", "--yes", "@tauri-apps/cli@2.12.3", "signer", "sign", "--private-key", private.read_text(encoding="ascii").strip(),
                             "--password", password, "--app-version", "9.9.9", str(installer)], capture_output=True, text=True, timeout=600, shell=True)
    sig = Path(str(installer) + ".sig")
    check("the Tauri CLI signs with it", signed.returncode == 0 and sig.is_file(), signed.stdout + signed.stderr)
    if sig.is_file():
        try:
            fields = update_files.verify(installer.read_bytes(), sig.read_text(encoding="ascii"), public.read_text(encoding="ascii"))
            check("the signature checks against the public key, for its version", fields.get("version") == "9.9.9" and fields.get("file") == installer.name, str(fields))
        except update_files.BadSignature as e:
            check("the signature checks against the public key, for its version", False, str(e))
        installer.write_bytes(installer.read_bytes() + b"!")
        try:
            update_files.verify(installer.read_bytes(), sig.read_text(encoding="ascii"), public.read_text(encoding="ascii"))
            check("an installer changed after signing is refused", False)
        except update_files.BadSignature:
            check("an installer changed after signing is refused", True)

    before = (private.read_bytes(), public.read_bytes())
    again = helper(folder)
    check("run again, it keeps the key it made", again.returncode == 0 and "Using the key made before" in again.stdout and (private.read_bytes(), public.read_bytes()) == before, again.stdout + again.stderr)

    lost = out / "lost"
    lost.mkdir()
    shutil.copy(private, lost / "private.key")
    shutil.copy(public, lost / "public.key")
    refused = helper(lost)
    check("a key without its password.txt is refused, not replaced", refused.returncode != 0 and "password.txt" in refused.stdout and (lost / "private.key").read_bytes() == before[0], refused.stdout)

print("all passed" if not failures else f"{len(failures)} failed")
sys.exit(1 if failures else 0)
