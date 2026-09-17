#!/usr/bin/env python3
"""Assemble the machine-readable evidence under docs/audit/evidence/adm/ from the work directory.

    python build_evidence.py --work <work directory> --repo <repository root> [--out docs/audit/evidence/adm]

Every file is an evidence envelope (see admaudit.evidence) whose payload is
taken verbatim from the measurement JSONs written by the run_* drivers; this
script selects and groups, it does not measure.  Results and classifications
recorded here are the audit's judgements and are repeated in the matrix.
"""
from __future__ import annotations

import argparse
import glob
import json
import os
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import dolby, evidence, provenance  # noqa: E402

SPECS = r"E:\oadec-work\specs"
PROFILE_TXT = os.path.join(SPECS, "dolby_atmos_master_adm_profile_v1.0.txt")
BS2076_TXT = os.path.join(SPECS, "itu_bs2076-3.txt")
TS103420_TXT = os.path.join(SPECS, "ts_103420v010201p.txt")
EBU3306_TXT = os.path.join(SPECS, "ebu_tech3306v1_1.txt")


def load(path):
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def lines(path, a, b):
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8", errors="replace") as f:
        ls = f.read().splitlines()
    return "\n".join(ls[a - 1 : b])


def inp(*paths):
    out = []
    for p in paths:
        if p and os.path.isfile(p):
            out.append(provenance.file_record(p))
    return out


def ledger_summary(cmp_: dict) -> dict:
    L = cmp_["ledger"]
    return {
        "classes": L["classes"], "defects": L["defects"], "loss": L["loss"], "identity_ok": L["identity_ok"],
        "tiling": L["tiling"], "bed_changes_lost": L["bed_changes_lost"], "time_lost_events": L.get("time_lost_events", 0),
        "max_matched_pos_delta": L.get("max_matched_pos_delta"), "timecodes": cmp_["timecode_roundtrip"],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--repo", required=True)
    ap.add_argument("--out", default=None)
    a = ap.parse_args()
    W = a.work.replace("\\", "/")
    out = a.out or os.path.join(a.repo, "docs", "audit", "evidence", "adm")
    os.makedirs(out, exist_ok=True)
    gen = "docs/audit/adm/tools/build_evidence.py"

    # ---------------------------------------------------------------- 00 provenance
    py = sys.executable
    freeze = subprocess.run([py, "-m", "pip", "freeze"], capture_output=True, text=True).stdout.splitlines()
    tools = {}
    for name, path, args in (
        ("conversion_tool", dolby.CONVERSION_TOOL, ["--version"]), ("dee", dolby.DEE, None),
        ("atmos_info_5.7.2", dolby.ATMOS_INFO["5.7.2"], ["-v", "1"]), ("atmos_info_1.1", dolby.ATMOS_INFO["1.1"], ["--version"]),
        ("bwf_info", dolby.BWF_INFO, ["--help"]), ("ear_render", os.path.join(os.path.dirname(py), "ear-render.exe"), None),
        ("truehdd", shutil.which("truehdd") or "truehdd", ["--version"]),
    ):
        try:
            tools[name] = provenance.tool_record(path, args) if os.path.isfile(path) else {"missing": path}
        except Exception as e:  # noqa: BLE001
            tools[name] = {"path": path, "error": str(e)}
    ff = subprocess.run(["ffmpeg", "-version"], capture_output=True, text=True).stdout.splitlines()[:1]
    binaries = {}
    for rec_path in glob.glob(f"{W}/work/**/*.run.json", recursive=True) + glob.glob(f"{W}/big/**/*.run.json", recursive=True):
        d = load(rec_path)
        if d and "binary" in d:
            binaries[d["binary"]["sha256"]] = d["binary"]
    evidence.write(out, "00-provenance", {
        "git": provenance.git_record(a.repo),
        "oadec_binaries_used": list(binaries.values()),
        "note_on_binaries": "the release binary on disk before the audit (sha256 f5aae50b...) predated the merge commit 796e022 and was rebuilt from HEAD before any decode; every artefact records the sha256 of the binary that made it",
        "tools": tools, "ffmpeg": ff, "python": provenance.env_record(), "pip_freeze": freeze,
        "specs": inp(PROFILE_TXT, os.path.join(SPECS, "dolby_atmos_master_adm_profile_v1.0.pdf"), os.path.join(SPECS, "itu_bs2076-3.pdf"), os.path.join(SPECS, "ts_103420v010201p.pdf"), os.path.join(SPECS, "ebu_tech3285s7.pdf"), os.path.join(SPECS, "ebu_tech3306v1_1.pdf")),
        "spec_gaps": ["ITU-R BS.2088 (BW64) not on disk and not downloadable from this machine; RF64 structure taken from EBU Tech 3306", "Dolby Atmos Master ADM Profile v1.1 not on disk; v1.0 (22 July 2019) used"],
    }, title="Provenance of the ADM audit", topics=["provenance"], evidence_class="MEASURED", structural_result=None, semantic_result=None,
        classification="record", method="git, sha256, --version, pip freeze", inputs=[], generated_by=gen)

    # ---------------------------------------------------------------- 01 spec extracts
    evidence.write(out, "01-spec-extracts", {
        "dolby_profile_v1_0": {
            "file": PROFILE_TXT,
            "table_9_10_directspeakers_blocks": lines(PROFILE_TXT, 241, 280),
            "table_11_objects_blocks": lines(PROFILE_TXT, 281, 342),
            "section_2_5_1_jump_position": lines(PROFILE_TXT, 345, 375),
            "table_5_stream_format_note": lines(PROFILE_TXT, 153, 166),
            "table_16_bed_sets": lines(PROFILE_TXT, 433, 445),
            "table_17_audio_object": lines(PROFILE_TXT, 452, 482),
            "table_21_programme": lines(PROFILE_TXT, 536, 549),
            "table_23_track_uid": lines(PROFILE_TXT, 569, 579),
            "general_limits": lines(PROFILE_TXT, 86, 97),
        },
        "bs2076_3": {"file": BS2076_TXT, "section_9_3": lines(BS2076_TXT, 3218, 3233), "figures_a1_6_to_a1_9": lines(BS2076_TXT, 3235, 3280), "block_format_table_rows": lines(BS2076_TXT, 795, 832)},
        "ts_103420": {"file": TS103420_TXT, "clause_5_6_2_ramp_and_timing": lines(TS103420_TXT, 1100, 1122), "clause_5_6_2_6_to_8_ramp_tables": lines(TS103420_TXT, 2141, 2175)},
        "ebu_tech_3306": {"file": EBU3306_TXT, "ds64_rules": lines(EBU3306_TXT, 233, 285)},
    }, title="Verbatim specification extracts the audit rests on", topics=["spec"], evidence_class="SPEC", structural_result=None, semantic_result=None,
        classification="record", method="pdftotext -layout; line ranges quoted verbatim", inputs=inp(PROFILE_TXT, BS2076_TXT, TS103420_TXT, EBU3306_TXT), generated_by=gen)

    # ---------------------------------------------------------------- batch results (TrueHD, JOC, pi-head50m)
    batches = {}
    per_input = {}
    for kind in ("thd", "joc"):
        b = load(f"{W}/work/{kind}/batch-summary.json")
        if b:
            batches[kind] = b
    for stem in ("pi-head50m",):
        for variant in ("default", "nbc"):
            c = load(f"{W}/work/{stem}/{variant}/compare.json")
            if c:
                per_input[f"{stem}/{variant}"] = c
    for kind in ("thd", "joc"):
        for c in glob.glob(f"{W}/work/{kind}/*/default/compare.json"):
            stem = c.replace("\\", "/").split("/")[-3]
            per_input[f"{kind}/{stem}"] = load(c)
    full = load(f"{W}/big/pi/compare.json")
    if full:
        per_input["pi-full-film"] = full

    def rows(fn):
        return {k: fn(v) for k, v in per_input.items()}

    evidence.write(out, "adm-batch-summary", {"batches": batches, "inputs_analysed": sorted(per_input)},
                   title="Decode and compare summary for every TrueHD, E-AC-3 JOC and authored input", topics=["batch"], evidence_class="MEASURED",
                   structural_result="PASS", semantic_result="PARTIAL", classification="real material: 0 ledger defects, PCM identical; ramp/bed/importance losses counted separately",
                   method="run_batch_decode.py -> run_decode.py + run_compare.py", inputs=inp(f"{W}/work/thd/batch-summary.json", f"{W}/work/joc/batch-summary.json"), generated_by=gen)

    evidence.write(out, "adm-object-pcm-compare", rows(lambda c: {"source": {k: c["adm"]["source"].get(k) for k in ("frames", "channels", "sample_rate", "fourcc")},
                                                                  "pairs": [{k: p.get(k) for k in ("role", "label", "damf_track", "adm_track", "identical", "differing", "first_diff", "max_abs", "samples_a", "samples_b", "sha256_a", "sha256_b", "silence_pct_a", "peak_a", "rms_a", "corr")} for p in c["pcm"]["pairs"]],
                                                                  "matrix": {k: c["pcm"]["matrix"].get(k) for k in ("window_samples", "all_declared_pairs_confirmed", "non_silent_pairs_unique", "silent_adm_tracks")} if c["pcm"]["matrix"] else None}),
                   title="Per-track PCM identity between DAMF audio and ADM data, with the declared pairing verified against the full pairing matrix", topics=["pcm", "mapping"],
                   evidence_class="MEASURED", structural_result="PASS", semantic_result="PASS", classification="sample-exact",
                   method="compare_pcm.compare_interleaved / compare_tracks; distance_matrix over a 20 s window", inputs=[], generated_by=gen)

    evidence.write(out, "adm-track-mapping", rows(lambda c: {"tracks": c["adm"]["tracks"], "beds": c["adm"]["beds"], "objects": c["adm"]["objects"], "damf_beds": c["damf"]["beds"], "damf_objects": c["damf"]["objects"]}),
                   title="chna -> audioTrackUID -> audioTrackFormat -> audioStreamFormat -> audioChannelFormat chain per PCM track, against the DAMF declaration", topics=["mapping", "chna"],
                   evidence_class="MEASURED", structural_result="PASS", semantic_result="PASS", classification="declaration-derived, PCM-confirmed",
                   method="axml.track_chain + normalise.layout", inputs=[], generated_by=gen)

    evidence.write(out, "adm-reference-graph", rows(lambda c: {"adm_findings": c["adm"]["findings"], "damf_findings": c["damf"]["findings"], "element_counts": c["adm"]["source"].get("element_counts"), "chunks": c["adm"]["source"].get("chunks"), "dbmd_creator": c["adm"]["source"].get("dbmd_creator")}),
                   title="Reference-graph, chna and Dolby-profile rule findings for every ADM file (empty lists = no finding)", topics=["axml", "chna", "profile"],
                   evidence_class="MEASURED", structural_result="PASS", semantic_result=None, classification="structural",
                   method="axml.check_references + axml.check_chna + profile.check on every produced ADM", inputs=[], generated_by=gen)

    timing = rows(ledger_summary)
    for c in glob.glob(f"{W}/work/thd/*/default/oamd-timing.json") + [f"{W}/work/pi-head50m/default/oamd-timing.json"]:
        d = load(c)
        if d:
            stem = c.replace("\\", "/").split("/")[-3]
            key = next((k for k in timing if k.endswith("/" + stem) or k.startswith(stem)), stem)
            timing.setdefault(key, {})["raw_oamd_crosscheck"] = {k: d[k] for k in ("units", "sample_offset_hist", "block_offset_factor_hist", "ramp_hist", "totals", "block_offset_times", "coded_bed_objects", "pos_ok", "coordinate_map")}
    evidence.write(out, "adm-event-timing", timing,
                   title="Event timing: DAMF samplePos vs ADM rtime (integer samples), block tiling, timecode re-encoding, and raw OAMD update times including 32 x block_offset_factor", topics=["timing", "block-offset", "tiling"],
                   evidence_class="MEASURED", structural_result="PASS", semantic_result="PASS", classification="sample-exact",
                   method="compare_events.reconcile + run_oamd_timing.py", inputs=[], generated_by=gen)

    evidence.write(out, "adm-coordinate-diff", rows(lambda c: {"value_mismatches": [d for d in c["ledger"]["defects"] if d["cls"] == "value_mismatch"], "max_matched_pos_delta_f32": c["ledger"].get("max_matched_pos_delta"),
                                                               "trajectory_loss": c["trajectory_loss"], "selfcheck_max_e": c["trajectory_selfcheck_max_e"], "displacements": c["displacements"]}),
                   title="Coordinates: DAMF vs ADM position equality at float32 precision, and the metadata-domain trajectory deviation caused by the fixed interpolation length", topics=["coordinates", "trajectory"],
                   evidence_class="MEASURED", structural_result=None, semantic_result="PASS-TOL", classification="positions exact; trajectory loss is the profile's fixed ramp",
                   method="compare_events.expressible_diff (float32) + trajectory.loss", inputs=[], generated_by=gen)

    evidence.write(out, "adm-interpolation-diff", rows(lambda c: {"loss_ramp": c["ledger"]["loss"].get("ramp", 0), "ramp_hist": {k: v["ramp_hist"] for k, v in c["displacements"].items()}, "gap_hist": {k: v["gap_hist"] for k, v in c["displacements"].items()},
                                                                  "trajectory_max_e": {k: v["max_e"] for k, v in c["trajectory_loss"].items()}, "trajectory_mean_e": {k: v["mean_e"] for k, v in c["trajectory_loss"].items()},
                                                                  "tiling_blocks_shorter_than_interp": c["ledger"]["tiling"].get("blocks_shorter_than_interp")}),
                   title="Interpolation: DAMF rampLength vs ADM interpolationLength per block, and the resulting trajectory deviation", topics=["interpolation"],
                   evidence_class="MEASURED", structural_result="PASS", semantic_result="PASS-TOL", classification="profile rule (table 11): 0 then 250 samples; loss measured",
                   method="compare_events (loss.ramp) + trajectory.loss", inputs=[], generated_by=gen)

    # ---------------------------------------------------------------- gain / importance / size / bed
    harness = load(f"{W}/work/harness/harness-report.json") or {}
    f5 = load(f"{W}/work/f5/f5-report.json") or {}
    wild = {}
    for k, c in per_input.items():
        wild[k] = {"adm_gain_present_blocks": None, "damf_gain_values": None}
    evidence.write(out, "adm-gain-diff", {
        "harness": {k: {kk: harness[k].get(kk) for kk in ("why", "expect", "compare", "adm_blocks", "damf_states", "damf_bed_events", "ct")} for k in ("C02-gain", "C03-importance", "C05-bed-events", "C16-active-toggle") if k in harness},
        "dolby_reference": {k: f5.get("stimuli", {}).get(k) for k in ("S2-gain-size", "S3-importance", "S5-inactive", "S8-bed-gain")},
        "real_material": "no real stream in the corpus carries an object gain other than 0 dB, an importance other than 1.0 or a non-zero size (see docs/audit/evidence/remediation/object-gain-and-size.json); on real material the ADM omits gain/importance exactly as the profile requires",
    }, title="Gain, importance and bed events: what DAMF carries, what oadec's ADM carries, what Dolby's converters carry", topics=["gain", "importance", "bed"],
        evidence_class="MEASURED", structural_result="PASS", semantic_result="PARTIAL", classification="gain: oadec drops what Dolby keeps (avoidable loss); importance and bed events: dropped by both (profile)",
        method="run_harness.py (+ Conversion Tool cross-reference) and run_dolby_reference.py", inputs=inp(f"{W}/work/harness/harness-report.json", f"{W}/work/f5/f5-report.json"), generated_by=gen)

    evidence.write(out, "adm-harness", {k: {kk: v.get(kk) for kk in ("why", "expect", "exit", "summary", "compare", "adm_blocks", "damf_states", "damf_bed_events", "adm_track_tones", "adm_track_labels", "ct", "compare_error")} for k, v in harness.items()},
                   title="Writer-level cases: the same controlled events through AdmWriter and DamfWriter (and through Dolby's converter)", topics=["harness", "robustness"],
                   evidence_class="MEASURED", structural_result=None, semantic_result=None, classification="per case",
                   method="docs/audit/adm/harness (Rust, path dependency on oadec-spatial) + run_harness.py", inputs=inp(f"{W}/work/harness/harness-report.json"), generated_by=gen)

    evidence.write(out, "adm-dolby-reference", {"stimuli": {k: {"what": v["what"], "elements": v["elements"], "ct": _trim_tool(v.get("ct")), "dee": _trim_tool(v.get("dee"))} for k, v in f5.get("stimuli", {}).items()},
                                                "reverse": f5.get("reverse"), "validate": f5.get("validate")},
                   title="Dolby reference leg: Conversion Tool and DEE DAMF->ADM on controlled stimuli, ADM->DAMF read-back, atmos_info and bwf_info verdicts", topics=["reference", "dolby"],
                   evidence_class="REFERENCE", structural_result=None, semantic_result=None, classification="calibration of the Dolby chain",
                   method="run_dolby_reference.py", inputs=inp(f"{W}/work/f5/f5-report.json"), generated_by=gen)

    neg = load(f"{W}/work/negative/negative-controls.json")
    if neg:
        evidence.write(out, "adm-negative-controls", neg, title="Negative controls: injected defects and the detectors that fired", topics=["negative-controls"],
                       evidence_class="MEASURED", structural_result=None, semantic_result=None, classification="tool validation",
                       method="run_negative_controls.py on the pi-head50m ADM/DAMF pair", inputs=inp(f"{W}/work/negative/negative-controls.json"), generated_by=gen)

    cont = {"pi-full-film": load(f"{W}/big/pi/container.json"), "C22-rf64": load(f"{W}/work/harness/out/C22-rf64/container.json"), "C22b-riff-max": load(f"{W}/work/harness/out/C22b-riff-max/container.json"),
            "batch_adm_findings": rows(lambda c: {"container_and_reference_findings": c["adm"]["findings"], "fourcc": c["adm"]["source"]["fourcc"], "chunks": c["adm"]["source"]["chunks"], "format_tag": c["adm"]["source"].get("format_tag")})}
    for k in ("pi-full-film", "C22-rf64", "C22b-riff-max"):
        if cont[k]:
            for tool in ("bwf_info", "atmos_info_5.7.2", "atmos_info_1.1", "ffprobe"):
                if tool in cont[k] and isinstance(cont[k][tool], dict) and "run" in cont[k][tool]:
                    cont[k][tool]["run"] = {kk: cont[k][tool]["run"].get(kk) for kk in ("argv", "exit_code", "seconds", "stdout_tail")}
    evidence.write(out, "adm-container-validation", cont, title="BW64/RF64 container validation: our walker, ffprobe, bwf_info, atmos_info; > 4 GiB files and the two size thresholds", topics=["container", "rf64"],
                   evidence_class="MEASURED", structural_result="PASS", semantic_result=None, classification="RF64 FourCC (not BW64) noted",
                   method="run_container.py", inputs=inp(f"{W}/big/pi/container.json"), generated_by=gen)

    renders = {os.path.relpath(p, f"{W}/work/render").replace(os.sep, "/"): load(p) for p in glob.glob(f"{W}/work/render/*/render-report*.json")}
    for r in renders.values():
        if r:
            for k, v in r.get("renders", {}).items():
                v.pop("stderr_tail", None)
    evidence.write(out, "adm-render-comparison", renders, title="EBU ADM Renderer (BS.2127) speaker outputs: oadec ADM vs Dolby ADM vs a real-ramp variant", topics=["render"],
                   evidence_class="REFERENCE", structural_result=None, semantic_result=None, classification="renderer equality is supporting evidence only",
                   method="run_render.py (audioStreamFormat pack reference removed identically for EAR)", inputs=[], generated_by=gen)

    if full:
        big = {"frames": full["adm"]["source"]["frames"], "channels": full["adm"]["source"]["channels"], "ledger": ledger_summary(full), "adm_run": load(f"{W}/big/pi/pi.adm.run.json"), "damf_run": load(f"{W}/big/pi/pi.damf.run.json"),
               "container": {k: (load(f"{W}/big/pi/container.json") or {}).get(k) for k in ("fourcc", "ds64", "data", "programme_span_samples", "u32_overflow")},
               "first_last_events": {k: {"first": min(v["max_e"] and [0] or [0]), } for k, v in {}.items()}}
        # first and last event per object from the ledger items
        items = full["ledger"]["items"]
        per = {}
        for it in items:
            if it["cls"] in ("matched", "inexpressible_change") and it["t_damf"] is not None:
                o = per.setdefault(it["obj"], {"first": None, "last": None, "n": 0})
                o["first"] = it["t_damf"] if o["first"] is None else min(o["first"], it["t_damf"])
                o["last"] = it["t_damf"] if o["last"] is None else max(o["last"], it["t_damf"])
                o["n"] += 1
        big["first_last_events"] = per
        big["drift"] = "every matched ADM block rtime equals its DAMF samplePos exactly (time_mismatch 0), so the drift between first and last event is 0 samples"
        for rr in ("adm_run", "damf_run"):
            if big[rr]:
                big[rr] = {"exit": big[rr]["run"]["exit_code"], "seconds": big[rr]["run"]["seconds"], "stderr_tail": big[rr]["run"]["stderr_tail"][-1200:], "outputs": big[rr]["outputs"], "binary": big[rr]["binary"]["sha256"]}
        evidence.write(out, "adm-long-duration", big, title="Whole-film Pi (1:24:08, 242 322 080 frames): ledger, timing drift, RF64 container", topics=["long-duration", "rf64"],
                       evidence_class="MEASURED", structural_result="PASS", semantic_result="PASS", classification="no drift; identical PCM",
                       method="run_decode.py (whole film) + run_compare.py --pcm full + run_container.py", inputs=inp(f"{W}/big/pi/compare.json", f"{W}/big/pi/container.json"), generated_by=gen)

    # ---------------------------------------------------------------- work-directory inventory
    # Written before the reproducible work tree is deleted: every file's size and
    # SHA-256, every small record verbatim (run records, validator logs, DEE job
    # files, DAMF headers, harness summaries) and, for the large derived JSONs,
    # every hash and command line they contain.
    inv = work_inventory(W)
    evidence.write(out, "adm-work-inventory", inv, title="Inventory of the audit work directory before deletion: hashes of every file, run records and job files verbatim, hashes and command lines extracted from the large derived reports",
                   topics=["provenance"], evidence_class="MEASURED", structural_result=None, semantic_result=None, classification="record",
                   method="sha256 of every file under work/ and big/; JSON/XML/atmos/txt/log files up to 64 KB embedded; larger JSON reduced to their sha256 strings and argv lists", inputs=[], generated_by=gen)

    m = evidence.manifest(out)
    print(f"wrote {len(m['files'])} evidence files to {out}")
    return 0




def work_inventory(W: str) -> dict:
    import hashlib
    import re
    sha_re = re.compile(r"\b[0-9a-f]{64}\b")
    embed_ext = (".json", ".xml", ".atmos", ".metadata", ".txt", ".log", ".md", ".py")
    files = []
    totals = {"files": 0, "bytes": 0, "embedded": 0, "reduced": 0}
    for root_name in ("work", "big"):
        root = os.path.join(W, root_name)
        if not os.path.isdir(root):
            continue
        for dp, dn, fn in os.walk(root):
            for f in sorted(fn):
                p = os.path.join(dp, f)
                rel = os.path.relpath(p, W).replace(os.sep, "/")
                size = os.path.getsize(p)
                h = hashlib.sha256()
                with open(p, "rb") as fh:
                    for chunk in iter(lambda: fh.read(8 << 20), b""):
                        h.update(chunk)
                rec = {"path": rel, "bytes": size, "sha256": h.hexdigest(), "mtime": provenance.file_record(p)["mtime"]}
                low = f.lower()
                if low.endswith(embed_ext):
                    with open(p, encoding="utf-8", errors="replace") as fh:
                        txt = fh.read()
                    if size <= 64 * 1024:
                        if low.endswith(".json"):
                            try:
                                rec["content"] = json.loads(txt)
                            except Exception:
                                rec["text"] = txt
                        else:
                            rec["text"] = txt
                        totals["embedded"] += 1
                    else:
                        rec["sha256_strings"] = sorted(set(sha_re.findall(txt)))
                        argvs = []
                        if low.endswith(".json"):
                            try:
                                j = json.loads(txt)
                            except Exception:
                                j = None

                            def find_argv(o):
                                if isinstance(o, dict):
                                    if isinstance(o.get("argv"), list):
                                        argvs.append(o["argv"])
                                    for v in o.values():
                                        find_argv(v)
                                elif isinstance(o, list):
                                    for v in o:
                                        find_argv(v)
                            find_argv(j)
                        rec["argv"] = argvs
                        totals["reduced"] += 1
                files.append(rec)
                totals["files"] += 1
                totals["bytes"] += size
    return {"root": W, "totals": totals, "files": files}


def _trim_tool(t):
    if not t:
        return None
    return {"run": t.get("run"), "tool_findings": t.get("tool_findings"), "adm": t.get("adm"),
            "vs_stimulus": {k: (t.get("vs_stimulus") or {}).get(k) for k in ("ledger", "pcm", "timecode_roundtrip")} if isinstance(t.get("vs_stimulus"), dict) else t.get("vs_stimulus")}


if __name__ == "__main__":
    sys.exit(main())
