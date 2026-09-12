#!/usr/bin/env python3
"""Decode a list of streams to DAMF and ADM under provenance, then compare each pair.

    python run_batch_decode.py --oadec <exe> --out-root <dir> --inputs a.thd b.ec3 ... [--nbc] [--pcm full|window]

For every input: <out-root>/<stem>/default/<stem>.{atmos,...,wav} (+ nbc/ when
asked), one ``.<format>.run.json`` per decode and ``compare.json`` per pair.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from run_compare import compare  # noqa: E402


def decode(py: str, oadec: str, src: str, base: str, fmt: str, nbc: bool) -> dict:
    argv = [py, os.path.join(HERE, "run_decode.py"), "--oadec", oadec, "--input", src, "--out", base, "--format", fmt]
    if nbc:
        argv.append("--no-bed-conform")
    p = subprocess.run(argv, capture_output=True, text=True, errors="replace")
    rec_path = f"{base}.{fmt}.run.json"
    rec = json.load(open(rec_path, encoding="utf-8")) if os.path.isfile(rec_path) else None
    return {"exit": p.returncode, "stdout": p.stdout.strip()[-300:], "record": rec_path, "oadec_exit": rec["run"]["exit_code"] if rec else None, "seconds": rec["run"]["seconds"] if rec else None}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--oadec", required=True)
    ap.add_argument("--out-root", required=True)
    ap.add_argument("--inputs", nargs="+", required=True)
    ap.add_argument("--nbc", action="store_true", help="also decode with --no-bed-conform")
    ap.add_argument("--pcm", default="full", choices=["full", "window", "none"])
    ap.add_argument("--skip-existing", action="store_true")
    a = ap.parse_args()
    py = sys.executable
    summary = {}
    for src in a.inputs:
        stem = os.path.splitext(os.path.basename(src))[0]
        entry = {"input": src}
        for variant, nbc in (("default", False), ("nbc", True)):
            if nbc and not a.nbc:
                continue
            d = os.path.join(a.out_root, stem, variant)
            os.makedirs(d, exist_ok=True)
            base = os.path.join(d, stem)
            v = {}
            for fmt in ("damf", "adm"):
                marker = base + (".atmos.metadata" if fmt == "damf" else ".wav")
                if a.skip_existing and os.path.isfile(marker) and os.path.isfile(f"{base}.{fmt}.run.json"):
                    v[fmt] = {"skipped": True}
                    continue
                v[fmt] = decode(py, a.oadec, src, base, fmt, nbc)
                print(f"{stem} {variant} {fmt}: oadec exit {v[fmt].get('oadec_exit')} in {v[fmt].get('seconds')}s", flush=True)
            try:
                rep = compare(base, base + ".wav", 0.0, "effective", a.pcm, 20.0, f"{stem}/{variant}")
                with open(os.path.join(d, "compare.json"), "w", encoding="utf-8", newline="\n") as f:
                    json.dump(rep, f, indent=1, default=lambda o: sorted(o) if isinstance(o, set) else str(o))
                    f.write("\n")
                L = rep["ledger"]
                v["compare"] = {"classes": L["classes"], "defects": len(L["defects"]), "loss": L["loss"], "identity_ok": L["identity_ok"],
                                "tiling": {k: L["tiling"][k] for k in ("gaps", "overlaps", "starts_at_zero", "ends_at_frames", "overrun_objects", "blocks_shorter_than_interp")},
                                "pcm_identical": sum(1 for p in rep["pcm"]["pairs"] if p.get("identical")), "pcm_pairs": len(rep["pcm"]["pairs"]),
                                "matrix_confirmed": rep["pcm"]["matrix"]["all_declared_pairs_confirmed"] if rep["pcm"]["matrix"] else None,
                                "timecodes": rep["timecode_roundtrip"], "adm_findings": sorted({f["kind"] for f in rep["adm"]["findings"]}),
                                "damf_findings": sorted({f["kind"] for f in rep["damf"]["findings"]}),
                                "max_e": {k: max(x["max_e"].values()) for k, x in rep["trajectory_loss"].items()},
                                "frames": rep["adm"]["source"]["frames"], "channels": rep["adm"]["source"]["channels"], "sample_rate": rep["adm"]["source"]["sample_rate"]}
                print(f"{stem} {variant} compare: defects {v['compare']['defects']} classes {dict(L['classes'])} pcm {v['compare']['pcm_identical']}/{v['compare']['pcm_pairs']} findings adm={v['compare']['adm_findings']} damf={v['compare']['damf_findings']}", flush=True)
            except Exception as e:  # noqa: BLE001
                v["compare"] = {"error": f"{type(e).__name__}: {e}"}
                print(f"{stem} {variant} compare FAILED: {e}", flush=True)
            entry[variant] = v
        summary[stem] = entry
        with open(os.path.join(a.out_root, "batch-summary.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(summary, f, indent=1, default=str)
    return 0


if __name__ == "__main__":
    sys.exit(main())
