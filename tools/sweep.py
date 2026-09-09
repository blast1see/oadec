#!/usr/bin/env python3
"""Run the decoder over the head of every Dolby track in a library.

A corpus of twenty streams answers "does it work on what I have". A sweep over
a few hundred answers "does it work on what encoders actually produce", which
is a different and harder question: catalogue titles from the nineties, recent
Atmos remuxes and streaming ladders were all made by different encoder
generations with different tools switched on.

Reading one minute of each track keeps that affordable. `ffmpeg -c copy` pulls
a head out of a remux in about two seconds without touching the video, and the
head exercises framing, the major syncs, the channel layout, the metadata
containers and every coding tool the encoder chose.

    python tools/sweep.py "E:/films" "D:/films" --out E:/oadec-work/sweep
    python tools/sweep.py --report E:/oadec-work/sweep/report.json

Heads that verify clean are deleted; anything with a failure is kept next to
the report so it can be looked at.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

# ffmpeg codec name -> the raw format to write, and the extension we use
FORMATS = {
    "truehd": ("truehd", "thd"),
    "mlp": ("mlp", "mlp"),
    "eac3": ("eac3", "ec3"),
    "ac3": ("ac3", "ac3"),
}
MEDIA = {".mkv", ".m2ts", ".mp4", ".ts"}


def which(name: str, fallback: str) -> str:
    return shutil.which(name) or fallback


FFMPEG = which("ffmpeg", r"C:\ffmpeg\bin\ffmpeg.exe")
FFPROBE = which("ffprobe", r"C:\ffmpeg\bin\ffprobe.exe")


def probe(path: Path) -> list[dict]:
    """The Dolby audio tracks of one file."""
    cmd = [
        FFPROBE, "-v", "error", "-select_streams", "a",
        "-show_entries", "stream=index,codec_name,channels,profile:stream_tags=language,title",
        "-of", "json", str(path),
    ]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=120).stdout
        streams = json.loads(out).get("streams", [])
    except (subprocess.SubprocessError, json.JSONDecodeError):
        return []
    return [s for s in streams if s.get("codec_name") in FORMATS]


def head(path: Path, stream_index: int, codec: str, out: Path, seconds: int) -> bool:
    fmt, _ = FORMATS[codec]
    cmd = [
        FFMPEG, "-v", "error", "-y", "-i", str(path),
        "-map", f"0:{stream_index}", "-c", "copy", "-t", str(seconds),
        "-f", fmt, str(out),
    ]
    try:
        subprocess.run(cmd, capture_output=True, timeout=900)
    except subprocess.SubprocessError:
        return False
    return out.exists() and out.stat().st_size > 0


def verify(oadec: str, path: Path) -> dict:
    try:
        res = subprocess.run(
            [oadec, "verify", "--json", str(path)],
            capture_output=True, text=True, timeout=1800,
        )
        return json.loads(res.stdout)
    except (subprocess.SubprocessError, json.JSONDecodeError) as e:
        return {"clean": False, "first_error": f"verify did not answer: {e}"}


def summarise(report: list[dict]) -> None:
    total = len(report)
    bad = [r for r in report if not r.get("clean")]
    print(f"\n{total} tracks, {total - len(bad)} clean, {len(bad)} with a failure")
    tools: dict[str, int] = {}
    layouts: dict[str, int] = {}
    for r in report:
        for t in r.get("coverage") or []:
            tools[t] = tools.get(t, 0) + 1
        key = r.get("format", "?")
        layouts[key] = layouts.get(key, 0) + 1
    print("formats: " + ", ".join(f"{k} {v}" for k, v in sorted(layouts.items())))
    print("coding tools: " + ", ".join(f"{k} {v}" for k, v in sorted(tools.items())))
    for r in bad:
        print(f"  FAIL {r['title']} [{r['codec']}] {r.get('first_error')}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("dirs", nargs="*", type=Path)
    ap.add_argument("--out", type=Path, default=Path("sweep"))
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--oadec", default="target/release/oadec")
    ap.add_argument("--report", type=Path, help="print an existing report and stop")
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()

    if args.report:
        summarise(json.loads(args.report.read_text(encoding="utf-8")))
        return 0

    args.out.mkdir(parents=True, exist_ok=True)
    files: list[Path] = []
    for d in args.dirs:
        files += [p for p in sorted(d.rglob("*")) if p.suffix.lower() in MEDIA]
    if args.limit:
        files = files[: args.limit]
    print(f"{len(files)} files")

    report: list[dict] = []
    report_path = args.out / "report.json"
    for n, f in enumerate(files, 1):
        for s in probe(f):
            codec = s["codec_name"]
            _, ext = FORMATS[codec]
            name = f"{f.stem[:60]}.a{s['index']}.{ext}".replace(" ", "_")
            out = args.out / name
            row = {
                "title": f.name,
                "track": s["index"],
                "codec": codec,
                "channels": s.get("channels"),
                "profile": s.get("profile"),
                "language": (s.get("tags") or {}).get("language"),
                "name": (s.get("tags") or {}).get("title"),
            }
            if not head(f, s["index"], codec, out, args.seconds):
                row |= {"clean": False, "first_error": "the head could not be written"}
                report.append(row)
                continue
            v = verify(args.oadec, out)
            row |= {
                "clean": bool(v.get("clean")),
                "format": v.get("format") or v.get("syntax") or codec,
                "coverage": v.get("coverage"),
                "first_error": v.get("first_error"),
                "objects": v.get("object_presentation") or v.get("joc") is not None,
                "bytes": out.stat().st_size,
            }
            report.append(row)
            if row["clean"]:
                out.unlink(missing_ok=True)
            print(f"[{n}/{len(files)}] {'ok  ' if row['clean'] else 'FAIL'} {name}")
            report_path.write_text(json.dumps(report, indent=1), encoding="utf-8")
    report_path.write_text(json.dumps(report, indent=1), encoding="utf-8")
    summarise(report)
    return 0


if __name__ == "__main__":
    sys.exit(main())
