#!/usr/bin/env python3
"""Three-way comparison of AC-3 / E-AC-3 decodes: oadec, FFmpeg and a Dolby
decoder (DEE `ddp_decode` WAVE output).

Decoders disagree by their dither sequences (clause 6.3.4 leaves the noise
to the implementation), so sample equality is not the test. The test is
whether oadec sits inside the envelope the two independent decoders span:
its distance to the Dolby decode should not exceed the FFmpeg-to-Dolby
distance by more than a small margin, channel by channel.

    python tools/three_way.py ours.f32 ffmpeg.f32 dolby.wav [--channels 6] [--seconds 120]

`ours.f32` and `ffmpeg.f32` are interleaved 32-bit float little-endian in
WAVE order; the Dolby WAVE may carry a decoder delay and a gain, both of
which are measured and removed before the comparison.
"""

import argparse
import struct
import sys

import numpy as np


def read_wav(path, seconds):
    with open(path, "rb") as f:
        head = f.read(12)
        if head[:4] != b"RIFF":
            sys.exit(f"{path}: not RIFF")
        fmt = None
        while True:
            h = f.read(8)
            if len(h) < 8:
                break
            cid, size = struct.unpack("<4sI", h)
            if cid == b"fmt ":
                body = f.read(size + (size & 1))
                tag, ch, rate, _, block, bits = struct.unpack("<HHIIHH", body[:16])
                fmt = (tag, ch, rate, bits, block)
            elif cid == b"data":
                tag, ch, rate, bits, block = fmt
                want = min(size, int(seconds * rate) * block) if seconds else size
                pcm = f.read(want)
                if bits == 24:
                    a = np.frombuffer(pcm, dtype=np.uint8).reshape(-1, 3)
                    v = a[:, 0].astype(np.int32) | (a[:, 1].astype(np.int32) << 8) | (a[:, 2].astype(np.int32) << 16)
                    v = np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.float64) / (1 << 23)
                elif bits == 32:
                    v = np.frombuffer(pcm, dtype="<f4").astype(np.float64)
                elif bits == 16:
                    v = np.frombuffer(pcm, dtype="<i2").astype(np.float64) / 32768.0
                else:
                    sys.exit(f"{path}: unsupported {bits}-bit")
                return v.reshape(-1, ch), rate
            else:
                f.seek(size + (size & 1), 1)
    sys.exit(f"{path}: no data chunk")


def best_lag(x, y, maxlag=8192):
    n = 1 << int(np.ceil(np.log2(len(x) + len(y))))
    c = np.fft.irfft(np.fft.rfft(x, n) * np.conj(np.fft.rfft(y, n)), n)
    c = np.concatenate([c[-maxlag:], c[: maxlag + 1]])
    k = int(np.argmax(np.abs(c)))
    return k - maxlag


def snr_db(signal, error):
    e = float(np.dot(error, error))
    return float("inf") if e == 0 else 10 * np.log10(float(np.dot(signal, signal)) / e)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("ours")
    ap.add_argument("ffmpeg")
    ap.add_argument("dolby")
    ap.add_argument("--channels", type=int, default=6)
    ap.add_argument("--seconds", type=float, default=0, help="compare only the first N seconds (0 = all)")
    ap.add_argument("--names", default="")
    args = ap.parse_args()

    dolby, rate = read_wav(args.dolby, args.seconds)
    ch = dolby.shape[1]
    count = int(args.seconds * rate) * ch if args.seconds else -1
    ours = np.fromfile(args.ours, dtype="<f4", count=count).reshape(-1, ch).astype(np.float64)
    ff = np.fromfile(args.ffmpeg, dtype="<f4", count=count).reshape(-1, ch).astype(np.float64)
    names = args.names.split(",") if args.names else {
        1: ["C"], 2: ["L", "R"], 3: ["L", "R", "C"], 4: ["L", "R", "Ls", "Rs"],
        5: ["L", "R", "C", "Ls", "Rs"], 6: ["L", "R", "C", "LFE", "Ls", "Rs"],
    }.get(ch, [f"ch{i}" for i in range(ch)])

    # alignment of ours against ffmpeg (expected 0) and of dolby against ffmpeg
    probe = min(len(ff), rate * 30)
    ref_ch = int(np.argmax([np.dot(ff[:probe, c], ff[:probe, c]) for c in range(ch)]))
    lag_ours = best_lag(ours[:probe, ref_ch], ff[:probe, ref_ch])
    lag_dolby = best_lag(dolby[:probe, ref_ch], ff[:probe, ref_ch])
    print(f"channels {ch}, rate {rate}, alignment vs FFmpeg: ours {lag_ours:+d} samples, Dolby {lag_dolby:+d} samples")

    def align(a, lag):
        return a[lag:] if lag >= 0 else a[:lag]

    def align_ref(a, lag):
        return a[: len(a) - lag] if lag >= 0 else a[-lag:]

    d = align(dolby, lag_dolby)
    f_d = align_ref(ff, lag_dolby)
    o = align(ours, lag_ours)
    f_o = align_ref(ff, lag_ours)
    # common span: put everything on the ffmpeg time base
    start_d = max(-lag_dolby, 0)
    start_o = max(-lag_ours, 0)
    start = max(start_d, start_o)
    n = min(len(dolby) - max(lag_dolby, 0), len(ours) - max(lag_ours, 0), len(ff)) - start
    F = ff[start : start + n]
    D = dolby[start + lag_dolby : start + lag_dolby + n]
    O = ours[start + lag_ours : start + lag_ours + n]
    print(f"compared span: {n} samples ({n / rate:.1f} s)")
    print()
    print("channel  gain(Dolby)  SNR Dolby-vs-FF  SNR ours-vs-FF  SNR ours-vs-Dolby  max|ours-Dolby|  max|FF-Dolby|  verdict")
    worst_margin = 0.0
    for c in range(ch):
        g = float(np.dot(D[:, c], F[:, c]) / max(np.dot(F[:, c], F[:, c]), 1e-30))
        dg = D[:, c] / g if g != 0 else D[:, c]
        s_df = snr_db(dg, dg - F[:, c])
        s_of = snr_db(F[:, c], O[:, c] - F[:, c])
        s_od = snr_db(dg, O[:, c] - dg)
        margin = s_df - s_od  # positive when ours is farther from Dolby than FFmpeg is
        worst_margin = max(worst_margin, margin)
        verdict = "inside envelope" if s_od + 1.0 >= s_df else "OUTSIDE"
        print(
            f"{names[c]:<7} {g:11.5f} {s_df:16.1f} {s_of:15.1f} {s_od:18.1f} {np.abs(O[:, c] - dg).max():16.3e} {np.abs(F[:, c] - dg).max():14.3e}  {verdict}"
        )
    print()
    print(
        "RESULT:",
        "oadec is at least as close to the Dolby decode as FFmpeg is (within 1 dB) on every channel"
        if worst_margin <= 1.0
        else f"oadec is farther from the Dolby decode than FFmpeg by up to {worst_margin:.1f} dB",
    )
    return 0 if worst_margin <= 1.0 else 1


if __name__ == "__main__":
    sys.exit(main())
