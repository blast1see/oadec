#!/usr/bin/env python3
"""Writer-level cases: controlled events through oadec's ADM and DAMF writers.

    python run_harness.py --exe <adm_harness.exe> --work <dir> [--only C01,C02] [--ct]

Generates the case files, runs the harness, compares each ADM/DAMF pair with
the audit toolkit and records, per case, the measurement the case was built
for.  ``--ct`` additionally converts the harness DAMF with the Dolby Atmos
Conversion Tool so that Dolby's rendering of the same events sits next to
oadec's.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import dolby, normalise, riff  # noqa: E402
from run_compare import compare  # noqa: E402

FS = 48000


def obj(pos, **kw):
    d = {"pos": list(pos)}
    d.update(kw)
    return d


def ev(id_, t, **state):
    if "bed" in state:
        return {"id": id_, "sample_pos": t, "bed": state["bed"]}
    return {"id": id_, "sample_pos": t, "object": state["object"]}


def cases() -> dict:
    C = {}
    lfe = {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 2}
    tones = {"hz": [0, 440, 880], "dbfs": -20}
    C["C01-ramps"] = {
        "why": "A: ramp 0/32/480/1536/2048 on successive events of object 10",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0), ramp=0)), ev(11, 0, object=obj((1, 1, 0)))]
        + [ev(10, t, object=obj((x, 1, 0), ramp=r)) for t, x, r in ((9600, -0.5, 32), (19200, 0.0, 480), (28800, 0.5, 1536), (38400, 1.0, 2048))],
        "expect": "ADM interpolationLength 0 then 250 x4; ledger.loss.ramp == 4",
    }
    C["C02-gain"] = {
        "why": "B: constant -6 dB, +3 dB and -inf gain on active objects; a gain change mid-stream",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 4}, "frames": 96000, "audio": {"hz": [0, 440, 880, 1320, 1760], "dbfs": -20},
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0), gain_db=-6)), ev(11, 0, object=obj((1, 1, 0), gain_db=3)),
                   ev(12, 0, object=obj((0, 1, 0), gain_minus_inf=True)), ev(13, 0, object=obj((0, -1, 0), gain_db=0)), ev(13, 48000, object=obj((0, -1, 0), gain_db=-12))],
        "expect": "no <gain> on active blocks in oadec's ADM; DAMF carries -6/+3/-inf/-12; ledger.loss.gain counts them; PCM unscaled",
    }
    C["C03-importance"] = {
        "why": "C: importance 0.5 and 0.0 on active objects",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0), importance=0.5)), ev(11, 0, object=obj((1, 1, 0), importance=0.0))],
        "expect": "no <importance> on active blocks; ledger.loss.importance == 2",
    }
    C["C04-size3d"] = {
        "why": "D: OAMD three-axis size [0.2,0.5,0.8] and uniform [0.5,0.5,0.5] through Timeline::push",
        "program": lfe, "frames": 96000, "audio": tones,
        "oamd": [{"base": 0, "sample_offset": 0, "blocks": [[0, 1536]],
                  "objects": [[{"gain_db": 0, "priority": 1.0}], [{"pos": [0.0, 0.0, 0.0], "size": [0.2, 0.5, 0.8]}], [{"pos": [1.0, 0.0, 0.0], "size": [0.5, 0.5, 0.5]}]]}],
        "expect": "DAMF size 0.2 (first axis only) and ADM width=depth=height=0.2 for object 10; 0.5 for object 11",
    }
    C["C05-bed-events"] = {
        "why": "E: bed gain -6 at 0, -12 at 48000, inactive at 72000",
        "program": lfe, "frames": 96000, "audio": {"hz": [100, 440, 880], "dbfs": -20},
        "events": [ev(3, 0, bed={"gain_db": -6}), ev(10, 0, object=obj((-1, 1, 0))), ev(11, 0, object=obj((1, 1, 0))),
                   ev(3, 48000, bed={"gain_db": -12}), ev(3, 72000, bed={"active": False, "gain_db": -12})],
        "expect": "DAMF keeps 3 bed events; ADM bed block static; ledger.bed_changes_lost == 2; LFE PCM unscaled",
    }
    C["C06-late-first"] = {
        "why": "G: first event of object 10 at 96000 (active) and of object 11 at 96000 (inactive)",
        "program": lfe, "frames": 192000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 96000, object=obj((-1, 1, 0))), ev(11, 96000, object=obj((1, 1, 0), active=False))],
        "expect": "synthetic block at 0 holding the first state; ledger.synthetic_block0 == 2 with holds first-state",
    }
    C["C07-beyond-end"] = {
        "why": "F/Q: events at frames and frames+40; the preceding block's duration",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0))), ev(10, 48000, object=obj((0, 1, 0))), ev(10, 96040, object=obj((1, 1, 0))),
                   ev(11, 0, object=obj((1, 1, 0))), ev(11, 96000, object=obj((0, 1, 0)))],
        "expect": "beyond_end 2; object 10 block 2 ends at 96040 > frames (tiling overrun); object 11 last block ends at 96000",
    }
    C["C08-same-pos"] = {
        "why": "F: two events of object 10 at sample 1536",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0))), ev(10, 1536, object=obj((0.5, 1, 0))), ev(10, 1536, object=obj((1, 1, 0))), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "superseded_same_pos 1; ADM keeps the last state; identity holds",
    }
    C["C09-out-of-order"] = {
        "why": "R: object 10 events at 0, 48000, 24000 (writer never sorts)",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0))), ev(10, 48000, object=obj((0, 1, 0))), ev(10, 24000, object=obj((1, 1, 0))), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "overlapping blocks (tiling overlaps > 0) and/or unsorted blocks",
    }
    C["C10-zero-objects"] = {
        "why": "S31: a programme with a bed and no dynamic objects",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 0}, "frames": 48000, "audio": {"hz": [100], "dbfs": -20},
        "events": [ev(3, 0, bed={})],
        "expect": "a valid file with 10 bed tracks and no Objects audioObject, or an explicit error",
    }
    C["C11-no-bed"] = {
        "why": "S31: two dynamic objects and no bed, bed_conform off",
        "program": {"beds": [], "isf_objects": 0, "dynamic_objects": 2}, "frames": 48000, "bed_conform": False, "audio": {"hz": [440, 880], "dbfs": -20},
        "events": [ev(10, 0, object=obj((-1, 1, 0))), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "a valid 2-track file, or an explicit error",
    }
    C["C12-118-objects"] = {
        "why": "S31: 118 dynamic objects + LFE = 119 channels (profile maximum 128)",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 118}, "frames": 9600, "bed_conform": False, "audio": {"hz": [0] + [200 + 10 * k for k in range(118)], "dbfs": -30},
        "events": [ev(3, 0, bed={})] + [ev(10 + k, 0, object=obj((-1 + 2 * k / 117, 1 - 2 * k / 117, 0))) for k in range(118)],
        "expect": "119 tracks, ids in profile ranges, no structural findings",
    }
    C["C13-isf"] = {
        "why": "J: 4 ISF objects between the bed and 2 dynamic objects (OAMD isf_index 0)",
        "program": {"beds": [["LFE"]], "isf_objects": 4, "dynamic_objects": 2}, "frames": 48000, "audio": {"hz": [100, 300, 320, 340, 360, 440, 880], "dbfs": -20},
        "oamd": [{"base": 0, "sample_offset": 0, "blocks": [[0, 1536]], "isf_index": 0,
                  "objects": [[{}], [{}], [{}], [{}], [{}], [{"pos": [0.0, 0.0, 0.0]}], [{"pos": [1.0, 0.0, 0.0]}]]}],
        "expect": "ISF tones (300-360 Hz) appear in no ADM/DAMF track; no diagnostic",
    }
    C["C14-bed-tfl"] = {
        "why": "K: a bed with Lfh/Rfh (Tfl/Tfr) channels the ADM profile has no label for",
        "program": {"beds": [["L", "R", "C", "LFE", "Lss", "Rss", "Lrs", "Rrs", "Lfh", "Rfh"]], "isf_objects": 0, "dynamic_objects": 1}, "frames": 48000,
        "audio": {"hz": [200, 210, 220, 100, 230, 240, 250, 260, 270, 280, 440], "dbfs": -20},
        "events": [ev(k, 0, bed={}) for k in (0, 1, 2, 3, 4, 5, 6, 7, 130, 131)] + [ev(10, 0, object=obj((0, 1, 0)))],
        "expect": "AdmWriter refuses (UnsupportedBedChannel) while DAMF is written",
    }
    C["C15-96k"] = {
        "why": "O: 96 kHz programme",
        "program": lfe, "frames": 96000, "sample_rate": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0))), ev(10, 48000, object=obj((1, 1, 0))), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "file written at 96000 with interpolationLength 250/96000 = 0.002604; profile-sample-rate finding; no warning from the writer",
    }
    C["C16-active-toggle"] = {
        "why": "active -> inactive -> active on object 10",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0))), ev(10, 24000, object=obj((-1, 1, 0), active=False)), ev(10, 48000, object=obj((-1, 1, 0))), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "inactive block carries gain 0.0 + importance 0; all three matched",
    }
    C["C17-zones"] = {
        "why": "zone constraints 1..5 and elevation off",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 6}, "frames": 48000, "audio": {"hz": [0] + [440 * (k + 1) for k in range(6)], "dbfs": -20},
        "events": [ev(3, 0, bed={})] + [ev(10 + k, 0, object=obj((0, 0, 0), zones=k + 1)) for k in range(5)] + [ev(15, 0, object=obj((0, 0, 0.5), elevation=False))],
        "expect": "zone rectangles equal the profile tables; DAMF zone names map 1:1; no profile findings",
    }
    C["C18-snap"] = {
        "why": "snap -> channelLock",
        "program": lfe, "frames": 48000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0), snap=True)), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "channelLock 1 on object 10 only",
    }
    C["C19-oamd-extras"] = {
        "why": "I: OAMD distance factor 2.0 and screen reference (0.5, 0.25) through Timeline::push",
        "program": lfe, "frames": 48000, "audio": tones,
        "oamd": [{"base": 0, "sample_offset": 0, "blocks": [[0, 1536]],
                  "objects": [[{}], [{"pos": [0.25, 0.25, 0.0], "distance": "2.0", "screen": [0.5, 0.25]}], [{"pos": [0.75, 0.75, 0.0], "distance": "inf"}]]}],
        "expect": "DAMF screenFactor 0.5 kept, distance nowhere; ADM carries neither",
    }
    C["C20-ramp-only"] = {
        "why": "H: a ramp-only change in the middle (duplicate block) and at the end (popped)",
        "program": lfe, "frames": 96000, "audio": tones,
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((-1, 1, 0), ramp=1536)), ev(10, 24000, object=obj((-1, 1, 0), ramp=32)),
                   ev(10, 48000, object=obj((1, 1, 0), ramp=1536)), ev(10, 72000, object=obj((1, 1, 0), ramp=32)), ev(11, 0, object=obj((1, 1, 0)))],
        "expect": "inexpressible_change 1 (block at 24000 duplicates 0), trailing_popped 1 (72000 absent)",
    }
    C["C21-bed-order"] = {
        "why": "bed coded in the order Rs, Ls, LFE: PCM must land under the right label (conform on)",
        "program": {"beds": [["Rss", "Lss", "LFE"]], "isf_objects": 0, "dynamic_objects": 1}, "frames": 48000, "audio": {"hz": [500, 600, 100, 440], "dbfs": -20},
        "events": [ev(k, 0, bed={}) for k in (5, 4, 3)] + [ev(10, 0, object=obj((0, 1, 0)))],
        "expect": "RC_Rss track carries 500 Hz, RC_Lss 600 Hz, RC_LFE 100 Hz",
    }
    C["C21b-bed-order-nbc"] = dict(C["C21-bed-order"], bed_conform=False, why=C["C21-bed-order"]["why"].replace("conform on", "conform off"))
    C["C22-rf64"] = {
        "why": "L: data crosses 4 GiB (2 tracks, silence): RF64 promotion",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 1}, "frames": 715827883, "bed_conform": False, "audio": {"hz": [0, 0]},
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((0, 1, 0))), ev(10, 715000000, object=obj((1, 1, 0)))],
        "expect": "RF64/ds64 with 64-bit sizes; every reader agrees on frame count",
        "heavy": True,
    }
    C["C22b-riff-max"] = {
        "why": "L: data just below 4 GiB but RIFF size above u32::MAX",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 1}, "frames": 715827882, "bed_conform": False, "audio": {"hz": [0, 0]},
        "events": [ev(3, 0, bed={}), ev(10, 0, object=obj((0, 1, 0)))],
        "expect": "RF64 because the RIFF size overflows even though data does not",
        "heavy": True,
    }
    for name, c in C.items():
        c.setdefault("case", name)
        c.setdefault("sample_rate", FS)
        c.setdefault("bed_conform", True)
    return C


def dominant_hz(x: np.ndarray, fs: int) -> float | None:
    x = np.asarray(x, dtype=np.float64)
    if not np.any(x):
        return None
    n = min(len(x), 1 << 16)
    spec = np.abs(np.fft.rfft(x[:n] * np.hanning(n)))
    return float(np.argmax(spec) * fs / n)


def track_tones(path: str) -> list:
    c = riff.scan(path)
    return [dominant_hz(riff.read_track_all(path, c, i), c.fmt.sample_rate) for i in range(c.fmt.channels)]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--only", default=None)
    ap.add_argument("--heavy", action="store_true", help="include the >4 GiB cases")
    ap.add_argument("--ct", action="store_true", help="also convert each harness DAMF with the Conversion Tool")
    a = ap.parse_args()
    only = set(a.only.split(",")) if a.only else None
    cdir = os.path.join(a.work, "cases")
    odir = os.path.join(a.work, "out")
    os.makedirs(cdir, exist_ok=True)
    rpath = os.path.join(a.work, "harness-report.json")
    report = json.load(open(rpath, encoding="utf-8")) if os.path.isfile(rpath) else {}
    for name, c in cases().items():
        if only is not None and name not in only:
            continue
        if c.get("heavy") and not a.heavy and only is None:
            continue
        spec = {k: v for k, v in c.items() if k not in ("why", "expect", "heavy")}
        cpath = os.path.join(cdir, f"{name}.json")
        with open(cpath, "w", encoding="utf-8", newline="\n") as f:
            json.dump(spec, f, indent=1)
        out = os.path.join(odir, name)
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([a.exe, cpath, out], capture_output=True, text=True, errors="replace")
        entry = {"why": c["why"], "expect": c["expect"], "exit": p.returncode, "stderr": p.stderr.strip()[-1000:]}
        sp = os.path.join(out, f"{name}.summary.json")
        summ = json.load(open(sp, encoding="utf-8")) if os.path.isfile(sp) else None
        entry["summary"] = summ
        adm = os.path.join(out, f"{name}.wav")
        base = os.path.join(out, name)
        if summ and "adm" in summ and os.path.isfile(base + ".atmos"):
            try:
                rep = compare(base, adm, 0.0, "effective", "window" if c.get("heavy") else "full", 10.0, name)
                L = rep["ledger"]
                entry["compare"] = {
                    "classes": L["classes"], "defects": L["defects"][:10], "loss": L["loss"], "loss_items": L.get("loss_items", [])[:12],
                    "identity_ok": L["identity_ok"], "tiling": L["tiling"], "bed_changes_lost": L["bed_changes_lost"],
                    "items": [i for i in L["items"] if i["cls"] in ("synthetic_block0", "beyond_end", "superseded_same_pos", "trailing_popped", "inexpressible_change")][:12],
                    "pcm": [{k: p_[k] for k in ("role", "label", "damf_track", "adm_track", "identical", "rms_a", "rms_b")} for p_ in rep["pcm"]["pairs"]],
                    "adm_findings": [f for f in rep["adm"]["findings"]], "damf_findings": [f for f in rep["damf"]["findings"]],
                    "adm_source": {k: rep["adm"]["source"][k] for k in ("frames", "channels", "sample_rate", "fourcc", "rf64", "ds64", "element_counts", "chunks")},
                    "adm_objects": rep["adm"]["objects"], "adm_beds": rep["adm"]["beds"],
                    "trajectory_max_e": {k: v["max_e"] for k, v in rep["trajectory_loss"].items()},
                }
                sc = normalise.from_adm(adm)
                entry["adm_blocks"] = {o.ordinal: [{"t": e.t, "dur": e.dur, "pos": e.pos, "gain": e.gain, "imp": e.importance, "size": e.size, "interp": e.interp["len_samples"] if e.interp else None, "zones": e.zones["names"], "snap": e.snap, "active": e.active, "extra": e.extra} for e in o.events] for o in sc.objects}
                sd = normalise.from_damf(base)
                entry["damf_states"] = {o.ordinal: [{"t": e.t, "pos": e.pos, "gain": e.gain, "imp": e.importance, "size": e.size, "ramp": e.ramp, "zones": e.zones["names"], "snap": e.snap, "active": e.active, "screen": e.screen_factor} for e in o.events] for o in sd.objects}
                entry["damf_bed_events"] = {b.label: [(e.t, e.gain["db"], e.active) for e in b.events] for b in sd.beds}
            except Exception as e:  # noqa: BLE001
                entry["compare_error"] = f"{type(e).__name__}: {e}"
            if name.startswith("C21") or name.startswith("C13"):
                try:
                    entry["adm_track_tones"] = track_tones(adm)
                    entry["adm_track_labels"] = [(t.index, t.role, t.label) for t in normalise.from_adm(adm).tracks]
                except Exception as e:  # noqa: BLE001
                    entry["tones_error"] = str(e)
        if a.ct and os.path.isfile(base + ".atmos") and not c.get("heavy"):
            ctdir = os.path.join(out, "ct")
            r = dolby.conversion_tool(base + ".atmos", ctdir, "wav", timeout=1800)
            entry["ct"] = {"exit": r["run"]["exit_code"], "findings": r["findings"]}
            wav = os.path.join(ctdir, "output.wav")
            if os.path.isfile(wav):
                try:
                    sc = normalise.from_adm(wav)
                    entry["ct"]["adm_blocks"] = {o.ordinal: [{"t": e.t, "dur": e.dur, "pos": e.pos, "gain": e.gain, "imp": e.importance, "size": e.size, "interp": e.interp["len_samples"] if e.interp else None, "zones": e.zones["names"], "snap": e.snap, "active": e.active, "extra": e.extra} for e in o.events] for o in sc.objects}
                    entry["ct"]["beds"] = [(b.label, b.track) for b in sc.beds]
                    entry["ct"]["channels"] = sc.source["channels"]
                    entry["ct"]["findings_adm"] = [f.__dict__ for f in sc.findings]
                    rep = compare(base, wav, 0.0, "effective", "window", 10.0, name + "/ct")
                    entry["ct"]["pcm"] = [{k: p_[k] for k in ("role", "label", "damf_track", "adm_track", "identical", "rms_a", "rms_b")} for p_ in rep["pcm"]["pairs"]]
                    entry["ct"]["ledger_classes"] = rep["ledger"]["classes"]
                    entry["ct"]["loss"] = rep["ledger"]["loss"]
                except Exception as e:  # noqa: BLE001
                    entry["ct"]["error"] = f"{type(e).__name__}: {e}"
        report[name] = entry
        cm = entry.get("compare", {})
        print(f"{name:20} exit {p.returncode} adm={'ok' if summ and 'adm' in summ else summ.get('adm_error') if summ else 'n/a'} classes={cm.get('classes')} loss={cm.get('loss')} findings={[f['kind'] for f in cm.get('adm_findings', [])]}"
              + (f" ct: gain-present={sum(1 for bl in entry['ct'].get('adm_blocks', {}).values() for b in bl if b['gain']['present'])}" if entry.get("ct") and "adm_blocks" in entry["ct"] else ""), flush=True)
    with open(os.path.join(a.work, "harness-report.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(report, f, indent=1, default=lambda o: sorted(o) if isinstance(o, set) else str(o))
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
