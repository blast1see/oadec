#!/usr/bin/env python3
"""What Dolby's decoder adds to enhanced coupled channels, per amplitude code.

    python tools/ecpl_weights.py ECPL_REF.f32 ECPL_OURS.f32 --channels 6 --coupled 0,1,4,5 \\
        --first 109 --sub-bands 7 [--standard STD_REF.f32 STD_OURS.f32] [--out OUT.json]

With `--code K` the stream carries code K in every block (a plan `amp K`),
and the report has the one code: the same frames under every code, which
takes the content of the frames out of the comparison between codes.

`ECPL_REF` is Dolby's decode (`dlbac3dec`, `drop-delay=true`) of a stream
written by `eac3-ecpl-inject --plan` with one amplitude code a frame
(`amp sweep-frames`), `ECPL_OURS` oadec's default decode of it, both raw
32-bit float in WAVE order (`tools/ecpl_amp_sweep.py` writes them). The
`--standard` pair is the same for the stream before the rewrite, whose
standard coupling Dolby's decoder decorrelates as `a X + b D(X)`
(`tools/coupling_mix.py`, `docs/eac3.md`).

Working on the transform coefficients of the blocks (`dither_probe`), it
reports:

1. The filter `D` along the blocks, learned from each stream on its own
   (`coupling_mix.learn`, read as an all-pass of three poles after one
   block): the common filter test compares the two.
2. Per amplitude code and coupled channel, over the blocks of that code:
   the one pair `(a, b)` that takes oadec's `X` and `D(X)` nearest to
   Dolby's coefficients, with `D` the all-pass of the standard stream (or
   of the enhanced one without `--standard`); what is left; the angle
   `atan2(|b|, a)`; and the coherence of Dolby's departure from `a X` with
   `b D(X)`: `|<y - a x, z>|^2 / (|y - a x|^2 |z|^2)`, 1 where the departure
   is that filtered part and nothing else. The amplitude code of a block is
   read from the stream: the frames of code 31 are the only ones whose
   coupled coefficients are zero in oadec's decode (to the rounding of its
   32-bit samples).
3. The level the code leaves: the power of oadec's coupled coefficients
   over the power of the channel's own coefficients under the coupling
   region (dB, `level_ratio_db`).
4. With `--standard`: the angle of the standard stream as a function of the
   same level ratio (the blocks of each channel cut into bins of the ratio,
   a pair per bin), and that curve read at each code's ratio. Where the
   enhanced angles follow the standard curve, the weight follows the level
   and not the code; where they do not, the code (or the tool) sets it.

The comparison is of one stream; it is a measurement, not a model.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import coupling_mix as cm  # noqa: E402
from dither_probe import read_decode  # noqa: E402

SUB_BAND = cm.SUB_BAND
CODES = 32
RATIO_BINS = 16


def code_of_rows(mine, coupled, lo, hi):
    """The amplitude code of every row: rows of code 31 are zero, to the rounding
    of the samples, over the coupled coefficients of every coupled channel,
    in runs of six."""
    e = np.sum([(mine[ch][:, lo:hi] ** 2).sum(axis=1) for ch in coupled], axis=0)
    # zero but for the rounding of the 32-bit samples the coefficients are read from
    zero = e < 1e-9 * float(np.median(e))
    starts = [r for r in range(1, len(zero) - 6) if zero[r] and not zero[r - 1] and zero[r:r + 6].all()]
    if not starts:
        sys.exit("no run of silent coupled blocks: is this an amplitude sweep with code 31?")
    r31 = starts[0]
    gaps = np.diff(starts)
    # a run may be missed where the block has something else there; the ones
    # found must still lie a whole number of 32 frames apart
    if len(gaps) and not (gaps % (CODES * 6) == 0).all():
        sys.exit(f"the silent runs are not 192 blocks apart: {sorted(set(gaps.tolist()))[:5]}")
    rows = np.arange(len(zero))
    return (31 + np.floor_divide(rows - r31, 6)) % CODES, r31


def fit(y, x, z):
    """One pair (a, b) for all of y ~ a x + b z, what it leaves, and the
    coherence of y - a x with z."""
    pair, *_ = np.linalg.lstsq(np.stack([x.ravel(), z.ravel()], axis=1), y.ravel(), rcond=None)
    a, b = float(pair[0]), float(pair[1])
    dev = y - a * x
    left = dev - b * z
    zz, dd = float((z * z).sum()), float((dev * dev).sum())
    coh = float((dev * z).sum()) ** 2 / (dd * zz) if dd > 0 and zz > 0 else float("nan")
    return {"a": round(a, 4), "b": round(b, 4),
            "angle": round(float(np.arctan2(abs(b), a)), 4),
            "left_db": cm.ratio_db(float((y * y).sum()), float((left * left).sum())),
            "coherence": round(coh, 4)}


def learned(theirs, mine, first, sub_bands, learn_from):
    d = cm.learn(theirs[learn_from], mine[learn_from], first, sub_bands, 40, 3)
    if d is None:
        return None
    shape, _ = cm.all_pass(d)
    return shape


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("reference")
    ap.add_argument("ours")
    ap.add_argument("--channels", type=int, required=True)
    ap.add_argument("--coupled", required=True, help="coupled channels, by index in WAVE order")
    ap.add_argument("--first", type=int, required=True)
    ap.add_argument("--sub-bands", type=int, required=True)
    ap.add_argument("--lag", type=int, default=256)
    ap.add_argument("--learn", type=int, help="the channel the filter is learned from")
    ap.add_argument("--standard", nargs=2, metavar=("REF", "OURS"))
    ap.add_argument("--code", type=int, help="every block carries this amplitude code (a plan `amp K`)")
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()

    coupled = [int(c) for c in a.coupled.split(",")]
    lo, hi = a.first, a.first + SUB_BAND * a.sub_bands
    own_lo = max(8, lo - (hi - lo))
    learn_from = a.learn if a.learn is not None else coupled[-2 if len(coupled) > 2 else 0]

    theirs, mine = cm.as_coefficients(read_decode(a.reference, a.channels), read_decode(a.ours, a.channels), a.lag)
    if a.code is None:
        codes, r31 = code_of_rows(mine, coupled, lo, hi)
    else:
        codes, r31 = np.full(len(mine[0]), a.code), -1
    report: dict = {"coupled_coefficients": [lo, hi - 1], "own_coefficients": [own_lo, lo - 1],
                    "first_silent_row": int(r31), "filters": {}}
    report["filters"]["enhanced"] = learned(theirs, mine, a.first, a.sub_bands, learn_from)

    std = None
    if a.standard:
        s_theirs, s_mine = cm.as_coefficients(read_decode(a.standard[0], a.channels),
                                              read_decode(a.standard[1], a.channels), a.lag)
        std = (s_theirs, s_mine)
        report["filters"]["standard"] = learned(s_theirs, s_mine, a.first, a.sub_bands, learn_from)
    ref_filter = report["filters"]["standard" if std else "enhanced"]
    if ref_filter is None:
        sys.exit("no filter found")
    den = np.array(ref_filter["denominator"])
    report["filter_used"] = "standard" if std else "enhanced"

    skip = cm.SKIP
    curves = {}
    if std:
        s_theirs, s_mine = std
        for ch in coupled:
            m = s_mine[ch]
            z = cm.through_all_pass(m[:, lo:hi], den)
            own = (m[:, own_lo:lo] ** 2).sum(axis=1)
            cpl = (m[:, lo:hi] ** 2).sum(axis=1)
            ok = (own > 0) & (cpl > 0)
            ok[:skip] = False
            ratio = np.full(len(m), np.nan)
            ratio[ok] = 10 * np.log10(cpl[ok] / own[ok])
            edges = np.nanpercentile(ratio[ok], np.linspace(0, 100, RATIO_BINS + 1))
            pts = []
            for i in range(RATIO_BINS):
                sel = ok & (ratio >= edges[i]) & (ratio <= edges[i + 1])
                if sel.sum() < 20:
                    continue
                f = fit(s_theirs[ch][sel, lo:hi], m[sel, lo:hi], z[sel])
                pts.append({"ratio_db": round(float(np.nanmedian(ratio[sel])), 2), "blocks": int(sel.sum()), **f})
            whole = fit(s_theirs[ch][skip:, lo:hi], m[skip:, lo:hi], z[skip:])
            curves[ch] = {"bins": pts, "whole": whole}
        report["standard_by_level_ratio"] = {str(k): v for k, v in curves.items()}

    table = []
    for ch in coupled:
        m = mine[ch]
        z = cm.through_all_pass(m[:, lo:hi], den)
        own = (m[:, own_lo:lo] ** 2).sum(axis=1)
        cpl = (m[:, lo:hi] ** 2).sum(axis=1)
        rows = []
        for k in range(CODES):
            sel = codes == k
            sel[:skip] = False
            if (k == 31 and a.code is None) or sel.sum() < 6:
                rows.append({"code": k, "blocks": int(sel.sum()), "silent": True})
                continue
            f = fit(theirs[ch][sel, lo:hi], m[sel, lo:hi], z[sel])
            ratio = 10 * np.log10(cpl[sel].sum() / own[sel].sum()) if own[sel].sum() > 0 else None
            row = {"code": k, "blocks": int(sel.sum()), "level_ratio_db": None if ratio is None else round(float(ratio), 2), **f}
            if ch in curves and ratio is not None and curves[ch]["bins"]:
                xs = np.array([p["ratio_db"] for p in curves[ch]["bins"]])
                ys = np.array([p["angle"] for p in curves[ch]["bins"]])
                order = np.argsort(xs)
                row["standard_angle_at_ratio"] = round(float(np.interp(ratio, xs[order], ys[order])), 4)
                row["inside_standard_range"] = bool(xs.min() <= ratio <= xs.max())
            rows.append(row)
        table.append({"channel": ch, "codes": rows})
    report["by_code"] = table

    # print
    for name, f in report["filters"].items():
        if f:
            print(f"filter from {name}: denominator {f['denominator']}, poles {f['real_poles']} {f['pair']}, misses {f['largest_tap_it_misses_by']}")
    print(f"filter used for the pairs: {report['filter_used']}")
    for t in table:
        ch = t["channel"]
        print(f"\nchannel {ch}" + (f"  (standard, whole stream: a {curves[ch]['whole']['a']} b {curves[ch]['whole']['b']} "
                                  f"angle {curves[ch]['whole']['angle']} coherence {curves[ch]['whole']['coherence']})" if ch in curves else ""))
        print(" code  ratio dB      a       b   angle  std@ratio  coherence  left dB")
        for r in t["codes"]:
            if r.get("silent"):
                continue
            sa = r.get("standard_angle_at_ratio")
            print(f" {r['code']:4} {r['level_ratio_db'] if r['level_ratio_db'] is not None else float('nan'):8.2f} {r['a']:7.3f} {r['b']:7.3f} {r['angle']:7.3f}"
                  f"  {'' if sa is None else f'{sa:7.3f}'}{'' if r.get('inside_standard_range', True) else '*':1}  {r['coherence']:9.3f}  {r['left_db']}")
    if curves:
        for ch, c in curves.items():
            print(f"\nstandard, channel {ch}: angle by level ratio")
            for p in c["bins"]:
                print(f"  {p['ratio_db']:8.2f} dB  angle {p['angle']:.3f}  coherence {p['coherence']:.3f}  ({p['blocks']} blocks)")
    if a.out:
        a.out.write_text(json.dumps(report, indent=1) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
