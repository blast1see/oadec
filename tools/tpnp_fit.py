#!/usr/bin/env python3
"""Ask a reference decode where transient pre-noise processing puts its
corrections and which windows it fades them with.

Clause E.2.7 counts the transient from "the first decoded PCM sample contained
in the audio frame" and leaves the two cross-fade windows open. Both can be
read off a reference decode, because the correction is nothing but the
decode itself, copied and faded: decode the stream without the tool, apply
the correction of the clause under a hypothesis, and see how far the result
is from the reference inside the corrected regions. A hypothesis is where
the frame begins in the samples as oadec hands them out (0 is their first
sample, 256 one block in) and the shape of the windows (a straight line, a
raised cosine).

    python tools/tpnp_fit.py stream.ec3 reference.f32 --out fit.json

The reference is headerless 32-bit float or a WAVE file (16, 24 or 32 bit),
interleaved in the order `oadec decode --format wav` writes. Its delay and
its gain against our decode are measured on a stretch of the first channel
and taken out. The corrections are applied in the order the decoder applies
them, each one reading what the ones before it wrote.

Printed for every hypothesis: the error against the reference over all
corrected samples, in dB below the reference, and the same for the samples
of the two fades alone. Then, for the best one, the same with the end of
every correction moved by a few samples either way: a location that is
right has its minimum at zero.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np

import oadec_bin

FRAME, BLOCK, TC1, TC2 = 1536, 256, 256, 128


def read_audio(path, channels):
    raw = Path(path).read_bytes()
    if raw[:4] not in (b'RIFF', b'RF64'):
        body = raw[:len(raw) // (4 * channels) * (4 * channels)]
        return np.frombuffer(body, dtype='<f4').reshape(-1, channels).astype(np.float64)
    at = raw.find(b'fmt ')
    fmt = raw[at + 8:at + 8 + int.from_bytes(raw[at + 4:at + 8], 'little')]
    tag = int.from_bytes(fmt[0:2], 'little')
    bits = int.from_bytes(fmt[14:16], 'little')
    if tag == 0xFFFE and len(fmt) >= 26:
        # WAVE_FORMAT_EXTENSIBLE keeps the real tag in the sub-format GUID
        tag = int.from_bytes(fmt[24:26], 'little')
    at = raw.find(b'data')
    size = int.from_bytes(raw[at + 4:at + 8], 'little')
    body = raw[at + 8:] if size in (0, 0xFFFFFFFF) else raw[at + 8:at + 8 + size]
    step = bits // 8 * channels
    body = body[:len(body) // step * step]
    if bits == 32 and tag == 3:
        a = np.frombuffer(body, dtype='<f4').astype(np.float64)
    elif bits == 32:
        a = np.frombuffer(body, dtype='<i4').astype(np.float64) / 2147483648.0
    elif bits == 24:
        b = np.frombuffer(body, dtype=np.uint8).reshape(-1, 3).astype(np.int32)
        v = b[:, 0] | (b[:, 1] << 8) | (b[:, 2] << 16)
        a = np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.float64) / 8388608.0
    elif bits == 16:
        a = np.frombuffer(body, dtype='<i2').astype(np.float64) / 32768.0
    else:
        sys.exit(f'{path}: {bits}-bit WAVE is not handled')
    return a.reshape(-1, channels)


def align(ours, ref, span=2048):
    """The samples the reference is late by (negative: early) and its gain."""
    n = min(len(ours), len(ref)) - 2 * span
    n = min(n, FRAME * 400)
    a = ours[span:span + n, 0]
    best = None
    for lag in range(-span, span + 1):
        b = ref[span + lag:span + lag + n, 0]
        c = float(a @ b)
        if best is None or c > best[0]:
            best = (c, lag)
    lag = best[1]
    b = ref[span + lag:span + lag + n, 0]
    # The ratio of the energies, not the least-squares slope: each decode has
    # its own dither in the bins without bits, which a slope takes for signal
    # of ours alone and comes out some per cent low for.
    return lag, float(np.sqrt((b @ b) / (a @ a)) * np.sign(best[0]))


def windows(shape):
    def one(n):
        i = (np.arange(n) + 0.5) / n
        return i if shape == 'line' else 0.5 - 0.5 * np.cos(np.pi * i)
    return one(TC1), one(TC2)


def corrected(off, transients, origin, shape, moved=0):
    """`off` with every correction applied, and the samples each one wrote."""
    out = off.copy()
    w1, w2 = windows(shape)
    regions = []
    order = sorted(transients, key=lambda t: (t['frame'] * FRAME + t['loc'], t['channel']))
    for t in order:
        loc = t['frame'] * FRAME + origin + t['loc']
        pnlen = loc - (loc // BLOCK - 1) * BLOCK
        tot = pnlen + t['len'] + TC1
        src = loc - (2 * TC1 + 2 * pnlen)
        dst = loc - tot
        loc, src, dst = loc + moved, src + moved, dst + moved
        if src < 0 or loc > len(out):
            continue
        ch = out[:, t['channel']]
        synth = ch[src:src + tot].copy()
        ch[dst:dst + TC1] = ch[dst:dst + TC1] * (1 - w1) + synth[:TC1] * w1
        ch[dst + TC1:loc - TC2] = synth[TC1:tot - TC2]
        ch[loc - TC2:loc] = ch[loc - TC2:loc] * w2 + synth[tot - TC2:] * (1 - w2)
        regions.append((t['channel'], dst, loc))
    return out, regions


def below(ref, model, picks):
    """The error of `model` over the picked samples, in dB below the reference."""
    signal = noise = 0.0
    for ch, lo, hi in picks:
        signal += float(np.sum(ref[lo:hi, ch] ** 2))
        noise += float(np.sum((ref[lo:hi, ch] - model[lo:hi, ch]) ** 2))
    if signal == 0.0 or noise == 0.0:
        return None
    return round(10 * np.log10(signal / noise), 1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('stream')
    ap.add_argument('reference')
    ap.add_argument('--oadec', help='the decoder; found as tools/oadec_bin.py finds it otherwise')
    ap.add_argument('--out', help='write the numbers as JSON')
    args = ap.parse_args()
    args.oadec = args.oadec or oadec_bin.find()

    r = subprocess.run([args.oadec, 'verify', '--json', args.stream], capture_output=True, text=True,
                       encoding='utf-8')
    report = json.loads(r.stdout)
    channels = len(report['channels'])
    transients = report['transients']
    if not transients:
        sys.exit('the stream codes no transient')
    with tempfile.TemporaryDirectory() as tmp:
        wav = Path(tmp) / 'off.wav'
        subprocess.run([args.oadec, 'decode', '--format', 'wav', '--no-tpnp', '-o', str(wav), args.stream],
                       check=True, capture_output=True)
        off = read_audio(wav, channels)
        # A transient names its channel in the order the stream codes them (L C R
        # for three front channels); the decode and the reference are in WAVE
        # order (L R C). The same decode in the order of the stream says which
        # column is which, with no table of layouts to keep.
        subprocess.run([args.oadec, 'decode', '--format', 'wav', '--no-tpnp', '--order', 'stream', '-o', str(wav),
                        args.stream], check=True, capture_output=True)
        coded = read_audio(wav, channels)
    probe = slice(0, min(len(off), len(coded), FRAME * 200))
    column = []
    for c in range(channels):
        same = [j for j in range(channels) if np.array_equal(coded[probe, c], off[probe, j])]
        if len(same) < 1:
            sys.exit(f'coded channel {c} is no column of the decode in WAVE order')
        column.append(same[0])
    transients = [dict(t, channel=column[t['channel']]) for t in transients]
    ref = read_audio(args.reference, channels)
    lag, gain = align(off, ref)
    if lag >= 0:
        ref = ref[lag:]
    else:
        ref = np.concatenate([np.zeros((-lag, channels)), ref])
    ref = ref / gain
    n = min(len(off), len(ref))
    off, ref = off[:n], ref[:n]
    print(f'{len(transients)} transients in {report["tpnp_frames"]} frames; the reference is {lag} samples late, '
          f'gain {gain:.4f}')

    result = {'transients': len(transients), 'frames_with_a_transient': report['tpnp_frames'],
              'reference_delay_samples': lag, 'reference_gain': round(gain, 4), 'hypotheses': []}
    rows = []
    for origin in (0, BLOCK, 2 * BLOCK):
        for shape in ('line', 'raised cosine'):
            model, regions = corrected(off, transients, origin, shape)
            first = [(ch, lo, lo + TC1) for ch, lo, hi in regions]
            second = [(ch, hi - TC2, hi) for ch, lo, hi in regions]
            row = {
                'frame_begins_at_sample': origin,
                'windows': shape,
                'corrections': len(regions),
                'samples': sum(hi - lo for _, lo, hi in regions),
                'without_the_tool_db': below(ref, off, regions),
                'with_it_db': below(ref, model, regions),
                'first_fade_db': below(ref, model, first),
                'second_fade_db': below(ref, model, second),
            }
            rows.append(row)
            result['hypotheses'].append(row)
            print(f'frame begins at sample {origin:3d}, {shape:13s}: over the corrections {row["with_it_db"]} dB '
                  f'(without the tool {row["without_the_tool_db"]}), first fade {row["first_fade_db"]}, '
                  f'second fade {row["second_fade_db"]}')
    best = max(rows, key=lambda r: r['with_it_db'] if r['with_it_db'] is not None else -999)
    result['best'] = {'frame_begins_at_sample': best['frame_begins_at_sample'], 'windows': best['windows']}
    result['the_best_moved_by_samples'] = {}
    print(f'the best: the frame begins at sample {best["frame_begins_at_sample"]}, {best["windows"]}; moved by a few '
          'samples, the second fade:')
    for moved in range(-4, 5):
        model, regions = corrected(off, transients, best['frame_begins_at_sample'], best['windows'], moved)
        second = [(ch, hi - TC2, hi) for ch, lo, hi in regions]
        value = below(ref, model, second)
        result['the_best_moved_by_samples'][f'{moved:+d}'] = value
        print(f'   {moved:+d}: {value} dB')
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=1), encoding='utf-8')
    return 0


if __name__ == '__main__':
    sys.exit(main())
