#!/usr/bin/env python3
"""Compare our decode with the Dolby one when there is no third opinion.

`three_way.py` judges us against the distance between FFmpeg and Dolby, which
only works for the tools FFmpeg implements. Enhanced coupling is not one of
them: FFmpeg's decoder refuses the stream outright. This script therefore
reports the plain per-channel figures and leaves the judgement to the reader,
after removing the constant lag and the gain the Dolby decode carries.

    python tools/two_way.py ours.f32 dolby.wav [--channels 6] [--names L,R,C,LFE,Ls,Rs]
"""

from __future__ import annotations

import argparse
import sys

import numpy as np

sys.path.insert(0, __file__.rsplit("/", 1)[0] if "/" in __file__ else ".")
from tpnp_window import best_lag, read_wav  # noqa: E402


def db(x: float) -> str:
    return "-inf" if x <= 0 else f"{20 * np.log10(x):.1f}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("ours")
    ap.add_argument("dolby")
    ap.add_argument("--channels", type=int, default=6)
    ap.add_argument("--names", default="")
    ap.add_argument("--seconds", type=float, default=0.0, help="0 for the whole file")
    args = ap.parse_args()

    ours = np.fromfile(args.ours, dtype="<f4").reshape(-1, args.channels).astype(np.float64)
    ref, refch = read_wav(args.dolby)
    if refch != args.channels:
        raise SystemExit(f"the reference has {refch} channels, expected {args.channels}")
    if args.seconds:
        n = int(args.seconds * 48000)
        ours, ref = ours[:n], ref[:n]
    names = args.names.split(",") if args.names else [f"ch{i}" for i in range(args.channels)]

    print(f"{'channel':>8} {'lag':>5} {'gain':>7} {'signal':>8} {'diff':>8} {'SNR':>7} {'max diff':>10}")
    worst = None
    for c in range(args.channels):
        lag = best_lag(ours[:, c], ref[:, c])
        m = min(len(ours) - max(lag, 0), len(ref) - max(-lag, 0))
        x = ours[max(lag, 0) : max(lag, 0) + m, c]
        y = ref[max(-lag, 0) : max(-lag, 0) + m, c]
        denom = float(y @ y)
        gain = float(x @ y) / denom if denom else 0.0
        if abs(gain) < 1e-9:
            print(f"{names[c]:>8} {lag:>5} {'silent':>7}")
            continue
        d = x - gain * y
        s = float(np.sqrt(np.mean(y**2))) * abs(gain)
        e = float(np.sqrt(np.mean(d**2)))
        snr = 20 * np.log10(s / e) if e > 0 else float("inf")
        worst = snr if worst is None else min(worst, snr)
        print(
            f"{names[c]:>8} {lag:>5} {gain:>7.4f} {db(s):>8} {db(e):>8} {snr:>7.1f} "
            f"{float(np.abs(d).max()):>10.3e}"
        )
    if worst is not None:
        print(f"\nworst channel {worst:.1f} dB against the Dolby decode.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
