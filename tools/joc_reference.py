#!/usr/bin/env python3
"""An independent reference for the JOC decoder: ETSI TS 103 420 V1.2.1
clauses 6.5 and 6.6 written down from the text, in plain Python floats (IEEE
754 binary64), sharing nothing with oadec's Rust code.

    python tools/joc_reference.py vectors OUT.json
        writes the cases and what the reference makes of them; the tests of
        crates/oadec-engine/tests/joc_configs.rs read OUT.json
        (crates/oadec-engine/tests/support/joc_vectors.json) and hold oadec's
        f64 intermediate values to it within 1e-12.
    python tools/joc_reference.py check OUT.json
        recomputes every case of a vectors file and says whether it still
        holds (so a hand edit of the file cannot pass unnoticed).
    python tools/joc_reference.py trees ts_103420_tables.c OUT.rs
        writes the Huffman trees of Annex A.1 for the test support's payload
        writer, from ETSI's own C tables (ts_103420v010201p0.zip), as a copy
        of its own, not taken from crates/oadec-emdf/src/joc_tables.rs.

The input of a case is the transmitted symbols: joc_mtx for a dense object,
joc_channel_idx and joc_vec for a sparse one, with the header and data point
fields. Everything Huffman is left to the writer and the parser; everything
after the symbols is here:

    differential   pseudo-codes 2 and 3 (clause 6.6.2). Sparse in two readings:
                   'printed' as the clause prints it, 'measured' as docs/joc.md
                   settled it (unselected channels at nquant/2, the index
                   accumulated from the resolved one with no modulo on the first
                   band, the selected value chained unbroken across the bands
                   from the printed seed 50/100).
    dequantize     pseudo-code 5 (clause 6.6.4).
    band_map       table 54 (clause 6.5), copied here from the table.
    interpolate    pseudo-code 6 (clause 6.6.5). Steep in two readings:
                   'printed' (ts < joc_offset_ts) and 'measured'
                   (ts < joc_offset_ts - 1), which docs/joc.md measured for one
                   data point and which is carried to two unmeasured.
    apply          pseudo-code 7 (clause 6.6.6), on subband samples drawn from
                   Lcg, the generator the Rust side has too.

Nothing here knows about the matrix alignment, the filter bank delay or the
90-degree operator: those are oadec's decisions about where the matrix of a
slot meets the samples, and the tests that need them work them out from oadec's
constants.
"""
from __future__ import annotations

import argparse
import json
import sys

NUM_BANDS = [1, 3, 5, 7, 9, 12, 15, 23]          # table 50
NUM_CHANNELS = {0: 5, 1: 7, 2: 7, 3: 5, 4: 7}      # table 48
SUBBANDS = 64
NUM_TS = 24                                         # six blocks of 256 samples

# Table 54: first subband of each row, then the band per column
# (23, 15, 12, 9, 7, 5, 3, 1 bands).
TABLE_54 = [
    (0, [0, 0, 0, 0, 0, 0, 0, 0]),
    (1, [1, 1, 1, 1, 1, 1, 0, 0]),
    (2, [2, 2, 2, 2, 2, 1, 0, 0]),
    (3, [3, 3, 3, 3, 2, 2, 1, 0]),
    (4, [4, 4, 4, 3, 3, 2, 1, 0]),
    (5, [5, 5, 4, 4, 3, 2, 1, 0]),
    (6, [6, 6, 5, 4, 3, 2, 1, 0]),
    (7, [7, 7, 5, 5, 3, 2, 1, 0]),
    (8, [8, 8, 6, 5, 4, 2, 1, 0]),
    (9, [9, 9, 6, 6, 4, 3, 1, 0]),
    (10, [10, 9, 6, 6, 4, 3, 1, 0]),
    (11, [11, 10, 7, 6, 4, 3, 1, 0]),
    (12, [12, 10, 7, 6, 4, 3, 1, 0]),
    (14, [13, 11, 8, 7, 5, 3, 2, 0]),
    (16, [14, 11, 8, 7, 5, 3, 2, 0]),
    (18, [15, 12, 9, 7, 5, 3, 2, 0]),
    (20, [16, 12, 9, 7, 5, 3, 2, 0]),
    (23, [17, 13, 10, 8, 6, 4, 2, 0]),
    (26, [18, 13, 10, 8, 6, 4, 2, 0]),
    (30, [19, 13, 10, 8, 6, 4, 2, 0]),
    (35, [20, 14, 11, 8, 6, 4, 2, 0]),
    (41, [21, 14, 11, 8, 6, 4, 2, 0]),
    (48, [22, 14, 11, 8, 6, 4, 2, 0]),
]
_COLUMN = {23: 0, 15: 1, 12: 2, 9: 3, 7: 4, 5: 5, 3: 6, 1: 7}


def band_map(num_bands: int) -> list[int]:
    """sb_to_pb of clause 6.5 for every subband."""
    col = _COLUMN[num_bands]
    out = []
    for i, (start, cols) in enumerate(TABLE_54):
        end = TABLE_54[i + 1][0] if i + 1 < len(TABLE_54) else SUBBANDS
        out.extend([cols[col]] * (end - start))
    assert len(out) == SUBBANDS
    return out


def nquant(quant_idx: int) -> int:
    return 96 if quant_idx == 0 else 192


def dense(mtx, nch: int, nb: int, quant_idx: int):
    """Pseudo-code 3 for one data point: mtx[ch][pb] -> q[ch][pb]."""
    n, offset = nquant(quant_idx), (48 if quant_idx == 0 else 96)
    q = [[0] * nb for _ in range(nch)]
    for ch in range(nch):
        q[ch][0] = (offset + mtx[ch][0]) % n
        for pb in range(1, nb):
            q[ch][pb] = (q[ch][pb - 1] + mtx[ch][pb]) % n
    return q


def sparse(channel_idx, vec, nch: int, nb: int, quant_idx: int, reading: str):
    """Pseudo-code 2 for one data point: (joc_channel_idx, joc_vec) -> q."""
    n = nquant(quant_idx)
    seed = 50 if quant_idx == 0 else 100
    if reading == "printed":
        q = [[0] * nb for _ in range(nch)]
        for pb in range(nb):
            sel = channel_idx[0] if pb == 0 else (channel_idx[pb - 1] + channel_idx[pb]) % nch
            for ch in range(nch):
                if ch == sel:
                    q[ch][pb] = (seed + vec[0]) % n if pb == 0 else (q[ch][pb - 1] + vec[pb]) % n
                else:
                    q[ch][pb] = seed
        return q
    assert reading == "measured", reading
    zero = n // 2
    q = [[zero] * nb for _ in range(nch)]
    sel, value = None, seed
    for pb in range(nb):
        sel = channel_idx[0] if pb == 0 else (sel + channel_idx[pb]) % nch
        value = (value + vec[pb]) % n
        if sel < nch:            # a first index past the channels selects none
            q[sel][pb] = value
    return q


def dequantize(q: int, quant_idx: int) -> float:
    """Pseudo-code 5."""
    return (q - nquant(quant_idx) / 2) * 820 / (4096 * (1 + quant_idx))


def interpolate(prev, dq, slope_steep: bool, offsets, num_ts: int, reading: str):
    """Pseudo-code 6 for one object: prev[ch][sb], dq[dp][ch][sb] ->
    interp[ts][ch][sb]. `offsets` are the one-based joc_offset_ts."""
    nch = len(prev)
    ndp = len(dq)
    back = 1 if reading == "measured" else 0
    out = []
    for ts in range(num_ts):
        m = [[0.0] * SUBBANDS for _ in range(nch)]
        for ch in range(nch):
            for sb in range(SUBBANDS):
                if not slope_steep:
                    if ndp == 1:
                        delta = dq[0][ch][sb] - prev[ch][sb]
                        m[ch][sb] = prev[ch][sb] + (ts + 1) * delta / num_ts
                    else:
                        ts_2 = num_ts // 2
                        if ts < ts_2:
                            delta = dq[0][ch][sb] - prev[ch][sb]
                            m[ch][sb] = prev[ch][sb] + (ts + 1) * delta / ts_2
                        else:
                            delta = dq[1][ch][sb] - dq[0][ch][sb]
                            m[ch][sb] = dq[0][ch][sb] + (ts - ts_2 + 1) * delta / (num_ts - ts_2)
                else:
                    o0 = offsets[0] - back
                    if ndp == 1:
                        m[ch][sb] = prev[ch][sb] if ts < o0 else dq[0][ch][sb]
                    else:
                        o1 = offsets[1] - back
                        m[ch][sb] = (prev[ch][sb] if ts < o0
                                     else dq[0][ch][sb] if ts < o1 else dq[1][ch][sb])
        out.append(m)
    return out


def apply(matrices, x):
    """Pseudo-code 7 for one slot: matrices[obj][ch][sb] (None for an object
    whose matrix the case does not carry), x[ch][sb] complex -> z[obj][sb]."""
    z = []
    for m in matrices:
        z.append([sum((x[ch][sb] * m[ch][sb] for ch in range(len(m))), 0j)
                  for sb in range(SUBBANDS)])
    return z


class Lcg:
    """The generator the Rust support has too: Knuth's MMIX constants, the top
    53 bits as a value in [-1, 1)."""

    def __init__(self, seed: int):
        self.state = seed & (2**64 - 1)

    def next_f64(self) -> float:
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & (2**64 - 1)
        return (self.state >> 11) / float(1 << 52) - 1.0


def cases():
    """The cases of the vectors file: every axis of the test matrix. Stop 2."""
    raise SystemExit("joc_reference.py: the cases are written at stop 2")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    v = sub.add_parser("vectors")
    v.add_argument("out")
    c = sub.add_parser("check")
    c.add_argument("vectors")
    t = sub.add_parser("trees")
    t.add_argument("tables_c")
    t.add_argument("out")
    args = ap.parse_args(argv)
    if args.cmd == "vectors":
        data = {"generator": "tools/joc_reference.py vectors",
                "band_map": [band_map(n) for n in NUM_BANDS],
                "cases": cases()}
        with open(args.out, "w", encoding="utf-8") as f:
            json.dump(data, f, separators=(",", ":"))
        return 0
    raise SystemExit(f"joc_reference.py {args.cmd}: written at stop 2")


if __name__ == "__main__":
    sys.exit(main())
