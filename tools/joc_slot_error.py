#!/usr/bin/env python3
"""Where the JOC objects of this decoder part from those of a reference
decoder (Dolby's), by object, parameter band and window of time slots, and
whether the matrix or the core is to blame.

The reference decoder writes only PCM, so the comparison is built round it:

1. Its objects and ours go through one Python analysis bank
   (`joc_qmf_reference.analysis`) onto the time slots of the tap
   (`examples/joc_tap.rs`). Only the difference of the two is analysed, so the
   near-PR aliasing of the bank cancels to first order; what is left of it,
   the leakage floor, is measured in every cell on our own objects: the tap's
   object subband samples against our objects taken through the same Python
   analysis.
2. Per cell (object, parameter band, window of W slots) the difference is
   fitted onto our downmix subband samples (the tap) by least squares: real
   coefficients over the stacked real and imaginary parts. What the fit
   explains is a matrix the reference applied and we did not (a matrix offset,
   a slot of timing); what it leaves, what no matrix makes of our downmix, is
   the core (dither, spectral extension, coupling).
3. (`--reference-core`, optional) the reference decoder's core decode is held
   to ours on the same grid and taken through our matrix: where step 2 says
   `core`, the two must agree within CROSS_DB or the cell is `inconsistent`.

How far a fit is to be trusted, per cell; the thresholds are those of
`tools/joc_slot_error_calibration.json`, fixed by `--calibrate` on our own
output with known faults injected, before the tool met a reference decode:

- the design's singular values (from its Gram matrix), its rank and
  kappa = sigma_max / sigma_min: good <= kappa_good < weak <= kappa_weak <
  unsolvable. A downmix column under the energy floor in the cell is left out
  of the design: it adds nothing to either decode there;
- `not_measured`: the reference object under energy_floor_dbfs or under
  leak_margin_db over the leakage floor, no column left, or fewer than
  rows_per_column real rows a column;
- `below_floor`: the difference under floor_margin_db over the leakage floor;
- the matrix part is significant when a 95 % interval of a coefficient by a
  block bootstrap over slots (block_slots) leaves out zero, the part is
  floor_margin_db over the floor and over chance_margin_db the share of the
  residual any random column takes;
- `matrix`: significant and the larger part; `core`: the residual is the
  larger part and over the floor; `unresolved`: an unsolvable design whose
  matrix part is significant;
- a heading whose `unresolved + not_measured + below_floor + inconsistent`
  cells hold more than unresolved_share of the difference energy is "the
  measurement could not resolve it".

    python tools/joc_slot_error.py --self-test
    python tools/joc_slot_error.py --calibrate --tap <dir> [--tap <dir> ...] --out <calibration.json>
    python tools/joc_slot_error.py --tap <dir> --reference <objects.f32> --out <cells.json>

Exit codes: 0 held, 1 a check failed or the input does not compare, 2 bad
arguments.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import joc_qmf_reference as qmf  # noqa: E402

N = qmf.N
RATE = 48000
CALIBRATION = Path(__file__).resolve().parent / "joc_slot_error_calibration.json"

# The starting values of the round 5 plan; the calibration replaces them.
DEFAULTS = {
    "window_slots": 24,
    "rank_rtol": 1e-6,
    "kappa_good": 100.0,
    "kappa_weak": 1000.0,
    "energy_floor_dbfs": -80.0,
    "column_range_db": 60.0,
    "reference_floor_db": 140.0,
    "leak_margin_db": 20.0,
    "rows_per_column": 8,
    "floor_margin_db": 10.0,
    "chance_margin_db": 6.0,
    "block_slots": 4,
    "bootstrap_draws": 200,
    "known_answer_db": 1.0,
    "cross_db": 3.0,
    "unresolved_share": 0.30,
}
# kept for the self-test and for readers of the stop 1 design
RANK_RTOL = DEFAULTS["rank_rtol"]
KAPPA_GOOD = DEFAULTS["kappa_good"]
KAPPA_WEAK = DEFAULTS["kappa_weak"]

CLASSES = ("matrix", "core", "unresolved", "not_measured", "below_floor", "inconsistent")
CONDITION = ("good", "weak", "unsolvable")


# ---------------------------------------------------------------------------
# one cell, by SVD: the reference the batched solver is held to


@dataclass
class Fit:
    """The least-squares fit of one cell."""

    coefficients: np.ndarray  # real, one a downmix column; NaN where not identifiable
    singular_values: np.ndarray
    rank: int
    kappa: float
    condition: str
    residual: np.ndarray  # complex, one a row
    prediction: np.ndarray  # complex, X @ coefficients


def condition_of(kappa: float, rank: int, cols: int, th: dict | None = None) -> str:
    th = th or DEFAULTS
    if rank < cols or kappa > th["kappa_weak"]:
        return "unsolvable"
    if kappa > th["kappa_good"]:
        return "weak"
    return "good"


def fit_cell(x: np.ndarray, y: np.ndarray, rtol: float = RANK_RTOL) -> Fit:
    """Real coefficients m minimising |X m - y| for complex X (rows x columns)
    and y (rows), real and imaginary parts stacked, solved by SVD with the
    numerical rank cut at sigma_max * rtol."""
    x = np.asarray(x)
    y = np.asarray(y)
    a = np.vstack([x.real, x.imag])
    b = np.concatenate([y.real, y.imag])
    u, s, vt = np.linalg.svd(a, full_matrices=False)
    keep = s > (s[0] * rtol if s.size else 0.0)
    rank = int(keep.sum())
    coef = vt[keep].T @ ((u[:, keep].T @ b) / s[keep]) if rank else np.zeros(a.shape[1])
    kappa = float(s[0] / s[-1]) if rank == a.shape[1] and s[-1] > 0 else float("inf")
    condition = condition_of(kappa, rank, a.shape[1])
    coef_out = np.full(a.shape[1], np.nan) if rank < a.shape[1] else coef
    prediction = x @ coef
    return Fit(coef_out, s, rank, kappa, condition, y - prediction, prediction)


# ---------------------------------------------------------------------------
# many cells at once: normal equations from Gram matrices


def solve_gram(g: np.ndarray, h: np.ndarray, rtol: float) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Batched minimum-norm solutions of g m = h for real symmetric g
    (..., k, k): the coefficients, the singular values of the design (the
    square roots of g's eigenvalues, largest first) and the rank."""
    lam, v = np.linalg.eigh(g)
    lam = np.clip(lam[..., ::-1], 0.0, None)
    v = v[..., ::-1]
    sigma = np.sqrt(lam)
    keep = sigma > sigma[..., :1] * rtol
    inv = np.where(keep, 1.0 / np.where(keep, lam, 1.0), 0.0)
    coef = np.einsum("...ij,...j,...kj,...k->...i", v, inv, v, h)
    return coef, sigma, keep.sum(axis=-1)


# ---------------------------------------------------------------------------
# inputs


def read_reference(path: Path, channels: int) -> np.ndarray:
    """Headerless little-endian f32, interleaved, `channels` a row."""
    v = np.fromfile(path, dtype="<f4")
    return v[: len(v) // channels * channels].reshape(-1, channels).astype(np.float64)


def match_objects(ours: np.ndarray, ref: np.ndarray, lag_most: int = 3000) -> tuple[list, int]:
    """Which reference channel is which of our objects, and how many samples
    the reference runs behind ours: on the loudest object, by correlation,
    then every object to the channel it is most alike."""
    n = min(len(ours), len(ref))
    loud = int(np.argmax((ours[:n] ** 2).mean(axis=0)))
    seg = slice(lag_most, min(n - lag_most, lag_most + 8 * RATE))
    best, lag = -np.inf, 0
    a = ours[seg, loud]
    for ch in range(ref.shape[1]):
        r = ref[:, ch]
        if not np.any(r[: n]):
            continue
        size = 1 << int(np.ceil(np.log2(len(a) + 2 * lag_most + 1)))
        b = r[seg.start - lag_most: seg.stop + lag_most]
        cc = np.fft.irfft(np.fft.rfft(b, size) * np.conj(np.fft.rfft(a, size)), size)[: 2 * lag_most + 1]
        k = int(np.argmax(np.abs(cc)))
        if abs(cc[k]) > best:
            best, lag = abs(cc[k]), k - lag_most
    shifted = ref[lag:] if lag >= 0 else np.vstack([np.zeros((-lag, ref.shape[1])), ref])
    n = min(len(ours), len(shifted))
    mapping = []
    used = set()
    for o in range(ours.shape[1]):
        x = ours[:n, o]
        ex = float(np.dot(x, x))
        if ex == 0:
            mapping.append(None)
            continue
        scores = []
        for ch in range(shifted.shape[1]):
            y = shifted[:n, ch]
            ey = float(np.dot(y, y))
            scores.append(abs(float(np.dot(x, y))) / np.sqrt(ex * ey) if ey > 0 and ch not in used else -1.0)
        ch = int(np.argmax(scores))
        if scores[ch] < 0.5:
            mapping.append(None)
            continue
        used.add(ch)
        mapping.append(ch)
    return mapping, lag


# ---------------------------------------------------------------------------
# the grid


class Grid:
    """One tap dump on the slot grid: our downmix, matrix and objects, our
    objects through the Python analysis, and the transform that takes any
    object PCM onto the same slots."""

    def __init__(self, directory: Path, window: np.ndarray):
        self.tap = qmf.read_tap(directory)
        ix = self.tap["index"]
        self.index = ix
        self.window = window
        self.slots = len(ix["slots"])
        self.channels = ix["channels"]
        self.objects = ix["objects"]
        self.delay = ix["object_delay"]
        self.downmix = qmf.complex_plane(np.asarray(self.tap["downmix"]))  # [t, ch, sb]
        self.pcm = np.asarray(self.tap["pcm_objects"])  # [sample, obj]
        self.q_ours = qmf.complex_plane(np.asarray(self.tap["objects"]))  # [t, obj, sb]
        self.shift, self.slot_lag = self._find_alignment()
        self.a_ours = self.transform(self.pcm)
        # What the plan called the leakage floor: our objects through the
        # synthesis and back through the analysis, against the tap. On real
        # content it is not a floor: where a matrix steps between bands or
        # slots, the object subband samples are none an analysis could make,
        # and the round trip does not give them back (`round_trip_kernel`).
        # Reported by band, not used as a floor.
        self.round_trip = np.abs(self.a_ours - self.q_ours) ** 2
        self.g = self._white_gain()

    def _analyse(self, pcm: np.ndarray, shift: int, lag: int, slots: int | None = None) -> np.ndarray:
        slots = slots or self.slots
        total = slots * N
        out = np.zeros((slots, pcm.shape[1], N), dtype=np.complex128)
        for o in range(pcm.shape[1]):
            s = np.zeros(total + N)
            n = max(0, min(len(pcm), total - self.delay))
            s[self.delay: self.delay + n] = pcm[:n, o]
            a = qmf.analysis(s[shift: shift + total], self.window)
            out[: slots - lag, o] = a[lag:]
        return out

    def _find_alignment(self) -> tuple[int, int]:
        """The sample shift and slot lag at which object PCM, written with the
        object delay of the dump, comes back onto the slots it was made of:
        found on subband samples an analysis made of noise, so that it is a
        property of the banks and the delay, not of the content."""
        slots = 200
        rng = np.random.default_rng(5)
        q = qmf.analysis(rng.standard_normal(slots * N), self.window)
        pcm = qmf.synthesis(q, self.window)[self.delay:][:, None]
        best = (-np.inf, 0, 0)
        for shift in range(N):
            a = self._analyse(pcm, shift, 0, slots)[:, 0]
            for lag in range(0, 16):
                db = qmf.snr_db(q[40: slots - 40], a[40 + lag: slots - 40 + lag])
                if db > best[0]:
                    best = (db, shift, lag)
        if best[0] < 70:
            raise SystemExit(f"no alignment of the banks: best {best[0]:.1f} dB")
        return best[1], best[2]

    def transform(self, pcm: np.ndarray) -> np.ndarray:
        """Object PCM aligned with ours onto the slots of the tap: [t, obj, sb]."""
        return self._analyse(pcm, self.shift, self.slot_lag)

    def round_trip_by_band(self) -> list:
        """dB of our object subband samples over what the round trip changes
        of them, by subband, over every object (slots 40 on)."""
        q = np.sum(np.abs(self.q_ours[40:]) ** 2, axis=(0, 1))
        d = np.sum(self.round_trip[40:], axis=(0, 1))
        with np.errstate(divide="ignore", invalid="ignore"):
            return np.round(10 * np.log10(q / d), 1).tolist()

    def _white_gain(self) -> float:
        """Mean |Q|^2 a subband of the analysis gives for white noise of unit
        variance: what turns a cell's energy into dB of full scale."""
        rng = np.random.default_rng(1)
        q = qmf.analysis(rng.standard_normal(400 * N), self.window)[20:]
        return float(np.mean(np.abs(q) ** 2))

    def matrix(self) -> np.ndarray:
        return np.asarray(self.tap["matrix"])  # [t, obj, ch, sb]

    def kernel(self) -> np.ndarray:
        if not hasattr(self, "_kernel"):
            self._kernel = round_trip_kernel(self)
        return self._kernel

    def band_edges(self) -> list[int]:
        """The parameter bands, as the subbands where the matrix of some object
        in some slot changes: the finest partition every payload of the clip
        agrees with."""
        m = self.matrix()
        change = np.zeros(N - 1, dtype=bool)
        step = max(1, self.slots // 2000)
        for t in range(0, self.slots, step):
            change |= np.any(m[t, :, :, 1:] != m[t, :, :, :-1], axis=(0, 1))
        return [0] + [int(i) + 1 for i in np.flatnonzero(change)] + [N]


# ---------------------------------------------------------------------------
# the cells


REACH_SLOTS = 7  # slots the synthesis-analysis round trip spreads a sample over, either way
REACH_BANDS = 2  # and subbands


def round_trip_kernel(grid: "Grid") -> np.ndarray:
    """What the synthesis followed by the analysis of the grid makes of one
    subband sample: `K[k', part, tau, dk]` is the output in subband k' + dk,
    tau - REACH_SLOTS slots later, of a unit real (part 0) or imaginary
    (part 1) sample in subband k'. The round trip keeps only the real part
    of what it is given, so it is real-linear, not complex-linear: arbitrary
    subband samples come back halved and spread, samples an analysis made
    come back whole."""
    t_len = 48
    at = 20
    k = np.zeros((N, 2, 2 * REACH_SLOTS + 1, 2 * REACH_BANDS + 1), dtype=np.complex128)
    for kp in range(N):
        for part, unit in enumerate((1.0, 1j)):
            q = np.zeros((t_len, N), dtype=np.complex128)
            q[at, kp] = unit
            y = qmf.synthesis(q, grid.window)
            s = np.zeros(t_len * N + N)
            s[: len(y) - grid.shift] = y[grid.shift:]
            a = qmf.analysis(s[: t_len * N], grid.window)
            a = np.concatenate([a[grid.slot_lag:], np.zeros((grid.slot_lag, N))])
            for dk in range(-REACH_BANDS, REACH_BANDS + 1):
                if 0 <= kp + dk < N:
                    k[kp, part, :, dk + REACH_BANDS] = a[at - REACH_SLOTS: at + REACH_SLOTS + 1, kp + dk]
    return k


def project(kernel: np.ndarray, q: np.ndarray, lo: int, hi: int) -> tuple[np.ndarray, int, int]:
    """The round trip of subband samples q [..., W, hi - lo] that are zero
    outside subbands lo..hi and outside the W slots: [..., W + 2 R, bands]
    over subbands blo..bhi, which are returned with it."""
    blo, bhi = max(0, lo - REACH_BANDS), min(N, hi + REACH_BANDS)
    w = q.shape[-2]
    out = np.zeros(q.shape[:-2] + (w + 2 * REACH_SLOTS, bhi - blo), dtype=np.complex128)
    parts = (q.real, q.imag)
    for i in range(hi - lo):
        kp = lo + i
        for part in (0, 1):
            src = parts[part][..., i]  # [..., W]
            if not np.any(src):
                continue
            for dk in range(-REACH_BANDS, REACH_BANDS + 1):
                kk = kp + dk
                if not blo <= kk < bhi:
                    continue
                resp = kernel[kp, part, :, dk + REACH_BANDS]
                for tau in range(2 * REACH_SLOTS + 1):
                    r = resp[tau]
                    if r == 0:
                        continue
                    out[..., tau: tau + w, kk - blo] += src * r
    return out, blo, bhi


CELL_DTYPE = [("obj", "i4"), ("band", "i4"), ("lo", "i4"), ("hi", "i4"), ("win", "i4"), ("cls", "U12"),
              ("res", "f8"), ("pred", "f8"), ("core", "f8"), ("leak", "f8"), ("ref", "f8"), ("kappa", "f8"),
              ("cols", "i4"), ("rows", "i4"), ("cond", "U10"), ("dme", "f8"), ("timing", "f8")]


def timing_signal(m_o: np.ndarray, x: np.ndarray) -> np.ndarray:
    """What our object would gain if its matrix came one slot late:
    sum_ch (M[t-1] - M[t]) X[t], [t, sb]."""
    prev = np.concatenate([m_o[:1], m_o[:-1]])
    return np.einsum("tcs,tcs->ts", prev - m_o, x)


def cells(grid: Grid, diff_q: np.ndarray, th: dict, valid: np.ndarray | None = None,
          rng: np.random.Generator | None = None) -> dict:
    """Every cell of a difference `diff_q` [t, obj, sb] (the reference less
    ours, through the transform), classified.

    The design of a cell is our downmix in the cell, one column a downmix
    channel, and one more column for the matrix a slot late (`timing_signal`),
    every column taken through the round trip (`project`); the fit runs over
    the cell widened by the reach of the round trip, and the energies of the
    classes are those of the cell's own rows. The matrix part of a cell is
    what the downmix and timing columns explain; `dme` is its energy before
    the round trip, the size of the matrix error itself. `valid` [obj] leaves
    out objects the reference does not carry."""
    rng = rng or np.random.default_rng(20261121)
    kernel = grid.kernel()
    w = int(th["window_slots"])
    b = int(th["block_slots"])
    r = REACH_SLOTS
    first = qmf.FIRST_SLOT
    windows = (grid.slots - first - 16 - r) // w
    edges = grid.band_edges()
    bands = list(zip(edges[:-1], edges[1:]))
    x = grid.downmix
    k = grid.channels
    g = grid.g
    m = grid.matrix()
    floor_e = 10 ** (th["energy_floor_dbfs"] / 10) * g
    span = w + 2 * r
    t0 = first
    widx = t0 + np.arange(windows)[:, None] * w + np.arange(span)[None, :] - r  # [win, span]
    inside = (widx >= 0) & (widx < grid.slots)
    widx = widx.clip(0, grid.slots - 1)
    nblk = int(np.ceil(span / b))
    pad = nblk * b - span
    draws = int(th["bootstrap_draws"])
    boot = rng.integers(0, nblk, size=(draws, nblk))
    sl = slice(t0, t0 + windows * w)
    refw = (np.abs(grid.q_ours[sl] + diff_q[sl]) ** 2).reshape(windows, w, grid.objects, N)
    dcore = diff_q[sl].reshape(windows, w, grid.objects, N)
    timing = {o: timing_signal(m[:, o], x) for o in range(grid.objects)}
    out = []

    def blocks(a):  # [..., span, nb] -> [..., nblk, b, nb]
        if pad:
            a = np.concatenate([a, np.zeros(a.shape[:-2] + (pad, a.shape[-1]), dtype=a.dtype)], axis=-2)
        return a.reshape(a.shape[:-2] + (nblk, b, a.shape[-1]))

    for bi, (lo, hi) in enumerate(bands):
        bw = hi - lo
        xs = x[sl, :, lo:hi].reshape(windows, w, k, bw).transpose(0, 2, 1, 3)  # [win, k, W, bw]
        p, blo, bhi = project(kernel, xs, lo, hi)  # [win, k, span, nb]
        nbands = bhi - blo
        pb = blocks(p)  # [win, k, nblk, b, nb]
        gxx = np.einsum("wcnsi,wdnsi->wncd", pb.conj(), pb).real
        rxx = np.einsum("wcsi,wdsi->wcd", xs.conj(), xs).real / (w * bw)
        crow = 2 * w * bw
        nrows = 2 * span * nbands
        for o in range(grid.objects):
            if valid is not None and not valid[o]:
                continue
            ts = timing[o][sl, lo:hi].reshape(windows, w, bw)[:, None]  # [win, 1, W, bw]
            pt, _, _ = project(kernel, ts, lo, hi)
            ptb = blocks(pt)
            # the Gram of downmix and timing columns, by block: [win, nblk, k+1, k+1]
            gfull = np.zeros((windows, nblk, k + 1, k + 1))
            gfull[:, :, :k, :k] = gxx
            gxt = np.einsum("wcnsi,wnsi->wnc", pb.conj(), ptb[:, 0]).real
            gfull[:, :, :k, k] = gxt
            gfull[:, :, k, :k] = gxt
            gfull[:, :, k, k] = np.einsum("wnsi->wn", np.abs(ptb[:, 0]) ** 2)
            rfull = np.zeros((windows, k + 1, k + 1))
            rfull[:, :k, :k] = rxx
            rxt = np.einsum("wcsi,wsi->wc", xs.conj(), ts[:, 0]).real / (w * bw)
            rfull[:, :k, k] = rxt
            rfull[:, k, :k] = rxt
            rfull[:, k, k] = np.einsum("wsi->w", np.abs(ts[:, 0]) ** 2) / (w * bw)
            col_e = np.einsum("wcc->wc", rfull)
            active = (col_e > floor_e) & (col_e > col_e.max(axis=1, keepdims=True) * 10 ** (-th["column_range_db"] / 10))
            y = np.where(inside[..., None], diff_q[widx, o, blo:bhi], 0)  # [win, span, nb]
            yb = blocks(y)
            hfull = np.zeros((windows, nblk, k + 1))
            hfull[:, :, :k] = np.einsum("wcnsi,wnsi->wnc", pb.conj(), yb).real
            hfull[:, :, k] = np.einsum("wnsi,wnsi->wn", ptb[:, 0].conj(), yb).real
            ref_e = refw[:, :, o, lo:hi].sum(axis=(1, 2)) / (w * bw)
            leak = ref_e * 10 ** (-th["reference_floor_db"] / 10)
            yc = dcore[:, :, o, lo:hi]
            res_e = np.sum(np.abs(yc) ** 2, axis=(1, 2)) / (w * bw)
            pcore = np.concatenate([p[:, :, r: r + w, lo - blo: hi - blo], pt[:, :, r: r + w, lo - blo: hi - blo]], axis=1)
            for pattern in np.unique(active, axis=0):
                idx = np.flatnonzero(np.all(active == pattern, axis=1))
                cols = np.flatnonzero(pattern)
                kk = len(cols)
                rec = np.zeros(len(idx), dtype=CELL_DTYPE)
                rec["obj"], rec["band"], rec["lo"], rec["hi"], rec["win"] = o, bi, lo, hi, idx
                rec["res"], rec["leak"], rec["ref"], rec["rows"] = res_e[idx], leak[idx], ref_e[idx], crow
                if kk == 0:
                    rec["cls"], rec["core"], rec["kappa"], rec["cond"] = "not_measured", res_e[idx], np.inf, "none"
                    out.append(rec)
                    continue
                G = gfull[idx][:, :, cols][:, :, :, cols]
                H = hfull[idx][:, :, cols]
                Gs, Hs = G.sum(axis=1), H.sum(axis=1)
                coef, sigma, rank = solve_gram(Gs, Hs, th["rank_rtol"])
                small = np.where(sigma[:, -1] > 0, sigma[:, -1], 1.0)
                kappa = np.where((rank == kk) & (sigma[:, -1] > 0), sigma[:, 0] / small, np.inf)
                pc = np.einsum("cksi,ck->csi", pcore[idx][:, cols], coef)
                pred_e = np.mean(np.abs(pc) ** 2, axis=(1, 2))
                e_e = np.mean(np.abs(yc[idx] - pc) ** 2, axis=(1, 2))
                gr = rfull[idx][:, cols][:, :, cols]
                dme = np.einsum("ci,cij,cj->c", coef, gr, coef)
                tcol = np.flatnonzero(cols == k)
                timing_e = coef[:, tcol[0]] ** 2 * rfull[idx, k, k] if len(tcol) else np.zeros(len(idx))
                cb, _, _ = solve_gram(G[:, boot].sum(axis=2), H[:, boot].sum(axis=2), th["rank_rtol"])
                lo_ci = np.percentile(cb, 2.5, axis=1)
                hi_ci = np.percentile(cb, 97.5, axis=1)
                ci_excl = np.any((lo_ci > 0) | (hi_ci < 0), axis=1)
                chance = e_e * kk / max(nrows - kk, 1)
                cond = condition_vec(kappa, rank, kk, th)
                rec["pred"], rec["core"], rec["kappa"], rec["cols"], rec["cond"] = pred_e, e_e, kappa, kk, cond
                rec["dme"], rec["timing"] = dme, timing_e
                rec["cls"] = classify_vec(ref_e[idx], res_e[idx], pred_e, e_e, leak[idx], chance, ci_excl, cond,
                                          crow, kk, th, g)
                out.append(rec)
    arr = np.concatenate(out) if out else np.zeros(0, dtype=CELL_DTYPE)
    return {"cells": arr, "window_slots": w, "first_slot": t0, "windows": windows, "bands": bands}


def condition_vec(kappa, rank, cols, th):
    return np.where((rank < cols) | (kappa > th["kappa_weak"]), "unsolvable",
                    np.where(kappa > th["kappa_good"], "weak", "good"))


def classify_vec(ref_e, res_e, pred_e, core_e, leak, chance, ci_excl, cond, rows, cols, th, g):
    """`classify_cell` over arrays."""
    db = lambda v: 10 * np.log10(np.maximum(v, 1e-300))  # noqa: E731
    ref_e, res_e, pred_e, core_e, leak, chance = map(np.asarray, (ref_e, res_e, pred_e, core_e, leak, chance))
    n = len(res_e)
    out = np.full(n, "below_floor", dtype="U12")
    over = db(leak) + th["floor_margin_db"]
    unsolvable = np.asarray(cond) == "unsolvable"
    significant = (db(pred_e) >= over) & (db(pred_e) >= db(chance) + th["chance_margin_db"]) & (ci_excl | unsolvable)
    core_over = db(core_e) >= over
    solv = ~unsolvable
    out[solv & core_over] = "core"
    out[solv & significant & ((pred_e >= core_e) | ~core_over)] = "matrix"
    out[unsolvable & core_over] = "core"
    out[unsolvable & significant] = "unresolved"
    out[db(res_e) < over] = "below_floor"
    nm = (db(ref_e / g) < th["energy_floor_dbfs"]) | (db(ref_e) < db(leak) + th["leak_margin_db"]) \
        | (np.broadcast_to(rows, (n,)) < th["rows_per_column"] * cols)
    out[nm] = "not_measured"
    return out


def classify_cell(ref_e, res_e, pred_e, core_e, leak, chance, ci_excl, cond, nrows, cols, th, g) -> str:
    """The class of one cell (energies are mean |Q|^2 a subband sample)."""
    db = lambda v: 10 * np.log10(max(v, 1e-300))  # noqa: E731
    if (db(ref_e / g) < th["energy_floor_dbfs"] or db(ref_e) < db(leak) + th["leak_margin_db"]
            or nrows < th["rows_per_column"] * cols):
        return "not_measured"
    over = db(leak) + th["floor_margin_db"]
    if db(res_e) < over:
        return "below_floor"
    significant = (db(pred_e) >= over and db(pred_e) >= db(chance) + th["chance_margin_db"]
                   and (ci_excl or cond == "unsolvable"))
    if cond == "unsolvable":
        if significant:
            return "unresolved"
        return "core" if db(core_e) >= over else "below_floor"
    if significant and pred_e >= core_e:
        return "matrix"
    if db(core_e) >= over:
        return "core"
    return "matrix" if significant else "below_floor"


# ---------------------------------------------------------------------------
# summaries


KHZ_BANDS = {"0-6 kHz": (0, 16), "6-12 kHz": (16, 32), "12-18 kHz": (32, 48), "18-24 kHz": (48, 64),
             "12 kHz band (32)": (32, 33), "18 kHz band (48)": (48, 49), "21 kHz band (56)": (56, 57)}


def shares(c: np.ndarray, th: dict) -> dict:
    """Energy of the difference by class, and the verdict the 30 % rule
    allows."""
    total = float(c["res"].sum())
    out = {"cells": int(len(c)), "difference_energy": total}
    by = {}
    for cls in CLASSES:
        m = c["cls"] == cls
        by[cls] = {"cells": int(m.sum()), "share": float(c["res"][m].sum() / total) if total > 0 else 0.0}
    out["by_class"] = by
    m = (c["cls"] == "matrix") | (c["cls"] == "core")
    explained_m = float(c["pred"][c["cls"] == "matrix"].sum())
    explained_c = float(c["core"][c["cls"] == "core"].sum())
    out["matrix_part_of_judged"] = explained_m / max(explained_m + explained_c, 1e-300)
    unresolved = 1.0 - (by["matrix"]["share"] + by["core"]["share"])
    out["unresolved_share"] = unresolved
    if total == 0 or not m.any():
        out["verdict"] = "not measured"
    elif unresolved > th["unresolved_share"]:
        out["verdict"] = "the measurement could not resolve it"
    else:
        out["verdict"] = "matrix" if out["matrix_part_of_judged"] >= 0.5 else "core"
    return out


def summarise(grid: Grid, result: dict, diff_q: np.ndarray, valid, th: dict) -> dict:
    c = result["cells"]
    out = {"whole": shares(c, th), "by_band": {}, "by_object": {}}
    for name, (lo, hi) in KHZ_BANDS.items():
        m = (c["lo"] < hi) & (c["hi"] > lo)
        out["by_band"][name] = shares(c[m], th)
    for o in range(grid.objects):
        m = c["obj"] == o
        if m.any():
            out["by_object"][str(o)] = shares(c[m], th)
    # whole-clip figures per object and QMF band: SNR, coherence, phase
    first = result["first_slot"]
    last = first + result["windows"] * result["window_slots"]
    ref_q = grid.q_ours[first:last] + diff_q[first:last]
    ours_q = grid.a_ours[first:last]
    snr, coh, phase = {}, {}, {}
    for o in range(grid.objects):
        if valid is not None and not valid[o]:
            continue
        r, u = ref_q[:, o], ours_q[:, o]
        e_r = np.sum(np.abs(r) ** 2, axis=0)
        e_d = np.sum(np.abs(r - u) ** 2, axis=0)
        cross = np.sum(r * u.conj(), axis=0)
        e_u = np.sum(np.abs(u) ** 2, axis=0)
        with np.errstate(divide="ignore", invalid="ignore"):
            snr[str(o)] = np.round(10 * np.log10(e_r / e_d), 2).tolist()
            coh[str(o)] = np.round(np.abs(cross) / np.sqrt(e_r * e_u), 6).tolist()
            phase[str(o)] = np.round(np.degrees(np.angle(cross)), 3).tolist()
    out["per_object_band"] = {"snr_db": snr, "coherence": coh, "phase_deg": phase}
    return out


# ---------------------------------------------------------------------------
# known answers


def synthesise_into(grid: Grid, q: np.ndarray) -> np.ndarray:
    """Object subband samples [t, obj, sb] as PCM aligned with our objects."""
    out = np.zeros_like(grid.pcm)
    for o in range(grid.objects):
        if not np.any(q[:, o]):
            continue
        y = qmf.synthesis(q[:, o], grid.window)[grid.delay: grid.delay + len(out)]
        out[: len(y), o] = y
    return out


def inject(grid: Grid, kind: str, level_db: float, rng: np.random.Generator, th: dict, share: float = 1.0):
    """A fault of known size in our own objects, as subband samples [t, obj,
    sb], and which cells [obj, band, win] carry it. `matrix`: a matrix offset
    of level_db against the rms of our matrix in the cell, a random sign a
    downmix channel; `timing`: our matrix one slot late; `core`: noise of
    level_db of full scale in every downmix channel, through our matrix. A
    cell where our object or its matrix is silent carries no fault."""
    w = int(th["window_slots"])
    first = qmf.FIRST_SLOT
    windows = (grid.slots - first - 16 - REACH_SLOTS) // w
    edges = grid.band_edges()
    bands = list(zip(edges[:-1], edges[1:]))
    m = grid.matrix()
    x = grid.downmix
    sl = slice(first, first + windows * w)
    fault = np.zeros_like(grid.q_ours)
    if kind == "core":
        noise_q = np.zeros_like(x)
        op = qmf.read_operator(qmf.EVIDENCE_49)
        for ch, carry in enumerate(grid.index["carry"]):
            noise = rng.standard_normal(grid.slots * N) * 10 ** (level_db / 20)
            noise_q[:, ch] = qmf.quadrature(qmf.analysis(noise, grid.window), carry.startswith("Shifted"), op,
                                            "plus: true" in carry)
        fault = np.einsum("tocs,tcs->tos", m, noise_q)
        return fault, np.ones((grid.objects, len(bands), windows), dtype=bool), bands, windows
    marked = rng.random((grid.objects, len(bands), windows)) < share
    for bi, (lo, hi) in enumerate(bands):
        mm = m[sl, :, :, lo:hi].reshape(windows, w, grid.objects, grid.channels, hi - lo)
        xs = x[sl, :, lo:hi].reshape(windows, w, grid.channels, hi - lo)
        qo = grid.q_ours[sl, :, lo:hi].reshape(windows, w, grid.objects, hi - lo)
        alive = (np.any(mm != 0, axis=(1, 3, 4)) & np.any(qo != 0, axis=(1, 3))).T  # [obj, win]
        marked[:, bi] &= alive
        if kind == "matrix":
            scale = np.sqrt(np.mean(mm ** 2, axis=(1, 3, 4)))  # [win, obj]
            sign = rng.choice([-1.0, 1.0], size=(windows, grid.objects, grid.channels))
            dm = sign * (scale * 10 ** (level_db / 20))[..., None]  # [win, obj, ch]
            f = np.einsum("woc,wtcs->wtos", dm, xs)
        else:  # timing
            mt = m[slice(first - 1, first - 1 + windows * w), :, :, lo:hi].reshape(mm.shape)
            f = np.einsum("wtocs,wtcs->wtos", mt - mm, xs)
        f = f * marked[:, bi].T[:, None, :, None]
        fault[sl, :, lo:hi] = f.reshape(windows * w, grid.objects, hi - lo)
    return fault, marked, bands, windows


def score(res: np.ndarray, kind: str, fault: np.ndarray, marked: np.ndarray, th: dict, first: int) -> dict:
    """How a run with a known fault came out, by cell: whether the cell
    carries the fault, whether the tool judged it, the class it gave, and the
    error in dB of the size it gave."""
    w = int(th["window_slots"])
    hit = marked[res["obj"], res["band"], res["win"]]
    windows = marked.shape[2]
    sl = slice(first, first + windows * w)
    fw = np.abs(fault[sl]) ** 2
    fw = fw.reshape(windows, w, fault.shape[1], N)
    truth = np.array([fw[c["win"], :, c["obj"], c["lo"]:c["hi"]].mean() for c in res]) if len(res) else np.zeros(0)
    want = "core" if kind == "core" else "matrix"
    est = res["dme"] if want == "matrix" else res["core"]
    ref = truth if want == "matrix" else res["res"]
    with np.errstate(divide="ignore", invalid="ignore"):
        err = 10 * np.log10(np.maximum(est, 1e-300) / np.maximum(ref, 1e-300))
    return {"hit": hit, "want": want, "err": err, "truth": truth}


def evaluate(res: np.ndarray, s: dict, th: dict, rows_per_column: float) -> dict:
    """The verdict of one run under a candidate rows_per_column: of the cells
    with the fault that are judged (not not_measured, not below_floor), the
    share in the right class and, of those, the share whose size is within
    known_answer_db; of the cells without, the share given a class."""
    cls = res["cls"].copy()
    cls[res["rows"] < rows_per_column * res["cols"]] = "not_measured"
    judged = s["hit"] & ~np.isin(cls, ["not_measured", "below_floor"])
    right = judged & (cls == s["want"])
    within = right & (np.abs(s["err"]) <= th["known_answer_db"])
    others = ~s["hit"]
    fa = others & np.isin(cls, ["matrix", "core", "unresolved"])
    nj = int(judged.sum())
    return {
        "cells_with_fault": int(s["hit"].sum()), "judged": nj,
        "judged_share": nj / max(int(s["hit"].sum()), 1),
        "right_class": float(right.sum() / nj) if nj else None,
        "size_within": float(within.sum() / max(int(right.sum()), 1)) if nj else None,
        "size_error_db_median": float(np.median(s["err"][right])) if right.any() else None,
        "unresolved": int((judged & (cls == "unresolved")).sum()),
        "false_alarms_elsewhere": float(fa.sum() / max(int(others.sum()), 1)),
    }


KINDS = (("matrix", -20.0), ("matrix", -40.0), ("matrix", -60.0), ("timing", 0.0),
         ("core", -50.0), ("core", -70.0), ("core", -90.0))


def known_answers(grid: Grid, th: dict, seed: int = 7, share: float = 1.0) -> list:
    """Every kind of fault at every level in our own objects, measured; the
    cells of each run and how to score them. The clean run has no fault."""
    rng = np.random.default_rng(seed)
    runs = []
    clean = cells(grid, np.zeros_like(grid.q_ours), th)["cells"]
    runs.append(("clean", 0.0, clean, None))
    for kind, level in KINDS:
        fault, marked, _bands, _windows = inject(grid, kind, level, rng, th, share)
        diff = grid.transform(synthesise_into(grid, fault))
        res = cells(grid, diff, th)["cells"]
        runs.append((kind, level, res, score(res, kind, fault, marked, th, qmf.FIRST_SLOT)))
        print(f"    {kind} {level:+.0f}: {len(res)} cells", flush=True)
    return runs


# ---------------------------------------------------------------------------
# entry points


def load_thresholds(path: Path | None) -> dict:
    th = dict(DEFAULTS)
    p = path or CALIBRATION
    if p.exists():
        th.update(json.loads(p.read_text(encoding="utf-8"))["thresholds"])
    return th


def measure(args, window) -> int:
    th = load_thresholds(args.calibration)
    t0 = time.time()
    grid = Grid(args.tap[0], window)
    ref = read_reference(args.reference, args.reference_channels)
    mapping, lag = match_objects(grid.pcm, ref)
    if lag < 0:
        ref = np.vstack([np.zeros((-lag, ref.shape[1])), ref])
    else:
        ref = ref[lag:]
    valid = np.array([m is not None for m in mapping])
    ref_objects = np.zeros_like(grid.pcm)
    for o, ch in enumerate(mapping):
        if ch is not None:
            n = min(len(ref), len(ref_objects))
            ref_objects[:n, o] = ref[:n, ch]
    diff_q = grid.transform(ref_objects - grid.pcm)
    result = cells(grid, diff_q, th)
    summary = summarise(grid, result, diff_q, valid, th)
    out = {
        "tool": "tools/joc_slot_error.py",
        "thresholds": th,
        "calibration_sha256": hashlib.sha256(Path(args.calibration or CALIBRATION).read_bytes()).hexdigest()
        if Path(args.calibration or CALIBRATION).exists() else None,
        "alignment": {"reference_lag_samples": lag, "object_to_reference_channel": mapping,
                      "transform_shift": grid.shift, "transform_slot_lag": grid.slot_lag},
        "bands": result["bands"],
        "windows": result["windows"],
        "summary": summary,
        "seconds": round(time.time() - t0, 1),
    }
    if args.cells:
        np.save(args.cells, result["cells"])
    text = json.dumps(out, indent=1, default=float)
    if args.out:
        args.out.write_text(text, encoding="utf-8")
    w = summary["whole"]
    print(f"cells {w['cells']}, verdict {w['verdict']}, unresolved share {w['unresolved_share']:.3f}, "
          + ", ".join(f"{k} {v['share']:.3f}" for k, v in w["by_class"].items()))
    return 0


ROWS_CANDIDATES = (8, 16, 32, 64, 128)
WINDOW_CANDIDATES = (24, 48)


def calibrate(args, window) -> int:
    """Fixes window_slots and rows_per_column on our own dumps (development
    clips only) with known faults injected, before the tool meets a reference
    decode. The rule, written before any run: a candidate holds when its clean
    runs raise under 1 % false alarms and, in every injected run of every
    clip, the judged cells with the fault are in the right class 95 % of the
    time and 95 % of those are within known_answer_db of the fault's size;
    of the candidates that hold, the one that judges the largest share of the
    cells with a fault is taken. Everything else keeps the plan's value.
    Cells a candidate leaves unjudged are its blind region, written down."""
    grids = [(d, Grid(d, window)) for d in args.tap]
    tried = []
    for w in WINDOW_CANDIDATES:
        th = dict(DEFAULTS, window_slots=w)
        per_clip = []
        for d, gr in grids:
            print(f"  window {w}, {d.name}", flush=True)
            per_clip.append((d.name, known_answers(gr, th)))
        for rows in ROWS_CANDIDATES:
            runs_out = []
            holds = True
            judged_shares = []
            for name, runs in per_clip:
                for kind, level, res, s in runs:
                    if s is None:
                        cls = res["cls"].copy()
                        cls[res["rows"] < rows * res["cols"]] = "not_measured"
                        fa = float(np.isin(cls, ["matrix", "core", "unresolved"]).mean()) if len(cls) else 0.0
                        runs_out.append({"clip": name, "kind": "clean", "false_alarm_rate": fa})
                        holds &= fa < 0.01
                        continue
                    ev = evaluate(res, s, th, rows)
                    runs_out.append({"clip": name, "kind": kind, "level_db": level, **ev})
                    judged_shares.append(ev["judged_share"])
                    if ev["judged"]:
                        holds &= ev["right_class"] >= 0.95 and ev["size_within"] >= 0.95
            cand = dict(th, rows_per_column=rows)
            tried.append({"thresholds": cand, "holds": bool(holds),
                          "judged_share_mean": float(np.mean(judged_shares)) if judged_shares else 0.0,
                          "runs": runs_out})
            print(f"  window {w} rows {rows}: holds {holds}, judged share "
                  f"{tried[-1]['judged_share_mean']:.3f}", flush=True)
    holding = [t for t in tried if t["holds"]]
    chosen = max(holding, key=lambda t: t["judged_share_mean"]) if holding else None
    out = {
        "what": "thresholds of tools/joc_slot_error.py, fixed on our own objects with known faults injected "
                "(development clips only), before the tool met a reference decode",
        "rule": calibrate.__doc__.split("The rule, written before any run: ")[1].split(" Everything else")[0]
                .replace("\n   ", "").replace("\n", " "),
        "clips": [d.name for d, _ in grids],
        "chosen": chosen is not None,
        "thresholds": chosen["thresholds"] if chosen else dict(DEFAULTS),
        "tried": tried,
    }
    args.out.write_text(json.dumps(out, indent=1, default=float), encoding="utf-8")
    print("chosen" if chosen else "no candidate held: the plan's values are kept and the tool's results "
          "may not be read as verdicts", flush=True)
    return 0 if chosen else 1


def self_test() -> int:
    rng = np.random.default_rng(20261121)
    ok = True

    def say(name, good, **v):
        nonlocal ok
        ok &= bool(good)
        print(f"{'ok  ' if good else 'FAIL'} {name}" + (f"  {v}" if v else ""))

    rows, cols = 96, 5
    x = rng.standard_normal((rows, cols)) + 1j * rng.standard_normal((rows, cols))
    m = rng.uniform(-1, 1, cols)
    f = fit_cell(x, x @ m)
    say("a real matrix comes back from a clean cell", np.allclose(f.coefficients, m, atol=1e-12)
        and f.condition == "good", kappa=round(f.kappa, 2))
    noisy = x @ m + 1e-3 * (rng.standard_normal(rows) + 1j * rng.standard_normal(rows))
    f = fit_cell(x, noisy)
    say("noise goes to the residual, not the coefficients",
        np.max(np.abs(f.coefficients - m)) < 1e-3 and abs(np.std(f.residual) - 1.41e-3) < 5e-4,
        most_coefficient_error=float(np.max(np.abs(f.coefficients - m))))
    dep = x.copy()
    dep[:, 1] = dep[:, 0]  # L and R the same content
    f = fit_cell(dep, dep @ m)
    say("dependent columns: rank short, no coefficients, the prediction holds",
        f.rank == cols - 1 and f.condition == "unsolvable" and np.isnan(f.coefficients).all()
        and np.allclose(f.prediction, dep @ m, atol=1e-10), rank=f.rank)
    near = x.copy()
    near[:, 1] = near[:, 0] + 1e-2 * (rng.standard_normal(rows) + 1j * rng.standard_normal(rows))
    f = fit_cell(near, near @ m)
    say("nearly dependent columns are weak, with full rank", f.condition == "weak" and f.rank == cols,
        kappa=round(f.kappa, 1), condition=f.condition)
    # the batched solver of the normal equations is the SVD fit
    for design in (x, near):
        y = noisy
        g = (design.conj().T @ design).real
        h = (design.conj().T @ y).real
        coef, sigma, rank = solve_gram(g[None], h[None], RANK_RTOL)
        ref = fit_cell(design, y)
        say("the Gram solver gives the SVD's coefficients and singular values",
            np.allclose(coef[0], ref.coefficients, rtol=1e-6, atol=1e-9)
            and np.allclose(sigma[0], ref.singular_values, rtol=1e-6) and rank[0] == ref.rank)
    # a significant matrix part, a core part, the floor, by classify_cell
    th = DEFAULTS
    g = 1.0
    say("classify: a large explained part is matrix",
        classify_cell(1.0, 1e-3, 9e-4, 1e-4, 1e-8, 1e-6, True, "good", 960, 5, th, g) == "matrix")
    say("classify: a large residual is core",
        classify_cell(1.0, 1e-3, 1e-5, 9.9e-4, 1e-8, 1e-6, False, "good", 960, 5, th, g) == "core")
    say("classify: under the floor",
        classify_cell(1.0, 2e-8, 1e-8, 1e-8, 1e-8, 1e-9, True, "good", 960, 5, th, g) == "below_floor")
    say("classify: too quiet to measure",
        classify_cell(1e-10, 1e-3, 1e-3, 0, 1e-12, 0, True, "good", 960, 5, th, g) == "not_measured")
    say("classify: an unsolvable design with a large explained part is unresolved",
        classify_cell(1.0, 1e-3, 9e-4, 1e-4, 1e-8, 1e-6, False, "unsolvable", 960, 5, th, g) == "unresolved")
    # the classifier over arrays is the classifier of one cell
    r = np.random.default_rng(3)
    n = 4000
    args = dict(ref_e=10 ** r.uniform(-14, 0, n), res_e=10 ** r.uniform(-16, -2, n),
                pred_e=10 ** r.uniform(-16, -2, n), core_e=10 ** r.uniform(-16, -2, n),
                leak=10 ** r.uniform(-18, -6, n), chance=10 ** r.uniform(-18, -4, n),
                ci=r.random(n) < 0.5, cond=r.choice(["good", "weak", "unsolvable"], n),
                rows=r.choice([20, 60, 400], n), cols=r.integers(1, 7, n))
    vec = np.concatenate([classify_vec(args["ref_e"][i:i + 1], args["res_e"][i:i + 1], args["pred_e"][i:i + 1],
                                       args["core_e"][i:i + 1], args["leak"][i:i + 1], args["chance"][i:i + 1],
                                       args["ci"][i:i + 1], args["cond"][i:i + 1], args["rows"][i],
                                       args["cols"][i], th, 1.0) for i in range(n)])
    one = [classify_cell(args["ref_e"][i], args["res_e"][i], args["pred_e"][i], args["core_e"][i],
                         args["leak"][i], args["chance"][i], bool(args["ci"][i]), args["cond"][i],
                         args["rows"][i], args["cols"][i], th, 1.0) for i in range(n)]
    say("the classifier over arrays agrees with the one of a cell", list(vec) == one,
        classes=sorted(set(one)))
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description="JOC error by object, band and time slot.")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--calibrate", action="store_true")
    ap.add_argument("--tables", type=Path, help="ts_103420_tables.c of ETSI, or its zip")
    ap.add_argument("--tap", type=Path, action="append", default=[], help="a dump of examples/joc_tap.rs")
    ap.add_argument("--reference", type=Path, help="the reference decoder's objects, f32")
    ap.add_argument("--reference-channels", type=int, default=16)
    ap.add_argument("--calibration", type=Path)
    ap.add_argument("--cells", type=Path, help="write every cell as a .npy")
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if not args.tables or not args.tap:
        ap.error("--tables and --tap are needed")
    window = qmf.read_window(args.tables)
    if args.calibrate:
        if not args.out:
            ap.error("--calibrate writes --out")
        return calibrate(args, window)
    if not args.reference:
        ap.error("nothing to do: --self-test, --calibrate, or --tap with --reference")
    return measure(args, window)


if __name__ == "__main__":
    sys.exit(main())
