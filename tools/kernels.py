#!/usr/bin/env python3
"""The kernels of the decoder, several runs of them, on processors of one kind.

    python tools/kernels.py --out kernels.json
    python tools/kernels.py --affinity FFFF --runs 5 --filter checksum

`cargo bench -p oadec-bench` times the kernels inside one process, on
whichever processor the system has the thread on at the time. On a processor
with cores of two speeds that is not one figure. The same kernel of the same
build measured 11.0 ms in one run and 21.2 ms in the next, each with a spread
under two per cent: the second ran on a slow core from its first batch to its
last, and the spread within a run cannot show that.

This runs the benchmark `--runs` times and gives every kernel as the least of
the medians of its runs, with the largest beside it. Where the two are far
apart a run was somewhere else, and the line is marked. `--affinity` binds
this process, and the benchmark it starts, to the processors of a hexadecimal
mask (FFFF is the first sixteen), which keeps every run on cores of one kind
where the mask names such cores.

A command measured from outside by `tools/bench.py` is a process of its own
and has not shown the same: see `docs/performance.md`.

Exit code: 0, 1 when the benchmark failed or no kernel matched, 2 on a usage
error.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from bench import machine  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
# A run this much slower than the fastest of its kernel was not the same run.
APART = 1.2


def bind(mask):
    """Binds this process, and every process it starts, to the processors of
    `mask`."""
    if os.name == 'nt':
        import ctypes
        from ctypes import wintypes
        k32 = ctypes.WinDLL('kernel32', use_last_error=True)
        k32.GetCurrentProcess.restype = wintypes.HANDLE
        k32.SetProcessAffinityMask.argtypes = [wintypes.HANDLE, ctypes.c_size_t]
        k32.SetProcessAffinityMask.restype = wintypes.BOOL
        if not k32.SetProcessAffinityMask(k32.GetCurrentProcess(), mask):
            raise OSError(ctypes.get_last_error(), 'the system refused the mask')
    else:
        os.sched_setaffinity(0, {i for i in range(mask.bit_length()) if mask >> i & 1})


def readable(ns):
    for unit, scale in (('s', 1e9), ('ms', 1e6), ('us', 1e3)):
        if ns >= scale:
            return f'{ns / scale:.3f} {unit}'
    return f'{ns:.1f} ns'


def gather(runs):
    """One row a kernel from the rows of every run."""
    kernels = []
    for rows in zip(*runs):
        assert len({r['kernel'] for r in rows}) == 1, rows
        best = min(rows, key=lambda r: r['median_ns'])
        medians = [r['median_ns'] for r in rows]
        kernels.append({
            'kernel': best['kernel'],
            'least_ns': min(medians),
            'most_ns': max(medians),
            'medians_ns': medians,
            'mad_share_of_the_least': best['mad_share'],
            'audio_seconds': best['audio_seconds'],
            'realtime': best['realtime'],
            'runs_apart': max(medians) > APART * min(medians),
        })
    return kernels


def table(kernels):
    width = max(len(k['kernel']) for k in kernels)
    lines = []
    for k in kernels:
        speed = '' if k['realtime'] is None else f"  {k['realtime']:.0f}x real time"
        mark = '  *' if k['runs_apart'] else ''
        lines.append(f"{k['kernel']:<{width}}  {readable(k['least_ns']):>12}  to {readable(k['most_ns']):>12}"
                     f"{speed}{mark}")
    if any(k['runs_apart'] for k in kernels):
        lines.append(f'* a run took more than {APART} times the fastest: it was on other processors, or '
                     'beside other work')
    return '\n'.join(lines)


def main():
    p = argparse.ArgumentParser(description=__doc__.split('\n')[0])
    p.add_argument('--runs', type=int, default=3, help='runs of the benchmark (default 3)')
    p.add_argument('--affinity', type=lambda s: int(s, 16), metavar='HEX',
                   help='bind the benchmark to the processors of this mask')
    p.add_argument('--filter', help='only the kernels whose name has this text')
    p.add_argument('--out', help='write the figures here as JSON')
    args = p.parse_args()
    if args.runs < 1:
        p.error('--runs is at least 1')
    if args.affinity is not None:
        if args.affinity <= 0:
            p.error('--affinity names at least one processor')
        bind(args.affinity)

    runs = []
    with tempfile.TemporaryDirectory() as tmp:
        for n in range(args.runs):
            out = Path(tmp) / f'{n}.json'
            cmd = ['cargo', 'bench', '--locked', '-p', 'oadec-bench', '--', '--json', str(out)]
            if args.filter:
                cmd += ['--filter', args.filter]
            done = subprocess.run(cmd, cwd=ROOT, stdin=subprocess.DEVNULL, capture_output=True, text=True)
            if done.returncode != 0:
                sys.stderr.write(done.stderr)
                return 1
            runs.append(json.loads(out.read_text(encoding='utf-8'))['kernels'])
    if not runs[0]:
        print('no kernel has that text in its name', file=sys.stderr)
        return 1

    kernels = gather(runs)
    print(table(kernels))
    if args.out:
        report = {'schema': 1, 'machine': machine(), 'runs': args.runs,
                  'affinity': None if args.affinity is None else f'{args.affinity:X}',
                  'kernels': kernels}
        Path(args.out).write_text(json.dumps(report, indent=1) + '\n', encoding='utf-8')
    return 0


if __name__ == '__main__':
    sys.exit(main())
