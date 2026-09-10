#!/usr/bin/env python3
"""Corrupt a dependent substream's header and repair the frame CRC, so the parser
has to be the thing that notices.

The plain bit-flip campaign says what it should about this path -- 150 sites, no
panic, no silent success -- and it does not exercise the parser at all: every
single-bit error lands under the frame CRC and is caught before any syntax is
read. The interesting failures are the ones the CRC cannot see.

That matters here more than elsewhere. The dependent substream's `bsi` had never
executed on real material before this round, and `frame.rs` seeks the audio frame
header to `bsi.end_bit`: one bit wrong in that path does not give a slightly
wrong frame, it gives a garbage frame. A corruption that passes the CRC is
exactly the input that finds out whether the parse is defensive.

    python tools/fuzz_dependent_bsi.py in.ec3 SEED COUNT out.json

Each trial flips one bit inside a dependent frame's `bsi` -- `dialnorm`, `compr`,
`chanmape`, `chanmap`, the mixing and information flags -- and rewrites `crc2` so
the frame is well formed. The decoder must not panic, and must not stay silent
while the merged audio moves.

The range stops where `bsi` does. Past it are the audio blocks, where every bit
pattern is legal: a flip there changes the audio without making the stream
malformed, no decoder can report it, and counting those would measure nothing.
"""

from __future__ import annotations

import hashlib
import json
import os
import random
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402
from replay_fuzz import judge, run  # noqa: E402

BIN = find_oadec()
POLY = 0x8005
TABLE = []
for _i in range(256):
    _v = _i << 8
    for _ in range(8):
        _v = ((_v << 1) ^ POLY) & 0xFFFF if _v & 0x8000 else (_v << 1) & 0xFFFF
    TABLE.append(_v)

# `bsi` starts after syncword(16) + strmtyp(2) + substreamid(3) + frmsiz(11) +
# fscod(2) + numblkscod(2) + acmod(3) + lfeon(1) + bsid(5), and `eac3-blocks`
# prints where it ends -- 79 for the dependent frames of the test stream. Past
# that is `audfrm` and the audio blocks, where every bit pattern is legal and a
# flip changes the audio without making the stream malformed: no decoder can
# report that, so including it would measure nothing.
BSI_FROM, BSI_TO = 40, 80


def crc16(data: bytes) -> int:
    c = 0
    for b in data:
        c = ((c << 8) ^ TABLE[((c >> 8) ^ b) & 0xFF]) & 0xFFFF
    return c


def bits(buf, bit, n):
    v = 0
    for i in range(n):
        b = bit + i
        v = (v << 1) | ((buf[b >> 3] >> (7 - (b & 7))) & 1)
    return v


def frames(data: bytes):
    """(offset, size, is_dependent) for every syncframe."""
    pos = 0
    while pos + 6 < len(data):
        if data[pos] != 0x0B or data[pos + 1] != 0x77:
            pos += 1
            continue
        size = (bits(data, pos * 8 + 21, 11) + 1) * 2
        if size < 8 or pos + size > len(data):
            pos += 1
            continue
        yield pos, size, bits(data, pos * 8 + 16, 2) == 1
        pos += size


def main() -> int:
    if len(sys.argv) != 5:
        print("usage: fuzz_dependent_bsi.py STREAM SEED COUNT OUT.json", file=sys.stderr)
        return 2
    src, seed, n, out_path = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    data = bytearray(open(src, "rb").read())
    deps = [(o, s) for o, s, dep in frames(bytes(data)) if dep]
    if not deps:
        print(f"{src} has no dependent substream", file=sys.stderr)
        return 2
    print(f"{len(deps)} dependent frames")

    random.seed(seed)
    work = tempfile.mkdtemp(prefix="depfuzz-")
    mutated = os.path.join(work, "in" + os.path.splitext(src)[1])
    # the clean decodes, to tell "said nothing and changed nothing", which is
    # correct, from "said nothing and moved the audio", which is the failure
    clean = {}
    for name, args in (("decode_programme", []), ("decode_core_only", ["--core-only"])):
        out = os.path.join(work, f"clean-{name}.f32")
        run([BIN, "decode", src, "--format", "pcm", *args, "-o", out])
        clean[name] = hashlib.md5(open(out, "rb").read()).hexdigest() if os.path.exists(out) else None
        if os.path.exists(out):
            os.remove(out)
    trials = []
    for i in range(n):
        off, size = random.choice(deps)
        bit = random.randrange(BSI_FROM, min(BSI_TO, (size - 4) * 8))
        buf = bytearray(data)
        b = off * 8 + bit
        buf[b >> 3] ^= 0x80 >> (b & 7)
        c = crc16(bytes(buf[off + 2:off + size - 2]))
        buf[off + size - 2] = (c >> 8) & 0xFF
        buf[off + size - 1] = c & 0xFF
        open(mutated, "wb").write(bytes(buf))

        t = {"trial": i, "frame_offset": off, "bit_in_frame": bit}
        out = os.path.join(work, "out")
        for name, cmd in (("verify", [BIN, "verify", mutated]),
                          ("decode_programme", [BIN, "decode", mutated, "--format", "pcm",
                                                "-o", out + ".f32"]),
                          ("decode_core_only", [BIN, "decode", mutated, "--format", "pcm",
                                                "--core-only", "-o", out + ".core.f32"])):
            rc, text = run(cmd)
            said, first = judge(text)
            produced = cmd[-1]
            digest = (hashlib.md5(open(produced, "rb").read()).hexdigest()
                      if name != "verify" and os.path.exists(produced) else None)
            moved = digest is not None and clean.get(name) is not None and digest != clean[name]
            t[name] = {"exit": rc, "reported": said, "first": first,
                       "output_moved": moved,
                       "silent": rc == 0 and not said,
                       "silently_corrupted": rc == 0 and not said and moved,
                       "panic": first.startswith("PANIC")}
        trials.append(t)
        for leftover in os.listdir(work):
            if leftover.startswith("out"):
                os.remove(os.path.join(work, leftover))
        print(f"\r{i + 1}/{n}", end="", flush=True)
    print()
    shutil.rmtree(work, ignore_errors=True)

    arms = ("verify", "decode_programme", "decode_core_only")
    summary = {a: {"silent": sum(t[a]["silent"] for t in trials),
                   "nonzero_exit": sum(t[a]["exit"] != 0 for t in trials),
                   "reported_but_exit_0": sum(t[a]["reported"] and t[a]["exit"] == 0
                                              for t in trials),
                   "said_nothing_and_changed_nothing": sum(
                       t[a]["silent"] and not t[a].get("output_moved") for t in trials),
                   "silently_corrupted": sum(t[a].get("silently_corrupted", False)
                                             for t in trials),
                   "panics": sum(t[a]["panic"] for t in trials)} for a in arms}
    json.dump({"source": src, "seed": seed, "trials": n, "binary": BIN,
               "note": "one bit inside a dependent substream's bsi, with crc2 repaired",
               "summary": summary, "detail": trials}, open(out_path, "w"), indent=1)
    print(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
