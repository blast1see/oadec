#!/usr/bin/env python3
"""Does any real TrueHD Atmos stream carry an object gain or an object size?

Both fields are in the Object Audio Metadata syntax, both are parsed, and
neither has ever been seen. That is not a coincidence to shrug at: Dolby's
TrueHD and Digital Plus encoders both **drop** the two fields, so nothing
authored through either can carry one, and the parse of a non-zero value rests
on hand-built payloads alone. The only thing that can answer the question is a
stream someone else made, which means counting over a library rather than over a
corpus.

    python tools/thd_gain_size.py --out gainsize.json E:/oadec-work/thd E:/oadec-work/thdsweep

Each argument is a directory of `.thd` elementary streams or one such file.
`oadec oamd --json` reports the counts per stream; this sums them and names the
streams that carry anything, with the access units to cut a clip at.

**A head clip cannot prove absence.** The twenty-second cuts say what the first
twenty seconds of many films use; whole streams say what a whole film uses. Both
are reported, separately, because merging them would let breadth pass itself off
as depth.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402


def scan(binary: str, path: Path) -> dict | None:
    res = subprocess.run([binary, "oamd", "--json", str(path)],
                         capture_output=True, text=True, timeout=7200)
    try:
        d = json.loads(res.stdout)
    except json.JSONDecodeError:
        return None
    return {
        "stream": path.name,
        "bytes": path.stat().st_size,
        "units": d.get("units"),
        "units_with_oamd": d.get("units_with_oamd"),
        "payloads": d.get("payloads"),
        "parse_errors": d.get("parse_errors"),
        "object_gains_db": d.get("object_gains_db") or {},
        "muted_updates": d.get("muted_updates") or 0,
        "sized_updates": d.get("sized_updates") or 0,
        "first_gain_units": d.get("first_gain_units") or [],
        "first_size_units": d.get("first_size_units") or [],
        "inactive_updates": d.get("inactive_updates") or 0,
    }


def summarise(rows: list[dict]) -> dict:
    gains: Counter[str] = Counter()
    for r in rows:
        gains.update({k: v for k, v in r["object_gains_db"].items()})
    return {
        "streams": len(rows),
        "streams_with_oamd": sum(1 for r in rows if r["payloads"]),
        "payloads": sum(r["payloads"] or 0 for r in rows),
        "object_gains_db": dict(gains),
        "non_unity_gains": sum(gains.values()),
        "muted_updates": sum(r["muted_updates"] for r in rows),
        "sized_updates": sum(r["sized_updates"] for r in rows),
        "inactive_updates": sum(r["inactive_updates"] for r in rows),
        "streams_with_gain": [r["stream"] for r in rows if r["object_gains_db"]],
        "streams_with_size": [r["stream"] for r in rows if r["sized_updates"]],
        "parse_errors": sum(r["parse_errors"] or 0 for r in rows),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("paths", nargs="+")
    ap.add_argument("--binary", default=find_oadec(required=False))
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    if not a.binary:
        sys.exit("no oadec binary; pass --binary")

    groups: dict[str, list[Path]] = {}
    for arg in a.paths:
        p = Path(arg)
        files = sorted(p.glob("*.thd")) if p.is_dir() else [p]
        groups[p.name or str(p)] = files

    out: dict[str, dict] = {}
    rows_by_group: dict[str, list[dict]] = {}
    for name, files in groups.items():
        rows = []
        for i, f in enumerate(files, 1):
            row = scan(a.binary, f)
            if row is None:
                print(f"  {f.name}: no JSON", flush=True)
                continue
            rows.append(row)
            if row["object_gains_db"] or row["sized_updates"]:
                print(f"  !! {f.name}: gains {row['object_gains_db']}, "
                      f"sizes {row['sized_updates']}", flush=True)
            if i % 25 == 0:
                print(f"  {name}: {i}/{len(files)}", flush=True)
        rows_by_group[name] = rows
        out[name] = summarise(rows)
        print(f"{name}: {json.dumps(out[name])}", flush=True)

    json.dump({"groups": out, "per_stream": rows_by_group},
              open(a.out, "w", encoding="utf-8"), indent=1, ensure_ascii=False)
    print(f"wrote {a.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
