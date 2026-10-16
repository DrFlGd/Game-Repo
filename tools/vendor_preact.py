#!/usr/bin/env python3
"""Vendor Preact (+ hooks) and htm as browser-ready ES modules, with no build step.

    git clone --depth 1 --branch 10.27.2 https://github.com/preactjs/preact /tmp/preact
    git clone --depth 1 https://github.com/developit/htm /tmp/htm
    python3 tools/vendor_preact.py --preact /tmp/preact --htm /tmp/htm

Preact's source is plain ES modules with extensionless imports; this copies it
unchanged except for adding ".js" to relative imports and pointing the hooks at
the copied core. Result: web/vendor/preact/{src,hooks}, web/vendor/htm, plus a
NOTICE with the exact commits and the MIT licenses.
"""
import argparse
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ap = argparse.ArgumentParser()
ap.add_argument("--preact", type=Path, required=True)
ap.add_argument("--htm", type=Path, required=True)
a = ap.parse_args()

out = ROOT / "web/vendor/preact"
if out.exists():
    shutil.rmtree(out)
spec = re.compile(r"""(\bfrom\s+|\bimport\s+)(['"])([^'"]+)\2""")


def fix(text: str, here: Path, src_root: Path, core: str) -> str:
    def repl(m):
        target = m.group(3)
        if target == "preact":
            return f"{m.group(1)}{m.group(2)}{core}{m.group(2)}"
        if not target.startswith("."):
            return m.group(0)
        path = (here.parent / target).resolve()
        if path.is_dir():
            target = target.rstrip("/") + "/index.js"
        elif not target.endswith(".js"):
            target += ".js"
        return f"{m.group(1)}{m.group(2)}{target}{m.group(2)}"
    return spec.sub(repl, text)


for f in sorted((a.preact / "src").rglob("*.js")):
    if f.name == "cjs.js":
        continue
    rel = f.relative_to(a.preact / "src")
    dst = out / "src" / rel
    dst.parent.mkdir(parents=True, exist_ok=True)
    dst.write_text(fix(f.read_text(), f, a.preact / "src", "../index.js"))
hooks = a.preact / "hooks/src/index.js"
(out / "hooks").mkdir(parents=True)
(out / "hooks/index.js").write_text(fix(hooks.read_text(), hooks, a.preact, "../src/index.js"))
shutil.copy2(a.preact / "LICENSE", out / "LICENSE")

htm_out = ROOT / "web/vendor/htm"
if htm_out.exists():
    shutil.rmtree(htm_out)
htm_out.mkdir(parents=True)
for name in ("index.mjs", "build.mjs", "constants.mjs"):
    shutil.copy2(a.htm / "src" / name, htm_out / name)
shutil.copy2(a.htm / "LICENSE", htm_out / "LICENSE")

rev = lambda p: subprocess.run(["git", "-C", str(p), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
tag = subprocess.run(["git", "-C", str(a.preact), "describe", "--tags"], capture_output=True, text=True).stdout.strip()
(ROOT / "web/vendor/PREACT_NOTICE.txt").write_text(
    f"Preact {tag} (https://github.com/preactjs/preact @ {rev(a.preact)}), MIT, src/ and hooks/src/ as ES modules\n"
    f"htm (https://github.com/developit/htm @ {rev(a.htm)}), Apache-2.0, src/index.mjs, build.mjs, constants.mjs\n"
    "Copied by tools/vendor_preact.py: unchanged except '.js' added to relative imports.\n")
print("vendored:", sum(1 for _ in out.rglob("*.js")), "preact files,", len(list(htm_out.glob("*.mjs"))), "htm files")
