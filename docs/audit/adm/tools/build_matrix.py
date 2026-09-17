#!/usr/bin/env python3
"""Render docs/audit/adm-conformance-matrix.md from the evidence files.

    python build_matrix.py --repo <repository root>

The rows (feature, semantics, results, classification) are the audit's
judgements and live in ``matrix_rows.py``; every number quoted in a row is
read from the evidence JSONs at render time so that the matrix cannot drift
from the evidence.  The same rows are written as
``docs/audit/evidence/adm/adm-conformance-matrix.json``.
"""
from __future__ import annotations

import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from matrix_rows import ROWS, SCORES, VERDICTS  # noqa: E402


def load(d, name):
    p = os.path.join(d, name + ".json")
    with open(p, encoding="utf-8") as f:
        return json.load(f)


def numbers(ev: str) -> dict:
    """Headline numbers pulled from the evidence for the row texts."""
    n = {}
    batch = load(ev, "adm-batch-summary")
    timing = load(ev, "adm-event-timing")
    pcm = load(ev, "adm-object-pcm-compare")
    inputs = [k for k in timing if not k.startswith(("id", "title", "topics", "evidence_class", "structural_result", "semantic_result", "classification", "method", "inputs", "tolerance", "generated_by", "utc"))]
    n["inputs"] = len(inputs)
    n["inputs_thd"] = sum(1 for k in inputs if k.startswith("thd/") or k.startswith("pi-head50m") or k == "pi-full-film")
    n["inputs_joc"] = sum(1 for k in inputs if k.startswith("joc/"))
    n["defects_total"] = sum(len(timing[k]["defects"]) for k in inputs)
    n["matched_total"] = sum(timing[k]["classes"].get("matched", 0) for k in inputs)
    n["loss_ramp_total"] = sum(timing[k]["loss"].get("ramp", 0) for k in inputs)
    n["inexpressible_total"] = sum(timing[k]["classes"].get("inexpressible_change", 0) for k in inputs)
    n["popped_total"] = sum(timing[k]["classes"].get("trailing_popped", 0) for k in inputs)
    n["bed_events_total"] = sum(timing[k]["classes"].get("bed_event", 0) for k in inputs)
    n["bed_changes_lost_total"] = sum(timing[k]["bed_changes_lost"] for k in inputs)
    n["timecodes_total"] = sum(timing[k]["timecodes"]["timecodes"] for k in inputs)
    n["timecode_mismatch_total"] = sum(timing[k]["timecodes"]["re_encode_mismatches"] for k in inputs)
    n["tiling_gaps"] = sum(timing[k]["tiling"]["gaps"] for k in inputs)
    n["tiling_overlaps"] = sum(timing[k]["tiling"]["overlaps"] for k in inputs)
    n["tiling_overrun_objects"] = sum(timing[k]["tiling"]["overrun_objects"] for k in inputs)
    n["ends_at_frames_all"] = all(timing[k]["tiling"]["ends_at_frames"] for k in inputs)
    n["pcm_pairs"] = sum(len(pcm[k]["pairs"]) for k in inputs if k in pcm)
    n["pcm_identical"] = sum(1 for k in inputs if k in pcm for p in pcm[k]["pairs"] if p.get("identical"))
    n["matrix_confirmed_all"] = all(pcm[k]["matrix"]["all_declared_pairs_confirmed"] for k in inputs if k in pcm and pcm[k]["matrix"])
    raw = {k: v["raw_oamd_crosscheck"] for k, v in timing.items() if isinstance(v, dict) and "raw_oamd_crosscheck" in v}
    n["oamd_clips"] = len(raw)
    n["oamd_damf_times_in"] = sum(v["totals"]["damf_times_in_oamd"] for v in raw.values())
    n["oamd_damf_times_out"] = sum(v["totals"]["damf_times_not_in_oamd"] for v in raw.values())
    n["oamd_adm_times_in"] = sum(v["totals"]["adm_times_in_oamd"] for v in raw.values())
    n["oamd_adm_times_out"] = sum(v["totals"]["adm_times_not_in_oamd"] for v in raw.values())
    n["oamd_adm_blocks_on_bo32"] = sum(v["block_offset_times"]["in_adm"] for v in raw.values())
    n["oamd_pos_max"] = max(max(v["totals"]["pos_max_delta_damf"], v["totals"]["pos_max_delta_adm"]) for v in raw.values()) if raw else None
    n["oamd_bof1_updates"] = sum(v["block_offset_factor_hist"].get("1", 0) for v in raw.values())
    neg = load(ev, "adm-negative-controls")
    n["neg_total"] = len([c for c in neg["controls"] if c["fired"] is not None])
    n["neg_fired"] = sum(1 for c in neg["controls"] if c["fired"])
    n["neg_nt"] = neg["not_testable"]
    n["neg_specificity"] = list(neg["specificity_violations"])
    cont = load(ev, "adm-container-validation")
    full = cont.get("pi-full-film") or {}
    n["full_frames"] = full.get("data", {}).get("frames")
    n["full_bytes"] = full.get("file_size")
    n["full_fourcc"] = full.get("fourcc")
    n["full_ds64"] = full.get("ds64")
    n["full_ffprobe_frames"] = (full.get("ffprobe") or {}).get("frames")
    n["full_bwf_frames"] = (full.get("bwf_info") or {}).get("frames_reported")
    n["full_container_findings"] = len(full.get("container_findings", []))
    n["full_profile_findings"] = len(full.get("reference_and_profile_findings", []))
    long_ = load(ev, "adm-long-duration") if os.path.isfile(os.path.join(ev, "adm-long-duration.json")) else None
    n["full_defects"] = len(long_["ledger"]["defects"]) if long_ else None
    n["full_matched"] = long_["ledger"]["classes"].get("matched") if long_ else None
    n["full_loss_ramp"] = long_["ledger"]["loss"].get("ramp") if long_ else None
    ref = load(ev, "adm-dolby-reference")
    v = ref.get("validate") or {}
    n["validators"] = {k: {kk: x.get("exit") for kk, x in vv.items()} if "missing" not in vv else "missing" for k, vv in v.items()}
    rev = ref.get("reverse") or {}
    n["reverse_ramp_back"] = {k: x["time_matched"]["totals"]["ramp_back_hist"] for k, x in rev.items()}
    n["reverse_matched"] = {k: (x["time_matched"]["totals"]["matched"], x["time_matched"]["totals"]["ref"]) for k, x in rev.items()}
    st = ref.get("stimuli") or {}
    s1 = st.get("S1-ramps", {}).get("ct", {}).get("adm", {})
    n["s1_ct_interp_hist"] = s1.get("interp_len_samples_hist")
    s2 = st.get("S2-gain-size", {}).get("ct", {}).get("adm", {})
    n["s2_ct_gain_present"] = s2.get("gain_present")
    n["s3_ct_imp_present"] = st.get("S3-importance", {}).get("ct", {}).get("adm", {}).get("importance_present")
    har = load(ev, "adm-harness")
    n["harness_cases"] = [k for k in har if k[0] == "C"]
    n["c02_loss_gain"] = (har.get("C02-gain", {}).get("compare") or {}).get("loss", {}).get("gain")
    n["c02_ct_gain_blocks"] = sum(1 for bl in (har.get("C02-gain", {}).get("ct") or {}).get("adm_blocks", {}).values() for b in bl if b["gain"]["present"])
    n["c03_loss_importance"] = (har.get("C03-importance", {}).get("compare") or {}).get("loss", {}).get("importance")
    n["c04_damf_sizes"] = {k: [s["size"]["w"] for s in v] for k, v in (har.get("C04-size3d", {}).get("damf_states") or {}).items()}
    n["c04_sizes_text"] = "; ".join(f"object {k}: {v}" for k, v in n["c04_damf_sizes"].items())
    n["c05_bed_lost"] = (har.get("C05-bed-events", {}).get("compare") or {}).get("bed_changes_lost")
    n["c06_classes"] = (har.get("C06-late-first", {}).get("compare") or {}).get("classes")
    n["c07_tiling"] = (har.get("C07-beyond-end", {}).get("compare") or {}).get("tiling")
    n["c09_tiling"] = (har.get("C09-out-of-order", {}).get("compare") or {}).get("tiling")
    n["c11_findings"] = [f["kind"] for f in (har.get("C11-no-bed", {}).get("compare") or {}).get("adm_findings", [])]
    n["c13_tones"] = har.get("C13-isf", {}).get("adm_track_tones")
    n["c14_error"] = (har.get("C14-bed-tfl", {}).get("summary") or {}).get("adm_error")
    n["c15_findings"] = [f["kind"] for f in (har.get("C15-96k", {}).get("compare") or {}).get("adm_findings", [])]
    n["c17_findings"] = [f["kind"] for f in (har.get("C17-zones", {}).get("compare") or {}).get("adm_findings", [])]
    n["c21_tones"] = har.get("C21-bed-order", {}).get("adm_track_tones")
    rend = load(ev, "adm-render-comparison")
    n["renders"] = {name: {k: {"identical": c["all_identical"], "min_sdr_db": c["min_sdr_db"], "max_abs": c["max_abs_overall"]} for k, c in (r or {}).get("comparisons", {}).items()} for name, r in rend.items() if isinstance(r, dict)}
    ct = [c for r in n["renders"].values() for k, c in r.items() if k.startswith("oadec vs dolby-ct")]
    n["render_ct_total"] = len(ct)
    n["render_ct_identical"] = sum(1 for c in ct if c["identical"])
    n["render_scenes"] = sorted({name.split("/")[0] for name in n["renders"]})
    rr = {}
    for name, r in n["renders"].items():
        scene = name.split("/")[0]
        for k, c in r.items():
            if k.startswith("oadec vs oadec-realramps") and c["min_sdr_db"] is not None:
                rr.setdefault(scene, []).append(c["min_sdr_db"])
    n["render_realramp_sdr"] = {s: (round(min(v), 1), round(max(v), 1)) for s, v in rr.items()}
    n["render_realramp_sdr_text"] = "; ".join(f"{s}: {lo}-{hi} dB" for s, (lo, hi) in sorted(n["render_realramp_sdr"].items()))
    return n


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    a = ap.parse_args()
    ev = os.path.join(a.repo, "docs", "audit", "evidence", "adm")
    n = numbers(ev)
    rows = [dict(r, **{"note": r["note"].format(**n) if r.get("note") else ""}) for r in ROWS]
    md = ["# ADM BWF semantic conformance matrix", "",
          "Generated by `docs/audit/adm/tools/build_matrix.py` from `docs/audit/evidence/adm/*.json` on the `adm-audit` branch;",
          "do not edit by hand. Evidence classes: SPEC / REFERENCE / MEASURED / INFERRED / IMPLEMENTATION CHOICE / UNKNOWN.",
          "Results: PASS / PASS-TOL / PARTIAL / FAIL / N/I / N/T / UNK. Structural and semantic results are separate columns.", "",
          "| # | Feature | Source semantic (OAMD / DAMF) | Internal representation | DAMF | ADM (as written) | Evidence | Class | Structural | Semantic | Classification |",
          "|---|---|---|---|---|---|---|---|---|---|---|"]
    for i, r in enumerate(rows, 1):
        md.append(f"| {i} | {r['feature']} | {r['source']} | {r['internal']} | {r['damf']} | {r['adm']} | {r['evidence']} | {r['cls']} | {r['structural']} | {r['semantic']} | {r['classification']} |")
    md += ["", "## Notes per row", ""]
    for i, r in enumerate(rows, 1):
        if r["note"]:
            md.append(f"{i}. **{r['feature']}** — {r['note']}")
    md += ["", "## Scores", "", "| Score | Value | Basis |", "|---|---|---|"]
    for s in SCORES:
        md.append(f"| {s['name']} | {s['value']} | {s['basis'].format(**n)} |")
    md += ["", "## Verdicts", ""]
    for k, v in VERDICTS.items():
        md.append(f"- **{k}:** {v}")
    md.append("")
    with open(os.path.join(a.repo, "docs", "audit", "adm-conformance-matrix.md"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(md))
    with open(os.path.join(ev, "adm-conformance-matrix.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump({"rows": rows, "scores": [dict(s, basis=s["basis"].format(**n)) for s in SCORES], "verdicts": VERDICTS, "numbers": n}, f, indent=1, default=str)
        f.write("\n")
    print(f"{len(rows)} rows; numbers: {json.dumps({k: n[k] for k in ('inputs', 'defects_total', 'matched_total', 'loss_ramp_total', 'pcm_pairs', 'pcm_identical', 'neg_fired', 'neg_total')})}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
