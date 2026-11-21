#!/usr/bin/env python3
"""Independent reference for enhanced coupling (ATSC A/52:2018 clause E.3.5.5).

Written from the text of the standard, not from oadec's Rust code. The clause
numbers are A/52:2018's. ETSI TS 102 366 V1.2.1 clause E.2.5.5 prints the same
process except for two signs, which `text` selects (docs/eac3.md). It takes a
case file, runs clause
E.3.5.5.1 to E.3.5.5.4 over every block of the case, and writes the
transform coefficients of every coupled channel.

    python tools/ecpl_reference.py CASE.case -o CASE.ref32.bin [--f64] [--all]

Only numpy is needed. The reference is a measuring instrument for the
`--ecpl-spec` path; it says nothing about what Dolby's decoders do, and a
Dolby measurement says nothing about it.

Case file (text, UTF-8, one directive a line, `#` starts a comment)
--------------------------------------------------------------------

    ecplref 1
    window kbd5|table
        Table 6.33 prints the window to five decimals. `table` uses the
        printed values; `kbd5` the Kaiser-Bessel-derived window with
        alpha 5, which the reference checks rounds to every printed entry.
    text a52-2018|ts102366-v121
        Which printing of the two lines the editions disagree on:
        A/52:2018 (the default) `ampbnd *= 1 + 0.38 * chaos` and
        `angle += chaos * rand`; V1.2.1 `1 - 0,38 * chaos` and
        `angle += -chaos * rand`.
    ola-gain 2|1
        Clause E.3.5.5.1 step 3 prints the overlap-add without the factor 2
        that clause 6.9.4 step 6 carries; as printed the chain is 6.02 dB
        low. 2 is the corrected reading, 1 the printed one.
    random lcg SEED | random file PATH
        The de-correlation values of clause E.3.5.5.3 (see "Random values").
    mant lcg SEED SCALE
        The enhanced coupling mantissas: for every block with enhanced
        coupling, in order, for bin = start .. end-1, SCALE * lcg().
    block nfchans=N begin=B end=E bndstrc=S interp=0|1
        A block with enhanced coupling. B and E are ecpl_begin_subbnd and
        ecpl_end_subbnd; S is ecplbndstrc[0..22] as 22 characters 0/1 (the
        entries below max(9, B + 1) are ignored, as the syntax never sends
        them). The `ch` lines of the block follow, one per channel 0..N-1:
    ch C off
    ch C amp=a,a,... angle=g,g,... chaos=c,c,... trans=0|1
        One code per enhanced coupling band. The first coupled channel's
        angle and chaos are given and must be ignored (clause E.3.5.5.2/3).
    block none
        A block without enhanced coupling: its coefficients count as zero
        for its neighbours (clause E.3.5.5.1), nothing is written for it.
    end
        End of stream: the last block's successor is zero.
    keep I,J,...
        Optional: the indices (among the blocks with enhanced coupling,
        from 0) whose coefficients the output holds. `--all` ignores it.

Random values
-------------

`lcg` is the 64-bit generator both sides implement bit for bit:

    state = (state * 6364136223846793005 + 1442695040888963407) mod 2**64
    value = ((state >> 11) / 2**53) * 2 - 1          # uniform on [-1, 1)

For `random lcg SEED` the first 5 * 256 draws are rand_notrans[ch][bin]
(ch-major), drawn once. rand_trans[ch][bnd] is then drawn per block with
enhanced coupling, for every coupled channel after the first in ascending
order, for every band 0..necplbnd-1, whether or not that channel carries a
transient. `random file PATH` gives the same values as little-endian f64:
5 * 256 notrans values followed by the trans stream in that order.

Output
------

For every block with enhanced coupling (those `keep` names), in order, for
every coupled channel
in ascending order, the coefficients chmant[ch][start..end] as little-endian
f32 (f64 with --f64). `start` and `end` are ecplsubbndtab[B] and
ecplsubbndtab[E].

Choices the text leaves open, made here and named in docs/eac3.md:
- angle interpolation with a single band has no pair to interpolate between,
  and the pseudo-code reads `y` and `slope` before setting them: the band
  angle is held flat;
- the interpolation's bin index is relative to the start of the region
  (`bin = count` after the first band), as its first-band branch writes it.
"""

from __future__ import annotations

import argparse
import math
import struct
import sys
from pathlib import Path

import numpy as np

N = 512
MASK64 = (1 << 64) - 1
MAX_FBW = 5

# ecplsubbndtab (clause E.3.3, enhanced coupling sub-band table)
ECPLSUBBNDTAB = (13, 19, 25, 31, 37, 49, 61, 73, 85, 97, 109, 121, 133, 145,
                 157, 169, 181, 193, 205, 217, 229, 241, 253)
# ecplampexptab, ecplampmanttab (table E3.10); code 31 is minus infinity
ECPLAMPEXPTAB = (0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3,
                 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, None)
ECPLAMPMANTTAB = (0x20, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10,
                  0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17,
                  0x13, 0x10, 0x1b, 0x17, 0x13, 0x10, 0x1b, 0x17, 0x13, 0x10,
                  0x1b, 0x17, 0x00)
# ecplangletab (table E3.11): codes 0..31 are k/32, 32..63 are (k-64)/32
ECPLANGLETAB = tuple(k / 32 if k < 32 else (k - 64) / 32 for k in range(64))
# ecplchaostab (table E3.12), as printed
ECPLCHAOSTAB = (0.000000, -0.142857, -0.285714, -0.428571, -0.571429,
                -0.714286, -0.857143, -1.000000)
# Table 6.33, w[0..255] as printed
WINDOW_TABLE = (
    0.00014, 0.00024, 0.00037, 0.00051, 0.00067, 0.00086, 0.00107, 0.00130,
    0.00157, 0.00187, 0.00220, 0.00256, 0.00297, 0.00341, 0.00390, 0.00443,
    0.00501, 0.00564, 0.00632, 0.00706, 0.00785, 0.00871, 0.00962, 0.01061,
    0.01166, 0.01279, 0.01399, 0.01526, 0.01662, 0.01806, 0.01959, 0.02121,
    0.02292, 0.02472, 0.02662, 0.02863, 0.03073, 0.03294, 0.03527, 0.03770,
    0.04025, 0.04292, 0.04571, 0.04862, 0.05165, 0.05481, 0.05810, 0.06153,
    0.06508, 0.06878, 0.07261, 0.07658, 0.08069, 0.08495, 0.08935, 0.09389,
    0.09859, 0.10343, 0.10842, 0.11356, 0.11885, 0.12429, 0.12988, 0.13563,
    0.14152, 0.14757, 0.15376, 0.16011, 0.16661, 0.17325, 0.18005, 0.18699,
    0.19407, 0.20130, 0.20867, 0.21618, 0.22382, 0.23161, 0.23952, 0.24757,
    0.25574, 0.26404, 0.27246, 0.28100, 0.28965, 0.29841, 0.30729, 0.31626,
    0.32533, 0.33450, 0.34376, 0.35311, 0.36253, 0.37204, 0.38161, 0.39126,
    0.40096, 0.41072, 0.42054, 0.43040, 0.44030, 0.45023, 0.46020, 0.47019,
    0.48020, 0.49022, 0.50025, 0.51028, 0.52031, 0.53033, 0.54033, 0.55031,
    0.56026, 0.57019, 0.58007, 0.58991, 0.59970, 0.60944, 0.61912, 0.62873,
    0.63827, 0.64774, 0.65713, 0.66643, 0.67564, 0.68476, 0.69377, 0.70269,
    0.71150, 0.72019, 0.72877, 0.73723, 0.74557, 0.75378, 0.76186, 0.76981,
    0.77762, 0.78530, 0.79283, 0.80022, 0.80747, 0.81457, 0.82151, 0.82831,
    0.83496, 0.84145, 0.84779, 0.85398, 0.86001, 0.86588, 0.87160, 0.87716,
    0.88257, 0.88782, 0.89291, 0.89785, 0.90264, 0.90728, 0.91176, 0.91610,
    0.92028, 0.92432, 0.92822, 0.93197, 0.93558, 0.93906, 0.94240, 0.94560,
    0.94867, 0.95162, 0.95444, 0.95713, 0.95971, 0.96217, 0.96451, 0.96674,
    0.96887, 0.97089, 0.97281, 0.97463, 0.97635, 0.97799, 0.97953, 0.98099,
    0.98236, 0.98366, 0.98488, 0.98602, 0.98710, 0.98811, 0.98905, 0.98994,
    0.99076, 0.99153, 0.99225, 0.99291, 0.99353, 0.99411, 0.99464, 0.99513,
    0.99558, 0.99600, 0.99639, 0.99674, 0.99706, 0.99736, 0.99763, 0.99788,
    0.99811, 0.99831, 0.99850, 0.99867, 0.99882, 0.99895, 0.99908, 0.99919,
    0.99929, 0.99938, 0.99946, 0.99953, 0.99959, 0.99965, 0.99969, 0.99974,
    0.99978, 0.99981, 0.99984, 0.99986, 0.99988, 0.99990, 0.99992, 0.99993,
    0.99994, 0.99995, 0.99996, 0.99997, 0.99998, 0.99998, 0.99998, 0.99999,
    0.99999, 0.99999, 0.99999, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000,
    1.00000, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000, 1.00000,
)


class Lcg:
    """The generator of the case format (module documentation)."""

    def __init__(self, seed: int) -> None:
        self.state = seed & MASK64

    def next(self) -> float:
        self.state = (self.state * 6364136223846793005 + 1442695040888963407) & MASK64
        return ((self.state >> 11) / 2.0**53) * 2.0 - 1.0


def kbd5() -> np.ndarray:
    """Kaiser-Bessel-derived window, alpha 5, the first 256 of 512."""
    m = N // 2
    j = np.arange(m + 1)
    kaiser = np.i0(math.pi * 5.0 * np.sqrt(1.0 - (2.0 * j / m - 1.0) ** 2))
    cum = np.cumsum(kaiser)
    return np.sqrt(cum[:m] / cum[m])


def window(kind: str) -> np.ndarray:
    table = np.array(WINDOW_TABLE, dtype=np.float64)
    if kind == "table":
        return table
    w = kbd5()
    worst = np.max(np.abs(np.round(w, 5) - table))
    if worst > 1e-12:
        raise SystemExit(f"kbd5 does not round to table 6.33 (worst {worst:.2e})")
    return w


def imdct_windowed(x: np.ndarray, w: np.ndarray) -> np.ndarray:
    """Steps 1 to 5 of clause 6.9.4.1: 256 coefficients to 512 windowed
    samples, as the pseudo-code writes them."""
    k = np.arange(N // 4)
    xcos1 = -np.cos(2 * math.pi * (8 * k + 1) / (8 * N))
    xsin1 = -np.sin(2 * math.pi * (8 * k + 1) / (8 * N))
    # step 2
    a = x[N // 2 - 2 * k - 1]
    b = x[2 * k]
    zk = (a * xcos1 - b * xsin1) + 1j * (b * xcos1 + a * xsin1)
    # step 3: z[n] = sum_k Z[k] (cos(8 pi k n / N) + j sin(8 pi k n / N))
    n = np.arange(N // 4)
    arg = 8 * math.pi * np.outer(n, k) / N
    z = (np.cos(arg) + 1j * np.sin(arg)) @ zk
    # step 4
    zr, zi = z.real, z.imag
    y = (zr * xcos1 - zi * xsin1) + 1j * (zi * xcos1 + zr * xsin1)
    yr, yi = y.real, y.imag
    # step 5
    out = np.zeros(N)
    for n in range(N // 8):
        out[2 * n] = -yi[N // 8 + n] * w[2 * n]
        out[2 * n + 1] = yr[N // 8 - n - 1] * w[2 * n + 1]
        out[N // 4 + 2 * n] = -yr[n] * w[N // 4 + 2 * n]
        out[N // 4 + 2 * n + 1] = yi[N // 4 - n - 1] * w[N // 4 + 2 * n + 1]
        out[N // 2 + 2 * n] = -yr[N // 8 + n] * w[N // 2 - 2 * n - 1]
        out[N // 2 + 2 * n + 1] = yi[N // 8 - n - 1] * w[N // 2 - 2 * n - 2]
        out[3 * N // 4 + 2 * n] = yi[n] * w[N // 4 - 2 * n - 1]
        out[3 * N // 4 + 2 * n + 1] = -yr[N // 4 - n - 1] * w[N // 4 - 2 * n - 2]
    return out


def carrier(prev: np.ndarray, curr: np.ndarray, nxt: np.ndarray, w: np.ndarray,
            gain: float) -> np.ndarray:
    """Clause E.3.5.5.1 steps 2 to 5: the complex carrier Z[k], k < N."""
    xp, xc, xn = (imdct_windowed(v, w) for v in (prev, curr, nxt))
    pcm = np.zeros(N)
    for n in range(N // 2):
        pcm[n] = gain * (xp[n + N // 2] + xc[n])
        pcm[n + N // 2] = gain * (xc[n + N // 2] + xn[n])
    n = np.arange(N)
    xcos3 = np.cos(math.pi * n / N)
    xsin3 = -np.sin(math.pi * n / N)
    wn = np.concatenate([w[: N // 2], w[N // 2 - 1 - np.arange(N // 2)]])
    re = pcm * wn * xcos3
    im = pcm * wn * xsin3
    arg = 2 * math.pi * np.outer(n, n) / N
    # Z[k] = 1/N sum_n (re + j im)(cos(2 pi k n / N) - j sin(2 pi k n / N))
    return ((np.cos(arg) - 1j * np.sin(arg)) @ (re + 1j * im)) / N


def ampbnd(code: int) -> float:
    if code == 31:
        return 0.0
    return (ECPLAMPMANTTAB[code] / 32) / 2 ** ECPLAMPEXPTAB[code]


def interpolate(angle_bnd: list[float], nbins_per_bnd: list[int]) -> list[float]:
    """Clause E.3.5.5.3, ecplangleintrp == 1, as the pseudo-code walks it.
    The bin index is relative to the start of the region; one band is
    held flat (module documentation)."""
    nbands = len(angle_bnd)
    total = sum(nbins_per_bnd)
    out = [math.nan] * total
    if nbands < 2:
        return [angle_bnd[0]] * total

    def wrap(v: float) -> float:
        while v > 1.0:
            v -= 2.0
        while v < -1.0:
            v += 2.0
        return v

    bin_ = 0
    y = 0.0
    slope = 0.0
    nbins_curr = nbins_per_bnd[0]
    for bnd in range(1, nbands):
        nbins_prev = nbins_per_bnd[bnd - 1]
        nbins_curr = nbins_per_bnd[bnd]
        angle_prev = angle_bnd[bnd - 1]
        angle_curr = angle_bnd[bnd]
        while (angle_curr - angle_prev) > 1.0:
            angle_curr -= 2.0
        while (angle_prev - angle_curr) > 1.0:
            angle_curr += 2.0
        slope = (angle_curr - angle_prev) / ((nbins_curr + nbins_prev) / 2.0)
        if bnd == 1 and nbins_prev > 1:
            if nbins_prev % 2 == 0:
                y = angle_prev - slope / 2
                bin_ = nbins_prev // 2 - 1
            else:
                y = angle_prev - slope
                bin_ = (nbins_prev - 3) // 2
            count = bin_ + 1
            for _ in range(count):
                out[bin_] = wrap(y)
                bin_ -= 1
                y -= slope
            bin_ = count
        if nbins_prev % 2 == 0:
            y = angle_prev + slope / 2
            count = nbins_curr // 2 + nbins_prev // 2
        else:
            y = angle_prev
            count = nbins_curr // 2 + (nbins_prev + 1) // 2
        for _ in range(count):
            out[bin_] = wrap(y)
            bin_ += 1
            y += slope
    count = nbins_curr // 2 if nbins_curr % 2 == 0 else nbins_curr // 2 + 1
    for _ in range(count):
        out[bin_] = wrap(y)
        bin_ += 1
        y += slope
    if any(math.isnan(v) for v in out) or bin_ != total:
        raise SystemExit("the interpolation did not cover the region exactly")
    return out


class Block:
    def __init__(self) -> None:
        self.ecpl = False
        self.begin = 0
        self.end = 0
        self.bndstrc = [0] * 22
        self.interp = False
        self.chans: list[dict | None] = []
        self.mant = np.zeros(N // 2)

    @property
    def start(self) -> int:
        return ECPLSUBBNDTAB[self.begin]

    @property
    def stop(self) -> int:
        return ECPLSUBBNDTAB[self.end]

    def strc(self, sbnd: int) -> int:
        """ecplbndstrc as the syntax delivers it: zero below max(9, begin+1)."""
        return self.bndstrc[sbnd] if sbnd >= max(9, self.begin + 1) else 0

    def first(self) -> int:
        return next(ch for ch, c in enumerate(self.chans) if c is not None)


class Case:
    def __init__(self, path: Path) -> None:
        self.window = "kbd5"
        self.gain = 2.0
        self.text = "a52-2018"
        self.random: tuple[str, str] | None = None
        self.mant: tuple[int, float] | None = None
        self.blocks: list[Block] = []
        self.keep: list[int] | None = None
        self.ended = False
        self.parse(path)

    def parse(self, path: Path) -> None:
        lines = path.read_text(encoding="utf-8").splitlines()
        cur: Block | None = None
        for no, raw in enumerate(lines, 1):
            line = raw.split("#", 1)[0].strip()
            if not line:
                continue
            words = line.split()
            head = words[0]
            try:
                if head == "ecplref":
                    if words[1] != "1":
                        raise ValueError("unknown version")
                elif head == "window":
                    if words[1] not in ("kbd5", "table"):
                        raise ValueError("window kbd5|table")
                    self.window = words[1]
                elif head == "text":
                    if words[1] not in ("a52-2018", "ts102366-v121"):
                        raise ValueError("text a52-2018|ts102366-v121")
                    self.text = words[1]
                elif head == "ola-gain":
                    self.gain = float(words[1])
                elif head == "random":
                    self.random = (words[1], words[2])
                elif head == "mant":
                    self.mant = (int(words[2], 0), float(words[3]))
                elif head == "keep":
                    self.keep = [int(v) for v in words[1].split(",")]
                elif head == "block":
                    cur = Block()
                    self.blocks.append(cur)
                    if words[1] == "none":
                        cur = None
                        continue
                    kv = dict(w.split("=", 1) for w in words[1:])
                    cur.ecpl = True
                    cur.begin = int(kv["begin"])
                    cur.end = int(kv["end"])
                    cur.bndstrc = [int(c) for c in kv["bndstrc"]]
                    cur.interp = kv["interp"] == "1"
                    cur.chans = [None] * int(kv["nfchans"])
                    if len(cur.bndstrc) != 22 or not 0 <= cur.begin < cur.end <= 22:
                        raise ValueError("bad region")
                elif head == "ch":
                    if cur is None:
                        raise ValueError("ch outside a block")
                    ch = int(words[1])
                    if words[2] == "off":
                        cur.chans[ch] = None
                        continue
                    kv = dict(w.split("=", 1) for w in words[2:])
                    cur.chans[ch] = {
                        "amp": [int(v) for v in kv["amp"].split(",")],
                        "angle": [int(v) for v in kv["angle"].split(",")],
                        "chaos": [int(v) for v in kv["chaos"].split(",")],
                        "trans": kv["trans"] == "1",
                    }
                elif head == "end":
                    self.ended = True
                else:
                    raise ValueError(f"unknown directive {head}")
            except (ValueError, IndexError, KeyError) as e:
                raise SystemExit(f"{path}:{no}: {e}") from e
        if self.mant is None or self.random is None or not self.ended:
            raise SystemExit(f"{path}: needs mant, random and end")
        seed, scale = self.mant
        gen = Lcg(seed)
        for b in self.blocks:
            if b.ecpl:
                for k in range(b.start, b.stop):
                    b.mant[k] = scale * gen.next()

    def draws(self) -> tuple[np.ndarray, "Trans"]:
        kind, arg = self.random  # type: ignore[misc]
        if kind == "lcg":
            gen = Lcg(int(arg, 0))
            notrans = np.array([[gen.next() for _ in range(N // 2)] for _ in range(MAX_FBW)])
            return notrans, Trans(gen.next)
        if kind == "file":
            data = Path(arg).read_bytes()
            vals = struct.unpack(f"<{len(data) // 8}d", data)
            notrans = np.array(vals[: MAX_FBW * (N // 2)]).reshape(MAX_FBW, N // 2)
            rest = iter(vals[MAX_FBW * (N // 2):])
            return notrans, Trans(lambda: next(rest))
        raise SystemExit("random lcg SEED | random file PATH")


class Trans:
    def __init__(self, draw) -> None:  # noqa: ANN001
        self.draw = draw


def run(case: Case) -> list[tuple[int, int, np.ndarray]]:
    """Every coupled channel of every block with enhanced coupling:
    (ecpl block index, channel, chmant[start..end])."""
    w = window(case.window)
    notrans, trans = case.draws()
    zero = np.zeros(N // 2)
    out = []
    eidx = 0
    blocks = case.blocks
    for i, b in enumerate(blocks):
        if not b.ecpl:
            continue
        prev = blocks[i - 1].mant if i > 0 and blocks[i - 1].ecpl else zero
        nxt = blocks[i + 1].mant if i + 1 < len(blocks) and blocks[i + 1].ecpl else zero
        z = carrier(prev, b.mant, nxt, w, case.gain)
        zr, zi = z.real, z.imag
        first = b.first()
        # band sizes (clause E.3.5.5.2)
        nbins_per_bnd: list[int] = []
        bnd_of_bin: dict[int, int] = {}
        necplbnds = -1
        for sbnd in range(b.begin, b.end):
            if b.strc(sbnd) == 0:
                necplbnds += 1
                nbins_per_bnd.append(0)
            for k in range(ECPLSUBBNDTAB[sbnd], ECPLSUBBNDTAB[sbnd + 1]):
                bnd_of_bin[k] = necplbnds
                nbins_per_bnd[necplbnds] += 1
        nbnd = necplbnds + 1
        kk = np.arange(N // 2)
        yk = np.cos(2 * math.pi * (N / 4 + 0.5) / N * (kk + 0.5))
        for ch, c in enumerate(b.chans):
            if c is None:
                continue
            if len(c["amp"]) != nbnd:
                raise SystemExit(f"block {i} channel {ch}: {len(c['amp'])} codes for {nbnd} bands")
            amp = [ampbnd(a) for a in c["amp"]]
            chaos = [0.0 if ch == first else ECPLCHAOSTAB[v] for v in c["chaos"]]
            if not c["trans"] and ch != first:
                # E.3.5.5.2: A/52:2018 `1 + 0.38 * chaos`, V1.2.1 `1 - 0,38 * chaos`
                sign = 1.0 if case.text == "a52-2018" else -1.0
                amp = [a * (1 + sign * 0.38 * h) for a, h in zip(amp, chaos)]
            angle_bnd = [0.0 if ch == first else ECPLANGLETAB[v] for v in c["angle"]]
            rand_trans = [trans.draw() for _ in range(nbnd)] if ch != first else [0.0] * nbnd
            if b.interp:
                rel = interpolate(angle_bnd, nbins_per_bnd)
            else:
                rel = [angle_bnd[bnd_of_bin[k]] for k in range(b.start, b.stop)]
            res = np.zeros(b.stop - b.start)
            for j, k in enumerate(range(b.start, b.stop)):
                bnd = bnd_of_bin[k]
                r = rand_trans[bnd] if c["trans"] else notrans[ch][k]
                # E.3.5.5.3: A/52:2018 `+= chaos * rand`, V1.2.1 `+= -chaos * rand`
                a = rel[j] + (chaos[bnd] if case.text == "a52-2018" else -chaos[bnd]) * r
                if a < -1.0:
                    a += 2.0
                elif a >= 1.0:
                    a -= 2.0
                g = amp[bnd]
                cr = zr[k] * g * math.cos(math.pi * a) - zi[k] * g * math.sin(math.pi * a)
                ci = zi[k] * g * math.cos(math.pi * a) + zr[k] * g * math.sin(math.pi * a)
                res[j] = -2 * (yk[k] * cr + yk[N // 2 - 1 - k] * ci)
            out.append((eidx, ch, res))
        eidx += 1
    return out


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Enhanced coupling reference (A/52 E.3.5.5)")
    ap.add_argument("case", help="case file (see the module documentation)")
    ap.add_argument("-o", "--output", required=True, help="coefficients out")
    ap.add_argument("--f64", action="store_true", help="write f64, not f32")
    ap.add_argument("--all", action="store_true", help="every block, ignoring `keep`")
    args = ap.parse_args(argv)
    case = Case(Path(args.case))
    rows = run(case)
    keep = None if args.all else case.keep
    fmt = "<d" if args.f64 else "<f"
    with open(args.output, "wb") as f:
        for eidx, _ch, res in rows:
            if keep is None or eidx in keep:
                f.write(np.asarray(res, dtype=fmt).tobytes())
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
