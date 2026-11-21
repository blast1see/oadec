#!/usr/bin/env python3
"""What another decoder does to the coupled region of a stream, read off what
it writes.

Above the coupling begin frequency the channels of an AC-3 or E-AC-3 stream
are one channel, the coupling channel, times a coordinate for each of them
(ATSC A/52 clause 7.4). A decoder that does only that writes, for a coupled
coefficient, what this decoder writes. Dolby's decoders do more to an E-AC-3
stream: what they write for a coupled coefficient `X` of this decoder is

    a X + b D(X),    a^2 + b^2 = 1,

where `D` is a filter along the blocks, the same coefficient in the blocks
before, and `a` and `b` are another pair for every block and every coupling
sub-band of twelve coefficients. This tool finds the filter and the pairs
from two decodes of one stream, and says how much of the other decoder's
coefficients they give.

    python tools/coupling_mix.py signals <directory>

writes the test signals the finding was made with, thirty-four 16-bit WAVE
files of twelve seconds: a band of noise both channels share above 5 kHz
(what an encoder at a low rate couples), a tone of 440 Hz under it, and one
thing apart from file to file (levels that move, noise of the channels' own
under the coupling begin frequency, ...). The first fifteen found the
decorrelation; the other nineteen, one thing at a time, what its weight
follows, and are what the weight of the experimental mode was fitted and
held out on (`docs/eac3.md`). `ffmpeg -i low-both.wav -c:a eac3 -b:a 128k
-f eac3 low-both.ec3` and the same with `ac3` make the streams.

    oadec eac3-blocks --frame 100 stream.ec3      (cplbegf, cplendf)
    python tools/coupling_mix.py regions reference.f32 ours.wav --channels 2 --first 49 --sub-bands 8

says, for every channel, how the reference stands to this decoder's decode in
the channel's own region and in the coupled region. `reference.f32` is the
other decoder's decode as 32-bit float, the channels interleaved in the order
of a WAVE file; `ours.wav` is `oadec decode`. The first coupled coefficient is
`37 + 12 * cplbegf` and the sub-bands are `cplendf + 3 - cplbegf` of them.

    python tools/coupling_mix.py mix reference.f32 ours.wav --channels 2 --first 49 --sub-bands 8 --out mix.json

finds the filter and the mix. The filter is fitted on the channel whose
coupled region is furthest from ours (`--learn` names another): first with one
mix for all blocks, then by turns the mix of every block and sub-band with the
filter held and the filter with the mixes held. It is then read as an all-pass
of three poles after a delay of one block, and the report says by how much
that misses the fitted taps. For every channel: the mean of `a` and of `b` by
sub-band, how far `a^2 + b^2` is from 1, the share of the blocks that take no
filtered part, and what is left of the reference once `a X + b D(X)` with the
pair of each block and sub-band is taken out.

`ours.wav` should be a decode with the dither on when the other decoder draws
the dither this one draws: the filtered part is made of the coefficients
without bits as well.

    python tools/coupling_mix.py measure reference.f32 ours.wav --channels 2 --first 49 --sub-bands 8 --out measure.json

sets a decode of this decoder with its experimental decorrelation
(`oadec_eac3::decorr`, which is not Dolby's rule and is not claimed to equal
Dolby's decode) against a reference decode of the same stream, and reports,
for every channel and every coupling sub-band of the coupled region, figures
that say how far the two stand apart and in what way. Both decodes are taken
back to coefficients as `regions` takes them; the first `--skip` blocks (300
by default, as there) are left out. With `--plain`, a decode of this decoder
without the decorrelation, it also reports how far each of the two moved the
power of the plain decode. The figures, under the names the JSON report gives
them:

- `ours_over_plain_db` and `reference_over_plain_db` (with `--plain`): the
  power of each decode in the sub-band over that of the plain decode, summed
  over the blocks and the twelve coefficients, so that what the
  decorrelation of each decoder does to the level of a channel can be put
  side by side.

- `power_difference_db`: ten times the logarithm of the power of ours over
  the power of the reference, summed over the blocks and the twelve
  coefficients of the sub-band. A decorrelation that keeps the level of a
  channel stands near 0.
- `spectral_difference_db`: the largest of the same figure taken coefficient
  by coefficient within the sub-band (each summed over the blocks), which
  says whether the power is where it should be inside the sub-band and not
  only in sum.
- `pairs`: for every pair of channels that take the filter with opposite
  signs (`--pairs`; left and right, the surround pair), by sub-band:
  `coherence_ours` and `coherence_reference`, the magnitude of the
  normalised cross-spectrum of the two channels over the blocks, in each
  decode, and `coherence_difference`, ours less the reference, whose target
  is 0.10 or less in magnitude; `phase_ours_degrees` and
  `phase_reference_degrees`, the angle of the same cross-spectrum, which says
  whether the two channels lead or lag each other as the reference has them.
  The cross-spectrum is of the discrete Fourier transform of the same
  windowed blocks of 512 samples, whose bin `k` lies half a bin under the
  coefficient `k`; the coefficients are real and carry no phase.
- `pre_echo_db`, by channel: in the blocks where the reference has a
  transient in the coupled region (the energy of the block over that of the
  block before by more than `--transient-db`, 10 dB by default), the energy
  ours has in the coupled region of the block before over the energy the
  reference has there, summed over those blocks. A filter along the blocks
  smears a transient forward, not back; a figure above 0 says ours puts
  energy before it. `null` where the reference has no transient.
- `boundary_discontinuity_db`, by channel: the power of the step from the
  last sample of a block of 256 to the first of the next, summed over the
  decode, of ours over the reference, for the coupled region alone (each
  decode taken back to samples from its coupled coefficients). A
  decorrelation that steps at the blocks stands above 0.

The report is JSON, and the same two decodes give the same report.

    python tools/coupling_mix.py --self-check

makes two decodes of which the answer is known and reads it back.

The decoder under study is run as it stands and only what it writes is read.
Exit code: 0, 1 when two decodes cannot be compared as asked, 2 on a usage
error.
"""

import argparse
import json
import struct
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from dither_probe import HALF, N, WINDOW, coefficients, read_decode, samples  # noqa: E402

RATE = 48000
SECONDS = 12
SUB_BAND = 12  # coefficients of a coupling sub-band
SKIP = 300  # blocks left out at the start of a comparison: the decoders start differently


# ---- the test signals

def band_noise(seed, low, high):
    """Noise between two frequencies, its largest sample 1."""
    rng = np.random.default_rng(seed)
    n = RATE * SECONDS
    spectrum = np.fft.rfft(rng.standard_normal(n))
    freq = np.fft.rfftfreq(n, 1 / RATE)
    spectrum[(freq < low) | (freq > high)] = 0
    x = np.fft.irfft(spectrum, n)
    return x / np.abs(x).max()


def write_wave(path, left, right):
    data = np.stack([left, right], axis=1)
    data = np.clip(np.round(data * 32767 * 0.5), -32768, 32767).astype('<i2')
    body = data.tobytes()
    with open(path, 'wb') as f:
        f.write(b'RIFF' + struct.pack('<I', 36 + len(body)) + b'WAVEfmt '
                + struct.pack('<IHHIIHH', 16, 1, 2, RATE, RATE * 4, 4, 16))
        f.write(b'data' + struct.pack('<I', len(body)) + body)


def signals(out):
    out.mkdir(parents=True, exist_ok=True)
    n = RATE * SECONDS
    t = np.arange(n) / RATE
    tone = 0.2 * np.sin(2 * np.pi * 440 * t)
    shared, other = band_noise(1, 5000, 14000), band_noise(2, 5000, 14000)
    made = {}

    def put(name, left, right, what):
        write_wave(out / f'{name}.wav', left, right)
        made[name] = what

    put('still', tone + 0.6 * shared, tone + 0.3 * shared, 'the shared band at levels that stay')
    both = 0.55 + 0.45 * np.sin(2 * np.pi * 8 * t)
    put('tremolo', tone + 0.6 * both * shared, tone + 0.3 * both * shared,
        'both channels louder and softer together, 8 times a second')
    for name, rate, often in (('pan8', 8.0, '8 times a second'), ('pan1', 1.0, 'once a second')):
        swing = 0.5 + 0.45 * np.sin(2 * np.pi * rate * t)
        put(name, tone + 0.6 * swing * shared, tone + 0.6 * (1 - swing) * shared,
            f'the level going from one channel to the other, {often}')
    gate = ((t % 0.2) < 0.05).astype(float)
    put('bursts', tone + 0.6 * gate * shared, tone + 0.3 * gate * shared, 'on for 50 ms and off for 150')
    rng = np.random.default_rng(7)
    left, right = tone.copy(), tone.copy()
    for f in rng.uniform(5000, 14000, 12):
        s = np.sin(2 * np.pi * f * t + rng.uniform(0, 6.28))
        left += 0.05 * rng.uniform(0.2, 1.0) * s
        right += 0.05 * rng.uniform(0.2, 1.0) * s
    put('tones', left, right, 'twelve sines between 5 and 14 kHz in place of the noise')
    put('apart', tone + 0.6 * shared, tone + 0.6 * other, 'different noise in the two channels above 5 kHz')
    swing = 0.5 + 0.45 * np.sin(2 * np.pi * 8 * t)
    put('apart8', tone + 0.6 * swing * shared, tone + 0.6 * (1 - swing) * other,
        'different noise, each channel louder as the other is softer, 8 times a second')
    hold = RATE // 10
    levels = np.repeat(rng.uniform(0.1, 1.0, (n // hold + 1, 2)), hold, axis=0)[:n]
    put('steps', tone + 0.6 * levels[:, 0] * shared, tone + 0.6 * levels[:, 1] * shared,
        'the levels of the two channels drawn anew every 100 ms')
    a, b = band_noise(11, 3500, 4500), band_noise(12, 3500, 4500)
    put('low-both', tone + 0.5 * shared + 0.3 * a, tone + 0.25 * shared + 0.3 * b,
        'different noise from 3,5 to 4,5 kHz in each channel, under the coupling begin frequency')
    put('low-left', tone + 0.5 * shared + 0.3 * a, tone + 0.25 * shared, 'such noise in the left channel only')
    m1, m2 = band_noise(13, 1000, 2000), band_noise(14, 1000, 2000)
    put('mid-both', tone + 0.5 * shared + 0.3 * m1, tone + 0.25 * shared + 0.3 * m2,
        'different noise from 1 to 2 kHz in each channel')
    put('low-same', tone + 0.5 * shared + 0.3 * a, tone + 0.25 * shared + 0.3 * a,
        'the same noise from 3,5 to 4,5 kHz in both channels')
    put('low-quiet', tone + 0.5 * shared + 0.01 * a, tone + 0.25 * shared + 0.01 * b, 'as low-both, 30 dB lower')
    put('low-only', tone + 0.3 * a, tone + 0.3 * b, 'as low-both, with nothing above 5 kHz')

    # What the weight of Dolby's decorrelation follows, one thing at a time
    # (docs/eac3.md): the level of a channel's own noise, the tone, the level
    # of the coupled region, and how alike the two channels' own content is.
    for level, tag in ((0.1, '10'), (0.03, '03'), (0.6, '60')):
        put(f'left-n{tag}', tone + 0.5 * shared + level * a, tone + 0.25 * shared,
            f'as low-left, the noise at {level} in place of 0,3')
    put('left-notone', 0.5 * shared + 0.3 * a, 0.25 * shared, 'as low-left, without the tone')
    put('left-eqcpl', tone + 0.4 * shared + 0.3 * a, tone + 0.4 * shared,
        'as low-left, the shared band at the same level in both channels')
    put('same-eqcpl', tone + 0.4 * shared + 0.3 * a, tone + 0.4 * shared + 0.3 * a,
        'the two channels the same')
    put('same-r125', tone + 0.5 * shared + 0.3 * a, tone + 0.125 * shared + 0.3 * a,
        'as low-same, the shared band 12 dB lower in the right channel')
    put('both-eqcpl', tone + 0.4 * shared + 0.3 * a, tone + 0.4 * shared + 0.3 * b,
        'as low-both, the shared band at the same level in both channels')
    for degrees in (30, 60):
        turn = np.radians(degrees)
        put(f'both-phi{degrees}', tone + 0.4 * shared + 0.3 * a,
            tone + 0.4 * shared + 0.3 * (np.cos(turn) * a + np.sin(turn) * b),
            f'the right channel\'s noise {degrees} degrees from the left\'s')
    put('both-r03', tone + 0.4 * shared + 0.3 * a, tone + 0.4 * shared + 0.03 * b,
        'as both-eqcpl, the right channel\'s noise 20 dB lower')
    w1, w2 = band_noise(21, 500, 4500), band_noise(22, 500, 4500)
    put('wide-both', tone + 0.4 * shared + 0.3 * w1, tone + 0.4 * shared + 0.3 * w2,
        'different noise from 0,5 to 4,5 kHz in each channel')
    put('both-cpl08', tone + 0.8 * shared + 0.3 * a, tone + 0.8 * shared + 0.3 * b,
        'as both-eqcpl, the shared band 6 dB higher')
    put('both-cpl02', tone + 0.2 * shared + 0.3 * a, tone + 0.2 * shared + 0.3 * b,
        'as both-eqcpl, the shared band 6 dB lower')

    def sine(f):
        return 0.2 * np.sin(2 * np.pi * f * t)

    put('tone2k', sine(2000) + 0.5 * shared, sine(2000) + 0.25 * shared, 'a tone of 2 kHz in place of 440 Hz')
    put('tone2k3k', sine(2000) + 0.5 * shared, sine(3000) + 0.25 * shared,
        'a tone of 2 kHz in the left channel and of 3 kHz in the right')
    put('tone2k-left', sine(2000) + 0.5 * shared, 0.25 * shared, 'a tone of 2 kHz in the left channel only')
    put('chord-both', sine(1500) + sine(2500) + sine(3700) + 0.5 * shared,
        sine(1700) + sine(2900) + sine(4100) + 0.25 * shared, 'three other tones in each channel')
    put('noise-tone', sine(2000) + 0.5 * shared + 0.3 * a, sine(3000) + 0.25 * shared + 0.3 * b,
        'tone2k3k with low-both\'s noise')
    return made


# ---- two decodes, as coefficients

def as_coefficients(reference, ours, lag):
    """The coefficients of every channel of both decodes over the blocks they
    share, the reference starting `lag` samples into ours."""
    n = min(len(reference), len(ours) - lag) // 256 * 256
    if n < (SKIP + 60) * 256:
        sys.exit('too little to compare: the decodes share less than 360 blocks')
    channels = reference.shape[1]
    theirs = [coefficients(reference[:n, ch]) for ch in range(channels)]
    mine = [coefficients(ours[lag:lag + n, ch]) for ch in range(channels)]
    return theirs, mine


def ratio_db(signal, rest):
    if signal <= 0:
        return None
    return 200.0 if rest <= 0 else round(float(10 * np.log10(signal / rest)), 1)


def regions(theirs, mine, first, sub_bands):
    last = first + SUB_BAND * sub_bands
    rows = []
    for ch, (r, m) in enumerate(zip(theirs, mine)):
        r, m = r[SKIP:], m[SKIP:]
        own = ratio_db(float((r[:, 8:first] ** 2).sum()), float(((r - m)[:, 8:first] ** 2).sum()))
        coupled = ratio_db(float((r[:, first:last] ** 2).sum()), float(((r - m)[:, first:last] ** 2).sum()))
        rows.append({'channel': ch, 'own_region_db': own, 'coupled_region_db': coupled})
    return rows


# ---- the filter and the mix

def lagged(m, lo, hi, taps, count):
    """Our coefficients of a range in the block and in the blocks before it:
    blocks x coefficients x taps."""
    return np.stack([m[SKIP - lag:SKIP - lag + count, lo:hi] for lag in range(taps)], axis=2)


def pairs(x, z, y):
    """The pair (a, b) of every block that takes y nearest to a x + b z."""
    xx, zz, xz = (x * x).sum(1), (z * z).sum(1), (x * z).sum(1)
    xy, zy = (x * y).sum(1), (z * y).sum(1)
    det = xx * zz - xz * xz
    safe = det > 1e-12 * np.maximum(xx * zz, 1e-300)
    det = np.where(safe, det, 1.0)
    a = np.where(safe, (xy * zz - zy * xz) / det, np.nan)
    b = np.where(safe, (zy * xx - xy * xz) / det, np.nan)
    return a, b


def through_all_pass(x, denominator):
    """Blocks of coefficients through z^-1 (c3 + c2 z^-1 + c1 z^-2 + z^-3) /
    (1 + c1 z^-1 + c2 z^-2 + c3 z^-3), each coefficient along the blocks."""
    den = np.concatenate([[1.0], denominator])
    num = den[::-1]
    y = np.zeros_like(x)
    for n in range(len(x)):
        acc = num[0] * x[n]
        for k in range(1, 4):
            if n - k >= 0:
                acc = acc + num[k] * x[n - k] - den[k] * y[n - k]
        y[n] = acc
    return np.vstack([np.zeros((1, x.shape[1])), y[:-1]])


def mixes(r, m, d, first, sub_bands, count, denominator=None):
    """The pair of every block and sub-band. The filtered part is made with
    the taps `d`, or with the all-pass of `denominator` when one is given."""
    taps = len(d)
    a = np.empty((count, sub_bands))
    b = np.empty((count, sub_bands))
    signal = np.zeros(sub_bands)
    rest = np.zeros(sub_bands)
    for s in range(sub_bands):
        lo = first + SUB_BAND * s
        y = r[SKIP:SKIP + count, lo:lo + SUB_BAND]
        if denominator is None:
            stack = lagged(m, lo, lo + SUB_BAND, taps, count)
            x, z = stack[:, :, 0], stack @ d
        else:
            x = m[SKIP:SKIP + count, lo:lo + SUB_BAND]
            z = through_all_pass(m[:SKIP + count, lo:lo + SUB_BAND], denominator)[SKIP:]
        a[:, s], b[:, s] = pairs(x, z, y)
        good = np.isfinite(a[:, s]) & (np.abs(a[:, s]) < 3) & (np.abs(b[:, s]) < 3)
        left = y[good] - a[good, s][:, None] * x[good] - b[good, s][:, None] * z[good]
        signal[s], rest[s] = float((y[good] ** 2).sum()), float((left ** 2).sum())
        a[~good, s] = np.nan
        b[~good, s] = np.nan
    return a, b, signal, rest


def one_mix(r, m, d, first, sub_bands, count, denominator=None):
    """One pair (a, b) for all blocks and sub-bands of a channel, and what it
    leaves: what a channel takes of the filtered part on the whole, also
    where the pairs of single blocks cannot be trusted."""
    lo, hi = first, first + SUB_BAND * sub_bands
    y = r[SKIP:SKIP + count, lo:hi]
    if denominator is None:
        stack = lagged(m, lo, hi, len(d), count)
        x, z = stack[:, :, 0], stack @ d
    else:
        x = m[SKIP:SKIP + count, lo:hi]
        z = through_all_pass(m[:SKIP + count, lo:hi], denominator)[SKIP:]
    pair, *_ = np.linalg.lstsq(np.stack([x.ravel(), z.ravel()], axis=1), y.ravel(), rcond=None)
    left = y - pair[0] * x - pair[1] * z
    return {'direct_weight': round(float(pair[0]), 4), 'filtered_weight': round(float(pair[1]), 4),
            'left_db': ratio_db(float((y ** 2).sum()), float((left ** 2).sum()))}


def learn(r, m, first, sub_bands, taps, rounds):
    """The filter along the blocks, at the level of what it is made of."""
    count = len(r) - SKIP - 2
    lo, hi = first, first + SUB_BAND * sub_bands
    stack = lagged(m, lo, hi, taps, count)
    y = r[SKIP:SKIP + count, lo:hi]
    one, *_ = np.linalg.lstsq(stack.reshape(-1, taps), y.ravel(), rcond=None)
    d = one.copy()
    d[0] = 0.0
    if float(d @ d) < 1e-6:
        return None
    d /= np.sqrt(float(d @ d))
    for _ in range(rounds):
        a, b, _, _ = mixes(r, m, d, first, sub_bands, count)
        good = np.isfinite(a).all(axis=1)
        wide_a = np.repeat(a[good], SUB_BAND, axis=1)
        wide_b = np.repeat(b[good], SUB_BAND, axis=1)
        target = y[good] - wide_a * stack[good][:, :, 0]
        regress = wide_b[:, :, None] * stack[good][:, :, 1:]
        new, *_ = np.linalg.lstsq(regress.reshape(-1, taps - 1), target.ravel(), rcond=None)
        d = np.concatenate([[0.0], new])
        d /= np.sqrt(float(d @ d))
    return d


def all_pass(d):
    """The filter read as z^-1 (c3 + c2 z^-1 + c1 z^-2 + z^-3) / (1 + c1 z^-1 +
    c2 z^-2 + c3 z^-3): the denominator from the tail of the taps, its poles,
    by how much the all-pass of those misses the taps, and whether the taps
    are that all-pass or its negative (a channel may take it either way)."""
    h = d[1:]
    rows = [[h[n - 1], h[n - 2], h[n - 3]] for n in range(4, min(30, len(h)))]
    rhs = [-h[n] for n in range(4, min(30, len(h)))]
    c, *_ = np.linalg.lstsq(np.array(rows), np.array(rhs), rcond=None)
    den = np.array([1.0, c[0], c[1], c[2]])
    num = den[::-1]
    made = np.zeros(len(h))
    for n in range(len(h)):
        acc = num[n] if n < 4 else 0.0
        acc -= sum(den[k] * made[n - k] for k in range(1, 4) if n - k >= 0)
        made[n] = acc
    sign = 1.0 if float(np.abs(made - h).max()) <= float(np.abs(made + h).max()) else -1.0
    roots = np.roots(den)
    real = sorted(float(z.real) for z in roots if abs(z.imag) < 1e-9)
    pair = [z for z in roots if z.imag > 1e-9]
    return {
        'denominator_unrounded': [float(v) for v in c],
        'denominator': [round(float(v), 4) for v in c],
        'real_poles': [round(v, 4) for v in real],
        'pair': None if not pair else {'radius': round(float(abs(pair[0])), 4),
                                       'angle_degrees': round(float(np.degrees(np.angle(pair[0]))), 2)},
        'largest_tap_it_misses_by': round(float(np.abs(made - sign * h).max()), 6),
    }, sign


def study(theirs, mine, first, sub_bands, taps=40, rounds=3, learn_from=None):
    last = first + SUB_BAND * sub_bands
    count = len(theirs[0]) - SKIP - 2
    apart = regions(theirs, mine, first, sub_bands)
    if learn_from is None:
        heard = [row for row in apart if row['coupled_region_db'] is not None]
        if not heard:
            sys.exit('no channel has anything in the coupled region')
        learn_from = min(heard, key=lambda row: row['coupled_region_db'])['channel']
    d = learn(theirs[learn_from], mine[learn_from], first, sub_bands, taps, rounds)
    report = {
        'blocks': count, 'first_block': SKIP, 'coupled_coefficients': [first, last - 1], 'sub_bands': sub_bands,
        'the_filter': None, 'channels': [],
    }
    if d is None:
        report['the_filter'] = {'learned_from_channel': learn_from, 'found': False}
        for row in apart:
            report['channels'].append({**row, 'direct_weight_mean_by_sub_band': None})
        return report
    shape, sign = all_pass(d)
    # the filtered weights are counted against the all-pass as it is written
    d = sign * d
    report['the_filter'] = {
        'learned_from_channel': learn_from, 'found': True,
        'taps_from_one_block_back': [round(float(v), 4) for v in d[1:17]],
        'sum_of_their_squares': round(float(d @ d), 4),
        'as_an_all_pass_of_three_poles_after_one_block': shape,
    }
    # where the taps are that all-pass, the filtered part is made with it: the taps stop after `taps` blocks
    # and the all-pass does not
    exact = np.array(shape.pop('denominator_unrounded'))
    if shape['largest_tap_it_misses_by'] >= 0.005:
        exact = None
    report['the_filter']['the_filtered_part_is_made_with'] = 'the fitted taps' if exact is None else 'the all-pass'
    for row in apart:
        ch = row['channel']
        if row['coupled_region_db'] is None:
            report['channels'].append({**row, 'direct_weight_mean_by_sub_band': None})
            continue
        a, b, signal, rest = mixes(theirs[ch], mine[ch], d, first, sub_bands, count, exact)
        squares = a * a + b * b
        report['channels'].append({
            **row,
            'with_one_mix_for_all_blocks': one_mix(theirs[ch], mine[ch], d, first, sub_bands, count, exact),
            'direct_weight_mean_by_sub_band': [round(float(v), 4) for v in np.nanmean(a, axis=0)],
            'direct_weight_spread_by_sub_band': [round(float(v), 4) for v in np.nanstd(a, axis=0)],
            'filtered_weight_mean_by_sub_band': [round(float(v), 4) for v in np.nanmean(b, axis=0)],
            'squares_of_the_two_summed_median': round(float(np.nanmedian(squares)), 4),
            'blocks_without_a_filtered_part': round(float(np.nanmean(np.abs(b) < 0.02)), 4),
            'left_with_the_mix_of_each_block_db': [ratio_db(s, e) for s, e in zip(signal, rest)],
        })
    return report


# ---- a decorrelated decode against a reference

def figure_db(num, den):
    if den <= 0 or num <= 0:
        return None if num == den == 0 else (200.0 if den <= 0 else -200.0)
    return round(float(10 * np.log10(num / den)), 3)


def spectra(pcm, lag, count):
    """The discrete Fourier transform of every windowed block of 512 samples
    of one channel, from `lag` on, bins 0 to 255."""
    out = np.empty((count, HALF), dtype=complex)
    for b in range(count):
        out[b] = np.fft.rfft(pcm[lag + b * HALF:lag + b * HALF + N] * WINDOW)[:HALF]
    return out


def measure(reference, ours, plain, first, sub_bands, lag, pairs, transient_db, skip):
    """The figures of `measure`, described at the head of the file."""
    channels = reference.shape[1]
    n = min(len(reference), len(ours) - lag) // HALF * HALF
    if plain is not None:
        n = min(n, (len(plain) - lag) // HALF * HALF)
    blocks = (n - N) // HALF + 1
    if blocks <= skip + 8:
        return None
    last = first + SUB_BAND * sub_bands
    r = [coefficients(reference[:n, ch])[skip:] for ch in range(channels)]
    m = [coefficients(ours[lag:lag + n, ch])[skip:] for ch in range(channels)]
    p = None if plain is None else [coefficients(plain[lag:lag + n, ch])[skip:] for ch in range(channels)]
    bands = [(first + SUB_BAND * s, first + SUB_BAND * (s + 1)) for s in range(sub_bands)]
    report = {'blocks': blocks - skip, 'first': first, 'sub_bands': sub_bands, 'channels': [], 'pairs': []}
    for ch in range(channels):
        row = {'channel': ch, 'power_difference_db': [], 'spectral_difference_db': []}
        if p is not None:
            row['ours_over_plain_db'] = []
            row['reference_over_plain_db'] = []
        for lo, hi in bands:
            pm = (m[ch][:, lo:hi] ** 2).sum(axis=0)
            pr = (r[ch][:, lo:hi] ** 2).sum(axis=0)
            row['power_difference_db'].append(figure_db(float(pm.sum()), float(pr.sum())))
            each = [figure_db(float(a), float(b)) for a, b in zip(pm, pr)]
            each = [v for v in each if v is not None]
            row['spectral_difference_db'].append(max(each, key=abs) if each else None)
            if p is not None:
                pp = float((p[ch][:, lo:hi] ** 2).sum())
                row['ours_over_plain_db'].append(figure_db(float(pm.sum()), pp))
                row['reference_over_plain_db'].append(figure_db(float(pr.sum()), pp))
        er = (r[ch][:, first:last] ** 2).sum(axis=1)
        em = (m[ch][:, first:last] ** 2).sum(axis=1)
        hits = [b for b in range(1, len(er)) if er[b] > er[b - 1] * 10 ** (transient_db / 10) and er[b] > 0]
        row['transients'] = len(hits)
        row['pre_echo_db'] = (figure_db(float(sum(em[b - 1] for b in hits)), float(sum(er[b - 1] for b in hits)))
                              if hits else None)
        steps = []
        for c in (m[ch], r[ch]):
            only = np.zeros_like(c)
            only[:, first:last] = c[:, first:last]
            y = samples(only)[HALF:-HALF]
            at = np.arange(HALF, len(y), HALF)
            steps.append(float(((y[at] - y[at - 1]) ** 2).sum()))
        row['boundary_discontinuity_db'] = figure_db(steps[0], steps[1])
        report['channels'].append(row)
    count = blocks - skip
    for a, b in pairs:
        if a >= channels or b >= channels:
            return None
        fr = [spectra(reference[:, ch], 0, blocks)[skip:skip + count] for ch in (a, b)]
        fm = [spectra(ours[:, ch], lag, blocks)[skip:skip + count] for ch in (a, b)]
        row = {'pair': [a, b], 'coherence_ours': [], 'coherence_reference': [], 'coherence_difference': [],
               'phase_ours_degrees': [], 'phase_reference_degrees': []}
        for lo, hi in bands:
            values = []
            for x, y in (fm, fr):
                cross = complex((x[:, lo:hi] * np.conj(y[:, lo:hi])).sum())
                power = float((np.abs(x[:, lo:hi]) ** 2).sum() * (np.abs(y[:, lo:hi]) ** 2).sum())
                values.append((abs(cross) / np.sqrt(power) if power > 0 else None, float(np.degrees(np.angle(cross)))))
            (co, po), (cr, pr_) = values
            row['coherence_ours'].append(None if co is None else round(float(co), 4))
            row['coherence_reference'].append(None if cr is None else round(float(cr), 4))
            row['coherence_difference'].append(None if co is None or cr is None else round(float(co - cr), 4))
            row['phase_ours_degrees'].append(round(po, 2))
            row['phase_reference_degrees'].append(round(pr_, 2))
        report['pairs'].append(row)
    return report


def parse_pairs(text):
    try:
        return [tuple(int(v) for v in part.split(':')) for part in text.split(',') if part]
    except ValueError:
        return None


# ---- a known answer

def self_check():
    """Three channels of which the first two are filtered and mixed and the
    third is not, through the transform both ways, read back."""
    rng = np.random.default_rng(3)
    blocks, first, sub_bands = 1500, 49, 4
    last = first + SUB_BAND * sub_bands
    den = np.array([1.0, -0.0944, 0.0242, -0.3957])
    num = den[::-1]
    coupling = rng.standard_normal((blocks, last - first)) * 0.01

    def filtered(x):
        """z^-1 times the all-pass, along the blocks."""
        y = np.zeros_like(x)
        for n in range(len(x)):
            acc = sum(num[k] * x[n - k] for k in range(4) if n - k >= 0)
            acc -= sum(den[k] * y[n - k] for k in range(1, 4) if n - k >= 0)
            y[n] = acc
        return np.vstack([np.zeros((1, x.shape[1])), y[:-1]])

    ours, theirs, wanted = [], [], []
    for ch, (coordinate, sign) in enumerate(((0.7, 1.0), (0.4, -1.0), (0.9, 0.0))):
        own = rng.standard_normal((blocks, first)) * 0.01
        x = coordinate * coupling
        a = np.ones((blocks, sub_bands)) if sign == 0.0 else rng.uniform(0.55, 0.95, (blocks, sub_bands))
        b = sign * np.sqrt(1 - a * a)
        z = filtered(x)
        mixed = np.repeat(a, SUB_BAND, axis=1) * x + np.repeat(b, SUB_BAND, axis=1) * z
        zero = np.zeros((blocks, 256 - last))
        ours.append(samples(np.hstack([own, x, zero])))
        theirs.append(samples(np.hstack([own, mixed, zero])))
        wanted.append((a[SKIP:SKIP + blocks - SKIP - 4].mean(axis=0), sign))
    ours = np.stack(ours, axis=1)
    reference = np.stack(theirs, axis=1)[256:].astype('<f4').astype(np.float64)
    theirs_c, mine_c = as_coefficients(reference, ours, 256)
    apart = regions(theirs_c, mine_c, first, sub_bands)
    report = study(theirs_c, mine_c, first, sub_bands)
    shape = report['the_filter']['as_an_all_pass_of_three_poles_after_one_block']
    ch = report['channels']
    checks = {
        'the own regions are the same in both decodes': all(row['own_region_db'] > 100 for row in apart),
        'the coupled regions of the mixed channels are not': all(row['coupled_region_db'] < 15 for row in apart[:2]),
        'the coupled region of the plain channel is': apart[2]['coupled_region_db'] > 100,
        'the filter is learned from a mixed channel': report['the_filter']['learned_from_channel'] in (0, 1),
        'it is the all-pass that was put in':
            float(np.abs(np.array(shape['denominator']) - den[1:]).max()) < 0.002 and shape['largest_tap_it_misses_by'] < 0.002,
        'the direct weights come back':
            all(float(np.abs(np.array(ch[k]['direct_weight_mean_by_sub_band']) - wanted[k][0]).max()) < 0.01 for k in range(3)),
        'the first channel takes the filtered part as it is and the second its negative':
            ch[0]['filtered_weight_mean_by_sub_band'][0] > 0 > ch[1]['filtered_weight_mean_by_sub_band'][0],
        'the squares of a pair sum to 1': all(abs(ch[k]['squares_of_the_two_summed_median'] - 1) < 0.01 for k in range(3)),
        'the plain channel takes no filtered part': ch[2]['blocks_without_a_filtered_part'] > 0.99,
        'the mixed channels take one in every block': all(ch[k]['blocks_without_a_filtered_part'] < 0.05 for k in range(2)),
        'the mix gives the reference': all(v > 60 for k in range(3) for v in ch[k]['left_with_the_mix_of_each_block_db']),
        'one mix for all blocks says which channel is filtered, and how':
            ch[0]['with_one_mix_for_all_blocks']['filtered_weight'] > 0.5
            and ch[1]['with_one_mix_for_all_blocks']['filtered_weight'] < -0.5
            and abs(ch[2]['with_one_mix_for_all_blocks']['filtered_weight']) < 0.001
            and abs(ch[2]['with_one_mix_for_all_blocks']['direct_weight'] - 1) < 0.001,
    }
    for name, ok in checks.items():
        print(f'{"ok  " if ok else "FAIL"} {name}')
    return 0 if all(checks.values()) else 1


def main():
    parser = argparse.ArgumentParser(description='What another decoder does to the coupled region of a stream.')
    parser.add_argument('--self-check', action='store_true')
    commands = parser.add_subparsers(dest='command')
    make = commands.add_parser('signals', help='write the test signals')
    make.add_argument('directory', type=Path)
    for name, text in (('regions', 'the own region and the coupled region of every channel'),
                       ('mix', 'the filter along the blocks and the mix of every block'),
                       ('measure', 'a decorrelated decode of ours against a reference, by channel and sub-band')):
        sub = commands.add_parser(name, help=text)
        sub.add_argument('reference', type=Path, help='the other decode: 32-bit float, headerless or WAVE')
        sub.add_argument('ours', type=Path, help='this decoder\'s decode: 32-bit float WAVE')
        sub.add_argument('--channels', type=int, required=True)
        sub.add_argument('--first', type=int, required=True, help='the first coupled coefficient: 37 + 12 * cplbegf')
        sub.add_argument('--sub-bands', type=int, required=True, help='coupling sub-bands: cplendf + 3 - cplbegf')
        sub.add_argument('--lag', type=int, default=256, help='samples into ours at which the reference starts')
        sub.add_argument('--out', type=Path)
        if name == 'mix':
            sub.add_argument('--learn', type=int, help='the channel the filter is fitted on')
            sub.add_argument('--taps', type=int, default=40)
            sub.add_argument('--rounds', type=int, default=3)
        if name == 'measure':
            sub.add_argument('--pairs', default='0:1',
                             help='the pairs of channels that take the filter with opposite signs, as a:b,c:d '
                                  '(coded order: 0:1 for 2/0, 0:2,3:4 for 3/2)')
            sub.add_argument('--transient-db', type=float, default=10.0,
                             help='how much louder than the block before a block is that holds a transient')
            sub.add_argument('--plain', type=Path,
                             help='a decode of ours without the decorrelation, same format and lag as ours')
            sub.add_argument('--skip', type=int, default=SKIP, help='blocks left out at the start')
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    if args.command is None:
        parser.print_usage(sys.stderr)
        return 2
    if args.command == 'signals':
        made = signals(args.directory)
        for name, what in made.items():
            print(f'{name}.wav: {what}')
        return 0
    if args.first < 9 or args.sub_bands < 1 or args.first + SUB_BAND * args.sub_bands > 253:
        print('the coupled region is not inside the 253 coefficients of a channel', file=sys.stderr)
        return 2
    if args.command == 'measure':
        pairs = parse_pairs(args.pairs)
        if not pairs or any(len(pair) != 2 for pair in pairs) or args.skip < 0:
            print('--pairs is a:b,c:d and --skip is not negative', file=sys.stderr)
            return 2
        reference, ours = read_decode(args.reference, args.channels), read_decode(args.ours, args.channels)
        plain = None if args.plain is None else read_decode(args.plain, args.channels)
        report = measure(reference, ours, plain, args.first, args.sub_bands, args.lag, pairs,
                         args.transient_db, args.skip)
        if report is None:
            print('the decodes cannot be compared as asked: too short, or a pair names a channel they lack',
                  file=sys.stderr)
            return 1
        text = json.dumps(report, indent=1) + '\n'
        if args.out:
            args.out.write_text(text, encoding='utf-8')
        for row in report['channels']:
            print(f"channel {row['channel']}: power {row['power_difference_db']} dB, pre-echo {row['pre_echo_db']} dB, "
                  f"boundaries {row['boundary_discontinuity_db']} dB")
        for row in report['pairs']:
            print(f"pair {row['pair']}: coherence difference {row['coherence_difference']}")
        return 0
    reference, ours = read_decode(args.reference, args.channels), read_decode(args.ours, args.channels)
    theirs, mine = as_coefficients(reference, ours, args.lag)
    if args.command == 'regions':
        report = {'blocks': len(theirs[0]) - SKIP, 'channels': regions(theirs, mine, args.first, args.sub_bands)}
    else:
        report = study(theirs, mine, args.first, args.sub_bands, args.taps, args.rounds, args.learn)
    text = json.dumps(report, indent=1) + '\n'
    if args.out:
        args.out.write_text(text, encoding='utf-8')
    for row in report['channels']:
        line = f"channel {row['channel']}: own region {row['own_region_db']} dB, coupled region {row['coupled_region_db']} dB"
        if row.get('direct_weight_mean_by_sub_band'):
            one = row['with_one_mix_for_all_blocks']
            least = min(v for v in row['left_with_the_mix_of_each_block_db'] if v is not None)
            line += (f"; one mix for all blocks: direct {one['direct_weight']}, filtered {one['filtered_weight']}, "
                     f"left {one['left_db']} dB; a mix for each block: direct {row['direct_weight_mean_by_sub_band'][0]} "
                     f"to {row['direct_weight_mean_by_sub_band'][-1]}, filtered "
                     f"{row['filtered_weight_mean_by_sub_band'][0]} to {row['filtered_weight_mean_by_sub_band'][-1]}, "
                     f"left {least} dB at the least"
                     + ('' if least >= 40 else ' (too much left for the pairs of single blocks to mean anything)'))
        print(line)
    if args.command == 'mix' and report['the_filter'].get('found'):
        shape = report['the_filter']['as_an_all_pass_of_three_poles_after_one_block']
        print(f"the filter, from channel {report['the_filter']['learned_from_channel']}: denominator {shape['denominator']}, "
              f"poles {shape['real_poles']} and {shape['pair']}, misses the taps by {shape['largest_tap_it_misses_by']}")
    return 0


if __name__ == '__main__':
    sys.exit(main())
