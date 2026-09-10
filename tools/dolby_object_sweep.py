#!/usr/bin/env python3
"""Which library streams does Dolby's own object decoder open, and which does it refuse?

One stream in the library is refused: Dolby gives Snatch's Dolby Digital Plus
track six channels where it gives every other Atmos track sixteen objects. Every
field this decoder parses agrees between it and an accepted stream at the same
downmix configuration, and the content is not the cause -- its own objects,
handed back to Dolby's encoder, come back as sixteen objects Dolby opens.

Before any more effort goes into that difference it is worth knowing whether it
is a class or a singleton. This sweeps the library through Dolby's object path
and counts the channels it returns.

    python tools/dolby_object_sweep.py "E:/films" "D:/films" --out E:/oadec-work/dolbysweep

The channel count is computed from the size of what Dolby wrote and the sample
count oadec reports, because the plugin does not print a layout in a form worth
parsing. A stream Dolby opens as objects gives sixteen; one it refuses gives the
six of the core.
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

MEDIA = {".mkv", ".m2ts", ".mp4", ".ts", ".mka"}
FFMPEG = shutil.which("ffmpeg") or r"C:\ffmpeg\bin\ffmpeg.exe"
FFPROBE = shutil.which("ffprobe") or r"C:\ffmpeg\bin\ffprobe.exe"
OADEC = find_oadec()
PLAYER = Path(r"C:\Program Files\Dolby\Dolby Reference Player")
GST = PLAYER / "gst-launch-1.0.exe"
SECONDS = "5"


def dolby_env() -> dict:
    env = dict(os.environ)
    env["GST_PLUGIN_PATH"] = str(PLAYER / "gst-plugins")
    env["PATH"] = str(PLAYER) + os.pathsep + env.get("PATH", "")
    return env


def dolby_objects(ec3: Path, out: Path) -> tuple[int, str]:
    """Bytes Dolby's object path writes for this stream, and any error tail."""
    # forward slashes: gst-launch's property parser treats a backslash as an escape
    r = subprocess.run(
        [str(GST), "filesrc", f"location={ec3.as_posix()}", "!", "dlbac3parse",
         "enable-metadata=true",
         "!", "dlbac3dec", "out-ch-config=21", "drc-suppress=true", "drc-mode=custom-0",
         "drc-cut=0", "drc-boost=0", "drop-delay=true",
         "!", "audio/x-raw(meta:DlbObjectAudioMeta),format=F32LE",
         "!", "identity", "!", "filesink", f"location={out.as_posix()}"],
        capture_output=True, text=True, env=dolby_env(), timeout=900)
    size = out.stat().st_size if out.exists() else 0
    return size, "" if r.returncode == 0 else (r.stdout + r.stderr)[-200:]


# what a stream is, rather than how much of it was read
DROP = ("file", "seconds", "speed", "duration", "samples", "frames", "units", "payloads",
        "containers", "bytes", "first_error", "bit_rate", "syncs", "errors", "count",
        "independent_frames", "dependent_frames", "other_program_frames", "transients")


def fields(v: dict) -> dict:
    """Everything `info` reports about the stream, minus what depends on the cut.

    A hand-picked list can only find what was picked, and the question is whether
    anything at all separates the two sets.
    """
    out = {}

    def walk(d, prefix=""):
        for k, val in (d or {}).items():
            if any(w in k.lower() for w in DROP):
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


def info(path: Path) -> dict:
    r = subprocess.run([OADEC, "info", "--json", str(path)], capture_output=True, text=True,
                       timeout=900)
    try:
        return json.loads(r.stdout)
    except json.JSONDecodeError:
        return {}


def dolby_tracks(path: Path) -> list[int]:
    r = subprocess.run([FFPROBE, "-v", "error", "-select_streams", "a", "-show_entries",
                        "stream=index,codec_name", "-of", "json", str(path)],
                       capture_output=True, text=True, timeout=600)
    try:
        return [s["index"] for s in json.loads(r.stdout or "{}").get("streams", [])
                if s.get("codec_name") == "eac3"]
    except json.JSONDecodeError:
        return []


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("roots", nargs="+")
    ap.add_argument("--out", required=True)
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()
    if not GST.is_file():
        raise SystemExit(
            f"the Dolby Reference Player is not at {PLAYER}, so `dlbac3dec` cannot be asked "
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
        for index in dolby_tracks(path):
            tag = f"{path.stem[:40]}#{index}".replace(" ", "_")
            ec3 = work / f"{tag}.ec3"
            if not ec3.exists():
                subprocess.run([FFMPEG, "-v", "error", "-y", "-i", str(path), "-map",
                                f"0:{index}", "-t", SECONDS, "-c", "copy", "-f", "eac3",
                                str(ec3)], capture_output=True, timeout=900)
            if not ec3.exists() or ec3.stat().st_size == 0:
                continue
            v = info(ec3)
            joc = v.get("joc") or {}
            if not (v.get("emdf") or {}).get("joc_payloads"):
                ec3.unlink(missing_ok=True)
                continue
            raw = work / f"{tag}.f32"
            size, err = dolby_objects(ec3, raw)
            samples = v.get("samples") or 0
            # The plugin drops its start-up delay from the end, so the output is
            # a fixed number of samples short of the input on every channel --
            # about 1 500 -- and the ratio of bytes to samples is not an integer.
            # A tolerance on the ratio is therefore wrong: on a five-second cut
            # the same shortfall is a tenth of a channel and on a long one it is
            # nothing. Judge the shortfall itself, and refuse to guess when the
            # output is longer than the input or short by more than a frame.
            ratio = size / 4 / samples if samples else 0.0
            channels = max(1, round(ratio))
            short = samples - (size / 4 / channels) if samples else 0.0
            exact = 0 <= short <= 4000
            rows.append({"file": path.name, "track": index,
                         "downmix_configs": joc.get("downmix_configs"),
                         "objects": joc.get("objects_per_payload"),
                         "joc_payloads": (v.get("emdf") or {}).get("joc_payloads"),
                         "samples": samples, "dolby_bytes": size,
                         "opens": channels >= 16 and exact,
                         "dolby_channels": channels, "channel_ratio": round(ratio, 4),
                         "samples_short_per_channel": round(short, 1),
                         "ratio_is_clean": exact, "fields": fields(v), "error": err})
            print(f"  {channels:2d} ch  cfg {joc.get('downmix_configs')}  {path.name[:70]}",
                  flush=True)
            raw.unlink(missing_ok=True)
            ec3.unlink(missing_ok=True)
            json.dump(rows, open(work / "sweep.json", "w"), indent=1)

    opened = [r for r in rows if r["dolby_channels"] >= 16]
    refused = [r for r in rows if 0 < r["dolby_channels"] < 16 and r["ratio_is_clean"]]
    unclear = [r for r in rows if 0 < r["dolby_channels"] < 16 and not r["ratio_is_clean"]]
    for r in unclear:
        print(f"  check by hand ({r['channel_ratio']} channels, "
              f"{r['samples_short_per_channel']} samples short): "
              f"{r['file']} #{r['track']}")
    print(f"\n{len(rows)} JOC tracks: {len(opened)} opened as objects, {len(refused)} refused")
    for r in refused:
        print(f"  refused: {r['file']} #{r['track']} -> {r['dolby_channels']} channels")
    return 0


if __name__ == "__main__":
    sys.exit(main())
