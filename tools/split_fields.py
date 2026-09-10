#!/usr/bin/env python3
"""Which recorded field, if any, splits two sets of streams exactly?

A sweep that records a verdict and a bag of fields per stream answers "is this a
class" on its own. It answers "what is the class" only if something in the bag
takes one set of values on one side and a disjoint set on the other. This looks
for that, and says so plainly when nothing does.

    python tools/split_fields.py sweep.json --key opens --out split.json

`--key` names the boolean the rows are split on. Fields whose values are counts
rather than properties -- anything that scales with how much audio was read --
are excluded by name, because a count always splits a set of two and never means
anything.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter

# a count is not a property: it scales with how much of the stream was read
COUNTS = ("frames", "samples", "bytes", "units", "payloads", "containers", "seconds",
          "duration", "syncs", "updates", "errors", "count")


def flatten(d, prefix=""):
    out = {}
    for k, v in (d or {}).items():
        p = f"{prefix}{k}"
        if isinstance(v, dict):
            out.update(flatten(v, p + "."))
        elif isinstance(v, list):
            out[p] = json.dumps(v, sort_keys=True)
        else:
            out[p] = v
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("sweep")
    ap.add_argument("--key", default="opens")
    ap.add_argument("--fields", default="fields",
                    help="the member holding the bag of fields, or '' for the whole row")
    ap.add_argument("--out")
    args = ap.parse_args()

    rows = json.load(open(args.sweep, encoding="utf-8"))
    yes = [r for r in rows if r.get(args.key)]
    no = [r for r in rows if not r.get(args.key)]
    print(f"{len(rows)} rows: {len(yes)} with {args.key}, {len(no)} without")
    if not yes or not no:
        print("one side is empty, so nothing can split them")
        return 0

    def bag(r):
        return flatten(r.get(args.fields) if args.fields else r)

    yf, nf = [bag(r) for r in yes], [bag(r) for r in no]
    keys = set().union(*[set(f) for f in yf + nf])
    # a field with nearly one value per row -- a peak bit rate, a duration -- will
    # split any partition by accident, so it cannot be evidence of one
    spread = {k: len({f.get(k) for f in yf + nf}) for k in keys}
    continuous = {k for k in keys if spread[k] > max(4, len(rows) // 2)}
    if continuous:
        print(f"ignoring {len(continuous)} field(s) with a value for nearly every row: "
              f"{sorted(continuous)[:6]}")
    found = []
    for k in sorted(keys):
        if k in continuous or any(w in k.lower() for w in COUNTS):
            continue
        a = {f.get(k) for f in yf}
        b = {f.get(k) for f in nf}
        if a.isdisjoint(b):
            found.append({"field": k,
                          "with": sorted(a, key=str)[:6],
                          "without": sorted(b, key=str)[:6]})
    if found:
        print(f"\n{len(found)} field(s) split the two sets exactly:")
        for f in found:
            print(f"  {f['field']}: with={f['with']}  without={f['without']}")
    else:
        print("\nno recorded field splits the two sets")

    # the next most useful thing: fields that are nearly a split
    near = []
    for k in sorted(keys):
        if (k in continuous or any(w in k.lower() for w in COUNTS)
                or any(f["field"] == k for f in found)):
            continue
        a, b = Counter(f.get(k) for f in yf), Counter(f.get(k) for f in nf)
        shared = set(a) & set(b)
        overlap = sum(min(a[v], b[v]) for v in shared)
        if overlap and overlap <= max(2, len(rows) // 10):
            near.append({"field": k, "rows_on_both_sides": overlap,
                         "with": dict(a), "without": dict(b)})
    if near:
        print(f"\n{len(near)} field(s) nearly split them:")
        for f in sorted(near, key=lambda x: x["rows_on_both_sides"])[:8]:
            print(f"  {f['field']}: {f['rows_on_both_sides']} row(s) on both sides; "
                  f"with={f['with']} without={f['without']}")

    if args.out:
        json.dump({"key": args.key, "with": len(yes), "without": len(no),
                   "splitting": found, "nearly_splitting": near},
                  open(args.out, "w"), indent=1, default=str)
        print(f"\nwritten to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
