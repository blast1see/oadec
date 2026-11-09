#!/usr/bin/env python3
"""Count every rule's findings over a corpus of TrueHD streams.

A rule's default severity is decided on what it finds in real material, not on
what its unit test builds: a rule that fires on streams nobody doubts is a
rule to weaken, and a rule that never fires on them is one that can be trusted
to mean something. This runs `verify --json --max-per-rule 1` over every
stream and reports, per rule, how many streams it fired on and how many
findings it made, with the streams named.

    python tools/rule_census.py --oadec target/release/oadec.exe \
        --out measure/census.json E:/work/thd E:/work/clips

The exit code is 0; the census is a measurement, not a gate.
"""

import argparse
import concurrent.futures
import json
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

SUFFIXES = {'.thd', '.mlp', '.truehd'}


def streams(paths):
    out = []
    for p in map(Path, paths):
        if p.is_dir():
            out += sorted(q for q in p.rglob('*') if q.suffix.lower() in SUFFIXES)
        elif p.is_file():
            out.append(p)
        else:
            sys.exit(f'not found: {p}')
    return out


def one(exe, path, decode):
    cmd = [exe, 'verify', '--json', '--max-per-rule', '1']
    if decode:
        cmd.append('--decode')
    r = subprocess.run(cmd + [str(path)], capture_output=True, stdin=subprocess.DEVNULL)
    try:
        report = json.loads(r.stdout)
    except json.JSONDecodeError:
        return {'stream': path.name, 'exit': r.returncode, 'error': r.stderr.decode('utf-8', 'replace')[:300]}
    d = report.get('diagnostics', {})
    return {
        'stream': path.name,
        'exit': r.returncode,
        'clean': report.get('clean'),
        'rules': {x['id']: {'count': x['count'], 'magnitude': x['magnitude'],
                            'first': x['first_message']} for x in d.get('rules', [])},
        'regions': len(d.get('regions', [])),
    }


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--oadec', required=True)
    ap.add_argument('--out')
    ap.add_argument('--decode', action='store_true', help='also run verify --decode')
    ap.add_argument('--jobs', type=int, default=4)
    ap.add_argument('paths', nargs='+')
    args = ap.parse_args()
    files = streams(args.paths)
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        rows = list(pool.map(lambda f: one(args.oadec, f, args.decode), files))
    per_rule = defaultdict(lambda: {'streams': [], 'findings': 0})
    for row in rows:
        for rule, v in row.get('rules', {}).items():
            per_rule[rule]['streams'].append(row['stream'])
            per_rule[rule]['findings'] += v['count']
    summary = {
        'streams': len(rows),
        'exit_codes': {str(c): sum(1 for r in rows if r['exit'] == c) for c in sorted({r['exit'] for r in rows})},
        'rules': {r: {'streams': len(v['streams']), 'findings': v['findings'], 'named': v['streams'][:20]}
                  for r, v in sorted(per_rule.items())},
        'rows': rows,
    }
    if args.out:
        Path(args.out).write_text(json.dumps(summary, indent=1), encoding='utf-8')
    print(f"{len(rows)} streams; exit codes {summary['exit_codes']}")
    for rule, v in summary['rules'].items():
        print(f"  {rule:<32} {v['streams']:>4} streams {v['findings']:>9} findings  {', '.join(v['named'][:4])}")
    return 0


if __name__ == '__main__':
    sys.exit(main())
