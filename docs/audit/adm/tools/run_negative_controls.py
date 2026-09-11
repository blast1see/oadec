#!/usr/bin/env python3
"""Negative controls: inject one known defect at a time and prove the detectors fire.

    python run_negative_controls.py --adm <file.wav> --damf <base> --work <dir> --out <json>

Each control records the mutation, the detector that must fire, the metric
observed on the mutated file, whether it fired, and every *other* change
against the unmutated baseline (specificity).  A tool that cannot see an
injected defect cannot certify the real file, so every later verdict is
conditional on this file.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
import traceback
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import mutate, normalise, riff  # noqa: E402
from run_compare import compare  # noqa: E402


def summarise(rep: dict) -> dict:
    L = rep["ledger"]
    pcm = rep["pcm"]
    return {
        "ledger_classes": dict(L["classes"]),
        "defect_classes": dict(Counter(d["cls"] for d in L["defects"])),
        "defect_fields": dict(Counter("+".join(d["fields"]) for d in L["defects"])),
        "loss": dict(L["loss"]),
        "tiling": {k: L["tiling"][k] for k in ("gaps", "overlaps", "starts_at_zero", "ends_at_frames", "blocks_shorter_than_interp")},
        "adm_findings": sorted({f["kind"] for f in rep["adm"]["findings"]}),
        "damf_findings": sorted({f["kind"] for f in rep["damf"]["findings"]}),
        "pcm_identical": sum(1 for p in pcm["pairs"] if p.get("identical")),
        "pcm_pairs": len(pcm["pairs"]),
        "pcm_first_diff": {f"{p['role']}:{p['label']}": p.get("first_diff") for p in pcm["pairs"] if p.get("first_diff") is not None},
        "matrix_confirmed": pcm["matrix"]["all_declared_pairs_confirmed"] if pcm["matrix"] else None,
        "timecode_mismatches": rep["timecode_roundtrip"]["re_encode_mismatches"],
        "selfcheck_max": max(rep["trajectory_selfcheck_max_e"].values() or [0.0]),
        "trajectory_max_e": {str(k): max(v["max_e"].values()) for k, v in rep["trajectory_loss"].items()},
        "trajectory_mean_e": {str(k): sum(v["mean_e"].values()) for k, v in rep["trajectory_loss"].items()},
        "presence_differences": len(L.get("presence_differences", [])),
    }


def diff_summaries(base: dict, mut: dict, ignore: set) -> list:
    out = []
    for k in base:
        if k in ignore:
            continue
        if base[k] != mut.get(k):
            out.append({"key": k, "baseline": base[k], "mutated": mut.get(k)})
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--adm", required=True)
    ap.add_argument("--damf", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--window-seconds", type=float, default=30.0)
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    W = a.window_seconds

    base_rep = compare(a.damf, a.adm, 0.0, "effective", "window", W, "baseline")
    base = summarise(base_rep)
    strict_base = summarise(compare(a.damf, a.adm, 0.0, "strict", "none", W, "baseline-strict"))
    scene = normalise.from_adm(a.adm)
    fs = scene.source["sample_rate"]
    objs = [o for o in scene.objects if o.channel_format and len(o.events) >= 3]
    ac1, ac2 = objs[0].channel_format, objs[1].channel_format
    ord1 = objs[0].ordinal
    tr1, tr2 = objs[0].track, objs[1].track
    zobj = next((o for o in scene.objects if any(e.z_present and e.pos[2] != 0.0 for e in o.events[1:])), None)
    inactive = next(((o, i) for o in scene.objects for i, e in enumerate(o.events) if not e.active), None)

    def M(path):
        return os.path.join(a.work, path)

    results = []

    def control(cid, description, mutate_fn, expect_fn, expected_detector, ignore=(), mode="effective", pcm="window", damf_base=None, adm_path=None):
        rec = {"id": cid, "description": description, "expected_detector": expected_detector}
        try:
            out = mutate_fn()
            adm_p = adm_path or (out if out and str(out).endswith(".wav") else a.adm)
            damf_b = damf_base or (out if out and not str(out).endswith(".wav") else a.damf)
            rep = compare(damf_b, adm_p, 0.0, mode, pcm, W, cid)
            s = summarise(rep)
            fired, metric = expect_fn(s, rep)
            rec.update({"fired": bool(fired), "metric": metric, "mutated_summary": s})
            ref = strict_base if mode == "strict" else base
            rec["specificity"] = diff_summaries(ref, s, set(ignore))
        except Exception as e:  # a control that expects an exception handles it in expect_fn via marker
            tb = traceback.format_exc()[-2000:]
            try:
                fired, metric = expect_fn(None, e)
            except Exception:
                fired, metric = False, f"unexpected exception: {type(e).__name__}: {e}"
            rec.update({"fired": bool(fired), "metric": metric, "exception": f"{type(e).__name__}: {e}"})
            if not fired:
                rec["traceback"] = tb
        results.append(rec)
        print(f"{cid:6} {'FIRED ' if rec['fired'] else 'MISSED'} {description} -> {rec['metric']}" + (f" | specificity: {len(rec.get('specificity', []))} other change(s)" if rec.get("specificity") else ""))

    # ---- ADM controls
    control("M-A1", f"rtime of block 1 of {ac1} +1 sample (neighbour durations adjusted)",
            lambda: (mutate.shift_block(a.adm, M("a1.wav"), ac1, 1, +1, fs), M("a1.wav"))[1],
            lambda s, r: (s["defect_classes"].get("time_mismatch", 0) == 1, s["defect_classes"]), "ledger.time_mismatch == 1 (the shifted state also leaves the ramp-loss tally: expected)",
            ignore={"ledger_classes", "defect_classes", "defect_fields", "loss"})

    def a2():
        def fn(k, block):
            if k != 1:
                return None
            m = re.search(r'(<position coordinate="X">)([^<]*)(</position>)', block)
            return block.replace(m.group(0), m.group(1) + f"{float(m.group(2)) + 0.001:.10f}" + m.group(3))
        mutate.edit_blocks(a.adm, M("a2.wav"), ac1, fn)
        return M("a2.wav")
    control("M-A2", f"X of block 1 of {ac1} += 0.001", a2,
            lambda s, r: (s["defect_classes"].get("value_mismatch", 0) == 1 and s["defect_fields"].get("pos", 0) == 1, s["defect_fields"]),
            "ledger.value_mismatch{pos} == 1", ignore={"ledger_classes", "defect_classes", "defect_fields", "trajectory_max_e", "trajectory_mean_e"})
    control("M-A2t", "same file at pos_tol 0.01 must NOT fire",
            lambda: M("a2.wav"),
            lambda s, r: (True, "n/a"), "tolerance sanity (see M-A2t-tol)", ignore={"ledger_classes", "defect_classes", "defect_fields", "trajectory_max_e", "trajectory_mean_e"})
    rep_tol = compare(a.damf, M("a2.wav"), 0.01, "effective", "none", W, "M-A2 at tol 0.01")
    results.append({"id": "M-A2t-tol", "description": "M-A2 file compared at pos_tol 0.01", "expected_detector": "no value_mismatch at 0.01",
                    "fired": summarise(rep_tol)["defect_classes"].get("value_mismatch", 0) == 0, "metric": summarise(rep_tol)["defect_classes"]})

    def a3():
        def fn(text):
            s1, e1 = mutate._channel_span(text, ac1)
            s2, e2 = mutate._channel_span(text, ac2)
            b1 = text[s1:e1]
            b2 = text[s2:e2]
            i1 = b1.index(">") + 1
            i2 = b2.index(">") + 1
            inner1 = b1[i1: b1.rindex("</audioChannelFormat>")]
            inner2 = b2[i2: b2.rindex("</audioChannelFormat>")]
            n1 = b1[:i1] + inner2 + "</audioChannelFormat>"
            n2 = b2[:i2] + inner1 + "</audioChannelFormat>"
            if s1 < s2:
                return text[:s1] + n1 + text[e1:s2] + n2 + text[e2:]
            return text[:s2] + n2 + text[e2:s1] + n1 + text[e1:]
        mutate.edit_axml(a.adm, M("a3.wav"), fn)
        return M("a3.wav")
    control("M-A3", f"block lists of {ac1} and {ac2} swapped (structure untouched)", a3,
            lambda s, r: (s["defect_classes"].get("value_mismatch", 0) >= 2 and s["adm_findings"] == base["adm_findings"], s["defect_classes"]),
            "ledger.value_mismatch on both objects, structural findings unchanged",
            ignore={"ledger_classes", "defect_classes", "defect_fields", "trajectory_max_e", "trajectory_mean_e", "loss"})
    control("M-A4", f"PCM of tracks {tr1} and {tr2} swapped",
            lambda: (mutate.swap_tracks(a.adm, M("a4.wav"), tr1, tr2), M("a4.wav"))[1],
            lambda s, r: (s["pcm_identical"] == base["pcm_identical"] - 2 and s["matrix_confirmed"] is False, {"identical": s["pcm_identical"], "matrix_confirmed": s["matrix_confirmed"]}),
            "pcm identical pairs -2 and pairing matrix not as declared", ignore={"pcm_identical", "matrix_confirmed", "pcm_first_diff"})
    control("M-A5", f"interior block 1 of {ac1} deleted",
            lambda: (mutate.delete_block(a.adm, M("a5.wav"), ac1, 1), M("a5.wav"))[1],
            lambda s, r: (s["tiling"]["gaps"] == base["tiling"]["gaps"] + 1 and s["defect_classes"].get("unexplained_missing", 0) == 1, {"gaps": s["tiling"]["gaps"], "defects": s["defect_classes"]}),
            "tiling gap +1 and unexplained_missing == 1", ignore={"ledger_classes", "defect_classes", "defect_fields", "tiling", "loss", "trajectory_max_e", "trajectory_mean_e"})
    control("M-A6", f"interpolationLength of block 1 of {ac1} := 1536 samples (the real ramp)",
            lambda: (mutate.set_interpolation(a.adm, M("a6.wav"), {ac1: {1: 1536}}, fs), M("a6.wav"))[1],
            lambda s, r: (s["loss"].get("ramp", 0) == base["loss"].get("ramp", 0) - 1
                          and (s["trajectory_mean_e"][str(ord1)] != base["trajectory_mean_e"][str(ord1)] or s["trajectory_max_e"][str(ord1)] != base["trajectory_max_e"][str(ord1)]),
                          {"loss": s["loss"], "mean_e": (base["trajectory_mean_e"][str(ord1)], s["trajectory_mean_e"][str(ord1)])}),
            "ledger.loss.ramp -1 and the trajectory statistics of that object change (profile checker also flags the non-250 length: expected)", ignore={"loss", "trajectory_max_e", "trajectory_mean_e", "adm_findings"})
    control("M-A7", "chna entry of the first object track points at a non-existent track format",
            lambda: (mutate.break_chna(a.adm, M("a7.wav"), tr1, track_ref="AT_00039999_01"), M("a7.wav"))[1],
            lambda s, r: ("chna-dangling" in s["adm_findings"], s["adm_findings"]), "axml/chna finding chna-dangling",
            ignore={"adm_findings", "pcm_identical", "pcm_pairs", "matrix_confirmed", "ledger_classes", "defect_classes", "defect_fields", "loss", "trajectory_max_e", "pcm_first_diff", "tiling", "timecode_mismatches"})
    control("M-A8", "data chunk shortened by 3 bytes (not a whole frame)",
            lambda: (mutate.set_data_size(a.adm, M("a8.wav"), -3), M("a8.wav"))[1],
            lambda s, r: ("data-partial-frame" in s["adm_findings"], s["adm_findings"]), "riff finding data-partial-frame",
            ignore={"adm_findings", "tiling", "pcm_identical", "pcm_first_diff", "ledger_classes", "defect_classes", "defect_fields"})
    if zobj is not None:
        zi = next(i for i, e in enumerate(zobj.events) if i > 0 and e.z_present and e.pos[2] != 0.0)
        control("M-A9a", f"non-zero Z removed from block {zi} of {zobj.channel_format}",
                lambda: (mutate.remove_child(a.adm, M("a9a.wav"), zobj.channel_format, zi, "position coordinate=\"Z\""), M("a9a.wav"))[1],
                lambda s, r: (s["defect_fields"].get("pos", 0) == 1, s["defect_fields"]), "ledger.value_mismatch{pos} == 1 (z)",
                ignore={"ledger_classes", "defect_classes", "defect_fields", "trajectory_max_e", "trajectory_mean_e"})
    else:
        results.append({"id": "M-A9a", "description": "non-zero Z removed", "fired": None, "metric": "N/T: no block with non-zero Z in this file"})
    control("M-A9b", f"<gain>1.0</gain> added to active block 1 of {ac1} (effective mode: no new defect)",
            lambda: (mutate.add_child(a.adm, M("a9b.wav"), ac1, 1, "<gain>1.0</gain>"), M("a9b.wav"))[1],
            lambda s, r: (s["defect_classes"] == base["defect_classes"], s["defect_classes"]), "no new defect in effective mode (the profile checker flags the lone gain element: expected, measured by M-A9bs)", ignore={"adm_findings"})
    control("M-A9bs", "same file: the profile checker must flag gain on an active object",
            lambda: M("a9b.wav"),
            lambda s, r: (bool({"profile-gain-on-active", "profile-inactive-encoding"} & set(s["adm_findings"])), s["adm_findings"]), "profile finding profile-gain-on-active or profile-inactive-encoding",
            pcm="none", ignore={"adm_findings", "pcm_identical", "pcm_pairs", "matrix_confirmed", "pcm_first_diff"})
    if inactive is not None:
        io, ii = inactive
        control("M-A10", f"importance of inactive block {ii} of {io.channel_format} 0 -> 5",
                lambda: (mutate.edit_blocks(a.adm, M("a10.wav"), io.channel_format, lambda k, b: b.replace("<importance>0</importance>", "<importance>5</importance>") if k == ii else None), M("a10.wav"))[1],
                lambda s, r: (s["defect_fields"].get("active", 0) >= 1, s["defect_fields"]), "ledger.value_mismatch{active}",
                ignore={"ledger_classes", "defect_classes", "defect_fields"})
    else:
        results.append({"id": "M-A10", "description": "inactive block importance 0 -> 5", "fired": None, "metric": "N/T: no inactive block in this file"})

    def a11():
        def fn(k, block):
            if k != 1:
                return None
            t = mutate._get_attr(block, "rtime")
            head, frac = t.rsplit(".", 1)
            return mutate._set_attr(block, "rtime", f"{head}.{int(frac) + 1:05d}")
        mutate.edit_blocks(a.adm, M("a11.wav"), ac1, fn)
        return M("a11.wav")
    control("M-A11", f"fifth decimal of rtime of block 1 of {ac1} +1 (10 us, less than half a sample)", a11,
            lambda s, r: (s["timecode_mismatches"] == base["timecode_mismatches"] + 1 and s["defect_classes"] == base["defect_classes"], {"re_encode_mismatches": s["timecode_mismatches"], "defects": s["defect_classes"]}),
            "timecode re-encode mismatch +1 while the sample position still matches", ignore={"timecode_mismatches"})
    control("M-A12", "file truncated inside the axml chunk",
            lambda: (mutate.truncate(a.adm, M("a12.wav"), inside="axml"), M("a12.wav"))[1],
            lambda s, e: (isinstance(e, riff.ContainerError), f"{type(e).__name__}: {e}" if e is not None else s["adm_findings"]), "riff.ContainerError")
    control("M-A13", "audioTrackUID sampleRate of the first object := 44100",
            lambda: (mutate.edit_axml(a.adm, M("a13.wav"), lambda s: s.replace(f'UID="{objs[0].uid}" bitDepth="24" sampleRate="{fs}"', f'UID="{objs[0].uid}" bitDepth="24" sampleRate="44100"') if f'UID="{objs[0].uid}" bitDepth="24" sampleRate="{fs}"' in s else s.replace(f'UID="{objs[0].uid}" sampleRate="{fs}"', f'UID="{objs[0].uid}" sampleRate="44100"')), M("a13.wav"))[1],
            lambda s, r: ("uid-sample-rate" in s["adm_findings"], s["adm_findings"]), "finding uid-sample-rate", ignore={"adm_findings"})
    control("M-A14", f"+1 LSB on sample 1000 of track {tr1}",
            lambda: (mutate.poke_sample(a.adm, M("a14.wav"), tr1, 1000, +1), M("a14.wav"))[1],
            lambda s, r: (s["pcm_identical"] == base["pcm_identical"] - 1 and list(s["pcm_first_diff"].values()) == [1000], s["pcm_first_diff"]),
            "exactly one pair not identical, first_diff == 1000", ignore={"pcm_identical", "pcm_first_diff"})

    # ---- DAMF controls
    control("M-D1", "DAMF: the samplePos line of the second event of the first object deleted",
            lambda: (mutate.damf_edit_text(a.damf, M("d1"), lambda t: _drop_nth_samplepos(t, objs[0].ordinal, scene_damf_id(a.damf, objs[0].ordinal), 1)), M("d1"))[1],
            lambda s, r: ("damf-no-samplepos" in s["damf_findings"], s["damf_findings"]), "damf finding damf-no-samplepos",
            ignore={"damf_findings", "ledger_classes", "defect_classes", "defect_fields", "loss", "trajectory_max_e", "trajectory_mean_e", "pcm_identical", "pcm_pairs", "matrix_confirmed"})
    control("M-D2", "DAMF: pos removed from the first event of the first object",
            lambda: (mutate.damf_edit_text(a.damf, M("d2"), lambda t: _drop_first_pos(t, scene_damf_id(a.damf, objs[0].ordinal))), M("d2"))[1],
            lambda s, r: ("damf-first-event-incomplete" in s["damf_findings"], s["damf_findings"]), "damf finding damf-first-event-incomplete",
            ignore={"damf_findings", "ledger_classes", "defect_classes", "defect_fields", "loss", "trajectory_max_e", "trajectory_mean_e", "pcm_identical", "pcm_pairs", "matrix_confirmed"})
    control("M-D3", "DAMF: sampleRate 44100 in the metadata",
            lambda: (mutate.damf_edit_text(a.damf, M("d3"), lambda t: t.replace(f"sampleRate: {fs}", "sampleRate: 44100", 1)), M("d3"))[1],
            lambda s, r: ("damf-rate" in s["damf_findings"], s["damf_findings"]), "damf finding damf-rate",
            ignore={"damf_findings", "ledger_classes", "defect_classes", "defect_fields", "loss", "trajectory_max_e", "timecode_mismatches", "tiling", "pcm_identical", "pcm_pairs", "matrix_confirmed", "pcm_first_diff"})
    results.append({"id": "M-T1", "description": "trajectory evaluator against a synthetic faithful ADM (interpolationLength = ramp) built from the DAMF",
                    "expected_detector": "max_e == 0 on the faithful file, non-zero on the written one",
                    "fired": base["selfcheck_max"] == 0.0 and max(base["trajectory_max_e"].values()) > 0.0,
                    "metric": {"selfcheck_max": base["selfcheck_max"], "written_max_e": max(base["trajectory_max_e"].values())}})
    results.append({"id": "P-0", "description": "positive control: the unmutated pair", "expected_detector": "no defects, all PCM pairs identical",
                    "fired": not base["defect_classes"] and base["pcm_identical"] == base["pcm_pairs"], "metric": base})

    doc = {"adm": os.path.abspath(a.adm), "damf": os.path.abspath(a.damf), "baseline": base, "controls": results,
           "all_fired": all(r["fired"] for r in results if r["fired"] is not None),
           "not_testable": [r["id"] for r in results if r["fired"] is None],
           "specificity_violations": {r["id"]: r["specificity"] for r in results if r.get("specificity")}}
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, indent=1, default=str)
        f.write("\n")
    print(f"\nall fired: {doc['all_fired']}; not testable: {doc['not_testable']}; specificity violations: {list(doc['specificity_violations'])}")
    return 0 if doc["all_fired"] else 1


def scene_damf_id(damf_base: str, ordinal: int) -> int:
    from admaudit import damf as _damf
    return _damf.read_atmos(damf_base + ".atmos").object_ids[ordinal - 1]


def _drop_nth_samplepos(text: str, _ordinal: int, element_id: int, n: int) -> str:
    header, events = mutate._split_metadata(text)
    k = -1
    for ev in events:
        if ev[0] == element_id:
            k += 1
            if k == n:
                ev[2] = [l for l in ev[2] if not re.match(r"^\s*samplePos:", l)]
    return mutate._join(header, events)


def _drop_first_pos(text: str, element_id: int) -> str:
    header, events = mutate._split_metadata(text)
    for ev in events:
        if ev[0] == element_id:
            ev[2] = [l for l in ev[2] if not re.match(r"^\s*pos:", l)]
            break
    return mutate._join(header, events)


if __name__ == "__main__":
    sys.exit(main())
