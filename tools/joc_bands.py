#!/usr/bin/env python3
"""Where two decodes of the objects of one stream differ, band by band.

    python tools/joc_bands.py ours.atmos.audio reference.f32 --out bands.json

`tools/objcmp.py` gives one figure an object: its distance to the reference
over the whole band. This says where that distance lies and what kind it is.
For every object:

- the signal-to-noise ratio in each of 64 bands of 375 Hz, which are the
  subbands of the JOC filter bank, though nothing of that bank is used here:
  the bands are cut out of plain Fourier transforms of both decodes;
- the same after the best linear fit, which is the reference less what a
  filter applied to our decode could explain of it. Where the first figure is
  low and the second high, the two decoders differ by a filter: a gain, a
  delay, a phase. Where both are low they differ by something no filter makes
  good: another noise, another matrix at another time. The fit is a gain for
  every 1,46 Hz, over frames of two thirds of a second, so that a filter whose
  phase turns quickly near direct current is still a filter to it;
- the gain and the phase of that fit in each band;
- all of it again below 750 Hz in steps of 11,7 Hz, which is where the lowest
  subband takes the 90-degree phase shift of the downmix back out;
- and, band by band, how far the correlation of every pair of objects is from
  what it is in the reference.

The first input is what `oadec decode --format damf` writes as
`<base>.atmos.audio`, or headerless 32-bit float; the second is headerless
32-bit float, little-endian, interleaved, with as many channels, which is what
a GStreamer `filesink` writes after Dolby's object decoder
(`docs/dolby-tools.md`). The reference may lag ours by a whole number of
samples, which is found on the loudest channel and taken out before anything
is compared: a difference that is only a delay is not one here.

Nothing is judged. Exit code: 0, 1 when the two cannot be compared, 2 on a
usage error.
"""
import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from caf import read_caf  # noqa: E402

RATE = 48000
NFFT = 4096  # the frames the bands are cut from: 11,7 Hz a bin
FIT = 32768  # the frames of the linear fit: 1,46 Hz a bin
BAND_HZ = 375.0
BANDS = 64
BINS = round(BAND_HZ / (RATE / NFFT))  # bins of a band
FIT_BINS = round(BAND_HZ / (RATE / FIT))  # bins of the fit in a band
STEP = FIT // NFFT  # bins of the fit in a step of the grid below 750 Hz
SILENT = 1e-6  # a channel whose reference is under this is not measured


def read(path, channels):
    """A decode as rows of `channels` values between -1 and 1."""
    with open(path, 'rb') as f:
        head = f.read(4)
    if head == b'caff':
        rows, _ = read_caf(str(path))
        return rows
    if channels is None:
        sys.exit(f'{path}: headerless, so --channels has to say how many it has')
    values = np.fromfile(path, dtype='<f4')
    return values[:len(values) // channels * channels].reshape(-1, channels).astype(np.float64)


def find_lag(ours, reference, most=3000):
    """How many samples the reference runs behind ours, on the loudest channel,
    in the four seconds where that channel is loudest: the lag at which the
    two are most alike, between `most` either way."""
    loud = int(np.argmax((reference ** 2).mean(axis=0)))
    n = min(4 * RATE, (min(len(ours), len(reference)) - 2 * most) // 2)
    if n < RATE:
        return 0, loud
    energy = np.cumsum(np.concatenate([[0.0], reference[:, loud] ** 2]))
    starts = np.arange(most, min(len(ours), len(reference)) - n - most, RATE // 2)
    if len(starts) == 0:
        return 0, loud
    start = int(starts[np.argmax(energy[starts + n] - energy[starts])])
    a = ours[start:start + n, loud]
    b = reference[start - most:start + n + most, loud]
    # the sum of a[t] b[t + lag] for every lag at once
    size = 1 << int(np.ceil(np.log2(len(b) + n)))
    cross = np.fft.irfft(np.fft.rfft(b, size) * np.conj(np.fft.rfft(a, size)), size)[:2 * most + 1]
    squares = np.cumsum(np.concatenate([[0.0], b ** 2]))
    under = np.sqrt(float((a ** 2).sum()) * (squares[n:n + 2 * most + 1] - squares[:2 * most + 1]))
    alike = np.where(under > 0, cross / np.where(under > 0, under, 1), -2.0)
    return int(np.argmax(alike)) - most, loud


def spectra(ours, reference, nfft, pairs=False):
    """The mean spectra of the frames of every channel: ours, the reference,
    the reference against ours, and their difference, as arrays of a bin by a
    channel; how many frames; and, where `pairs` is set, the spectra of every
    pair of channels summed over each band, of ours and of the reference."""
    window = np.hanning(nfft)[:, None]
    hop = nfft // 2
    frames = (len(ours) - nfft) // hop + 1
    shape = (nfft // 2 + 1, ours.shape[1])
    xx, yy, dd = np.zeros(shape), np.zeros(shape), np.zeros(shape)
    yx = np.zeros(shape, dtype=np.complex128)
    mine = theirs = None
    if pairs:
        mine = np.zeros((BANDS, shape[1], shape[1]), dtype=np.complex128)
        theirs = np.zeros_like(mine)
    for k in range(frames):
        x = np.fft.rfft(ours[k * hop:k * hop + nfft] * window, axis=0)
        y = np.fft.rfft(reference[k * hop:k * hop + nfft] * window, axis=0)
        xx += np.abs(x) ** 2
        yy += np.abs(y) ** 2
        dd += np.abs(x - y) ** 2
        yx += y * np.conj(x)
        if pairs:
            bins = nfft * BANDS * BINS // NFFT
            part = x[:bins].reshape(BANDS, -1, shape[1])
            mine += np.einsum('bkc,bkd->bcd', part, np.conj(part))
            part = y[:bins].reshape(BANDS, -1, shape[1])
            theirs += np.einsum('bkc,bkd->bcd', part, np.conj(part))
    return xx, yy, dd, yx, frames, mine, theirs


def db(ratio):
    return None if not np.isfinite(ratio) or ratio <= 0 else round(float(10 * np.log10(ratio)), 2)


def measure(xx, yy, dd, yx, bins):
    """Over a set of bins of one channel: the ratio of the reference to the
    difference, the same after the best gain a bin, and that gain taken over
    the set."""
    signal, noise = float(yy[bins].sum()), float(dd[bins].sum())
    # what of the reference a gain on ours explains, bin by bin
    explained = np.where(xx[bins] > 0, np.abs(yx[bins]) ** 2 / np.where(xx[bins] > 0, xx[bins], 1), 0)
    left = float((yy[bins] - explained).sum())
    power = float(xx[bins].sum())
    gain = complex(yx[bins].sum() / power) if power > 0 else 0j
    return {
        'snr_db': db(signal / noise) if noise > 0 else None,
        'snr_after_a_filter_db': db(signal / left) if left > 0 else None,
        'gain_db': db(abs(gain) ** 2) if gain else None,
        'phase_degrees': round(float(np.degrees(np.angle(gain))), 2) if gain else None,
    }


def correlation(pairs):
    """The correlation of every pair of channels in every band, from the
    spectra of the pairs."""
    out = np.zeros(pairs.shape)
    for b in range(BANDS):
        power = np.real(np.diag(pairs[b]))
        scale = np.sqrt(np.outer(power, power))
        out[b] = np.where(scale > 0, np.real(pairs[b]) / np.where(scale > 0, scale, 1), 0)
    return out


def compare(ours, reference, skip_seconds=2.0):
    """Everything this tool says of two decodes, as one record."""
    if ours.shape[1] != reference.shape[1]:
        sys.exit(f'{ours.shape[1]} channels against {reference.shape[1]}')
    lag, loud = find_lag(ours, reference)
    start = int(skip_seconds * RATE) + max(0, -lag)
    end = min(len(ours), len(reference) - lag) - RATE // 2
    if end - start < 8 * FIT:
        sys.exit('too short to compare: the fit wants a few seconds')
    a, b = ours[start:end], reference[start + lag:end + lag]
    measured = [c for c in range(a.shape[1]) if float(np.sqrt((b[:, c] ** 2).mean())) >= SILENT]
    a, b = a[:, measured], b[:, measured]
    # the ratio of a band from frames of 11,7 Hz a bin, the fit from frames of 1,46 Hz
    _, yy, dd, _, frames, mine, theirs = spectra(a, b, NFFT, pairs=len(measured) > 1)
    fit = spectra(a, b, FIT)
    objects = []
    for i, c in enumerate(measured):
        fitted = lambda bins, i=i: measure(fit[0][:, i], fit[1][:, i], fit[2][:, i], fit[3][:, i], bins)  # noqa: E731
        bands = []
        for k in range(BANDS):
            noise = float(dd[k * BINS:(k + 1) * BINS, i].sum())
            row = fitted(slice(k * FIT_BINS, (k + 1) * FIT_BINS))
            row['snr_db'] = db(float(yy[k * BINS:(k + 1) * BINS, i].sum()) / noise) if noise > 0 else None
            bands.append(row)
        noise = float(dd[:BANDS * BINS, i].sum())
        signal = float(yy[:BANDS * BINS, i].sum())
        objects.append({
            'channel': c,
            'level_dbfs': db(float((b[:, i] ** 2).mean())),
            'snr_db': db(signal / noise) if noise > 0 else None,
            'snr_after_a_filter_db': fitted(slice(0, BANDS * FIT_BINS))['snr_after_a_filter_db'],
            'of_the_difference_in_the_lowest_band': round(float(dd[:BINS, i].sum() / noise), 4) if noise > 0 else None,
            'of_the_signal_in_the_lowest_band': round(float(yy[:BINS, i].sum() / signal), 4),
            'bands': bands,
            'below_750_hz': [dict(hz=round((k + 0.5) * STEP * RATE / FIT, 1), **fitted(slice(k * STEP, (k + 1) * STEP)))
                             for k in range(2 * BINS)],
        })
    record = {
        'lag_samples': lag, 'aligned_on_channel': loud, 'compared_seconds': round((end - start) / RATE, 2),
        'channels': int(ours.shape[1]), 'channels_measured': measured,
        'frames': frames, 'frames_of_the_fit': fit[4],
        'band_hz': BAND_HZ, 'bin_hz': round(RATE / NFFT, 3), 'bin_of_the_fit_hz': round(RATE / FIT, 3),
        'objects': objects,
    }
    if len(measured) > 1:
        ours_pairs, reference_pairs = correlation(mine), correlation(theirs)
        upper = np.triu_indices(len(measured), 1)
        record['correlation_of_pairs'] = {
            'largest_difference_by_band': [round(float(np.abs(ours_pairs[k][upper] - reference_pairs[k][upper]).max()), 4)
                                           for k in range(BANDS)],
            'largest_in_the_reference_by_band': [round(float(np.abs(reference_pairs[k][upper]).max()), 4)
                                                 for k in range(BANDS)],
        }
    return record


def main():
    parser = argparse.ArgumentParser(description='Where two decodes of the objects of one stream differ, '
                                                 'band by band.')
    parser.add_argument('ours', type=Path, help='`<base>.atmos.audio`, or headerless 32-bit float')
    parser.add_argument('reference', type=Path, help='headerless 32-bit float, interleaved')
    parser.add_argument('--channels', type=int, help='channels of a headerless first input')
    parser.add_argument('--skip', type=float, default=2.0, help='seconds left out at the start (default 2)')
    parser.add_argument('--out', type=Path, help='write the record here as JSON')
    args = parser.parse_args()
    for path in (args.ours, args.reference):
        if not path.is_file():
            sys.exit(f'{path}: no such file')
    ours = read(args.ours, args.channels)
    reference = read(args.reference, ours.shape[1])
    record = compare(ours, reference, args.skip)
    print(f"{'channel':>7} {'dBFS':>7} {'SNR':>7} {'fitted':>7} {'band 0':>7} {'fitted':>7} "
          f"{'gain':>7} {'phase':>7}  of the difference in band 0")
    show = lambda v: '      -' if v is None else f'{v:7.2f}'  # noqa: E731
    for o in record['objects']:
        low = o['bands'][0]
        print(f"{o['channel']:7d} {show(o['level_dbfs'])} {show(o['snr_db'])} {show(o['snr_after_a_filter_db'])} "
              f"{show(low['snr_db'])} {show(low['snr_after_a_filter_db'])} {show(low['gain_db'])} "
              f"{show(low['phase_degrees'])}  {o['of_the_difference_in_the_lowest_band']}")
    if args.out:
        args.out.write_text(json.dumps({'schema': 1, **record}, indent=1) + '\n', encoding='utf-8', newline='\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
