#!/usr/bin/env python3
"""Scenes through Dolby's encoders and back: a matrix of round trips.

Everything else here that measures the object path takes a real stream and
asks whether two decoders agree on it. This asks what comes back of a
programme whose every number is known, and asks it across the things a
programme can be: its frame rate, its bed, how many objects it has, how they
move, what their metadata says, whether its elements sound together or one
after another, and what the encoder is told to make of it.

A row of the matrix is one value of each of those. Its scene gives every
object a tone of its own and every channel of the bed another, so that what
comes back is known by its sound and not by its number: an encoder's spatial
coding renumbers, turns bed channels into objects and spreads an object over
several of them. `oadec atmos-author` writes the master, Dolby's encoder
encodes it, and the stream is read four ways:

- what it says of itself (`info`, `oamd`, `emdf`, `verify`): the syntax the
  encoder chose and the metadata it kept;
- whether the two decoders make the same elements of it, sample by sample
  (`oadec decode --format damf --no-bed-conform` and Dolby's object decode);
- where the sound of each object and of each bed channel is, element by
  element, how much of it there is, and where the elements that carry it are
  while the object is where the scene put it;
- what the element that carries most of an object says of its place, gain,
  size, channel lock and zone, and when it gets to a new place.

The rows are a pairwise design: every value of every axis meets every value
of every other axis in some row, in far fewer rows than all the combinations.

    python tools/interop.py plan                       the rows
    python tools/interop.py run --work <dir> [--rows 0-9] [--path stream]
    python tools/interop.py report <dir>/rows-*.json --out matrix.json
    python tools/interop.py probe --work <dir> [--path truehd]
    python tools/interop.py programme --work <dir> [--rows 8,15,34,44]
    python tools/interop.py --self-check

Paths: `stream` is scene -> master -> E-AC-3 JOC (every row is meant to take
it); `conversion` puts Dolby's Conversion Tool between the master and the
encoder (master -> ADM BWF -> encoder); `own-adm` and `second` encode what
`oadec decode` makes of the first stream, as ADM BWF and as a master, and
hold that second stream to the scene; `truehd` encodes the master to TrueHD,
the value of the encoder axis choosing how many elements its spatial coding
leaves (12, 14 or 16).

`run` needs a Dolby Media Encoder installation (`DME_DEE`) and its licence;
the Reference Player (`DOLBY_PLAYER`) for the second decoder and the
Conversion Tool (`DOLBY_CONVERSION_TOOL`) for its path are used where they
are. A row whose encode is refused is a row of the result, with what the
encoder said. Dolby's tools are run as they stand and only what they write is
read.

`probe` is beside the matrix and narrower: one object alone over a silent
LFE, with one thing said of it (a channel lock, a zone, a size, a gain) at a
few places, through the same encoder. A row of the matrix says whether a
stream carries what a master said; a probe says what the encoder did with it.

`programme` asks what a second generation keeps of what the first stream
says of its programme as a whole: the dialogue normalisation, the downmix it
prefers and its levels, the DRC profiles, the warp mode. A master has a place
for the last of these only. The encoder of a first stream is told settings
that are not its defaults; the stream is decoded to a master
(`--warp-mode stream`) and the master encoded twice, once with the encoder
told nothing and once with it told what `tools/encoder_arguments.py` reads of
the first stream. What each of the three streams says is laid side by side.

Exit code: 0, 2 on a usage error, 3 when `run`, `probe` or `programme` cannot reach the encoder
(no `DME_DEE` folder or licence): it says so and runs nothing.
"""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import os
import random
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from caf import read_caf  # noqa: E402
from ground_truth import read_events  # noqa: E402
from oadec_bin import find as oadec_binary  # noqa: E402

DEE = Path(os.environ.get('DME_DEE', r'C:\Program Files\Dolby Media Encoder\resources\dee'))
LICENCE = Path(os.environ.get('APPDATA', '')) / 'Dolby Media Encoder' / 'license.lic'


def require_dee(*tools: str) -> None:
    '''Stops with exit code 3 when the encoder this run needs is not there: a
    run that cannot reach Dolby's tools says it could not run, instead of
    writing rows that read as results. `DME_DEE` names the folder of the
    tools, the licence is read from where Dolby Media Encoder keeps it.'''
    missing = [t for t in tools if not (DEE / t).is_file()]
    if not LICENCE.is_file():
        missing.append(f'the licence ({LICENCE})')
    if missing:
        print(f'cannot run: {", ".join(missing)} not found under {DEE}; '
              'set DME_DEE to the folder that holds the Dolby Media Encoder tools', file=sys.stderr)
        raise SystemExit(3)


PLAYER = Path(os.environ.get('DOLBY_PLAYER', r'C:\Program Files\Dolby\Dolby Reference Player'))
CONVERSION_TOOL = Path(os.environ.get(
    'DOLBY_CONVERSION_TOOL', r'C:\Program Files\Dolby\Dolby Atmos Conversion Tool\cmdline_atmos_conversion_tool.exe'))

RATE = 48000
TOGETHER = 9 * RATE  # the length of a scene whose elements sound together
SLOT = 36000  # what an element that sounds alone has for each place it rests at
TAIL = 12000  # silence after the last of them
LATE = 6000  # samples of a rest left out at its beginning: a move arrives late and takes a ramp
EARLY = 2000  # and at its end
WINDOW = 8192  # samples a level is taken over
HOP = 2048
REACH = 4096  # lags tried between the two decoders
TOLERANCE = 0.05  # room units
PATHS = ['stream', 'conversion', 'own-adm', 'second', 'truehd']

AXES = {
    'fps': ['23.976', '24', '25', '29.97', '30'],
    'bed': ['lfe', '2.0', '5.1', '7.1', '7.1.2', '5.0'],
    'objects': [1, 6, 15, 20],
    'motion': ['still', 'slow', 'jump', 'fast'],
    'metadata': ['plain', 'gain', 'size', 'snap', 'zones', 'warp'],
    'sounding': ['together', 'in turn'],
    'encoder': ['384', '448', '576', '640', '768', '1024', 'bluray'],
}
# The TrueHD encoder has no rate to be told; it is told how many elements its
# spatial coding is to leave, and a row's value of the encoder axis picks one
# of the three it takes, so that each of them meets every value of every
# other axis as the rates do.
TRUEHD_ELEMENTS = {'384': 12, '448': 14, '576': 16, '640': 12, '768': 14, '1024': 16, 'bluray': 16}
BEDS = {
    'lfe': ['LFE'],
    '2.0': ['L', 'R'],
    '5.1': ['L', 'R', 'C', 'LFE', 'Lss', 'Rss'],
    '7.1': ['L', 'R', 'C', 'LFE', 'Lss', 'Rss', 'Lrs', 'Rrs'],
    '7.1.2': ['L', 'R', 'C', 'LFE', 'Lss', 'Rss', 'Lrs', 'Rrs', 'Lts', 'Rts'],
    '5.0': ['L', 'R', 'C', 'Lss', 'Rss'],
}
# where a channel of the bed is, in the room of a master
SPEAKER = {
    'L': [-1.0, 1.0, 0.0], 'R': [1.0, 1.0, 0.0], 'C': [0.0, 1.0, 0.0],
    'Lss': [-1.0, 0.0, 0.0], 'Rss': [1.0, 0.0, 0.0], 'Lrs': [-1.0, -1.0, 0.0], 'Rrs': [1.0, -1.0, 0.0],
    'Lts': [-1.0, 0.0, 1.0], 'Rts': [1.0, 0.0, 1.0],
}
# The tones. All of them are below the lowest frequency the coupling these
# encoders write can begin at (3,47 kHz), where a core codes a tone as it is;
# the bed has the nine below 600 Hz and the LFE its own, and no two are nearer
# than 50 Hz, eight bins of the window a level is taken over.
BED_TONES = [150.0 + 50.0 * k for k in range(9)]
OBJECT_TONES = [round(600.0 * (3200.0 / 600.0) ** (k / 19), 1) for k in range(20)]
LFE_TONE = 60.0


# ---- the rows

def pairwise(axes, seed=1):
    """Rows in which every value of every axis meets every value of every
    other axis: each new row is the one of some candidates that covers the
    most pairs not covered yet."""
    names = list(axes)
    wanted = {((a, x), (b, y)) for a, b in itertools.combinations(names, 2) for x in axes[a] for y in axes[b]}
    rng = random.Random(seed)
    rows = []
    while wanted:
        best, gain = None, -1
        # start from a pair still wanted, so that every row covers one
        first = sorted(wanted, key=str)[0]
        for _ in range(60):
            row = {name: rng.choice(axes[name]) for name in names}
            row[first[0][0]], row[first[1][0]] = first[0][1], first[1][1]
            covers = sum(((a, row[a]), (b, row[b])) in wanted for a, b in itertools.combinations(names, 2))
            if covers > gain:
                best, gain = row, covers
        assert best is not None
        rows.append(best)
        wanted -= {((a, best[a]), (b, best[b])) for a, b in itertools.combinations(names, 2)}
    return rows


def uncovered(rows, axes):
    names = list(axes)
    have = {((a, r[a]), (b, r[b])) for r in rows for a, b in itertools.combinations(names, 2)}
    return [(a, x, b, y) for a, b in itertools.combinations(names, 2) for x in axes[a] for y in axes[b]
            if ((a, x), (b, y)) not in have]


def row_name(row):
    return hashlib.sha256(json.dumps(row, sort_keys=True).encode()).hexdigest()[:10]


# ---- the scene of a row

def on_grid(x, y, z):
    """A place a stream can say exactly: thirty-firsts across and along,
    fifteenths up."""
    return [round(round(x * 31) / 31, 7), round(round(y * 31) / 31, 7), round(round(z * 15) / 15, 7)]


def object_tone(i, count):
    """The tone of an object: the objects of a scene are spread over all the
    tones there are, so that few objects are far apart."""
    return OBJECT_TONES[0] if count == 1 else OBJECT_TONES[round(i * (len(OBJECT_TONES) - 1) / (count - 1))]


def bed_tone(channel, bed):
    return LFE_TONE if channel == 'LFE' else BED_TONES[[c for c in bed if c != 'LFE'].index(channel)]


def scene_of(row):
    rng = random.Random(json.dumps(row, sort_keys=True))
    bed = BEDS[row['bed']]
    count = row['objects']
    moves = [i < 3 and row['motion'] != 'still' for i in range(count)]
    in_turn = row['sounding'] == 'in turn'
    # when each element sounds: all of them through the scene, or one after another
    sounds = {}
    if in_turn:
        at = 0
        for c in bed:
            sounds[c] = (at, at + SLOT)
            at += SLOT
        for i in range(count):
            length = 3 * SLOT if moves[i] else SLOT
            sounds[i] = (at, at + length)
            at += length
        duration = at + TAIL
    else:
        duration = TOGETHER
        for key in [*bed, *range(count)]:
            sounds[key] = (0, duration)

    def tone(hz, dbfs, key):
        signal = {'kind': 'tone', 'hz': hz, 'dbfs': dbfs}
        if in_turn:
            signal['from'], signal['until'] = sounds[key]
        return signal

    def place():
        return on_grid(rng.uniform(-1, 1), rng.uniform(-1, 1), rng.choice([0.0, 0.0, 0.4, 1.0]))

    scene = {
        'sample_rate': RATE, 'duration_samples': duration, 'fps': row['fps'], 'bed': bed,
        'bed_signals': {c: tone(bed_tone(c, bed), -30.0, c) for c in bed},
        'objects': [],
    }
    if row['metadata'] == 'warp':
        scene['warp_mode'] = 'loro'
    for i in range(count):
        events = [{'sample': 0, 'pos': place()}]
        begins, ends = sounds[i]
        third = (ends - begins) // 3
        told = (begins + third, begins + 2 * third)  # when an object that moves is told to
        if moves[i] and row['motion'] == 'slow':
            events += [{'sample': s, 'pos': place()} for s in told]
        elif moves[i] and row['motion'] == 'jump':
            there = on_grid(-events[0]['pos'][0], -events[0]['pos'][1], 1.0 - events[0]['pos'][2])
            events += [{'sample': s, 'pos': p} for s, p in zip(told, (there, events[0]['pos']))]
        elif moves[i] and row['motion'] == 'fast':
            # between the two, a new place every 2048 samples, round a circle
            steps = range(told[0], told[1], 2048)
            for k, s in enumerate(steps):
                turn = 2 * np.pi * k / len(steps)
                events.append({'sample': s, 'pos': on_grid(0.8 * np.cos(turn), 0.8 * np.sin(turn), 0.0)})
            events.append({'sample': told[1], 'pos': place()})
        if i == 0:
            for e in events:
                if row['metadata'] == 'gain':
                    e['gain_db'] = -6
                elif row['metadata'] == 'size':
                    e['size'] = 0.4
                elif row['metadata'] == 'snap':
                    e['snap'] = True
                elif row['metadata'] == 'zones':
                    e['zones'] = 'no back'
        scene['objects'].append({'name': f'object {i}', 'signal': tone(object_tone(i, count), -20.0, i),
                                 'events': events})
    return scene


def rests_of(obj, duration):
    """Where an object stays at the place one of its states put it for as
    long as a slot, while it sounds: (from, until, the state)."""
    signal, events = obj['signal'], obj['events']
    begins, ends = signal.get('from', 0), signal.get('until', duration)
    rests = []
    for k, event in enumerate(events):
        lo = max(event['sample'], begins)
        hi = min(events[k + 1]['sample'] if k + 1 < len(events) else duration, ends)
        if hi - lo >= SLOT:
            rests.append((lo, hi, k))
    return rests


# ---- running the tools

def run(cmd, timeout=3600, env=None, cwd=None):
    began = time.monotonic()
    done = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, errors='replace', timeout=timeout,
                          stdin=subprocess.DEVNULL, env=env, cwd=cwd)
    return done.returncode, done.stdout, done.stderr, round(time.monotonic() - began, 2)


def what_it_said(text, n=3):
    """The lines of a tool's output that say what went wrong, or its last."""
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    wrong = [line for line in lines if 'ERROR' in line or 'error:' in line.lower()]
    return [re.sub(r'^\[[^\]]*\]\s*', '', line)[:240] for line in (wrong or lines)[-n:]]


def step(record, what, code, text, seconds):
    record['steps'].append({'step': what, 'exit': code, 'seconds': seconds,
                            **({'said': what_it_said(text)} if code != 0 else {})})
    return code == 0


def encode(source, out, encoder, told=()):
    """`told` is what the encoder is told beside its rate: `programme` says
    there how the stream is to be mixed down and how loud its dialogue is."""
    cmd = [DEE / 'dee_ddpjoc_encoder.exe', '-l', LICENCE, '--input-format', 'atmos_mezz', '-i', source, '-o', out,
           '--overwrite', '1', '--start', '0']
    cmd += ['--encoder', 'bluray', '--data-rate', '1536'] if encoder == 'bluray' else ['--data-rate', encoder]
    cmd += list(told)
    # an encoder keeps what it is working on in the folder it is run in: the row's
    code, stdout, stderr, seconds = run(cmd, cwd=out.parent)
    return (code if out.exists() or code else 1), stdout + stderr, seconds


def encode_truehd(source, out, encoder):
    code, stdout, stderr, seconds = run([DEE / 'dee_dthd_encoder.exe', '-l', LICENCE, '--presentation',
                                         f'atmos:number_of_elements={TRUEHD_ELEMENTS[encoder]}',
                                         '--input', source, '-o', out, '--overwrite', '1', '--start', '0'],
                                        cwd=out.parent)
    return (code if out.exists() or code else 1), stdout + stderr, seconds


def convert(master, outdir):
    """Dolby's Conversion Tool: a master as an ADM BWF."""
    shutil.rmtree(outdir, ignore_errors=True)
    outdir.mkdir(parents=True)
    code, stdout, stderr, seconds = run([CONVERSION_TOOL, '-i', master, '-o', outdir, '-f', 'wav'])
    made = sorted(outdir.glob('*.wav'))
    return (code if made or code else 1), stdout + stderr, seconds, (made[0] if made else None)


def dolby_objects(stream, out):
    """The elements Dolby's decoder makes of a stream: samples x sixteen, or
    None where the decoder is not or will not. It writes sixteen channels
    whatever the programme has, the bed and then the objects, and leaves the
    ones past the last element silent."""
    launch = PLAYER / 'gst-launch-1.0.exe'
    if not launch.exists():
        return None
    env = dict(os.environ)
    env['GST_PLUGIN_PATH'] = str(PLAYER / 'gst-plugins')
    env['PATH'] = str(PLAYER) + os.pathsep + env.get('PATH', '')
    out.unlink(missing_ok=True)
    code, _, _, _ = run(
        [launch, 'filesrc', f'location={stream.as_posix()}', '!', 'dlbac3parse', 'enable-metadata=true', '!',
         'dlbac3dec', 'out-ch-config=21', 'drc-suppress=true', 'drc-mode=custom-0', 'drc-cut=0', 'drc-boost=0',
         'drop-delay=true', '!', 'audio/x-raw(meta:DlbObjectAudioMeta),format=F32LE', '!', 'identity', '!',
         'filesink', f'location={out.as_posix()}'], env=env)
    if code != 0 or not out.exists():
        return None
    raw = np.fromfile(out, dtype='<f4')
    out.unlink(missing_ok=True)
    if len(raw) == 0 or len(raw) % 16:
        return None
    return raw.reshape(-1, 16).astype(np.float64)


# ---- what a stream says of itself

def facts(oadec, stream):
    def said(command):
        code, stdout, _, _ = run([oadec, command, '--json', stream])
        try:
            return code, json.loads(stdout)
        except ValueError:
            return code, {}

    info_exit, info = said('info')
    oamd_exit, oamd = said('oamd')
    emdf_exit, emdf = said('emdf')
    verify_exit, _, _, _ = run([oadec, 'verify', stream])
    joc = info.get('joc') or {}
    programmes = info.get('program') or [{}]
    return {
        'bytes': stream.stat().st_size,
        'sha256': hashlib.sha256(stream.read_bytes()).hexdigest(),
        'exit': {'info': info_exit, 'oamd': oamd_exit, 'emdf': emdf_exit, 'verify': verify_exit},
        'syntax': info.get('syntax'), 'frames': info.get('frames'), 'bit_rate': info.get('bit_rate'),
        'channels': info.get('channels'), 'substreams': [p.get('substream') for p in programmes],
        'core_syntax': sorted({str(p.get('syntax')) for p in programmes}),
        'dialnorm': info.get('dialnorm'), 'downmix': info.get('downmix'), 'coverage': info.get('coverage'),
        'complexity_index': (info.get('joc_extension') or {}).get('complexity_index'),
        'joc': {key: joc.get(key) for key in (
            'downmix_configs', 'bands', 'objects_per_payload', 'interpolation_branches', 'offset_ts',
            'two_data_points', 'sparse_objects', 'steep_objects', 'coarse_quantized_objects',
            'fine_quantized_objects', 'clipgain_x1000', 'seq_count_gaps')} if joc else None,
        'oamd': {key: oamd.get(key) for key in (
            'program', 'warp_modes', 'zone_constraints', 'snapped', 'sized_updates', 'object_gains_db',
            'ramp_durations', 'global_trim_modes', 'trim_configs', 'payloads_with_trim', 'render_status',
            'distances', 'screen_referenced', 'sample_offsets', 'block_offset_factors', 'object_counts',
            'differential_positions', 'inactive_updates')} if oamd else None,
        'loudness': emdf.get('loudness'), 'programme_information': emdf.get('programme_information'),
    }


# ---- holding a decode to its scene

def decoded_layout(base):
    """The bed channels and the object ids of a master, in the order of its
    audio."""
    text = Path(str(base) + '.atmos').read_text(encoding='utf-8', errors='replace')
    bed = re.findall(r'-\s*channel:\s*(\S+)\s*\n\s*ID:\s*(\d+)', text)
    tail = text.split('objects:')[-1] if 'objects:' in text else ''
    objects = [int(v) for v in re.findall(r'-\s*ID:\s*(\d+)', tail)]
    return [(name, int(i)) for name, i in bed], objects


def tone_in(audio, hz, lo, hi):
    """The amplitude of a tone in every channel, as a complex number, window
    by window between two samples: windows x channels."""
    lo, hi = max(lo, 0), min(hi, len(audio))
    count = (hi - lo - WINDOW) // HOP + 1
    if count < 1:
        return np.zeros((0, audio.shape[1]), dtype=complex)
    length = (count - 1) * HOP + WINDOW
    turned = audio[lo:lo + length] * np.exp(-2j * np.pi * hz * np.arange(lo, lo + length) / RATE)[:, None]
    window = np.hanning(WINDOW)
    window /= window.sum()
    return np.stack([2 * (turned[k * HOP:k * HOP + WINDOW] * window[:, None]).sum(axis=0) for k in range(count)])


def state_at(events, element, sample):
    said = [e for e in events.get(element, []) if e.get('samplePos', 0) <= sample]
    return said[-1] if said else None


def furthest(a, b):
    return round(max(abs(x - y) for x, y in zip(a, b)), 4)


def db(power):
    return round(float(10 * np.log10(power)), 2)


def hold(scene, base):
    """What came back of every object and every bed channel of a scene, and
    the audio it was read from."""
    audio, _ = read_caf(str(base) + '.atmos.audio')
    said = read_events(Path(str(base) + '.atmos.metadata'))
    bed, objects = decoded_layout(base)
    elements = [('bed', name, i) for name, i in bed] + [('object', None, i) for i in objects]
    if audio.shape[1] != len(elements):
        raise ValueError(f'{audio.shape[1]} channels of audio for {len(elements)} elements')
    duration = scene['duration_samples']

    def place(element, sample):
        kind, name, ident = element
        if kind == 'bed':
            return SPEAKER.get(name)
        pos = (state_at(said, ident, sample) or {}).get('pos')
        return pos if pos and len(pos) == 3 else None

    def carried(hz, lo, hi):
        """Of one tone between two samples: how much of it there is over all
        the elements and in their sum, the element that has most of it and
        its share, and the middle of where the elements that have it are,
        each weighed by the power it has."""
        amplitudes = tone_in(audio, hz, lo + LATE, hi - EARLY)
        if len(amplitudes) == 0:
            return None
        power = (np.abs(amplitudes) ** 2).mean(axis=0)
        total = float(power.sum())
        if total < 1e-7:  # 70 dB below full scale
            return {'heard': False}
        main = int(power.argmax())
        placed = [(float(power[ch]), place(e, (lo + hi) // 2)) for ch, e in enumerate(elements)]
        placed = [(p, at) for p, at in placed if at is not None]
        weight = sum(p for p, _ in placed)
        kind, name, _ = elements[main]
        return {
            'heard': True, 'level_dbfs': db(total),
            'level_of_the_sum_dbfs': db((np.abs(amplitudes.sum(axis=1)) ** 2).mean()),
            'carrier': main, 'carrier_is': 'object' if kind == 'object' else f'bed {name}',
            'share_of_the_carrier': round(float(power[main]) / total, 4),
            'elements_with_a_hundredth_of_it': int((power > 0.01 * total).sum()),
            'centre': [round(sum(p * at[k] for p, at in placed) / weight, 4) for k in range(3)] if weight else None,
        }

    def what_the_carrier_says(got, lo, hi, wanted, told):
        kind, _, ident = elements[got['carrier']]
        if kind != 'object':
            return {}
        state = state_at(said, ident, (lo + hi) // 2) or {}
        out = {'carrier_says': {key: state.get(key) for key in ('pos', 'gain', 'size', 'snap', 'zones')}}
        if state.get('pos') and len(state['pos']) == 3:
            out['carrier_error'] = furthest(state['pos'], wanted)
        if told is not None:
            # when the carrier first says the place it was told to go to
            there = [e['samplePos'] for e in said.get(ident, [])
                     if e.get('pos') and len(e['pos']) == 3 and furthest(e['pos'], wanted) <= TOLERANCE
                     and e.get('samplePos', 0) >= told - 8192]
            out['carrier_gets_there_after_samples'] = there[0] - told if there else None
        return out

    rows = []
    for obj in scene['objects']:
        signal = obj['signal']
        spans = []
        for lo, hi, k in rests_of(obj, duration):
            event = obj['events'][k]
            span = {'from_sample': lo, 'state': k, 'authored_pos': event['pos']}
            got = carried(signal['hz'], lo, hi)
            if got is None or not got['heard']:
                spans.append({**span, 'heard': False})
                continue
            span.update(got)
            span['level_minus_authored_db'] = round(got['level_dbfs'] - signal['dbfs'], 2)
            span['sum_minus_authored_db'] = round(got['level_of_the_sum_dbfs'] - signal['dbfs'], 2)
            if got['centre'] is not None:
                span['centre_error'] = furthest(got['centre'], event['pos'])
            span.update(what_the_carrier_says(got, lo, hi, event['pos'], event['sample'] if k else None))
            spans.append(span)
        first = obj['events'][0]
        rows.append({'object': obj['name'], 'hz': signal['hz'], 'states': len(obj['events']),
                     'authored': {key: first[key] for key in ('gain_db', 'size', 'snap', 'zones') if key in first},
                     'spans': spans})
    beds = []
    for name, signal in scene.get('bed_signals', {}).items():
        lo, hi = signal.get('from', 0), signal.get('until', duration)
        row = {'channel': name, 'hz': signal['hz']}
        got = carried(signal['hz'], lo, hi)
        if got is None or not got['heard']:
            beds.append({**row, 'heard': False})
            continue
        row.update(got)
        row['level_minus_authored_db'] = round(got['level_dbfs'] - signal['dbfs'], 2)
        row['sum_minus_authored_db'] = round(got['level_of_the_sum_dbfs'] - signal['dbfs'], 2)
        row['kept_as_the_bed_channel'] = got['carrier_is'] == f'bed {name}'
        if name in SPEAKER:
            if got['centre'] is not None:
                row['centre_error'] = furthest(got['centre'], SPEAKER[name])
            row.update(what_the_carrier_says(got, lo, hi, SPEAKER[name], None))
        beds.append(row)
    return {'decoded_bed': [name for name, _ in bed], 'decoded_objects': len(objects), 'objects': rows,
            'bed': beds}, audio


def lag_of(ours, theirs, reach=REACH, span=2 * RATE):
    """How many samples later `theirs` says what `ours` says, from where the
    programme begins: a tone lines up with itself every period, its beginning
    only once."""
    n = min(len(ours), len(theirs), span)
    size = 1 << (2 * n - 1).bit_length()
    total = np.zeros(size)
    for ch in range(min(ours.shape[1], theirs.shape[1])):
        total += np.fft.irfft(np.fft.rfft(theirs[:n, ch], size) * np.conj(np.fft.rfft(ours[:n, ch], size)), size)
    lags = np.concatenate([np.arange(0, reach + 1), np.arange(-reach, 0)])
    return int(lags[np.concatenate([total[:reach + 1], total[-reach:]]).argmax()])


def agree(ours, theirs):
    """How near the elements one decoder makes are to the other's, sample by
    sample: for each element that sounds, the other's power over the power of
    the difference. The other may have channels past the last element, which
    are to be silent."""
    if ours.shape[1] > theirs.shape[1]:
        return {'compared': False, 'why': f'{ours.shape[1]} elements against {theirs.shape[1]}'}
    spare = theirs[:, ours.shape[1]:]
    theirs = theirs[:, :ours.shape[1]]
    lag = lag_of(ours, theirs)
    a, b = (ours, theirs[lag:]) if lag >= 0 else (ours[-lag:], theirs)
    n = min(len(a), len(b))
    ratios = []
    for ch in range(ours.shape[1]):
        power = float((b[:n, ch] ** 2).mean())
        if power < 1e-9:
            ratios.append(None)
            continue
        wrong = float(((a[:n, ch] - b[:n, ch]) ** 2).mean())
        ratios.append(round(float(10 * np.log10(power / wrong)), 2) if wrong > 0 else 200.0)
    sounding = sorted(r for r in ratios if r is not None)
    return {'compared': True, 'lag_samples': lag, 'samples': n, 'elements_that_sound': len(sounding),
            'channels_past_the_elements': spare.shape[1],
            'and_they_are_silent': bool(spare.shape[1] == 0 or float((spare ** 2).mean()) < 1e-12),
            'worst_db': sounding[0] if sounding else None,
            'median_db': sounding[len(sounding) // 2] if sounding else None, 'each_db': ratios}


# ---- one row along one path

def one_row(row, work, oadec, path, keep=False, warp=False):
    name = row_name(row)
    return one_scene(scene_of(row), work / name, {'row': row, 'name': name}, oadec, path, row['encoder'], keep,
                     warp)


def one_scene(scene, here, record, oadec, path, encoder, keep=False, warp=False):
    """A scene along a path: the master, the stream, what the stream says,
    what both decoders make of it and what came back of the scene."""
    here.mkdir(parents=True, exist_ok=True)
    (here / 'scene.json').write_text(json.dumps(scene, indent=1), encoding='utf-8')
    record.update({'path': path, 'steps': [],
                   'authored': {'bed': scene['bed'], 'objects': len(scene['objects']),
                                'elements': len(scene['bed']) + len(scene['objects']),
                                'samples': scene['duration_samples']}})
    master = here / 'master.atmos'
    code, _, stderr, seconds = run([oadec, 'atmos-author', here / 'scene.json', '-o', here / 'master'])
    if not step(record, 'author', code, stderr, seconds):
        return record
    first = here / 'stream.ec3'

    def first_stream():
        if first.exists():
            return True
        return step(record, 'encode the master', *encode(master, first, encoder))

    if path == 'stream':
        stream = first
        first.unlink(missing_ok=True)
        if not first_stream():
            return record
    elif path == 'conversion':
        code, text, seconds, adm = convert(master, here / 'conversion')
        if not step(record, 'convert the master to ADM', code, text, seconds):
            return record
        stream = here / 'conversion.ec3'
        stream.unlink(missing_ok=True)
        if not step(record, 'encode the ADM', *encode(adm, stream, encoder)):
            return record
    elif path in ('own-adm', 'second'):
        if not first_stream():
            return record
        record['first_stream_sha256'] = hashlib.sha256(first.read_bytes()).hexdigest()
        if path == 'own-adm':
            source = here / 'own-adm.wav'
            # the encoders take an ADM file only with the creator string of Dolby's own tools in it
            cmd = [oadec, 'decode', '--format', 'adm', '--dolby-origin-tag', '-o', here / 'own-adm', first]
        else:
            source = here / 'first.atmos'
            cmd = [oadec, 'decode', '--format', 'damf', '-o', here / 'first', first]
        # a decode names no warp mode unless it is asked to name the stream's, and an encoder
        # reads none as warping
        record['the_decode_names_the_warp_mode_of_the_stream'] = warp
        if warp:
            cmd[2:2] = ['--warp-mode', 'stream']
        code, _, stderr, seconds = run(cmd)
        record['steps'].append({'step': 'decode the first stream', 'exit': code, 'seconds': seconds,
                                **({'said': what_it_said(stderr)} if code else {})})
        if not source.exists():
            return record
        stream = here / f'{path}.ec3'
        stream.unlink(missing_ok=True)
        if not step(record, 'encode the decode', *encode(source, stream, encoder)):
            return record
    else:
        stream = here / 'truehd.mlp'
        stream.unlink(missing_ok=True)
        record['truehd_elements'] = TRUEHD_ELEMENTS[encoder]
        if not step(record, 'encode the master', *encode_truehd(master, stream, encoder)):
            return record
    record['stream'] = facts(oadec, stream)
    decoded = here / f'decoded-{path}'
    code, _, stderr, seconds = run([oadec, 'decode', '--format', 'damf', '--no-bed-conform', '-o', decoded, stream])
    record['steps'].append({'step': 'decode', 'exit': code, 'seconds': seconds,
                            **({'said': what_it_said(stderr)} if code else {})})
    if Path(str(decoded) + '.atmos.audio').exists():
        record['held'], audio = hold(scene, decoded)
        if path != 'truehd':
            theirs = dolby_objects(stream, here / 'dolby.f32')
            record['decoders'] = ({'compared': False, 'why': 'no second decode'} if theirs is None
                                  else agree(audio, theirs))
    if not keep:
        for big in (here / 'master.atmos.audio', here / 'first.atmos.audio', here / 'own-adm.wav',
                    Path(str(decoded) + '.atmos.audio')):
            big.unlink(missing_ok=True)
        shutil.rmtree(here / 'conversion', ignore_errors=True)
    return record


# ---- the probes: one object alone, and one thing said of it

PLACES = {
    'inside the room': on_grid(0.35, 0.03, 0.0),
    'near the left rear corner': on_grid(-0.9, -0.94, 0.0),
    'near the right wall': on_grid(0.61, -0.1, 0.0),
    'at the back': on_grid(-0.26, -0.87, 0.0),
    'front right': on_grid(0.58, 0.81, 0.0),
    'overhead': on_grid(0.0, 0.48, 1.0),
}
ZONES = ['no back', 'no sides', 'center back', 'screen only', 'surround only']


def probes():
    """What is said of the one object of each probe: its name and its state."""
    out = [(f'plain, {where}', {'pos': pos}) for where, pos in PLACES.items()]
    out += [(f'locked to a channel, {where}', {'pos': pos, 'snap': True}) for where, pos in PLACES.items()]
    out += [(f'kept to "{zone}", {where}', {'pos': PLACES[where], 'zones': zone})
            for zone in ZONES for where in ('inside the room', 'at the back', 'front right')]
    out += [(f'size {size}, inside the room', {'pos': PLACES['inside the room'], 'size': size})
            for size in (0.1, 0.4, 1.0)]
    out += [(f'gain {gain} dB, inside the room', {'pos': PLACES['inside the room'], 'gain_db': gain})
            for gain in (-6, -20, 6)]
    return out


def probe_scene(state):
    return {'sample_rate': RATE, 'duration_samples': 2 * RATE, 'fps': '24', 'bed': ['LFE'], 'bed_signals': {},
            'objects': [{'name': 'the object', 'signal': {'kind': 'tone', 'hz': OBJECT_TONES[0], 'dbfs': -20.0},
                         'events': [{'sample': 0, **state}]}]}


def one_probe(name, state, work, oadec, path, keep=False):
    tag = hashlib.sha256(name.encode()).hexdigest()[:10]
    record = one_scene(probe_scene(state), work / tag, {'probe': name, 'said': state, 'name': tag}, oadec, path,
                       '768', keep)
    if 'held' in record:
        # what came back of the one object, and every element that has a hundredth of it
        span = record['held']['objects'][0]['spans'][0]
        record['came_back'] = {key: span.get(key) for key in (
            'heard', 'level_minus_authored_db', 'sum_minus_authored_db', 'share_of_the_carrier',
            'elements_with_a_hundredth_of_it', 'carrier_says', 'carrier_error', 'centre', 'centre_error')}
        oamd = (record.get('stream') or {}).get('oamd') or {}
        record['the_stream_says'] = {
            'a gain': bool(oamd.get('object_gains_db')), 'a size': bool(oamd.get('sized_updates')),
            'a lock': bool(oamd.get('snapped')),
            'a zone': any(key != '0' for key in (oamd.get('zone_constraints') or {})),
        }
    return record


# ---- the matrix as numbers

def spread(values):
    values = sorted(v for v in values if v is not None)
    if not values:
        return None
    return {'least': values[0], 'median': values[len(values) // 2], 'most': values[-1], 'of': len(values)}


def tally(values):
    out = {}
    for value in values:
        key = json.dumps(value, sort_keys=True) if isinstance(value, (list, dict)) else str(value)
        out[key] = out.get(key, 0) + 1
    return dict(sorted(out.items()))


def held_figures(rows):
    """What the rows that were held to their scene say, counted."""
    spans = [s for r in rows for o in r['held']['objects'] for s in o['spans']]
    heard = [s for s in spans if s.get('heard')]
    beds = [b for r in rows for b in r['held']['bed']]
    beds_heard = [b for b in beds if b.get('heard')]
    placed = [b for b in beds_heard if b['channel'] in SPEAKER]
    # the objects a scene says nothing of but where they are
    plain = [s for r in rows for o in r['held']['objects'] if not o['authored'] for s in o['spans']
             if s.get('heard')]
    return {
        'rows': len(rows),
        'objects_with_a_place_and_nothing_else': {
            'rests_heard': len(plain),
            'carried_by_one_element': sum(s.get('share_of_the_carrier', 0) >= 0.99 for s in plain),
            'carrier_at_the_authored_place': sum(
                s.get('carrier_error') is not None and s['carrier_error'] <= TOLERANCE for s in plain),
            'level_minus_authored_db': spread(s.get('level_minus_authored_db') for s in plain),
            'moves': sum('carrier_gets_there_after_samples' in s for s in plain),
            'moves_the_carrier_makes': sum(s.get('carrier_gets_there_after_samples') is not None for s in plain),
            'carrier_gets_there_after_samples': spread(s.get('carrier_gets_there_after_samples') for s in plain),
        },
        'objects_back': spread(r['held']['decoded_objects'] for r in rows),
        'rests_of_objects': len(spans), 'heard': len(heard),
        'level_minus_authored_db': spread(s.get('level_minus_authored_db') for s in heard),
        'sum_minus_authored_db': spread(s.get('sum_minus_authored_db') for s in heard),
        'share_of_the_carrier': spread(s.get('share_of_the_carrier') for s in heard),
        'carried_by_one_element': sum(s.get('share_of_the_carrier', 0) >= 0.99 for s in heard),
        'carrier_error': spread(s.get('carrier_error') for s in heard),
        'carrier_at_the_authored_place': sum(
            s.get('carrier_error') is not None and s['carrier_error'] <= TOLERANCE for s in heard),
        'centre_error': spread(s.get('centre_error') for s in heard),
        'centre_at_the_authored_place': sum(
            s.get('centre_error') is not None and s['centre_error'] <= TOLERANCE for s in heard),
        'moves': sum('carrier_gets_there_after_samples' in s for s in heard),
        'moves_the_carrier_makes': sum(s.get('carrier_gets_there_after_samples') is not None for s in heard),
        'carrier_gets_there_after_samples': spread(s.get('carrier_gets_there_after_samples') for s in heard),
        'bed_channels': len(beds), 'bed_channels_heard': len(beds_heard),
        'bed_channels_kept_as_bed': sum(b.get('kept_as_the_bed_channel', False) for b in beds_heard),
        'bed_level_minus_authored_db': spread(b.get('level_minus_authored_db') for b in beds_heard),
        'bed_share_of_the_carrier': spread(b.get('share_of_the_carrier') for b in beds_heard),
        'bed_channels_with_a_place': len(placed),
        'bed_carrier_at_the_speaker': sum(
            bool(b.get('kept_as_the_bed_channel')) or (b.get('carrier_error') is not None
                                                        and b['carrier_error'] <= TOLERANCE) for b in placed),
    }


def metadata_ledger(rows):
    """For each thing a scene can say of an object: what the streams of the
    rows that said it carry, and what the element that has most of that
    object says."""
    out = {}
    for value in AXES['metadata']:
        mine = [r for r in rows if r['row']['metadata'] == value and r.get('stream')]
        firsts = [o['spans'][0] for r in mine if 'held' in r for o in r['held']['objects'][:1]
                  if o['spans'] and o['spans'][0].get('heard')]
        says = [s.get('carrier_says') or {} for s in firsts]
        oamd = [(r['stream'].get('oamd') or {}) for r in mine]
        out[value] = {
            'rows': len(mine),
            'streams_with_a_gain': sum(bool(o.get('object_gains_db')) for o in oamd),
            'streams_with_a_size': sum(bool(o.get('sized_updates')) for o in oamd),
            'streams_with_a_lock': sum(bool(o.get('snapped')) for o in oamd),
            'streams_with_a_zone': sum(any(k != '0' for k in (o.get('zone_constraints') or {})) for o in oamd),
            'warp_modes': tally(k for o in oamd for k in (o.get('warp_modes') or {})),
            'carrier_gain': tally(s.get('gain') for s in says),
            'carrier_size': tally(s.get('size') for s in says),
            'carrier_snap': tally(s.get('snap') for s in says),
            'carrier_zones': tally(s.get('zones') for s in says),
            'first_object_level_minus_authored_db': spread(s.get('level_minus_authored_db') for s in firsts),
        }
    return out


def path_of(record):
    """The path of a record as the summary names it: a second generation
    whose decode named the warp mode of the stream is beside the one that
    named none."""
    named = record.get('the_decode_names_the_warp_mode_of_the_stream')
    return record['path'] + (', the warp mode named' if named else '')


def elements_back(record):
    return len(record['held']['decoded_bed']) + record['held']['decoded_objects']


def warp_of(record):
    return sorted(((record.get('stream') or {}).get('oamd') or {}).get('warp_modes') or {})


def lowest_tone_of_the_worst(record):
    """The lowest tone that has the element furthest from the other decoder's
    for its carrier, or None when no tone has."""
    each = record['decoders']['each_db']
    worst = min((value, ch) for ch, value in enumerate(each) if value is not None)[1]
    held = record['held']
    tones = [o['hz'] for o in held['objects'] for s in o['spans'] if s.get('heard') and s['carrier'] == worst]
    tones += [b['hz'] for b in held['bed'] if b.get('heard') and b['carrier'] == worst]
    return min(tones) if tones else None


def summarise(records):
    """What the rows say, counted, path by path, and what the paths have
    between them."""
    out = {}
    for path in sorted({path_of(r) for r in records}, key=lambda p: (PATHS.index(p.split(',')[0]), p)):
        rows = [r for r in records if path_of(r) == path]
        made = [r for r in rows if r.get('stream')]
        held = [r for r in rows if 'held' in r]
        compared = [r['decoders'] for r in rows if r.get('decoders', {}).get('compared')]
        joc = [(r['stream'].get('joc') or {}) for r in made]
        oamd = [(r['stream'].get('oamd') or {}) for r in made]
        by_encoder = {}
        for value in AXES['encoder']:
            mine = [r for r in made if r['row']['encoder'] == value]
            if mine and not path.startswith('truehd'):
                by_encoder[value] = {
                    'rows': len(mine),
                    'bit_rates': tally(r['stream'].get('bit_rate') for r in mine),
                    'core': tally(r['stream'].get('core_syntax') for r in mine),
                    'substreams': tally(r['stream'].get('substreams') for r in mine),
                    'downmix_configs': tally(v for r in mine for v in (r['stream'].get('joc') or {}).get(
                        'downmix_configs') or []),
                    'bands': tally(v for r in mine for v in (r['stream'].get('joc') or {}).get('bands') or []),
                    'objects': tally(v for r in mine for v in (r['stream'].get('joc') or {}).get(
                        'objects_per_payload') or []),
                    'complexity_index': tally(r['stream'].get('complexity_index') for r in mine),
                    'coverage': tally(v for r in mine for v in r['stream'].get('coverage') or []),
                }
        by_sounding = {}
        for value in AXES['sounding']:
            mine = [r for r in held if r['row']['sounding'] == value]
            by_sounding[value] = {
                'every row': held_figures(mine),
                'rows with no more elements than came back': held_figures(
                    [r for r in mine if r['authored']['elements'] <= elements_back(r)]),
                'rows with more elements than came back': held_figures(
                    [r for r in mine if r['authored']['elements'] > elements_back(r)]),
            }
        out[path] = {
            'rows': len(rows), 'streams': len(made), 'held_to_their_scene': len(held),
            'refused': [{'row': r['row'], 'step': r['steps'][-1]['step'], 'said': r['steps'][-1].get('said')}
                        for r in rows if not r.get('stream')],
            'exit_codes': {command: tally(r['stream']['exit'][command] for r in made)
                           for command in ('info', 'oamd', 'emdf', 'verify')},
            'decode_exit_codes': tally(s['exit'] for r in rows for s in r['steps'] if s['step'] == 'decode'),
            'by_encoder': by_encoder,
            'objects_back_by_elements_asked': {
                str(n): tally(r['held']['decoded_objects'] for r in held if r.get('truehd_elements') == n)
                for n in sorted({r['truehd_elements'] for r in held if 'truehd_elements' in r})},
            'interpolation_branches': tally(k for j in joc for k in (j.get('interpolation_branches') or {})),
            'streams_with_two_data_points': sum(bool(j.get('two_data_points')) for j in joc),
            'streams_with_steep_objects': sum(bool(j.get('steep_objects')) for j in joc),
            'streams_with_sparse_objects': sum(bool(j.get('sparse_objects')) for j in joc),
            'clip_gains': tally(v for j in joc for v in (j.get('clipgain_x1000') or [])),
            'programmes': tally((o.get('program') or {}).get('beds') for o in oamd),
            'streams_with_a_trim': sum(bool(o.get('payloads_with_trim')) for o in oamd),
            'warp_modes': tally(k for o in oamd for k in (o.get('warp_modes') or {})),
            'ramps': tally(k for o in oamd for k in (o.get('ramp_durations') or {})),
            'streams_with_a_ramp_that_is_not_1536': sum(
                any(k != '1536' for k in (o.get('ramp_durations') or {})) for o in oamd),
            'sample_offsets': tally(json.dumps(sorted(o.get('sample_offsets') or {}, key=int)) for o in oamd),
            'block_offset_factors': tally(json.dumps(sorted(o.get('block_offset_factors') or {}, key=int))
                                          for o in oamd),
            'dialnorm': tally(v for r in made for v in (r['stream'].get('dialnorm') or [])),
            'decoders': {
                'rows_compared': len(compared),
                'rows_not_compared': tally(r['decoders'].get('why') for r in rows
                                           if 'decoders' in r and not r['decoders'].get('compared')),
                'lags': tally(d['lag_samples'] for d in compared),
                'rows_with_a_sound_past_the_elements': sum(not d['and_they_are_silent'] for d in compared),
                'worst_db': spread(d['worst_db'] for d in compared),
                'median_db': spread(d['median_db'] for d in compared),
                # the lowest subband of the object decoder is 375 Hz wide
                'rows_whose_worst_element_carries_a_tone_below_375_hz': sum(
                    (lowest_tone_of_the_worst(r) or 1e9) < 375.0
                    for r in rows if r.get('decoders', {}).get('compared') and 'held' in r),
            },
            'held': by_sounding,
            'metadata': metadata_ledger(rows),
        }
    out['between_the_paths'] = between(records)
    return out


def generations(first, later):
    """What a second stream keeps of what the first kept, over the rows both
    have, rest by rest."""
    names = [n for n in later if n in first and 'held' in later[n] and 'held' in first[n]]
    out = {'rows': len(names)}

    def rests(record):
        return [s for o in record['held']['objects'] for s in o['spans']]

    def at_the_place(names):
        return sum(s.get('carrier_error') is not None and s['carrier_error'] <= TOLERANCE
                   for n in names for s in rests(n) if s.get('heard'))

    def dialnorm(record):
        values = record['stream'].get('dialnorm') or []
        return values[0] if len(values) == 1 else None

    def profiles(record):
        said = record['stream'].get('programme_information') or {}
        return sorted(said.get('drc_line') or {}), sorted(said.get('drc_rf') or {})

    # what an encoder measures or is told and a master does not carry
    norms = [(dialnorm(first[n]), dialnorm(later[n])) for n in names]
    norms = [(a, b) for a, b in norms if a is not None and b is not None]
    out['what_the_programme_says'] = {
        'rows_with_the_same_dialogue_normalisation': sum(a == b for a, b in norms),
        'rows_with_another': sum(a != b for a, b in norms),
        'its_field_second_minus_first': spread(b - a for a, b in norms if a != b),
        'rows_with_the_same_drc_profiles': sum(profiles(first[n]) == profiles(later[n]) for n in names),
    }
    for value in AXES['sounding']:
        mine = [n for n in names if first[n]['row']['sounding'] == value]
        pairs = [(a, b) for n in mine for a, b in zip(rests(first[n]), rests(later[n]))
                 if a.get('heard') and b.get('heard')]
        beds = [(a, b) for n in mine for a, b in zip(first[n]['held']['bed'], later[n]['held']['bed'])
                if a.get('heard') and b.get('heard')]
        out[value] = {
            'rows': len(mine),
            'rests_heard_in_both': len(pairs),
            'rests_heard': {'first': sum(s.get('heard', False) for n in mine for s in rests(first[n])),
                            'second': sum(s.get('heard', False) for n in mine for s in rests(later[n]))},
            'carrier_at_the_authored_place': {'first': at_the_place([first[n] for n in mine]),
                                              'second': at_the_place([later[n] for n in mine])},
            'level_second_minus_first_db': spread(
                round(b['level_minus_authored_db'] - a['level_minus_authored_db'], 2) for a, b in pairs),
            'share_of_the_carrier_second_minus_first': spread(
                round(b['share_of_the_carrier'] - a['share_of_the_carrier'], 4) for a, b in pairs),
            'bed_channels_heard_in_both': len(beds),
            'bed_level_second_minus_first_db': spread(
                round(b['level_minus_authored_db'] - a['level_minus_authored_db'], 2) for a, b in beds),
        }
    return out


def between(records):
    """The paths held to each other, row by row."""
    by = {}
    for r in records:
        by.setdefault(path_of(r), {})[r['name']] = r
    stream = by.get('stream', {})

    def sha(record):
        return (record.get('stream') or {}).get('sha256')

    def same(a, b):
        both = [n for n in a if n in b and sha(a[n]) and sha(b[n])]
        return {'rows': len(both), 'the_same_bytes': sum(sha(a[n]) == sha(b[n]) for n in both)}

    out = {}
    again = [(n, r) for path in ('own-adm', 'second', 'own-adm, the warp mode named', 'second, the warp mode named')
             for n, r in by.get(path, {}).items() if n in stream and 'first_stream_sha256' in r]
    if again:
        out['the_same_master_encoded_again'] = {
            'rows': len(again), 'the_same_bytes': sum(r['first_stream_sha256'] == sha(stream[n]) for n, r in again)}
    if 'conversion' in by:
        out['through_the_conversion_tool_and_direct'] = same(by['conversion'], stream)
    for path, key in (('second', 'a_second_generation_through_a_master'),
                      ('own-adm', 'a_second_generation_through_adm')):
        if path in by:
            out[key] = generations(stream, by[path])
    for suffix, key in (('', 'the_two_second_generations'),
                        (', the warp mode named', 'the_two_second_generations_with_the_warp_mode_named')):
        a, b = by.get('second' + suffix), by.get('own-adm' + suffix)
        if a and b:
            lo_ro = [n for n in a if n in stream and warp_of(stream[n]) == ['3']]
            out[key] = {
                **same(a, b),
                'rows_whose_first_stream_says_lo_ro': len(lo_ro),
                'and_whose_second_says_it': {'through_a_master': sum(warp_of(a[n]) == ['3'] for n in lo_ro),
                                             'through_adm': sum(warp_of(b[n]) == ['3'] for n in lo_ro if n in b)},
                'warp_modes_of_the_second': {'through_a_master': tally(k for r in a.values() for k in warp_of(r)),
                                             'through_adm': tally(k for r in b.values() for k in warp_of(r))},
            }
    return out


# ---- a known answer

def self_check():
    rows = pairwise(AXES)
    turn = next(r for r in rows if r['sounding'] == 'in turn' and r['motion'] == 'jump' and r['objects'] > 1)
    scene = scene_of(turn)
    crowded = scene_of({**turn, 'objects': 20, 'bed': '7.1.2', 'motion': 'fast'})
    tones = sorted([o['signal']['hz'] for o in crowded['objects']]
                   + [s['hz'] for s in crowded['bed_signals'].values()])
    sounding = sorted([(o['signal']['from'], o['signal']['until']) for o in crowded['objects']]
                      + [(s['from'], s['until']) for s in crowded['bed_signals'].values()])
    together = scene_of({**turn, 'sounding': 'together', 'motion': 'fast'})
    # two decodes made by hand: one tone in one element, another in two, and the second decode 37 samples late
    rng = np.random.default_rng(0)
    n = 4 * RATE
    t = np.arange(n) / RATE
    audio = np.zeros((n, 3))
    audio[:, 0] = 0.1 * np.sin(2 * np.pi * 600.0 * t)
    audio[:, 1] = 0.06 * np.sin(2 * np.pi * 932.0 * t)
    audio[:, 2] = 0.08 * np.sin(2 * np.pi * 932.0 * t)
    late = np.vstack([np.zeros((37, 3)), audio[:-37]]) + rng.standard_normal(audio.shape) * 1e-4
    whole, split = tone_in(audio, 600.0, 0, n), tone_in(audio, 932.0, 0, n)
    both = agree(audio, late)
    checks = {
        'every pair of values is in some row': not uncovered(rows, AXES),
        'the rows are fewer than a fiftieth of all the combinations':
            len(rows) < np.prod([len(v) for v in AXES.values()]) / 50,
        'the rows are the same every time': rows == pairwise(AXES),
        'a scene is the same every time': scene == scene_of(turn),
        'the places of a scene are places a stream can say':
            all(e['pos'] == on_grid(*e['pos']) for s in (scene, crowded, together) for o in s['objects']
                for e in o['events']),
        'no two tones of a scene are nearer than 50 Hz and none is above 3,2 kHz':
            min(b - a for a, b in zip(tones, tones[1:])) >= 50.0 and tones[-1] <= 3200.0,
        'elements that sound in turn never sound together':
            all(a[1] <= b[0] for a, b in zip(sounding, sounding[1:])),
        'an object that jumps rests three times and one that circles twice':
            len(rests_of(scene['objects'][0], scene['duration_samples'])) == 3
            and len(rests_of(together['objects'][0], together['duration_samples'])) == 2
            and len(rests_of(crowded['objects'][0], crowded['duration_samples'])) == 2,
        'a scene whose elements sound together names no time for its tones':
            all('from' not in o['signal'] for o in together['objects']),
        'a tone is found at its level in the element that has it':
            abs(20 * np.log10(np.abs(whole[:, 0]).mean()) + 20) < 0.01 and np.abs(whole[:, 1:]).max() < 1e-5,
        'a tone in two elements is found in both, at what each has':
            abs(np.abs(split[:, 1]).mean() / np.abs(split[:, 2]).mean() - 0.75) < 0.001,
        'what two elements have of a tone adds up to it':
            abs(np.abs(split.sum(axis=1)).mean() - 0.14) < 0.0001,
        'the nearest another tone can be, 50 Hz away, a tone is 60 dB down':
            np.abs(tone_in(audio, 650.0, 0, n)).max() < 1e-4,
        'a decode that is late is found late by as much': both['lag_samples'] == 37,
        'and is then as near as its noise leaves it':
            both['compared'] and abs(both['each_db'][0] - 57.0) < 0.5 and both['elements_that_sound'] == 3,
        'fewer elements than ours are not compared': not agree(audio, late[:, :2])['compared'],
        'channels past our elements are left out, and said to be silent or not':
            agree(audio[:, :2], late)['compared'] and not agree(audio[:, :2], late)['and_they_are_silent']
            and agree(audio[:, :2], np.hstack([late[:, :2], np.zeros((n, 2))]))['and_they_are_silent'],
    }
    # three records made by hand: a stream, the same bytes by another path, and a second generation
    made = {'row': turn, 'name': 'a', 'steps': [], 'stream': {'sha256': 'one', 'oamd': {'warp_modes': {'3': 1}}}}
    paths = between([{**made, 'path': 'stream'}, {**made, 'path': 'conversion'},
                     {**made, 'path': 'second', 'first_stream_sha256': 'one',
                      'stream': {'sha256': 'two', 'oamd': {'warp_modes': {'1': 1}}}},
                     {**made, 'path': 'own-adm', 'first_stream_sha256': 'other',
                      'stream': {'sha256': 'two', 'oamd': {'warp_modes': {'1': 1}}}}])
    nothing = {'objects': [], 'bed': [], 'decoded_bed': [], 'decoded_objects': 0}
    said = generations(
        {'a': {**made, 'held': nothing, 'stream': {'dialnorm': [31], 'programme_information': {}}}},
        {'a': {**made, 'held': nothing, 'stream': {'dialnorm': [23], 'programme_information': {}}}})
    checks.update({
        'a dialogue normalisation a second generation was given anew is counted':
            said['what_the_programme_says']['rows_with_another'] == 1
            and said['what_the_programme_says']['its_field_second_minus_first']['least'] == -8,
        'streams of the same bytes by two paths are counted the same':
            paths['through_the_conversion_tool_and_direct'] == {'rows': 1, 'the_same_bytes': 1}
            and paths['the_two_second_generations']['the_same_bytes'] == 1,
        'a first stream of other bytes is counted':
            paths['the_same_master_encoded_again'] == {'rows': 2, 'the_same_bytes': 1},
        'a warp mode a second generation lost is counted lost':
            paths['the_two_second_generations']['rows_whose_first_stream_says_lo_ro'] == 1
            and paths['the_two_second_generations']['and_whose_second_says_it'] == {
                'through_a_master': 0, 'through_adm': 0},
        'a second generation that named the warp mode is beside the one that did not':
            path_of({'path': 'second', 'the_decode_names_the_warp_mode_of_the_stream': True})
            == 'second, the warp mode named' and path_of({'path': 'second'}) == 'second',
    })
    one = {'dialnorm': [27], 'programme_information': {'drc_line': {'speech': 9}, 'drc_rf': {'none': 9}},
           'oamd': {'warp_modes': {'3': 9}},
           'downmix': [{'preferred': 'Lt/Rt', 'ltrt_centre_db': '-4.5', 'ltrt_surround_db': '-1.5',
                        'loro_centre_db': '+3.0', 'loro_surround_db': '-inf', 'frames': 9}]}
    other = {**one, 'dialnorm': [21], 'downmix': [{**one['downmix'][0], 'loro_surround_db': '-3.0', 'frames': 7}],
             'programme_information': {'drc_line': {'speech': 7}, 'drc_rf': {'film_light': 7}}}
    record = {'kept': {SECONDS[0]: kept(programme_of(one), programme_of(other)),
                       SECONDS[1]: kept(programme_of(one), programme_of({**one, 'frames': 5}))},
              'clean': {'first': True}, 'left_to_the_encoder': [],
              'measured': {'first': {'true_peaks': 0}, SECONDS[0]: {'true_peaks': 4}, SECONDS[1]: {'true_peaks': None}}}
    counted = programme_summary([record, {'steps': []}])
    checks.update({
        'a second stream is held to the first field by field, whatever its length':
            record['kept'][SECONDS[0]] == ['preferred_downmix', 'drc_line', 'warp_mode']
            and record['kept'][SECONDS[1]] == list(FIELDS),
        'what the second generations kept is counted for each way of telling the encoder':
            counted['scenes'] == 2 and counted['with_both_second_generations'] == 1
            and counted['second_generations_that_say_a_field_as_the_first_stream_did']['drc_rf']
            == {SECONDS[0]: 0, SECONDS[1]: 1, 'of': 1}
            and counted['streams_with_a_measured_true_peak'] == {'first': 0, SECONDS[0]: 1, SECONDS[1]: 0},
        'every setting a first encoder is told is another than the defaults, and the last leaves the dialogue to it':
            all('loro_cmix=-3:' not in p['encoder'] and 'film_light' not in p['encoder'] for p in PROGRAMMES)
            and [p['dialogue_normalisation'] is None for p in PROGRAMMES] == [False, False, True],
    })
    for name, ok in checks.items():
        print(f'{"ok  " if ok else "FAIL"} {name}')
    return 0 if all(checks.values()) else 1


# ---- what a second generation keeps of what the first stream says of its programme

# What the encoder of a first stream is told: three settings, none of them an
# encoder's defaults; the first two with a dialogue normalisation given by
# hand, the third with the one the encoder measures.
PROGRAMMES = [
    {'encoder': 'preferred_downmix_mode=ltrt:loro_cmix=-1.5:loro_smix=-6:ltrt_cmix=-4.5:ltrt_smix=-1.5:'
                'drc_profile_line=music_standard:drc_profile_rf=speech', 'dialogue_normalisation': -27},
    {'encoder': 'preferred_downmix_mode=not_indicated:loro_cmix=+3:loro_smix=-inf:ltrt_cmix=0:ltrt_smix=-4.5:'
                'drc_profile_line=film_standard:drc_profile_rf=music_light', 'dialogue_normalisation': -14},
    {'encoder': 'preferred_downmix_mode=ltrt-pl2:loro_cmix=-6:loro_smix=-1.5:ltrt_cmix=-inf:ltrt_smix=-3:'
                'drc_profile_line=none:drc_profile_rf=none', 'dialogue_normalisation': None},
]
# every stream of `programme` at one rate: a rate is not what it asks about
PROGRAMME_RATE = '768'
FIELDS = ('dialogue_normalisation', 'preferred_downmix', 'downmix_levels', 'drc_line', 'drc_rf', 'warp_mode')
SECONDS = ('second, told nothing', 'second, told the arguments')


def programme_of(said):
    """What a stream says of its programme as a whole, field by field, as
    `facts` has it."""
    downmix = said.get('downmix') or []
    information = said.get('programme_information') or {}
    return {
        'dialogue_normalisation': said.get('dialnorm'),
        'preferred_downmix': [d['preferred'] for d in downmix],
        'downmix_levels': [[d[key] for key in ('ltrt_centre_db', 'ltrt_surround_db', 'loro_centre_db',
                                               'loro_surround_db')] for d in downmix],
        'drc_line': sorted(information.get('drc_line') or {}),
        'drc_rf': sorted(information.get('drc_rf') or {}),
        'warp_mode': sorted((said.get('oamd') or {}).get('warp_modes') or {}),
    }


def kept(first, second):
    """The fields a second stream says as the first did."""
    return [field for field in FIELDS if first[field] == second[field]]


def measured(said):
    """What the loudness payloads of a stream hold that only a measurement gives."""
    loudness = said.get('loudness') or {}
    return {'true_peaks': (loudness.get('true_peak_dbtp') or {}).get('count'),
            'short_term': (loudness.get('short_term_lufs') or {}).get('count')}


def one_programme(row, k, work, oadec, keep=False):
    """A scene whose first stream says a programme of its own, and two second
    generations of it."""
    import encoder_arguments
    name = f'{row_name(row)}-{k}'
    here = work / name
    here.mkdir(parents=True, exist_ok=True)
    (here / 'scene.json').write_text(json.dumps(scene_of(row), indent=1), encoding='utf-8')
    record = {'row': row, 'name': name, 'the_first_encoder_is_told': PROGRAMMES[k], 'steps': []}
    code, _, stderr, seconds = run([oadec, 'atmos-author', here / 'scene.json', '-o', here / 'master'])
    if not step(record, 'author', code, stderr, seconds):
        return record
    told = ['--encoder', PROGRAMMES[k]['encoder']]
    if PROGRAMMES[k]['dialogue_normalisation'] is not None:
        told += ['--loudness-management', f"skip:custom_dialnorm={PROGRAMMES[k]['dialogue_normalisation']}"]
    first = here / 'first.ec3'
    first.unlink(missing_ok=True)
    if not step(record, 'encode the master', *encode(here / 'master.atmos', first, PROGRAMME_RATE, told)):
        return record
    code, _, stderr, seconds = run([oadec, 'decode', '--format', 'damf', '--warp-mode', 'stream',
                                    '-o', here / 'decoded', first])
    record['steps'].append({'step': 'decode the first stream', 'exit': code, 'seconds': seconds,
                            **({'said': what_it_said(stderr)} if code else {})})
    if not (here / 'decoded.atmos').exists():
        return record
    read = encoder_arguments.report(oadec, first)
    record['the_arguments'] = read['arguments']['e_ac_3']
    record['left_to_the_encoder'] = read['left_to_the_encoder']
    streams = {'first': first}
    for which, extra in zip(SECONDS, ([], record['the_arguments'])):
        out = here / ('second-told.ec3' if extra else 'second-plain.ec3')
        out.unlink(missing_ok=True)
        if step(record, f'encode the decode ({which})', *encode(here / 'decoded.atmos', out, PROGRAMME_RATE, extra)):
            streams[which] = out
    said = {which: facts(oadec, path) for which, path in streams.items()}
    record['says'] = {which: programme_of(f) for which, f in said.items()}
    record['measured'] = {which: measured(f) for which, f in said.items()}
    record['clean'] = {which: f['exit']['verify'] == 0 for which, f in said.items()}
    record['kept'] = {which: kept(record['says']['first'], record['says'][which])
                      for which in SECONDS if which in record['says']}
    if not keep:
        for big in (here / 'master.atmos.audio', here / 'decoded.atmos.audio'):
            big.unlink(missing_ok=True)
    return record


def programme_summary(records):
    """How many second generations keep each field, told nothing and told the
    arguments."""
    whole = [r for r in records if len(r.get('kept') or {}) == len(SECONDS)]
    return {
        'scenes': len(records),
        'with_both_second_generations': len(whole),
        'every_stream_clean': all(all(r['clean'].values()) for r in whole),
        'second_generations_that_say_a_field_as_the_first_stream_did': {
            field: {which: sum(field in r['kept'][which] for r in whole) for which in SECONDS} | {'of': len(whole)}
            for field in FIELDS},
        'with_nothing_left_to_the_encoder': sum(not r['left_to_the_encoder'] for r in whole),
        'streams_with_a_measured_true_peak': {
            which: sum((r['measured'][which]['true_peaks'] or 0) > 0 for r in whole)
            for which in ('first', *SECONDS)},
    }


def parse_rows(text, count):
    if not text:
        return list(range(count))
    picked = []
    for part in text.split(','):
        if '-' in part:
            lo, hi = part.split('-', 1)
            picked += list(range(int(lo), int(hi) + 1))
        else:
            picked.append(int(part))
    return [k for k in picked if 0 <= k < count]


def main():
    parser = argparse.ArgumentParser(description='Scenes through Dolby\'s encoders and back.')
    parser.add_argument('--self-check', action='store_true')
    commands = parser.add_subparsers(dest='command')
    commands.add_parser('plan', help='print the rows')
    go = commands.add_parser('run', help='run rows of the matrix')
    go.add_argument('--work', type=Path, required=True)
    go.add_argument('--rows', help='which rows, as 0-9 or 3,7 (all when absent)')
    go.add_argument('--path', choices=PATHS, default='stream')
    go.add_argument('--bin', help='the oadec binary (default: as tools/oadec_bin.py finds it)')
    go.add_argument('--keep', action='store_true', help='keep the audio of the masters and of the decodes')
    go.add_argument('--warp-mode-of-the-stream', action='store_true',
                    help='own-adm and second: decode the first stream with --warp-mode stream')
    look = commands.add_parser('probe', help='one object alone and one thing said of it, each through the encoder')
    look.add_argument('--work', type=Path, required=True)
    look.add_argument('--path', choices=['stream', 'truehd'], default='stream')
    look.add_argument('--bin', help='the oadec binary (default: as tools/oadec_bin.py finds it)')
    look.add_argument('--keep', action='store_true')
    said = commands.add_parser('programme', help='what a second generation keeps of what the first stream says '
                                                 'of its programme, its encoder told nothing and told the arguments')
    said.add_argument('--work', type=Path, required=True)
    said.add_argument('--rows', default='8,15,34,44', help='which rows of the matrix give the scenes')
    said.add_argument('--bin', help='the oadec binary (default: as tools/oadec_bin.py finds it)')
    said.add_argument('--keep', action='store_true', help='keep the audio of the masters')
    rep = commands.add_parser('report', help='count what rows that were run say')
    rep.add_argument('rows', type=Path, nargs='+')
    rep.add_argument('--out', type=Path)
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    rows = pairwise(AXES)
    if args.command == 'plan':
        for k, row in enumerate(rows):
            print(k, row_name(row), json.dumps(row))
        print(f'{len(rows)} rows of {int(np.prod([len(v) for v in AXES.values()]))} combinations')
        return 0
    if args.command == 'run':
        require_dee('dee_ddpjoc_encoder.exe', 'dee_dthd_encoder.exe')
        oadec = str(Path(args.bin).resolve()) if args.bin else oadec_binary()
        args.work = args.work.resolve()
        args.work.mkdir(parents=True, exist_ok=True)
        target = args.work / f'rows-{args.path}.json'
        done = {r['name']: r for r in json.loads(target.read_text(encoding='utf-8'))} if target.exists() else {}
        for k in parse_rows(args.rows, len(rows)):
            record = one_row(rows[k], args.work, oadec, args.path, args.keep, args.warp_mode_of_the_stream)
            record['index'] = k
            done[record['name']] = record
            target.write_text(json.dumps(sorted(done.values(), key=lambda r: r['index']), indent=1),
                              encoding='utf-8')
            held, both = record.get('held'), record.get('decoders', {})
            print(f"row {k} {json.dumps(rows[k])}: "
                  + ('; '.join(f"{s['step']} {s['exit']}" for s in record['steps'] if s['exit']) or 'every step 0')
                  + ('' if record.get('stream') else f"; NO STREAM after {record['steps'][-1]['step']}")
                  + ('' if held is None else f"; {held['decoded_objects']} objects back, bed {held['decoded_bed']}")
                  + (f"; decoders {both.get('worst_db')} dB at lag {both.get('lag_samples')}"
                     if both.get('compared') else ''), flush=True)
        return 0
    if args.command == 'probe':
        require_dee('dee_ddpjoc_encoder.exe', 'dee_dthd_encoder.exe')
        oadec = str(Path(args.bin).resolve()) if args.bin else oadec_binary()
        args.work = args.work.resolve()
        args.work.mkdir(parents=True, exist_ok=True)
        records = []
        for name, state in probes():
            record = one_probe(name, state, args.work, oadec, args.path, args.keep)
            records.append(record)
            (args.work / f'probes-{args.path}.json').write_text(json.dumps(records, indent=1), encoding='utf-8')
            back, says = record.get('came_back') or {}, record.get('the_stream_says') or {}
            carrier = (back.get('carrier_says') or {}).get('pos')
            print(f"{name}: " + ('NO STREAM' if not record.get('stream') else
                                 f"level {back.get('level_minus_authored_db')} dB, "
                                 f"{back.get('elements_with_a_hundredth_of_it')} elements, most of it "
                                 f"({back.get('share_of_the_carrier')}) at {carrier}, "
                                 f"{back.get('carrier_error')} from where it was put; the stream says "
                                 + (', '.join(key for key, value in says.items() if value) or 'none of it')),
                  flush=True)
        return 0
    if args.command == 'programme':
        require_dee('dee_ddpjoc_encoder.exe', 'dee_dthd_encoder.exe')
        oadec = str(Path(args.bin).resolve()) if args.bin else oadec_binary()
        args.work = args.work.resolve()
        args.work.mkdir(parents=True, exist_ok=True)
        records = []
        for k in parse_rows(args.rows, len(rows)):
            for setting in range(len(PROGRAMMES)):
                record = one_programme(rows[k], setting, args.work, oadec, args.keep)
                record['index'] = k
                records.append(record)
                (args.work / 'programme.json').write_text(
                    json.dumps({'records': records, 'summary': programme_summary(records)}, indent=1),
                    encoding='utf-8')
                print(f"row {k}, setting {setting}: "
                      + ('; '.join(f"{s['step']} {s['exit']}" for s in record['steps'] if s['exit']) or 'every step 0')
                      + ''.join(f"; {which}: kept {len(fields)} of {len(FIELDS)}"
                                for which, fields in (record.get('kept') or {}).items()), flush=True)
        print(json.dumps(programme_summary(records), indent=1))
        return 0
    if args.command == 'report':
        records = [r for path in args.rows for r in json.loads(path.read_text(encoding='utf-8'))]
        text = json.dumps(summarise(records), indent=1)
        if args.out:
            args.out.write_text(text + '\n', encoding='utf-8')
        print(text)
        return 0
    parser.print_usage(sys.stderr)
    return 2


if __name__ == '__main__':
    sys.exit(main())
