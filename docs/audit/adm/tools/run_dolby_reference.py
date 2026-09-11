#!/usr/bin/env python3
"""Dolby reference leg: what Dolby's own converters write, read back and accept.

    python run_dolby_reference.py --work <dir> --stage stimuli|ct|dee|reverse|validate|report [--only name,...]

Stages
  stimuli   DAMF stimuli (text edits of authored masters) under <work>/stimuli/<name>/
  ct        Dolby Atmos Conversion Tool: DAMF -> ADM (-f wav) for every stimulus
  dee       DEE 5.2.1 convert_atmos_mezz: DAMF -> ADM for every stimulus
  reverse   Conversion Tool ADM -> DAMF (-f atmos) on oadec's ADMs and on a
            variant carrying the real ramps as interpolationLength
  validate  atmos_info (5.7.2 and 1.1) and bwf_info on oadec's and Dolby's ADMs
  report    normalise + compare every Dolby output against its stimulus and
            against oadec's ADM where one exists; write <work>/f5-report.json
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import damf as damf_mod  # noqa: E402
from admaudit import dolby, mutate, normalise, provenance  # noqa: E402
from run_compare import compare  # noqa: E402

TRUTH = r"E:\oadec-work\audit\remediation\truth"
PIHEAD = r"E:\oadec-work\audit\adm\work\pi-head50m"
RAMP_CYCLE = [0, 32, 250, 480, 1536, 2048]


def _log(msg: str) -> None:
    print(f"[{provenance.utc_now()}] {msg}", flush=True)


# ------------------------------------------------------------------ stimuli

def _cycle_ramps(text: str) -> str:
    header, events = mutate._split_metadata(text)
    k: dict = {}
    for ev in events:
        idx = k.get(ev[0], 0)
        k[ev[0]] = idx + 1
        ramp = RAMP_CYCLE[idx % len(RAMP_CYCLE)]
        found = False
        for i, line in enumerate(ev[2]):
            if re.match(r"^\s*rampLength:", line):
                ev[2][i] = re.sub(r"(rampLength:\s*)\S+", lambda m: m.group(1) + str(ramp), line)
                found = True
        if not found:
            ev[2].insert(2, f"    rampLength: {ramp}")
    return mutate._join(header, events)


def _append_event(text: str, element_id: int, sample_pos: int, fields: dict) -> str:
    header, events = mutate._split_metadata(text)
    lines = [f"  - ID: {element_id}", f"    samplePos: {sample_pos}"] + [f"    {k}: {mutate._fmt_value(v)}" for k, v in fields.items()]
    events.append([element_id, sample_pos, lines])
    events.sort(key=lambda e: e[1] if e[1] is not None else -1)
    return mutate._join(header, events)


def make_stimuli(work: str, only: set | None) -> dict:
    S = os.path.join(work, "stimuli")
    os.makedirs(S, exist_ok=True)
    fast = os.path.join(TRUTH, "fast")
    gainsize = os.path.join(TRUTH, "gainsize")
    scene = os.path.join(TRUTH, "scene")
    made = {}

    def want(name):
        return only is None or name in only

    def out(name):
        d = os.path.join(S, name)
        os.makedirs(d, exist_ok=True)
        return os.path.join(d, name)

    if want("S1-ramps"):
        mutate.damf_edit_text(fast, out("S1-ramps"), _cycle_ramps)
        made["S1-ramps"] = {"base": fast, "what": "fast scene; rampLength cycles 0/32/250/480/1536/2048 per event of every element"}
    if want("S2-gain-size"):
        mutate.damf_copy(gainsize, out("S2-gain-size"))
        made["S2-gain-size"] = {"base": gainsize, "what": "authored gains 0/-3/-6/-12/-24 dB (objects 10-14) and sizes 0.25/0.5/1.0 (15-17), as written by atmos-author"}
    if want("S3-importance"):
        p = out("S3-importance")
        mutate.damf_set(gainsize, p, 10, None, "importance", 0.5)
        mutate.damf_set(p, p + "-tmp", 11, None, "importance", 0.25)
        mutate.damf_set(p + "-tmp", p, 12, None, "importance", 0.0)
        made["S3-importance"] = {"base": gainsize, "what": "importance 0.5 / 0.25 / 0.0 on objects 10 / 11 / 12"}
    if want("S5-inactive"):
        mutate.damf_set(gainsize, out("S5-inactive"), 13, None, "active", False)
        made["S5-inactive"] = {"base": gainsize, "what": "object 13 active: false on every event"}
    if want("S6-late-first"):
        mutate.damf_move_event(scene, out("S6-late-first"), 10, 0, 96000)
        made["S6-late-first"] = {"base": scene, "what": "first event of object 10 moved from 0 to 96000 (no state before it)"}
    if want("S7-zones"):
        p = out("S7-zones")
        mutate.damf_set(gainsize, p, 14, None, "zones", "no back")
        mutate.damf_set(p, p + "-tmp", 15, None, "zones", "surround only")
        mutate.damf_set(p + "-tmp", p, 15, None, "elevation", False)
        mutate.damf_set(p, p + "-tmp", 16, None, "snap", True)
        mutate.damf_copy(p + "-tmp", p)
        made["S7-zones"] = {"base": gainsize, "what": "zones 'no back' (14), 'surround only' + elevation false (15), snap (16)"}
    if want("S8-bed-gain"):
        p = out("S8-bed-gain")
        mutate.damf_set(gainsize, p, 3, None, "gain", -6)
        mutate.damf_edit_text(p, p + "-tmp", lambda t: _append_event(t, 3, 48000, {"gain": -12}))
        mutate.damf_copy(p + "-tmp", p)
        made["S8-bed-gain"] = {"base": gainsize, "what": "LFE bed gain -6 dB at 0 and -12 dB at 48000"}
    if want("S9-screen"):
        mutate.damf_set(gainsize, out("S9-screen"), 17, None, "screenFactor", 0.5)
        made["S9-screen"] = {"base": gainsize, "what": "object 17 screenFactor 0.5"}
    if want("S10-fps"):
        mutate.damf_set_header(gainsize, out("S10-fps"), "fps", "23.976")
        made["S10-fps"] = {"base": gainsize, "what": "header fps 23.976 instead of 24 (metadata unchanged)"}
    if want("S12-trailing-ramp"):
        mutate.damf_edit_text(gainsize, out("S12-trailing-ramp"), lambda t: _append_event(t, 10, 240000, {"rampLength": 1536}))
        made["S12-trailing-ramp"] = {"base": gainsize, "what": "object 10 gets a trailing event at 240000 that changes only rampLength"}
    if want("S13-same-pos"):
        mutate.damf_edit_text(gainsize, out("S13-same-pos"), lambda t: _append_event(_append_event(t, 10, 96000, {"pos": (0.5, 0.5, 0.0)}), 10, 96000, {"pos": (-0.5, -0.5, 0.0)}))
        made["S13-same-pos"] = {"base": gainsize, "what": "object 10 gets two events at samplePos 96000 (pos (0.5,0.5,0) then (-0.5,-0.5,0))"}
    if want("S0-pihead"):
        d = os.path.join(S, "S0-pihead")
        os.makedirs(d, exist_ok=True)
        mutate.damf_copy(os.path.join(PIHEAD, "default", "pi-head50m"), os.path.join(d, "S0-pihead"))
        made["S0-pihead"] = {"base": os.path.join(PIHEAD, "default", "pi-head50m"), "what": "pi-head50m decoded DAMF (default bed conform), unchanged"}
    if want("S11-nbc"):
        d = os.path.join(S, "S11-nbc")
        os.makedirs(d, exist_ok=True)
        mutate.damf_copy(os.path.join(PIHEAD, "nbc", "pi-head50m"), os.path.join(d, "S11-nbc"))
        made["S11-nbc"] = {"base": os.path.join(PIHEAD, "nbc", "pi-head50m"), "what": "pi-head50m decoded DAMF with --no-bed-conform (LFE-only bed)"}
    for name, rec in made.items():
        rec["damf"] = os.path.join(S, name, name)
        h = damf_mod.read_atmos(rec["damf"] + ".atmos")
        rec["elements"] = {"beds": h.bed_channels, "objects": h.object_ids}
    with open(os.path.join(S, "stimuli.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(made, f, indent=1)
    return made


def load_stimuli(work: str) -> dict:
    with open(os.path.join(work, "stimuli", "stimuli.json"), encoding="utf-8") as f:
        return json.load(f)


# ------------------------------------------------------------------ Dolby runs

def stage_ct(work: str, only: set | None) -> None:
    for name, rec in load_stimuli(work).items():
        if only is not None and name not in only:
            continue
        outdir = os.path.join(work, "stimuli", name, "ct")
        if os.path.isfile(os.path.join(outdir, "output.wav")):
            _log(f"ct {name}: exists, skipping")
            continue
        _log(f"ct {name} -> {outdir}")
        r = dolby.conversion_tool(rec["damf"] + ".atmos", outdir, "wav", timeout=3600)
        with open(os.path.join(outdir, "run.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(r, f, indent=1)
        _log(f"ct {name}: exit {r['run']['exit_code']} in {r['run']['seconds']}s, findings {len(r['findings'])}")


def stage_dee(work: str, only: set | None) -> None:
    for name, rec in load_stimuli(work).items():
        if only is not None and name not in only:
            continue
        outdir = os.path.join(work, "stimuli", name, "dee")
        if os.path.isfile(os.path.join(outdir, f"{name}-dee.wav")):
            _log(f"dee {name}: exists, skipping")
            continue
        _log(f"dee {name} -> {outdir}")
        r = dolby.dee_convert(rec["damf"] + ".atmos", outdir, f"{name}-dee.wav", "adm", timeout=3600)
        with open(os.path.join(outdir, "run.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(r, f, indent=1)
        _log(f"dee {name}: exit {r['run']['exit_code']} in {r['run']['seconds']}s, findings {len(r['findings'])}")


def _real_ramp_adm(work: str) -> str:
    """oadec's pi-head50m ADM with interpolationLength := the DAMF ramp of each block (block 0 stays 0)."""
    out = os.path.join(work, "reverse", "pi-head50m-realramps.wav")
    if os.path.isfile(out):
        return out
    os.makedirs(os.path.dirname(out), exist_ok=True)
    adm = os.path.join(PIHEAD, "tag", "pi-head50m.wav")
    a = normalise.from_adm(adm)
    d = normalise.from_damf(os.path.join(PIHEAD, "default", "pi-head50m"))
    fs = a.source["sample_rate"]
    spec = {}
    dm = {o.ordinal: o for o in d.objects}
    for o in a.objects:
        states = [s for s in dm[o.ordinal].events]
        # DAMF states that survive as blocks: same-position duplicates and trailing ramp-only states are gone;
        # match by time instead of assuming counts
        by_t = {s.t: s for s in states}
        per = {}
        for k, b in enumerate(o.events):
            if k == 0:
                continue
            s = by_t.get(b.t)
            if s is not None and s.ramp is not None:
                per[k] = int(s.ramp)
        if per:
            spec[o.channel_format] = per
    mutate.set_interpolation(adm, out, spec, fs)
    return out


def stage_reverse(work: str, only: set | None) -> None:
    R = os.path.join(work, "reverse")
    os.makedirs(R, exist_ok=True)
    jobs = {
        "oadec-default": os.path.join(PIHEAD, "default", "pi-head50m.wav"),
        "oadec-tag": os.path.join(PIHEAD, "tag", "pi-head50m.wav"),
        "oadec-nbc": os.path.join(PIHEAD, "nbc", "pi-head50m.wav"),
        "oadec-realramps": _real_ramp_adm(work),
        "ct-S0": os.path.join(work, "stimuli", "S0-pihead", "ct", "output.wav"),
        "dee-S0": os.path.join(work, "stimuli", "S0-pihead", "dee", "S0-pihead-dee.wav"),
        "ct-S1": os.path.join(work, "stimuli", "S1-ramps", "ct", "output.wav"),
    }
    for name, adm in jobs.items():
        if only is not None and name not in only:
            continue
        if not os.path.isfile(adm):
            _log(f"reverse {name}: input {adm} missing, skipping")
            continue
        outdir = os.path.join(R, name)
        if os.path.isfile(os.path.join(outdir, "output.atmos")):
            _log(f"reverse {name}: exists, skipping")
            continue
        _log(f"reverse {name}: {adm} -> {outdir}")
        r = dolby.conversion_tool(adm, outdir, "atmos", timeout=3600)
        with open(os.path.join(outdir, "run.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(r, f, indent=1)
        _log(f"reverse {name}: exit {r['run']['exit_code']} in {r['run']['seconds']}s")


def stage_validate(work: str, only: set | None) -> None:
    V = os.path.join(work, "validate")
    os.makedirs(V, exist_ok=True)
    files = {
        "oadec-default": os.path.join(PIHEAD, "default", "pi-head50m.wav"),
        "oadec-tag": os.path.join(PIHEAD, "tag", "pi-head50m.wav"),
        "oadec-nbc": os.path.join(PIHEAD, "nbc", "pi-head50m.wav"),
        "oadec-realramps": os.path.join(work, "reverse", "pi-head50m-realramps.wav"),
        "ct-S0": os.path.join(work, "stimuli", "S0-pihead", "ct", "output.wav"),
        "dee-S0": os.path.join(work, "stimuli", "S0-pihead", "dee", "S0-pihead-dee.wav"),
        "ct-S2": os.path.join(work, "stimuli", "S2-gain-size", "ct", "output.wav"),
    }
    results = {}
    for name, p in files.items():
        if only is not None and name not in only:
            continue
        if not os.path.isfile(p):
            results[name] = {"missing": p}
            continue
        _log(f"validate {name}")
        results[name] = {
            "file": provenance.file_record(p),
            "atmos_info_5.7.2_validate": dolby.atmos_info(p, True, "5.7.2"),
            "atmos_info_1.1": dolby.atmos_info(p, True, "1.1"),
            "bwf_info": dolby.bwf_info(p),
        }
        for k in ("atmos_info_5.7.2_validate", "atmos_info_1.1", "bwf_info"):
            r = results[name][k]
            _log(f"  {k}: exit {r['run']['exit_code']} findings {[x['line'][:90] for x in r['findings']][:3]}")
    with open(os.path.join(V, "validate.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(results, f, indent=1)


def _adm_summary(path: str) -> dict:
    s = normalise.from_adm(path)
    interp = {}
    gains = imps = sizes = zones = 0
    z_present = 0
    first_t = []
    blocks = 0
    for o in s.objects:
        for k, e in enumerate(o.events):
            blocks += 1
            key = f"{e.interp['len_samples']}" if e.interp else "none"
            interp[key] = interp.get(key, 0) + 1
            gains += e.gain["present"]
            imps += e.importance["present"]
            sizes += e.size["present"]
            zones += bool(e.zones["names"])
            z_present += e.z_present
        if o.events:
            first_t.append(o.events[0].t)
    return {
        "source": {k: s.source[k] for k in ("frames", "channels", "sample_rate", "fourcc", "dbmd_creator", "dbmd_tool", "time_notation", "programme_start", "programme_end", "element_counts")},
        "beds": [(b.label, b.track, b.pos) for b in s.beds],
        "objects": [(o.audio_object, o.ordinal, o.track, len(o.events)) for o in s.objects],
        "blocks": blocks, "interp_len_samples_hist": interp, "gain_present": gains, "importance_present": imps,
        "size_present": sizes, "zone_blocks": zones, "z_present": z_present, "first_block_t": first_t,
        "findings": [f.__dict__ for f in s.findings],
    }


def stage_report(work: str, only: set | None) -> None:
    stim = load_stimuli(work)
    rep = {"utc": provenance.utc_now(), "stimuli": {}, "reverse": {}, "validate": None}
    for name, rec in stim.items():
        if only is not None and name not in only:
            continue
        entry = {"what": rec["what"], "damf": rec["damf"], "elements": rec["elements"], "ct": None, "dee": None}
        for tool, wav in (("ct", os.path.join(work, "stimuli", name, "ct", "output.wav")), ("dee", os.path.join(work, "stimuli", name, "dee", f"{name}-dee.wav"))):
            runp = os.path.join(os.path.dirname(wav), "run.json")
            run = json.load(open(runp, encoding="utf-8")) if os.path.isfile(runp) else None
            if not os.path.isfile(wav):
                entry[tool] = {"run": run["run"] if run else None, "findings": run["findings"] if run else None, "output": None}
                continue
            _log(f"report {name} {tool}")
            try:
                cmp_ = compare(rec["damf"], wav, 0.0, "effective", "window", 20.0, f"{name}/{tool}")
                cmp_small = {k: cmp_[k] for k in ("ledger", "trajectory_loss", "timecode_roundtrip", "pcm")}
                cmp_small["ledger"] = {kk: cmp_small["ledger"][kk] for kk in ("classes", "defects", "loss", "identity_ok", "tiling", "bed_changes_lost", "loss_items")} if "loss_items" in cmp_small["ledger"] else cmp_small["ledger"]
            except Exception as e:  # noqa: BLE001
                cmp_small = {"error": f"{type(e).__name__}: {e}"}
            entry[tool] = {"run": {k: run["run"][k] for k in ("exit_code", "seconds")} if run else None,
                           "tool_findings": run["findings"] if run else None,
                           "adm": _adm_summary(wav), "vs_stimulus": cmp_small}
        rep["stimuli"][name] = entry
        with open(os.path.join(work, "stimuli", name, "report.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(entry, f, indent=1, default=str)
    # reverse: what Dolby read back from oadec's ADM, compared with oadec's own DAMF
    R = os.path.join(work, "reverse")
    if os.path.isdir(R):
        for name in sorted(os.listdir(R)):
            base = os.path.join(R, name, "output")
            if not os.path.isfile(base + ".atmos"):
                continue
            if only is not None and name not in only:
                continue
            _log(f"report reverse {name}")
            ref_damf = os.path.join(PIHEAD, "nbc" if "nbc" in name else "default", "pi-head50m")
            d_back = normalise.from_damf(base)
            d_ref = normalise.from_damf(ref_damf)
            from admaudit import compare_events
            # compare Dolby's DAMF (as ADM-like "b" side) with oadec's DAMF: reuse the ledger by treating the
            # read-back states as blocks with ramp carried in 'interp'
            back_events = {}
            for o in d_back.objects:
                back_events[o.ordinal] = [(e.t, e.ramp, e.gain, e.importance, e.pos, e.size) for e in o.events]
            ref_events = {o.ordinal: [(e.t, e.ramp, e.gain, e.importance, e.pos, e.size) for e in o.events] for o in d_ref.objects}
            ramp_hist_back = {}
            for evs in back_events.values():
                for e in evs:
                    ramp_hist_back[str(e[1])] = ramp_hist_back.get(str(e[1]), 0) + 1
            ramp_hist_ref = {}
            for evs in ref_events.values():
                for e in evs:
                    ramp_hist_ref[str(e[1])] = ramp_hist_ref.get(str(e[1]), 0) + 1
            times_equal = all([e[0] for e in back_events.get(k, [])] == [e[0] for e in ref_events.get(k, [])] for k in ref_events)
            pos_equal = all([tuple(round(float(x), 6) for x in e[4]) for e in back_events.get(k, [])] == [tuple(round(float(x), 6) for x in e[4]) for e in ref_events.get(k, [])] for k in ref_events if back_events.get(k))
            rep["reverse"][name] = {
                "damf_source": d_back.source, "findings": [f.__dict__ for f in d_back.findings],
                "ref_damf": ref_damf,
                "events_back": {k: len(v) for k, v in back_events.items()}, "events_ref": {k: len(v) for k, v in ref_events.items()},
                "ramp_hist_back": ramp_hist_back, "ramp_hist_ref": ramp_hist_ref,
                "event_times_equal": times_equal, "positions_equal_at_1e-6": pos_equal,
                "bed_events_back": [(b.label, len(b.events), [ (e.t, e.gain["db"]) for e in b.events]) for b in d_back.beds],
            }
    vp = os.path.join(work, "validate", "validate.json")
    if os.path.isfile(vp):
        v = json.load(open(vp, encoding="utf-8"))
        rep["validate"] = {name: ({"missing": r["missing"]} if "missing" in r else {k: {"exit": r[k]["run"]["exit_code"], "findings": [x["line"][:160] for x in r[k]["findings"]], "stdout_tail": r[k]["run"]["stdout_tail"][-400:]} for k in ("atmos_info_5.7.2_validate", "atmos_info_1.1", "bwf_info")}) for name, r in v.items()}
    with open(os.path.join(work, "f5-report.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(rep, f, indent=1, default=str)
    _log(f"report written: {os.path.join(work, 'f5-report.json')}")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--stage", required=True, choices=["stimuli", "ct", "dee", "reverse", "validate", "report"])
    ap.add_argument("--only", default=None)
    a = ap.parse_args()
    only = set(a.only.split(",")) if a.only else None
    os.makedirs(a.work, exist_ok=True)
    {"stimuli": make_stimuli, "ct": stage_ct, "dee": stage_dee, "reverse": stage_reverse, "validate": stage_validate, "report": stage_report}[a.stage](a.work, only)
    return 0


if __name__ == "__main__":
    sys.exit(main())
