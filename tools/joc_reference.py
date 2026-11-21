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


# ---------------------------------------------------------------------------
# the cases: symbols chosen so that the decoder arrives at a given matrix

ALL_SLOTS = list(range(NUM_TS))
SOME_SLOTS = [0, 1, 5, 11, 12, 13, 22, 23]


def symbols_dense(q, quant_idx):
    """joc_mtx that pseudo-code 3 turns into q[ch][pb]."""
    n, offset = nquant(quant_idx), (48 if quant_idx == 0 else 96)
    return [[(row[0] - offset) % n] + [(row[pb] - row[pb - 1]) % n for pb in range(1, len(row))]
            for row in q]


def symbols_sparse(sel, nch, quant_idx):
    """(joc_channel_idx, joc_vec) that the measured reading of pseudo-code 2
    turns into channel sel[pb][0] at value sel[pb][1] in every band."""
    n, seed = nquant(quant_idx), (50 if quant_idx == 0 else 100)
    idx = [sel[0][0]] + [(sel[pb][0] - sel[pb - 1][0]) % nch for pb in range(1, len(sel))]
    vec = [(sel[0][1] - seed) % n] + [(sel[pb][1] - sel[pb - 1][1]) % n for pb in range(1, len(sel))]
    return idx, vec


def code(gain, quant_idx):
    """The quantized value whose dequantization is nearest `gain`."""
    step = 820 / (4096 * (1 + quant_idx))
    return max(0, min(nquant(quant_idx) - 1, round(gain / step) + nquant(quant_idx) // 2))


def dense_object(obj, gains, quant_idx, bands_idx, steep=False, offsets=(), points=None):
    """An object from gains[dp][ch][pb] (a gain, not a code)."""
    nb = NUM_BANDS[bands_idx]
    points = points or gains
    mtx = []
    for g in points:
        q = [[code(g[ch][pb] if isinstance(g[ch], list) else g[ch], quant_idx) for pb in range(nb)]
             for ch in range(len(g))]
        mtx.append(symbols_dense(q, quant_idx))
    return {"obj": obj, "num_bands_idx": bands_idx, "sparse": False, "quant_idx": quant_idx,
            "steep": steep, "offset_ts_bits": [o - 1 for o in offsets] if steep else [],
            "mtx": mtx, "channel_idx": [], "vec": []}


def sparse_object(obj, sels, nch, quant_idx, bands_idx, steep=False, offsets=()):
    """An object from sels[dp][pb] = (channel, gain)."""
    idxs, vecs = [], []
    for sel in sels:
        i, v = symbols_sparse([(c, code(g, quant_idx)) for c, g in sel], nch, quant_idx)
        idxs.append(i)
        vecs.append(v)
    return {"obj": obj, "num_bands_idx": bands_idx, "sparse": True, "quant_idx": quant_idx,
            "steep": steep, "offset_ts_bits": [o - 1 for o in offsets] if steep else [],
            "mtx": [], "channel_idx": idxs, "vec": vecs}


def payload(dmx_config, num_objects, objects, seq=0, clip=(4, 0)):
    return {"dmx_config": dmx_config, "num_objects": num_objects, "clipgain_x": clip[0],
            "clipgain_y": clip[1], "seq_count": seq, "objects": objects}


def case_specs():
    """Every axis of the test matrix of the brief, as (name, axes, options, frames)."""
    import random
    r = random.Random(20261121)
    nch = 7
    eye = [[1.0 if ch == k else 0.0 for ch in range(nch)] for k in range(nch)]
    out = []

    # selection, dense fine, 23 bands, smooth one point, across a frame boundary
    sel = [dense_object(k, [eye[k]], 1, 7) for k in range(nch)]
    out.append(("selection", ["selection", "dense", "fine", "bands-23", "smooth-1", "config-1",
                              "frame-boundary"], {},
                [(payload(1, nch, sel, 0), SOME_SLOTS), (payload(1, nch, sel, 1), SOME_SLOTS)]))
    # permutation, coarse, 15 bands, configuration 2
    perm = [dense_object(k, [eye[(k + 3) % nch]], 0, 6) for k in range(nch)]
    out.append(("permutation", ["permutation", "coarse", "bands-15", "config-2", "smooth-1"], {},
                [(payload(2, nch, perm, 0), SOME_SLOTS), (payload(2, nch, perm, 1), SOME_SLOTS)]))
    # zero, 12 bands; negative, 9 bands; unequal, 7 bands; mixture, 5 bands
    zero = [dense_object(0, [[0.0] * nch], 1, 5)]
    out.append(("zero", ["zero", "bands-12", "config-1"], {},
                [(payload(1, 1, zero, 0), SOME_SLOTS)]))
    neg = [dense_object(k, [[[-r.uniform(0.1, 9.5) for _ in range(9)] if ch == k else [0.0] * 9
                             for ch in range(nch)]], 1, 3) for k in range(3)]
    out.append(("negative", ["negative", "bands-9", "config-2"], {},
                [(payload(2, 3, neg, 0), SOME_SLOTS), (payload(2, 3, neg, 1), SOME_SLOTS)]))
    uneq = [dense_object(k, [[[r.uniform(0.1, 4.0) for _ in range(7)] if ch == k else [0.0] * 7
                              for ch in range(nch)]], 0, 2) for k in range(4)]
    out.append(("unequal", ["unequal", "bands-7", "coarse", "config-1"], {},
                [(payload(1, 4, uneq, 0), SOME_SLOTS)]))
    mix = [dense_object(0, [[[r.uniform(-3.0, 3.0) for _ in range(5)] for _ in range(nch)]], 1, 1),
           dense_object(1, [[0.5, 0.0, 0.7, 0.0, 0.0, -0.3, 0.2]], 1, 1)]
    out.append(("mixture", ["mixture", "bands-5", "config-2"], {},
                [(payload(2, 2, mix, 0), SOME_SLOTS), (payload(2, 2, mix, 1), SOME_SLOTS)]))
    # smooth with two data points, 3 bands, every slot
    two = [dense_object(0, None, 1, 1,
                        points=[[[r.uniform(-2, 2)] * 3 for _ in range(nch)],
                                [[r.uniform(-2, 2)] * 3 for _ in range(nch)]])]
    out.append(("smooth-2", ["smooth-2", "bands-3", "config-1", "frame-boundary"], {},
                [(payload(1, 1, [dense_object(0, [eye[2]], 1, 1)], 0), SOME_SLOTS),
                 (payload(1, 1, two, 1), ALL_SLOTS)]))
    # steep with one point: an offset per object, 1 to 24, one band, every slot
    st1 = [dense_object(k, [[r.uniform(-5, 5) for _ in range(nch)]], 1, 0, steep=True,
                        offsets=(1 + (k * 23) // 15,)) for k in range(16)]
    out.append(("steep-1", ["steep-1", "bands-1", "config-1"], {},
                [(payload(1, 16, [dense_object(k, [eye[k % nch]], 1, 0) for k in range(16)], 0),
                  SOME_SLOTS),
                 (payload(1, 16, st1, 1), ALL_SLOTS)]))
    # steep with two points
    pairs = [(6, 15), (1, 24), (12, 13), (1, 2), (23, 24), (2, 3)]
    st2 = [dense_object(k, None, 0, 7, steep=True, offsets=p,
                        points=[[r.uniform(-5, 5) for _ in range(nch)],
                                [r.uniform(-5, 5) for _ in range(nch)]])
           for k, p in enumerate(pairs)]
    out.append(("steep-2", ["steep-2", "bands-23", "coarse", "config-2"], {},
                [(payload(2, len(pairs), st2, 0), ALL_SLOTS)]))
    # the printed steep reading, for the flag that restores it
    out.append(("steep-printed", ["steep-1", "steep-2", "config-1"], {"steep": "printed"},
                [(payload(1, 16, st1, 0), SOME_SLOTS), (payload(1, len(pairs), st2, 1), SOME_SLOTS)]))
    # sparse, fine and coarse, 23 and 7 bands, one and two points
    sp = [sparse_object(0, [[(r.randrange(nch), r.uniform(-4, 4)) for _ in range(23)]], nch, 1, 7),
          sparse_object(1, [[(r.randrange(nch), r.uniform(-4, 4)) for _ in range(7)],
                            [(r.randrange(nch), r.uniform(-4, 4)) for _ in range(7)]], nch, 0, 3),
          sparse_object(2, [[(r.randrange(nch), r.uniform(-4, 4)) for _ in range(15)]], nch, 0, 6,
                        steep=True, offsets=(9,))]
    out.append(("sparse", ["sparse", "fine", "coarse", "smooth-1", "smooth-2", "steep-1",
                           "config-1"], {},
                [(payload(1, 3, sp, 0), SOME_SLOTS), (payload(1, 3, sp, 1), SOME_SLOTS)]))
    out.append(("sparse-printed", ["sparse", "config-2"], {"sparse": "printed"},
                [(payload(2, 3, sp, 0), SOME_SLOTS)]))
    # hold: frames without a payload keep the matrices; absent objects keep theirs
    hold_a = [dense_object(k, [[r.uniform(-2, 2) for _ in range(nch)]], 1, 4) for k in range(3)]
    hold_b = [dense_object(1, [[r.uniform(-2, 2) for _ in range(nch)]], 1, 4)]
    out.append(("hold", ["hold", "absent", "frame-boundary", "config-1"], {},
                [(payload(1, 3, hold_a, 0), SOME_SLOTS), (None, SOME_SLOTS), (None, SOME_SLOTS),
                 (payload(1, 3, hold_b, 3), ALL_SLOTS), (payload(1, 3, [], 4), SOME_SLOTS)]))
    return out


def rle(row):
    """A row of 64 values as [[value, count], ...]: the subbands of a
    parameter band share their value, so the file stays small."""
    out = []
    for v in row:
        if out and out[-1][0] == v:
            out[-1][1] += 1
        else:
            out.append([v, 1])
    return out


def unrle(runs):
    return [v for v, n in runs for _ in range(n)]


def slots_of(interp_obj, slots):
    return [[ts, [rle(row) for row in interp_obj[ts]]] for ts in slots]


def run_case(name, axes, options, frames):
    steep_reading = options.get("steep", "measured")
    sparse_reading = options.get("sparse", "measured")
    first = next(p for p, _ in frames if p is not None)
    nch = NUM_CHANNELS[first["dmx_config"]]
    nobj = first["num_objects"]
    seed = sum(ord(c) * 131 ** i for i, c in enumerate(name)) % (2 ** 63)
    rng = Lcg(seed)
    prev = [[[0.0] * SUBBANDS for _ in range(nch)] for _ in range(nobj)]
    out_frames = []
    for f, (p, slots) in enumerate(frames):
        interp = [[[row[:] for row in prev[o]] for _ in range(NUM_TS)] for o in range(nobj)]
        expect = []
        present = {o["obj"]: o for o in (p["objects"] if p else [])}
        for obj in range(nobj):
            o = present.get(obj)
            if o is None:
                continue
            nb = NUM_BANDS[o["num_bands_idx"]]
            qi = o["quant_idx"]
            if o["sparse"]:
                q = [sparse(i, v, nch, nb, qi, sparse_reading) for i, v in zip(o["channel_idx"], o["vec"])]
            else:
                q = [dense(m, nch, nb, qi) for m in o["mtx"]]
            dq = [[[dequantize(v, qi) for v in row] for row in qd] for qd in q]
            bmap = band_map(nb)
            dq_sb = [[[d[ch][bmap[sb]] for sb in range(SUBBANDS)] for ch in range(nch)] for d in dq]
            offsets = [b + 1 for b in o["offset_ts_bits"]]
            interp[obj] = interpolate(prev[obj], dq_sb, o["steep"], offsets, NUM_TS, steep_reading)
            prev[obj] = dq_sb[-1]
            expect.append({"obj": obj, "num_bands_idx": o["num_bands_idx"], "sparse": o["sparse"],
                           "quant_idx": qi, "steep": o["steep"], "offset_ts": offsets,
                           "q": q, "dq": dq,
                           "slots": slots_of(interp[obj], slots)})
        # objects that hold are listed too, so a hold is checked
        for obj in range(nobj):
            if obj not in present:
                expect.append({"obj": obj, "hold": True,
                               "slots": slots_of(interp[obj], slots[:2])})
        z = []
        for ts in range(NUM_TS):
            x = [[complex(rng.next_f64(), rng.next_f64()) for _ in range(SUBBANDS)] for _ in range(nch)]
            zs = apply([interp[o][ts] for o in range(nobj)], x)
            for k in range(3):
                obj, sb = (ts + k) % nobj, (ts * 11 + k * 23) % SUBBANDS
                z.append([obj, ts, sb, zs[obj][sb].real, zs[obj][sb].imag])
        out_frames.append({"payload": p, "expect": expect, "z": z})
    return {"name": name, "axes": axes, "steep_printed": steep_reading == "printed",
            "sparse_printed": sparse_reading == "printed", "num_ts": NUM_TS, "seed": seed,
            "frames": out_frames}


def cases():
    return [run_case(*c) for c in case_specs()]


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
    if args.cmd == "check":
        with open(args.vectors, encoding="utf-8") as f:
            have = json.load(f)
        want = json.loads(json.dumps({"generator": "tools/joc_reference.py vectors",
                                      "band_map": [band_map(n) for n in NUM_BANDS],
                                      "cases": cases()}))
        if have != want:
            print(f"{args.vectors}: does not hold; regenerate it with `vectors`")
            return 1
        print(f"{args.vectors}: {len(have['cases'])} cases hold")
        return 0
    return trees(args.tables_c, args.out)


TREES = [  # (name in ts_103420_tables.c, Rust constant, Annex A.1 table)
    ("joc_huff_code_coarse_generic", "COARSE_MTX", "A.1, mode 0, type MTX"),
    ("joc_huff_code_fine_generic", "FINE_MTX", "A.2, mode 1, type MTX"),
    ("joc_huff_code_coarse_coeff_sparse", "COARSE_VEC", "A.3, mode 0, type VEC"),
    ("joc_huff_code_fine_coeff_sparse", "FINE_VEC", "A.4, mode 1, type VEC"),
    ("joc_huff_code_5ch_pos_index_sparse", "IDX_5CH", "A.5, mode 5, type IDX"),
    ("joc_huff_code_7ch_pos_index_sparse", "IDX_7CH", "A.6, mode 7, type IDX"),
]


def trees(tables_c: str, out: str) -> int:
    """The six Huffman trees of Annex A.1 as Rust, for the payload writer of
    the test support: read as data from ETSI's C file, nothing of it run."""
    import re
    src = open(tables_c, encoding="latin-1").read()
    lines = ["//! The Huffman trees of Annex A.1 of ETSI TS 103 420 V1.2.1, the test",
             "//! support's own copy: written by `python tools/joc_reference.py trees`",
             "//! from `ts_103420_tables.c` of ETSI's `ts_103420v010201p0.zip`, not taken",
             "//! from `oadec-emdf`. Node `[left, right]`; a child `c <= 0` is the leaf",
             "//! `-c - 1` (clause 6.6.3).", ""]
    for cname, rname, table in TREES:
        m = re.search(r"const int %s\[\]\[2\] =\s*\{(.*?)\};" % re.escape(cname), src, re.S)
        if not m:
            raise SystemExit(f"{cname} not found")
        pairs = re.findall(r"\{\s*(-?\d+)\s*,\s*(-?\d+)\s*\}", m.group(1))
        leaves = sorted(-int(v) - 1 for p in pairs for v in p if int(v) <= 0)
        if leaves != list(range(len(leaves))):
            raise SystemExit(f"{cname}: the leaves are not 0..{len(leaves) - 1}")
        lines.append(f"/// Table {table}: {len(leaves)} values.")
        lines.append(f"pub const {rname}: [[i16; 2]; {len(pairs)}] = [")
        lines += [f"    [{a}, {b}]," for a, b in pairs]
        lines += ["];", ""]
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines))
    print(f"{out}: {len(TREES)} trees")
    return 0


if __name__ == "__main__":
    sys.exit(main())
