#!/usr/bin/env python3
"""Which lines of the decoder nothing reaches, and which only one kind of
input reaches.

"Tested" has two meanings here and they are worth keeping apart. A line the
unit tests run was run on a bitstream somebody built by hand to run it; a line
the corpus runs was run on what an encoder really writes. A line only the
first kind reaches has no material behind it: the fixture and the code may
share the mistake. A line only the second kind reaches is tested by nothing
CI can run. And a line neither reaches is code that has never run at all.

Three steps, with `cargo-llvm-cov` installed:

    cargo llvm-cov --workspace --locked --lcov --output-path tests.lcov
    python tools/cov_regions.py run --lcov media.lcov --tmp DIR  WORK/thd WORK/ec3 WORK/clips
    python tools/cov_regions.py diff tests.lcov media.lcov --out regions.json

`run` builds an instrumented release binary and runs every reporting command
and every delivery over each stream under the paths given, then writes the
line counts. `diff` takes the lines of the non-test build as the universe (a
test module is not part of what ships) and sorts each into: reached by both,
by the tests only, by the media only, by neither. It reports the totals per
source file and the line ranges of the last three kinds.

Paths in the result are relative to the repository, whatever the machine the
measurement was made on.
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TRUEHD = {'.thd', '.mlp', '.truehd'}
AC3 = {'.ec3', '.eac3', '.ac3'}


def commands(stream, tmp):
    s = str(stream)
    out = str(tmp / 'out')
    if stream.suffix.lower() in TRUEHD:
        yield ['info', s]
        yield ['info', '--json', s]
        yield ['verify', '--decode', '--census', s]
        yield ['verify', '--json', s]
        yield ['emdf', s]
        yield ['oamd', s]
        yield ['au', '--index', '0', s]
        for presentation in range(4):
            yield ['decode', s, '-p', str(presentation), '--format', 'pcm', '-o', os.devnull]
        yield ['decode', s, '--format', 'wav', '-o', out + '.wav']
    else:
        yield ['info', s]
        yield ['verify', s]
        yield ['verify', '--json', s]
        yield ['emdf', s]
        yield ['oamd', s]
        yield ['decode', s, '--format', 'pcm', '-o', os.devnull]
        yield ['decode', s, '--format', 'pcm', '--core-only', '-o', os.devnull]
        yield ['decode', s, '--format', 'wav', '-o', out + '.wav']
    yield ['decode', s, '--format', 'damf', '-o', out]
    yield ['decode', s, '--format', 'adm', '-o', out]


def run(args):
    streams = []
    for p in map(Path, args.paths):
        if p.is_dir():
            streams += sorted(q for q in p.rglob('*') if q.suffix.lower() in TRUEHD | AC3)
        elif p.is_file():
            streams.append(p)
        else:
            sys.exit(f'not found: {p}')
    streams = [s for s in streams if s.stat().st_size <= args.max_bytes]
    if not streams:
        sys.exit('no stream under the size limit')
    tmp = Path(args.tmp)
    cargo = ['cargo', 'llvm-cov']
    # The environment an instrumented build needs, taken once: building once
    # and running the binary itself is a hundred times quicker than asking
    # cargo to run it for every command.
    shown = subprocess.run([*cargo, 'show-env'], cwd=ROOT, check=True, capture_output=True,
                           text=True, encoding='utf-8').stdout
    env = os.environ.copy()
    for line in shown.splitlines():
        name, eq, value = line.partition('=')
        if eq and name.replace('_', '').isalnum():
            env[name] = value.strip().strip('\'"')
    subprocess.run([*cargo, 'clean', '--workspace'], cwd=ROOT, env=env, check=True)
    subprocess.run(['cargo', 'build', '--release', '--locked', '-p', 'oadec-cli'],
                   cwd=ROOT, env=env, check=True)
    exe = ROOT / 'target' / 'release' / ('oadec.exe' if os.name == 'nt' else 'oadec')
    ran = failed = 0
    for n, stream in enumerate(streams, 1):
        for cmd in commands(stream, tmp):
            shutil.rmtree(tmp, ignore_errors=True)
            tmp.mkdir(parents=True)
            r = subprocess.run([str(exe), *cmd], env=env, stdin=subprocess.DEVNULL,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            ran += 1
            # 2 is a refusal (an object format of a stream without objects), 4
            # and 7 are verdicts; anything else did not run
            failed += r.returncode not in (0, 2, 4, 7)
        print(f'[{n}/{len(streams)}] {stream.suffix}', flush=True)
    shutil.rmtree(tmp, ignore_errors=True)
    subprocess.run([*cargo, 'report', '--release', '--lcov', '--output-path', args.lcov],
                   cwd=ROOT, env=env, check=True)
    print(f'{len(streams)} streams, {ran} commands, {failed} did not run')
    return 1 if failed else 0


def read_lcov(path):
    """{repository-relative source file: {line: count}}"""
    files, current = {}, None
    for line in Path(path).read_text(encoding='utf-8', errors='replace').splitlines():
        if line.startswith('SF:'):
            name = line[3:].replace('\\', '/')
            at = name.find('crates/')
            current = files.setdefault(name[at:], {}) if at >= 0 else None
        elif line.startswith('DA:') and current is not None:
            number, count = line[3:].split(',')[:2]
            current[int(number)] = current.get(int(number), 0) + int(count)
        elif line == 'end_of_record':
            current = None
    return files


def ranges(lines):
    """Runs of consecutive lines, as [first, last] pairs."""
    out = []
    for n in sorted(lines):
        if out and n == out[-1][1] + 1:
            out[-1][1] = n
        else:
            out.append([n, n])
    return out


def diff(args):
    tests, media = read_lcov(args.tests), read_lcov(args.media)
    result = {'files': {}, 'totals': {'lines': 0, 'both': 0, 'tests_only': 0,
                                      'media_only': 0, 'neither': 0}}
    for name in sorted(media):
        if '/src/' not in name or name.endswith(('_tests.rs', '/testutil.rs')):
            continue
        if args.only and not any(part in name for part in args.only):
            continue
        by_media = media[name]
        by_tests = tests.get(name, {})
        kinds = {'both': [], 'tests_only': [], 'media_only': [], 'neither': []}
        for line, count in by_media.items():
            hit_tests = by_tests.get(line, 0) > 0
            kind = ('both' if hit_tests else 'media_only') if count > 0 else \
                ('tests_only' if hit_tests else 'neither')
            kinds[kind].append(line)
        entry: dict = {'lines': len(by_media)}
        for kind, lines in kinds.items():
            entry[kind] = len(lines)
            result['totals'][kind] += len(lines)
            if kind != 'both':
                entry[kind + '_ranges'] = ranges(lines)
        result['totals']['lines'] += len(by_media)
        result['files'][name] = entry
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=1), encoding='utf-8')
    t = result['totals']
    print(f'{t["lines"]} lines: {t["both"]} reached by both, {t["tests_only"]} by the tests '
          f'only, {t["media_only"]} by the media only, {t["neither"]} by neither')
    for name, e in result['files'].items():
        print(f'  {name}: {e["lines"]} lines, tests only {e["tests_only"]}, '
              f'media only {e["media_only"]}, neither {e["neither"]}')
    return 0


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    sub = ap.add_subparsers(dest='command', required=True)
    r = sub.add_parser('run', help='run an instrumented build over the streams')
    r.add_argument('--lcov', required=True, help='where to write the line counts')
    r.add_argument('--tmp', required=True, help='where the decodes are written')
    r.add_argument('--max-bytes', type=int, default=200 << 20,
                   help='leave out streams larger than this (default 200 MiB)')
    r.add_argument('paths', nargs='+')
    r.set_defaults(run=run)
    d = sub.add_parser('diff', help='sort the lines by what reaches them')
    d.add_argument('tests')
    d.add_argument('media')
    d.add_argument('--only', action='append',
                   help='a part of a source path to keep; may be repeated')
    d.add_argument('--out')
    d.set_defaults(run=diff)
    args = ap.parse_args()
    return args.run(args)


if __name__ == '__main__':
    sys.exit(main())
