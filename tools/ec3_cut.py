#!/usr/bin/env python3
"""Cut a range of frame groups out of an AC-3 / E-AC-3 elementary stream.

Rare syntax does not sit at the front of a file. `verify --json` reports the
frames where sparse matrices, coarse quantisation and the unusual interpolation
branches occur; this turns one of those frame numbers into a clip that starts on
a syncframe boundary and contains it, which is what a reference decoder needs.

    python tools/ec3_cut.py in.ec3 out.ec3 --from-group 40800 --groups 400

A group is an independent substream and the dependent substreams that
immediately follow it (clause E.1.3.1.2), so the cut never lands between a core
frame and the substream that extends it.
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ec3_structure import parse_frame  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("input")
    ap.add_argument("output")
    ap.add_argument("--from-group", type=int, required=True)
    ap.add_argument("--groups", type=int, default=200)
    a = ap.parse_args()

    data = open(a.input, "rb").read()
    off = 0
    group = -1
    start = end = None
    while off + 6 <= len(data):
        f = parse_frame(data, off)
        if f is None or f["bytes"] <= 0 or off + f["bytes"] > len(data):
            break
        if f["strmtyp"] != 1:
            group += 1
            if group == a.from_group:
                start = off
            if start is not None and group == a.from_group + a.groups:
                end = off
                break
        off += f["bytes"]
    if start is None:
        print(f"group {a.from_group} not found; the file has {group + 1}", file=sys.stderr)
        return 1
    if end is None:
        end = off
    Path(a.output).write_bytes(data[start:end])
    print(f"groups {a.from_group}..{a.from_group + a.groups} -> bytes {start}..{end} "
          f"({end - start} bytes) -> {a.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
