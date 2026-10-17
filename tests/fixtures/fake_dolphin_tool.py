#!/usr/bin/env python3
"""A stand-in for dolphin-tool, for the tests: `convert` and `verify` as the app
calls them. Its "RVZ" is the real header's first fields (the magic and, at byte
36, the disc's size) and then the disc as it is, so it converts back exactly."""
import hashlib
import struct
import sys
import zlib

DATA = 128


def arg(name):
    a = sys.argv
    return a[a.index(name) + 1] if name in a else None


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    if cmd in ("--version", "-v"):
        print("dolphin-tool 2412 (test stand-in)")
        return
    src = arg("-i")
    if cmd == "verify":
        data = open(src, "rb").read()[DATA:]
        print("Problems Found: No")
        print(f"CRC32: {zlib.crc32(data) & 0xFFFFFFFF:08x}")
        print(f"MD5: {hashlib.md5(data).hexdigest()}")
        print(f"SHA1: {hashlib.sha1(data).hexdigest()}")
        return
    if cmd == "convert":
        out, fmt = arg("-o"), arg("-f")
        data = open(src, "rb").read()
        if fmt == "rvz":
            head = bytearray(DATA)
            head[0:4] = b"RVZ\x01"
            head[36:44] = struct.pack(">Q", len(data))
            open(out, "wb").write(bytes(head) + data)
        elif fmt == "iso":
            open(out, "wb").write(data[DATA:])
        else:
            sys.exit(f"unknown format {fmt}")
        return
    sys.exit(f"unknown command {cmd}")


if __name__ == "__main__":
    main()
