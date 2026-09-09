#!/usr/bin/env python3
"""Measure what Dolby does to the lowest QMF subband of a phase-shifted
downmix, and print the correction table of `oadec-joc/src/quadrature.rs`.

ETSI TS 103 420 table 47 names downmix configurations 3 and 4 "5.X with
90 degree phase shift" and never defines the shift. Rotating every subband
by -j is right above about 141 Hz and wrong below it, because subband 0
straddles direct current. This script measures the operator Dolby applies
there and fits the real filter across time slots that reproduces it.

The measurement needs three decodes of the same stream:

* the objects of the Dolby decoder, which the Dolby Reference Player will
  write once its GStreamer plugins are asked for the raw object mode:

      gst-launch-1.0 filesrc location=in.ec3
        ! dlbac3parse enable-metadata=true
        ! dlbac3dec out-ch-config=21 drc-suppress=true drc-mode=custom-0
                    drc-cut=0 drc-boost=0 drop-delay=true
        ! "audio/x-raw(meta:DlbObjectAudioMeta),format=F32LE"
        ! identity ! filesink location=dolby.f32

  (GST_PLUGIN_PATH must point at the player's `gst-plugins` directory and
  the player's directory must be on PATH.)

* ours with the rotation applied flat to every subband:

      oadec decode in.ec3 --format damf --no-bed-conform --flat-quadrature -o flat

* ours with subband 0 left alone:

      OADEC_JOC_LOW=untouched oadec decode in.ec3 --format damf \
          --no-bed-conform -o untouched

Those two differ only in subband 0, so their difference is a basis: at every
frequency Dolby's answer is a complex multiple R(f) of it. Because one time
slot of delay is exactly 64 samples of delay, a real filter across slots has
the audio-domain response `sum c[k] exp(-2 pi i f k / 750)`, and fitting that
to R(f) gives the table.

    python tools/gen_joc_quadrature.py --taps 37 \\
        --set dolby.f32 flat.atmos.audio untouched.atmos.audio \\
        --set ...

Give several sets from different titles and encoders; the fit pools them.
"""

from __future__ import annotations

import argparse
import sys

import numpy as np

RATE = 48000
BANDS = 64
SLOT_RATE = RATE / BANDS  # 750 Hz
OBJECTS = 16


def read_caf(path: str) -> np.ndarray:
    """The `.atmos.audio` of a DAMF set: Core Audio Format, 24-bit big-endian."""
    b = np.fromfile(path, dtype=np.uint8)
    if b[:4].tobytes() != b"caff":
        raise SystemExit(f"{path}: not a Core Audio Format file")
    i, channels, bits = 8, None, None
    while i < len(b):
        name = b[i : i + 4].tobytes().decode("latin1")
        size = int.from_bytes(b[i + 4 : i + 12].tobytes(), "big", signed=True)
        body = i + 12
        if name == "desc":
            channels = int.from_bytes(b[body + 24 : body + 28].tobytes(), "big")
            bits = int.from_bytes(b[body + 28 : body + 32].tobytes(), "big")
        elif name == "data":
            if bits != 24:
                raise SystemExit(f"{path}: {bits} bits, expected 24")
            n = (len(b) - body - 4) if size <= 0 else size - 4
            raw = b[body + 4 : body + 4 + n].reshape(-1, 3).astype(np.int32)
            v = (raw[:, 0] << 16) | (raw[:, 1] << 8) | raw[:, 2]
            v = np.where(v & 0x800000, v - 0x1000000, v)
            return v.reshape(-1, channels).astype(np.float64) / 2**23
        i = body + (size if size > 0 else len(b))
    raise SystemExit(f"{path}: no data chunk")


def measure(sets, top: float, window: int):
    """R(f) and its weight, pooled over the sets."""
    w = np.hanning(window)
    f = np.fft.rfftfreq(window, 1 / RATE)
    keep = f <= top
    num = np.zeros(int(keep.sum()), dtype=complex)
    den = np.zeros(int(keep.sum()))
    for dolby, flat, untouched in sets:
        d = np.fromfile(dolby, dtype="<f4").reshape(-1, OBJECTS).astype(np.float64)
        a = read_caf(flat)
        b = read_caf(untouched)
        n = min(len(d), len(a), len(b))
        for at in range(RATE, n - window, window // 2):
            for c in range(1, OBJECTS):
                left = np.fft.rfft((d[at : at + window, c] - a[at : at + window, c]) * w)
                base = np.fft.rfft((b[at : at + window, c] - a[at : at + window, c]) * w)
                num += np.conj(base[keep]) * left[keep]
                den += np.abs(base[keep]) ** 2
    return f[keep], num / np.maximum(den, 1e-30), den


def fit(freq, response, weight, taps: int):
    """The real filter across time slots that comes closest to `response`."""
    half = taps // 2
    k = np.arange(-half, half + 1)
    grid = np.exp(-2j * np.pi * np.outer(freq, k) / SLOT_RATE)
    root = np.sqrt(weight / weight.max())[:, None]
    lhs = np.concatenate([(grid * root).real, (grid * root).imag], axis=0)
    rhs = np.concatenate(
        [(response * root[:, 0]).real, (response * root[:, 0]).imag]
    )
    c, *_ = np.linalg.lstsq(lhs, rhs, rcond=None)
    left = grid @ c - response
    scale = weight / weight.max()
    err = 10 * np.log10(
        np.sum(scale * np.abs(left) ** 2) / np.sum(scale * np.abs(response) ** 2)
    )
    return c, err


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--set",
        nargs=3,
        action="append",
        metavar=("DOLBY", "FLAT", "UNTOUCHED"),
        required=True,
        help="one title: the Dolby objects, ours flat, ours with subband 0 left alone",
    )
    ap.add_argument("--taps", type=int, default=37, help="odd, the table length")
    ap.add_argument("--top", type=float, default=470.0, help="Hz to fit up to")
    ap.add_argument("--window", type=int, default=1 << 16)
    args = ap.parse_args()
    if args.taps % 2 == 0:
        raise SystemExit("--taps must be odd")

    freq, response, weight = measure(args.set, args.top, args.window)
    c, err = fit(freq, response, weight, args.taps)

    print("// measured on:", file=sys.stderr)
    for s in args.set:
        print("//   ", s[0], file=sys.stderr)
    print(f"// weighted fit {err:.1f} dB, response at 0 Hz {c.sum():.4f}", file=sys.stderr)
    print(f"pub const LOW_TAPS: usize = {args.taps};")
    print("const CORRECTION: [f64; LOW_TAPS] = [")
    for v in c:
        print(f"    {v:.14e},".replace("e-0", "e-").replace("e+0", "e"))
    print("];")
    for hz in (0.0, 25.0, 50.0, 100.0, 141.0, 200.0, 300.0):
        got = np.sum(c * np.exp(-2j * np.pi * hz * np.arange(-(args.taps // 2), args.taps // 2 + 1) / SLOT_RATE))
        want = np.interp(hz, freq, response.real) + 1j * np.interp(hz, freq, response.imag)
        print(
            f"// {hz:6.1f} Hz  fitted {abs(got):.3f} at {np.degrees(np.angle(got)):7.1f} deg"
            f"   measured {abs(want):.3f} at {np.degrees(np.angle(want)):7.1f} deg",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
