#!/usr/bin/env python3
"""Which JOC downmix configuration every Dolby track in a library uses.

The configuration is a header field and is constant within a stream in every
scan so far, so a head clip answers it -- unlike the per-frame syntax counts,
which head clips famously hide. What a head clip does not excuse is looking at
one track per file: the library's configuration 4 material sits on discs that
also carry a configuration 3 track, and a sweep that stops at the first Dolby
track sees only the second. Reading every track found five configuration 4
streams where the earlier one-track-per-file sweep had found none.

    python tools/joc_config_sweep.py tracks.json out.json

`tracks.json` is a mapping of name to path, which is how the library inventory
is kept; the output records every Dolby track, its configuration and its
programme channels.
"""
import json, subprocess, sys, tempfile
from pathlib import Path

BIN = r'target\release\oadec.exe'
FFMPEG, FFPROBE = 'ffmpeg', 'ffprobe'
SECONDS = '10'


def tracks(path):
    r = subprocess.run([FFPROBE, '-v', 'error', '-select_streams', 'a', '-show_entries',
                        'stream=index,codec_name,channels', '-of', 'json', str(path)],
                       capture_output=True, text=True, timeout=300)
    try:
        return [(s['index'], s.get('channels'))
                for s in json.loads(r.stdout or '{}').get('streams', [])
                if s.get('codec_name') in ('eac3', 'ac3')]
    except json.JSONDecodeError:
        return []


def main():
    files = json.load(open(sys.argv[1], encoding='utf-8'))
    out_path = sys.argv[2]
    work = Path(tempfile.mkdtemp(prefix='cfg-sweep-'))
    clip = work / 'head.ec3'
    rows = []
    names = sorted(files)
    for i, name in enumerate(names, 1):
        src = files[name]
        for idx, ch in tracks(src):
            cut = subprocess.run([FFMPEG, '-v', 'error', '-y', '-i', src, '-map', f'0:{idx}',
                                  '-c', 'copy', '-t', SECONDS, '-f', 'eac3', str(clip)],
                                 capture_output=True, text=True, timeout=1800)
            if cut.returncode != 0:
                continue
            info = subprocess.run([BIN, 'info', '--json', str(clip)],
                                  capture_output=True, text=True, timeout=600)
            try:
                d = json.loads(info.stdout)
            except json.JSONDecodeError:
                continue
            joc = d.get('joc') or {}
            rows.append({'file': name, 'track': idx, 'ffprobe_channels': ch,
                         'channels': d.get('channels'),
                         'downmix_configs': joc.get('downmix_configs'),
                         'objects': joc.get('objects_per_payload'),
                         'joc_payloads': (d.get('emdf') or {}).get('joc_payloads')})
        print(f'\r{i}/{len(names)}', end='', flush=True)
    print()
    clip.unlink(missing_ok=True)
    work.rmdir()
    configs = {}
    for r in rows:
        for c in (r['downmix_configs'] or []):
            configs.setdefault(c, []).append(f"{r['file']}#{r['track']}")
    summary = {'files': len(names), 'dolby_tracks': len(rows),
               'tracks_with_joc': sum(1 for r in rows if r['downmix_configs']),
               'configurations': {str(k): len(v) for k, v in sorted(configs.items())},
               'streams_per_configuration': {str(k): v for k, v in sorted(configs.items())}}
    json.dump({'summary': summary, 'tracks': rows}, open(out_path, 'w', encoding='utf-8'), indent=1)
    print(json.dumps(summary['configurations'], indent=1))
    print('tracks with JOC:', summary['tracks_with_joc'], 'of', summary['dolby_tracks'])


if __name__ == '__main__':
    main()
