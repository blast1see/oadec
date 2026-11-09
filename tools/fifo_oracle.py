#!/usr/bin/env python3
"""Hold oadec's decoder-buffer model to the report of Dolby's TrueHD encoder.

Dolby's TrueHD encoder verifies every stream it writes and prints what its
verifier found: the highest delay through the decoder buffer, the deepest each
substream and each decoder fills it, the highest delivery rate and the unit it
was reached at, the largest and the average access unit, and which disc
formats the stream is valid for. That report is the oracle here: an encode is
a black box with a known answer. For every `NAME.mlp` with the encoder's text
log beside it (`NAME.mlp.log`, written with `--extras log_format=txt`), this
reads the report and runs `oadec verify --json` on the stream, and prints the
two side by side.

    python tools/fifo_oracle.py --oadec target/release/oadec.exe --out x.json \
        E:/work/dme/noise71.mlp E:/work/dme/scene.mlp

Only numbers leave the log: the report also names the machine it ran on, and
the log is never to be committed. Exit code 0 when every figure agrees, 1 when
one does not.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


def need(pattern, text, flags=0):
    m = re.search(pattern, text, flags)
    if m is None:
        raise SystemExit(f'the report has no match for {pattern!r}')
    return m


def report(log):
    t = Path(log).read_text(encoding='utf-8', errors='replace')
    m = need(r'\|Maximum data rate\s*\|([\d.]+) Mbps, at access_unit (\d+)', t)
    lat = need(r'\|Maximum FIFO latency\s*\|(.*)\|\s*$', t, re.M).group(1)
    dep = need(r'\|Maximum FIFO depth\s*\|(.*)\|\s*$', t, re.M).group(1)
    allowed = need(r'\|Maximum allowed FIFO depth\|(.*)\|\s*$', t, re.M).group(1)
    block = t[t.index('Maximum FIFO Depth'):]
    block = block[:block.index('+\n', block.index('Whole stream'))]
    cumulative = {m2.group(1).strip(): int(m2.group(2))
                  for m2 in re.finditer(r'\|(\d)\. [^|]+?\s*\|(\d+) bytes', block)}
    validity = t[t.index('Stream validity'):][:400]
    return {
        'max_rate': [round(float(m.group(1)) * 1e6), int(m.group(2))],
        'max_au_bytes': int(need(r'\|Maximum access unit size\s*\|(\d+) bytes', t).group(1)),
        'average_au_bytes': float(need(r'\|Average access unit size\s*\|([\d.]+) bytes', t).group(1)),
        'max_latency': max(int(x) for x in re.findall(r'(\d+) samples', lat)),
        'substream_depth': [int(x) for x in re.findall(r'(\d+) bytes', dep)],
        'substream_allowed': [int(x) for x in re.findall(r'(\d+) bytes', allowed)],
        # 1: substream 0, 2: substreams 0 and 1, 3: the 8-channel decoder,
        # 4: the 16-channel decoder, 5: the whole stream
        'cumulative': cumulative,
        'validity': dict(re.findall(r'\|(DVD-A|HD DVD-Video|BluRay)\s*\|(\w+)', validity)),
        'verification_passed': 'Verification passed' in t,
    }


def oadec(exe, mlp):
    r = subprocess.run([exe, 'verify', '--json', str(mlp)], capture_output=True)
    v = json.loads(r.stdout)
    b = v['buffer']
    by_presentation = {d['presentation']: d['depth'] for d in b['decoders']}
    # the report's rows: 1 = presentation 0, 2 = presentation 1, 3 = the
    # 8-channel decoder (presentation 2, or 1 when there is none), 4 = 16-ch
    cumulative = {'1': by_presentation.get(0), '2': by_presentation.get(1),
                  '3': by_presentation.get(2, by_presentation.get(1)),
                  '5': b['whole_stream']['depth']}
    if 3 in by_presentation:
        cumulative['4'] = by_presentation[3]
    return {
        'max_rate': [b['max_data_rate']['bps'], b['max_data_rate']['unit']],
        'max_au_bytes': v['max_unit_bytes'],
        'average_au_bytes': round(v['total_unit_bytes'] / v['units'], 2),
        'max_latency': b['max_latency']['samples'],
        'substream_depth': [s['depth'] for s in b['substreams']],
        'substream_allowed': [s['allowed'] for s in b['substreams']],
        'cumulative': cumulative,
        'disc': v.get('disc'),
    }


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--oadec', required=True)
    ap.add_argument('--out')
    ap.add_argument('streams', nargs='+')
    args = ap.parse_args()
    args.oadec = str(Path(args.oadec).resolve())
    rows, all_agree = [], True
    for s in map(Path, args.streams):
        d = report(str(s) + '.log')
        o = oadec(args.oadec, s)
        agree = {k: d[k] == o[k] for k in ('max_rate', 'max_au_bytes', 'average_au_bytes',
                                           'max_latency', 'substream_depth', 'substream_allowed')}
        agree['cumulative'] = all(o['cumulative'].get(k) == v for k, v in d['cumulative'].items())
        all_agree &= all(agree.values())
        rows.append({'stream': s.name, 'dolby': d, 'oadec': o, 'agree': agree})
        print(f'{s.name}: ' + ', '.join(f'{k} {"ok" if v else "DIFFERS"}' for k, v in agree.items()))
    if args.out:
        Path(args.out).write_text(json.dumps(rows, indent=1), encoding='utf-8')
    return 0 if all_agree else 1


if __name__ == '__main__':
    sys.exit(main())
