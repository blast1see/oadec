#!/usr/bin/env python3
"""Run ``oadec decode`` under provenance capture.

    python run_decode.py --oadec <exe> --input <stream> --out <base> --format damf|adm [--no-bed-conform] [--extra ...]

Writes ``<base>.run.json`` next to the output with the command, exit code,
stdout/stderr, timing, the binary's SHA-256 and a record of every output file.
``OADEC_*`` environment variables are removed for the run and listed.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import provenance  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--oadec", required=True)
    ap.add_argument("--input", required=True)
    ap.add_argument("--out", required=True, help="output base (oadec -o)")
    ap.add_argument("--format", required=True, choices=["damf", "adm", "wav", "pcm"])
    ap.add_argument("--no-bed-conform", action="store_true")
    ap.add_argument("--dolby-origin-tag", action="store_true")
    ap.add_argument("--presentation", type=int, default=None)
    ap.add_argument("--extra", nargs="*", default=[])
    ap.add_argument("--timeout", type=float, default=None)
    a = ap.parse_args()

    os.makedirs(os.path.dirname(os.path.abspath(a.out)) or ".", exist_ok=True)
    argv = [a.oadec, "decode", a.input, "--format", a.format, "-o", a.out]
    if a.presentation is not None:
        argv += ["-p", str(a.presentation)]
    if a.no_bed_conform:
        argv.append("--no-bed-conform")
    if a.dolby_origin_tag:
        argv.append("--dolby-origin-tag")
    argv += a.extra
    rec = provenance.run(argv, timeout=a.timeout)
    outputs = []
    patterns = {"damf": [a.out + ".atmos", a.out + ".atmos.metadata", a.out + ".atmos.audio"],
                "adm": [a.out + ".wav"], "wav": [a.out + ".wav", a.out], "pcm": [a.out]}[a.format]
    for p in patterns:
        if os.path.isfile(p):
            outputs.append(provenance.file_record(p))
    doc = {
        "run": rec.to_json(),
        "binary": provenance.file_record(a.oadec),
        "input": provenance.file_record(a.input),
        "outputs": outputs,
    }
    record = f"{a.out}.{a.format}.run.json"
    with open(record, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print(f"exit {rec.exit_code} in {rec.seconds:.1f}s; {len(outputs)} output file(s); record {record}")
    if rec.stderr.strip():
        print(rec.stderr.strip()[-2000:])
    return 0 if rec.exit_code == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
