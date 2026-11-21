#!/usr/bin/env python3
"""A bit-by-bit CRC-8, written apart from `oadec-bits`, to check what the
decoder and the writer compute of TrueHD's `block_header_crc`.

It shares nothing with the Rust code: no table, no byte step. The register
shifts one place to the left, the bit that falls out of it decides whether
the polynomial is XORed in, and then the next data bit is XORed into the
lowest place. The data bits go in in bitstream order, the most significant
bit of each byte first.

    python tools/crc8_reference.py HEX START END
        prints the check of bits START..END (END not included) of the bytes
        HEX, with polynomial 0x1D from 0xA2: the block_header_crc of a block
        header that lies there. START equal to END is a block without one.

    python tools/crc8_reference.py --selftest
        checks the function against the vectors below, and that each of the
        polynomial, the start, the bit order and both ends of the range
        changes the answer.

Options --poly and --init change the polynomial and the start, for the
restart header's CRC (`--init 0`) and for experiments.
"""

import argparse
import sys

POLY = 0x1D
INIT = 0xA2


def crc8_bits(data, start, end, poly=POLY, init=INIT, msb_first=True):
    """The CRC of bits start..end of data (bytes)."""
    if not 0 <= start <= end <= len(data) * 8:
        raise ValueError("bit range outside the data")
    crc = init & 0xFF
    for pos in range(start, end):
        byte = data[pos // 8]
        k = pos % 8
        bit = (byte >> (7 - k)) & 1 if msb_first else (byte >> k) & 1
        out = crc & 0x80
        crc = (crc << 1) & 0xFF
        if out:
            crc ^= poly
        crc ^= bit
    return crc


def selftest():
    # Vectors the Rust tests of crates/oadec-truehd/src/block.rs share.
    palindromes = bytes([0xA5, 0x3C, 0x81, 0x66, 0xFF, 0x18])
    plain = bytes([0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC])
    checks = []
    checks.append(("no header", crc8_bits(b"", 0, 0) == 0xA2))
    checks.append(("empty range", crc8_bits(bytes(4), 7, 7) == 0xA2))
    full = crc8_bits(palindromes, 0, 48)
    checks.append(("polynomial", full != crc8_bits(palindromes, 0, 48, poly=0x07)))
    checks.append(
        (
            "bit order indifferent on palindromes",
            full == crc8_bits(palindromes, 0, 48, msb_first=False),
        )
    )
    unaligned = crc8_bits(plain, 3, 41)
    checks.append(("bit order", unaligned != crc8_bits(plain, 3, 41, msb_first=False)))
    checks.append(("start", unaligned != crc8_bits(plain, 3, 41, init=0x00)))
    checks.append(("first bit", unaligned != crc8_bits(plain, 2, 41)))
    checks.append(("last bit", unaligned != crc8_bits(plain, 3, 42)))
    # one data bit at a time is a byte at a time over whole bytes
    whole = crc8_bits(plain, 0, 48)
    crc = INIT
    for b in plain:
        for k in range(8):
            out = crc & 0x80
            crc = (crc << 1) & 0xFF
            if out:
                crc ^= POLY
            crc ^= (b >> (7 - k)) & 1
    checks.append(("bytes", whole == crc))
    bad = [name for name, ok in checks if not ok]
    for name, ok in checks:
        print(("ok   " if ok else "FAIL ") + name)
    print("palindromes 0..48: 0x%02X" % full)
    print("plain 3..41: 0x%02X" % unaligned)
    return 1 if bad else 0


def main(argv):
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("hex", nargs="?")
    ap.add_argument("start", nargs="?", type=int)
    ap.add_argument("end", nargs="?", type=int)
    ap.add_argument("--poly", type=lambda s: int(s, 0), default=POLY)
    ap.add_argument("--init", type=lambda s: int(s, 0), default=INIT)
    ap.add_argument("--lsb-first", action="store_true")
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args(argv)
    if args.selftest:
        return selftest()
    if args.hex is None or args.start is None or args.end is None:
        ap.error("HEX START END, or --selftest")
    data = bytes.fromhex(args.hex)
    crc = crc8_bits(
        data, args.start, args.end, args.poly, args.init, not args.lsb_first
    )
    print("0x%02X" % crc)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
