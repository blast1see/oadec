#!/usr/bin/env python3
"""Raw OAMD (``oadec oamd --dump``) against DAMF states and ADM blocks: timing and coordinates.

    python run_oamd_timing.py --oadec <exe> --stream <file.thd> --damf <base> --adm <file.wav> --out <json>

For every OAMD update the dump gives ``sample_offset`` and per block
``(block_offset_factor, ramp_duration)``; the event time is
``AU*40 + sample_offset + 32*block_offset_factor`` (TS 103 420 clause 5.6.2).
The script checks that (a) DAMF ``samplePos`` values and (b) ADM ``rtime``
sample positions are exactly those times -- including the ``32 x
block_offset_factor`` term -- and that the DAMF/ADM positions equal the OAMD
room position mapped by ``X = 2x-1, Y = 1-2y, Z = z`` to within the dump's
four decimals.  TrueHD only (the dump command refuses E-AC-3).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from collections import Counter

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import normalise, oamd_dump, provenance  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--oadec", required=True)
    ap.add_argument("--stream", required=True)
    ap.add_argument("--damf", required=True)
    ap.add_argument("--adm", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--au-samples", type=int, default=40)
    a = ap.parse_args()
    rec = provenance.run([a.oadec, "oamd", a.stream, "--dump", "100000000"], timeout=3600)
    if rec.exit_code not in (0, 7):
        print("oamd dump failed", rec.exit_code, rec.stderr[-500:])
        return 1
    dump = oamd_dump.parse(rec.stdout)
    events = oamd_dump.events(dump, a.au_samples)
    d = normalise.from_damf(a.damf)
    s = normalise.from_adm(a.adm)
    # OAMD object index -> DAMF ordinal: the coded bed objects come first in the OAMD object list
    u0 = dump.units[0]
    bed_n = sum(1 for (i, b), o in u0.objects.items() if b == 0 and o.bed)
    per_obj = {}
    bof_hist = Counter(e.block_offset_factor for e in events)
    so_hist = Counter(e.sample_offset for e in events)
    ramp_hist = Counter(e.ramp for e in events)
    tot = {"oamd_updates": 0, "oamd_distinct_times": 0, "damf_states": 0, "adm_blocks": 0, "damf_times_in_oamd": 0, "adm_times_in_oamd": 0,
           "adm_times_not_in_oamd": 0, "damf_times_not_in_oamd": 0, "events_with_block_offset": 0, "pos_max_delta_damf": 0.0, "pos_max_delta_adm": 0.0, "pos_checked": 0}
    for o in d.objects:
        idx = bed_n + o.ordinal - 1
        oe = [e for e in events if e.obj == idx]
        # keep only the last update per time (same-position updates: the last wins, as both writers do)
        by_t = {}
        for e in oe:
            by_t[e.t] = e
        oamd_times = set(by_t)
        damf_times = {e.t for e in o.events}
        ao = next((x for x in s.objects if x.ordinal == o.ordinal), None)
        adm_times = {b.t for b in ao.events} if ao else set()
        tot["oamd_updates"] += len(oe)
        tot["oamd_distinct_times"] += len(oamd_times)
        tot["damf_states"] += len(damf_times)
        tot["adm_blocks"] += len(adm_times)
        tot["damf_times_in_oamd"] += len(damf_times & oamd_times)
        tot["damf_times_not_in_oamd"] += len(damf_times - oamd_times)
        tot["adm_times_in_oamd"] += len(adm_times & oamd_times)
        tot["adm_times_not_in_oamd"] += len(adm_times - oamd_times - {0})
        tot["events_with_block_offset"] += sum(1 for e in oe if e.block_offset_factor)
        # positions: the dump prints four decimals of the OAMD room position
        dmax = amax = 0.0
        dpos = {e.t: e.pos for e in o.events}
        apos = {b.t: b.pos for b in (ao.events if ao else [])}
        for t, e in by_t.items():
            exp = e.pos_damf
            if t in dpos:
                dmax = max(dmax, max(abs(float(x) - y) for x, y in zip(dpos[t], exp)))
                tot["pos_checked"] += 1
            if t in apos:
                amax = max(amax, max(abs(float(x) - y) for x, y in zip(apos[t], exp)))
        tot["pos_max_delta_damf"] = max(tot["pos_max_delta_damf"], dmax)
        tot["pos_max_delta_adm"] = max(tot["pos_max_delta_adm"], amax)
        per_obj[o.ordinal] = {"oamd_updates": len(oe), "distinct_times": len(oamd_times), "damf_states": len(damf_times), "adm_blocks": len(adm_times),
                              "damf_not_in_oamd": sorted(damf_times - oamd_times)[:10], "adm_not_in_oamd": sorted(adm_times - oamd_times - {0})[:10],
                              "block_offset_events": sum(1 for e in oe if e.block_offset_factor), "pos_max_delta_damf": dmax, "pos_max_delta_adm": amax}
    # every ADM rtime with a non-zero block offset term: is it exactly reproduced?
    bo_times = {e.t for e in events if e.block_offset_factor}
    adm_all = {b.t for o in s.objects for b in o.events}
    damf_all = {e.t for o in d.objects for e in o.events}
    rep = {
        "stream": provenance.file_record(a.stream), "adm": os.path.abspath(a.adm), "damf": os.path.abspath(a.damf),
        "oamd_dump_exit": rec.exit_code, "units": len(dump.units), "footer": dump.footer,
        "sample_offset_hist": dict(so_hist), "block_offset_factor_hist": dict(bof_hist), "ramp_hist": dict(ramp_hist),
        "totals": tot,
        "block_offset_times": {"count": len(bo_times), "in_adm": len(bo_times & adm_all), "in_damf": len(bo_times & damf_all),
                               "examples_missing_from_adm": sorted(bo_times - adm_all)[:10]},
        "per_object": per_obj,
        "coordinate_map": "X = 2x-1, Y = 2(0.5-y), Z = z (dump prints 4 decimals of x,y in 0..1 -> 1e-4 in room units; tolerance 1.5e-4)",
        "coded_bed_objects": bed_n,
        "pos_ok": tot["pos_max_delta_damf"] <= 1.5e-4 and tot["pos_max_delta_adm"] <= 1.5e-4,
    }
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(rep, f, indent=1, default=str)
        f.write("\n")
    print(json.dumps({k: rep[k] for k in ("units", "sample_offset_hist", "block_offset_factor_hist", "ramp_hist", "totals", "block_offset_times", "pos_ok")}, default=str))
    return 0


if __name__ == "__main__":
    sys.exit(main())
