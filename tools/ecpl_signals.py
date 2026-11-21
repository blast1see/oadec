#!/usr/bin/env python3
"""Source signals for the enhanced coupling carriers (`docs/eac3.md`).

    python tools/ecpl_signals.py tones  --layout 2.0|5.1 --seconds 20 --out x.wav
    python tools/ecpl_signals.py noise  --layout 2.0|5.1 --seconds 20 --seed 1 --out x.wav
    python tools/ecpl_signals.py bursts --layout 2.0|5.1 --seconds 20 --out x.wav

Each is 48 kHz, 24-bit WAVE, channels in WAVE order (L R, or L R C LFE Ls
Rs), and made the same on every run. All three put something of each
channel's own under the region an encoder couples at these rates (1 to 4,5
kHz) and something shared above it (5 to 18 kHz): Dolby's decoder
decorrelates a coupled channel only where the channel has content of its
own under the coupling begin frequency (evidence 52), so a signal without
it measures nothing.

- `tones`: in the shared region a tone every kHz from 5,5 to 17,5 kHz at
  -30 dBFS, each channel's a quarter of a kHz off the others' and turning
  in phase (so the coupled channels differ), and a 440 Hz tone and noise of
  the channel's own between 1 and 4,5 kHz at -36 dBFS.
- `noise`: band-limited noise in the shared region, half shared and half
  the channel's own, the channels' own noise under it, and every two
  seconds an impulse, alternately at the middle of a block of 256 samples
  and at its edge.
- `bursts`: 50 ms tone bursts at 9 and 13 kHz in the shared region, their
  starts 0, 64 and 128 samples after a block edge in turn, over the
  channels' own noise.

The LFE of a 5.1 signal is a 55 Hz tone at -30 dBFS.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import numpy as np

RATE = 48000
NAMES = {"2.0": ["L", "R"], "5.1": ["L", "R", "C", "LFE", "Ls", "Rs"]}


def db(x: float) -> float:
    return 10 ** (x / 20)


def band_noise(rng, n: int, low: float, high: float) -> np.ndarray:
    spec = np.fft.rfft(rng.standard_normal(n))
    f = np.fft.rfftfreq(n, 1 / RATE)
    spec[(f < low) | (f >= high)] = 0
    x = np.fft.irfft(spec, n)
    return x / np.sqrt(np.mean(x * x))


def own(rng, n: int, ch: int) -> np.ndarray:
    t = np.arange(n) / RATE
    return db(-36) * band_noise(rng, n, 1000, 4500) + db(-36) * np.sin(2 * np.pi * 440 * t + ch)


def tones(layout: str, n: int, rng) -> np.ndarray:
    t = np.arange(n) / RATE
    out = np.zeros((n, len(NAMES[layout])))
    for ch, name in enumerate(NAMES[layout]):
        if name == "LFE":
            out[:, ch] = db(-30) * np.sin(2 * np.pi * 55 * t)
            continue
        x = own(rng, n, ch)
        for k in range(13):
            f = 5500 + 1000 * k + 250 * (ch % 4)
            x += db(-30) * np.sin(2 * np.pi * f * t + 0.7 * ch * np.sin(2 * np.pi * 0.25 * t))
        out[:, ch] = x
    return out


def noise(layout: str, n: int, rng) -> np.ndarray:
    shared = band_noise(rng, n, 5000, 18000)
    out = np.zeros((n, len(NAMES[layout])))
    pulses = np.zeros(n)
    for i, at in enumerate(range(RATE, n - 512, 2 * RATE)):
        at = at // 256 * 256 + (128 if i % 2 == 0 else 0)
        pulses[at] = db(-6)
    t = np.arange(n) / RATE
    for ch, name in enumerate(NAMES[layout]):
        if name == "LFE":
            out[:, ch] = db(-30) * np.sin(2 * np.pi * 55 * t)
            continue
        mine = band_noise(rng, n, 5000, 18000)
        out[:, ch] = own(rng, n, ch) + db(-30) * (shared + mine) / np.sqrt(2) + pulses
    return out


def bursts(layout: str, n: int, rng) -> np.ndarray:
    t = np.arange(n) / RATE
    out = np.zeros((n, len(NAMES[layout])))
    length = RATE // 20
    env = np.zeros(n)
    for i, at in enumerate(range(RATE // 2, n - length, RATE // 2)):
        start = at // 256 * 256 + (0, 64, 128)[i % 3]
        env[start:start + length] = np.hanning(length + 2)[1:-1] ** 0.25
    for ch, name in enumerate(NAMES[layout]):
        if name == "LFE":
            out[:, ch] = db(-30) * np.sin(2 * np.pi * 55 * t)
            continue
        x = own(rng, n, ch)
        x += db(-20) * env * (np.sin(2 * np.pi * 9000 * t + ch) + np.sin(2 * np.pi * 13000 * t + 2 * ch))
        out[:, ch] = x
    return out


def write_wave(path: Path, x: np.ndarray) -> None:
    peak = float(np.abs(x).max())
    if peak >= 1.0:
        x = x * (0.98 / peak)
    q = np.clip(np.round(x * 8388607), -8388608, 8388607).astype("<i4")
    b = q.reshape(-1)
    raw = np.stack([(b & 0xFF), (b >> 8) & 0xFF, (b >> 16) & 0xFF], axis=1).astype(np.uint8).tobytes()
    ch = x.shape[1]
    hdr = b"RIFF" + (36 + len(raw)).to_bytes(4, "little") + b"WAVEfmt " + (16).to_bytes(4, "little")
    hdr += (1).to_bytes(2, "little") + ch.to_bytes(2, "little") + RATE.to_bytes(4, "little")
    hdr += (RATE * ch * 3).to_bytes(4, "little") + (ch * 3).to_bytes(2, "little") + (24).to_bytes(2, "little")
    hdr += b"data" + len(raw).to_bytes(4, "little")
    path.write_bytes(hdr + raw)


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("kind", choices=["tones", "noise", "bursts"])
    ap.add_argument("--layout", choices=sorted(NAMES), required=True)
    ap.add_argument("--seconds", type=float, default=20.0)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--out", type=Path, required=True)
    a = ap.parse_args()
    n = int(a.seconds * RATE)
    rng = np.random.default_rng(a.seed)
    x = {"tones": tones, "noise": noise, "bursts": bursts}[a.kind](a.layout, n, rng)
    write_wave(a.out, x)
    print(f"{a.out.name}: {a.kind} {a.layout} {a.seconds} s, peak {20 * np.log10(np.abs(x).max()):.1f} dBFS")
    return 0


if __name__ == "__main__":
    sys.exit(main())
