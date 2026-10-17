"""Write the made-up disc images the disc tests use (no real game data).

    python3 make_discs.py <dir>          the source files: a two-track CD as cue/bin
                                         and a DVD as an iso, and a CSO of the iso
    python3 make_discs.py --chd <dir>    also their CHDs, made by chdman (which must
                                         be on the PATH); tests/fixtures/discs/ keeps
                                         the CHDs made this way, so tests don't need it

The bytes are a formula of their position, so they are the same each time;
discs/expected.json holds each file's hashes, which the tests compare with what
they read out of the CHDs and the CSO.
"""
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import zlib

SECTOR = 2352
CD_NAME = "Disc Game (USA)"
DVD_NAME = "Dvd Game (Europe)"
# track 1: data; track 2: audio, its first PREGAP frames in the file as INDEX 00
DATA_FRAMES, AUDIO_FRAMES, PREGAP = 45, 31, 10
DVD_SECTORS = 70


def pattern(n, seed):
    out = bytearray(n)
    for i in range(n):
        out[i] = ((i * 7 + seed) ^ (i >> 9) ^ (i >> 13)) & 0xFF
    return bytes(out)


def cd_files():
    t1 = pattern(DATA_FRAMES * SECTOR, 11)
    t2 = pattern(AUDIO_FRAMES * SECTOR, 101)
    cue = (
        f'FILE "{CD_NAME} (Track 1).bin" BINARY\r\n'
        "  TRACK 01 MODE1/2352\r\n"
        "    INDEX 01 00:00:00\r\n"
        f'FILE "{CD_NAME} (Track 2).bin" BINARY\r\n'
        "  TRACK 02 AUDIO\r\n"
        "    INDEX 00 00:00:00\r\n"
        f"    INDEX 01 00:00:{PREGAP:02d}\r\n"
    ).encode()
    return {
        f"{CD_NAME}.cue": cue,
        f"{CD_NAME} (Track 1).bin": t1,
        f"{CD_NAME} (Track 2).bin": t2,
    }


def dvd_files():
    return {f"{DVD_NAME}.iso": pattern(DVD_SECTORS * 2048, 57)}


def cso(data, block=2048):
    """A CSO (v1) of data: zlib raw deflate blocks, stored as they are when that's smaller."""
    blocks = (len(data) + block - 1) // block
    header = b"CISO" + struct.pack("<IQIBB2x", 24, len(data), block, 1, 0)
    index_size = (blocks + 1) * 4
    body = bytearray()
    index = []
    pos = len(header) + index_size
    for b in range(blocks):
        chunk = data[b * block:(b + 1) * block]
        c = zlib.compressobj(9, zlib.DEFLATED, -15)
        z = c.compress(chunk) + c.flush()
        index.append(pos + len(body) if len(z) < len(chunk) else (pos + len(body)) | 0x80000000)
        body += z if len(z) < len(chunk) else chunk
    index.append(pos + len(body))
    return header + b"".join(struct.pack("<I", i) for i in index) + bytes(body)


def hashes(b):
    return {"size": len(b), "crc32": f"{zlib.crc32(b) & 0xFFFFFFFF:08x}", "sha1": hashlib.sha1(b).hexdigest()}


def write(out, chd=False):
    os.makedirs(out, exist_ok=True)
    files = {**cd_files(), **dvd_files()}
    for name, b in files.items():
        with open(os.path.join(out, name), "wb") as f:
            f.write(b)
    iso = files[f"{DVD_NAME}.iso"]
    with open(os.path.join(out, f"{DVD_NAME}.cso"), "wb") as f:
        f.write(cso(iso))
    if chd:
        tool = shutil.which("chdman") or sys.exit("chdman isn't on the PATH")
        subprocess.run([tool, "createcd", "-f", "-i", os.path.join(out, f"{CD_NAME}.cue"), "-o", os.path.join(out, f"{CD_NAME}.chd")], check=True, capture_output=True)
        subprocess.run([tool, "createdvd", "-f", "-i", os.path.join(out, f"{DVD_NAME}.iso"), "-o", os.path.join(out, f"{DVD_NAME}.chd")], check=True, capture_output=True)
    return {name: hashes(b) for name, b in files.items()}


if __name__ == "__main__":
    args = sys.argv[1:]
    chd = "--chd" in args
    args = [a for a in args if a != "--chd"]
    if len(args) != 1:
        sys.exit(__doc__)
    print(json.dumps(write(args[0], chd), indent=2))
