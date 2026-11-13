#!/usr/bin/env python3
"""Add up what the major syncs of TrueHD and MLP streams say of them.

A major sync carries fields that no decode depends on: the channel modifiers
and the types of `format_info`, and bits of `flags` that describe the stream.
Whether a rule about one is worth anything is a count over real streams, and
so is how the tables of the description are to be read where it does not say:
this makes the count.

    python tools/declared.py --oadec target/release/oadec.exe --noun streams \
        --out declared.json work/thd work/clips
    python tools/declared.py --rows work/sweep/report.json --noun tracks --out declared.json

The first form reads every stream of the directories (files ending in .thd,
.mlp or .truehd) with `info --json` and `verify --census --json`. The second
adds up the rows `tools/sweep.py` wrote for the tracks of a library, which
hold the same things of the head of each track. `--noun` is what a row is
called in the output.

The output: the bits of `flags` and how many major syncs and rows set each;
the modifier of each presentation against the channels the presentation has;
the types; how many rows keep one delay through the decoder buffer and how
many of them say so; and what the two rules about these fields found. Nothing
in it names a file. `--keep-rows` writes the rows of the first form, which do.
"""

import argparse
import collections
import concurrent.futures
import json
import subprocess
import sys
from pathlib import Path

SUFFIXES = ('.thd', '.mlp', '.truehd')
MODIFIERS = ('twoch_channel_modifier', 'sixch_channel_modifier', 'eightch_channel_modifier')
# The channels with which the tables of the description read a modifier
# whatever else is there or not: with another channel in an eight-channel
# presentation the description would have to say whether it is a surround.
PLAIN = {'L', 'R', 'C', 'LFE', 'Ls', 'Rs'}
RULES = ('buffer.latency_not_constant', 'sync.reserved_bits')


def answer(oadec, *args):
    r = subprocess.run([oadec, *args], capture_output=True, stdin=subprocess.DEVNULL, timeout=3600)
    return r.returncode, json.loads(r.stdout)


def row_of(oadec, path):
    """One stream, in the shape of a row of tools/sweep.py."""
    _, info = answer(oadec, 'info', '--json', str(path))
    code, verified = answer(oadec, 'verify', '--census', '--json', str(path))
    buffer = verified.get('buffer') or {}
    return {
        'title': path.name,
        'exit': code,
        'clean': bool(verified.get('clean')),
        'census': verified.get('census'),
        'rules': {x['id']: x['count'] for x in (verified.get('diagnostics') or {}).get('rules', [])},
        'declared': {
            'flags': info.get('flags'),
            'constant_delay_declared': info.get('constant_delay_declared'),
            'heavy_drc_announced': info.get('heavy_drc_announced'),
            'format_info': info.get('format_info'),
            'presentations': [{k: p.get(k) for k in ('kind', 'channels', 'modifier', 'modifier_meaning')}
                              for p in info.get('presentations') or []],
        },
        'delay_samples': [(buffer.get(k) or {}).get('samples') for k in ('min_latency', 'max_latency')],
    }


def added_up(rows, noun):
    rows = [r for r in rows if r.get('census') and r.get('declared')]
    sync = lambda r: r['census']['major_sync']  # noqa: E731
    fbb = [r for r in rows if sync(r)['fbb_major_syncs'] > 0]
    fba = [r for r in rows if sync(r)['fbb_major_syncs'] == 0]
    out: dict = {
        noun: len(rows),
        f'{noun}_of_the_fba_syntax': len(fba),
        f'{noun}_of_the_fbb_syntax': len(fbb),
        'major_syncs': sum(sync(r)['major_syncs'] for r in rows),
        'major_syncs_of_the_fbb_syntax': sum(sync(r)['fbb_major_syncs'] for r in rows),
    }

    bits = {}
    for bit in range(15, -1, -1):
        count = sum(sync(r)['flag_bits'][str(bit)] for r in rows)
        if count:
            bits[str(bit)] = {
                'major_syncs': count,
                noun: sum(sync(r)['flag_bits'][str(bit)] > 0 for r in rows),
                f'{noun}_of_the_fbb_syntax': sum(sync(r)['flag_bits'][str(bit)] > 0 for r in fbb),
            }
    out['flags_bits_set'] = bits
    out['the_first_major_sync_says_the_delay_is_constant'] = sum(
        bool(r['declared']['constant_delay_declared']) for r in rows)
    out['the_first_major_sync_announces_heavy_drc'] = sum(
        bool(r['declared']['heavy_drc_announced']) for r in rows)

    # The tables below are of the first major sync. They speak for a stream
    # whose modifiers are one value in every major sync, which the census
    # says: it counts each value over all of them.
    out[f'{noun}_whose_modifiers_are_one_value_throughout'] = sum(
        all(sum(1 for n in sync(r)[m].values() if n) == 1 for m in MODIFIERS) for r in fba)
    tables = {}
    for index, name in enumerate(('two channels', 'six channels', 'eight channels')):
        cells = collections.Counter()
        for r in fba:
            p = r['declared']['presentations'][index]
            cells[(' '.join(p['channels']), p['modifier'], p['modifier_meaning'])] += 1
        tables[name] = [
            {'channels': channels, 'modifier': f'{modifier:02b}', 'meaning': meaning, noun: n}
            for (channels, modifier, meaning), n in sorted(cells.items(), key=lambda kv: (kv[0][0], kv[0][1]))
        ]
    out['the_modifier_of_each_presentation_by_its_channels'] = tables
    out['major_syncs_with_the_type_of_six_channels_set'] = sum(sync(r)['sixch_multichannel_type'] for r in rows)
    out['major_syncs_with_the_type_of_eight_channels_set'] = sum(sync(r)['eightch_multichannel_type'] for r in rows)
    beyond = [r for r in fba if set(r['declared']['presentations'][2]['channels']) - PLAIN]
    out['eight_channel_presentations_with_a_channel_beyond_five_and_one'] = {
        noun: len(beyond),
        'of_them_modifier_00': sum(r['declared']['presentations'][2]['modifier'] == 0 for r in beyond),
    }

    placed = [r for r in rows if (r.get('delay_samples') or [None])[0] is not None]
    constant = [r for r in placed if r['delay_samples'][0] == r['delay_samples'][1]]
    out['the_delay_through_the_decoder_buffer'] = {
        f'{noun}_the_model_placed': len(placed),
        f'{noun}_whose_delay_is_one_value': len(constant),
        'of_them_of_the_fbb_syntax': sum(r in fbb for r in constant),
        'of_them_saying_so_in_flags_bit_15': sum(bool(r['declared']['constant_delay_declared']) for r in constant),
    }
    out['what_the_rules_found'] = {
        rule: {noun: sum(rule in (r.get('rules') or {}) for r in rows),
               'findings': sum((r.get('rules') or {}).get(rule, 0) for r in rows)}
        for rule in RULES
    }
    return out


def main():
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--oadec', help='the binary to ask, with directories of streams')
    ap.add_argument('--rows', help='the report of tools/sweep.py, in place of directories')
    ap.add_argument('--noun', default='streams', help='what a row is called in the output')
    ap.add_argument('--out', help='where to write the output; it is printed as well')
    ap.add_argument('--keep-rows', help='where to write the rows read from directories (they name files)')
    ap.add_argument('dirs', nargs='*')
    args = ap.parse_args()
    if bool(args.rows) == bool(args.dirs) or (args.dirs and not args.oadec):
        ap.error('give --rows, or --oadec and directories')
    if args.rows:
        rows = json.loads(Path(args.rows).read_text(encoding='utf-8'))
    else:
        files = sorted(p for d in args.dirs for p in Path(d).rglob('*') if p.suffix.lower() in SUFFIXES)
        with concurrent.futures.ThreadPoolExecutor(4) as pool:
            rows = list(pool.map(lambda p: row_of(args.oadec, p), files))
        if args.keep_rows:
            Path(args.keep_rows).write_text(json.dumps(rows, indent=1), encoding='utf-8')
    out = added_up(rows, args.noun)
    text = json.dumps(out, indent=1, ensure_ascii=False)
    print(text)
    if args.out:
        Path(args.out).write_text(text + '\n', encoding='utf-8')
    return 0


if __name__ == '__main__':
    sys.exit(main())
