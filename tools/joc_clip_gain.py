#!/usr/bin/env python3
"""Read off a decode of the objects of a stream when and how the clip gain of
the JOC payloads was put back into it.

A JOC payload carries the gain the encoder took off the downmix
(`joc_clipgain`, ETSI TS 103 420 clause 6.3.3.2), and the standard gives it a
value and no use. `oadec decode --no-clip-gain` writes the objects without it,
so a decode with it, divided by that one, is the gain as it was applied: sample
by sample for the LFE, which goes through nothing of JOC, and in the mean for
the objects. This tool takes the objects another decoder wrote for the stream,
solves for that gain in steps of a few samples around every change of the gain
that stands alone, the LFE and the objects apart, and says where the gain
holds whether it is the value of the payload.

    python tools/joc_clip_gain.py in.ec3 dolby.f32 [--step 16] [--out gain.json]

`dolby.f32` is what Dolby's decoder writes for the stream, headerless 32-bit
float, as `tools/gen_joc_quadrature.py` says how to get; that decoder is run
as it stands and only what it writes is read. Positions are samples of the
frame the new gain comes with, as the core decodes it: the first sample the
frame decodes to is 0, and a row of the object output is 256 samples behind
it. A value is how far the gain has gone from the value before (0) to the
new one (1), the median over the changes.

    python tools/joc_clip_gain.py in.ec3 --self-check

asks the tool a question whose answer is known: the reference is this
decoder's own decode with the gain, which puts it back in a straight line
over the first 256 samples of a frame, and that line has to come back.
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

CORE_DELAY = 256  # the first sample a frame decodes to is this many rows before the row of its number


def decode(oadec: str, clip: str, work: Path, *flags: str) -> np.ndarray:
    base = work / 'gain'
    done = subprocess.run([oadec, 'decode', '--format', 'damf', '--no-bed-conform', '--no-dither', *flags,
                           '-o', str(base), clip], capture_output=True, text=True, timeout=1800)
    audio = Path(str(base) + '.atmos.audio')
    if done.returncode not in (0, 7) or not audio.exists():
        sys.exit(f'{clip}: the decode ended with {done.returncode}: {done.stderr.strip()[-300:]}')
    rows = joc_bands.read(audio, None)
    for ext in ('.atmos', '.atmos.metadata', '.atmos.audio'):
        Path(str(base) + ext).unlink(missing_ok=True)
    return rows


def frame_length(oadec: str, clip: str) -> int:
    """Samples a frame decodes to: 256 a block."""
    done = subprocess.run([oadec, 'info', '--json', clip], capture_output=True, text=True, timeout=600)
    try:
        return 256 * int(json.loads(done.stdout)['blocks_per_frame'])
    except (ValueError, KeyError, TypeError):
        sys.exit(f'{clip}: `info --json` does not say how many blocks a frame has')


def over(top: np.ndarray, bottom: np.ndarray, rows: slice, channels: list[int]) -> float | None:
    """The gain that takes `bottom` to `top` over some rows of some channels,
    by least squares, or None where `bottom` is as good as silent."""
    b = bottom[rows][:, channels]
    power = float((b ** 2).sum())
    if power < 1e-9:
        return None
    return float((top[rows][:, channels] * b).sum()) / power


def main() -> int:
    ap = argparse.ArgumentParser(description='When and how the clip gain was put back into a decode of the objects.')
    ap.add_argument('clip', help='an E-AC-3 stream with JOC')
    ap.add_argument('reference', nargs='?', help='the objects another decoder wrote for it, headerless 32-bit float')
    ap.add_argument('--self-check', action='store_true', help='read the gain back off this decoder\'s own decode')
    ap.add_argument('--binary', default=find_oadec(required=False))
    ap.add_argument('--work', type=Path, default=Path('.'), help='a directory for the decodes (default: this one)')
    ap.add_argument('--step', type=int, default=16, help='samples a step (default 16)')
    ap.add_argument('--out', type=Path, help='write the report here as JSON')
    args = ap.parse_args()
    if bool(args.reference) == args.self_check:
        ap.error('give the objects of another decoder, or --self-check')
    args.work.mkdir(parents=True, exist_ok=True)

    with_gain = decode(args.binary, args.clip, args.work)
    plain = decode(args.binary, args.clip, args.work, '--no-clip-gain')
    channels = plain.shape[1]
    if args.self_check:
        reference = with_gain
    else:
        values = np.fromfile(args.reference, dtype='<f4')
        if len(values) % channels:
            sys.exit(f'{args.reference}: not rows of {channels} values')
        reference = values.reshape(-1, channels).astype(np.float64)
    if abs(len(reference) - len(plain)) > 4096:
        sys.exit(f'{len(reference)} rows of the reference against {len(plain)} of the decode')
    n = min(len(plain), len(reference))
    frame = frame_length(args.binary, args.clip)
    frames = (n + CORE_DELAY) // frame - 1
    objects = list(range(1, channels))

    def rows_of(f: int, a: int, b: int) -> slice:
        """The rows of the samples `a` to `b` of what frame `f` decodes to."""
        start = f * frame - CORE_DELAY
        return slice(max(0, start + a), max(0, start + b))

    # The gain of every frame as its payload gives it: off the part of the frame
    # where every decoder holds it, on the reference, rounded to the 32nd the
    # payload counts in. The LFE where it sounds, the objects together where it
    # does not.
    gains = np.full(frames, np.nan)
    hold = (min(320, frame // 2), frame - 36)
    for f in range(1, frames):
        for members in ([0], objects):
            g = over(reference, plain, rows_of(f, *hold), members)
            if g is not None:
                gains[f] = round(g * 32) / 32
                break
    known = np.isfinite(gains)
    held = []
    for f in np.flatnonzero(known):
        for members in ([0], objects):
            g = over(reference, plain, rows_of(int(f), *hold), members)
            if g is not None:
                held.append(abs(g - gains[f]))
                break
    alone = [f for f in range(3, frames - 3)
             if known[f - 2:f + 3].all() and gains[f - 1] == gains[f - 2] and gains[f] != gains[f - 1]
             and gains[f + 1] == gains[f] and abs(gains[f] - gains[f - 1]) >= 0.03]
    report = {
        'frames': int(frames), 'samples_a_frame': frame,
        'frames_whose_gain_was_read': int(known.sum()),
        'gains': sorted({float(v) for v in gains[known]}),
        'frames_where_the_gain_changes': int((np.diff(gains[known]) != 0).sum()),
        'where_the_gain_holds_its_largest_distance_to_a_32nd': float(max(held)) if held else None,
        'changes_that_stand_alone': len(alone),
        'step_samples': args.step,
    }
    print(f"{frames} frames of {frame} samples; the gain takes {len(report['gains'])} values from "
          f"{report['gains'][0]} to {report['gains'][-1]} and changes at {report['frames_where_the_gain_changes']} "
          f"frames; where it holds it is a 32nd to {report['where_the_gain_holds_its_largest_distance_to_a_32nd']:.1e}; "
          f"{len(alone)} changes of 3 per cent or more stand alone", file=sys.stderr)
    positions = list(range(-256, 512, args.step))
    for name, members in (('the_lfe', [0]), ('the_objects', objects)):
        shapes = []
        for f in alone:
            before, after = gains[f - 1], gains[f]
            row = []
            for p in positions:
                g = over(reference, plain, rows_of(f, p, p + args.step), members)
                row.append(np.nan if g is None else (g - before) / (after - before))
            shapes.append(row)
        if not shapes or not np.isfinite(np.array(shapes)).any():
            report[name] = None
            print(f'{name}: silent at every change', file=sys.stderr)
            continue
        shapes = np.array(shapes)
        read = np.isfinite(shapes).sum(axis=0)
        with np.errstate(all='ignore'):
            middle = np.nanmedian(shapes, axis=0)
        report[name] = {
            'changes_read_at_most': int(read.max()),
            'by_sample_of_the_frame': {str(p): (round(float(m), 3) if np.isfinite(m) else None)
                                       for p, m in zip(positions, middle)},
        }
        print(f'\n{name}: from the value before (0) to the new one (1), by sample of the frame the new gain comes '
              f'with; the median of up to {int(read.max())} changes', file=sys.stderr)
        for i in range(0, len(positions), 8):
            print('  ' + '  '.join(f'{positions[j]:5d}:{middle[j]:6.3f}' for j in range(i, min(i + 8, len(positions)))),
                  file=sys.stderr)
    verdict = 0
    if args.self_check:
        # this decoder: i / 256 of the way at sample i of the frame, and there from sample 256
        want = {p: (0.0 if p + args.step <= 0 else 1.0 if p >= 256 else (p + (args.step - 1) / 2) / 256)
                for p in positions if p + args.step <= 0 or p >= 0}
        off = []
        for name in ('the_lfe', 'the_objects'):
            if report[name]:
                got = report[name]['by_sample_of_the_frame']
                off += [abs(got[str(p)] - w) for p, w in want.items() if got[str(p)] is not None]
        report['self_check'] = {'largest_distance_to_the_line': max(off) if off else None, 'steps_read': len(off)}
        print(f"\nself-check: {len(off)} steps read, the furthest {max(off) if off else float('nan'):.4f} off the line",
              file=sys.stderr)
        verdict = 0 if off and max(off) < 0.01 else 1
    print(json.dumps({k: v for k, v in report.items() if k not in ('the_lfe', 'the_objects', 'gains')}))
    if args.out:
        args.out.write_text(json.dumps(report, indent=1) + '\n', encoding='utf-8', newline='\n')
    return verdict


if __name__ == '__main__':
    sys.exit(main())
