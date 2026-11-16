#!/usr/bin/env python3
"""What DaVinci Resolve reads of a master it imports, and whether it renders two files the same.

An ADM file is written for other programs to read. Dolby's own tools read
this decoder's (`tools/interop.py`); this puts it to a program people master
in. Resolve Studio is run as a black box through its scripting interface: a
project of its own is made, every file is imported, what the program says of
the clip is recorded (the codec it names, the channels, the format of the
track it maps the clip to, the length), and the project is deleted. Nothing
of the program is read but what it answers.

What it says of an import is a label. Every Dolby Atmos master is "Dolby
Atmos" and twelve channels whatever it holds, so the label does not say that
the programme was read right. `--render DIR` asks for the sound: each clip is
put on a track of as many discrete channels as the program gives the clip
("adaptive12" for a master), where its own 7.1.4 rendering of the master
comes out channel for channel, and the render is read back with ffmpeg.

Files given with `--same` say one programme in different containers, and
each is held against the first of its group sample by sample: an ADM file
this decoder wrote against the ADM file Dolby's Conversion Tool wrote of the
same programme, say. A group of Dolby's own pair, a master and the Conversion
Tool's ADM file of it, is the control: without it a difference between two
renders would say nothing of the files.

    python tools/resolve_import.py plain.wav --out resolve.json --render work/renders \\
        --same own-adm.wav conversion-of-own-master/output.wav own-master.atmos \\
        --same master.atmos conversion-of-master/output.wav
    python tools/resolve_import.py --self-check

Resolve Studio must be running (`Resolve -nogui` will do) and must take
scripts from this machine: Preferences, System, General, "External scripting
using", Local. The free edition takes no script from outside itself. Run it
on a Resolve that was started for it: making a project closes the one that is
open, and although the tool opens that one again when it is done, what was
not saved in it is the program's to keep or lose.

The output names a file by the last two parts of its path, or by as many
more as tell the files apart.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

FFMPEG = shutil.which('ffmpeg') or 'ffmpeg'
FFPROBE = shutil.which('ffprobe') or 'ffprobe'
# what the program says of a clip that is of use here; the rest of its
# properties are dates, paths and fields of a picture
PROPERTIES = ('Type', 'Format', 'Audio Codec', 'Audio Ch', 'Audio Bit Depth', 'Sample Rate', 'Duration', 'FPS')
RENDER_SECONDS = 600


def scripting_paths() -> tuple[Path, Path]:
    """Where an installed Resolve keeps its scripting module and its library
    (the program's own README names these places)."""
    if sys.platform == 'win32':
        return (Path(os.environ.get('PROGRAMDATA', r'C:\ProgramData'))
                / 'Blackmagic Design' / 'DaVinci Resolve' / 'Support' / 'Developer' / 'Scripting',
                Path(os.environ.get('PROGRAMFILES', r'C:\Program Files'))
                / 'Blackmagic Design' / 'DaVinci Resolve' / 'fusionscript.dll')
    if sys.platform == 'darwin':
        return (Path('/Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/Scripting'),
                Path('/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/Libraries/Fusion/fusionscript.so'))
    return Path('/opt/resolve/Developer/Scripting'), Path('/opt/resolve/libs/Fusion/fusionscript.so')


def connect():
    """The running program, or a word on why there is none."""
    api, library = scripting_paths()
    os.environ.setdefault('RESOLVE_SCRIPT_API', str(api))
    os.environ.setdefault('RESOLVE_SCRIPT_LIB', str(library))
    sys.path.insert(0, str(Path(os.environ['RESOLVE_SCRIPT_API']) / 'Modules'))
    try:
        import DaVinciResolveScript as script
    except ImportError as e:
        raise SystemExit(f'the scripting module of DaVinci Resolve is not at {api}: {e}')
    resolve = script.scriptapp('Resolve')
    if resolve is None:
        raise SystemExit('no DaVinci Resolve answers: it is not running, it is not the Studio edition, '
                         'or "External scripting using" in its preferences is not Local')
    return resolve


def names(paths: list[Path]) -> dict[Path, str]:
    """A name for every file that no other of them has: the last two parts of
    its path, or as many more as it takes to tell them apart."""
    parts = {p: p.resolve().parts for p in paths}
    depth = 2
    while True:
        named = {p: '/'.join(parts[p][-depth:]) for p in paths}
        if len(set(named.values())) == len(set(paths)) or depth >= max(map(len, parts.values()), default=0):
            return named
        depth += 1


def said(item) -> dict:
    """What the program says of a clip it imported."""
    properties = item.GetClipProperty() or {}
    try:
        mapping = json.loads(item.GetAudioMapping() or '{}')
    except ValueError:
        mapping = {}
    tracks = mapping.get('track_mapping') or {}
    return {
        **{key.lower().replace(' ', '_'): properties.get(key) or None for key in PROPERTIES},
        'embedded_channels': mapping.get('embedded_audio_channels'),
        'track_formats': [tracks[k].get('type') for k in sorted(tracks, key=int)],
    }


def start(project) -> str | None:
    """Add a render job of the settings in force and start it."""
    job = project.AddRenderJob()
    if not job:
        return None
    for _ in range(3):
        if project.StartRendering([job], False):
            return job
        time.sleep(1)
    project.DeleteRenderJob(job)
    return None


def spend_the_first_render(project, pool, item, outdir: Path) -> None:
    """Ask for a render nobody wants, of the first clip.

    The first timeline a project is given does not render through this
    interface: no job of it starts, however often it is started or added
    again, and the program does not say why. The timelines after it do. So
    the first one is made here and left; if it does render, what it wrote is
    deleted."""
    name = 'spent'
    timeline = pool.CreateTimelineFromClips(name, [item])
    if timeline is None:
        return
    project.SetCurrentTimeline(timeline)
    project.SetRenderSettings({'SelectAllFrames': True, 'TargetDir': str(outdir), 'CustomName': name,
                               'ExportVideo': False, 'ExportAudio': True})
    job = project.AddRenderJob()
    if job and project.StartRendering([job], False):
        began = time.monotonic()
        while project.IsRenderingInProgress() and time.monotonic() - began < RENDER_SECONDS:
            time.sleep(0.5)
    elif job:
        project.DeleteRenderJob(job)
    for made in outdir.iterdir():
        if made.stem == name:
            made.unlink()


def render(project, pool, item, channels: int, outdir: Path, name: str) -> tuple[dict, Path | None]:
    """The sound the program makes of one clip, through a track that mixes
    nothing: as many discrete channels as the clip is given."""
    timeline = pool.CreateEmptyTimeline(name)
    if timeline is None:
        return {'rendered': False, 'why': 'no timeline'}, None
    project.SetCurrentTimeline(timeline)
    if not timeline.AddTrack('audio', f'adaptive{channels}'):
        return {'rendered': False, 'why': f'no track of {channels} discrete channels'}, None
    index = timeline.GetTrackCount('audio')
    if not pool.AppendToTimeline([{'mediaPoolItem': item, 'trackIndex': index, 'mediaType': 2}]):
        return {'rendered': False, 'why': 'the clip could not be put on the track'}, None
    project.SetRenderSettings({
        'SelectAllFrames': True, 'TargetDir': str(outdir), 'CustomName': name,
        'ExportVideo': False, 'ExportAudio': True, 'AudioBitDepth': 24, 'AudioSampleRate': 48000,
    })
    job = start(project)
    if job is None:
        return {'rendered': False, 'why': 'no render job started'}, None
    began = time.monotonic()
    while project.IsRenderingInProgress() and time.monotonic() - began < RENDER_SECONDS:
        time.sleep(0.5)
    status = (project.GetRenderJobStatus(job) or {}).get('JobStatus')
    made = sorted(p for p in outdir.iterdir() if p.stem == name)
    if status != 'Complete' or not made:
        return {'rendered': False, 'why': f'the render job ended as {status}'}, None
    return {'rendered': True, 'track': f'adaptive{channels}'}, made[0]


def samples(path: Path) -> np.ndarray:
    """The first audio stream of a file as 24-bit integers, frames by channels."""
    probe = subprocess.run([FFPROBE, '-v', 'error', '-select_streams', 'a:0', '-show_entries',
                            'stream=channels', '-of', 'json', str(path)], capture_output=True, check=True)
    channels = json.loads(probe.stdout)['streams'][0]['channels']
    raw = subprocess.run([FFMPEG, '-v', 'error', '-i', str(path), '-map', '0:a:0', '-f', 's32le',
                          '-acodec', 'pcm_s32le', '-'], capture_output=True, check=True).stdout
    return np.frombuffer(raw, dtype='<i4').reshape(-1, channels) >> 8


def level(x: np.ndarray) -> list[float | None]:
    """RMS of every channel in dB below full scale; None for one that is silent."""
    rms = np.sqrt((x.astype(np.float64) ** 2).mean(axis=0)) / 2**23 if len(x) else np.zeros(x.shape[1])
    return [round(float(20 * np.log10(v)), 2) if v > 0 else None for v in rms]


def heard(x: np.ndarray) -> dict:
    return {'channels': int(x.shape[1]), 'samples': int(x.shape[0]),
            'sha256': hashlib.sha256(np.ascontiguousarray(x).tobytes()).hexdigest(),
            'rms_dbfs': level(x)}


def held(first: np.ndarray, other: np.ndarray) -> dict:
    """`other` against `first`, sample by sample."""
    if first.shape != other.shape:
        return {'identical': False, 'shapes': [list(first.shape), list(other.shape)]}
    difference = other.astype(np.int64) - first
    if not difference.any():
        return {'identical': True}
    signal = (first.astype(np.float64) ** 2).sum(axis=0)
    noise = (difference.astype(np.float64) ** 2).sum(axis=0)
    return {
        'identical': False,
        'channels_that_differ': [int(k) for k in np.flatnonzero(noise)],
        'largest_difference_in_24_bit_steps': int(np.abs(difference).max()),
        'snr_db': [round(float(10 * np.log10(s / n)), 2) if n > 0 and s > 0 else None
                   for s, n in zip(signal, noise)],
    }


def run(files: list[Path], groups: list[list[Path]], outdir: Path | None) -> dict:
    resolve = connect()
    manager = resolve.GetProjectManager()
    before = manager.GetCurrentProject()
    before_name = before.GetName() if before else None
    name = f'oadec-import-{os.getpid()}-{int(time.time())}'
    project = manager.CreateProject(name)
    if project is None:
        raise SystemExit('the program made no project')
    record = {'program': resolve.GetProductName(), 'version': resolve.GetVersionString(), 'files': {}}
    renders: dict[Path, np.ndarray] = {}
    every = list(dict.fromkeys([*files, *(f for g in groups for f in g)]))
    short = names(every)
    try:
        pool = project.GetMediaPool()
        spent = False
        for k, path in enumerate(every):
            items = pool.ImportMedia([str(path.resolve())]) or []
            row: dict = {'clips': len(items)}
            if items:
                row |= said(items[0])
                if outdir is not None:
                    if not spent:
                        spend_the_first_render(project, pool, items[0], outdir)
                        spent = True
                    channels = int(row.get('audio_ch') or 0)
                    row['render'], made = render(project, pool, items[0], channels, outdir, f'render-{k:03d}')
                    if made is not None:
                        renders[path] = samples(made)
                        row['render'] |= heard(renders[path])
            record['files'][short[path]] = row
            print(f"{short[path]}: {row.get('audio_codec')}, {row.get('audio_ch')} channels, "
                  f"{row.get('track_formats')}", flush=True)
    finally:
        manager.CloseProject(project)
        record['the_project_was_deleted'] = bool(manager.DeleteProject(name))
        if before_name:
            manager.LoadProject(before_name)
    if outdir is not None:
        record['same'] = []
        for group in groups:
            first = group[0]
            row = {'first': short[first], 'against_it': {}}
            for other in group[1:]:
                if first in renders and other in renders:
                    row['against_it'][short[other]] = held(renders[first], renders[other])
                else:
                    row['against_it'][short[other]] = {'identical': None, 'why': 'a render is missing'}
            record['same'].append(row)
            words = {True: 'the same', False: 'not the same', None: 'not compared'}
            print(f"{short[first]}: " + ', '.join(
                f"{n} {words[v.get('identical')]}" for n, v in row['against_it'].items()), flush=True)
    return record


def self_check() -> int:
    rng = np.random.default_rng(1)
    a = rng.integers(-2**20, 2**20, size=(4800, 12), dtype=np.int64).astype(np.int32)
    a[:, 3] = 0
    one_off = a.copy()
    one_off[100, 5] += 1
    quieter = (a // 2).astype(np.int32)
    answers = {
        'a render against itself is identical': held(a, a.copy()) == {'identical': True},
        'one step in one sample of one channel is found there':
            (lambda r: r['identical'] is False and r['channels_that_differ'] == [5]
             and r['largest_difference_in_24_bit_steps'] == 1 and r['snr_db'][5] > 100
             and r['snr_db'][0] is None)(held(a, one_off)),
        'half the level is 6 dB of difference in every channel that sounds':
            (lambda r: r['channels_that_differ'] == [c for c in range(12) if c != 3]
             and all(abs(v - 6.02) < 0.01 for c, v in enumerate(r['snr_db']) if c != 3)
             and r['snr_db'][3] is None)(held(a, quieter)),
        'renders of two lengths are not compared': held(a, a[:-1])['shapes'] == [[4800, 12], [4799, 12]],
        'a silent channel has no level, and full scale is 0 dB':
            (lambda v: v[0] is None and abs(v[1]) < 0.001)(
                level(np.stack([np.zeros(64, dtype=np.int32), np.full(64, 2**23, dtype=np.int32)], axis=1))),
        'a render is named by its samples': heard(a)['sha256'] != heard(one_off)['sha256']
            and heard(a)['sha256'] == heard(a.copy())['sha256'] and heard(a)['channels'] == 12,
        'a file is named by two parts of its path':
            list(names([Path('a') / 'b' / 'c.wav', Path('a') / 'd.wav']).values()) == ['b/c.wav', 'a/d.wav'],
        'and by as many more as tell two files apart':
            list(names([Path('x') / 'one' / 'out' / 'c.wav', Path('x') / 'two' / 'out' / 'c.wav']).values())
            == ['one/out/c.wav', 'two/out/c.wav'],
    }
    for what, ok in answers.items():
        print('ok  ' if ok else 'FAIL', what)
    return 0 if all(answers.values()) else 1


def main() -> int:
    parser = argparse.ArgumentParser(description='What DaVinci Resolve reads of a master it imports.')
    parser.add_argument('files', nargs='*', type=Path, help='files to import, each for itself')
    parser.add_argument('--same', nargs='+', type=Path, action='append', default=[], metavar='FILE',
                        help='files that say one programme: each is held against the first (needs --render)')
    parser.add_argument('--render', type=Path, metavar='DIR',
                        help='render the sound of every clip into this folder and read it back')
    parser.add_argument('--out', type=Path, help='write the record here as JSON')
    parser.add_argument('--self-check', action='store_true')
    args = parser.parse_args()
    if args.self_check:
        return self_check()
    if not args.files and not args.same:
        parser.error('a file, or --self-check')
    if args.same and args.render is None:
        parser.error('--same compares renders: it needs --render')
    missing = [str(p) for p in [*args.files, *(f for g in args.same for f in g)] if not p.exists()]
    if missing:
        parser.error('not found: ' + ', '.join(missing))
    if args.render is not None:
        args.render.mkdir(parents=True, exist_ok=True)
    record = run(args.files, args.same, args.render.resolve() if args.render else None)
    text = json.dumps(record, indent=1, ensure_ascii=False)
    if args.out:
        args.out.write_text(text + '\n', encoding='utf-8')
    else:
        print(text)
    return 0


if __name__ == '__main__':
    sys.exit(main())
