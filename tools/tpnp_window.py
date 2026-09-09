#!/usr/bin/env python3
"""Measure what transient pre-noise processing changes, and whether it moves
the decoder towards the Dolby reference.

The correction regions are found by differencing our own two decodes rather
than by parsing the bit stream: every sample the tool touches differs, and no
other sample does. The reference decode is aligned by cross-correlation and
its channel permutation is detected the same way, so the script needs no
knowledge of the channel order of either side.

    python tools/tpnp_window.py on.f32 off.f32 dolby.wav [--channels 6]
"""

from __future__ import annotations

import argparse
import struct
import sys

import numpy as np


def read_raw(path: str, channels: int) -> np.ndarray:
    a = np.fromfile(path, dtype="<f4")
    return a.reshape(-1, channels)


def read_wav(path: str) -> tuple[np.ndarray, int]:
    with open(path, "rb") as fh:
        data = fh.read(12)
        if data[:4] not in (b"RIFF", b"RF64"):
            raise SystemExit(f"{path}: not a RIFF file")
        fmt = None
        while True:
            head = fh.read(8)
            if len(head) < 8:
                break
            cid, size = struct.unpack("<4sI", head)
            if cid == b"fmt ":
                fmt = fh.read(size)
            elif cid == b"data":
                if size in (0xFFFFFFFF, 0):
                    raw = fh.read()
                else:
                    raw = fh.read(size)
                break
            elif cid == b"ds64":
                body = fh.read(size)
                # RF64: the real data size is the third 64-bit field
                _, data_size = struct.unpack("<QQ", body[:16])
                fh.seek(0, 2)
                end = fh.tell()
                fh.seek(-(end - fh.tell()), 2)
                raise SystemExit(f"{path}: RF64 not needed here")
            else:
                fh.seek(size + (size & 1), 1)
        if fmt is None:
            raise SystemExit(f"{path}: no fmt chunk")
        channels, _rate = struct.unpack("<HI", fmt[2:8])
        bits = struct.unpack("<H", fmt[14:16])[0]
    if bits == 24:
        b = np.frombuffer(raw[: (len(raw) // 3) * 3], dtype=np.uint8).reshape(-1, 3)
        v = (
            b[:, 0].astype(np.int32)
            | (b[:, 1].astype(np.int32) << 8)
            | (b[:, 2].astype(np.int8).astype(np.int32) << 16)
        )
        a = v.astype(np.float64) / 8388608.0
    elif bits == 32:
        a = np.frombuffer(raw, dtype="<f4").astype(np.float64)
    elif bits == 16:
        a = np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768.0
    else:
        raise SystemExit(f"{path}: {bits}-bit is not handled")
    return a.reshape(-1, channels), channels


def best_lag(a: np.ndarray, b: np.ndarray, span: int = 2048) -> int:
    """The shift of `b` against `a`, searched over +/- span."""
    n = min(len(a), len(b), 1 << 20)
    a = a[:n] - a[:n].mean()
    b = b[:n] - b[:n].mean()
    fa = np.fft.rfft(a, 2 * n)
    fb = np.fft.rfft(b, 2 * n)
    c = np.fft.irfft(fa * np.conj(fb), 2 * n)
    c = np.concatenate([c[-span:], c[: span + 1]])
    return int(np.argmax(c)) - span


def db(x: float) -> str:
    return "-inf" if x <= 0 else f"{20 * np.log10(x):.1f}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("on")
    ap.add_argument("off")
    ap.add_argument("dolby")
    ap.add_argument("--channels", type=int, default=6)
    ap.add_argument("--names", default="")
    args = ap.parse_args()

    on = read_raw(args.on, args.channels).astype(np.float64)
    off = read_raw(args.off, args.channels).astype(np.float64)
    ref, refch = read_wav(args.dolby)
    if refch != args.channels:
        raise SystemExit(f"reference has {refch} channels, expected {args.channels}")
    names = args.names.split(",") if args.names else [f"ch{i}" for i in range(args.channels)]

    n = min(len(on), len(off))
    on, off = on[:n], off[:n]

    # Which reference channel is which, and by how much it lags. The
    # assignment has to be a bijection: two of our channels correlating best
    # with the same reference channel means one of them is nearly silent, and
    # letting it win would report a nonsense error.
    table = []
    for c in range(args.channels):
        for r in range(args.channels):
            lag = best_lag(off[:, c], ref[:, r])
            m = min(len(off) - max(lag, 0), len(ref) - max(-lag, 0), 1 << 19)
            x = off[max(lag, 0) : max(lag, 0) + m, c]
            y = ref[max(-lag, 0) : max(-lag, 0) + m, r]
            d = np.linalg.norm(x) * np.linalg.norm(y)
            table.append((abs(float(x @ y)) / d if d else 0.0, c, r, lag))
    table.sort(reverse=True)
    taken_c: set[int] = set()
    taken_r: set[int] = set()
    pairs = []
    for corr, c, r, lag in table:
        if c in taken_c or r in taken_r:
            continue
        taken_c.add(c)
        taken_r.add(r)
        pairs.append((c, r, lag, corr))
    pairs.sort()

    print(f"{'channel':>8} {'ref':>4} {'lag':>6} {'corr':>6} {'touched':>9} "
          f"{'err off':>9} {'err on':>9} {'change dB':>10}")
    total_off = total_on = 0.0
    touched_total = 0
    for c, r, lag, corr in pairs:
        diff = on[:, c] - off[:, c]
        idx = np.nonzero(diff)[0]
        if idx.size == 0:
            print(f"{names[c]:>8} {r:>4} {lag:>6} {corr:>6.3f} {0:>9} "
                  f"{'-':>9} {'-':>9} {'-':>10}")
            continue
        # align: reference sample for our sample i is i - lag
        j = idx - lag
        keep = (j >= 0) & (j < len(ref))
        idx, j = idx[keep], j[keep]
        e_off = float(np.sqrt(np.mean((off[idx, c] - ref[j, r]) ** 2)))
        e_on = float(np.sqrt(np.mean((on[idx, c] - ref[j, r]) ** 2)))
        total_off += e_off**2 * idx.size
        total_on += e_on**2 * idx.size
        touched_total += idx.size
        change = 20 * np.log10(e_on / e_off) if e_off > 0 and e_on > 0 else float("nan")
        print(f"{names[c]:>8} {r:>4} {lag:>6} {corr:>6.3f} {idx.size:>9} "
              f"{db(e_off):>9} {db(e_on):>9} {change:>+10.2f}")

    if touched_total:
        eo = np.sqrt(total_off / touched_total)
        en = np.sqrt(total_on / touched_total)
        print(f"\n{touched_total} samples touched; error against the reference "
              f"{db(eo)} dB without the tool, {db(en)} dB with it, "
              f"{20 * np.log10(en / eo):+.2f} dB.")
        print("A negative change means the tool moved the decoder towards Dolby.")
    else:
        print("\nNo sample changed: the stream signals no transient pre-noise.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
