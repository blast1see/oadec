#!/usr/bin/env python3
"""Run two oadec binaries over the same streams and report what changed.

A rule that classifies streams is only known to be harmless once the old and
the new binary have been run over real material and every change of verdict has
been named. This does the running and the comparing:

    python tools/old_new.py --old old/oadec.exe --new new/oadec.exe \
        --out measure/old-new.json E:/work/thd E:/work/clips

For every stream (files ending in .thd, .mlp, .truehd, .ec3, .eac3 or .ac3, or
given by name) it runs the four report commands, `info`, `verify`, `emdf` and
`oamd`, each as text and with `--json`, with both binaries, and with `--decode`
also `decode -p 2 --format pcm` (TrueHD only), then reports:

- every exit code that differs;
- every JSON field the old binary wrote whose value the new one changed or
  dropped (fields only the new binary writes are listed by name, not judged);
- every line the old binary printed, on stdout or on stderr, that the new one
  no longer prints in the same order (lines only the new binary prints are
  listed by what they begin with, not judged), and whether a report that
  ended on `Speed:` and `Result:` still does;
- for decodes, whether the output bytes are identical, and the lines the
  decode printed while it wrote them, as above.

`--decode-all` compares every delivery instead of one: for TrueHD and MLP the
PCM of presentations 0 to 3, for AC-3 and E-AC-3 the PCM of the programme and
of the core alone, and for both a WAVE file of what `decode` writes when it is
not told which, the DAMF set and the ADM file. Each command
writes into a directory of its own and every file found there afterwards is
hashed, whatever the command named it, so a delivery this tool has the name of
wrong cannot pass as two empty sets agreeing. A command that exits 0 and writes
nothing is reported, and the summary counts what each kind of delivery
compared. The outputs are large; `--tmp` says where they go.

What differs between two runs of the same binary is left out: the fields
`seconds` and `realtime_factor`, the seconds and the speed a line names, and
the directory a delivery was written to. The summary counts what each command
compared, because two commands that both printed nothing also agree.

Two binaries that are both told of a file that is not there also agree, to the
line. So a stream is looked at before the first command and after the last
one, and one that was not there either time, or that was another size or
written at another time by the end, is a change of its own: what was compared
was not that stream. The summary counts the bytes of the streams.

The exit code of this tool is 0 when nothing changed, 1 when something did, 2
on a usage error.
"""

import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

STREAM_SUFFIXES = {'.thd', '.mlp', '.truehd', '.ec3', '.eac3', '.ac3'}
TRUEHD_SUFFIXES = {'.thd', '.mlp', '.truehd'}
VOLATILE = {'seconds', 'realtime_factor'}
# The report commands, and whether the text of each closes on `Speed:` and
# `Result:`.
REPORTS = {'info': False, 'verify': True, 'emdf': True, 'oamd': True}
# The time a run took, as a line names it: `0.05 s (38x realtime)` anywhere,
# and the `0.02 s` that is all a `Speed:` line has when nothing was decoded.
TIMED = re.compile(r'\d+(?:\.\d+)? s \(\d+(?:\.\d+)?x realtime\)|(?<=^Speed:)(\s+)\d+(?:\.\d+)? s$')


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


def seen(stream):
    """The size of a stream and when it was last written, or None when it is
    not there to be read."""
    try:
        st = os.stat(stream)
    except OSError:
        return None
    return {'bytes': st.st_size, 'written_ns': st.st_mtime_ns}


def held(before, after):
    """What became of a stream between the first command and the last, as the
    changes to report: none when it was there both times and the same."""
    if before is None or after is None:
        when = 'before the first command' if before is None else 'after the last command'
        return [{'command': 'every command', 'what': 'the stream was not there to be read',
                 'detail': f'not there {when}'}]
    if before != after:
        return [{'command': 'every command', 'what': 'the stream changed while it was compared',
                 'before': before, 'after': after}]
    return []


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


def steady(text, work=None):
    """The lines of what a command printed, without what a second run of the
    same binary would print otherwise: the time it took, and `work`, the
    directory it was told to write into."""
    lines = []
    for line in text.splitlines():
        if work is not None:
            for sep in ('\\', '/'):
                line = line.replace(f'{work}{sep}', '<out>/')
        lines.append(TIMED.sub(lambda m: (m.group(1) or '') + '<time>', line))
    return lines


def label(line):
    """What a line begins with: its name up to the colon, or its first words."""
    head = line.split(':', 1)[0] if ':' in line[:40] else ' '.join(line.split()[:4])
    return head.strip()[:40]


def leaves(value):
    """Every value of a JSON report that is not itself a list or an object."""
    if isinstance(value, dict):
        for k, v in value.items():
            if k not in VOLATILE:
                yield from leaves(v)
    elif isinstance(value, list):
        for v in value:
            yield from leaves(v)
    else:
        yield value


def text_check(old_lines, new_lines, closes=False):
    """Old lines must appear among the new ones in the same order. Returns the
    problems and the labels of the lines only the new output has. With
    `closes`, a new output must end with Speed then Result where the old one
    printed anything."""
    problems, added = [], []
    at = 0
    for line in old_lines:
        found = at
        while found < len(new_lines) and new_lines[found] != line:
            found += 1
        if found == len(new_lines):
            problems.append(f'missing or out of order: {line!r}')
            break
        added += new_lines[at:found]
        at = found + 1
    else:
        added += new_lines[at:]
    if closes and new_lines and old_lines:
        if not new_lines[-1].startswith('Result:'):
            problems.append(f'last line is not Result: {new_lines[-1]!r}')
        elif len(new_lines) < 2 or not new_lines[-2].startswith('Speed:'):
            problems.append('the line before Result is not Speed')
    return problems, sorted({label(line) for line in added if line.strip()})


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def deliveries(stream):
    """(name, arguments, value of -o) for every delivery of one stream."""
    s = str(stream)
    if stream.suffix.lower() in TRUEHD_SUFFIXES:
        for p in range(4):
            yield f'pcm -p {p}', ['decode', s, '-p', str(p), '--format', 'pcm'], 'out.pcm'
    else:
        yield 'pcm', ['decode', s, '--format', 'pcm'], 'out.pcm'
        yield 'pcm --core-only', ['decode', s, '--format', 'pcm', '--core-only'], 'out.pcm'
    # the header, and the mask that says which channel is which
    yield 'wav', ['decode', s, '--format', 'wav'], 'out.wav'
    # the object formats take a base name and add their own extensions
    yield 'damf', ['decode', s, '--format', 'damf'], 'out'
    yield 'adm', ['decode', s, '--format', 'adm'], 'out'


def deliver(exe, cmd, target, work, timeout):
    """Runs one delivery into `work`, hashes every file it left there and
    keeps the lines it printed."""
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    said = {'stdout': [], 'stderr': []}
    try:
        code, out, err = run(exe, [*cmd, '-o', str(work / target)], timeout)
        said = {'stdout': steady(out, work), 'stderr': steady(err, work)}
    except subprocess.TimeoutExpired:
        code = 'timeout'
    hashes = {f.name: {'bytes': f.stat().st_size, 'sha256': sha256(f)}
              for f in sorted(work.iterdir()) if f.is_file()}
    shutil.rmtree(work, ignore_errors=True)
    return code, hashes, said


def compare_text(rec, command, old, new, closes=False):
    """Compares what two runs printed, stream by stream of the two they
    printed on, and returns how many lines the old run printed."""
    for stream in ('stdout', 'stderr'):
        problems, added = text_check(old[stream], new[stream], closes and stream == 'stdout')
        for p in problems:
            rec['changes'].append({'command': command, 'what': f'text on {stream}', 'detail': p})
        for a in added:
            name = f'{command}: {a}'
            if name not in rec['added_lines']:
                rec['added_lines'].append(name)
    return len(old['stdout']) + len(old['stderr'])


def compare_deliveries(args, stream, rec):
    with tempfile.TemporaryDirectory(dir=args.tmp) as tmp:
        for name, cmd, target in deliveries(stream):
            co, ho, so = deliver(args.old, cmd, target, Path(tmp) / 'old', args.timeout)
            cn, hn, sn = deliver(args.new, cmd, target, Path(tmp) / 'new', args.timeout)
            command = f'decode ({name})'
            lines = compare_text(rec, command, so, sn)
            rec['deliveries'].append({'delivery': name, 'exit': co, 'files': sorted(ho),
                                      'bytes': sum(v['bytes'] for v in ho.values()),
                                      'lines': lines})
            if co != cn:
                rec['changes'].append({'command': command, 'what': 'exit code',
                                       'old': co, 'new': cn})
            if ho != hn:
                rec['changes'].append({'command': command, 'what': 'output bytes',
                                       'old': ho, 'new': hn})
            for side, code, hashes in (('old', co, ho), ('new', cn, hn)):
                if code == 0 and not any(v['bytes'] for v in hashes.values()):
                    rec['changes'].append({'command': command,
                                           'what': f'{side}: exit 0 and nothing written'})


def delivery_kind(name):
    return 'pcm -p N' if name.startswith('pcm -p ') else name


def compare_one(args, stream):
    before = seen(stream)
    rec = {'stream': str(stream), 'bytes': before['bytes'] if before else 0, 'changes': [],
           'added_fields': [], 'added_lines': [], 'reports': [], 'deliveries': []}
    compare_commands(args, stream, rec)
    rec['changes'] += held(before, seen(stream))
    return rec


def compare_commands(args, stream, rec):
    s = str(stream)
    for report, closes in REPORTS.items():
        for name, cmd in ((report, [report, s]), (f'{report} --json', [report, '--json', s])):
            (ce_o, out_o, err_o), (ce_n, out_n, err_n) = (run(args.old, cmd, args.timeout),
                                                          run(args.new, cmd, args.timeout))
            if ce_o != ce_n:
                rec['changes'].append({'command': name, 'what': 'exit code', 'old': ce_o, 'new': ce_n})
            ran = {'command': name, 'exit': ce_o, 'lines': 0, 'fields': 0}
            rec['reports'].append(ran)
            old = {'stdout': steady(out_o), 'stderr': steady(err_o)}
            new = {'stdout': steady(out_n), 'stderr': steady(err_n)}
            if name == report:
                ran['lines'] = compare_text(rec, name, old, new, closes)
                continue
            # what a report says beside its JSON, and all of it when it wrote none
            ran['lines'] = compare_text(rec, name, {'stdout': [], 'stderr': old['stderr']},
                                        {'stdout': [], 'stderr': new['stderr']})
            try:
                jo, jn = json.loads(out_o), json.loads(out_n)
            except json.JSONDecodeError:
                if (out_o.strip() != '') != (out_n.strip() != ''):
                    rec['changes'].append({'command': name, 'what': 'json', 'detail': 'one side wrote no JSON'})
                else:
                    ran['lines'] += compare_text(rec, name, {'stdout': old['stdout'], 'stderr': []},
                                                 {'stdout': new['stdout'], 'stderr': []})
                continue
            ran['fields'] = sum(1 for _ in leaves(jo))
            for path, a, b in walk(jo, jn):
                if path.startswith('+'):
                    field = f'{report}: {path[1:]}'
                    if field not in rec['added_fields']:
                        rec['added_fields'].append(field)
                else:
                    rec['changes'].append({'command': name, 'what': 'field', 'path': path,
                                           'old': a, 'new': b})
    if args.decode_all:
        compare_deliveries(args, stream, rec)
    elif args.decode and stream.suffix.lower() in TRUEHD_SUFFIXES:
        with tempfile.TemporaryDirectory(dir=args.tmp) as tmp:
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


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--old', required=True)
    ap.add_argument('--new', required=True)
    ap.add_argument('--out')
    ap.add_argument('--decode', action='store_true',
                    help='also compare the PCM of presentation 2 (TrueHD only)')
    ap.add_argument('--decode-all', action='store_true',
                    help='also compare every delivery: PCM, DAMF and ADM')
    ap.add_argument('--tmp', help='where the decodes are written (default: the system temporary directory)')
    ap.add_argument('--jobs', type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument('--timeout', type=int, default=3600)
    ap.add_argument('paths', nargs='+')
    args = ap.parse_args()
    # A relative path with forward slashes does not start a process on Windows.
    args.old, args.new = os.path.abspath(args.old), os.path.abspath(args.new)
    for exe in (args.old, args.new):
        if not os.path.isfile(exe):
            print(f'not a file: {exe}', file=sys.stderr)
            return 2
    if args.tmp:
        os.makedirs(args.tmp, exist_ok=True)
    files = streams(args.paths)
    if not files:
        print('no streams found', file=sys.stderr)
        return 2
    with concurrent.futures.ThreadPoolExecutor(args.jobs) as pool:
        recs = list(pool.map(lambda f: compare_one(args, f), files))
    changed = [r for r in recs if r['changes']]
    added = sorted({f for r in recs for f in r['added_fields']})
    added_lines = sorted({f for r in recs for f in r['added_lines']})
    reports, kinds = {}, {}
    for r in recs:
        for ran in r['reports']:
            k = reports.setdefault(ran['command'], {'runs': 0, 'lines': 0, 'fields': 0, 'exit_codes': {}})
            k['runs'] += 1
            k['lines'] += ran['lines']
            k['fields'] += ran['fields']
            k['exit_codes'][str(ran['exit'])] = k['exit_codes'].get(str(ran['exit']), 0) + 1
        for d in r['deliveries']:
            k = kinds.setdefault(delivery_kind(d['delivery']),
                                 {'runs': 0, 'wrote_output': 0, 'bytes': 0, 'lines': 0, 'exit_codes': {}})
            k['runs'] += 1
            k['wrote_output'] += bool(d['bytes'])
            k['bytes'] += d['bytes']
            k['lines'] += d['lines']
            k['exit_codes'][str(d['exit'])] = k['exit_codes'].get(str(d['exit']), 0) + 1
    # The binaries by name and digest: a path says where they were on one
    # machine, the digest says which binaries they were.
    summary = {'old': os.path.basename(args.old), 'old_sha256': sha256(args.old),
               'new': os.path.basename(args.new), 'new_sha256': sha256(args.new),
               'streams': len(recs), 'stream_bytes': sum(r['bytes'] for r in recs),
               'streams_changed': len(changed),
               'fields_added': added, 'lines_added': added_lines, 'reports': reports,
               'deliveries': kinds, 'changed': changed}
    if args.out:
        Path(args.out).write_text(json.dumps(summary, indent=1), encoding='utf-8')
    print(f'{len(recs)} streams, {len(changed)} changed; fields added: {", ".join(added) or "none"}; '
          f'lines added: {", ".join(added_lines) or "none"}')
    print(f'  the streams: {summary["stream_bytes"]} bytes')
    for name, k in reports.items():
        print(f'  {name}: {k["runs"]} runs, {k["lines"]} lines and {k["fields"]} fields compared, '
              f'exit codes {k["exit_codes"]}')
    for kind, k in kinds.items():
        print(f'  {kind}: {k["runs"]} runs, {k["wrote_output"]} wrote output, '
              f'{k["bytes"]} bytes and {k["lines"]} lines compared, exit codes {k["exit_codes"]}')
    for r in changed:
        print(f'  {r["stream"]}')
        for c in r['changes'][:12]:
            print(f'    {c}')
    if args.decode_all and not any(k['bytes'] for k in kinds.values()):
        print('--decode-all compared no output at all', file=sys.stderr)
        return 1
    if not any(k['lines'] or k['fields'] for k in reports.values()):
        print('no report printed a line: nothing was compared', file=sys.stderr)
        return 1
    return 1 if changed else 0


if __name__ == '__main__':
    sys.exit(main())
