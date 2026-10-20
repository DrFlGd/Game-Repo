#!/usr/bin/env python3
"""The Android signing key helper, tools/android_signing_key.cmd, as Jerred runs it on his
PC (docs/PLAN.md, "Phase 9b design", D104), on Windows only.

Each of its two ways (.NET's CertificateRequest, and Windows' New-SelfSignedCertificate) is
run in a scratch folder: the key it makes is read by Java's keytool with its password under
the name game-repo-go, as an RSA 2048 key with a certificate good for 50 years; it signs a
jar that jarsigner then verifies; the SHA-256 the helper prints is the certificate's; and
run again, it keeps the key it made rather than making another.

    python tests/android_key.py --out <folder>
"""
import argparse
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ap = argparse.ArgumentParser()
ap.add_argument("--out", required=True, help="a scratch folder (emptied first)")
a = ap.parse_args()
if os.name != "nt":
    raise SystemExit("the helper is a Windows batch file; this test runs on Windows")

out = Path(a.out).resolve()
shutil.rmtree(out, ignore_errors=True)
out.mkdir(parents=True)


def java_tool(name):
    for var in ("JAVA_HOME_17_X64", "JAVA_HOME_21_X64", "JAVA_HOME"):
        home = os.environ.get(var)
        if home and (Path(home) / "bin" / f"{name}.exe").is_file():
            return str(Path(home) / "bin" / f"{name}.exe")
    found = shutil.which(name)
    if not found:
        raise SystemExit(f"no {name} (a JDK) on this machine")
    return found


keytool, jarsigner = java_tool("keytool"), java_tool("jarsigner")
failures = []


def check(what, ok, detail=""):
    print(("PASS " if ok else "FAIL ") + what + ("" if ok or not detail else f": {detail}"), flush=True)
    if not ok:
        failures.append(what)


def helper(folder, method):
    env = {**os.environ, "GAMEREPO_KEY_DIR": str(folder), "GAMEREPO_KEY_QUIET": "1"}
    env.pop("GAMEREPO_KEY_METHOD", None)
    if method:
        env["GAMEREPO_KEY_METHOD"] = method
    r = subprocess.run(["cmd", "/c", str(ROOT / "tools" / "android_signing_key.cmd")], env=env, capture_output=True, text=True, timeout=120)
    print("  helper: " + "\n  helper: ".join((r.stdout + r.stderr).strip().splitlines()), flush=True)
    return r


for method in ("", "store"):
    way = "Windows' own way" if method else ".NET's way"
    folder = out / (method or "dotnet")
    first = helper(folder, method)
    key, pw_file = folder / "game-repo-go.p12", folder / "password.txt"
    check(f"{way}: the helper makes the key, its password and the README", first.returncode == 0 and key.is_file() and pw_file.is_file() and (folder / "README.txt").is_file(), first.stdout + first.stderr)
    if first.returncode != 0 or not key.is_file():
        continue
    check(f"{way}: it made it that way, with no fallback", "trying Windows' own way" not in first.stdout, first.stdout)
    password = pw_file.read_text(encoding="ascii").strip()
    check(f"{way}: a password of 32 letters and digits", re.fullmatch(r"[A-Za-z0-9]{32}", password) is not None, repr(password))

    listed = subprocess.run([keytool, "-list", "-v", "-keystore", str(key), "-storetype", "PKCS12", "-storepass", password], capture_output=True, text=True)
    text = listed.stdout + listed.stderr
    check(f"{way}: Java reads it with its password", listed.returncode == 0 and "PrivateKeyEntry" in text, text)
    check(f"{way}: Java knows the key as game-repo-go", re.search(r"Alias name: game-repo-go\b", text, re.I) is not None, text)
    check(f"{way}: an RSA 2048 key, SHA-256 signed", "2048-bit RSA" in text and "SHA256withRSA" in text, text)
    until = re.search(r"until: .*?(\d{4})\s*$", text, re.M)
    check(f"{way}: its certificate is good for 50 years", until is not None and int(until.group(1)) >= 2070, until.group(0) if until else text)
    sha = re.search(r"SHA256:\s*([0-9A-F:]+)", text)
    printed = re.search(r"SHA-256: ([0-9A-F:]+)", first.stdout)
    check(f"{way}: the SHA-256 the helper prints is the certificate's", sha and printed and sha.group(1) == printed.group(1), f"{sha and sha.group(1)} vs {printed and printed.group(1)}")

    jar = folder / "signed.jar"
    with zipfile.ZipFile(jar, "w") as z:
        z.writestr("META-INF/MANIFEST.MF", "Manifest-Version: 1.0\r\n\r\n")
        z.writestr("hello.txt", "Game Repo Go")
    signed = subprocess.run([jarsigner, "-keystore", str(key), "-storetype", "PKCS12", "-storepass", password, str(jar), "game-repo-go"], capture_output=True, text=True)
    verified = subprocess.run([jarsigner, "-verify", str(jar)], capture_output=True, text=True)
    check(f"{way}: it signs, and the signature verifies", signed.returncode == 0 and "jar verified" in verified.stdout, signed.stdout + signed.stderr + verified.stdout + verified.stderr)

    before = key.read_bytes()
    again = helper(folder, method)
    check(f"{way}: run again, it keeps the key it made", again.returncode == 0 and "Using the key made before" in again.stdout and key.read_bytes() == before
          and printed and printed.group(0) in again.stdout, again.stdout + again.stderr)

pw_only = out / "lost"
pw_only.mkdir()
shutil.copy(out / "dotnet" / "game-repo-go.p12", pw_only / "game-repo-go.p12")
lost = helper(pw_only, "")
check("a key without its password.txt is refused, not replaced", lost.returncode != 0 and "password.txt" in lost.stdout and (pw_only / "game-repo-go.p12").read_bytes() == (out / "dotnet" / "game-repo-go.p12").read_bytes(), lost.stdout)

print("all passed" if not failures else f"{len(failures)} failed")
sys.exit(1 if failures else 0)
