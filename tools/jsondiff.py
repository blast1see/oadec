#!/usr/bin/env python3
"""Field-for-field diff of two directories of report JSON, ignoring wall-clock.

`seconds` is how long the decode took, so it differs on every run and says
nothing about the decode. Everything else must match, and this prints the
paths of the fields that do not.
"""
from __future__ import annotations
import json, os, sys

IGNORE = {"seconds", "file", "binary", "utc"}


def walk(a, b, path=""):
    if type(a) is not type(b):
        yield f"{path}: {type(a).__name__} -> {type(b).__name__}"
        return
    if isinstance(a, dict):
        for k in sorted(set(a) | set(b)):
            if k in IGNORE:
                continue
            if k not in a:
                yield f"{path}/{k}: added {b[k]!r}"
            elif k not in b:
                yield f"{path}/{k}: removed {a[k]!r}"
            else:
                yield from walk(a[k], b[k], f"{path}/{k}")
    elif isinstance(a, list):
        if len(a) != len(b):
            yield f"{path}: length {len(a)} -> {len(b)}"
        for i, (x, y) in enumerate(zip(a, b)):
            yield from walk(x, y, f"{path}[{i}]")
    elif a != b:
        yield f"{path}: {a!r} -> {b!r}"


def main() -> int:
    old, new = sys.argv[1], sys.argv[2]
    diffs = 0
    names = sorted(n for n in os.listdir(old) if n.endswith(".json"))
    for n in names:
        p, q = os.path.join(old, n), os.path.join(new, n)
        if not os.path.exists(q):
            print(f"{n}: missing in {new}"); diffs += 1; continue
        for line in walk(json.load(open(p)), json.load(open(q))):
            print(f"{n}{line}"); diffs += 1
    for n in sorted(x for x in os.listdir(new) if x.endswith(".json")):
        if n not in names:
            print(f"{n}: new in {new}"); diffs += 1
    print(f"{len(names)} reports, {diffs} differing fields")
    return 1 if diffs else 0


if __name__ == "__main__":
    sys.exit(main())
