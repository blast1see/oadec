#!/usr/bin/env python3
"""Renderer leg (section 26): render ADM variants with the EBU ADM Renderer and compare speaker outputs.

    python run_render.py --work <dir> --out <json> --layouts 0+2+0,0+5+0,4+5+0,4+7+0
        --adm label=path [label=path ...] [--realramps "label=adm_path|damf_base"]

``--realramps L=adm|damf`` builds one more variant of ``adm`` whose object
blocks carry ``interpolationLength = rampLength`` of the matching DAMF state
(the "faithful" mapping the profile forbids), so that the rendered effect of
the fixed 250-sample ramp can be measured against a BS.2127 renderer.

The renderer is an independent reference only: identical renders say the two
files describe the same scene *to this renderer*; they do not prove metadata
equality (a renderer may ignore what was lost).  Speaker outputs are compared
sample-exactly and with lag / correlation / SDR.
"""
from __future__ import annotations

import argparse
import itertools
import json
import os
import re
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import compare_pcm, mutate, normalise, provenance, riff  # noqa: E402

EAR = os.path.join(os.path.dirname(sys.executable), "ear-render.exe")


def real_ramp_variant(adm: str, damf_base: str, out: str) -> dict:
    a = normalise.from_adm(adm)
    d = normalise.from_damf(damf_base)
    fs = a.source["sample_rate"]
    dm = {o.ordinal: o for o in d.objects}
    spec = {}
    replaced = 0
    for o in a.objects:
        by_t = {s.t: s for s in dm[o.ordinal].events} if o.ordinal in dm else {}
        per = {}
        for k, b in enumerate(o.events):
            if k == 0:
                continue
            s = by_t.get(b.t)
            if s is not None and s.ramp is not None:
                per[k] = int(s.ramp)
                replaced += 1
        if per:
            spec[o.channel_format] = per
    mutate.set_interpolation(adm, out, spec, fs)
    return {"blocks_with_real_ramp": replaced, "objects": len(spec)}


STREAM_PACK_REF = re.compile(r"(<audioStreamFormat[^>]*>(?:(?!</audioStreamFormat>).)*?)(\s*<audioPackFormatIDRef>[^<]*</audioPackFormatIDRef>)", re.S)


def ear_compatible(adm: str, out: str) -> dict:
    """Copy ``adm`` with the audioPackFormatIDRef removed from every audioStreamFormat.

    The Dolby Atmos Master ADM profile (table 5, note) deliberately carries both a
    channel-format and a pack-format reference in audioStreamFormat; the EBU ADM
    Renderer follows BS.2076 strictly and refuses such files.  The same edit is
    applied to every input so that the comparison stays fair; nothing else changes.
    """
    count = {"n": 0}

    def fn(text: str) -> str:
        new, n = STREAM_PACK_REF.subn(lambda m: m.group(1), text)
        count["n"] = n
        return new

    mutate.edit_axml(adm, out, fn)
    return {"removed_pack_refs": count["n"]}


def render(adm: str, layout: str, out: str) -> dict:
    rec = provenance.run([EAR, "-s", layout, adm, out], timeout=7200)
    return {"run": rec.to_json(3000), "ok": rec.exit_code == 0 and os.path.isfile(out)}


def read_all(path: str) -> list:
    c = riff.scan(path)
    return [riff.read_track_all(path, c, i) for i in range(c.fmt.channels)], c


def sdr_db(ref: np.ndarray, test: np.ndarray) -> float | None:
    ref = ref.astype(np.float64)
    test = test.astype(np.float64)
    n = min(ref.size, test.size)
    e = ref[:n] - test[:n]
    pr = float(np.dot(ref[:n], ref[:n]))
    pe = float(np.dot(e, e))
    if pr == 0.0:
        return None
    if pe == 0.0:
        return float("inf")
    return 10 * np.log10(pr / pe)


def compare_renders(a: str, b: str, max_lag: int = 4096) -> dict:
    ta, ca = read_all(a)
    tb, cb = read_all(b)
    out = {"channels": (ca.fmt.channels, cb.fmt.channels), "frames": (ca.frames, cb.frames), "per_channel": []}
    for i, (x, y) in enumerate(zip(ta, tb)):
        cmp_ = compare_pcm.compare_tracks(x, y)
        lag = compare_pcm.lag(x[: 48000 * 20], y[: 48000 * 20], max_lag) if np.any(x) and np.any(y) else 0
        out["per_channel"].append({"ch": i, "identical": cmp_.identical, "differing": cmp_.differing, "max_abs": cmp_.max_abs, "rms_a": cmp_.rms_a, "rms_b": cmp_.rms_b,
                                   "corr": cmp_.corr, "sdr_db": sdr_db(x, y), "lag": lag, "peak_a": cmp_.peak_a, "peak_b": cmp_.peak_b})
    ident = all(p["identical"] for p in out["per_channel"])
    out["all_identical"] = ident
    sdrs = [p["sdr_db"] for p in out["per_channel"] if p["sdr_db"] is not None and p["sdr_db"] != float("inf")]
    out["min_sdr_db"] = min(sdrs) if sdrs else None
    out["max_abs_overall"] = max(p["max_abs"] for p in out["per_channel"]) if out["per_channel"] else None
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--layouts", default="0+2+0,0+5+0,4+5+0,4+7+0")
    ap.add_argument("--adm", nargs="+", required=True, help="label=path")
    ap.add_argument("--realramps", nargs="*", default=[], help="label=adm_path|damf_base")
    a = ap.parse_args()
    os.makedirs(a.work, exist_ok=True)
    files = {}
    for item in a.adm:
        label, path = item.split("=", 1)
        files[label] = path
    variants = {}
    for item in a.realramps:
        label, rest = item.split("=", 1)
        adm, damf = rest.split("|", 1)   # '|' because Windows paths carry ':'

        out = os.path.join(a.work, f"{label}.wav")
        variants[label] = real_ramp_variant(adm, damf, out)
        files[label] = out
    layouts = a.layouts.split(",")
    rep = {"utc": provenance.utc_now(), "ear": provenance.file_record(EAR), "inputs": {k: provenance.file_record(v) for k, v in files.items()},
           "realramp_variants": variants, "ear_compatibility": {}, "renders": {}, "comparisons": {}}
    for label in list(files):
        out = os.path.join(a.work, f"{label}__ear.wav")
        rep["ear_compatibility"][label] = ear_compatible(files[label], out)
        files[label] = out
    for label, path in files.items():
        for lay in layouts:
            out = os.path.join(a.work, f"{label}__{lay.replace('+', '')}.wav")
            r = render(path, lay, out)
            rep["renders"][f"{label}/{lay}"] = {"ok": r["ok"], "exit": r["run"]["exit_code"], "seconds": r["run"]["seconds"], "stderr_tail": r["run"]["stderr_tail"][-600:], "output": out if r["ok"] else None}
            print(f"render {label} {lay}: exit {r['run']['exit_code']} in {r['run']['seconds']:.1f}s", flush=True)
    for lay in layouts:
        for la, lb in itertools.combinations(files.keys(), 2):
            ra = rep["renders"][f"{la}/{lay}"]["output"]
            rb = rep["renders"][f"{lb}/{lay}"]["output"]
            if not ra or not rb:
                continue
            c = compare_renders(ra, rb)
            rep["comparisons"][f"{la} vs {lb} @ {lay}"] = c
            print(f"compare {la} vs {lb} @ {lay}: identical={c['all_identical']} min_sdr={c['min_sdr_db']} max_abs={c['max_abs_overall']} lags={[p['lag'] for p in c['per_channel']]}", flush=True)
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(rep, f, indent=1, default=str)
        f.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
