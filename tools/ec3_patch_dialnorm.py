#!/usr/bin/env python3
"""Rewrite `dialnorm` in every E-AC-3 syncframe and repair the frame CRC.

Across the nine configuration-0 streams in one library, `dialnorm` is the only
field that separates the two Dolby's object decoder refuses from the seven it
opens: 31 in both refused, 23 to 27 in every accepted one. A correlation across
nine streams is not a cause, and this project has been caught by exactly that
shape before.

`dialnorm` can be asked directly, which the fields in an EMDF payload cannot:
it lives in `bsi()`, at bit 45 of an E-AC-3 syncframe, and the frame's CRC-16 is
recomputable, so a patched stream is well-formed rather than merely edited. That
matters because Dolby's object path discards any *payload* rewritten in place --
see docs/audit/evidence/remediation/payload-rewrite-rejected.json -- and the
question here is whether the same is true of a header field.

**Read the control before believing a result.** A refusal after changing one
field says something about that field only if changing an unrelated one does
not produce the same refusal. Patch `compr` or `bsmod` the same way and check.

    python tools/ec3_patch_dialnorm.py in.ec3 out.ec3 27
    python tools/ec3_patch_dialnorm.py in.ec3 out.ec3 31 --field compr

`oadec verify` should report the patched file as clean; if it does not, the
patch is wrong and any conclusion drawn from it is worthless.
"""

from __future__ import annotations

import argparse
import sys

POLY = 0x8005
TABLE = []
for _i in range(256):
    _v = _i << 8
    for _ in range(8):
        _v = ((_v << 1) ^ POLY) & 0xFFFF if _v & 0x8000 else (_v << 1) & 0xFFFF
    TABLE.append(_v)

# bit offset from the start of an E-AC-3 syncframe, and width
FIELDS = {
    # syncword 16, strmtyp 2, substreamid 3, frmsiz 11, fscod 2,
    # numblkscod/fscod2 2, acmod 3, lfeon 1, bsid 5
    "dialnorm": (45, 5),
    # the byte after dialnorm: compre 1, then compr 8 when it is set. Patching
    # `compre` alone changes the length of bsi, so the control patches the five
    # bits of `bsid` instead, which no decoder may act on beyond version checks.
    "bsid": (40, 5),
}


def crc16(data: bytes) -> int:
    c = 0
    for b in data:
        c = ((c << 8) ^ TABLE[((c >> 8) ^ b) & 0xFF]) & 0xFFFF
    return c


def read_bits(buf: bytes, bit: int, n: int) -> int:
    v = 0
    for i in range(n):
        b = bit + i
        v = (v << 1) | ((buf[b >> 3] >> (7 - (b & 7))) & 1)
    return v


def write_bits(buf: bytearray, bit: int, n: int, value: int) -> None:
    for i in range(n):
        b = bit + i
        mask = 0x80 >> (b & 7)
        if (value >> (n - 1 - i)) & 1:
            buf[b >> 3] |= mask
        else:
            buf[b >> 3] &= ~mask & 0xFF


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("dst")
    ap.add_argument("value", type=int)
    ap.add_argument("--field", default="dialnorm", choices=sorted(FIELDS))
    args = ap.parse_args()
    bit_off, width = FIELDS[args.field]
    if not 0 <= args.value < (1 << width):
        print(f"{args.value} does not fit in {width} bits", file=sys.stderr)
        return 2

    data = bytearray(open(args.src, "rb").read())
    pos, frames, changed, skipped = 0, 0, 0, 0
    was = {}
    while pos + 5 < len(data):
        if data[pos] != 0x0B or data[pos + 1] != 0x77:
            pos += 1
            continue
        # frmsiz is 11 bits after syncword(16) + strmtyp(2) + substreamid(3)
        frmsiz = read_bits(data, pos * 8 + 21, 11)
        size = (frmsiz + 1) * 2
        if pos + size > len(data) or size < 8:
            pos += 1
            continue
        bsid = read_bits(data, pos * 8 + 40, 5)
        frames += 1
        if bsid < 10:  # AC-3 lays bsi out differently; leave those alone
            skipped += 1
            pos += size
            continue
        old = read_bits(data, pos * 8 + bit_off, width)
        was[old] = was.get(old, 0) + 1
        if old != args.value:
            write_bits(data, pos * 8 + bit_off, width, args.value)
            changed += 1
            # crc2 is the last two bytes of the frame and covers bytes [2, size-2).
            # A CRC-16 with init zero and no final xor leaves remainder zero once
            # its own value is appended big-endian, which is what `crc_ok` checks.
            c = crc16(bytes(data[pos + 2:pos + size - 2]))
            data[pos + size - 2] = (c >> 8) & 0xFF
            data[pos + size - 1] = c & 0xFF
        pos += size

    open(args.dst, "wb").write(bytes(data))
    print(f"{frames} syncframes, {changed} rewritten, {skipped} left alone (AC-3 syntax); "
          f"{args.field} was {was}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
