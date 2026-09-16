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

The campaign judges itself. A silent accept, a fault reported at exit 0, a panic
or a timeout makes the run exit non-zero, so a runner that reads exit codes --
`tools/media_regression.sh` -- cannot pass over what the campaign found. A
non-zero exit from the decoder is not a fault: it is what catching the
corruption looks like.
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


USAGE = ("usage: replay_fuzz.py STREAM {joc|ddp71|truehd} SEED COUNT OUT.json\n"
         "  STREAM  the clean stream to inject single-bit errors into\n"
         "  SEED    the seed that names the campaign; the same seed replays the same sites\n"
         "  COUNT   how many sites\n"
         "  OUT     where the per-site exit codes and diagnostics go")


def main() -> int:
    if len(sys.argv) != 6:
        print(USAGE, file=sys.stderr)
        return 2
    src, kind, seed, n, out_path = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5]
    if not os.path.isfile(src):
        print(f"{src} is not a file\n\n{USAGE}", file=sys.stderr)
        return 2
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
            # A timeout leaves a non-zero exit and nothing said, which is what a
            # command that caught the corruption also looks like. It is counted
            # on its own so that a campaign cannot pass because a decoder hung.
            timed_out = rc == 124 and out == "TIMEOUT"
            t[name] = {"exit": rc, "reported": said,
                       "first": "TIMEOUT" if timed_out else first,
                       "silent": rc == 0 and not said,
                       "panic": first.startswith("PANIC"),
                       "timeout": timed_out}
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
                   "panics": sum(t[c]["panic"] for t in trials),
                   "timeouts": sum(t[c]["timeout"] for t in trials)} for c in cmds}
    # What the campaign went looking for, in the four counters that mean it was
    # found. `nonzero_exit` is not among them: that is the decoder catching the
    # corruption, which is the outcome this campaign wants.
    faults = {c: {k: v for k, v in s.items()
                  if k in ("silent", "reported_but_exit_0", "panics", "timeouts") and v}
              for c, s in summary.items()}
    faults = {c: f for c, f in faults.items() if f}
    doc = {"source": src, "kind": kind, "seed": seed, "trials": n, "binary": BIN,
           "verdict": "FAIL" if faults else "PASS", "faults": faults,
           "summary": summary, "detail": trials}
    with open(out_path, "w") as f:
        json.dump(doc, f, indent=1)
    print(json.dumps(summary, indent=1))
    if faults:
        for command, found in faults.items():
            print(f"{command}: " + ", ".join(f"{k} {v}" for k, v in found.items()), file=sys.stderr)
        print(f"campaign FAILED: {len(faults)} of {len(cmds)} commands; see {out_path}", file=sys.stderr)
        return 1
    print("campaign PASSED: no silent accept, nothing reported at exit 0, no panic, no timeout")
    return 0


def commands(kind, mutated, work):
    out = os.path.join(work, "out")
    if kind == "joc":
        return [("verify", [BIN, "verify", mutated]),
                ("decode_pcm", [BIN, "decode", mutated, "--format", "pcm", "-o", out + ".f32"]),
                ("decode_damf", [BIN, "decode", mutated, "--format", "damf", "-o", out])]
    if kind == "ddp71":
        # the programme path, which merges dependent substreams, and the
        # independent-only path beside it: a corruption that reaches one and not
        # the other is worth seeing separately
        return [("verify", [BIN, "verify", mutated]),
                ("decode_programme", [BIN, "decode", mutated, "--format", "pcm",
                                      "-o", out + ".f32"]),
                ("decode_core_only", [BIN, "decode", mutated, "--format", "pcm", "--core-only",
                                      "-o", out + ".core.f32"])]
    return [("verify", [BIN, "verify", mutated]),
            ("decode_pcm", [BIN, "decode", mutated, "-p", "3", "--format", "pcm", "-o", out + ".pcm"])]


if __name__ == "__main__":
    sys.exit(main())
