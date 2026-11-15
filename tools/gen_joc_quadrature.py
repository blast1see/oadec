#!/usr/bin/env python3
"""Measure the operator Dolby's decoder applies to the lowest QMF subband of a
phase-shifted downmix channel, and print the table of
`crates/oadec-joc/src/quadrature.rs`.

ETSI TS 103 420 table 47 names downmix configurations 3 and 4 "5.X with
90 degree phase shift" and never defines the shift. Rotating every subband
by -j is right above the lowest subband and wrong in it, because subband 0
straddles direct current. What Dolby's decoder does there is a filter across
time slots, and this script solves for its taps.

A decode is linear in those taps. `oadec decode --joc-low-fir FILE` puts any
operator in the place of the measured one, so the script decodes a stream with
no operator at all and then with one tap at a time set to 1 and to j: what each
tap adds to the objects. The taps are the combination of those that comes
closest to the objects Dolby's decoder writes for the same stream, below 450 Hz,
by least squares. Dolby's decoder is run as it stands and only what it writes
is read:

    gst-launch-1.0 filesrc location=in.ec3
      ! dlbac3parse enable-metadata=true
      ! dlbac3dec out-ch-config=21 drc-suppress=true drc-mode=custom-0
                  drc-cut=0 drc-boost=0 drop-delay=true
      ! "audio/x-raw(meta:DlbObjectAudioMeta),format=F32LE"
      ! identity ! filesink location=dolby.f32

(`GST_PLUGIN_PATH` must point at the `gst-plugins` directory of the Dolby
Reference Player and the player's directory must be on `PATH`;
`docs/dolby-tools.md`.)

    python tools/gen_joc_quadrature.py --work DIR \\
        --pair in.ec3 dolby.f32 --pair ... [--out taps.json]

Every stream is solved on its first half and held to its second half and to
the second half of every other stream; the table is what all of them give
together, each weighed by its own power. Give streams of several titles and
encoders. A stream whose shifted channels carry next to nothing below 450 Hz
does not say what the taps are, and the report says how much each carries.

`--reach` is how many slots the operator may reach either side of its middle
(10: 21 taps) and `--free` lets the two sides differ; the taps are tied in
pairs otherwise. The report also solves the same data with the outermost pairs
held at zero, with real taps, and with the form the decoder had before the
measurement, which is how one sees that 21 taps are needed, that they are
complex, and that the turn with a real correction is not among them.

    python tools/gen_joc_quadrature.py --work DIR --self-check in.ec3

asks the method a question whose answer is known: the reference is this
decoder with an operator drawn at random, and the taps that come back have to
be the ones that were drawn.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import joc_bands  # noqa: E402
from oadec_bin import find as find_oadec  # noqa: E402

RATE = 48000
SILENT = 1e-6  # an object under -120 dBFS in the reference carries nothing to solve on


def write_taps(path: Path, taps: np.ndarray) -> None:
    """An operator as `--joc-low-fir` reads one: a tap a line, real part then
    imaginary part, written so that reading them gives the same numbers back."""
    path.write_text(''.join(f'{float(t.real)!r} {float(t.imag)!r}\n' for t in taps), encoding='ascii')


def decode(oadec: str, clip: str, taps: np.ndarray, work: Path) -> np.ndarray:
    """The objects of `clip` with `taps` on the lowest subband, our dither left out."""
    file = work / 'taps.txt'
    write_taps(file, taps)
    base = work / 'decode'
    done = subprocess.run(
        [oadec, 'decode', '--format', 'damf', '--no-bed-conform', '--no-dither', '--joc-low-fir', str(file),
         '-o', str(base), clip], capture_output=True, text=True, timeout=1800)
    audio = Path(str(base) + '.atmos.audio')
    if done.returncode not in (0, 7) or not audio.exists():
        sys.exit(f'{clip}: the decode ended with {done.returncode}: {done.stderr.strip()[-300:]}')
    rows = joc_bands.read(audio, None)
    for ext in ('.atmos', '.atmos.metadata', '.atmos.audio'):
        Path(str(base) + ext).unlink(missing_ok=True)
    return rows


def unknowns(reach: int, free: bool, length: int) -> list[np.ndarray]:
    """One operator for every unknown, real parts first: a tap (or the pair of
    taps the same distance either side of the middle) set to 1, then to j."""
    middle = length // 2
    places = range(-reach, reach + 1) if free else range(reach + 1)
    out = []
    for value in (1.0, 1j):
        for k in places:
            taps = np.zeros(length, dtype=complex)
            taps[middle + k] = value
            if not free:
                taps[middle - k] = value
            out.append(taps)
    return out


def below(rows: np.ndarray, segment: tuple[int, int], top: float) -> np.ndarray:
    """The spectrum of a stretch of every channel, up to `top` Hz."""
    start, end = segment
    n = end - start
    edge = min(RATE // 4, n // 8)
    window = np.ones(n)
    ramp = 0.5 - 0.5 * np.cos(np.pi * np.arange(edge) / edge)
    window[:edge], window[-edge:] = ramp, ramp[::-1]
    return np.fft.rfft(rows[start:end] * window[:, None], axis=0)[:int(top * n / RATE)]


def measure(oadec: str, clip: str, reference: np.ndarray | str, basis: list[np.ndarray], top: float,
            skip: float, work: Path) -> dict:
    """What a stream says of the taps: for each half of it, the products of
    every unknown's share of the objects with every other's and with what the
    reference has that the decode without an operator has not. The reference
    is rows of objects, or a file of them as headerless 32-bit float."""
    length = len(basis[0])
    nothing = decode(oadec, clip, np.zeros(length, dtype=complex), work)
    if isinstance(reference, str):
        values = np.fromfile(reference, dtype='<f4')
        if len(values) % nothing.shape[1]:
            sys.exit(f'{reference}: not rows of {nothing.shape[1]} values')
        reference = values.reshape(-1, nothing.shape[1]).astype(np.float64)
    if nothing.shape[1] != reference.shape[1]:
        sys.exit(f'{clip}: {nothing.shape[1]} channels against {reference.shape[1]} of the reference')
    if abs(len(nothing) - len(reference)) > 4096:
        sys.exit(f'{clip}: {len(nothing)} rows against {len(reference)} of the reference')
    # the operator is what the two differ by at the bottom, so they are laid
    # side by side on what is above it: twice differenced, 50 Hz is 88 dB down
    lag, _ = joc_bands.find_lag(np.diff(nothing, n=2, axis=0), np.diff(reference, n=2, axis=0))
    if lag != 0:
        sys.exit(f'{clip}: the reference runs {lag} samples behind the decode; the two have to be aligned')
    rows = min(len(nothing), len(reference))
    edge = int(skip * RATE)
    half = (rows - 2 * edge) // 2
    if half < RATE // 2:
        sys.exit(f'{clip}: {rows / RATE:.1f} s is too short to halve')
    segments = [(edge, edge + half), (edge + half, edge + 2 * half)]
    # the first channel is the LFE, which goes through nothing of JOC
    heard = [c for c in range(1, reference.shape[1]) if float(np.sqrt((reference[:rows, c] ** 2).mean())) >= SILENT]
    if not heard:
        sys.exit(f'{clip}: the reference carries no object')
    base = [below(nothing[:, heard], s, top) for s in segments]
    columns: list[list[np.ndarray]] = [[], []]
    for taps in basis:
        with_it = decode(oadec, clip, taps, work)
        for i, s in enumerate(segments):
            columns[i].append((below(with_it[:, heard], s, top) - base[i]).reshape(-1))
    grams, powers = [], []
    for i, s in enumerate(segments):
        wanted = below(reference[:, heard], s, top)
        m = np.stack(columns[i] + [(wanted - base[i]).reshape(-1)], axis=1)
        grams.append(np.real(np.conj(m).T @ m))
        powers.append(float((np.abs(wanted) ** 2).sum()))
    return {'grams': grams, 'powers': powers, 'objects': len(heard), 'seconds': round(2 * half / RATE, 1)}


def left(gram: np.ndarray, u: np.ndarray) -> float:
    """The power of what `u` leaves of the reference."""
    return float(gram[-1, -1] - 2 * u @ gram[:-1, -1] + u @ gram[:-1, :-1] @ u)


def solve(gram: np.ndarray, keep: list[int] | None = None) -> np.ndarray:
    """Least squares on every unknown, or on those in `keep` with the rest at zero."""
    a, b = gram[:-1, :-1], gram[:-1, -1]
    if keep is None:
        return np.linalg.lstsq(a, b, rcond=1e-13)[0]
    u = np.zeros(len(b))
    u[keep] = np.linalg.lstsq(a[np.ix_(keep, keep)], b[keep], rcond=1e-13)[0]
    return u


def db(ratio: float) -> float | None:
    return round(float(10 * np.log10(ratio)), 1) if ratio > 0 and np.isfinite(ratio) else None


def taps_of(u: np.ndarray, free: bool) -> np.ndarray:
    """The unknowns as an operator, a tap a slot."""
    count = len(u) // 2
    values = u[:count] + 1j * u[count:]
    return values if free else np.concatenate([values[:0:-1], values])


def literal(v: float) -> str:
    """A number as the table of the source writes it: ten decimals, in threes."""
    whole, decimals = f'{abs(v):.10f}'.split('.')
    groups = '_'.join(decimals[i:i + 3] for i in range(0, len(decimals), 3))
    return f'{"-" if v < 0 else ""}{whole}.{groups}'


def report(measured: dict[str, dict], reach: int, free: bool) -> dict:
    """Every stream alone, every stream against every other, all together, and
    the same data with fewer unknowns."""
    names = list(measured)
    alone = {n: solve(measured[n]['grams'][0]) for n in names}
    total = np.sum([m['grams'][i] / m['powers'][i] for m in measured.values() for i in (0, 1)], axis=0)
    joint = solve(total)

    def whole(n, u):
        m = measured[n]
        return db((m['powers'][0] + m['powers'][1]) / (left(m['grams'][0], u) + left(m['grams'][1], u)))

    streams = []
    print('stream  objects  seconds | through the operator, each half | own taps on the other half | '
          'the joint taps | own taps off the joint by', file=sys.stderr)
    for i, n in enumerate(names):
        m = measured[n]
        through = [db(float(joint @ g[:-1, :-1] @ joint) / p) for g, p in zip(m['grams'], m['powers'])]
        held = db(m['powers'][1] / left(m['grams'][1], alone[n]))
        # how far the taps each half gives alone are from the joint ones
        off = [float(np.abs(solve(g) - joint).max()) for g in m['grams']]
        streams.append({
            'stream': i + 1, 'objects': m['objects'], 'seconds': m['seconds'],
            'through_the_operator_db': through,
            'own_taps_of_the_first_half_on_the_second_db': held,
            'the_joint_taps_db': whole(n, joint),
            'own_taps_of_each_half_off_the_joint_by': off,
            'the_taps_of_this_stream_on_the_second_half_of_each_db':
                [db(measured[o]['powers'][1] / left(measured[o]['grams'][1], alone[n])) for o in names],
        })
        print(f'{i + 1:6d} {m["objects"]:8d} {m["seconds"]:8.1f} | {through[0]!s:>8} {through[1]!s:>8} dB'
              f'            | {held!s:>8} dB                | {whole(n, joint)!s:>8} dB    | '
              f'{off[0]:.1e} {off[1]:.1e}', file=sys.stderr)
    out = {'reach': reach, 'taps_tied_in_pairs': not free, 'streams': streams}
    joint_taps = taps_of(joint, free)
    out['taps'] = {'real': [float(t.real) for t in joint_taps], 'imaginary': [float(t.imag) for t in joint_taps]}
    out['sum_of_the_taps'] = [float(joint_taps.sum().real), float(joint_taps.sum().imag)]
    if not free:
        count = reach + 1
        forms: dict[str, list[int] | None] = {f'{2 * reach + 1} taps, the same either side': None}
        for fewer in (1, 2):
            if reach - fewer >= 1:
                gone = [reach - k for k in range(fewer)]
                forms[f'{2 * (reach - fewer) + 1} taps (the outermost held at zero)'] = [
                    i for i in range(2 * count) if i % count not in gone]
        forms['real taps'] = list(range(count))
        out['with_fewer_unknowns'] = {}
        print(file=sys.stderr)
        for name, keep in forms.items():
            u = solve(total, keep)
            values = sorted(v for v in (whole(n, u) for n in names) if v is not None)
            out['with_fewer_unknowns'][name] = {'lowest_db': values[0], 'median_db': values[len(values) // 2]}
            print(f'{name:40} lowest {values[0]:6} dB, median {values[len(values) // 2]:6} dB over the streams',
                  file=sys.stderr)
        # the form the decoder had before the measurement: a turn by -j at the
        # middle and a real correction times 1 + j. The real and the imaginary
        # part of every tap are then one unknown, and the turn moves to the
        # other side of the equation.
        turn = np.zeros(2 * count)
        turn[count] = -1.0
        both = np.vstack([np.eye(count), np.eye(count)])
        a, b = total[:-1, :-1], total[:-1, -1]
        u = turn + both @ np.linalg.lstsq(both.T @ a @ both, both.T @ (b - a @ turn), rcond=1e-13)[0]
        values = sorted(v for v in (whole(n, u) for n in names) if v is not None)
        name = 'a turn by -j and a real correction times 1 + j'
        out['with_fewer_unknowns'][name] = {'lowest_db': values[0], 'median_db': values[len(values) // 2]}
        print(f'{name:40} lowest {values[0]:6} dB, median {values[len(values) // 2]:6} dB over the streams',
              file=sys.stderr)
    return out


def main() -> int:
    ap = argparse.ArgumentParser(description='Solve for the operator Dolby\'s decoder applies to the lowest '
                                             'subband of a phase-shifted channel.')
    ap.add_argument('--pair', nargs=2, action='append', metavar=('EC3', 'DOLBY'), default=[],
                    help='a stream and the objects Dolby\'s decoder writes for it (headerless 32-bit float)')
    ap.add_argument('--self-check', metavar='EC3',
                    help='recover an operator drawn at random from this decoder\'s own objects')
    ap.add_argument('--work', required=True, type=Path, help='a directory for the decodes')
    ap.add_argument('--binary', default=find_oadec(required=False))
    ap.add_argument('--reach', type=int, default=10, help='slots either side of the middle (default 10)')
    ap.add_argument('--free', action='store_true', help='do not tie the taps in pairs')
    ap.add_argument('--top', type=float, default=450.0, help='Hz to solve up to (default 450)')
    ap.add_argument('--skip', type=float, default=2.0, help='seconds left out at each end (default 2)')
    ap.add_argument('--out', type=Path, help='write the report here as JSON')
    args = ap.parse_args()
    if bool(args.pair) == bool(args.self_check):
        ap.error('give --pair, once or more, or --self-check')
    if args.reach < 1:
        ap.error('--reach is at least 1')
    args.work.mkdir(parents=True, exist_ok=True)
    # the decoder holds every channel back by half the operator and wants at least its own 21 taps
    length = 2 * max(args.reach, 10) + 1
    basis = unknowns(args.reach, args.free, length)

    if args.self_check:
        # small taps: an operator that made the objects louder could take them
        # past full scale, and a file that clips is not linear in anything
        drawn = 0.1 * (np.random.default_rng(20261115).normal(size=2 * args.reach + 1)
                       + 1j * np.random.default_rng(7).normal(size=2 * args.reach + 1))
        if not args.free:
            drawn = (drawn + drawn[::-1]) / 2
        padded = np.zeros(length, dtype=complex)
        padded[length // 2 - args.reach:length // 2 + args.reach + 1] = drawn
        reference = decode(args.binary, args.self_check, padded, args.work)
        m = measure(args.binary, args.self_check, reference, basis, args.top, args.skip, args.work)
        u = solve(m['grams'][0] + m['grams'][1])
        found = taps_of(u, args.free)
        off = float(np.abs(found - drawn).max())
        leaves = db((m['powers'][0] + m['powers'][1]) / max(left(m['grams'][0] + m['grams'][1], u), 1e-300))
        result = {'self_check': True, 'taps_drawn': len(drawn), 'tied_in_pairs': not args.free,
                  'objects': m['objects'], 'seconds': m['seconds'],
                  'largest_error_of_a_tap': off, 'what_the_taps_found_leave_db': leaves}
        print(json.dumps(result))
        if args.out:
            args.out.write_text(json.dumps(result, indent=1) + '\n', encoding='utf-8', newline='\n')
        # the reference is a 24-bit file, so the taps come back to about its rounding
        return 0 if off < 1e-4 else 1

    measured = {}
    for clip, dolby in args.pair:
        measured[clip] = measure(args.binary, clip, dolby, basis, args.top, args.skip, args.work)
        print(f'measured {len(measured)} of {len(args.pair)}', file=sys.stderr, flush=True)
    out = report(measured, args.reach, args.free)
    taps = np.array(out['taps']['real']) + 1j * np.array(out['taps']['imaginary'])
    middle = len(taps) // 2
    if args.free:
        print(f'// {len(taps)} taps, newest slot first')
        for t in taps:
            print(f'    tap({literal(t.real)}, {literal(t.imag)}),')
    else:
        print('pub const OPERATOR: [Complex; LOW_DELAY + 1] = [')
        for t in taps[middle:]:
            print(f'    tap({literal(t.real)}, {literal(t.imag)}),')
        print('];')
    print(f'// the sum of the taps: {out["sum_of_the_taps"][0]:.7f} {out["sum_of_the_taps"][1]:+.7f}j', file=sys.stderr)
    if args.out:
        args.out.write_text(json.dumps(out, indent=1) + '\n', encoding='utf-8', newline='\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
