#!/usr/bin/env python3
"""A release's update files, tools/update_files.py (docs/PLAN.md, "In-app updates", D136),
with no tools but Python's own.

A key is made and installers signed here as the Tauri CLI signs them (minisign: Ed25519
over the file's BLAKE2b, the version in the trusted comment). The update files name each
installer's address in the release (a space as GitHub keeps it, a dot) with its signature,
and carry what's new: the Unreleased section as this version and the releases before it.
A file changed after signing, a signature by another key, one for another version, an
installer without a signature and a trusted comment changed are each refused, with nothing
written.

    python tests/update_files.py
"""
import base64
import hashlib
import json
import os
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import update_files as uf  # noqa: E402

failures = []


def check(what, ok):
    print(("ok   " if ok else "FAIL ") + what)
    if not ok:
        failures.append(what)


# ---- signing, as the Tauri CLI does (the test's side only) ------------------------------

def expand(seed):
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], "little")
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


def encode(point):
    zinv = pow(point[2], uf.P - 2, uf.P)
    x, y = point[0] * zinv % uf.P, point[1] * zinv % uf.P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def ed_sign(seed, message):
    a, prefix = expand(seed)
    public = encode(uf._mul(a, uf.BASE))
    r = int.from_bytes(hashlib.sha512(prefix + message).digest(), "little") % uf.L
    big_r = encode(uf._mul(r, uf.BASE))
    h = int.from_bytes(hashlib.sha512(big_r + public + message).digest(), "little") % uf.L
    return big_r + ((r + h * a) % uf.L).to_bytes(32, "little")


def make_key(seed, keynum):
    public = encode(uf._mul(expand(seed)[0], uf.BASE))
    box = f"untrusted comment: minisign public key: {keynum.hex().upper()}\n{base64.b64encode(b'Ed' + keynum + public).decode()}\n"
    return base64.b64encode(box.encode()).decode()


def sign_file(seed, keynum, path, version, name=None):
    data = Path(path).read_bytes()
    sig = ed_sign(seed, hashlib.blake2b(data, digest_size=64).digest())
    trusted = f"timestamp:1760000000\tfile:{name or Path(path).name}\tversion:{version}"
    glob = ed_sign(seed, sig + trusted.encode())
    box = (f"untrusted comment: signature from tauri secret key\n{base64.b64encode(b'ED' + keynum + sig).decode()}\n"
           f"trusted comment: {trusted}\n{base64.b64encode(glob).decode()}\n")
    return base64.b64encode(box.encode()).decode()


# ---- Ed25519 against RFC 8032's first test vector ---------------------------------------
seed = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
public = bytes.fromhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
vector = bytes.fromhex("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b")
check("Ed25519 signs RFC 8032's test vector", ed_sign(seed, b"") == vector and encode(uf._mul(expand(seed)[0], uf.BASE)) == public)
check("Ed25519 verifies it", uf.ed25519_verify(public, b"", vector))
check("and refuses another message", not uf.ed25519_verify(public, b"x", vector))

# ---- a release's update files -------------------------------------------------------------
V = "0.5.42"
work = Path(tempfile.mkdtemp(prefix="gamerepo-update-files-"))
dist, sigs = work / "dist", work / "sigs"
dist.mkdir()
sigs.mkdir()
key_seed, keynum = os.urandom(32), os.urandom(8)
pubkey = make_key(key_seed, keynum)
names = [p.format(v=V) for systems in uf.PROGRAMS.values() for p in systems.values()]
for n in names:
    (dist / n).write_bytes(os.urandom(2048) + n.encode())
    (sigs / f"{n}.sig").write_text(sign_file(key_seed, keynum, dist / n, V))
changelog = """# Changelog

## For agents

Notes.

## Unreleased

- Updates in the app.

## 0.5.41 (2026-10-21)

- Faster imports.

## 0.5.40 (2026-10-20)

- Something before.
"""
written = uf.write(V, dist, sigs, "DrFlGd/Game-Repo", pubkey, changelog, dist, today="2026-10-22")
check("three update files are written", sorted(p.name for p in written) == ["latest-game-repo-go.json", "latest-game-repo-server.json", "latest-game-repo.json"])
go = json.loads((dist / "latest-game-repo-go.json").read_text(encoding="utf-8"))
check("Game Repo Go's has Windows, the Steam Deck and Android", sorted(go["platforms"]) == ["android-aarch64", "linux-x86_64", "windows-x86_64"])
check("Android's is the APK in the release", go["platforms"]["android-aarch64"]["url"].endswith(f"/v{V}/Game.Repo.Go_{V}_arm64.apk"))
check("each installer's address is in this version's release, its spaces as dots",
      go["platforms"]["linux-x86_64"]["url"] == f"https://github.com/DrFlGd/Game-Repo/releases/download/v{V}/Game.Repo.Go_{V}_amd64.AppImage")
check("with the signature the CLI made", go["platforms"]["windows-x86_64"]["signature"] == (sigs / f"Game Repo Go_{V}_x64-setup.exe.sig").read_text())
check("the version and its date", go["version"] == V and go["pub_date"] == "2026-10-22T00:00:00Z")
check("what's new: Unreleased as this version, then the releases before it",
      go["notes"].startswith(f"## {V} (2026-10-22)\n\n- Updates in the app.") and "## 0.5.41 (2026-10-21)" in go["notes"] and "For agents" not in go["notes"])
repo = json.loads((dist / "latest-game-repo.json").read_text(encoding="utf-8"))
check("Game Repo's has only Windows", list(repo["platforms"]) == ["windows-x86_64"])
check("a signature is checked as the updater does", uf.verify((dist / names[0]).read_bytes(), (sigs / f"{names[0]}.sig").read_text(), pubkey)["version"] == V)


def refused(what, change):
    out = work / "out"
    shutil.rmtree(out, ignore_errors=True)
    out.mkdir()
    saved = {p: p.read_bytes() for p in list(dist.iterdir()) + list(sigs.iterdir())}
    try:
        change()
        uf.write(V, dist, sigs, "DrFlGd/Game-Repo", pubkey, changelog, out)
        ok = False
    except SystemExit as e:
        ok = not any(out.iterdir())
        print(f"     ({e})")
    finally:
        for p in list(dist.iterdir()) + list(sigs.iterdir()):
            if p not in saved:
                p.unlink()
        for p, b in saved.items():
            p.write_bytes(b)
    check(what, ok)


server = f"Game Repo Server_{V}_x64-setup.exe"
refused("an installer changed after it was signed is refused", lambda: (dist / server).write_bytes(b"changed"))
other_seed, other_num = os.urandom(32), os.urandom(8)
refused("one signed with another key is refused", lambda: (sigs / f"{server}.sig").write_text(sign_file(other_seed, other_num, dist / server, V)))
refused("one signed for another version is refused", lambda: (sigs / f"{server}.sig").write_text(sign_file(key_seed, keynum, dist / server, "0.5.41")))
refused("one signed as another file is refused", lambda: (sigs / f"{server}.sig").write_text(sign_file(key_seed, keynum, dist / server, V, name="Game Repo_0.5.42_x64-setup.exe")))
refused("an installer without its signature is refused", lambda: (sigs / f"{server}.sig").unlink())
apk = f"Game Repo Go_{V}_arm64.apk"
refused("an APK without its signature is refused", lambda: (sigs / f"{apk}.sig").unlink())


def without_apk():
    (dist / apk).unlink()
    (sigs / f"{apk}.sig").unlink()
    out = work / "no-apk"
    out.mkdir(exist_ok=True)
    uf.write(V, dist, sigs, "DrFlGd/Game-Repo", pubkey, changelog, out, today="2026-10-22")
    return json.loads((out / "latest-game-repo-go.json").read_text(encoding="utf-8"))


saved_apk = ((dist / apk).read_bytes(), (sigs / f"{apk}.sig").read_bytes())
check("with no APK in the release (no Android signing key yet) Game Repo Go's leaves Android out", sorted(without_apk()["platforms"]) == ["linux-x86_64", "windows-x86_64"])
(dist / apk).write_bytes(saved_apk[0])
(sigs / f"{apk}.sig").write_bytes(saved_apk[1])


def change_trusted():
    box = base64.b64decode((sigs / f"{server}.sig").read_text()).decode()
    (sigs / f"{server}.sig").write_text(base64.b64encode(box.replace(f"version:{V}", "version:9.9.9").encode()).decode())


refused("a trusted comment changed after signing is refused", change_trusted)
shutil.rmtree(work, ignore_errors=True)
print(f"{len(failures)} failed" if failures else "all passed")
sys.exit(1 if failures else 0)
