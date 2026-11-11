#!/usr/bin/env python3
"""What decoding costs: wall time, processor time and memory, command by command.

    python tools/bench.py --out bench.json  WORK/thd WORK/ec3 WORK/clips
    python tools/bench.py --refs --out bench.json  WORK/thd WORK/ec3
    python tools/bench.py --compare OLD NEW --out compare.json  WORK/thd WORK/ec3
    python tools/bench.py --report bench.json

Every command runs as a process of its own and is measured from outside it:
the time between its start and its end, the processor time it used in user and
kernel mode, and the most memory it held at once. The decoder cannot report
these about itself without code this repository forbids, and a figure a
program prints about itself is not a measurement of it anyway.

What is run, per stream:

- TrueHD and MLP: `decode -p N --format pcm` for every presentation the stream
  carries (2, 6, 8 and 16 channels on an object stream), `verify`, and
  `verify --decode`; on an object stream also `decode --format damf` and
  `--format adm`.
- AC-3 and E-AC-3: `decode --format pcm` and `verify`; on a stream with object
  metadata also the two object formats.

PCM goes to the null device, so the figure is the decoder's and not the
disk's. The object formats seek back to finish their headers and are written
to `--tmp`; their figures include the writing. `decode` checks the stream on a
second thread while it decodes (`docs/exit-codes.md`), so its processor time
is that of both.

The streams are found under the paths given, sorted into kinds by what
`info --json` says of them (a TrueHD stream also by the channels of its
widest presentation, so that a 16-channel programme is measured where there
is one), and the longest and heaviest `--per-kind` of each kind are measured. Each is read once before it is measured, so the first run does not
pay for the disk. A report names a stream by its kind, its length and its
size, never by its file name, unless `--names` asks for the names.

`--refs` times FFmpeg and `truehdd` on the same streams with the same
measurement, for scale: they do not do the same work (neither checks the
stream beside the decode, and FFmpeg stops at eight channels).

`--compare OLD NEW` runs two binaries turn and turn about and reports the
ratio of their medians with the spread of each, which is what a change that
claims to be faster has to show.

`--verify-floor X` fails the run when `verify` of a TrueHD stream reads less
than X times faster than real time. The conformance suite used to assert that
of one film while it was also the only thing reading the disk.

Exit code: 0, or 1 when a command failed, the floor was missed or a reference
decoder asked for is not installed, 2 on a usage error.
"""

import argparse
import hashlib
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import oadec_bin  # noqa: E402

TRUEHD = {'.thd', '.mlp', '.truehd'}
AC3 = {'.ec3', '.eac3', '.ac3'}
# An integrity fault (7) or a declared loss (4) still decoded the stream.
RAN = {0, 4, 7}

if os.name == 'nt':
    import ctypes
    from ctypes import wintypes

    class _FILETIME(ctypes.Structure):
        _fields_ = [('low', wintypes.DWORD), ('high', wintypes.DWORD)]

        def seconds(self):
            return ((self.high << 32) | self.low) / 1e7

    class _MEMORY(ctypes.Structure):
        _fields_ = [('cb', wintypes.DWORD), ('faults', wintypes.DWORD)] + [
            (name, ctypes.c_size_t) for name in (
                'peak_working_set', 'working_set', 'peak_paged_pool', 'paged_pool',
                'peak_nonpaged_pool', 'nonpaged_pool', 'pagefile', 'peak_pagefile')]

    _K32 = ctypes.WinDLL('kernel32', use_last_error=True)
    _K32.GetProcessTimes.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(_FILETIME)] * 4
    _K32.GetProcessTimes.restype = wintypes.BOOL
    _K32.K32GetProcessMemoryInfo.argtypes = [wintypes.HANDLE, ctypes.POINTER(_MEMORY),
                                             wintypes.DWORD]
    _K32.K32GetProcessMemoryInfo.restype = wintypes.BOOL


def measure(cmd):
    """Runs `cmd` to its end. Returns its exit code, wall seconds, user and
    kernel processor seconds and peak resident bytes."""
    proc = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)
    if os.name == 'nt':
        proc.wait()
        handle = int(proc._handle)
        created, ended, kernel, user = _FILETIME(), _FILETIME(), _FILETIME(), _FILETIME()
        memory = _MEMORY()
        memory.cb = ctypes.sizeof(memory)
        if not _K32.GetProcessTimes(handle, created, ended, kernel, user):
            raise ctypes.WinError(ctypes.get_last_error())
        if not _K32.K32GetProcessMemoryInfo(handle, memory, memory.cb):
            raise ctypes.WinError(ctypes.get_last_error())
        return {'exit': proc.returncode, 'wall': ended.seconds() - created.seconds(),
                'user': user.seconds(), 'kernel': kernel.seconds(),
                'peak': memory.peak_working_set}
    start = time.perf_counter()
    _, status, usage = os.wait4(proc.pid, 0)
    wall = time.perf_counter() - start
    proc.returncode = os.waitstatus_to_exitcode(status)
    # ru_maxrss is kilobytes on Linux and bytes on macOS
    scale = 1 if sys.platform == 'darwin' else 1024
    return {'exit': proc.returncode, 'wall': wall, 'user': usage.ru_utime,
            'kernel': usage.ru_stime, 'peak': usage.ru_maxrss * scale}


def spread(values):
    """Median, the extremes, and the median absolute deviation as a share of
    the median."""
    median = statistics.median(values)
    mad = statistics.median(abs(v - median) for v in values)
    return {'median': round(median, 4), 'min': round(min(values), 4),
            'max': round(max(values), 4),
            'mad_share': round(mad / median, 4) if median else 0.0}


def summarise(runs, seconds):
    wall = spread([r['wall'] for r in runs])
    cpu = spread([r['user'] + r['kernel'] for r in runs])
    out = {'exit': runs[0]['exit'], 'runs': len(runs), 'wall_s': wall, 'cpu_s': cpu,
           'user_s': round(statistics.median(r['user'] for r in runs), 4),
           'kernel_s': round(statistics.median(r['kernel'] for r in runs), 4),
           'peak_mib': round(max(r['peak'] for r in runs) / (1 << 20), 1)}
    if seconds and wall['median']:
        out['realtime'] = round(seconds / wall['median'], 1)
    if seconds and cpu['median']:
        out['realtime_cpu'] = round(seconds / cpu['median'], 1)
    return out


def sha256(path, limit=None):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        left = limit
        while left is None or left > 0:
            chunk = f.read(1 << 20 if left is None else min(left, 1 << 20))
            if not chunk:
                break
            h.update(chunk)
            if left is not None:
                left -= len(chunk)
    return h.hexdigest()


def facts(exe, path):
    """What `info --json` says of a stream, reduced to what a benchmark needs."""
    r = subprocess.run([exe, 'info', '--json', str(path)], capture_output=True,
                       stdin=subprocess.DEVNULL)
    try:
        info = json.loads(r.stdout)
    except json.JSONDecodeError:
        return None
    if path.suffix.lower() in TRUEHD:
        # a presentation the stream does not carry is listed with no channels
        shown = [(p['index'], len(p['channels'])) for p in info.get('presentations', [])
                 if p['channels']]
        objects = bool(info.get('has_16ch_presentation'))
        if objects:
            # the object presentation's channels are counted by the major sync
            shown = [(i, n) for i, n in shown if i != 3]
            shown.append((3, (info.get('sixteen_channel') or {}).get('channels', 16)))
        # a kind per width of the widest presentation, so that a run measures
        # every width the streams have and not only that of the heaviest ones
        widest = max((n for _, n in shown), default=0)
        return {'kind': f'truehd-objects-{widest}ch' if objects else f'truehd-{widest}ch',
                'path': path,
                'seconds': info.get('duration_seconds') or 0.0,
                'bytes': path.stat().st_size, 'sample_rate': info.get('sampling_frequency'),
                'presentations': shown, 'objects': objects}
    channels = len(info.get('channels') or [])
    objects = bool((info.get('joc_extension') or {}).get('flag'))
    kind = 'joc' if objects else 'eac3-7.1' if channels > 6 else \
        'eac3' if info.get('syntax') == 'E-AC-3' else 'ac3'
    return {'kind': kind, 'path': path, 'seconds': info.get('duration') or 0.0,
            'bytes': path.stat().st_size, 'sample_rate': info.get('sample_rate'),
            'channels': channels, 'objects': objects}


def commands(stream, tmp):
    """(label, arguments, directory to empty afterwards) for one stream."""
    s = str(stream['path'])
    if stream['path'].suffix.lower() in TRUEHD:
        for index, channels in stream['presentations']:
            yield (f'decode -p {index} ({channels} ch) pcm',
                   ['decode', s, '-p', str(index), '--format', 'pcm', '-o', os.devnull], None)
        yield 'verify', ['verify', s], None
        yield 'verify --decode', ['verify', '--decode', s], None
    else:
        yield (f'decode ({stream["channels"]} ch) pcm',
               ['decode', s, '--format', 'pcm', '-o', os.devnull], None)
        yield 'verify', ['verify', s], None
    if stream['objects']:
        yield 'decode damf', ['decode', s, '--format', 'damf', '-o', str(tmp / 'out')], tmp
        yield 'decode adm', ['decode', s, '--format', 'adm', '-o', str(tmp / 'out')], tmp


def references(stream, tmp):
    """(label, whole command, directory to empty afterwards), other decoders."""
    s = str(stream['path'])
    if stream['path'].suffix.lower() in TRUEHD:
        yield ('ffmpeg, its default presentation',
               ['ffmpeg', '-v', 'error', '-nostdin', '-i', s, '-f', 's24le', '-y', os.devnull],
               None)
        for index, channels in stream['presentations']:
            yield (f'truehdd --presentation {index} ({channels} ch)',
                   ['truehdd', '--loglevel', 'off', 'decode', '--presentation', str(index),
                    '--format', 'pcm', '--output-path', str(tmp / 'ref'), s], tmp)
    else:
        yield ('ffmpeg, DRC off',
               ['ffmpeg', '-v', 'error', '-nostdin', '-drc_scale', '0', '-i', s,
                '-f', 'f32le', '-y', os.devnull], None)


def timed(cmd, runs, clear, seconds):
    out = []
    for _ in range(runs):
        if clear:
            shutil.rmtree(clear, ignore_errors=True)
            clear.mkdir(parents=True)
        out.append(measure(cmd))
    if clear:
        shutil.rmtree(clear, ignore_errors=True)
    return summarise(out, seconds), out


def machine():
    name = platform.processor()
    if os.name == 'nt':
        import winreg
        with winreg.OpenKey(winreg.HKEY_LOCAL_MACHINE,
                            r'HARDWARE\DESCRIPTION\System\CentralProcessor\0') as key:
            name = winreg.QueryValueEx(key, 'ProcessorNameString')[0].strip()
    elif os.path.exists('/proc/cpuinfo'):
        for line in open('/proc/cpuinfo', encoding='utf-8', errors='replace'):
            if line.startswith('model name'):
                name = line.split(':', 1)[1].strip()
                break
    # what a figure depends on, and nothing that names the machine or its owner
    return {'processor': name, 'logical_processors': os.cpu_count(),
            'system': f'{platform.system()} {platform.release()}'}


def binary(exe):
    version = subprocess.run([exe, '--version'], capture_output=True, text=True).stdout.strip()
    return {'name': os.path.basename(exe), 'version': version, 'sha256': sha256(exe)}


def select(exe, paths, args):
    found = []
    for p in map(Path, paths):
        if p.is_dir():
            found += sorted(q for q in p.rglob('*') if q.suffix.lower() in TRUEHD | AC3)
        elif p.is_file():
            found.append(p)
        else:
            sys.exit(f'not found: {p}')
    kinds = {}
    for path in found:
        f = facts(exe, path)
        if f and args.min_seconds <= f['seconds'] <= args.max_seconds:
            kinds.setdefault(f['kind'], []).append(f)
    chosen = []
    for kind in sorted(kinds):
        ranked = sorted(kinds[kind], key=lambda f: (-f['seconds'], -f['bytes'], f['path'].name))
        for n, f in enumerate(ranked[:args.per_kind], 1):
            f['alias'] = f'{kind}-{n}'
            chosen.append(f)
    return chosen


def describe(stream, names):
    out = {'alias': stream['alias'], 'kind': stream['kind'],
           'seconds': round(stream['seconds'], 3), 'bytes': stream['bytes'],
           'sample_rate': stream['sample_rate']}
    if names:
        out['file'] = stream['path'].name
    return out


def print_report(path):
    """Prints a saved run as a Markdown table: a row a command, the reference
    decoders in brackets. An evidence file that holds a run under `result`
    is read as that run."""
    try:
        data = json.loads(Path(path).read_text(encoding='utf-8'))
    except (OSError, ValueError) as e:
        print(f'{path}: {e}', file=sys.stderr)
        return 2
    if isinstance(data, dict) and 'workloads' not in data:
        data = data.get('result', {})
    if not isinstance(data, dict) or 'workloads' not in data:
        print(f'{path} holds no run of this tool', file=sys.stderr)
        return 2
    m = data['machine']
    print(f'{m["processor"]}, {m["logical_processors"]} logical processors, {m["system"]}; '
          f'{data["runs"]} runs a command, the median of each.')
    for b in data['binaries']:
        print(f'{b["version"]}, SHA-256 {b["sha256"]}')
    print()
    print('| stream | command | x real time | processor s per minute | peak MiB | spread |')
    print('|---|---|---:|---:|---:|---:|')
    for work in data['workloads']:
        minutes = work['seconds'] / 60
        rows = [(r, False) for r in work['results']]
        rows += [(r, True) for r in work.get('references', [])]
        for row, reference in rows:
            if 'wall_s' not in row:
                continue
            name = f'[{row["command"]}]' if reference else row['command']
            print(f'| {work["alias"]}, {work["seconds"]:.0f} s | {name} | {row.get("realtime", 0)} '
                  f'| {row["cpu_s"]["median"] / minutes:.2f} | {row["peak_mib"]} '
                  f'| {100 * row["wall_s"]["mad_share"]:.1f} % |')
    return 0


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--report', metavar='JSON',
                    help='print the table of a saved run and measure nothing')
    ap.add_argument('--bin', help='the binary to measure (default: as tools/oadec_bin.py finds it)')
    ap.add_argument('--compare', nargs=2, metavar=('OLD', 'NEW'))
    ap.add_argument('--refs', action='store_true', help='also time FFmpeg and truehdd')
    ap.add_argument('--runs', type=int, default=5)
    ap.add_argument('--per-kind', type=int, default=2)
    ap.add_argument('--min-seconds', type=float, default=30.0)
    ap.add_argument('--max-seconds', type=float, default=1200.0)
    ap.add_argument('--verify-floor', type=float)
    ap.add_argument('--tmp', help='where the object formats are written')
    ap.add_argument('--names', action='store_true', help='record the file names')
    ap.add_argument('--out')
    ap.add_argument('paths', nargs='*')
    args = ap.parse_args()
    if args.report:
        return print_report(args.report)
    if not args.paths:
        ap.error('give the streams or the directories to measure')
    if args.runs < 1 or args.per_kind < 1:
        ap.error('--runs and --per-kind are at least 1')

    exes = [os.path.abspath(e) for e in (args.compare or [args.bin or oadec_bin.find()])]
    for exe in exes:
        if not os.path.isfile(exe):
            print(f'not a file: {exe}', file=sys.stderr)
            return 2
    tools = {}
    if args.refs:
        tools = {name: shutil.which(name) for name in ('ffmpeg', 'truehdd')}
    tmp = Path(args.tmp or os.path.join(os.environ.get('TEMP', '/tmp'), f'oadec-bench-{os.getpid()}'))
    tmp.mkdir(parents=True, exist_ok=True)

    streams = select(exes[-1], args.paths, args)
    if not streams:
        print('no stream of the length asked for', file=sys.stderr)
        return 2
    failed = [f'{name} is not installed' for name, path in tools.items() if not path]
    report = {'schema': 1, 'machine': machine(), 'runs': args.runs,
              'binaries': [binary(e) for e in exes], 'workloads': []}
    for stream in streams:
        # read once, so that no measured run pays for the disk
        with open(stream['path'], 'rb') as f:
            while f.read(1 << 22):
                pass
        work = describe(stream, args.names)
        work['results'] = []
        print(f'{work["alias"]}: {work["seconds"]:.0f} s, {work["bytes"]} bytes', flush=True)
        for label, cmd, clear in commands(stream, tmp):
            row: dict = {'command': label}
            if args.compare:
                turns = {exe: [] for exe in exes}
                for _ in range(args.runs):
                    for exe in exes:
                        turns[exe] += timed([exe, *cmd], 1, clear, stream['seconds'])[1]
                old, new = (summarise(turns[e], stream['seconds']) for e in exes)
                row.update({'old': old, 'new': new})
                for key in ('wall_s', 'cpu_s'):
                    ratio = new[key]['median'] / old[key]['median'] if old[key]['median'] else 0.0
                    noise = max(0.02, 3 * max(old[key]['mad_share'], new[key]['mad_share']))
                    row[key.replace('_s', '_ratio')] = round(ratio, 3)
                    row[key.replace('_s', '_verdict')] = (
                        'faster' if ratio < 1 - noise else 'slower' if ratio > 1 + noise else 'same')
                exits = {old['exit'], new['exit']}
                print(f'  {label}: wall x{row["wall_ratio"]} ({row["wall_verdict"]}), '
                      f'cpu x{row["cpu_ratio"]} ({row["cpu_verdict"]})', flush=True)
            else:
                row.update(timed([exes[0], *cmd], args.runs, clear, stream['seconds'])[0])
                exits = {row['exit']}
                print(f'  {label}: {row["wall_s"]["median"]:.3f} s wall, '
                      f'{row["cpu_s"]["median"]:.3f} s cpu, {row["peak_mib"]} MiB, '
                      f'{row.get("realtime", 0)}x', flush=True)
                if (args.verify_floor and label == 'verify' and stream['kind'].startswith('truehd')
                        and row.get('realtime', 0) < args.verify_floor):
                    failed.append(f'{work["alias"]}: verify at {row.get("realtime")}x, '
                                  f'under the floor of {args.verify_floor}x')
            if not exits <= RAN:
                failed.append(f'{work["alias"]}: {label} exited {sorted(exits)}')
            work['results'].append(row)
        if args.refs:
            work['references'] = []
            for label, cmd, clear in references(stream, tmp):
                if not tools.get(cmd[0]):
                    continue
                row: dict = {'command': label}
                row.update(timed([tools[cmd[0]], *cmd[1:]], args.runs, clear, stream['seconds'])[0])
                print(f'  [{label}]: {row["wall_s"]["median"]:.3f} s wall, '
                      f'{row["cpu_s"]["median"]:.3f} s cpu, {row["peak_mib"]} MiB, '
                      f'{row.get("realtime", 0)}x', flush=True)
                if row['exit'] != 0:
                    failed.append(f'{work["alias"]}: {label} exited {row["exit"]}')
                work['references'].append(row)
        report['workloads'].append(work)
    shutil.rmtree(tmp, ignore_errors=True)
    report['failed'] = failed
    if args.out:
        Path(args.out).write_text(json.dumps(report, indent=1), encoding='utf-8')
    for line in failed:
        print(f'FAILED: {line}', file=sys.stderr)
    return 1 if failed else 0


if __name__ == '__main__':
    sys.exit(main())
