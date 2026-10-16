#!/usr/bin/env python3
"""A TorrentZip writer in Python, using zlib's own deflate, to check the app's
(desktop/core/src/torrentzip.rs) byte for byte. TorrentZip's rules: members sorted
by lower-case name, the date 1996-12-24 23:32, flags 2 (and 0x800 for UTF-8 names),
deflate at level 9 with a 32 KB window and memory level 8, no extra fields, and the
comment TORRENTZIPPED-<CRC32 of the central directory, upper-case hex>.

    python3 tests/fixtures/torrentzip.py <out.zip>      writes the test's ZIP, prints its SHA1
"""

import hashlib
import struct
import sys
import zlib

from make_fixtures import rom

TIME, DATE = 0xBC00, 0x2198


def torrentzip(members):
    """members: [(name, bytes)]; returns the ZIP's bytes."""
    out, central = bytearray(), bytearray()
    for name, data in sorted(members, key=lambda m: (m[0].lower(), m[0])):
        n = name.encode("utf-8")
        flags = 2 | (0 if name.isascii() else 0x800)
        c = zlib.compressobj(9, zlib.DEFLATED, -15, 8, zlib.Z_DEFAULT_STRATEGY)
        packed = c.compress(data) + c.flush()
        crc = zlib.crc32(data) & 0xFFFFFFFF
        at = len(out)
        out += struct.pack("<IHHHHHIIIHH", 0x04034B50, 20, flags, 8, TIME, DATE, crc, len(packed), len(data), len(n), 0) + n + packed
        central += struct.pack("<IHHHHHHIIIHHHHHII", 0x02014B50, 0, 20, flags, 8, TIME, DATE, crc, len(packed), len(data),
                               len(n), 0, 0, 0, 0, 0, at) + n
    comment = b"TORRENTZIPPED-%08X" % (zlib.crc32(bytes(central)) & 0xFFFFFFFF)
    count = len(members)
    end = struct.pack("<IHHHHIIH", 0x06054B50, 0, 0, count, count, len(central), len(out), len(comment)) + comment
    return bytes(out + central + end)


def main(path):
    z = torrentzip([("b (Track 2).bin", rom(2, 70_000)), ("A (Track 1).bin", rom(1, 100_000)), ("empty.txt", b"")])
    open(path, "wb").write(z)
    print(hashlib.sha1(z).hexdigest())


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    main(sys.argv[1])
