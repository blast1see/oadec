#!/usr/bin/env python3
"""Run two oadec binaries over the same streams and report what changed.

A rule that classifies streams is only known to be harmless once the old and
the new binary have been run over real material and every change of verdict has
been named. This does the running and the comparing:

    python tools/old_new.py --old old/oadec.exe --new new/oadec.exe \
        --out measure/old-new.json E:/work/thd E:/work/clips

For every stream (files ending in .thd, .mlp, .truehd, .ec3, .eac3 or .ac3, or
given by name) it runs `verify`, `verify --json` and `info --json` with both
binaries, and with `--decode` also `decode -p 2 --format pcm` (TrueHD only),
then reports:

- every exit code that differs;
- every JSON field the old binary wrote whose value the new one changed or
  dropped (fields only the new binary writes are listed by name, not judged);
- every text line of `verify` the old binary printed that the new one no
  longer prints in the same order, and whether `Result:` is still the last
  line and `Speed:` the one before it;
- for decodes, whether the output bytes are identical.

Fields that differ between two runs of the same binary (`seconds`,
`realtime_factor`, the `Speed:` line) are left out. The exit code of this tool
is 0 when nothing changed, 1 when something did, 2 on a usage error.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

STREAM_SUFFIXES = {'.thd', '.mlp', '.truehd', '.ec3', '.eac3', '.ac3'}
VOLATILE = {'seconds', 'realtime_factor'}


def streams(paths):
    out = []
    for p in map(Path, paths):
        if p.is_dir():
            out += sorted(q for q in p.rglob('*') if q.suffix.lower() in STREAM_SUFFIXES)
        elif p.is_file():
            out.append(p)
        else:
            sys.exit(f'not found: {p}')
    return out


def run(exe, args, timeout):
    r = subprocess.run([exe, *args], capture_output=True, timeout=timeout,
                       stdin=subprocess.DEVNULL)
    return r.returncode, r.stdout.decode('utf-8', 'replace'), r.stderr.decode('utf-8', 'replace')


def walk(old, new, path=''):
    """Yield (path, old, new) for every old leaf the new value changed or dropped,
    and ('+' + path) for keys only the new value has."""
    if isinstance(old, dict) and isinstance(new, dict):
        for k, v in old.items():
            if k in VOLATILE:
                continue
            sub = f'{path}.{k}' if path else k
            if k not in new:
                yield sub, v, '<dropped>'
            else:
                yield from walk(v, new[k], sub)
        for k in new:
            if k not in old:
                yield '+' + (f'{path}.{k}' if path else k), None, None
    elif isinstance(old, list) and isinstance(new, list):
        if len(old) != len(new):
            yield path + '[len]', len(old), len(new)
        for i, (a, b) in enumerate(zip(old, new)):
            yield from walk(a, b, f'{path}[{i}]')
    elif old != new:
        yield path, old, new


def text_check(old_out, new_out):
    """Old lines (minus Speed) must appear in the new output in the same order;
    the new output must end with Speed then Result."""
    old_lines = [l for l in old_out.splitlines() if not l.startswith('Speed:')]
    new_lines = new_out.splitlines()
    problems = []
    it = iter(new_lines)
    for line in old_lines:
        for cand in it:
            if cand == line:
                break
        else:
            problems.append(f'missing or out of order: {line!r}')
            break
    if new_lines and old_out.strip():
        if not new_lines[-1].startswith('Result:'):
            problems.append(f'last line is not Result: {new_lines[-1]!r}')
        elif len(new_lines) < 2 or not new_lines[-2].startswith('Speed:'):
            problems.append('the line before Result is not Speed')
    return problems


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def compare_one(args, stream):
    rec = {'stream': str(stream), 'changes': [], 'added_fields': []}
    s = str(stream)
    for name, cmd in (('verify', ['verify', s]),
                      ('verify --json', ['verify', '--json', s]),
                      ('info --json', ['info', '--json', s])):
        (ce_o, out_o, _), (ce_n, out_n, _) = (run(args.old, cmd, args.timeout),
                                              run(args.new, cmd, args.timeout))
        if ce_o != ce_n:
            rec['changes'].append({'command': name, 'what': 'exit code', 'old': ce_o, 'new': ce_n})
        if name == 'verify':
            for p in text_check(out_o, out_n):
                rec['changes'].append({'command': name, 'what': 'text', 'detail': p})
            continue
        try:
            jo, jn = json.loads(out_o), json.loads(out_n)
        except json.JSONDecodeError:
            if out_o.strip() or out_n.strip():
                if (out_o.strip() != '') != (out_n.strip() != ''):
                    rec['changes'].append({'command': name, 'what': 'json', 'detail': 'one side wrote no JSON'})
            continue
        for path, a, b in walk(jo, jn):
            if path.startswith('+'):
                if path[1:] not in rec['added_fields']:
                    rec['added_fields'].append(path[1:])
            else:
                rec['changes'].append({'command': name, 'what': 'field', 'path': path,
                                       'old': a, 'new': b})
    if args.decode and stream.suffix.lower() in {'.thd', '.mlp', '.truehd'}:
        with tempfile.TemporaryDirectory() as tmp:
            outs = []
            for tag, exe in (('old', args.old), ('new', args.new)):
                target = os.path.join(tmp, f'{tag}.pcm')
                code, _, _ = run(exe, ['decode', s, '-p', '2', '--format', 'pcm', '-o', target],
                                 args.timeout)
                outs.append((code, sha256(target) if os.path.exists(target) else None))
            if outs[0][0] != outs[1][0]:
                rec['changes'].append({'command': 'decode -p 2', 'what': 'exit code',
                                       'old': outs[0][0], 'new': outs[1][0]})
            if outs[0][1] != outs[1][1]:
                rec['changes'].append({'command': 'decode -p 2', 'what': 'output bytes',
                                       'old': outs[0][1], 'new': outs[1][1]})
    return rec


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--old', required=True)
    ap.add_argument('--new', required=True)
    ap.add_argument('--out')
    ap.add_argument('--decode', action='store_true')
    ap.add_argument('--jobs', type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument('--timeout', type=int, default=3600)
    ap.add_argument('paths', nargs='+')
    args = ap.parse_args()
    files = streams(args.paths)
    if not files:
        print('no streams found', file=sys.stderr)
        return 2
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        recs = list(pool.map(lambda f: compare_one(args, f), files))
    changed = [r for r in recs if r['changes']]
    added = sorted({f for r in recs for f in r['added_fields']})
    summary = {'old': args.old, 'new': args.new, 'streams': len(recs),
               'streams_changed': len(changed), 'fields_added': added,
               'changed': changed}
    if args.out:
        Path(args.out).write_text(json.dumps(summary, indent=1), encoding='utf-8')
    print(f'{len(recs)} streams, {len(changed)} changed; fields added: {", ".join(added) or "none"}')
    for r in changed:
        print(f'  {r["stream"]}')
        for c in r['changes'][:12]:
            print(f'    {c}')
    return 1 if changed else 0


if __name__ == '__main__':
    sys.exit(main())
