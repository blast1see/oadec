#!/usr/bin/env python3
"""Say what the whole tracks of a library hold that their heads do not.

`tools/sweep.py` reads the first minute of every Dolby track of a library, or
with `--seconds 0` all of every track. A minute is seconds of a file and a
whole track is the reading of it, so most of what is counted over a library
is counted over heads, and a head does not exclude what an encoder does once
an hour. This holds the rows of a sweep of whole tracks against the rows of a
sweep of the heads of the same tracks:

    python tools/whole_tracks.py --whole sweep-whole/report.json \
        --heads sweep-heads/report.json --out whole.json
    python tools/whole_tracks.py --self-check

A track is a file's name and the index of the track in it, so the two sweeps
have to be of the same files; a whole track without a head is counted and
compared with nothing. Both sweeps should be made with one binary, or a rule
the newer one has is read as something the heads lack.

What it says, each as counts over the tracks:

- how much was read: tracks, hours, access units and frames;
- the tracks that are clean as a head and not as a whole;
- every rule that fired, in the whole tracks and in the heads, and the tracks
  whose head does not have it;
- every coding tool, the same way;
- of TrueHD and MLP tracks: the places where the timing jumps (a seamless
  branch), which a head has only by accident; the counters and values of the
  syntax census that are zero in every head and not in every whole track,
  and how many stay zero in both; and how many tracks keep one delay through
  the decoder buffer for a minute and how many for all of their length;
- of AC-3 and E-AC-3 tracks: the tracks that say one thing of their downmix
  and of their dialogue level from end to end and those that say more, the
  tracks whose metadata containers change what they declare of their
  protection (no word is read), and of the tracks with JOC the downmix
  configurations, the tracks whose number of objects changes, and the tracks
  with a clip gain their head does not have.

The output names no file.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from census import leaves  # noqa: E402

LOSSLESS = ('truehd', 'mlp')


def key(row: dict) -> tuple:
    return row['title'], row['track']


def lossless(row: dict) -> bool:
    return row.get('codec') in LOSSLESS


def seconds(row: dict) -> float:
    """How long a row's stream is. An AC-3 family row has it. An access unit
    of TrueHD or MLP is forty samples at the base rate of the stream's
    family, whatever multiple of it the stream runs at."""
    if not lossless(row):
        return row.get('duration') or 0.0
    code = ((row.get('declared') or {}).get('format_info') or {}).get('sampling_frequency_code') or 0
    base = 44100 if code & 8 else 48000
    return (row.get('units') or 0) * 40 / base


def counted(rows: list[dict], field: str) -> dict:
    """Per name of `field` (a list of names, or a mapping of names to
    counts): the rows that have it and the sum of its counts."""
    out: dict = {}
    for row in rows:
        value = row.get(field) or {}
        for name in value:
            slot = out.setdefault(name, {'tracks': 0, 'findings': 0})
            slot['tracks'] += 1
            slot['findings'] += value[name] if isinstance(value, dict) else 1
    return out


def beside(pairs: list[tuple], field: str, findings: bool) -> dict:
    """Every name of `field` in the whole tracks and in their heads."""
    whole = counted([w for w, _ in pairs], field)
    heads = counted([h for _, h in pairs if h], field)
    out = {}
    for name in sorted(set(whole) | set(heads)):
        w, h = whole.get(name, {'tracks': 0, 'findings': 0}), heads.get(name, {'tracks': 0, 'findings': 0})
        entry = {'whole_tracks': w['tracks'], 'heads': h['tracks'],
                 'tracks_whose_head_does_not_have_it': sum(
                     1 for a, b in pairs if b and name in (a.get(field) or {}) and name not in (b.get(field) or {}))}
        if findings:
            entry = {'whole_tracks': w['tracks'], 'findings_in_them': w['findings'],
                     'heads': h['tracks'], 'findings_in_the_heads': h['findings'],
                     'tracks_whose_head_does_not_have_it': entry['tracks_whose_head_does_not_have_it']}
        out[name] = entry
    return out


def census_names(rows: list[dict]) -> tuple[dict, set]:
    """Of the censuses of `rows`: per counter or value, the rows in which it
    is not zero and its total; and every name any of them lists."""
    seen: dict = {}
    listed: set = set()
    for row in rows:
        mine: dict = {}
        for group, counter, value, count in leaves(row['census']):
            # the groups per substream repeat the ones of all substreams
            if group.startswith('substream '):
                continue
            name = f'{group}: {counter}' if value is None else f'{group}: {counter}[{value}]'
            mine[name] = mine.get(name, 0) + count
        for name, count in mine.items():
            listed.add(name)
            if count:
                slot = seen.setdefault(name, {'tracks': 0, 'total': 0})
                slot['tracks'] += 1
                slot['total'] += count
    return seen, listed


def branches(row: dict) -> list:
    return (row.get('timing') or {}).get('branches') or []


def one_delay(row: dict) -> bool:
    delay = row.get('delay_samples') or [None, None]
    return delay[0] is not None and delay[0] == delay[1]


def statements(row: dict) -> list:
    """What a row says of its downmix, as the codes of the bit stream."""
    return sorted(json.dumps(x.get('codes'), sort_keys=True) for x in row.get('downmix') or [])


def truehd(pairs: list[tuple]) -> dict:
    whole = [w for w, _ in pairs]
    heads = [h for _, h in pairs if h]
    with_branch = [w for w in whole if branches(w)]
    out: dict = {
        'tracks': len(whole),
        'places_where_the_timing_jumps': {
            'tracks_with_one': len(with_branch),
            'places': sum(len(branches(w)) for w in whole),
            'of_them_a_valid_branch': sum((w.get('timing') or {}).get('valid_branches', 0) for w in whole),
            'of_them_not': sum((w.get('timing') or {}).get('invalid_branches', 0) for w in whole),
            'most_in_one_track': max((len(branches(w)) for w in whole), default=0),
            'heads_with_one': sum(1 for h in heads if branches(h)),
            'places_in_the_heads': sum(len(branches(h)) for h in heads),
        },
        'the_delay_through_the_decoder_buffer': {
            'heads_with_one_value': sum(one_delay(h) for h in heads),
            'whole_tracks_with_one_value': sum(one_delay(w) for w in whole),
        },
    }
    counted_whole = [w for w, h in pairs if w.get('census') and h and h.get('census')]
    counted_heads = [h for w, h in pairs if w.get('census') and h and h.get('census')]
    if counted_whole:
        in_whole, listed = census_names(counted_whole)
        in_heads, listed_heads = census_names(counted_heads)
        listed |= listed_heads
        beyond = sorted(set(in_whole) - set(in_heads))
        out['the_syntax_census'] = {
            'tracks_counted_both_ways': len(counted_whole),
            'counters_and_values': len(listed),
            'zero_in_every_head': len(listed - set(in_heads)),
            'zero_in_every_whole_track': len(listed - set(in_whole)),
            'zero_in_every_head_and_not_in_every_whole_track': [
                {'counter': name, 'tracks': in_whole[name]['tracks'], 'total': in_whole[name]['total']}
                for name in beyond],
            'still_zero_in_every_whole_track': sorted(listed - set(in_whole)),
        }
    return out


def ac3_family(pairs: list[tuple]) -> dict:
    whole = [w for w, _ in pairs]
    with_head = [(w, h) for w, h in pairs if h]

    def said(rows):
        return {
            'tracks_that_say_nothing_of_it': sum(len(statements(r)) == 0 for r in rows),
            'tracks_that_say_one_thing': sum(len(statements(r)) == 1 for r in rows),
            'tracks_that_say_more': sum(len(statements(r)) > 1 for r in rows),
        }

    def levels(rows):
        return {
            'tracks_with_one_value': sum(len(r.get('dialnorm') or []) == 1 for r in rows),
            'tracks_with_more': sum(len(r.get('dialnorm') or []) > 1 for r in rows),
        }

    def protection(rows):
        declared = [r.get('protection') or {} for r in rows]
        return {
            'tracks_with_containers': sum((p.get('containers') or 0) > 0 for p in declared),
            'tracks_whose_containers_change_what_they_declare': sum((p.get('changes') or 0) > 0 for p in declared),
            'key_identifiers': sorted({k for p in declared for k in p.get('key_ids') or {}}, key=int),
            'versions': sorted({k for p in declared for k in p.get('versions') or {}}, key=int),
        }

    objects = [(w, h) for w, h in pairs if w.get('downmix_configs')]
    configurations: dict = {}
    for w, _ in objects:
        for c in w['downmix_configs']:
            configurations[str(c)] = configurations.get(str(c), 0) + 1
    return {
        'tracks': len(whole),
        'the_downmix': {
            'whole': said(whole),
            'heads': said([h for _, h in with_head]),
            'tracks_whose_head_says_something_else': sum(statements(w) != statements(h) for w, h in with_head),
            'whole_tracks_that_say_it_in_every_frame': sum(
                len(statements(w)) == 1 and w['downmix'][0].get('frames') == w.get('units') for w in whole),
        },
        'the_dialogue_normalisation': {
            'whole': levels(whole),
            'heads': levels([h for _, h in with_head]),
            'tracks_whose_head_has_fewer_values': sum(
                len(set(w.get('dialnorm') or [])) > len(set(h.get('dialnorm') or [])) for w, h in with_head),
        },
        'what_the_containers_declare_of_their_protection': {
            'whole': protection(whole),
            'heads': protection([h for _, h in with_head]),
        },
        'the_objects_of_joc': {
            'tracks': len(objects),
            'tracks_by_downmix_configuration': dict(sorted(configurations.items(), key=lambda kv: int(kv[0]))),
            'tracks_with_a_configuration_their_head_does_not_have': sum(
                1 for w, h in objects if h and set(w['downmix_configs']) - set(h.get('downmix_configs') or [])),
            'tracks_whose_number_of_objects_changes': sum(len(w.get('objects_per_payload') or []) > 1 for w, _ in objects),
            'tracks_with_a_clip_gain_other_than_one': sum(
                any(g != 1000 for g in w.get('clipgains') or []) for w, _ in objects),
            'of_them_with_none_in_the_head': sum(
                1 for w, h in objects if h and any(g != 1000 for g in w.get('clipgains') or [])
                and not any(g != 1000 for g in h.get('clipgains') or [])),
        },
    }


def added_up(whole: list[dict], heads: list[dict]) -> dict:
    head_of = {key(h): h for h in heads}
    pairs = [(w, head_of.get(key(w))) for w in whole]
    with_head = [(w, h) for w, h in pairs if h]
    lossless_pairs = [(w, h) for w, h in pairs if lossless(w)]
    lossy_pairs = [(w, h) for w, h in pairs if not lossless(w)]
    formats: dict = {}
    for w in whole:
        name = str(w.get('format') or w.get('codec'))
        slot = formats.setdefault(name, {'tracks': 0, 'hours': 0.0})
        slot['tracks'] += 1
        slot['hours'] += seconds(w) / 3600
    for slot in formats.values():
        slot['hours'] = round(slot['hours'], 1)
    out = {
        'files': len({w['title'] for w in whole}),
        'tracks': len(whole),
        'tracks_with_a_head': len(with_head),
        'tracks_that_could_not_be_cut': sum('could not be written' in str(w.get('first_error')) for w in whole),
        'read': {
            'hours': round(sum(seconds(w) for w in whole) / 3600, 1),
            'hours_of_the_heads': round(sum(seconds(h) for _, h in with_head) / 3600, 1),
            'bytes': sum(w.get('bytes') or 0 for w in whole),
            'access_units_of_truehd_and_mlp': sum(w.get('units') or 0 for w, _ in lossless_pairs),
            'frames_of_the_ac3_family': sum(w.get('units') or 0 for w, _ in lossy_pairs),
            'by_format': dict(sorted(formats.items())),
        },
        'clean': {
            'whole_tracks': sum(bool(w.get('clean')) for w in whole),
            'heads': sum(bool(h.get('clean')) for _, h in with_head),
            'clean_as_a_head_and_not_as_a_whole': sum(bool(h.get('clean')) and not w.get('clean') for w, h in with_head),
            'clean_as_a_whole_and_not_as_a_head': sum(bool(w.get('clean')) and not h.get('clean') for w, h in with_head),
        },
        'rules': beside(pairs, 'rules', findings=True),
        'coding_tools': beside(pairs, 'coverage', findings=False),
        'truehd_and_mlp': truehd(lossless_pairs),
        'ac3_and_e_ac3': ac3_family(lossy_pairs),
    }
    return out


def said_in_short(out: dict) -> str:
    read, clean = out['read'], out['clean']
    thd, ac3 = out['truehd_and_mlp'], out['ac3_and_e_ac3']
    lines = [
        f"{out['tracks']} tracks of {out['files']} files, {read['hours']} hours "
        f"(their heads: {read['hours_of_the_heads']}); {out['tracks_with_a_head']} have a head",
        f"clean: {clean['whole_tracks']} whole, {clean['heads']} heads; "
        f"{clean['clean_as_a_head_and_not_as_a_whole']} clean as a head and not as a whole",
    ]
    for name, rule in out['rules'].items():
        lines.append(f"  rule {name}: {rule['whole_tracks']} whole tracks ({rule['findings_in_them']} findings), "
                     f"{rule['heads']} heads; {rule['tracks_whose_head_does_not_have_it']} only beyond the head")
    beyond = {name: tool['tracks_whose_head_does_not_have_it'] for name, tool in out['coding_tools'].items()
              if tool['tracks_whose_head_does_not_have_it']}
    lines.append('coding tools some tracks use only beyond their head: '
                 + (', '.join(f'{k} {v}' for k, v in sorted(beyond.items(), key=lambda kv: -kv[1])) or 'none'))
    places = thd['places_where_the_timing_jumps']
    lines.append(f"TrueHD and MLP, {thd['tracks']} tracks: the timing jumps at {places['places']} places of "
                 f"{places['tracks_with_one']} tracks ({places['of_them_not']} not a valid branch); "
                 f"the heads have {places['places_in_the_heads']} of them")
    census = thd.get('the_syntax_census')
    if census:
        lines.append(f"  the census: {census['counters_and_values']} counters and values, "
                     f"{census['zero_in_every_head']} zero in every head, "
                     f"{census['zero_in_every_whole_track']} in every whole track")
    mix, level = ac3['the_downmix'], ac3['the_dialogue_normalisation']
    lines.append(f"AC-3 and E-AC-3, {ac3['tracks']} tracks: {mix['whole']['tracks_that_say_more']} say more than "
                 f"one thing of their downmix, {level['whole']['tracks_with_more']} have more than one dialogue "
                 f"normalisation; {mix['tracks_whose_head_says_something_else']} heads say something else of the "
                 f"downmix than their track")
    return '\n'.join(lines)


def self_check() -> int:
    def census(**counts):
        fields = {'fir_order': {'0': 0, '8': 0}, 'iir_sent': 0}
        for name, count in counts.items():
            if name in fields['fir_order']:
                fields['fir_order'][name] = count
            else:
                fields[name] = count
        return {'schema': 1, 'major_sync': {'major_syncs': 1}, 'substreams': [{'filter': fields}]}

    def lossless_row(title, units, branches=(), **counts):
        return {'title': title, 'track': 1, 'codec': 'truehd', 'format': 'truehd', 'clean': True, 'units': units,
                'declared': {'format_info': {'sampling_frequency_code': 0}}, 'delay_samples': [40, 40 + units],
                'coverage': ['fir'] + (['iir'] if counts.get('iir_sent') else []), 'rules': {},
                'census': census(**counts),
                'timing': {'branches': list(branches), 'valid_branches': len(branches), 'invalid_branches': 0}}

    def lossy_row(title, track, frames, dialnorm, mixes, **more):
        return {'title': title, 'track': track, 'codec': 'eac3', 'format': 'E-AC-3', 'clean': True,
                'units': frames, 'duration': frames * 0.032, 'coverage': ['dither'], 'rules': {},
                'dialnorm': dialnorm,
                'downmix': [{'frames': n, 'codes': {'dmixmod': code}} for code, n in mixes],
                'protection': {'containers': frames, 'changes': 0, 'key_ids': {'0': frames}, 'versions': {'0': frames}},
                **more}

    heads = [
        lossless_row('a', 72000, **{'8': 5}),
        lossy_row('a', 2, 1875, [27], [(2, 1875)]),
        lossy_row('b', 1, 1875, [31], [], downmix_configs=[3], objects_per_payload=[15], clipgains=[1000]),
    ]
    whole = [
        lossless_row('a', 7200000, branches=[{'unit': 9, 'valid': True}], **{'8': 500, '0': 7, 'iir_sent': 3}),
        {**lossy_row('a', 2, 225000, [27, 31], [(2, 200000), (1, 25000)]), 'clean': False,
         'rules': {'frame.tail_overrun': 4}},
        lossy_row('b', 1, 225000, [31], [], downmix_configs=[3], objects_per_payload=[15, 11],
                  clipgains=[1000, 1031]),
        lossy_row('c', 1, 1000, [31], [(2, 1000)]),
    ]
    out = added_up(whole, heads)
    thd, ac3 = out['truehd_and_mlp'], out['ac3_and_e_ac3']
    census_said = thd['the_syntax_census']
    answers = {
        'a track without a head is counted and compared with nothing':
            out['tracks'] == 4 and out['tracks_with_a_head'] == 3 and out['files'] == 3,
        'the hours are those of the access units and of the frames':
            out['read']['hours'] == round((7200000 / 1200 + (225000 + 225000 + 1000) * 0.032) / 3600, 1)
            and out['read']['access_units_of_truehd_and_mlp'] == 7200000
            and out['read']['frames_of_the_ac3_family'] == 451000,
        'a track that is clean for a minute and not for its length is counted':
            out['clean'] == {'whole_tracks': 3, 'heads': 3, 'clean_as_a_head_and_not_as_a_whole': 1,
                             'clean_as_a_whole_and_not_as_a_head': 0},
        'a rule only the whole track has is one its head does not have':
            out['rules'] == {'frame.tail_overrun': {
                'whole_tracks': 1, 'findings_in_them': 4, 'heads': 0, 'findings_in_the_heads': 0,
                'tracks_whose_head_does_not_have_it': 1}},
        'a coding tool used only beyond the head is counted for its track':
            out['coding_tools']['iir'] == {'whole_tracks': 1, 'heads': 0, 'tracks_whose_head_does_not_have_it': 1}
            and out['coding_tools']['fir']['tracks_whose_head_does_not_have_it'] == 0,
        'a branch is a place of the whole track and of no head':
            thd['places_where_the_timing_jumps'] == {
                'tracks_with_one': 1, 'places': 1, 'of_them_a_valid_branch': 1, 'of_them_not': 0,
                'most_in_one_track': 1, 'heads_with_one': 0, 'places_in_the_heads': 0},
        'what is zero in every head and not in the whole track is listed with its count':
            census_said['zero_in_every_head_and_not_in_every_whole_track'] == [
                {'counter': 'all substreams filter: fir_order[0]', 'tracks': 1, 'total': 7},
                {'counter': 'all substreams filter: iir_sent', 'tracks': 1, 'total': 3}]
            and census_said['zero_in_every_whole_track'] == 0 and census_said['zero_in_every_head'] == 2
            and census_said['counters_and_values'] == 4,
        'a track that says two things of its downmix and of its dialogue is told from one that says one':
            ac3['the_downmix']['whole'] == {'tracks_that_say_nothing_of_it': 1, 'tracks_that_say_one_thing': 1,
                                            'tracks_that_say_more': 1}
            and ac3['the_downmix']['tracks_whose_head_says_something_else'] == 1
            and ac3['the_downmix']['whole_tracks_that_say_it_in_every_frame'] == 1
            and ac3['the_dialogue_normalisation']['whole'] == {'tracks_with_one_value': 2, 'tracks_with_more': 1}
            and ac3['the_dialogue_normalisation']['tracks_whose_head_has_fewer_values'] == 1,
        'what changes in the objects beyond the head is counted':
            ac3['the_objects_of_joc'] == {
                'tracks': 1, 'tracks_by_downmix_configuration': {'3': 1},
                'tracks_with_a_configuration_their_head_does_not_have': 0,
                'tracks_whose_number_of_objects_changes': 1, 'tracks_with_a_clip_gain_other_than_one': 1,
                'of_them_with_none_in_the_head': 1},
        'the output names no file':
            not any(t in json.dumps(out) for t in ('"a"', '"b"', '"c"', 'title')),
        'the short form says the count':
            '4 tracks of 3 files' in said_in_short(out),
    }
    for what, ok in answers.items():
        print(f"{'ok  ' if ok else 'FAIL'} {what}")
    failed = [what for what, ok in answers.items() if not ok]
    print(f'{len(answers) - len(failed)} of {len(answers)} known answers')
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
    ap.add_argument('--whole', help='the report of a sweep with --seconds 0')
    ap.add_argument('--heads', help='the report of a sweep of the heads of the same files')
    ap.add_argument('--out', help='where to write the output; a short form is printed')
    ap.add_argument('--self-check', action='store_true', help='answer known questions and stop')
    args = ap.parse_args()
    if args.self_check:
        return self_check()
    if not args.whole or not args.heads:
        ap.error('give --whole and --heads')
    whole = json.loads(Path(args.whole).read_text(encoding='utf-8'))
    heads = json.loads(Path(args.heads).read_text(encoding='utf-8'))
    out = added_up(whole, heads)
    if args.out:
        Path(args.out).write_text(json.dumps(out, indent=1, ensure_ascii=False) + '\n', encoding='utf-8')
    print(said_in_short(out))
    return 0


if __name__ == '__main__':
    sys.exit(main())
