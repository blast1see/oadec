#!/usr/bin/env python3
"""Compare a DAMF set with an ADM BWF of the same programme.

    python run_compare.py --damf <base> --adm <file.wav> --out <report.json>
        [--pos-tol 0] [--mode effective|strict] [--pcm full|window|none] [--window-seconds 20]
        [--label <name>] [--reference-adm <dolby.wav>]

Produces one JSON with: container findings, reference-graph findings, the
reconciliation ledger, per-object trajectory loss, per-track PCM identity,
the track-pairing matrix and (optionally) the same against a reference ADM.
"""
from __future__ import annotations

import argparse
import json
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import caf, compare_events, compare_pcm, normalise, provenance, riff, trajectory  # noqa: E402

DAMF_TO_RC = {"L": "RC_L", "R": "RC_R", "C": "RC_C", "LFE": "RC_LFE", "Lss": "RC_Lss", "Rss": "RC_Rss",
              "Lrs": "RC_Lrs", "Rrs": "RC_Rrs", "Lts": "RC_Lts", "Rts": "RC_Rts", "Ls": "RC_Ls", "Rs": "RC_Rs"}


def track_pairs(d: normalise.Scene, a: normalise.Scene) -> list:
    """[(damf track, adm track, role, label)] by declaration, never by position."""
    pairs = []
    adm_beds = {b.label: b for b in a.beds}
    for b in d.beds:
        rc = DAMF_TO_RC.get(b.label)
        ab = adm_beds.get(rc)
        pairs.append((b.track, ab.track if ab else None, "bed", b.label))
    adm_objs = {o.ordinal: o for o in a.objects}
    for o in d.objects:
        ao = adm_objs.get(o.ordinal)
        pairs.append((o.track, ao.track if ao else None, "object", f"obj{o.ordinal}"))
    return pairs


def pcm_compare(d: normalise.Scene, a: normalise.Scene, damf_base: str, adm_path: str, mode: str, window_seconds: float) -> dict:
    out = {"pairs": [], "matrix": None}
    if mode == "none":
        return out
    info = caf.read_header(d.source["audio_path"])
    c = riff.scan(adm_path)
    fs = info.sample_rate
    n_win = int(window_seconds * fs)
    pairs = track_pairs(d, a)
    live = [(dt, at, role, label) for dt, at, role, label in pairs if at is not None]
    for dt, at, role, label in pairs:
        if at is None:
            out["pairs"].append({"role": role, "label": label, "damf_track": dt, "adm_track": None, "error": "no ADM counterpart"})
    if mode == "window":
        for dt, at, role, label in live:
            x = np.concatenate(list(_take(caf.read_track(d.source["audio_path"], info, dt, block_frames=n_win), n_win)))
            y = np.concatenate(list(_take(riff.read_track(adm_path, c, at, block_frames=n_win), n_win)))
            r = compare_pcm.compare_tracks(x, y).to_json()
            r.update({"role": role, "label": label, "damf_track": dt, "adm_track": at})
            out["pairs"].append(r)
    else:
        # one pass over both files for every declared pair (a full film is read once, not once per track)
        results = compare_pcm.compare_interleaved(compare_pcm.caf_frames(d.source["audio_path"], info), compare_pcm.wav_frames(adm_path, c), [(dt, at) for dt, at, _r, _l in live])
        for (dt, at, role, label), res in zip(live, results):
            r = res.to_json()
            r.update({"role": role, "label": label, "damf_track": dt, "adm_track": at})
            out["pairs"].append(r)
    # pairing matrix on a window over every track of both files
    dt_all = [np.concatenate(list(_take(caf.read_track(d.source["audio_path"], info, i, block_frames=n_win), n_win))) for i in range(info.channels)]
    at_all = [np.concatenate(list(_take(riff.read_track(adm_path, c, i, block_frames=n_win), n_win))) for i in range(c.fmt.channels)]
    m = compare_pcm.distance_matrix(dt_all, at_all)
    best = compare_pcm.best_matches(m)
    expected = {dt: at for dt, at, _r, _l in pairs if at is not None}
    silent_d = [i for i, x in enumerate(dt_all) if not np.any(x)]
    silent_a = [i for i, x in enumerate(at_all) if not np.any(x)]
    rows = []
    for b in best:
        tie = bool(b.margin == 0.0)
        # a silent track is at distance 0 from every other silent track: the declared
        # pairing is confirmed if the declared partner is among the zero-distance matches
        declared_ok = (expected.get(b.i) == b.j) or (expected.get(b.i) is not None and m[b.i, expected[b.i]] == b.distance)
        rows.append({"damf": b.i, "adm": b.j, "distance": b.distance, "margin": b.margin, "tie": tie,
                     "expected_adm": expected.get(b.i), "as_declared": declared_ok, "silent": b.i in silent_d})
    out["matrix"] = {
        "window_samples": n_win,
        "damf_tracks": info.channels, "adm_tracks": c.fmt.channels,
        "best": rows,
        "all_declared_pairs_confirmed": all(r["as_declared"] for r in rows if r["expected_adm"] is not None),
        "non_silent_pairs_unique": all(not r["tie"] for r in rows if not r["silent"]),
        "silent_damf_tracks": silent_d,
        "silent_adm_tracks": silent_a,
    }
    return out


def _take(it, n):
    got = 0
    for blk in it:
        if got >= n:
            break
        yield blk[: n - got]
        got += len(blk)


def compare(damf_base: str, adm_path: str, pos_tol: float, mode: str, pcm_mode: str, window_seconds: float, label: str | None) -> dict:
    d = normalise.from_damf(damf_base)
    a = normalise.from_adm(adm_path)
    ledger = compare_events.reconcile(d, a, mode=mode, pos_tol=pos_tol)
    adm_by = {o.ordinal: o for o in a.objects}
    loss = {}
    disp = {}
    for o in d.objects:
        ao = adm_by.get(o.ordinal)
        if ao is None or not o.events or not ao.events:
            continue
        frames = d.source.get("frames") or a.source.get("frames")
        loss[o.ordinal] = trajectory.loss(o.events, ao.events, frames).to_json()
        disp[o.ordinal] = trajectory.displacements(o.events)
    # a faithful-ADM self-check: the evaluator must return 0 against a synthetic ADM carrying the ramps
    selfcheck = {}
    for o in d.objects:
        if o.events:
            frames = d.source.get("frames")
            selfcheck[o.ordinal] = max(trajectory.loss(o.events, trajectory.faithful_adm(o.events, frames), frames).max_e.values() or [0.0])
    rep = {
        "label": label,
        "utc": provenance.utc_now(),
        "damf": {"base": os.path.abspath(damf_base), "source": d.source, "findings": [f.__dict__ for f in d.findings],
                 "beds": [(b.element_id, b.label, b.track, len(b.events)) for b in d.beds],
                 "objects": [(o.element_id, o.ordinal, o.track, len(o.events)) for o in d.objects]},
        "adm": {"path": os.path.abspath(adm_path), "source": a.source, "findings": [f.__dict__ for f in a.findings],
                "tracks": [t.__dict__ for t in a.tracks],
                "beds": [(b.label, b.track, b.pos) for b in a.beds],
                "objects": [(o.audio_object, o.ordinal, o.track, o.channel_format, len(o.events)) for o in a.objects]},
        "ledger": ledger.to_json(),
        "trajectory_loss": loss,
        "trajectory_selfcheck_max_e": selfcheck,
        "displacements": {k: {"gap_hist": _hist(v["gaps"]), "ramp_hist": _hist(v["ramps"]), "n": len(v["gaps"])} for k, v in disp.items()},
        "timecode_roundtrip": timecode_roundtrip(a),
        "pcm": pcm_compare(d, a, damf_base, adm_path, pcm_mode, window_seconds),
    }
    return rep


def _hist(values: list) -> dict:
    h = {}
    for v in values:
        h[str(v)] = h.get(str(v), 0) + 1
    return dict(sorted(h.items(), key=lambda kv: -kv[1]))


def timecode_roundtrip(a: normalise.Scene) -> dict:
    """Every rtime/duration re-encoded from its recovered sample must reproduce the string."""
    from admaudit import timecode
    fs = a.source["sample_rate"]
    total = mism = inexact_t = inexact_d = 0
    examples = []
    for o in a.objects:
        for e in o.events:
            for raw, val in ((e.t_raw, e.t), (e.dur_raw, e.dur)):
                if raw is None or val is None:
                    continue
                total += 1
                if timecode.encode(val, fs) != raw:
                    mism += 1
                    if len(examples) < 10:
                        examples.append({"obj": o.ordinal, "raw": raw, "samples": val, "re_encoded": timecode.encode(val, fs)})
            inexact_t += 0 if e.t_exact else 1
            inexact_d += 0 if e.dur_exact else 1
    return {"timecodes": total, "re_encode_mismatches": mism, "inexact_rtime": inexact_t, "inexact_duration": inexact_d, "examples": examples}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--damf", required=True)
    ap.add_argument("--adm", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--pos-tol", type=float, default=0.0)
    ap.add_argument("--mode", default="effective", choices=["effective", "strict"])
    ap.add_argument("--pcm", default="full", choices=["full", "window", "none"])
    ap.add_argument("--window-seconds", type=float, default=20.0)
    ap.add_argument("--label", default=None)
    a = ap.parse_args()
    rep = compare(a.damf, a.adm, a.pos_tol, a.mode, a.pcm, a.window_seconds, a.label)
    os.makedirs(os.path.dirname(os.path.abspath(a.out)) or ".", exist_ok=True)
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(rep, f, indent=1, default=lambda o: sorted(o) if isinstance(o, set) else str(o))
        f.write("\n")
    L = rep["ledger"]
    print(f"ledger classes: {L['classes']}")
    print(f"defects: {len(L['defects'])}; loss: {L['loss']}; identity_ok: {L['identity_ok']}; tiling: {L['tiling']}")
    print(f"trajectory max_e per object: { {k: v['max_e'] for k, v in rep['trajectory_loss'].items()} }")
    print(f"selfcheck: {rep['trajectory_selfcheck_max_e']}")
    print(f"timecodes: {rep['timecode_roundtrip']}")
    if rep["pcm"]["pairs"]:
        ident = [p for p in rep["pcm"]["pairs"] if p.get("identical")]
        mx = rep["pcm"]["matrix"]
        print(f"pcm identical pairs: {len(ident)}/{len(rep['pcm']['pairs'])}; declared pairs confirmed: {mx['all_declared_pairs_confirmed']}; non-silent unique: {mx['non_silent_pairs_unique']}; silent adm tracks: {mx['silent_adm_tracks']}")
    print(f"findings damf={len(rep['damf']['findings'])} adm={len(rep['adm']['findings'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
