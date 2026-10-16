#!/usr/bin/env python3
"""Make a small synthetic ROM set and DATs describing it, for the page tests and to
try the CLI by hand. Nothing here is real game data: every "ROM" is pseudo-random
bytes and every DAT is written by this script.

    python3 tests/fixtures/make_fixtures.py <out dir>

writes

    <out>/Roms/Game Boy/          correct, misnamed, bad-dump, unknown, zipped and duplicate files
    <out>/Roms/NES/               a ROM with an iNES header (the DAT lists it headerless)
    <out>/dats/gb.dat             Logiqx, "Nintendo - Game Boy (Parent-Clone)"
    <out>/dats/nes.zip            a zipped ClrMamePro DAT, "Nintendo - Nintendo Entertainment System"
    <out>/dats/odd.dat            a DAT whose platform isn't recognised ("Nintendo - Satellaview")
    <out>/expected.json           what the Game Boy audit should find

The Rust tests (desktop/core/src/audit.rs) build the same kinds of files in code.
"""

import hashlib
import json
import sys
import zipfile
import zlib
from pathlib import Path
from xml.sax.saxutils import escape, quoteattr


def rom(seed, n):
    """n pseudo-random bytes (xorshift32), the same on every run."""
    x = (seed * 2654435761 + 1) & 0xFFFFFFFF
    out = bytearray(n)
    for i in range(n):
        x ^= (x << 13) & 0xFFFFFFFF
        x ^= x >> 17
        x ^= (x << 5) & 0xFFFFFFFF
        out[i] = x & 0xFF
    return bytes(out)


def hashes(b):
    return {
        "size": len(b),
        "crc": f"{zlib.crc32(b) & 0xFFFFFFFF:08x}",
        "md5": hashlib.md5(b).hexdigest(),
        "sha1": hashlib.sha1(b).hexdigest(),
    }


def rom_xml(name, b, status=None):
    h = hashes(b)
    extra = f" status={quoteattr(status)}" if status else ""
    return f'<rom name={quoteattr(name)} size="{h["size"]}" crc="{h["crc"]}" md5="{h["md5"]}" sha1="{h["sha1"]}"{extra}/>'


def logiqx(name, version, games):
    out = ['<?xml version="1.0"?>', "<datafile>", "\t<header>",
           f"\t\t<name>{escape(name)}</name>", f"\t\t<description>{escape(name)}</description>",
           f"\t\t<version>{escape(version)}</version>", "\t\t<homepage>Game Repo tests</homepage>", "\t</header>"]
    for g, parent, roms in games:
        clone = f" cloneof={quoteattr(parent)}" if parent else ""
        out.append(f"\t<game name={quoteattr(g)}{clone}>\n\t\t<description>{escape(g)}</description>")
        out += [f"\t\t{r}" for r in roms]
        out.append("\t</game>")
    out.append("</datafile>")
    return "\n".join(out) + "\n"


def main(out):
    out = Path(out)
    gb = out / "Roms" / "Game Boy"
    nes = out / "Roms" / "NES"
    for d in (gb / "Copies", nes, out / "dats"):
        d.mkdir(parents=True, exist_ok=True)
    a, b, c, d, e = rom(1, 32768), rom(2, 32768), rom(3, 32768), rom(4, 65536), rom(5, 16384)
    (gb / "Alpha (USA).gb").write_bytes(a)
    (gb / "Copies" / "Alpha copy.gb").write_bytes(a)
    (gb / "beta.gb").write_bytes(b)
    bad = bytearray(c)
    bad[100] ^= 0xFF
    (gb / "Gamma (Japan).gb").write_bytes(bytes(bad))
    (gb / "My Hack.gb").write_bytes(rom(99, 32768))
    with zipfile.ZipFile(gb / "Delta (World).zip", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("Delta (World).gb", d)
    (nes / "Echo (USA).nes").write_bytes(b"NES\x1a\x01\x01" + bytes(10) + e)

    (out / "dats" / "gb.dat").write_text(logiqx("Nintendo - Game Boy (Parent-Clone)", "20261001-000000", [
        ("Alpha (USA)", None, [rom_xml("Alpha (USA).gb", a)]),
        ("Alpha (Europe)", "Alpha (USA)", [rom_xml("Alpha (Europe).gb", rom(11, 32768))]),
        ("Beta (Europe)", None, [rom_xml("Beta (Europe).gb", b)]),
        ("Gamma (Japan)", None, [rom_xml("Gamma (Japan).gb", c)]),
        ("Delta (World)", None, [rom_xml("Delta (World).gb", d)]),
        ("Epsilon (USA)", None, [rom_xml("Epsilon (USA).gb", rom(12, 1024)), rom_xml("Epsilon (USA) (Track 2).gb", b)]),
        ("Zeta (USA)", None, ['<rom name="Zeta (USA).gb" size="1" status="nodump"/>']),
    ]), encoding="utf-8")
    h = hashes(e)
    cmp = (
        'clrmamepro (\n\tname "Nintendo - Nintendo Entertainment System"\n\tversion 2026\n)\n\n'
        f'game (\n\tname "Echo (USA)"\n\tdescription "Echo (USA)"\n\trom ( name "Echo (USA).nes" size {h["size"]} '
        f'crc {h["crc"]} md5 {h["md5"]} sha1 {h["sha1"]} )\n)\n'
    )
    with zipfile.ZipFile(out / "dats" / "nes.zip", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("Nintendo - Nintendo Entertainment System (2026).dat", cmp)
    (out / "dats" / "odd.dat").write_text(logiqx("Nintendo - Satellaview", "1", [
        ("BS Example (Japan)", None, [rom_xml("BS Example (Japan).bs", rom(7, 1024))]),
    ]), encoding="utf-8")
    (out / "expected.json").write_text(json.dumps({
        "games": 6, "have": 3, "partial": 1, "missing": 2, "groups": 5, "groups_have": 3,
        "have_list": ["Alpha (USA)", "Beta (Europe)", "Delta (World)"],
        "missing_list": ["Alpha (Europe)", "Gamma (Japan)"],
        "matched": ["Alpha (USA).gb", "Delta (World).gb"],
        "misnamed": ["Alpha copy.gb", "beta.gb"],
        "baddump": ["Gamma (Japan).gb"],
        "unknown": ["My Hack.gb"],
        "duplicates": ["Alpha (USA).gb", "Alpha copy.gb"],
    }, indent=2) + "\n")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
