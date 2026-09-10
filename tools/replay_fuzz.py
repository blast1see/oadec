#!/usr/bin/env python3
"""Replay a stored bit-flip campaign and record what every command did.

The audit ran 120 flips on a JOC stream and 100 on a TrueHD stream and kept
only the totals. Both site lists are reproducible: `fuzz2.py` drew them from
`random.seed(seed)` over the file size, and the seeds are 1 for the JOC run
(recorded in the evidence) and 7 for the TrueHD run (recovered by searching
for the seed whose 100 sites contain both stored silent accepts).

For each site this runs `verify`, `decode --format pcm` and the object decode,
and records the exit code, whether the command said anything about integrity,
and the first line it said. A trial is `silent` when the command exits 0 and
reports nothing: that is the defect.
"""
from __future__ import annotations
import json, os, random, re, shutil, subprocess, sys, tempfile

import sys as _sys
from pathlib import Path as _Path

_sys.path.insert(0, str(_Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402

BIN = find_oadec()

# A line that names a non-zero integrity counter, or a first-problem line.
COUNTERS = re.compile(
    r"(\d+)\s+(?:decode errors?|CRC failures?|payload errors?|parse errors?|"
    r"frames? ending inside|sync errors?|lossless check failures?|segment problems?|"
    r"container errors?|OAMD errors?|JOC errors?)", re.I)
FIRST = re.compile(r"first (?:problem|error)\s*:", re.I)
LOSSLESS = re.compile(r"lossless checks?: \d+ performed, (\d+) failed", re.I)
SEGMENT = re.compile(r"segment problems?: (\d+)", re.I)


def judge(out: str) -> tuple[bool, str]:
    """(reported something, the first thing it said)."""
    if "panicked" in out or "RUST_BACKTRACE" in out:
        return True, "PANIC: " + out.strip().splitlines()[0][:160]
    said = []
    for m in COUNTERS.finditer(out):
        if int(m.group(1)) > 0:
            said.append(m.group(0).strip())
    for rx in (LOSSLESS, SEGMENT):
        m = rx.search(out)
        if m and int(m.group(1)) > 0:
            said.append(m.group(0).strip())
    for line in out.splitlines():
        if FIRST.search(line) and "None" not in line and not line.rstrip().endswith(":"):
            said.append(line.strip()[:160])
        if re.search(r"^\s*(error|warning):", line, re.I):
            said.append(line.strip()[:160])
        if re.search(r"PROBLEMS|NON-CONFORMANT", line):
            said.append(line.strip()[:160])
    return bool(said), (said[0] if said else "")


def run(cmd, timeout=180):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return 124, "TIMEOUT"
    return p.returncode, (p.stdout or "") + (p.stderr or "")


def main() -> int:
    src, kind, seed, n, out_path = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
    size = os.path.getsize(src)
    random.seed(seed)
    sites = [(random.randrange(size), random.randrange(8)) for _ in range(n)]

    work = tempfile.mkdtemp(prefix="replay-")
    mutated = os.path.join(work, "in" + os.path.splitext(src)[1])
    trials = []
    for i, (off, bit) in enumerate(sites):
        shutil.copy(src, mutated)
        with open(mutated, "r+b") as f:
            f.seek(off); b = f.read(1)[0]; f.seek(off); f.write(bytes([b ^ (1 << bit)]))

        t = {"trial": i, "offset": off, "bit": bit}
        for name, cmd in commands(kind, mutated, work):
            rc, out = run(cmd)
            said, first = judge(out)
            t[name] = {"exit": rc, "reported": said, "first": first,
                       "silent": rc == 0 and not said,
                       "panic": first.startswith("PANIC")}
        trials.append(t)
        for leftover in os.listdir(work):
            if leftover.startswith("out"):
                os.remove(os.path.join(work, leftover))
        print(f"\r{i + 1}/{n}", end="", flush=True)
    print()
    shutil.rmtree(work, ignore_errors=True)

    cmds = [c for c, _ in commands(kind, "", "")]
    summary = {c: {"silent": sum(t[c]["silent"] for t in trials),
                   "nonzero_exit": sum(t[c]["exit"] != 0 for t in trials),
                   "reported_but_exit_0": sum(t[c]["reported"] and t[c]["exit"] == 0 for t in trials),
                   "panics": sum(t[c]["panic"] for t in trials)} for c in cmds}
    doc = {"source": src, "kind": kind, "seed": seed, "trials": n,
           "binary": BIN, "summary": summary, "detail": trials}
    with open(out_path, "w") as f:
        json.dump(doc, f, indent=1)
    print(json.dumps(summary, indent=1))
    return 0


def commands(kind, mutated, work):
    out = os.path.join(work, "out")
    if kind == "joc":
        return [("verify", [BIN, "verify", mutated]),
                ("decode_pcm", [BIN, "decode", mutated, "--format", "pcm", "-o", out + ".f32"]),
                ("decode_damf", [BIN, "decode", mutated, "--format", "damf", "-o", out])]
    return [("verify", [BIN, "verify", mutated]),
            ("decode_pcm", [BIN, "decode", mutated, "-p", "3", "--format", "pcm", "-o", out + ".pcm"])]


if __name__ == "__main__":
    sys.exit(main())
