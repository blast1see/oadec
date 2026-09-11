#!/usr/bin/env python3
"""Which TrueHD Atmos streams does Dolby's object path open, and which does it refuse?

Three titles of six are refused at `presentation=16` with `out-ch-config=21`,
and nothing in the 74 fields oadec reports separates the three from the other
three. Six is a small sample for that question. The library holds around a
hundred TrueHD Atmos tracks and twenty seconds of each is cheap, so the same
sweep that answered it for Dolby Digital Plus can answer it here: is the refusal
a small class or a large one, and does a bigger control set show a field that
six titles could not.

    python tools/dolby_truehd_sweep.py "E:/films" "D:/films" --out E:/oadec-work/thdsweep

Dolby's parser will not read a raw elementary stream, so each cut is wrapped in
MP4 first; the unpatched-stream control in an earlier round showed the wrapping
itself is innocent.

**Cut from the start.** A twenty-second cut taken ten minutes in is refused even
for a title this decoder opens -- Pi from the head is accepted and Pi from 600
seconds is not -- because a byte seek into a raw elementary stream lands inside
an access unit and the preroll never finds the presentation. The refusal message
is the same one a genuine refusal gives, so a sweep that seeks would report every
title as refused and look like a discovery.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import sys as _sys
from pathlib import Path as _Path

_sys.path.insert(0, str(_Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402

MEDIA = {".mkv", ".m2ts", ".mp4", ".ts"}
FFMPEG = shutil.which("ffmpeg") or r"C:\ffmpeg\bin\ffmpeg.exe"
FFPROBE = shutil.which("ffprobe") or r"C:\ffmpeg\bin\ffprobe.exe"
OADEC = find_oadec()
PLAYER = Path(r"C:\Program Files\Dolby\Dolby Reference Player")
GST = PLAYER / "gst-launch-1.0.exe"
REFUSAL = "not available"
SECONDS = "20"
# from the start: see the note in the docstring about mid-file cuts
START = "0"


def dolby_env() -> dict:
    env = dict(os.environ)
    env["GST_PLUGIN_PATH"] = str(PLAYER / "gst-plugins")
    env["PATH"] = str(PLAYER) + os.pathsep + env.get("PATH", "")
    return env


def opens_objects(mp4: Path) -> tuple[bool, str]:
    r = subprocess.run(
        [str(GST), "filesrc", f"location={mp4.as_posix()}", "!", "qtdemux", "!", "capssetter",
         "caps=audio/x-true-hd", "replace=true", "join=false", "!", "dlbtruehdparse",
         "!", "dlbtruehddec", "out-ch-config=21", "presentation=16", "!", "fakesink"],
        capture_output=True, text=True, env=dolby_env(), timeout=900)
    text = r.stdout + r.stderr
    return REFUSAL not in text, text[-200:] if r.returncode else ""


def truehd_tracks(path: Path) -> list[int]:
    r = subprocess.run([FFPROBE, "-v", "error", "-select_streams", "a", "-show_entries",
                        "stream=index,codec_name", "-of", "json", str(path)],
                       capture_output=True, text=True, timeout=600)
    try:
        return [s["index"] for s in json.loads(r.stdout or "{}").get("streams", [])
                if s.get("codec_name") in ("truehd", "mlp")]
    except json.JSONDecodeError:
        return []


# what a stream is, rather than how much of it was read
DROP = ("file", "file_bytes", "units", "total_unit_bytes", "duration_seconds",
        "first_unit_offset", "first_input_timing", "last_input_timing", "crc_present_units",
        "drc_updates", "timing_stats", "average_bit_rate", "major_syncs")


def fields(v: dict) -> dict:
    """Everything `info` reports about the stream, minus what depends on the cut.

    A hand-picked list can only find what was picked. The question this sweep
    exists for is whether *anything* separates the two sets, so the bag is
    everything and the analysis decides.
    """
    out = {}

    def walk(d, prefix=""):
        for k, val in (d or {}).items():
            if k in DROP:
                continue
            p = f"{prefix}{k}"
            if isinstance(val, dict):
                walk(val, p + ".")
            elif isinstance(val, list):
                walk({str(i): x for i, x in enumerate(val)}, p + ".")
            else:
                out[p] = val
    walk(v)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("roots", nargs="+")
    ap.add_argument("--out", required=True)
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()
    if not GST.is_file():
        raise SystemExit(
            f"the Dolby Reference Player is not at {PLAYER}, so `dlbtruehddec` cannot be asked "
            "anything. Install it, or point PLAYER at it.")
    work = Path(args.out)
    work.mkdir(parents=True, exist_ok=True)

    files: list[Path] = []
    for root in args.roots:
        for p in sorted(Path(root).rglob("*")):
            if p.suffix.lower() in MEDIA and p.is_file():
                files.append(p)
    if args.limit:
        files = files[:args.limit]
    print(f"{len(files)} files", flush=True)

    rows = []
    for path in files:
        for index in truehd_tracks(path):
            tag = f"{path.stem[:36]}_{index}".replace(" ", "_")
            thd = work / f"{tag}.thd"
            mp4 = work / f"{tag}.mp4"
            if not mp4.exists():
                subprocess.run([FFMPEG, "-v", "error", "-y", "-ss", START, "-i", str(path),
                                "-map", f"0:{index}", "-t", SECONDS, "-c", "copy", "-f", "truehd",
                                str(thd)], capture_output=True, timeout=900)
                if thd.exists() and thd.stat().st_size:
                    subprocess.run([FFMPEG, "-v", "error", "-y", "-i", str(thd), "-c:a", "copy",
                                    "-strict", "-2", "-movflags", "faststart", "-f", "mp4",
                                    str(mp4)], capture_output=True, timeout=900)
            if not mp4.exists() or mp4.stat().st_size == 0:
                thd.unlink(missing_ok=True)
                continue
            v = {}
            if thd.exists():
                try:
                    v = json.loads(subprocess.run([OADEC, "info", "--json", str(thd)],
                                                  capture_output=True, text=True).stdout or "{}")
                except json.JSONDecodeError:
                    v = {}
            if not v.get("has_16ch_presentation"):
                thd.unlink(missing_ok=True)
                mp4.unlink(missing_ok=True)
                continue
            ok, err = opens_objects(mp4)
            rows.append({"file": path.name, "track": index, "opens": ok,
                         "fields": fields(v), "error": err})
            print(f"  {'opens ' if ok else 'REFUSED'}  {path.name[:70]}", flush=True)
            # the cut is kept: reading it again is seconds, cutting it again is
            # minutes of the library's disks
            mp4.unlink(missing_ok=True)
            json.dump(rows, open(work / "sweep.json", "w"), indent=1)

    refused = [r for r in rows if not r["opens"]]
    print(f"\n{len(rows)} object presentations: {len(rows) - len(refused)} opened, "
          f"{len(refused)} refused")
    for r in refused:
        print(f"  refused: {r['file']} #{r['track']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
