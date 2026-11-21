#!/usr/bin/env python3
"""An independent model of the 64-band complex QMF bank of ETSI TS 103 420
V1.2.1 clause 7 and of what the decoder does to the downmix before the JOC
matrix, in float64 numpy.

Written from the text, not from the Rust:

- analysis (clause 7.2) and synthesis (clause 7.3) from the pseudocode and the
  matrix equations of the clauses, with the prototype window QWIN read from
  the table file ETSI ships with the specification (`ts_103420_tables.c`,
  array `prot64`, inside `ts_103420v010201p0.zip`), given with `--tables`;
- the operator on the lowest subband of a channel that carries the 90-degree
  phase shift of table 47 from the measurement record (evidence 49,
  `result.the_operator.taps_from_the_middle_outwards`), given with
  `--operator`. That operator is not in the standard; it was measured on the
  Dolby decoder's output, and the record is where it lives.

The synthesis of clause 7.3 is printed twice and the two disagree: the matrix
equation gives the phase `(j - 2n + 1/2)`, the pseudocode `(2j - 2n - 1)/2`.
`--synthesis-phase` picks one; the self-check says which one inverts the
analysis rather than taking it on trust.

    python tools/joc_qmf_reference.py --tables <ts_103420_tables.c|zip> --self-check
    python tools/joc_qmf_reference.py --tables <...> --compare-tap <dir> [--out <json>]

Exit codes: 0 every check held, 1 a check failed, 2 bad arguments.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import zipfile
from pathlib import Path

import numpy as np

N = 64  # subbands, and samples a time slot
LENGTH = 640  # prototype length (clause 7.4)
LOW_REACH = 10  # slots the operator on the lowest subband reaches either side

ROOT = Path(__file__).resolve().parent.parent
EVIDENCE_49 = ROOT / "docs/audit/evidence/engine-round-2026-11/49-the-operator-on-the-lowest-subband.json"


# ---------------------------------------------------------------------------
# inputs


def read_window(path: Path) -> np.ndarray:
    """`prot64` of the ETSI table file, from the .c or from the zip around it."""
    if path.suffix.lower() == ".zip":
        with zipfile.ZipFile(path) as z:
            name = next(n for n in z.namelist() if n.endswith(".c"))
            text = z.read(name).decode("latin-1")
    else:
        text = path.read_text(encoding="latin-1")
    m = re.search(r"const\s+float\s+prot64\s*\[\s*(\d+)\s*\]\s*=\s*\{(.*?)\};", text, re.S)
    if not m:
        raise SystemExit(f"{path}: no table prot64")
    values = re.findall(r"[+-]?\d+\.\d+e[+-]\d+", m.group(2))
    if int(m.group(1)) != LENGTH or len(values) != LENGTH:
        raise SystemExit(f"{path}: prot64 has {len(values)} values, {LENGTH} expected")
    return np.array([float(v) for v in values], dtype=np.float64)


def read_operator(path: Path) -> np.ndarray:
    """The 21 complex taps, `op[d + 10]` meeting the slot `d` from the middle."""
    d = json.loads(path.read_text(encoding="utf-8"))
    half = d["result"]["the_operator"]["taps_from_the_middle_outwards"]
    mid = np.array(half["real"]) + 1j * np.array(half["imaginary"])
    if len(mid) != LOW_REACH + 1:
        raise SystemExit(f"{path}: {len(mid)} taps from the middle, {LOW_REACH + 1} expected")
    return np.concatenate([mid[:0:-1], mid])


def operator_sum(path: Path) -> complex:
    re_, im = json.loads(path.read_text(encoding="utf-8"))["result"]["the_operator"]["the_sum_of_the_21"]
    return complex(re_, im)


# ---------------------------------------------------------------------------
# the bank


def analysis_matrix() -> np.ndarray:
    """Clause 7.2 step 5: M[k, j] = exp(i pi (k + 1/2)(j - 1/2) / n)."""
    k = np.arange(N)[:, None]
    j = np.arange(2 * N)[None, :]
    return np.exp(1j * np.pi * (k + 0.5) * (j - 0.5) / N)


def synthesis_matrix(phase: str) -> np.ndarray:
    """Clause 7.3 step 2, `[j, k]`, the 1/n included."""
    j = np.arange(2 * N)[:, None]
    k = np.arange(N)[None, :]
    if phase == "matrix":
        arg = np.pi * (k + 0.5) * (j - 2 * N + 0.5) / N
    elif phase == "pseudocode":
        arg = np.pi / (4 * N) * (2 * k + 1) * (2 * j - 2 * N - 1)
    else:
        raise ValueError(phase)
    return np.exp(1j * arg) / N


def analysis(x: np.ndarray, window: np.ndarray, block: int = 4096) -> np.ndarray:
    """`[slot, sb]` subband samples of a real signal whose length is a whole
    number of slots, from an empty state. Steps 1-4 of clause 7.2 make, for
    slot t, `qana_filt[j] = x[64 (t + 1) - 1 - j]` (zero before the start), so
    every slot is a reversed window of 640 samples ending at its own."""
    x = np.asarray(x, dtype=np.float64)
    slots = len(x) // N
    padded = np.concatenate([np.zeros(LENGTH - N), x[: slots * N]])
    m = analysis_matrix()
    out = np.empty((slots, N), dtype=np.complex128)
    for s0 in range(0, slots, block):
        s1 = min(slots, s0 + block)
        idx = (np.arange(s0, s1)[:, None] * N + LENGTH - 1) - np.arange(LENGTH)[None, :]
        z = padded[idx] * window[None, :]
        u = z.reshape(s1 - s0, LENGTH // (2 * N), 2 * N).sum(axis=1)
        out[s0:s1] = u @ m.T
    return out


def analysis_literal(x: np.ndarray, window: np.ndarray) -> np.ndarray:
    """Clause 7.2 to the letter, a loop a slot: the check of `analysis`."""
    buf = np.zeros(LENGTH)
    out = []
    for t in range(len(x) // N):
        pcm = x[t * N:(t + 1) * N]
        buf[N:] = buf[: LENGTH - N].copy()
        for j in range(N - 1, -1, -1):
            buf[j] = pcm[N - 1 - j]
        z = buf * window
        u = np.array([sum(z[j + k * 2 * N] for k in range(LENGTH // (2 * N))) for j in range(2 * N)])
        q = np.array([sum(u[j] * np.exp(1j * np.pi * (sb + 0.5) * (j - 0.5) / N) for j in range(2 * N))
                      for sb in range(N)])
        out.append(q)
    return np.array(out)


def synthesis(q: np.ndarray, window: np.ndarray, phase: str = "matrix") -> np.ndarray:
    """Samples of `[slot, sb]` subband samples, from an empty state (clause 7.3)."""
    nmat = synthesis_matrix(phase)
    state = np.zeros(2 * LENGTH)
    out = np.empty(len(q) * N)
    blocks = LENGTH // (2 * N)
    for t, slot in enumerate(q):
        state[2 * N:] = state[: 2 * LENGTH - 2 * N].copy()
        state[: 2 * N] = np.real(nmat @ slot)
        g = np.empty(LENGTH)
        for j in range(blocks):
            g[2 * N * j: 2 * N * j + N] = state[4 * N * j: 4 * N * j + N]
            g[2 * N * j + N: 2 * N * (j + 1)] = state[4 * N * j + 3 * N: 4 * N * j + 4 * N]
        w = g * window
        out[t * N:(t + 1) * N] = w.reshape(LENGTH // N, N).sum(axis=0)
    return out


# ---------------------------------------------------------------------------
# what comes before the matrix


def quadrature(q: np.ndarray, shifted: bool, operator: np.ndarray, plus: bool = False) -> np.ndarray:
    """One downmix channel `[slot, sb]` as the matrix meets it: held back by the
    reach of the operator and, when the channel carries the phase shift, turned
    back by -j (+j when `plus`) above the lowest subband and put through the
    operator on it. Slot t of the output is slot t - 10 of the input."""
    slots = len(q)
    held = np.zeros_like(q)
    if slots > LOW_REACH:
        held[LOW_REACH:] = q[: slots - LOW_REACH]
    if not shifted:
        return held
    turn = 1j if plus else -1j
    out = held * turn
    op = np.conj(operator) if plus else operator
    low = np.concatenate([np.zeros(2 * LOW_REACH, dtype=np.complex128), q[:, 0]])
    # slot t of the output is centred on input slot t - 10
    for t in range(slots):
        window = low[t: t + 2 * LOW_REACH + 1]  # input slots t-20 .. t
        out[t, 0] = np.dot(op, window)  # the operator is symmetric in d
    return out


DECODER_DELAY = 256  # samples the core decoder emits before the first frame's audio
FRAME = 1536
CLIP_RAMP = 256
TAP_FLOOR_DB = 120.0
# slots before every input of the comparison is known: the dump drops the
# decoder delay of the core (256 samples), the analysis window reaches 640
# samples back and the low-band operator 20 slots further
FIRST_SLOT = 40


def read_tap(directory: Path) -> dict:
    """A dump of `examples/joc_tap.rs`, its planes as memory maps."""
    index = json.loads((directory / "index.json").read_text(encoding="utf-8"))
    slots = len(index["slots"])
    ch, obj = index["channels"], index["objects"]
    def plane(name, shape, dtype=np.float64):
        return np.memmap(directory / name, dtype="<f8", mode="r").reshape(shape)
    tap = {
        "index": index,
        "downmix": plane("downmix.f64", (slots, ch, N, 2)),
        "matrix": plane("matrix.f64", (slots, obj, ch, N)),
        "objects": plane("objects.f64", (slots, obj, N, 2)),
        "pcm_objects": plane("pcm_objects.f64", (-1, obj)),
        "pcm_core": plane("pcm_core.f64", (-1, len(index["core_channels"]))),
    }
    return tap


def complex_plane(p: np.ndarray) -> np.ndarray:
    return p[..., 0] + 1j * p[..., 1]


def clip_gain_curve(index: dict, samples: int) -> np.ndarray:
    """The gain the decoder puts on the core before the analysis, by sample of
    the decode (decoder delay included): from the gain of the frame before to
    the frame's own over its first block, a straight line."""
    gain = np.ones(samples)
    frames = {}
    for frame, ts, _seq, _reset, flush, before, after in index["slots"]:
        if not flush and ts == 0:
            frames[frame] = (before, after)
    ramp = np.arange(FRAME) / CLIP_RAMP
    for f, (before, after) in frames.items():
        at = f * FRAME
        if at >= samples:
            break
        g = np.where(np.arange(FRAME) < CLIP_RAMP, before + (after - before) * ramp, after)
        gain[at:at + FRAME] = g[: max(0, min(FRAME, samples - at))]
    return gain


def compare_tap(directory: Path, window: np.ndarray, operator: np.ndarray) -> dict:
    """Holds a dump of `examples/joc_tap.rs` to this model, stage by stage, and
    the whole chain: each must agree to TAP_FLOOR_DB or better."""
    tap = read_tap(directory)
    index = tap["index"]
    slots = len(index["slots"])
    first = FIRST_SLOT
    rows = []

    def row(stage, ref, test, **extra):
        db = snr_db(np.asarray(ref), np.asarray(test))
        rows.append({"stage": stage, "db": round(db, 2) if np.isfinite(db) else "identical",
                     "ok": bool(db >= TAP_FLOOR_DB), **extra})

    # 1. the downmix: the core, its clip gain, the analysis and the quadrature
    core = np.asarray(tap["pcm_core"])
    total = slots * N
    gain = clip_gain_curve(index, total)
    downmix_tap = complex_plane(np.asarray(tap["downmix"]))
    downmix_model = np.empty_like(downmix_tap)
    for k, coded in enumerate(index["inputs"]):
        x = np.zeros(total)
        x[DECODER_DELAY:DECODER_DELAY + len(core)] = core[: total - DECODER_DELAY, coded]
        q = analysis(x * gain, window)
        carry = index["carry"][k]
        shifted = carry.startswith("Shifted")
        if carry.startswith(("Flat", "FlatAbove")):
            raise SystemExit(f"channel {k}: carry {carry} is not modelled")
        plus = "plus: true" in carry
        downmix_model[:, k] = quadrature(q, shifted, operator, plus)
    for k in range(index["channels"]):
        row(f"downmix ch {k} ({index['carry'][k]})", downmix_model[first:, k], downmix_tap[first:, k])
    row("downmix, every channel", downmix_model[first:], downmix_tap[first:])

    # 2. the matrix: the objects are the tap's matrix over the tap's downmix
    matrix = np.asarray(tap["matrix"])
    objects_tap = complex_plane(np.asarray(tap["objects"]))
    objects_model = np.einsum("tocs,tcs->tos", matrix, downmix_tap)
    row("objects = matrix x downmix", objects_model, objects_tap)

    # 3. the synthesis: the tap's object subband samples give the objects
    delay = index["object_delay"]
    pcm = np.asarray(tap["pcm_objects"])
    n = len(pcm)
    for o in range(index["objects"]):
        y = synthesis(objects_tap[:, o], window)[delay:delay + n]
        row(f"synthesis object {o}", pcm[: len(y), o], y)

    # 4. the chain: the model's downmix, the tap's matrix, the model's synthesis
    chain = np.einsum("tocs,tcs->tos", matrix, downmix_model)
    keep = slice(first * N + LENGTH, n)  # past the slots the model cannot know
    worst = np.inf
    for o in range(index["objects"]):
        y = synthesis(chain[:, o], window)[delay:delay + n]
        ref = pcm[keep, o]
        if np.sum(ref ** 2) > 0:
            worst = min(worst, snr_db(ref, y[keep]))
    rows.append({"stage": "chain: model downmix, tap matrix, model synthesis, worst object",
                 "db": round(worst, 2), "ok": bool(worst >= TAP_FLOOR_DB)})
    return {"dump": index["what"], "slots": slots, "first_slot_compared": first,
            "floor_db": TAP_FLOOR_DB, "ok": all(r["ok"] for r in rows), "rows": rows}


# ---------------------------------------------------------------------------
# self-check


def snr_db(ref: np.ndarray, test: np.ndarray) -> float:
    err = np.sum(np.abs(ref - test) ** 2)
    sig = np.sum(np.abs(ref) ** 2)
    return float("inf") if err == 0 else float(10 * np.log10(sig / err))


def best_delay(x: np.ndarray, y: np.ndarray, most: int = 2 * LENGTH) -> int:
    """The delay of y against x, in whole samples."""
    scores = [np.dot(x[: len(x) - d], y[d: len(x)]) for d in range(most)]
    return int(np.argmax(np.abs(scores)))


def self_check(window: np.ndarray, op_path: Path) -> dict:
    rng = np.random.default_rng(20261121)
    checks = []

    def check(name, ok, **values):
        checks.append({"check": name, "ok": bool(ok), **values})

    check("window: 640 taps, first zero", len(window) == LENGTH and window[0] == 0.0,
          taps=len(window), first=float(window[0]))

    # the vectorised analysis is the clause to the letter
    x = rng.standard_normal(12 * N)
    a, b = analysis(x, window), analysis_literal(x, window)
    check("analysis equals the clause's loop", np.max(np.abs(a - b)) < 1e-12,
          most_abs_difference=float(np.max(np.abs(a - b))))

    # analysis then synthesis: a delay of whole samples and a level
    x = rng.standard_normal(400 * N)
    q = analysis(x, window)
    result = {}
    for phase in ("matrix", "pseudocode"):
        y = synthesis(q, window, phase)
        d = best_delay(x, y)
        keep = slice(LENGTH * 2, len(x) - d - LENGTH)
        ref, got = x[keep], y[keep.start + d: keep.stop + d]
        gain = float(np.dot(ref, got) / np.dot(ref, ref))
        result[phase] = {"delay": d, "snr_db": round(snr_db(ref, got), 2),
                         "snr_after_gain_db": round(snr_db(ref, got / gain), 2), "gain": gain}
    check("the matrix phase inverts the analysis (near-PR, >= 75 dB)",
          result["matrix"]["snr_db"] >= 75.0, **result["matrix"])
    check("the delay of the pair is 640 - 64 + 1 samples", result["matrix"]["delay"] == LENGTH - N + 1,
          delay=result["matrix"]["delay"])
    check("the pseudocode phase does not invert it", result["pseudocode"]["snr_db"] < 20.0,
          **result["pseudocode"])

    # a tone in the middle of band k lands in band k
    worst = np.inf
    for k in (0, 1, 7, 31, 47, 63):
        f = (k + 0.5) / (2 * N)  # cycles a sample: band k spans k/128 .. (k+1)/128
        t = np.arange(200 * N)
        qq = analysis(np.cos(2 * np.pi * f * t), window)[40:]
        energy = np.sum(np.abs(qq) ** 2, axis=0)
        others = np.delete(energy, [b for b in (k - 1, k, k + 1) if 0 <= b < N])
        worst = min(worst, 10 * np.log10(energy[k] / max(others.max(), 1e-300)))
    check("a tone in the middle of a band stays out of the bands beyond its neighbours by >= 60 dB",
          worst >= 60.0, least_db=round(float(worst), 1))

    # linearity
    x1, x2 = rng.standard_normal(50 * N), rng.standard_normal(50 * N)
    lin = np.max(np.abs(analysis(2 * x1 - 3 * x2, window) - (2 * analysis(x1, window) - 3 * analysis(x2, window))))
    check("the analysis is linear", lin < 1e-10, most_abs_difference=float(lin))

    # the operator, as the record has it
    op = read_operator(op_path)
    total = operator_sum(op_path)
    check("the operator: 21 taps, symmetric, summing to the record's sum",
          len(op) == 2 * LOW_REACH + 1 and np.allclose(op, op[::-1]) and abs(op.sum() - total) < 1e-8,
          taps=len(op), sum=[op.sum().real, op.sum().imag], record=[total.real, total.imag])

    # a channel without the shift is held back by the reach and nothing else
    qq = analysis(rng.standard_normal(60 * N), window)
    held = quadrature(qq, False, op)
    check("a plain channel is held back ten slots and not touched",
          np.array_equal(held[LOW_REACH:], qq[:-LOW_REACH]) and not held[:LOW_REACH].any())
    turned = quadrature(qq, True, op)
    check("a shifted channel is turned by -j above the lowest subband",
          np.allclose(turned[LOW_REACH:, 1:], -1j * qq[:-LOW_REACH, 1:]))
    # the lowest subband of a slot is the operator over the 21 slots round it
    t = 30
    expect = np.dot(op, qq[t - 2 * LOW_REACH: t + 1, 0])
    check("a shifted channel's lowest subband is the operator over the slots round it",
          abs(turned[t, 0] - expect) < 1e-12)

    return {"ok": all(c["ok"] for c in checks), "checks": checks}


def main() -> int:
    ap = argparse.ArgumentParser(description="Independent model of the JOC QMF bank (TS 103 420 clause 7).")
    ap.add_argument("--tables", type=Path, required=True,
                    help="ts_103420_tables.c of ETSI, or ts_103420v010201p0.zip")
    ap.add_argument("--operator", type=Path, default=EVIDENCE_49,
                    help="the measurement record of the operator (evidence 49)")
    ap.add_argument("--self-check", action="store_true")
    ap.add_argument("--compare-tap", type=Path)
    ap.add_argument("--out", type=Path, help="write the result as JSON")
    args = ap.parse_args()
    if not args.self_check and not args.compare_tap:
        ap.error("nothing to do: --self-check or --compare-tap")
    window = read_window(args.tables)
    if args.compare_tap:
        result = compare_tap(args.compare_tap, window, read_operator(args.operator))
        for r in result["rows"]:
            print(f"{'ok  ' if r['ok'] else 'FAIL'} {r['stage']}: {r['db']} dB")
        if args.out:
            args.out.write_text(json.dumps(result, indent=1), encoding="utf-8")
        return 0 if result["ok"] else 1
    result = self_check(window, args.operator)
    for c in result["checks"]:
        extra = {k: v for k, v in c.items() if k not in ("check", "ok")}
        print(f"{'ok  ' if c['ok'] else 'FAIL'} {c['check']}" + (f"  {extra}" if extra else ""))
    if args.out:
        args.out.write_text(json.dumps(result, indent=1), encoding="utf-8")
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
