#!/usr/bin/env python3
"""Count which programme metadata the streams of a library actually carry.

`docs/oamd.md` records that six films carry no trim, no snap and no screen
reference. Six films cannot say whether a writer for those fields would ever
run, and that is the question before writing one. This reads the head of every
Dolby track, records what the metadata commands report, and aggregates it.

    python tools/metadata_census.py films/ more-films/ --out work/census
    python tools/metadata_census.py --report work/census/census.jsonl
    python tools/metadata_census.py --report ... --public evidence.json

The rows name the files they came from and stay in the work directory. `--public`
writes the aggregate: counts, with a title and year only where a rare property has
three carriers or fewer. That is the only form that belongs in the repository, which
has no reason to publish what is on somebody's shelf.

A head is a sample of the track. Programme-level configuration is written at the
start of a title, but a field that first appears later is not counted here.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import threading
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as oadec_binary  # noqa: E402
from sweep import FORMATS, MEDIA, head, probe  # noqa: E402

OAMD_KEYS = (
    "payloads",
    "payloads_with_trim",
    "payloads_with_extended",
    "payloads_with_unknown_elements",
    "additional_table_data",
    "element_ids",
    "object_counts",
    "program",
    "screen_referenced",
    "snapped",
    "zone_constraints",
    "object_gains_db",
    "muted_updates",
    "inactive_updates",
    "sized_updates",
    "differential_positions",
    "distances",
    "warp_modes",
    "global_trim_modes",
    "trim_configs",
    "per_object_trim_disables",
    "alternate_object_data",
)

YEAR = re.compile(r"[ ._(]((?:19|20)\d\d)(?=[ ._)]|$)")


def public_title(file_name: str) -> str:
    """Title and year, without the release name around them."""
    stem = re.split(r"[. ](?:\d{3,4}p|UHD|BluRay|Blu-ray|WEB|REMUX)", Path(file_name).stem)[0]
    # the last year-like word: "Blade Runner 2049 2017" and "1917 2019" are titles first
    years = list(YEAR.finditer(stem))
    if not years:
        return stem.replace(".", " ").strip()
    last = years[-1]
    return f"{stem[:last.start()].replace('.', ' ').strip(' (')} ({last.group(1)})"


def run_json(oadec: str, command: str, path: Path) -> dict:
    try:
        res = subprocess.run(
            [oadec, command, "--json", str(path)],
            capture_output=True,
            text=True,
            timeout=900,
        )
        return json.loads(res.stdout)
    except (subprocess.SubprocessError, json.JSONDecodeError):
        return {}


def dialnorm_of(codec: str, info: dict) -> list:
    """The dialogue normalisation the stream declares for its top presentation."""
    if codec in ("eac3", "ac3"):
        return info.get("dialnorm") or []
    extra = info.get("sixteen_channel") or info.get("extra") or {}
    if isinstance(extra, dict) and extra.get("dialogue_norm") is not None:
        return [extra["dialogue_norm"]]
    meaning = info.get("channel_meaning") or {}
    return [meaning[k] for k in ("eightch_dialogue_norm", "sixch_dialogue_norm") if k in meaning][:1]


def census_track(args, f: Path, s: dict) -> dict:
    codec = s["codec_name"]
    _, ext = FORMATS[codec]
    # by the path: one file name can be in two folders, with different tracks
    out = args.out / f"{abs(hash(str(f))) % 10**10}.a{s['index']}.{ext}"
    row = {
        "folder": str(f.parent),
        "file": f.name,
        "title": public_title(f.name),
        "track": s["index"],
        "codec": codec,
        "channels": s.get("channels"),
        "language": (s.get("tags") or {}).get("language"),
    }
    if not head(f, s["index"], codec, out, args.seconds):
        return row | {"error": "the head could not be written"}
    try:
        oamd = run_json(args.oadec, "oamd", out)
        emdf = run_json(args.oadec, "emdf", out)
        info = run_json(args.oadec, "info", out)
        row["bytes"] = out.stat().st_size
        row["oamd"] = {k: oamd.get(k) for k in OAMD_KEYS if k in oamd}
        row["payload_ids"] = emdf.get("payload_ids")
        row["loudness"] = emdf.get("loudness")
        row["programme_information"] = emdf.get("programme_information")
        row["dialnorm"] = dialnorm_of(codec, info)
        row["clean"] = bool(oamd.get("clean"))
    finally:
        out.unlink(missing_ok=True)
    return row


def collect(args) -> int:
    args.out.mkdir(parents=True, exist_ok=True)
    rows_path = args.out / "census.jsonl"
    done: set[tuple] = set()
    if rows_path.exists():
        with rows_path.open(encoding="utf-8") as fh:
            rows = [json.loads(line) for line in fh if line.strip()]
            done = {(r.get("folder"), r["file"]) for r in rows}

    files: list[Path] = []
    for d in args.dirs:
        walk = d.rglob("*") if args.recursive else d.iterdir()
        files += [p for p in sorted(walk) if p.is_file() and p.suffix.lower() in MEDIA]
    files = [f for f in files if (str(f.parent), f.name) not in done]
    if args.limit:
        files = files[: args.limit]
    print(f"{len(files)} files to read, {len(done)} already in the census", flush=True)

    lock = threading.Lock()
    count = [0]

    def one(f: Path) -> None:
        rows = [census_track(args, f, s) for s in probe(f)]
        with lock:
            count[0] += 1
            with rows_path.open("a", encoding="utf-8") as fh:
                # a file with no Dolby track still gets a row, so a resumed run skips it
                empty = {"folder": str(f.parent), "file": f.name, "title": public_title(f.name), "codec": None}
                for row in rows or [empty]:
                    fh.write(json.dumps(row, ensure_ascii=False) + "\n")
            print(f"[{count[0]}/{len(files)}] {len(rows)} tracks {public_title(f.name)}", flush=True)

    with ThreadPoolExecutor(args.jobs) as pool:
        list(pool.map(one, files))
    return 0


def report(path: Path, public: Path | None) -> int:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    for r in rows:
        r["title"] = public_title(r["file"])  # the rule may have improved since the row was written
    tracks = [r for r in rows if r.get("codec")]
    with_oamd = [r for r in tracks if (r.get("oamd") or {}).get("payloads")]

    def tally(field) -> dict:
        c: Counter = Counter()
        for r in with_oamd:
            value = field(r)
            if value:
                c[str(value) if not isinstance(value, bool) else "yes"] += 1
        return dict(sorted(c.items()))

    def titles(field) -> list[str]:
        return sorted({r["title"] for r in with_oamd if field(r)})

    oamd = lambda r: r.get("oamd") or {}  # noqa: E731
    payload_ids: Counter = Counter()
    for r in tracks:
        for pid in r.get("payload_ids") or {}:
            payload_ids[(r["codec"], int(pid))] += 1

    ec3 = [r for r in tracks if r["codec"] in ("eac3", "ac3")]

    def payload_totals(key: str) -> dict:
        total: Counter = Counter()
        for r in ec3:
            p = r.get(key) or {}
            for k in ("payloads", "unreadable", "size_mismatches"):
                total[k] += p.get(k) or 0
            total["tracks"] += 1 if p.get("payloads") else 0
        return dict(total)

    def by_mode(key: str, group: list) -> dict:
        c: Counter = Counter()
        for r in group:
            modes = (r.get("programme_information") or {}).get(key) or {}
            c[" ".join(sorted(modes)) or "absent"] += 1
        return dict(sorted(c.items()))

    # Clause H.3.1.3.10 ties the dialnorm to the speech-gated loudness when the
    # correction was dialogue gated, H.3.1.3.8 to the relative-gated one when it was
    # not. A wrong field or scale could not keep the first of these.
    against: Counter = Counter()
    apart_gated = []
    for r in ec3:
        loud = r.get("loudness") or {}
        if not r.get("dialnorm"):
            continue
        gated = "dialogue gated" if "yes" in (loud.get("dialogue_gated_correction") or {}) else "not dialogue gated"
        for name, key in (("speech", "speech_gated_lufs"), ("relative", "relative_gated_lufs")):
            values = loud.get(key) or {}
            if not values:
                continue
            value = float(max(values, key=values.get))
            verdict = "match" if abs(value + r["dialnorm"][0]) <= 0.5 else "apart"
            against[f"{gated}, {name} gated: {verdict}"] += 1
            if gated == "dialogue gated" and name == "speech" and verdict == "apart":
                apart_gated.append([public_title(r["file"]), r["dialnorm"][0], value])
    practices: Counter = Counter()
    for r in ec3:
        for k in (r.get("loudness") or {}).get("practices") or {}:
            practices[f"{r['codec']} {'objects' if (r.get('oamd') or {}).get('payloads') else 'channels'}: {k}"] += 1
    object_ec3 = [r for r in with_oamd if r["codec"] == "eac3"]

    summary = {
        "files": len({(r.get("folder"), r["file"]) for r in rows}),
        "loudness_payloads": payload_totals("loudness"),
        "programme_information_payloads": payload_totals("programme_information"),
        "drc_line_all_tracks": by_mode("drc_line", ec3),
        "drc_rf_all_tracks": by_mode("drc_rf", ec3),
        "drc_line_object_tracks": by_mode("drc_line", object_ec3),
        "drc_rf_object_tracks": by_mode("drc_rf", object_ec3),
        "phase_shift_90_object_tracks": by_mode("phase_shift_90", object_ec3),
        "surround_attenuation_object_tracks": by_mode("surround_attenuation", object_ec3),
        "lfe_filter_object_tracks": by_mode("lfe_filter", object_ec3),
        "loudness_against_dialnorm": dict(sorted(against.items())),
        "dialogue_gated_speech_apart_from_dialnorm": apart_gated,
        "loudness_practices_tracks": dict(sorted(practices.items())),
        "dolby_tracks": len(tracks),
        "tracks_by_codec": dict(sorted(Counter(r["codec"] for r in tracks).items())),
        "tracks_with_oamd": len(with_oamd),
        "tracks_with_oamd_by_codec": dict(sorted(Counter(r["codec"] for r in with_oamd).items())),
        "emdf_payload_ids_tracks": {f"{c} id {p}": n for (c, p), n in sorted(payload_ids.items())},
        "dialnorm_object_tracks": tally(lambda r: tuple(r.get("dialnorm") or [])),
        "with_trim_element": titles(lambda r: oamd(r).get("payloads_with_trim")),
        "with_extended_element": titles(lambda r: oamd(r).get("payloads_with_extended")),
        "with_unknown_elements": titles(lambda r: oamd(r).get("payloads_with_unknown_elements")),
        "with_additional_table_data": titles(lambda r: oamd(r).get("additional_table_data")),
        "with_screen_reference": titles(lambda r: oamd(r).get("screen_referenced")),
        "with_snap": titles(lambda r: oamd(r).get("snapped")),
        "with_object_gain": titles(lambda r: oamd(r).get("object_gains_db")),
        "with_size": titles(lambda r: oamd(r).get("sized_updates")),
        "with_alternate_object_data": titles(lambda r: oamd(r).get("alternate_object_data")),
        "bed_chan_distribute": titles(lambda r: (oamd(r).get("program") or {}).get("bed_chan_distribute")),
        "warp_modes_tracks": tally(lambda r: tuple(sorted((oamd(r).get("warp_modes") or {})))),
        "global_trim_modes_tracks": tally(lambda r: tuple(sorted((oamd(r).get("global_trim_modes") or {})))),
        "trim_configs_tracks": tally(lambda r: json.dumps(oamd(r).get("trim_configs"), sort_keys=True)
                                      if oamd(r).get("trim_configs") else None),
        "object_only_programs": sum(1 for r in with_oamd if (oamd(r).get("program") or {}).get("dyn_object_only")),
        "seconds_read_per_track": "see --seconds of the collecting run",
    }
    print(json.dumps(summary, indent=1, ensure_ascii=False))
    if public:
        view = {k: (len(v) if isinstance(v, list) and len(v) > 3 else v) for k, v in summary.items()}
        public.write_text(json.dumps(view, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("dirs", nargs="*", type=Path)
    ap.add_argument("--out", type=Path, default=Path("census"))
    ap.add_argument("--seconds", type=int, default=60)
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--recursive", action="store_true")
    ap.add_argument("--oadec", default=None)
    ap.add_argument("--report", type=Path, help="aggregate an existing census and stop")
    ap.add_argument("--public", type=Path, help="with --report, also write the aggregate here")
    args = ap.parse_args()
    if args.report:
        return report(args.report, args.public)
    args.oadec = args.oadec or str(oadec_binary())
    return collect(args)


if __name__ == "__main__":
    raise SystemExit(main())
