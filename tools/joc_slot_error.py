#!/usr/bin/env python3
"""Where the JOC objects of this decoder part from those of the Dolby decoder,
by object, parameter band and time slot, and whether the matrix or the core
is to blame.

The Dolby decoder writes only PCM, so the comparison is built round it:

1. Dolby's objects and ours go through one Python analysis bank
   (`joc_qmf_reference.analysis`). The difference is linear, so the near-PR
   aliasing of the bank cancels to first order; what is left of it, the
   leakage floor, is measured apart, band by band (the tap's object subband
   samples through our synthesis and back through the Python analysis).
2. Per cell (object, parameter band, sliding window of slots) Dolby's object
   subband samples are fitted onto our downmix subband samples (the tap,
   `examples/joc_tap.rs`) by least squares. The fitted matrix less ours is the
   matrix/timing error; the residual of the fit, what no matrix makes of our
   downmix, is the core difference (dither, spectral extension, coupling).
3. Dolby's core decode (`out-ch-config=7`) is held to ours on the same grid:
   an upper bound on the core difference that does not lean on step 2.

How far a fit is to be trusted, per cell (thresholds are fixed on synthetic
data before the tool meets Dolby's output, and then not moved):

- the matrix is real and the samples complex, so real and imaginary parts are
  stacked and real coefficients fitted, by SVD (`fit_cell`);
- singular values, numerical rank (sigma < sigma_max * RANK_RTOL is zero) and
  kappa = sigma_max / sigma_min: GOOD <= KAPPA_GOOD < WEAK <= KAPPA_WEAK <
  unsolvable; a rank-deficient design reports only the prediction difference
  (M_fit - M_ours) X, never coefficients;
- energy: the object and every downmix column above ENERGY_FLOOR_DBFS and
  LEAK_MARGIN_DB over the leakage floor, and at least ROWS_PER_COLUMN rows a
  column, or the cell is `not_measured`;
- an error less than FLOOR_MARGIN_DB over the leakage floor is `below_floor`;
- a 95 % interval per coefficient by block bootstrap over slots (BLOCK_SLOTS);
  a matrix error counts only outside it and over the floor;
- known answers before Dolby: a matrix offset, a one-slot shift and core noise
  of known size injected into our own output, apart and together, must come
  back within KNOWN_ANSWER_DB and in the right class; what does not is the
  tool's blind region, written down;
- cross-check: the step-2 residual and the step-3 core difference through our
  matrix agree within CROSS_DB, or the cell is `inconsistent`;
- classes: matrix / core / unresolved / not_measured / below_floor /
  inconsistent. A suspect is judged from `matrix` and `core` cells only; when
  the other classes hold more than UNRESOLVED_SHARE of the residual energy of
  a heading, the heading gets "the measurement could not resolve it".

    python tools/joc_slot_error.py --self-test
    python tools/joc_slot_error.py --tap <dir> --dolby <objects.f32> --out <cells.json>   (round 5, stop 2)

Exit codes: 0 held, 1 a check failed, 2 bad arguments, 3 not built yet.
"""

from __future__ import annotations

import argparse
import sys
from dataclasses import dataclass

import numpy as np

# Starting values from the round 5 plan. Each is fixed by the calibration on
# synthetic data (`calibrate`, stop 2) and recorded with the result; a value
# the calibration does not bear out is changed there, with its reason, once.
RANK_RTOL = 1e-6
KAPPA_GOOD = 100.0
KAPPA_WEAK = 1000.0
ENERGY_FLOOR_DBFS = -80.0
LEAK_MARGIN_DB = 20.0
ROWS_PER_COLUMN = 8
FLOOR_MARGIN_DB = 10.0
BLOCK_SLOTS = 8
BOOTSTRAP_DRAWS = 1000
KNOWN_ANSWER_DB = 1.0
CROSS_DB = 3.0
UNRESOLVED_SHARE = 0.30

CLASSES = ("matrix", "core", "unresolved", "not_measured", "below_floor", "inconsistent")
CONDITION = ("good", "weak", "unsolvable")


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
    if rank < a.shape[1] or kappa > KAPPA_WEAK:
        condition = "unsolvable"
    elif kappa > KAPPA_GOOD:
        condition = "weak"
    else:
        condition = "good"
    if rank < a.shape[1]:
        # only the identifiable subspace has a meaning: report predictions
        coef_out = np.full(a.shape[1], np.nan)
    else:
        coef_out = coef
    prediction = x @ coef
    return Fit(coef_out, s, rank, kappa, condition, y - prediction, prediction)


def classify(cell: dict) -> str:
    """The class of one cell from its fit, energies, floor and interval (stop 2)."""
    raise NotImplementedError("round 5, stop 2")


def calibrate() -> dict:
    """Fixes the thresholds on synthetic data (stop 2): our own tap output with
    known matrix offsets, one-slot shifts and core noise injected over a grid
    of energy and kappa; the result is the blind region and the thresholds."""
    raise NotImplementedError("round 5, stop 2")


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
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description="JOC error by object, band and time slot.")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--tap", help="a dump of examples/joc_tap.rs")
    ap.add_argument("--dolby", help="Dolby's objects (out-ch-config=21), f32")
    ap.add_argument("--out")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.tap or args.dolby:
        print("joc_slot_error: the measurement is not built yet (round 5, stop 2)", file=sys.stderr)
        return 3
    ap.error("nothing to do: --self-test, or --tap with --dolby")
    return 2


if __name__ == "__main__":
    sys.exit(main())
