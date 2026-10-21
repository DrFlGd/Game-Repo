#!/usr/bin/env python3
"""The update files of a release (docs/PLAN.md, "In-app updates", D134-D136).

Game Repo, Game Repo Server and Game Repo Go each read latest-<program>.json from the
newest release (desktop/update): its version, what's new, and for each system the
installer's address and signature. CI's release job writes them from the installers it
publishes and the signatures the Tauri CLI made beside them as it built them (with the
key tools/updater_key.cmd made), and checks each signature against the public key built
into the programs, here, with no other tool, before anything is published:

    python tools/update_files.py --version 0.5.42 --dist dist --sigs sigs \\
        --repo DrFlGd/Game-Repo --changelog CHANGELOG.md --out dist

The public key is UPDATER_PUBKEY's (or --pubkey's), as tauri.conf.json takes it. "What's
new" is CHANGELOG.md's Unreleased section, under this version's heading, and the releases
before it, so a program a few versions behind shows everything it missed.

    python tools/update_files.py --verify FILE --signature FILE.sig [--pubkey KEY] [--version V]

checks one signature (tests/updater_key.py checks the key Game Repo makes against the
Tauri CLI's signer with it), and

    python tools/update_files.py --build-config FILE >> "$GITHUB_ENV"

writes the Tauri config that has a build sign its installer and carry the public key
(when UPDATER_PUBKEY is set and is a key) and prints UPDATER_CONFIG=--config FILE for
the build steps; with no key, UPDATER_CONFIG= and the builds are as before. With
--android, the config has only the public key (Tauri's Android build makes no updater
files; CI signs the APK itself once apksigner has) and the line is UPDATER_CONFIG_ANDROID.
"""
import argparse
import base64
import datetime
import hashlib
import json
import os
import re
import sys
from pathlib import Path

# each program's installers in a release, by the updater's name for the system
PROGRAMS = {
    "game-repo": {"windows-x86_64": "Game Repo_{v}_x64-setup.exe"},
    "game-repo-server": {"windows-x86_64": "Game Repo Server_{v}_x64-setup.exe"},
    "game-repo-go": {
        "windows-x86_64": "Game Repo Go_{v}_x64-setup.exe",
        "linux-x86_64": "Game Repo Go_{v}_amd64.AppImage",
        "android-aarch64": "Game Repo Go_{v}_arm64.apk",
    },
}
# published only once its own signing key is in the secrets (tools/android_signing_key.cmd):
# left out of the update file until then
OPTIONAL = {"android-aarch64"}
# the releases before this one whose notes go in too
EARLIER = 20

# ---- Ed25519 (RFC 8032), for checking signatures without anything installed ------------

P = 2 ** 255 - 19
L = 2 ** 252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P:
        return None
    return P - x if (x & 1) != sign else x


_GY = 4 * pow(5, P - 2, P) % P
_GX = _x(_GY, 0)
BASE = (_GX, _GY, 1, _GX * _GY % P)


def _add(a, b):
    e1 = (a[1] - a[0]) * (b[1] - b[0]) % P
    e2 = (a[1] + a[0]) * (b[1] + b[0]) % P
    c = 2 * a[3] * b[3] * D % P
    d = 2 * a[2] * b[2] % P
    e, f, g, h = e2 - e1, d - c, d + c, e2 + e1
    return (e * f % P, g * h % P, f * g % P, e * h % P)


def _mul(s, point):
    q = (0, 1, 1, 0)
    while s:
        if s & 1:
            q = _add(q, point)
        point = _add(point, point)
        s >>= 1
    return q


def _same(a, b):
    return (a[0] * b[2] - b[0] * a[2]) % P == 0 and (a[1] * b[2] - b[1] * a[2]) % P == 0


def _point(s):
    y = int.from_bytes(s, "little")
    sign, y = y >> 255, y & ((1 << 255) - 1)
    x = _x(y, sign)
    return None if x is None else (x, y, 1, x * y % P)


def ed25519_verify(public, message, signature):
    if len(public) != 32 or len(signature) != 64:
        return False
    a, r = _point(public), _point(signature[:32])
    s = int.from_bytes(signature[32:], "little")
    if a is None or r is None or s >= L:
        return False
    h = int.from_bytes(hashlib.sha512(signature[:32] + public + message).digest(), "little") % L
    return _same(_mul(s, BASE), _add(r, _mul(h, a)))


# ---- minisign, as the Tauri CLI writes its keys and signatures (each base64 once more) -----

class BadSignature(Exception):
    pass


def _box_lines(b64, what):
    try:
        text = base64.b64decode(b64.strip(), validate=True).decode("utf-8")
    except Exception as e:
        raise BadSignature(f"the {what} isn't what the Tauri CLI writes ({e})")
    return [line for line in text.splitlines() if line.strip()]


def public_key(b64):
    """(key number, Ed25519 key) from a public key as tauri.conf.json takes it."""
    lines = _box_lines(b64, "public key")
    raw = base64.b64decode(lines[1]) if len(lines) >= 2 else b""
    if len(raw) != 42 or raw[:2] != b"Ed":
        raise BadSignature("the public key isn't a minisign Ed25519 key")
    return raw[2:10], raw[10:]


def verify(data, signature_b64, pubkey_b64):
    """Checks a .sig the Tauri CLI wrote for `data` against the public key, as the updater
    does; answers its trusted comment's fields (timestamp, file, version)."""
    keynum, key = public_key(pubkey_b64)
    lines = _box_lines(signature_b64, "signature")
    if len(lines) < 4 or not lines[2].startswith("trusted comment: "):
        raise BadSignature("the signature isn't a minisign signature")
    sig = base64.b64decode(lines[1])
    if len(sig) != 74 or sig[:2] not in (b"ED", b"Ed"):
        raise BadSignature("the signature isn't a minisign Ed25519 signature")
    if sig[2:10] != keynum:
        raise BadSignature("it was signed with another key than the public key's")
    message = hashlib.blake2b(data, digest_size=64).digest() if sig[:2] == b"ED" else data
    if not ed25519_verify(key, message, sig[10:]):
        raise BadSignature("the signature doesn't match the file")
    trusted = lines[2][len("trusted comment: "):]
    if not ed25519_verify(key, sig[10:] + trusted.encode("utf-8"), base64.b64decode(lines[3])):
        raise BadSignature("the signature's trusted comment was changed")
    return dict(f.split(":", 1) for f in trusted.split("\t") if ":" in f)


# ---- the update files ---------------------------------------------------------------------

def asset_name(name):
    """A release asset's name as GitHub keeps it (a space becomes a dot)."""
    return name.replace(" ", ".")


def notes(changelog, version, today):
    """What's new: the Unreleased section as this version, then the releases before it."""
    sections = re.split(r"^## ", changelog, flags=re.M)[1:]
    out = []
    for s in sections:
        title, _, body = s.partition("\n")
        title, body = title.strip(), body.strip()
        if re.match(r"unreleased", title, re.I):
            if body:
                out.append(f"## {version} ({today})\n\n{body}")
        elif re.match(r"\d+\.\d+\.\d+", title) and len(out) <= EARLIER and title.split()[0] != version:
            out.append(f"## {title}\n\n{body}")
    return "\n\n".join(out)


def find_sig(name, places):
    for d in places:
        p = Path(d) / f"{name}.sig"
        if p.is_file():
            return p
    return None


def write(version, dist, sigs, repo, pubkey, changelog, out, today=None):
    today = today or datetime.date.today().isoformat()
    what = notes(changelog, version, today)
    docs = {}
    for program, systems in PROGRAMS.items():
        platforms = {}
        for system, pattern in systems.items():
            name = pattern.format(v=version)
            file = Path(dist) / name
            if not file.is_file():
                if system in OPTIONAL:
                    continue
                raise SystemExit(f"update_files: {name} isn't in {dist}")
            sig = find_sig(name, [sigs, dist])
            if not sig:
                raise SystemExit(f"update_files: {name} has no signature ({name}.sig): was it built with the update key?")
            signature = sig.read_text(encoding="utf-8").strip()
            try:
                signed = verify(file.read_bytes(), signature, pubkey)
            except BadSignature as e:
                raise SystemExit(f"update_files: {name}: {e}")
            if signed.get("version") != version:
                raise SystemExit(f"update_files: {name} was signed as version {signed.get('version')!r}, not {version}")
            if signed.get("file") != name:
                raise SystemExit(f"update_files: {name}'s signature is for {signed.get('file')!r}")
            platforms[system] = {
                "signature": signature,
                "url": f"https://github.com/{repo}/releases/download/v{version}/{asset_name(name)}",
            }
        docs[program] = {"version": version, "notes": what, "pub_date": f"{today}T00:00:00Z", "platforms": platforms}
    # each one checked before any is written
    written = []
    for program, doc in docs.items():
        path = Path(out) / f"latest-{program}.json"
        path.write_text(json.dumps(doc, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        written.append(path)
    return written


def main(argv=None):
    a = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    a.add_argument("--version")
    a.add_argument("--dist", default="dist")
    a.add_argument("--sigs", default="sigs")
    a.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "DrFlGd/Game-Repo"))
    a.add_argument("--changelog", default=str(Path(__file__).resolve().parent.parent / "CHANGELOG.md"))
    a.add_argument("--out")
    a.add_argument("--pubkey", default=os.environ.get("UPDATER_PUBKEY", ""))
    a.add_argument("--verify", help="a file to check against --signature")
    a.add_argument("--signature")
    a.add_argument("--build-config", help="where to write the builds' Tauri config")
    a.add_argument("--android", action="store_true", help="with --build-config: Game Repo Go's Android build's")
    args = a.parse_args(argv)
    if args.build_config:
        var = "UPDATER_CONFIG_ANDROID" if args.android else "UPDATER_CONFIG"
        if not args.pubkey.strip():
            print(f"{var}=")
            print("update_files: no update key in the secrets: the programs are built without updates", file=sys.stderr)
            return
        try:
            public_key(args.pubkey)
        except BadSignature as e:
            raise SystemExit(f"update_files: UPDATER_PUBKEY: {e}")
        config = {"plugins": {"updater": {"pubkey": args.pubkey.strip()}}}
        if not args.android:
            config["bundle"] = {"createUpdaterArtifacts": True}
        Path(args.build_config).write_text(json.dumps(config), encoding="utf-8")
        print(f"{var}=--config {args.build_config}")
        return
    if not args.pubkey.strip():
        raise SystemExit("update_files: no public key (UPDATER_PUBKEY)")
    if args.verify:
        try:
            signed = verify(Path(args.verify).read_bytes(), Path(args.signature).read_text(encoding="utf-8"), args.pubkey)
        except BadSignature as e:
            raise SystemExit(f"update_files: {args.verify}: {e}")
        if args.version and signed.get("version") != args.version:
            raise SystemExit(f"update_files: signed as version {signed.get('version')!r}, not {args.version}")
        print(f"{args.verify}: signed with this key ({signed})")
        return
    if not args.version or not re.fullmatch(r"\d+\.\d+\.\d+", args.version):
        raise SystemExit("update_files: --version 0.0.0 is needed")
    changelog = Path(args.changelog).read_text(encoding="utf-8")
    for p in write(args.version, args.dist, args.sigs, args.repo, args.pubkey, changelog, args.out or args.dist):
        print(f"wrote {p}")


if __name__ == "__main__":
    main()
