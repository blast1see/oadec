#!/usr/bin/env python3
"""Compare oadec's JOC objects against Dolby's, title by title, and against the
figures the audit recorded.

E-AC-3 JOC objects are reconstructed parametrically, so there is no bit-exact
answer to check. What there is: a per-object distance to a reference decoder,
and the inter-object correlation structure, which is the number that says the
objects are the ones the stream describes rather than fifteen filtered copies of
a downmix. Both have to hold after a change to the substream layer, and the way
to know is to re-measure and compare with what the audit stored.

    python tools/joc_object_gate.py --work E:/oadec-work --out gate.json

The JSON is the record and the exit code is the verdict: 1 when a title
regressed, 3 when one could not be measured, 0 otherwise.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import objcmp  # noqa: E402

import sys as _sys
from pathlib import Path as _Path

_sys.path.insert(0, str(_Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402

# clip -> Dolby object dump, and the name the audit filed its result under
TITLES = [
    ("clips/disclosure-web-head.ec3", "ref-oar/disclosure-web-obj.f32", "disclosure-web-head"),
    ("clips/kingsman-joc-head.ec3", "ref-oar/kingsman-joc-head-obj.f32", "kingsman-joc-head"),
    ("clips/knivesout-joc-head.ec3", "ref-oar/knivesout-joc-obj.f32", "knivesout-joc-head"),
    ("clips/extraction-nf-head.ec3", "ref-oar/extraction-nf-head-obj.f32", None),
    ("clips/glassonion-nf-head.ec3", "ref-oar/glassonion-nf-obj.f32", None),
    ("clips/br2049-joc-head.ec3", "ref-oar/br2049-obj.f32", None),
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--binary", default=find_oadec(required=False))
    ap.add_argument("--baseline",
                    default=str(Path(__file__).resolve().parent.parent
                                / "docs/audit/evidence/06-11-joc-objects-vs-dolby.json"))
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    work = Path(a.work)
    tmp = work / "audit/remediation/tmp/objgate"
    tmp.mkdir(parents=True, exist_ok=True)

    baseline = {}
    try:
        for row in json.load(open(a.baseline)):
            baseline[row["name"]] = row
    except (OSError, ValueError):
        pass

    rows = []
    for clip, dolby, audit_name in TITLES:
        src, ref = work / clip, work / dolby
        name = Path(clip).stem
        if not src.exists() or not ref.exists():
            rows.append({"name": name, "verdict": "NOT RUN",
                         "detail": f"missing {src if not src.exists() else ref}"})
            continue
        print(f"-- {name}", flush=True)
        base = tmp / name
        # `--no-bed-conform` writes the coded bed rather than a conformed 7.1.2
        # one, which is what Dolby's object decoder emits.
        res = subprocess.run(
            [a.binary, "decode", "--format", "damf", "--no-bed-conform", "-o", str(base), str(src)],
            capture_output=True, text=True, timeout=3600)
        audio = base.with_suffix(base.suffix + ".atmos.audio")
        if not audio.exists():
            rows.append({"name": name, "verdict": "NOT RUN",
                         "detail": res.stderr.strip()[-400:]})
            continue
        row = objcmp.run(name, str(audio), str(ref))
        row["decode_exit"] = res.returncode
        was = baseline.get(audit_name or "")
        if was:
            row["audit"] = {k: was[k] for k in
                            ("lag_samples", "objects_worst_sdr_db", "objects_median_sdr_db",
                             "corr_structure_max_abs_delta")}
            worst_drop = (was["objects_worst_sdr_db"] or 0) - (row["objects_worst_sdr_db"] or 0)
            median_drop = (was["objects_median_sdr_db"] or 0) - (row["objects_median_sdr_db"] or 0)
            row["worst_sdr_change_db"] = round(-worst_drop, 2)
            row["median_sdr_change_db"] = round(-median_drop, 2)
            row["verdict"] = ("PASS" if worst_drop <= 0.5 and median_drop <= 0.5
                              else "REGRESSED")
        else:
            row["verdict"] = "HELD OUT"
        rows.append(row)
        for ext in (".atmos", ".atmos.metadata", ".atmos.audio"):
            Path(str(base) + ext).unlink(missing_ok=True)
        json.dump({"titles": rows}, open(a.out, "w"), indent=1)

    measured = [r for r in rows if r.get("objects_worst_sdr_db") is not None]
    summary = {
        "titles": len(rows),
        "measured": len(measured),
        "regressed": sum(1 for r in rows if r.get("verdict") == "REGRESSED"),
        "held_out": sum(1 for r in rows if r.get("verdict") == "HELD OUT"),
        "worst_sdr_db": min((r["objects_worst_sdr_db"] for r in measured), default=None),
        "worst_correlation_structure_delta":
            max((r["corr_structure_max_abs_delta"] for r in measured), default=None),
        "lags": sorted({r["lag_samples"] for r in measured}),
    }
    json.dump({"summary": summary, "titles": rows}, open(a.out, "w"), indent=1)
    print(json.dumps(summary, indent=1))
    if summary["regressed"]:
        return 1
    if any(r.get("verdict") == "NOT RUN" for r in rows):
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
