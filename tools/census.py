#!/usr/bin/env python3
"""Add up the census of many TrueHD streams and name what none of them carries.

`oadec verify --census --json` counts, for one stream, how often each element
of the TrueHD syntax occurs and with which values. One stream says what one
encoder did with one programme. This adds the counts of many, which is the
only way to say that an element has no material: not "I never saw it", but
"it is zero in every one of N streams, and here is N".

    python tools/census.py sweep/report.json --out census.json
    python tools/census.py a.json b.json sweep/report.json

An input is a report of `tools/sweep.py`, whose TrueHD rows carry a census, or
the output of `oadec verify --census --json` for one stream. For every counter
the result has the total and the number of streams in which it is not zero;
for a count per value the same per value. Substreams are added up by index,
and once more all together.

`never` lists what is zero everywhere: every plain counter, and every value of
a field short enough for the census to list all its values. A value of a long
field (a block size, say) that was never seen is not listed, since the census
does not say which values such a field could take.

The result names no stream. `--names` adds, for every counter seen in at most
`--rare` streams, the titles it was seen in, for looking at them; that output
is for the work directory, not for the repository.
"""

import argparse
import json
import sys
from pathlib import Path


def censuses(path):
    """(title, coverage, census) of every stream an input file holds."""
    data = json.loads(Path(path).read_text(encoding='utf-8'))
    rows = data if isinstance(data, list) else [data]
    for row in rows:
        census = row.get('census')
        if census:
            title = row.get('title') or row.get('file') or str(path)
            if row.get('track') is not None:
                title = f'{title} #{row["track"]}'
            yield title, row.get('coverage') or [], census


def leaves(census):
    """Yields (group, counter, value or None, count) for every number in a
    census. `group` is 'major_sync' or 'substream N <group>'."""
    def group(name, fields):
        for counter, value in fields.items():
            if isinstance(value, dict):
                for key, count in value.items():
                    yield name, counter, key, count
            else:
                yield name, counter, None, value
    yield from group('major_sync', census.get('major_sync', {}))
    for i, substream in enumerate(census.get('substreams', [])):
        for part, fields in substream.items():
            yield from group(f'substream {i} {part}', fields)
            yield from group(f'all substreams {part}', fields)


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('inputs', nargs='+')
    ap.add_argument('--out')
    ap.add_argument('--names', action='store_true',
                    help='name the streams of every counter seen in at most --rare of them')
    ap.add_argument('--rare', type=int, default=3)
    args = ap.parse_args()

    totals = {}   # (group, counter, value) -> [total, streams, titles]
    tools = {}
    streams = 0
    schemas = set()
    for path in args.inputs:
        for title, coverage, census in censuses(path):
            streams += 1
            schemas.add(census.get('schema'))
            for tool in coverage:
                tools[tool] = tools.get(tool, 0) + 1
            seen = {}
            for group, counter, value, count in leaves(census):
                key = (group, counter, value)
                seen[key] = seen.get(key, 0) + count
            for key, count in seen.items():
                slot = totals.setdefault(key, [0, 0, []])
                slot[0] += count
                if count:
                    slot[1] += 1
                    if len(slot[2]) <= args.rare:
                        slot[2].append(title)
    if not streams:
        print('no census in the inputs', file=sys.stderr)
        return 2
    if len(schemas) != 1:
        print(f'the inputs mix census schemas: {sorted(map(str, schemas))}', file=sys.stderr)
        return 2

    groups = {}
    never, rare = [], []
    for (group, counter, value), (total, count, titles) in sorted(
            totals.items(), key=lambda kv: (kv[0][0], kv[0][1], str(kv[0][2]))):
        name = counter if value is None else f'{counter}[{value}]'
        groups.setdefault(group, {})[name] = {'total': total, 'streams': count}
        # the per-index groups repeat the all-substreams ones; report those once
        if group.startswith('substream '):
            continue
        if total == 0:
            never.append(f'{group}: {name}')
        elif count <= args.rare:
            entry = {'counter': f'{group}: {name}', 'total': total, 'streams': count}
            if args.names:
                entry['titles'] = titles
            rare.append(entry)

    result = {'schema': schemas.pop(), 'streams': streams,
              'coding_tools': dict(sorted(tools.items())),
              'never': never, 'rare': rare, 'counters': groups}
    if args.out:
        Path(args.out).write_text(json.dumps(result, indent=1), encoding='utf-8')
    print(f'{streams} streams')
    print('coding tools, by streams that use them: '
          + ', '.join(f'{k} {v}' for k, v in sorted(tools.items(), key=lambda kv: -kv[1])))
    print(f'{len(never)} counters and values are zero in every stream:')
    for line in never:
        print(f'  {line}')
    print(f'{len(rare)} are seen in at most {args.rare} streams:')
    for entry in rare:
        print(f'  {entry["counter"]}: {entry["total"]} in {entry["streams"]}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
