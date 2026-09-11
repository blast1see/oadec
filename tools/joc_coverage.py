#!/usr/bin/env python3
"""Count which JOC syntax every object-carrying stream in a library actually uses.

The specification offers sparse and dense matrices, coarse and fine
quantisation, smooth and steep slopes, one or two data points, and five downmix
configurations. A decoder can implement all of it and still have exercised half
of it, and a corpus of head clips will not tell you which half: the audit's zero
for sparse mode came from scanning three megabytes off the front of fifteen
clips, while a whole-file scan of one streaming title had already found sixty
sparse objects a fortnight earlier.

So this reads whole files, not heads, and reports exact counts.

    python tools/joc_coverage.py --out coverage.json E:/oadec-work/ec3
    python tools/joc_coverage.py --out coverage.json --library E:/films D:/films

With `--library` it walks the media in those trees, pulls each Dolby track out
with `ffmpeg -c copy` and scans that; without it, the arguments are elementary
streams to scan directly.
"""

from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

import sys as _sys
from pathlib import Path as _Path

_sys.path.insert(0, str(_Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402

FFMPEG = shutil.which("ffmpeg") or r"C:\ffmpeg\bin\ffmpeg.exe"
FFPROBE = shutil.which("ffprobe") or r"C:\ffmpeg\bin\ffprobe.exe"
MEDIA = {".mkv", ".mka", ".m2ts", ".mp4", ".ts"}
KEYS = ("sparse_objects", "dense_objects", "fine_quantized_objects",
        "coarse_quantized_objects", "two_data_points", "steep_objects",
        "seq_count_zero", "absent_objects", "errors", "size_mismatches",
        "non_zero_padding", "parsed")


def scan(binary: str, path: Path) -> dict | None:
    res = subprocess.run([binary, "info", "--json", str(path)],
                         capture_output=True, text=True, timeout=7200)
    try:
        d = json.loads(res.stdout)
    except json.JSONDecodeError:
        return None
    j = d.get("joc") or {}
    e = d.get("emdf") or {}
    return {
        "carries_joc": bool(j.get("parsed")),
        "auxdata_frames": e.get("auxdata_frames"),
        "auxdata_bytes": e.get("auxdata_bytes"),
        "containers_in_auxdata": e.get("containers_in_auxdata"),
        "auxdata_overruns": e.get("auxdata_overruns"),
        "containers": e.get("containers"),
        # object gain and object size: parsed, never seen, and countable only
        # over real streams because both Dolby encoders drop the two fields
        "oamd_object_gains_db": e.get("oamd_object_gains_db", {}),
        "oamd_muted_updates": e.get("oamd_muted_updates"),
        "oamd_sized_updates": e.get("oamd_sized_updates"),
        "containers_in_independent_substream": e.get("containers_in_independent_substream"),
        "frames": d["frames"],
        "samples": d["samples"],
        "bit_rate": d["bit_rate"],
        "joc": {k: j.get(k) for k in KEYS},
        "downmix_configs": j.get("downmix_configs"),
        "objects_per_payload": j.get("objects_per_payload"),
        "bands": j.get("bands"),
        "interpolation_branches": j.get("interpolation_branches", {}),
        "offset_ts": j.get("offset_ts", {}),
        "clipgains": j.get("clipgain_x1000"),
        "clean": d["clean"],
    }


def dolby_tracks(path: Path) -> list[int]:
    res = subprocess.run(
        [FFPROBE, "-v", "error", "-select_streams", "a", "-show_entries",
         "stream=index,codec_name", "-of", "json", str(path)],
        capture_output=True, text=True, timeout=300)
    try:
        streams = json.loads(res.stdout or "{}").get("streams", [])
    except json.JSONDecodeError:
        return []
    return [s["index"] for s in streams if s.get("codec_name") in ("eac3", "ac3")]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--binary", default=find_oadec(required=False))
    ap.add_argument("--out", required=True)
    ap.add_argument("--library", action="store_true",
                    help="the paths are media trees or media files, not elementary streams")
    a = ap.parse_args()

    streams: list[tuple[str, Path, int | None]] = []
    if a.library:
        for root in a.paths:
            p = Path(root)
            # a path may be one media file rather than a tree, which is how a
            # chosen subset of a library is scanned without copying it
            files = [p] if p.is_file() else sorted(p.rglob("*"))
            for f in files:
                if f.suffix.lower() in MEDIA:
                    for idx in dolby_tracks(f):
                        streams.append((f"{f.name}#{idx}", f, idx))
    else:
        for root in a.paths:
            p = Path(root)
            files = sorted(p.rglob("*")) if p.is_dir() else [p]
            for f in files:
                if f.suffix.lower() in (".ec3", ".eac3", ".ac3"):
                    streams.append((f.name, f, None))

    work = Path(tempfile.mkdtemp(prefix="joc-cov-"))
    rows, total = [], Counter()
    for i, (name, path, idx) in enumerate(streams, 1):
        print(f"[{i}/{len(streams)}] {name[:70]}", flush=True)
        target = path
        if idx is not None:
            target = work / "track.ec3"
            cut = subprocess.run(
                [FFMPEG, "-v", "error", "-y", "-i", str(path), "-map", f"0:{idx}",
                 "-c", "copy", "-f", "eac3", str(target)],
                capture_output=True, text=True, timeout=3600)
            if cut.returncode != 0:
                continue
        row = scan(a.binary, target)
        if idx is not None:
            target.unlink(missing_ok=True)
        if row is None:
            continue
        row["stream"] = name
        rows.append(row)
        total["auxdata_frames"] += row.get("auxdata_frames") or 0
        total["containers_in_auxdata"] += row.get("containers_in_auxdata") or 0
        total["auxdata_overruns"] += row.get("auxdata_overruns") or 0
        total["oamd_muted_updates"] += row.get("oamd_muted_updates") or 0
        total["oamd_sized_updates"] += row.get("oamd_sized_updates") or 0
        for db, n in (row.get("oamd_object_gains_db") or {}).items():
            total[f"gain_db:{db}"] += n
        if not row["carries_joc"]:
            continue
        for k in KEYS:
            total[k] += row["joc"].get(k) or 0
        for k, v in row["interpolation_branches"].items():
            total[f"interp:{k}"] += v
        for c in row["downmix_configs"] or []:
            total[f"dmx:{c}"] += 1
        json.dump({"streams": rows}, open(a.out, "w"), indent=1)
    shutil.rmtree(work, ignore_errors=True)

    updates = total["sparse_objects"] + total["dense_objects"]
    with_joc = [r for r in rows if r["carries_joc"]]
    summary = {
        "streams_scanned": len(streams),
        "streams_decoded": len(rows),
        "streams_carrying_joc": len(with_joc),
        "auxdata_frames": total["auxdata_frames"],
        "containers_in_auxdata": total["containers_in_auxdata"],
        "auxdata_overruns": total["auxdata_overruns"],
        "oamd_object_gains_db": {k.split(":", 1)[1]: v for k, v in total.items()
                                 if k.startswith("gain_db:")},
        "oamd_muted_updates": total["oamd_muted_updates"],
        "oamd_sized_updates": total["oamd_sized_updates"],
        "object_updates": updates,
        "sparse": total["sparse_objects"],
        "dense": total["dense_objects"],
        "fine": total["fine_quantized_objects"],
        "coarse": total["coarse_quantized_objects"],
        "steep": total["steep_objects"],
        "two_data_points": total["two_data_points"],
        "interpolation_branches": {k[7:]: v for k, v in total.items() if k.startswith("interp:")},
        "downmix_configs": {k[4:]: v for k, v in total.items() if k.startswith("dmx:")},
        "parse_errors": total["errors"],
        "size_mismatches": total["size_mismatches"],
        "non_zero_padding": total["non_zero_padding"],
        "streams_with_sparse": [r["stream"] for r in with_joc if r["joc"]["sparse_objects"]],
        "streams_with_coarse": [r["stream"] for r in with_joc if r["joc"]["coarse_quantized_objects"]],
        "streams_with_two_data_points": [r["stream"] for r in with_joc if r["joc"]["two_data_points"]],
        "streams_with_object_gain": [r["stream"] for r in rows
                                     if r.get("oamd_object_gains_db")],
        "streams_with_object_size": [r["stream"] for r in rows
                                     if r.get("oamd_sized_updates")],
    }
    json.dump({"summary": summary, "streams": rows}, open(a.out, "w"), indent=1)
    print(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
