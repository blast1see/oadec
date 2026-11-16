#!/usr/bin/env python3
"""What a stream says of its programme that a master has no place for, as arguments for an encoder.

A decode of a stream to a master (`oadec decode --format damf` or `adm`) has
the sound and where it is. It does not have what the stream says of the
programme as a whole: how loud its dialogue is, how it would be mixed down to
two channels, which dynamic range control it was made with. A master has no
field for any of it, so an encoder that is given the master measures the
first anew and takes its own defaults for the rest, and the second generation
of a stream says something else than the first (`tools/interop.py
programme` counts it).

This reads what a stream says and writes it as what Dolby's encoders take:

    python tools/encoder_arguments.py stream.ec3
    python tools/encoder_arguments.py stream.ec3 --json
    python tools/encoder_arguments.py --self-check

Without `--json` it prints the arguments, a line for an encoder. With it, the
whole of it: what the stream says (`says`), the arguments for each encoder
(`arguments`), what the decode has to be told (`decode`), and what is left to
the encoder and why (`left_to_the_encoder`).

What can be told. Of an AC-3 or E-AC-3 stream: the dialogue normalisation
(`--loudness-management skip:custom_dialnorm=`), the downmix it prefers and
the four levels, and the DRC profiles of line and of RF mode (`--encoder`),
where the stream says one thing of it from its first frame to its last. Of a
TrueHD stream: the dialogue normalisation of its object presentation. The
warp mode is the decode's to keep (`--warp-mode stream`), since a master does
have a place for it.

What cannot, and what it costs. Told a dialogue normalisation, an encoder
measures nothing: the loudness the stream then reports is the figure it was
given, and no true peak. A TrueHD stream says no DRC profile, only the gains
one led to. Where a stream says two things of a field, as a splice of two
programmes does, nothing is told of that field and the output says so.

The options are those of the Dolby Media Encoder's `dee_ddpjoc_encoder` and
`dee_dthd_encoder` (`docs/dolby-tools.md`); `dee_ddp_encoder` takes the same
`--encoder` options after `ddp:` or `dd:`. The first of them does not list
`skip` in its help and takes it.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as oadec_binary  # noqa: E402

# `dmixmod` as the encoders call it; 3 is the code the tables of TS 102 366
# reserve, which the E-AC-3 encoders write when they are told `ltrt-pl2`
PREFERRED = {0: 'not_indicated', 1: 'ltrt', 2: 'loro', 3: 'ltrt-pl2'}
# a level in dB as `info --json` has it, and as the encoders take it
LEVEL = {'+3.0': '+3', '+1.5': '+1.5', '0.0': '0', '-1.5': '-1.5', '-3.0': '-3', '-4.5': '-4.5',
         '-6.0': '-6', '-inf': '-inf'}
SURROUND = ('-1.5', '-3.0', '-4.5', '-6.0', '-inf')
PROFILES = ('film_standard', 'film_light', 'music_standard', 'music_light', 'speech', 'none')


def answer(oadec: str, command: str, stream: Path) -> dict:
    r = subprocess.run([oadec, command, '--json', str(stream)], capture_output=True, text=True,
                       encoding='utf-8', errors='replace')
    try:
        return json.loads(r.stdout)
    except ValueError:
        raise SystemExit(f'`oadec {command} --json` gave no report of {stream.name}: '
                         f'{(r.stderr.strip().splitlines() or ["nothing said"])[-1]}')


def says(oadec: str, stream: Path) -> dict:
    """What a stream says of its programme: every value of each field, in
    the order `oadec` reports them."""
    info = answer(oadec, 'info', stream)
    if 'presentations' in info:
        objects = info.get('sixteen_channel') if info.get('has_16ch_presentation') else None
        return {
            'syntax': 'TrueHD',
            'object_presentation': objects is not None,
            'dialogue_normalisation': [objects['dialogue_norm']] if objects else [],
        }
    emdf = answer(oadec, 'emdf', stream)
    programme = emdf.get('programme_information') or {}
    out = {
        'syntax': info.get('syntax'),
        'frames': info.get('frames'),
        'dialogue_normalisation': info.get('dialnorm') or [],
        'downmix': info.get('downmix'),
        'drc_line': sorted(programme.get('drc_line') or {}),
        'drc_rf': sorted(programme.get('drc_rf') or {}),
    }
    if info.get('joc') is not None:
        out['warp_modes'] = sorted((answer(oadec, 'oamd', stream).get('warp_modes') or {}))
    return out


def arguments(said: dict) -> dict:
    """The arguments that make an encoder write what `said` has, and what is
    left to the encoder."""
    left = []
    loudness = []
    norms = said.get('dialogue_normalisation') or []
    if len(norms) == 1 and 1 <= norms[0] <= 31:
        loudness = ['--loudness-management', f'skip:custom_dialnorm=-{norms[0]}']
    elif len(norms) > 1:
        left.append(f'the dialogue normalisation: the stream says {len(norms)} of them')
    elif said.get('syntax') == 'TrueHD' and not said.get('object_presentation'):
        left.append('the dialogue normalisation: the stream has no object presentation to make a master of')
    else:
        left.append('the dialogue normalisation: the stream says none an encoder takes')
    if said.get('syntax') == 'TrueHD':
        left.append('the downmix and the DRC profiles: a TrueHD stream says neither')
        return {'e_ac_3': loudness, 'truehd': loudness, 'left_to_the_encoder': left}
    options = []
    downmix = said.get('downmix')
    if downmix is None:
        left.append('the downmix: this `oadec` does not report it')
    elif not downmix:
        left.append('the downmix: the stream says none')
    elif len(downmix) > 1 or downmix[0]['frames'] != said.get('frames'):
        left.append('the downmix: the frames of the stream do not all say the same')
    else:
        one, codes = downmix[0], downmix[0]['codes']
        if codes['dmixmod'] is not None:
            options.append(f"preferred_downmix_mode={PREFERRED[codes['dmixmod']]}")
        for option, key in (('loro_cmix', 'loro_centre_db'), ('loro_smix', 'loro_surround_db'),
                            ('ltrt_cmix', 'ltrt_centre_db'), ('ltrt_smix', 'ltrt_surround_db')):
            level = one[key]
            if level is None:
                continue
            if option.endswith('smix') and level not in SURROUND:
                continue
            options.append(f'{option}={LEVEL[level]}')
        reserved = [k for k in ('ltrtsurmixlev', 'lorosurmixlev')
                    if codes[k] is not None and one['ltrt_surround_db' if k.startswith('ltrt') else
                                                    'loro_surround_db'] is None]
        if reserved:
            left.append('a surround level of the downmix: the stream says a reserved code (' + ', '.join(reserved) + ')')
    for option, key in (('drc_profile_line', 'drc_line'), ('drc_profile_rf', 'drc_rf')):
        profiles = said.get(key) or []
        if len(profiles) == 1 and profiles[0] in PROFILES:
            options.append(f'{option}={profiles[0]}')
        elif len(profiles) > 1:
            left.append(f'{option}: the stream says {len(profiles)} of them')
        else:
            left.append(f'{option}: the stream says none an encoder takes')
    return {
        'e_ac_3': loudness + (['--encoder', ':'.join(options)] if options else []),
        # the TrueHD encoder has no option for a downmix, and its DRC profile is of a presentation
        'truehd': loudness,
        'left_to_the_encoder': left,
    }


def report(oadec: str, stream: Path) -> dict:
    said = says(oadec, stream)
    out = {'says': said, 'arguments': arguments(said), 'decode': []}
    out['left_to_the_encoder'] = out['arguments'].pop('left_to_the_encoder')
    if len(said.get('warp_modes') or []) == 1:
        out['decode'] = ['--warp-mode', 'stream']
    return out


def self_check() -> int:
    one = {'frames': 100, 'preferred': 'Lt/Rt', 'ltrt_centre_db': '-4.5', 'ltrt_surround_db': '-1.5',
           'loro_centre_db': '+3.0', 'loro_surround_db': '-inf',
           'codes': {'dmixmod': 1, 'ltrtcmixlev': 5, 'ltrtsurmixlev': 3, 'lorocmixlev': 0, 'lorosurmixlev': 7}}
    stream = {'syntax': 'E-AC-3', 'frames': 100, 'dialogue_normalisation': [27], 'downmix': [one],
              'drc_line': ['music_standard'], 'drc_rf': ['speech']}
    told = arguments(stream)

    def change(**fields):
        return arguments({**stream, **fields})

    def downmix(**fields):
        codes = {**one['codes'], **fields.pop('codes', {})}
        return [{**one, **fields, 'codes': codes}]

    every = [arguments({**stream, 'downmix': downmix(
        ltrt_centre_db=db, loro_centre_db=db, codes={'dmixmod': code % 4})})['e_ac_3'][3]
        for code, db in enumerate(LEVEL)]
    answers = {
        'a stream that says one thing of each field is told whole':
            told['e_ac_3'] == ['--loudness-management', 'skip:custom_dialnorm=-27', '--encoder',
                               'preferred_downmix_mode=ltrt:loro_cmix=+3:loro_smix=-inf:ltrt_cmix=-4.5:'
                               'ltrt_smix=-1.5:drc_profile_line=music_standard:drc_profile_rf=speech']
            and told['left_to_the_encoder'] == [],
        'the TrueHD encoder is told the dialogue normalisation and no more':
            told['truehd'] == ['--loudness-management', 'skip:custom_dialnorm=-27'],
        'every level and every preferred downmix is written as the encoders take it':
            [e.split(':')[0].split('=')[1] for e in every]
            == ['not_indicated', 'ltrt', 'loro', 'ltrt-pl2'] * 2
            and [e.split(':')[1].split('=')[1] for e in every]
            == ['+3', '+1.5', '0', '-1.5', '-3', '-4.5', '-6', '-inf'],
        'two dialogue normalisations are left to the encoder':
            (lambda a: '--loudness-management' not in a['e_ac_3'] and a['e_ac_3'][0] == '--encoder'
             and any('says 2' in w for w in a['left_to_the_encoder']))(change(dialogue_normalisation=[27, 31])),
        'a downmix some frames do not say is left to the encoder':
            (lambda a: 'preferred_downmix_mode' not in ' '.join(a['e_ac_3'])
             and 'drc_profile_line=music_standard' in a['e_ac_3'][3])(change(frames=101)),
        'and so is one of two statements':
            'loro_cmix' not in ' '.join(change(downmix=[{**one, 'frames': 60}, {**one, 'frames': 40}])['e_ac_3']),
        'a stream without a centre channel is told no centre level':
            change(downmix=downmix(ltrt_centre_db=None, loro_centre_db=None,
                                   codes={'ltrtcmixlev': None, 'lorocmixlev': None}))['e_ac_3'][3]
            .startswith('preferred_downmix_mode=ltrt:loro_smix=-inf:ltrt_smix=-1.5:drc'),
        'a reserved surround code is not told, and is named':
            (lambda a: 'ltrt_smix' not in a['e_ac_3'][3] and 'loro_smix=-inf' in a['e_ac_3'][3]
             and any('ltrtsurmixlev' in w for w in a['left_to_the_encoder']))(
                change(downmix=downmix(ltrt_surround_db=None, codes={'ltrtsurmixlev': 1}))),
        'a stream that says no downmix is told none, and no profile it does not name':
            (lambda a: a['e_ac_3'] == ['--loudness-management', 'skip:custom_dialnorm=-27']
             and len(a['left_to_the_encoder']) == 3)(change(downmix=[], drc_line=[], drc_rf=['a profile of tomorrow'])),
        'a decoder that does not report the downmix is said to be one':
            any('does not report' in w for w in change(downmix=None)['left_to_the_encoder']),
        'a TrueHD stream gives its dialogue normalisation to both encoders':
            (lambda a: a['e_ac_3'] == a['truehd'] == ['--loudness-management', 'skip:custom_dialnorm=-24']
             and len(a['left_to_the_encoder']) == 1)(
                arguments({'syntax': 'TrueHD', 'object_presentation': True, 'dialogue_normalisation': [24]})),
        'and none without an object presentation':
            (lambda a: a['truehd'] == [] and 'no object presentation' in a['left_to_the_encoder'][0])(
                arguments({'syntax': 'TrueHD', 'object_presentation': False, 'dialogue_normalisation': []})),
    }
    for what, ok in answers.items():
        print('ok  ' if ok else 'FAIL', what)
    return 0 if all(answers.values()) else 1


def main() -> int:
    parser = argparse.ArgumentParser(description='What a stream says of its programme, as arguments for an encoder.')
    parser.add_argument('stream', nargs='?', type=Path)
    parser.add_argument('--json', action='store_true', help='print all of it as JSON')
    parser.add_argument('--bin', help='the oadec binary (default: as tools/oadec_bin.py finds it)')
    parser.add_argument('--self-check', action='store_true')
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    if args.stream is None:
        parser.error('a stream, or --self-check')
    if not args.stream.is_file():
        parser.error(f'not found: {args.stream}')
    out = report(args.bin or oadec_binary(), args.stream)
    if args.json:
        print(json.dumps(out, indent=1, ensure_ascii=False))
        return 0
    print('E-AC-3 (dee_ddpjoc_encoder):', ' '.join(out['arguments']['e_ac_3']) or 'nothing to tell')
    print('TrueHD (dee_dthd_encoder):  ', ' '.join(out['arguments']['truehd']) or 'nothing to tell')
    if out['decode']:
        print('the decode (oadec decode):  ', ' '.join(out['decode']))
    for what in out['left_to_the_encoder']:
        print('left to the encoder:', what)
    return 0


if __name__ == '__main__':
    sys.exit(main())
