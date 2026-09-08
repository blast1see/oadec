#!/usr/bin/env python3
"""Compare JOC-decoded objects with the source objects they were encoded from.

    python tools/joc_roundtrip.py source.atmos.audio decoded.atmos.audio [lag]

Both inputs are DAMF audio files (CAF, 24-bit) whose first ten channels are
the bed and the rest the objects. The comparison uses the correlation of
analytic signals, which is blind to a constant all-pass phase shift (the
90-degree downmix configurations of JOC), and reports per decoded object the
best-matching source object, the residual phase and the signal-to-noise
ratio implied by the correlation, then how much of every source object the
decoded objects explain together. Reads 40 seconds from 20 seconds in.
"""
import struct
import sys

import numpy as np


def read_caf(path, start_frame, frames):
    with open(path, "rb") as f:
        f.read(8)
        desc = None
        while True:
            h = f.read(12)
            if len(h) < 12:
                break
            ctype, size = struct.unpack(">4sq", h)
            if ctype == b"desc":
                body = f.read(size)
                rate, fmt, flags, bpp, fpp, ch, bits = struct.unpack(">d4sIIIII", body[:32])
                desc = (rate, ch, bits, bpp)
            elif ctype == b"data":
                rate, ch, bits, bpp = desc
                f.seek(4 + start_frame * bpp, 1)
                raw = f.read(frames * bpp)
                a = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3)
                v = (a[:, 0].astype(np.int32) << 16) | (a[:, 1].astype(np.int32) << 8) | a[:, 2].astype(np.int32)
                v = np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.float64) / (1 << 23)
                return v.reshape(-1, ch)
            else:
                f.seek(size, 1)


def analytic(x):
    n = len(x)
    X = np.fft.fft(x)
    h = np.zeros(n)
    h[0] = 1
    h[1 : n // 2] = 2
    if n % 2 == 0:
        h[n // 2] = 1
    return np.fft.ifft(X * h)


src_path, joc_path = sys.argv[1], sys.argv[2]
lag = int(sys.argv[3]) if len(sys.argv) > 3 else -1
start = 48000 * 20
frames = 48000 * 40
src = read_caf(src_path, start, frames)
joc = read_caf(joc_path, start, frames)
n = min(len(src), len(joc)) - abs(lag) - 1
if lag >= 0:
    so, jo = src[:n, 10:], joc[lag : lag + n, 10:]
else:
    so, jo = src[-lag : -lag + n, 10:], joc[:n, 10:]
nj, ns = jo.shape[1], so.shape[1]
A_s = np.stack([analytic(so[:, k]) for k in range(ns)], axis=1)
A_j = np.stack([analytic(jo[:, k]) for k in range(nj)], axis=1)
norm_s = np.linalg.norm(A_s, axis=0) + 1e-12
norm_j = np.linalg.norm(A_j, axis=0) + 1e-12
C = (A_j.conj().T @ A_s) / np.outer(norm_j, norm_s)  # complex correlation [joc][src]
np.set_printoptions(precision=2, suppress=True, linewidth=220)
print("|complex correlation| (rows joc objects, cols source objects):")
print(np.abs(C))
print()
print("joc  src  |corr|  phase(deg)  SNR_phase(dB)  real-corr  rms(joc)")
for k in range(nj):
    if norm_j[k] < 1e-6:
        print(f"{k:3d}  silent")
        continue
    j = int(np.argmax(np.abs(C[k])))
    c = C[k, j]
    mag = abs(c)
    snr = 10 * np.log10(mag * mag / max(1 - mag * mag, 1e-12))
    rc = np.dot(jo[:, k], so[:, j]) / (np.linalg.norm(jo[:, k]) * np.linalg.norm(so[:, j]) + 1e-12)
    print(f"{k:3d} {j:4d} {mag:7.3f} {np.degrees(np.angle(c)):10.1f} {snr:13.1f} {rc:10.3f} {np.sqrt(np.mean(jo[:, k]**2)):9.5f}")
# how much of each source object is explained by all joc objects (complex least squares)
print()
print("source object explained by all JOC objects (complex regression): R^2 and the real-gain R^2")
for j in range(ns):
    y = A_s[:, j]
    X = A_j[:, norm_j > 1e-6]
    coef, *_ = np.linalg.lstsq(X, y, rcond=None)
    r = y - X @ coef
    r2 = 1 - np.vdot(r, r).real / max(np.vdot(y, y).real, 1e-30)
    Xr = jo[:, norm_j > 1e-6]
    cr, *_ = np.linalg.lstsq(Xr, so[:, j], rcond=None)
    rr = so[:, j] - Xr @ cr
    r2r = 1 - np.dot(rr, rr) / max(np.dot(so[:, j], so[:, j]), 1e-30)
    print(f"  src {j:2d}: complex R2 {r2:.3f} ({10*np.log10(1/max(1-r2,1e-12)):.1f} dB), real R2 {r2r:.3f}, rms {np.sqrt(np.mean(so[:, j]**2)):.5f}")
