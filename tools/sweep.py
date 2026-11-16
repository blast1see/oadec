#!/usr/bin/env python3
"""Run the decoder over the head of every Dolby track in a library, or over all of it.

A corpus of twenty streams answers "does it work on what I have". A sweep over
a few hundred answers "does it work on what encoders actually produce", which
is a different and harder question: catalogue titles from the nineties, recent
Atmos remuxes and streaming ladders were all made by different encoder
generations with different tools switched on.

Reading one minute of each track keeps that affordable. `ffmpeg -c copy` pulls
a head out of a remux in about two seconds without touching the video, and the
head exercises framing, the major syncs, the channel layout, the metadata
containers and every coding tool the encoder chose.

    python tools/sweep.py "films-a" "films-b" --out oadec-work/sweep
    python tools/sweep.py "films-a" --codecs ac3,eac3 --out oadec-work/sweep-ddp
    python tools/sweep.py "films-a" --seconds 0 --out oadec-work/sweep-whole \
        --tmp fast-disk/cuts --low-priority --stop-file oadec-work/sweep-whole/stop
    python tools/sweep.py --report oadec-work/sweep/report.json

Heads that verify clean are deleted; anything with a failure is kept next to
the report so it can be looked at.

`--seconds 0` reads whole tracks instead of heads, for what an encoder does
once an hour and for what a head cannot hold: a seamless branch, a reel
change. A whole track is as long as the file it is in, so the tracks of a file
are all written in one reading of it, and a library is read once: at the speed
of its disk, which for a shelf of remuxes is most of a day. Three options are
for that. `--tmp` puts the cut tracks on another disk than the report.
`--low-priority` puts the sweep behind everything else on the machine, its
reading of the disk too. `--stop-file` names a file whose appearing ends the
sweep after the file in hand; run again, a sweep goes on where its report
ends. A track that fails is kept as a head is, up to `--keep-gb` in all.

A TrueHD or MLP row also carries the census of the stream (`verify --census`):
how often each element of the syntax occurs. `tools/census.py` adds the rows up
and names what no track carries. It carries what the major sync says of the
stream as well (`info --json`: the flags, the channel modifiers and the
channels each speaks of) and the shortest and the longest delay through the
decoder buffer, so that a field which describes and decodes nothing can be
counted against the layouts real streams have.
"""

from __future__ import annotations

import argparse
import collections
import concurrent.futures
import hashlib
import json
import os
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

# ffmpeg codec name -> the raw format to write, and the extension we use
FORMATS = {
    "truehd": ("truehd", "thd"),
    "mlp": ("mlp", "mlp"),
    "eac3": ("eac3", "ec3"),
    "ac3": ("ac3", "ac3"),
}
MEDIA = {".mkv", ".m2ts", ".mp4", ".ts"}
# a head is written in seconds; a whole track takes as long as its file takes
# to read, behind whatever else is reading the disk
HEAD_TIMEOUT = 900
WHOLE_TIMEOUT = 6 * 3600


def which(name: str, fallback: str) -> str:
    return shutil.which(name) or fallback


FFMPEG = which("ffmpeg", r"ffmpeg.exe")
FFPROBE = which("ffprobe", r"ffprobe.exe")


def low_priority() -> None:
    """Put this process, and every process it starts, behind everything else.

    On Windows the idle priority class leaves a process's reading of the disk
    at the priority of everybody's. The background mode lowers that too, and a
    process started from one in that mode is in it. Measured, with
    `NtQueryInformationProcess`: a process started in the idle class reads at
    "normal", one started from a process in the background mode at "very
    low". Elsewhere this is `nice`.
    """
    if sys.platform == "win32":
        import ctypes

        kernel32 = ctypes.WinDLL("kernel32")
        kernel32.GetCurrentProcess.restype = ctypes.c_void_p
        kernel32.SetPriorityClass.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        # PROCESS_MODE_BACKGROUND_BEGIN
        kernel32.SetPriorityClass(kernel32.GetCurrentProcess(), 0x00100000)
    else:
        os.nice(19)


def probe(path: Path) -> list[dict]:
    """The Dolby audio tracks of one file."""
    cmd = [
        FFPROBE, "-v", "error", "-select_streams", "a",
        "-show_entries", "stream=index,codec_name,channels,profile:stream_tags=language,title",
        "-of", "json", str(path),
    ]
    try:
        # UTF-8 by name: on Windows `text=True` alone decodes with the console
        # code page, and a track title outside it would kill the reader thread
        # and leave this file out without a word
        res = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                             errors="replace", timeout=300)
        streams = json.loads(res.stdout or "{}").get("streams", [])
    except (OSError, ValueError, subprocess.SubprocessError):
        # a file ffprobe cannot open is not a decoder problem; note and move on
        return []
    return [s for s in streams if s.get("codec_name") in FORMATS]


def cut_arguments(stream_index: int, codec: str, out: Path, seconds: int) -> list[str]:
    """What ffmpeg is told of one track: which, how much of it, and where to."""
    fmt, _ = FORMATS[codec]
    return [
        "-map", f"0:{stream_index}", "-c", "copy",
        # 0 seconds is the whole track
        *(["-t", str(seconds)] if seconds else []),
        "-f", fmt, str(out),
    ]


def head(path: Path, stream_index: int, codec: str, out: Path, seconds: int) -> bool:
    cmd = [FFMPEG, "-v", "error", "-y", "-i", str(path),
           *cut_arguments(stream_index, codec, out, seconds)]
    try:
        subprocess.run(cmd, capture_output=True, timeout=HEAD_TIMEOUT if seconds else WHOLE_TIMEOUT)
    except subprocess.SubprocessError:
        return False
    return out.exists() and out.stat().st_size > 0


def cut(path: Path, tracks: list[dict], outs: list[Path], seconds: int) -> list[bool]:
    """Write several tracks of one file in one reading of it.

    The bytes are those of a track cut alone: every output is the packets of
    its track and nothing else. Where the one reading fails, or leaves a track
    unwritten, the tracks are cut alone, as they were before.
    """
    cmd = [FFMPEG, "-v", "error", "-y", "-i", str(path)]
    for s, out in zip(tracks, outs):
        cmd += cut_arguments(s["index"], s["codec_name"], out, seconds)
    try:
        res = subprocess.run(cmd, capture_output=True,
                             timeout=HEAD_TIMEOUT if seconds else WHOLE_TIMEOUT)
        together = res.returncode == 0
    except subprocess.SubprocessError:
        together = False
    return [
        (together and out.exists() and out.stat().st_size > 0)
        or head(path, s["index"], s["codec_name"], out, seconds)
        for s, out in zip(tracks, outs)
    ]


def verify(oadec: str, path: Path, census: bool = False) -> dict:
    try:
        res = subprocess.run(
            [oadec, "verify", "--json", *(["--census"] if census else []), str(path)],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800,
        )
        return json.loads(res.stdout)
    except (subprocess.SubprocessError, json.JSONDecodeError, TypeError) as e:
        return {"clean": False, "first_error": f"verify did not answer: {e}"}


def declared(oadec: str, path: Path) -> dict | None:
    """What the first major sync of a TrueHD stream says of it."""
    try:
        res = subprocess.run(
            [oadec, "info", "--json", str(path)],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800,
        )
        info = json.loads(res.stdout)
    except (subprocess.SubprocessError, json.JSONDecodeError, TypeError):
        return None
    return {
        "flags": info.get("flags"),
        "constant_delay_declared": info.get("constant_delay_declared"),
        "heavy_drc_announced": info.get("heavy_drc_announced"),
        "format_info": info.get("format_info"),
        "presentations": [
            {k: p.get(k) for k in ("kind", "channels", "modifier", "modifier_meaning")}
            for p in info.get("presentations") or []
        ],
    }


class Kept:
    """How many bytes of failing tracks may stay beside the report.

    A failing head is a few megabytes and all of them are kept. A failing
    whole track is gigabytes, and a fault that every track has would fill the
    disk with them; past the allowance a failing track leaves its row only.
    """

    def __init__(self, allowance: int) -> None:
        self.left = allowance
        self.lock = threading.Lock()

    def keep(self, size: int) -> bool:
        with self.lock:
            if size > self.left:
                return False
            self.left -= size
            return True


def examine(oadec: str, title: str, s: dict, out: Path, written: bool, whole: bool,
            kept: Kept, keep_in: Path) -> dict:
    """The row of one track: what the file says of it and what the decoder says."""
    codec = s["codec_name"]
    row = {
        "title": title,
        "track": s["index"],
        "codec": codec,
        "channels": s.get("channels"),
        "profile": s.get("profile"),
        "language": (s.get("tags") or {}).get("language"),
        "name": (s.get("tags") or {}).get("title"),
    }
    if not written:
        what = "track" if whole else "head"
        return row | {"clean": False, "first_error": f"the {what} could not be written"}
    lossless = codec in ("truehd", "mlp")
    v = verify(oadec, out, census=lossless)
    buffer = v.get("buffer") or {}
    size = out.stat().st_size
    row |= {
        # what the major sync says of the stream, and the delay
        # through the decoder buffer its timing fields give
        "declared": declared(oadec, out) if lossless else None,
        "delay_samples": [
            (buffer.get(k) or {}).get("samples") for k in ("min_latency", "max_latency")
        ] if lossless else None,
        "clean": bool(v.get("clean")),
        "format": v.get("format") or v.get("syntax") or codec,
        "coverage": v.get("coverage"),
        "census": v.get("census"),
        # the rules that fired and how often, and what the metadata
        # containers declare of their protection (no word is read):
        # what a severity is decided on is a count over a library
        "rules": {x["id"]: x["count"] for x in (v.get("diagnostics") or {}).get("rules", [])},
        "protection": (v.get("emdf") or v.get("extra") or {}).get("protection"),
        "first_error": v.get("first_error"),
        "objects": v.get("object_presentation") or v.get("joc") is not None,
        # which JOC downmix configuration and how many objects: the
        # decoder's phase-shift path is only exercised by 3 and 4
        "downmix_configs": (v.get("joc") or {}).get("downmix_configs"),
        "objects_per_payload": (v.get("joc") or {}).get("objects_per_payload"),
        "clipgains": (v.get("joc") or {}).get("clipgain_x1000"),
        "bytes": size,
        # how much was read: the access units of a TrueHD or MLP track, the
        # frames and the seconds of an AC-3 or E-AC-3 one
        "units": v.get("units") if lossless else v.get("frames"),
        "duration": v.get("duration"),
        # where the timing of a TrueHD track jumps: a head has no branch,
        # a whole track may
        "timing": v.get("timing_stats"),
        "whole": whole,
    }
    if row["clean"] or not kept.keep(size):
        out.unlink(missing_ok=True)
    elif out.parent != keep_in:
        shutil.move(str(out), str(keep_in / out.name))
    return row


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


def write_report(path: Path, report: list[dict]) -> None:
    """All of the report or the one before it: a sweep that is stopped in the
    middle of writing must not leave half a file to go on from."""
    part = path.with_name(path.name + ".part")
    part.write_text(json.dumps(report, indent=1), encoding="utf-8")
    os.replace(part, path)


def cut_name(f: Path, s: dict) -> str:
    """A name for the cut of one track that no other file's track has.

    The stem alone is cut at sixty characters, and two files that begin alike
    are cut and examined at the same time."""
    _, ext = FORMATS[s["codec_name"]]
    mark = hashlib.sha256(f.name.encode("utf-8")).hexdigest()[:6]
    return f"{f.stem[:60]}.{mark}.a{s['index']}.{ext}".replace(" ", "_")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("dirs", nargs="*", type=Path)
    ap.add_argument("--out", type=Path, default=Path("sweep"))
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--oadec", default="target/release/oadec")
    ap.add_argument("--report", type=Path, help="print an existing report and stop")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--codecs", default=",".join(FORMATS),
                    help="the track codecs to sweep, comma separated; a sweep "
                         "for an AC-3 coding tool need not cut TrueHD heads")
    ap.add_argument("--tmp", type=Path,
                    help="where the tracks are cut to (default: --out); whole tracks "
                         "are gigabytes, and a fast disk that is not the library's "
                         "keeps the reading of the library in one line")
    ap.add_argument("--jobs", type=int, default=2,
                    help="tracks examined at once, while the next file is read")
    ap.add_argument("--keep-gb", type=float, default=20.0,
                    help="how much of failing tracks is kept in all")
    ap.add_argument("--low-priority", action="store_true",
                    help="run behind everything else on the machine, the disk too")
    ap.add_argument("--stop-file", type=Path,
                    help="stop after the file in hand once this file exists")
    args = ap.parse_args()
    codecs = {c.strip() for c in args.codecs.split(",") if c.strip()}
    if not codecs or not codecs <= FORMATS.keys():
        ap.error(f"--codecs takes some of {', '.join(FORMATS)}")

    if args.report:
        summarise(json.loads(args.report.read_text(encoding="utf-8")))
        return 0
    if args.low_priority:
        low_priority()

    args.out.mkdir(parents=True, exist_ok=True)
    tmp = args.tmp or args.out
    tmp.mkdir(parents=True, exist_ok=True)
    whole = args.seconds == 0
    files: list[Path] = []
    for d in args.dirs:
        files += [p for p in sorted(d.rglob("*")) if p.suffix.lower() in MEDIA]
    if args.limit:
        files = files[: args.limit]
    print(f"{len(files)} files", flush=True)

    # resume: a sweep over a library takes long enough that it should not
    # start over when one file upsets ffprobe. The rows of a file are added
    # together, so a file in the report is a file that was finished.
    report_path = args.out / "report.json"
    report: list[dict] = []
    if report_path.exists():
        report = json.loads(report_path.read_text(encoding="utf-8"))
        done = {r["title"] for r in report}
        files = [f for f in files if f.name not in done]
        print(f"{len(report)} tracks already in the report, {len(files)} files left", flush=True)

    kept = Kept(int(args.keep_gb * 2**30))
    # the files whose tracks are cut and are being examined, oldest first
    waiting: collections.deque[tuple[int, list[tuple[str, concurrent.futures.Future]]]] = (
        collections.deque())

    def settle() -> None:
        n, tracks = waiting.popleft()
        for name, future in tracks:
            row = future.result()
            report.append(row)
            print(f"[{n}/{len(files)}] {'ok  ' if row['clean'] else 'FAIL'} {name}", flush=True)
        write_report(report_path, report)

    stopped = False
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        for n, f in enumerate(files, 1):
            if args.stop_file and args.stop_file.exists():
                stopped = True
                break
            tracks = [s for s in probe(f) if s["codec_name"] in codecs]
            if not tracks:
                continue
            outs = [tmp / cut_name(f, s) for s in tracks]
            began = time.monotonic()
            written = cut(f, tracks, outs, args.seconds)
            if whole:
                took = time.monotonic() - began
                print(f"[{n}/{len(files)}] read {f.stat().st_size / 2**30:.1f} GB "
                      f"in {took:.0f} s", flush=True)
            waiting.append((n, [
                (out.name, pool.submit(examine, args.oadec, f.name, s, out, ok, whole, kept, args.out))
                for s, out, ok in zip(tracks, outs, written)
            ]))
            # one file is examined while the next is read, and no more: the
            # cuts of every file that waits are on the disk
            while len(waiting) > 1:
                settle()
        while waiting:
            settle()
    write_report(report_path, report)
    summarise(report)
    if stopped:
        print(f"stopped by {args.stop_file}; run again to go on")
    return 0


if __name__ == "__main__":
    sys.exit(main())
