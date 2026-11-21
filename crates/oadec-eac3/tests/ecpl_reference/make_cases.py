#!/usr/bin/env python3
"""Writes the case files of the enhanced coupling test matrix.

    python make_cases.py [DIR]

The cases are plain text in the format `tools/ecpl_reference.py` documents;
the reference answers them (`.ref32.bin`), and `tests/ecpl_reference.rs` holds
the decoder to the answers. To write them all again, from this directory:

    python make_cases.py .
    for c in *.case; do python ../../../../tools/ecpl_reference.py "$c" -o "${c%.case}.ref32.bin"; done

Everything is a function of the case name, so the files are the same on
every run. This script needs nothing beyond Python; the reference needs numpy.
"""

from __future__ import annotations

import sys
from pathlib import Path

ZEROS = "0" * 22
# ecplbndstrc, the default of table E3.x (sub-bands 9, 11, 13, 15-17, 19-21
# join the band before them)
DEFAULT_STRC = "0000000001010101110111"

TAB = (13, 19, 25, 31, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145, 157, 169,
       181, 193, 205, 217, 229, 241, 253)


def nbands(begin: int, end: int, strc: str) -> int:
    floor = max(9, begin + 1)
    return sum(1 for s in range(begin, end) if not (s >= floor and strc[s] == "1"))


def ch_line(ch: int, amp: list[int], angle: list[int], chaos: list[int], trans: bool) -> str:
    j = lambda v: ",".join(str(x) for x in v)  # noqa: E731
    return f"ch {ch} amp={j(amp)} angle={j(angle)} chaos={j(chaos)} trans={int(trans)}"


def block(nf: int, begin: int, end: int, strc: str, interp: bool,
          code, coupled=lambda ch: True) -> list[str]:  # noqa: ANN001
    """`code(ch, bnd) -> (amp, angle, chaos, trans)` for every band."""
    n = nbands(begin, end, strc)
    out = [f"block nfchans={nf} begin={begin} end={end} bndstrc={strc} interp={int(interp)}"]
    for ch in range(nf):
        if not coupled(ch):
            out.append(f"ch {ch} off")
            continue
        cols = [code(ch, b) for b in range(n)]
        out.append(ch_line(ch, [c[0] for c in cols], [c[1] for c in cols],
                           [c[2] for c in cols], cols[0][3]))
    return out


def head(seed: int) -> list[str]:
    return [
        "ecplref 1",
        "window kbd5",
        "ola-gain 2",
        f"random lcg {seed}",
        f"mant lcg {seed + 1} 1.0",
    ]


def cases() -> dict[str, list[str]]:
    c: dict[str, list[str]] = {}

    def angle_all(interp: bool) -> list[str]:
        lines = head(101 + interp)
        for blk in range(3):
            lines += block(5, 0, 22, ZEROS, interp,
                           lambda ch, b, blk=blk: (0, ((ch - 1) * 22 + b + blk) % 64, 0, False))
        return lines + ["end"]

    c["angle-all-flat"] = angle_all(False)
    c["angle-all-interp"] = angle_all(True)

    wrap = [31, 32, 63, 0, 0, 63, 32, 31, 1, 62, 33, 30]
    lines = head(103)
    for blk in range(3):
        lines += block(3, 4, 16, ZEROS, True,
                       lambda ch, b, blk=blk: (0, wrap[(b + blk) % 12] if ch == 1 else wrap[(11 - b + blk) % 12], 0, False))
    c["angle-wrap-interp"] = lines + ["end"]

    lines = head(104)
    for blk in range(3):
        lines += block(5, 0, 22, ZEROS, False,
                       lambda ch, b, blk=blk: ((ch * 22 + b + blk) % 32, (b * 5 + ch) % 64, 0, False))
    c["amp-all"] = lines + ["end"]

    def chaos_all(seed: int, trans) -> list[str]:  # noqa: ANN001
        lines = head(seed)
        for blk in range(3):
            lines += block(5, 4, 20, ZEROS, blk % 2 == 1,
                           lambda ch, b, blk=blk: ((b + ch) % 9, (b * 7 + ch * 3) % 64,
                                                   (ch - 1 + b + blk) % 8, trans(ch, blk)))
        return lines + ["end"]

    c["chaos-all-steady"] = chaos_all(105, lambda ch, blk: False)
    c["chaos-all-transient"] = chaos_all(106, lambda ch, blk: True)
    c["chaos-transient-switch"] = chaos_all(107, lambda ch, blk: (ch + blk) % 2 == 1)

    lines = head(108)
    for blk in range(3):
        lines += block(3, 0, 22, ZEROS, True,
                       lambda ch, b, blk=blk: ((b + blk) % 6, (b * 13 + ch * 7 + blk) % 64, (b + ch) % 8, False))
    c["bands-max"] = lines + ["end"]

    lines = head(109)
    for blk in range(3):
        lines += block(3, 2, 21, DEFAULT_STRC, True,
                       lambda ch, b, blk=blk: (b % 5, (b * 9 + ch * 11 + blk) % 64, (b + blk) % 8, ch == 2))
    c["bands-merged"] = lines + ["end"]

    for name, interp in (("bands-min-flat", False), ("bands-min-interp", True)):
        lines = head(110 + interp)
        for blk in range(3):
            lines += block(3, 20, 21, ZEROS, interp,
                           lambda ch, b, blk=blk: (ch, (ch * 21 + blk) % 64, (ch + blk) % 8, False))
        c[name] = lines + ["end"]

    lines = head(112)
    for blk in range(3):
        lines += block(3, 0, 4, ZEROS, True,
                       lambda ch, b, blk=blk: (b, (b * 17 + ch * 5 + blk) % 64, (b + ch) % 8, blk == 2))
    c["bands-narrow"] = lines + ["end"]

    # channel 0 not coupled: channel 1 is first and its codes are not zero;
    # in the second block channel 1 drops out and channel 2 is first
    lines = head(113)
    lines += block(4, 6, 14, ZEROS, False, lambda ch, b: (1, 40 + b, 7, False),
                   coupled=lambda ch: ch != 0)
    lines += block(4, 6, 14, ZEROS, True, lambda ch, b: (2, 20 + b + ch, 5, ch == 3),
                   coupled=lambda ch: ch >= 2)
    lines += block(4, 6, 14, ZEROS, False, lambda ch, b: (0, 3 * b, 3, False),
                   coupled=lambda ch: ch != 0)
    c["first-channel-later"] = lines + ["end"]

    # on, off, on with another region, on, off (a concealed frame gives a
    # block without enhanced coupling), on
    lines = head(114)
    lines += block(2, 4, 12, ZEROS, False, lambda ch, b: (b, 9 * b, 4, False))
    lines += ["block none"]
    lines += block(2, 6, 18, DEFAULT_STRC, True, lambda ch, b: (b % 4, 11 * b + 3, 6, False))
    lines += block(2, 6, 18, DEFAULT_STRC, True, lambda ch, b: (b % 4, 5 * b, 2, True))
    lines += ["block none"]
    lines += block(2, 4, 12, ZEROS, False, lambda ch, b: (0, 7 * b, 7, False))
    c["timeline-on-off"] = lines + ["end"]

    # the stream ends: the last block's successor is zero
    lines = head(115)
    for blk in range(3):
        lines += block(2, 4, 16, ZEROS, blk == 2, lambda ch, b, blk=blk: (blk, (6 * b + blk) % 64, b % 8, False))
    c["timeline-eof"] = lines + ["end"]
    return c


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
    for name, lines in cases().items():
        (out / f"{name}.case").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
