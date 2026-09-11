#!/usr/bin/env python3
"""Which channel of A is which channel of B, decided by measurement.

Appending a dependent substream's channels to a core's is not the same as
knowing what they are. This builds the full distance matrix between every
channel of two interleaved float files and reports, for each channel of A, the
channel of B it matches and by how much it beats the runner-up. A merge is only
proven when every intended pairing wins by a wide margin.

    python pairing.py ours.f32 ref.f32 --ach 8 --bch 8 \
        --anames L,R,C,LFE,Lrs,Rrs,Ls,Rs --bnames FL,FR,FC,LFE,BL,BR,SL,SR
"""
from __future__ import annotations
import argparse, json
import numpy as np


def load(path, ch, dtype="<f4", limit=0):
    a = np.fromfile(path, dtype=dtype)
    n = len(a) // ch
    x = a[: n * ch].reshape(n, ch).astype(np.float64)
    return x[:limit] if limit else x


def sdr(a, b):
    err = np.sqrt(((a - b) ** 2).mean())
    sig = np.sqrt((b ** 2).mean())
    if sig == 0:
        return None
    return float("inf") if err == 0 else round(20 * np.log10(sig / err), 2)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("a"); ap.add_argument("b")
    ap.add_argument("--ach", type=int, required=True)
    ap.add_argument("--bch", type=int, required=True)
    ap.add_argument("--anames", default=""); ap.add_argument("--bnames", default="")
    ap.add_argument("--seconds", type=float, default=0)
    ap.add_argument("--rate", type=int, default=48000)
    ap.add_argument("--json", default="")
    o = ap.parse_args()
    limit = int(o.seconds * o.rate) if o.seconds else 0
    A, B = load(o.a, o.ach, limit=limit), load(o.b, o.bch, limit=limit)
    n = min(len(A), len(B)); A, B = A[:n], B[:n]
    an = o.anames.split(",") if o.anames else [f"a{i}" for i in range(o.ach)]
    bn = o.bnames.split(",") if o.bnames else [f"b{i}" for i in range(o.bch)]

    matrix, rows = [], []
    for i in range(o.ach):
        row = [sdr(A[:, i], B[:, j]) for j in range(o.bch)]
        matrix.append(row)
        scored = sorted(((v if v is not None else -999, j) for j, v in enumerate(row)), reverse=True)
        best, second = scored[0], scored[1]
        rows.append({"a": an[i], "best": bn[best[1]], "best_sdr_db": row[best[1]],
                     "runner_up": bn[second[1]], "runner_up_sdr_db": row[second[1]],
                     "margin_db": round(best[0] - second[0], 2)})
    doc = {"a": o.a, "b": o.b, "samples": int(n), "a_names": an, "b_names": bn,
           "matrix_sdr_db": matrix, "pairing": rows,
           "unambiguous": all(r["margin_db"] >= 10 for r in rows)}
    print(json.dumps(doc, indent=1))
    if o.json:
        open(o.json, "w").write(json.dumps(doc, indent=1))


if __name__ == "__main__":
    main()
