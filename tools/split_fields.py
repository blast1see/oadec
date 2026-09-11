#!/usr/bin/env python3
"""Which recorded field, if any, splits two sets of streams exactly?

**A split is only evidence when both sides are large.** With two rows on one side
and a hundred and sixty fields, the pairs available to search number in the tens
of thousands while the two-element subsets to isolate number about twenty-five
thousand: a conjunction that picks out exactly those two rows is what such a
search is *expected* to produce by chance. The tool refuses to look for near
splits when the smaller side is under ten rows for the same reason, and prints
the arithmetic when a split is found on a lopsided set.


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


def small_side_warning(a: int, b: int, hits: int) -> bool:
    """Say plainly when a split is what chance would have produced anyway."""
    small = min(a, b)
    if small >= 10:
        return False
    subsets = 1
    for i in range(small):
        subsets = subsets * (a + b - i) // (i + 1)
    print(f"  the smaller side has {small} row(s): there are about {subsets:,} ways to choose "
          f"{small} of {a + b}, so a field or pair that isolates exactly these is a candidate "
          f"and not a finding")
    return True


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
    ap.add_argument("--where", action="append", default=[],
                    help="keep only rows where FIELD=VALUE (repeatable); the field is read from "
                         "the row itself, not from the bag")
    ap.add_argument("--rank", action="store_true",
                    help="rank every field by how many rows its best partition leaves "
                         "unclassifiable, so the best one can be read against the rest")
    ap.add_argument("--pairs", action="store_true",
                    help="also look for a pair of fields whose combination splits the sets")
    args = ap.parse_args()

    rows = json.load(open(args.sweep, encoding="utf-8"))
    for clause in args.where:
        field, _, want = clause.partition("=")
        keep = {"true": True, "false": False}.get(want.lower(), want)
        before = len(rows)
        rows = [r for r in rows if r.get(field) == keep]
        print(f"{clause}: {len(rows)} of {before} rows kept")
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
    if found and small_side_warning(len(yes), len(no), len(found)):
        pass
    if found:
        print(f"{chr(10)}{len(found)} field(s) split the two sets exactly:")
        for f in found:
            print(f"  {f['field']}: with={f['with']}  without={f['without']}")
    else:
        print("\nno recorded field splits the two sets")

    # A near-split means nothing when one side is tiny: with three rows against
    # two hundred, any field whose rare values happen to land there "nearly
    # splits", and dozens will.
    small = min(len(yes), len(no))
    if small < 10 and not args.rank:
        print(f"{chr(10)}not looking for near-splits: the smaller side has {small} row(s), "
              f"where almost any field would appear to nearly split them")
        near = []
        if args.out:
            json.dump({"key": args.key, "with": len(yes), "without": len(no),
                       "splitting": found, "nearly_splitting": near,
                       "splitting_pairs": None},
                      open(args.out, "w"), indent=1, default=str)
        return 0

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

    if args.rank:
        # How good is the best field, next to the others? A near-split means
        # something only if the field that achieves it is far ahead of the field
        # that does not: on a balanced set, a field unrelated to the verdict
        # leaves about as many rows unclassifiable as the smaller side has.
        table = []
        for k in sorted(keys):
            if k in continuous or any(w in k.lower() for w in COUNTS):
                continue
            a, b = Counter(f.get(k) for f in yf), Counter(f.get(k) for f in nf)
            overlap = sum(min(a[v], b[v]) for v in set(a) & set(b))
            table.append((overlap, k, len({f.get(k) for f in yf + nf})))
        table.sort()
        mid = table[len(table) // 2][0] if table else 0
        print(f"{chr(10)}{len(table)} usable fields ranked by the rows no partition of their "
              f"values can classify:")
        for o, k, n in table[:6]:
            print(f"  {o:5d}  {k}  ({n} distinct values)")
        print(f"  median across all of them: {mid}, against {min(len(yes), len(no))} for a field "
              f"that says nothing")

    if args.pairs:
        # A pair of fields splits the sets when no combination of their values
        # appears on both sides. With enough fields some pair always will, so the
        # number that do is the first thing to look at: one is a lead, forty is
        # arithmetic.
        cand = [k for k in sorted(keys)
                if k not in continuous and not any(w in k.lower() for w in COUNTS)]
        pairs = []
        for i, a in enumerate(cand):
            for b in cand[i + 1:]:
                ya = {(f.get(a), f.get(b)) for f in yf}
                nb = {(f.get(a), f.get(b)) for f in nf}
                if ya.isdisjoint(nb):
                    pairs.append({"fields": [a, b], "combinations": len(ya | nb)})
        pairs.sort(key=lambda x: x["combinations"])
        print(f"{chr(10)}{len(pairs)} pair(s) of {len(cand)} usable fields split the sets"
              f" ({len(cand) * (len(cand) - 1) // 2} pairs tested)")
        for x in pairs[:8]:
            print(f"  {x['fields'][0]} + {x['fields'][1]}: {x['combinations']} combinations "
                  f"over {len(rows)} rows")
        if pairs and pairs[0]["combinations"] > len(rows) // 3:
            print("  -- every one needs a combination for a third of the rows or more, "
                  "which is a lookup table rather than a rule")

    if args.out:
        json.dump({"key": args.key, "with": len(yes), "without": len(no),
                   "splitting": found, "nearly_splitting": near,
                   "splitting_pairs": pairs[:20] if args.pairs else None},
                  open(args.out, "w"), indent=1, default=str)
        print(f"\nwritten to {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
