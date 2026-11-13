#!/usr/bin/env python3
"""Compare two decodes of one AC-3 or E-AC-3 stream by frequency region.

Two decoders of a stream that uses the low-rate tools cannot agree sample for
sample over the whole band: the dither of a bin that got no bits, the noise
spectral extension blends in and the phase terms of enhanced coupling are each
decoder's own. They can agree closely where every bin is coded with bits, and
the regions a stream codes differently are known from its syntax: the bins
each channel codes itself, the region a coupling channel carries, the region
made by spectral extension (a bin is 93,75 Hz at 48 kHz). So the comparison
is made region by region:

    python tools/band_compare.py ours.wav reference.f32 --channels 2 \
        --edges 1200 7950 12450 --out compare.json

Either file is a WAVE file of 32-bit float (what `oadec decode --format wav`
writes) or headerless 32-bit float, little-endian, interleaved (what a
GStreamer `filesink` after `audio/x-raw,format=F32LE` writes). The reference
may be delayed and scaled against ours (a decoder that applies dialogue
normalisation attenuates by `31 - dialnorm` dB); both are measured in the
lowest region and taken out before anything is compared.

For every region and channel: the signal-to-noise ratio over the whole
stream, and the median and the fifth percentile of the ratio frame by frame
(1536 samples). Nothing is judged; the numbers go to the evidence that quotes
them.
"""

import argparse
import json
import sys

import numpy as np

FRAME = 1536


def read(path, channels):
    raw = open(path, 'rb').read()
    if raw[:4] in (b'RIFF', b'RF64'):
        at = raw.find(b'data')
        if at < 0:
            sys.exit(f'{path}: a WAVE file without a data chunk')
        size = int.from_bytes(raw[at + 4:at + 8], 'little')
        body = raw[at + 8:] if size in (0, 0xFFFFFFFF) else raw[at + 8:at + 8 + size]
    else:
        body = raw
    body = body[:len(body) // (4 * channels) * (4 * channels)]
    return np.frombuffer(body, dtype='<f4').reshape(-1, channels).astype(np.float64)


def snr(reference, ours):
    signal = float(np.sum(reference ** 2))
    noise = float(np.sum((reference - ours) ** 2))
    if signal == 0.0:
        return None
    if noise == 0.0:
        return 999.0
    return 10 * np.log10(signal / noise)


def region(x, lo, hi, rate):
    spectrum = np.fft.rfft(x, axis=0)
    hz = np.fft.rfftfreq(len(x), 1 / rate)
    spectrum[(hz < lo) | (hz >= hi)] = 0
    return np.fft.irfft(spectrum, n=len(x), axis=0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('ours')
    ap.add_argument('reference')
    ap.add_argument('--channels', type=int, required=True)
    ap.add_argument('--rate', type=int, default=48000)
    ap.add_argument('--edges', type=float, nargs='+', required=True,
                    help='the frequencies in Hz that part the band into regions')
    ap.add_argument('--max-delay', type=int, default=2048)
    ap.add_argument('--out', help='write the numbers as JSON')
    args = ap.parse_args()

    ours = read(args.ours, args.channels)
    ref = read(args.reference, args.channels)
    edges = [0.0] + sorted(args.edges) + [args.rate / 2]

    # delay and gain of the reference, from the lowest region of the first channel
    span = min(len(ours), len(ref) - args.max_delay, FRAME * 60)
    if span < FRAME * 8:
        sys.exit('the decodes are too short to align')
    low_ours = region(ours[:span + args.max_delay], edges[0], edges[1], args.rate)[:, 0]
    low_ref = region(ref[:span + args.max_delay], edges[0], edges[1], args.rate)[:, 0]
    a = low_ours[FRAME * 4:span]
    best = None
    for delay in range(args.max_delay + 1):
        b = low_ref[FRAME * 4 + delay:span + delay]
        gain = float(np.sum(a * b) / np.sum(a * a))
        s = snr(b, a * gain)
        if s is not None and (best is None or s > best[0]):
            best = (s, delay, gain)
    if best is None:
        sys.exit('the lowest region of the first channel is silent; nothing to align on')
    _, delay, gain = best
    n = min(len(ours), len(ref) - delay)
    ours = ours[:n] * gain
    ref = ref[delay:delay + n]
    frames = n // FRAME

    result = {
        'samples': n,
        'frames': frames,
        'reference_delay_samples': delay,
        'reference_gain': round(gain, 5),
        'reference_gain_db': round(20 * np.log10(abs(gain)), 2),
        'regions': [],
    }
    print(f'{n} samples, {frames} frames; the reference is {delay} samples late and '
          f'{result["reference_gain_db"]} dB against ours')
    for lo, hi in zip(edges[:-1], edges[1:]):
        o, r = region(ours, lo, hi, args.rate), region(ref, lo, hi, args.rate)
        row = {'from_hz': lo, 'to_hz': hi, 'channels': []}
        for ch in range(args.channels):
            # the first frame is the decoders' start-up and is left out
            whole = snr(r[FRAME:, ch], o[FRAME:, ch])
            per = [snr(r[k * FRAME:(k + 1) * FRAME, ch], o[k * FRAME:(k + 1) * FRAME, ch])
                   for k in range(1, frames)]
            per = np.array([p for p in per if p is not None])
            level = 10 * np.log10(float(np.mean(r[FRAME:, ch] ** 2)) + 1e-30)
            entry = {
                'channel': ch,
                'reference_level_db': round(level, 1),
                'snr_db': None if whole is None else round(whole, 1),
                'frame_median_db': None if len(per) == 0 else round(float(np.median(per)), 1),
                'frame_5th_percentile_db': None if len(per) == 0 else round(float(np.percentile(per, 5)), 1),
            }
            row['channels'].append(entry)
            print(f'{lo:7.0f} to {hi:7.0f} Hz, channel {ch}: level {entry["reference_level_db"]:6.1f} dB, '
                  f'whole {entry["snr_db"]} dB, frame median {entry["frame_median_db"]}, '
                  f'5th percentile {entry["frame_5th_percentile_db"]}')
        result['regions'].append(row)
    if args.out:
        with open(args.out, 'w', encoding='utf-8') as f:
            json.dump(result, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
