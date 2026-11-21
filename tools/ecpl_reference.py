#!/usr/bin/env python3
"""Independent reference for enhanced coupling (ATSC A/52:2018 clause E.3.5.5).

Written from the text of the standard, not from oadec's Rust code. The clause
numbers are A/52:2018's; ETSI TS 102 366 V1.2.1 clause E.2.5.5 prints the same
process word for word (docs/eac3.md). It takes a case file, runs clause
E.3.5.5.1 to E.3.5.5.4 over every block of the case, and writes the
transform coefficients of every coupled channel.

    python tools/ecpl_reference.py CASE.case -o CASE.ref.f32 [--f64]

Only numpy is needed. The reference is a measuring instrument for the
`--ecpl-spec` path; it says nothing about what Dolby's decoders do, and a
Dolby measurement says nothing about it.

Case file (text, UTF-8, one directive a line, `#` starts a comment)
--------------------------------------------------------------------

    ecplref 1
    window kbd5|table
        Table 6.33 prints the window to five decimals. `table` uses the
        printed values; `kbd5` the Kaiser-Bessel-derived window with
        alpha 5, which the reference checks rounds to every printed entry.
    ola-gain 2|1
        Clause E.3.5.5.1 step 3 prints the overlap-add without the factor 2
        that clause 6.9.4 step 6 carries; as printed the chain is 6.02 dB
        low. 2 is the corrected reading, 1 the printed one.
    random lcg SEED | random file PATH
        The de-correlation values of clause E.3.5.5.3 (see "Random values").
    mant lcg SEED SCALE
        The enhanced coupling mantissas: for every block with enhanced
        coupling, in order, for bin = start .. end-1, SCALE * lcg().
    block nfchans=N begin=B end=E bndstrc=S interp=0|1
        A block with enhanced coupling. B and E are ecpl_begin_subbnd and
        ecpl_end_subbnd; S is ecplbndstrc[0..22] as 22 characters 0/1 (the
        entries below max(9, B + 1) are ignored, as the syntax never sends
        them). The `ch` lines of the block follow, one per channel 0..N-1:
    ch C off
    ch C amp=a,a,... angle=g,g,... chaos=c,c,... trans=0|1
        One code per enhanced coupling band. The first coupled channel's
        angle and chaos are given and must be ignored (clause E.3.5.5.2/3).
    block none
        A block without enhanced coupling: its coefficients count as zero
        for its neighbours (clause E.3.5.5.1), nothing is written for it.
    end
        End of stream: the last block's successor is zero.

Random values
-------------

`lcg` is the 64-bit generator both sides implement bit for bit:

    state = (state * 6364136223846793005 + 1442695040888963407) mod 2**64
    value = ((state >> 11) / 2**53) * 2 - 1          # uniform on [-1, 1)

For `random lcg SEED` the first 5 * 256 draws are rand_notrans[ch][bin]
(ch-major), drawn once. rand_trans[ch][bnd] is then drawn per block with
enhanced coupling, for every coupled channel after the first in ascending
order, for every band 0..necplbnd-1, whether or not that channel carries a
transient. `random file PATH` gives the same values as little-endian f64:
5 * 256 notrans values followed by the trans stream in that order.

Output
------

For every block with enhanced coupling, in order, for every coupled channel
in ascending order, the coefficients chmant[ch][start..end] as little-endian
f32 (f64 with --f64). `start` and `end` are ecplsubbndtab[B] and
ecplsubbndtab[E].

Choices the text leaves open, made here and named in docs/eac3.md:
- angle interpolation with a single band has no pair to interpolate between,
  and the pseudo-code reads `y` and `slope` before setting them: the band
  angle is held flat;
- the interpolation's bin index is relative to the start of the region
  (`bin = count` after the first band), as its first-band branch writes it.
"""

from __future__ import annotations

import argparse
import sys

N = 512
ECPLSUBBNDTAB = (13, 19, 25, 31, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145,
                 157, 169, 181, 193, 205, 217, 229, 241, 253)


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Enhanced coupling reference (A/52 E.3.5.5)")
    ap.add_argument("case", help="case file (see the module documentation)")
    ap.add_argument("-o", "--output", required=True, help="coefficients out")
    ap.add_argument("--f64", action="store_true", help="write f64, not f32")
    ap.parse_args(argv)
    raise NotImplementedError("P3 stop 1: the interface is fixed, the process follows at stop 2")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
