#!/usr/bin/env python3
"""The enhanced coupling amplitude sweep against Dolby's decoder, written down.

Round 4 measured the default (amplitude-only) reading of enhanced coupling
against the Dolby Reference Player's `dlbac3dec` with a script that never
entered the repository; this is that measurement, step by step, so it can be
run again on any carrier with standard coupling.

    python tools/ecpl_amp_sweep.py make    --oadec OADEC --clip SRC.ec3 --work DIR [--plan-file P --name N]
    python tools/ecpl_amp_sweep.py dolby   --clip SRC.ec3 --work DIR [--names N,...] [--layout 2.0]
    python tools/ecpl_amp_sweep.py analyse --work DIR [--name N] [--layout L] [--band LO HI] [--json OUT]
    python tools/ecpl_amp_sweep.py matrix  --oadec OADEC --clip SRC.ec3 --work DIR --plans PLANDIR
    python tools/ecpl_amp_sweep.py cells   --work DIR --model PCM --carrier NAME --role fit|holdout
                                           --source SIGNAL --aht 0|1 [--layout L] [--band LO HI] --out C.json

`make` rewrites the clip's standard coupling as enhanced coupling with
`oadec eac3-ecpl-inject --plan` (by default the plan `amp sweep-frames /
angle 0 / chaos 0 / interp 0 / trans never`: frame f carries amplitude code
f % 32 in every band of every coupled channel) and decodes the clip and the
rewritten stream with `oadec decode --format pcm`, each twice. `dolby`
decodes the same streams with `dlbac3dec` (`out-ch-config=7` for 5.1, `0` for
2/0; DRC off, `drop-delay=true`), each twice; the player is found under
`OADEC_DOLBY_PLAYER`, else under `%ProgramFiles%/Dolby/Dolby Reference
Player`. `analyse` and `cells` need numpy and read only the decodes.

What `analyse` reports, over the coupling region (by default bins 109 to 193
of the 256 a block has, 10218,75 to 18093,75 Hz: the region of the clip the
sweep was first measured on; `--band` gives another carrier's, which
`ecpl_carrier_probe` reads):

- per amplitude code (the frame's number modulo 32) and channel, the SNR of
  oadec's decode of the rewritten stream against Dolby's decode of it,
  beside the same SNR for the clip itself (standard coupling) over the same
  frames;
- the pairs of 32 codes and the coupled channels where enhanced coupling
  comes out further from Dolby than standard coupling, how far, and how many
  of them are beyond the floor of the measurement: what an untouched channel
  moves between the two decodes (the centre of a 5.1 carrier whose centre is
  not coupled); a 2/0 carrier has none, and its floor is not measured. What
  each coupled channel moves in a band of its own under the coupling region
  is reported beside it, as information: that band stands near 90 dB where
  the coupling band stands near 3 dB, so it is no floor for it;
- per code and channel, the least-squares gain of Dolby's decode on oadec's
  (a complex projection of the short-time spectra over the band) and the SNR
  left after that gain.

`matrix` injects every `*.plan` of a folder, decodes each stream once with
oadec and once with Dolby, and groups the streams whose decodes are the same
bytes: the question whether a field is read at all. `cells` sets a decode of
the rewritten stream with an enhanced coupling model (`ecpl_model_decode`)
beside the default decode, both against Dolby's, cell by cell, and writes
the carrier file `tools/ecpl_promotion.py` judges.

The frame segmentation follows the round-4 script: Dolby leaves out the first
256 samples (`drop-delay=true`), oadec's decode is shifted by them, and both
are cut into 1536-sample segments with a Hann window; segment k is counted as
amplitude code k % 32, and segment 0 is left out (it has no predecessor). A
segment spans the last 1280 samples of frame k and the first 256 of frame
k + 1, so a bucket is mostly, not purely, one code.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

PLAN_AMP = "amp sweep-frames\nangle 0\nchaos 0\ninterp 0\ntrans never\n"
SR = 48000
FRAME = 1536
LATE = 256  # samples `drop-delay=true` leaves out of Dolby's decode
CODES = 32
LAYOUTS = {
    "5.1": {"names": ["L", "R", "C", "LFE", "Ls", "Rs"], "coupled": ["L", "R", "Ls", "Rs"],
            "untouched": "C", "out_ch": 7},
    "2.0": {"names": ["L", "R"], "coupled": ["L", "R"], "untouched": None, "out_ch": 0},
}
DOLBY_ARGS = "drc-suppress=true drc-mode=custom-0 drc-cut=0 drc-boost=0 drop-delay=true"


def hz(b: int) -> float:
    return b * SR / 512


def md5(p: Path) -> str:
    return hashlib.md5(p.read_bytes()).hexdigest()


def sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def run(cmd: list[str], env: dict[str, str] | None = None) -> str:
    r = subprocess.run(cmd, capture_output=True, text=True, env=env)
    if r.returncode != 0:
        raise SystemExit(f"{Path(cmd[0]).name} failed ({r.returncode}): {r.stdout[-400:]} {r.stderr[-400:]}")
    return r.stdout


def manifest(work: Path) -> dict:
    p = work / "manifest.json"
    return json.loads(p.read_text()) if p.exists() else {}


def save_manifest(work: Path, m: dict) -> None:
    (work / "manifest.json").write_text(json.dumps(m, indent=1, sort_keys=True) + "\n")


def inject(oadec: str, clip: Path, plan_text: str, out: Path) -> None:
    plan = out.with_suffix(".plan")
    plan.write_text(plan_text)
    run([oadec, "eac3-ecpl-inject", str(clip), "-o", str(out), "--plan", str(plan)])


# ---------------------------------------------------------------- make


def cmd_make(a: argparse.Namespace) -> int:
    work = Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    clip = Path(a.clip)
    m = manifest(work)
    m["clip"] = {"name": clip.name, "bytes": clip.stat().st_size, "sha256": sha256(clip)}
    m["oadec"] = {"version": run([a.oadec, "--version"]).strip(), "sha256": sha256(Path(a.oadec))}
    plan_text = Path(a.plan_file).read_text() if a.plan_file else PLAN_AMP
    stream = work / f"{a.name}.ec3"
    inject(a.oadec, clip, plan_text, stream)
    m[a.name] = {"plan": plan_text, "md5": md5(stream), "sha256": sha256(stream)}
    for name, s in {"src": clip, a.name: stream}.items():
        sums = []
        for k in (1, 2):
            out = work / f"{name}.oadec{k}.f32"
            run([a.oadec, "decode", "--format", "pcm", "-o", str(out), str(s)])
            sums.append(md5(out))
        m.setdefault("decodes", {})[f"{name}.oadec"] = {"md5": sums[0], "repeat_same": sums[0] == sums[1]}
        print(f"{name}: oadec {sums[0]} (repeat {'same' if sums[0] == sums[1] else 'DIFFERENT'})")
    print(f"{a.name}.ec3 md5 {m[a.name]['md5']}")
    save_manifest(work, m)
    return 0


# ---------------------------------------------------------------- dolby


def player() -> Path:
    env = os.environ.get("OADEC_DOLBY_PLAYER")
    if env:
        return Path(env)
    return Path(os.environ.get("ProgramFiles", "")) / "Dolby" / "Dolby Reference Player"


def dolby_decode(src: Path, out: Path, layout: str = "5.1") -> None:
    p = player()
    gst = p / "gst-launch-1.0.exe"
    if not gst.exists():
        raise SystemExit(f"no gst-launch-1.0.exe under {p}; set OADEC_DOLBY_PLAYER")
    env = dict(os.environ)
    env["GST_PLUGIN_PATH"] = str(p / "gst-plugins")
    env["PATH"] = str(p) + os.pathsep + env.get("PATH", "")
    fwd = lambda x: str(x).replace("\\", "/")  # noqa: E731
    run([str(gst), "-q", "filesrc", f"location={fwd(src)}", "!", "dlbac3parse", "!", "dlbac3dec",
         f"out-ch-config={LAYOUTS[layout]['out_ch']}", *DOLBY_ARGS.split(), "!",
         "audio/x-raw,format=F32LE", "!", "filesink", f"location={fwd(out)}"], env)


def cmd_dolby(a: argparse.Namespace) -> int:
    work = Path(a.work)
    m = manifest(work)
    srcs = {"src": Path(a.clip)}
    for name in a.names.split(","):
        srcs[name] = work / f"{name}.ec3"
    for name, s in srcs.items():
        sums = []
        for k in (1, 2):
            out = work / f"{name}.dolby{k}.f32"
            dolby_decode(s, out, a.layout)
            sums.append(md5(out))
        m.setdefault("decodes", {})[f"{name}.dolby"] = {"md5": sums[0], "repeat_same": sums[0] == sums[1]}
        print(f"{name}: dolby {sums[0]} (repeat {'same' if sums[0] == sums[1] else 'DIFFERENT'})")
    m["dolby"] = {"element": f"dlbac3dec out-ch-config={LAYOUTS[a.layout]['out_ch']} {DOLBY_ARGS}"}
    save_manifest(work, m)
    return 0


# ---------------------------------------------------------------- matrix


def cmd_matrix(a: argparse.Namespace) -> int:
    work = Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    clip = Path(a.clip)
    plans = sorted(Path(a.plans).glob("*.plan"))
    if not plans:
        raise SystemExit(f"no *.plan in {a.plans}")
    rows = []
    for p in plans:
        name = p.stem
        stream = work / f"{name}.ec3"
        inject(a.oadec, clip, p.read_text(), stream)
        o, d = work / f"{name}.oadec.f32", work / f"{name}.dolby.f32"
        run([a.oadec, "decode", "--format", "pcm", "-o", str(o), str(stream)])
        dolby_decode(stream, d, a.layout)
        row = {"plan": name, "text": p.read_text(), "stream_md5": md5(stream),
               "oadec_md5": md5(o), "dolby_md5": md5(d)}
        if not a.keep:
            o.unlink()
            d.unlink()
        rows.append(row)
        print(f"{name:24} stream {row['stream_md5']}  oadec {row['oadec_md5']}  dolby {row['dolby_md5']}", flush=True)
    groups: dict[str, list[str]] = {}
    for r in rows:
        groups.setdefault(r["dolby_md5"], []).append(r["plan"])
    ogroups: dict[str, list[str]] = {}
    for r in rows:
        ogroups.setdefault(r["oadec_md5"], []).append(r["plan"])
    print(f"\nDolby: {len(groups)} distinct decodes of {len(rows)} streams")
    for k, v in groups.items():
        print(f"  {k}: {', '.join(v)}")
    print(f"oadec: {len(ogroups)} distinct decodes of {len(rows)} streams")
    out = {"clip": {"name": clip.name, "sha256": sha256(clip)}, "rows": rows,
           "dolby_groups": list(groups.values()), "oadec_groups": list(ogroups.values())}
    (work / "matrix.json").write_text(json.dumps(out, indent=1) + "\n")
    return 0


# ---------------------------------------------------------------- analysis


def geometry(a: argparse.Namespace):
    lay = LAYOUTS[a.layout]
    coupled = a.coupled.split(",") if a.coupled else lay["coupled"]
    untouched = None if a.coupled and lay["untouched"] in coupled else lay["untouched"]
    lo, hi = a.band
    width = min(hi - lo, lo - 4)  # a band of the channels' own, under the coupling region
    return lay["names"], coupled, untouched, (hz(lo), hz(hi)), (hz(lo - width), hz(lo))


def decodes(work: Path, names: list[str], stems: list[str], bands):
    """Aligned short-time spectra of every stem over each band: {stem: [band spectra]}."""
    import numpy as np

    ch = len(names)

    def load(stem: str):
        return np.fromfile(work / f"{stem}.f32", dtype="<f4").reshape(-1, ch).astype(np.float64)

    def spectra(x):
        n = x.shape[0] // FRAME
        seg = x[: n * FRAME].reshape(n, FRAME, ch) * np.hanning(FRAME)[None, :, None]
        s = np.fft.rfft(seg, axis=1)
        f = np.fft.rfftfreq(FRAME, 1 / SR)
        return [s[:, (f >= b[0]) & (f < b[1]), :] for b in bands]

    loaded = {s: load(s) for s in stems}
    n = min(min(v.shape[0] for k, v in loaded.items() if "dolby" not in k) - LATE,
            min(v.shape[0] for k, v in loaded.items() if "dolby" in k))
    out = {}
    for s, v in loaded.items():
        out[s] = spectra(v[:n] if "dolby" in s else v[LATE:LATE + n])
    return out


def snr_db(sig: float, err: float) -> float:
    import math

    if err <= 0:
        return float("inf")
    return 10 * math.log10(sig / err)


def cmd_analyse(a: argparse.Namespace) -> int:
    import numpy as np

    work = Path(a.work)
    names, coupled, untouched, band, own = geometry(a)
    name = a.name
    sp = decodes(work, names, [f"{name}.oadec1", f"{name}.dolby1", "src.oadec1", "src.dolby1"], [band, own])
    n = min(v[0].shape[0] for v in sp.values())
    codes = np.arange(n) % CODES

    def energy(s):
        return (np.abs(s[:n]) ** 2).sum(axis=1)

    per_band = []
    for b in (0, 1):
        oe, de = sp[f"{name}.oadec1"][b], sp[f"{name}.dolby1"][b]
        os_, ds = sp["src.oadec1"][b], sp["src.dolby1"][b]
        per_band.append((energy(de), energy(oe[:n] - de[:n]), energy(ds), energy(os_[:n] - ds[:n])))

    shown = [nm for nm in names if nm != "LFE"]
    table = []
    for k in range(CODES):
        mask = codes == k
        mask[0] = False
        row = {"code": k, "channels": {}}
        for ch, nm in enumerate(names):
            if nm == "LFE":
                continue
            cell: dict = {}
            sig_e, err_e, sig_s, err_s = per_band[0]
            se, ee, ss, es = (x[mask, ch].sum() for x in (sig_e, err_e, sig_s, err_s))
            if ss > 0 and se > 1e-12 * ss:
                cell["enhanced_db"] = snr_db(se, ee)
                cell["standard_db"] = snr_db(ss, es)
            sig_e, err_e, sig_s, err_s = per_band[1]
            se, ee, ss, es = (x[mask, ch].sum() for x in (sig_e, err_e, sig_s, err_s))
            if ss > 0 and se > 0:
                cell["own_enhanced_db"] = snr_db(se, ee)
                cell["own_standard_db"] = snr_db(ss, es)
            a_ = sp[f"{name}.oadec1"][0][:n][mask, :, ch].ravel()
            b_ = sp[f"{name}.dolby1"][0][:n][mask, :, ch].ravel()
            ea = float(np.vdot(a_, a_).real)
            if ea > 0:
                g = np.vdot(a_, b_) / ea
                r = b_ - g * a_
                cell["gain"] = float(abs(g))
                cell["gain_snr_db"] = snr_db(float(np.vdot(b_, b_).real), max(float(np.vdot(r, r).real), 1e-300))
            row["channels"][nm] = cell
        table.append(row)

    def diffs(chs, lo="enhanced_db", hi="standard_db"):
        out = []
        for row in table:
            for nm in chs:
                c = row["channels"][nm]
                if lo in c and c[lo] < c[hi]:
                    out.append((row["code"], nm, c[lo], c[hi]))
        return out

    worse = diffs(coupled)
    centre = diffs([untouched]) if untouched else []
    centre_floor = max((s - e for _, _, e, s in centre), default=0.0)
    own_low = diffs(coupled, "own_enhanced_db", "own_standard_db")
    own_floor = max((s - e for _, _, e, s in own_low), default=0.0)
    # The own band is not a floor for the coupling band: it stands near 90 dB
    # where the coupling band stands near 3 dB, and moves by other causes.
    floor = centre_floor if untouched else None
    gains = [row["channels"][nm]["gain"] for row in table for nm in coupled if "gain" in row["channels"][nm]]
    worse_r = [w for w in worse if round(w[2], 1) < round(w[3], 1)]
    beyond = [w for w in worse if w[3] - w[2] > round(floor, 1)] if floor is not None else []

    print(f"band {band[0]:.2f} to {band[1]:.2f} Hz; own band {own[0]:.2f} to {own[1]:.2f} Hz")
    print("SNR in the coupling band (dB), oadec default against dlbac3dec,")
    print("enhanced (amp code k) / standard coupling (same frames)")
    print("code " + "".join(f"{c:>16}" for c in shown))
    for row in table:
        line = f"{row['code']:4} "
        for nm in shown:
            c = row["channels"][nm]
            line += f"{c['enhanced_db']:7.1f} /{c['standard_db']:6.1f} " if "enhanced_db" in c else f"{'-':>16}"
        print(line)
    print("\nleast-squares |gain| of Dolby on oadec, SNR after it (dB)")
    for row in table:
        line = f"{row['code']:3}"
        for nm in shown:
            c = row["channels"][nm]
            line += f"  {nm} {c['gain']:6.3f} {c['gain_snr_db']:5.1f}" if "gain" in c else f"{'-':>18}"
        print(line)
    worst = max(worse, key=lambda w: w[3] - w[2]) if worse else None
    print(f"\npairs (code x coupled channel) with enhanced < standard: {len(worse)} of {CODES * len(coupled)}"
          f" ({len(worse_r)} where the rounded figures differ)")
    if worst:
        print(f"largest: {worst[3] - worst[2]:.2f} dB at code {worst[0]} {worst[1]}")
        for w in sorted(worse, key=lambda w: w[2] - w[3])[:6]:
            print(f"   code {w[0]:2} {w[1]:2}: {w[2]:6.2f} / {w[3]:6.2f}  ({w[3] - w[2]:.2f} dB)")
    if untouched:
        print(f"centre (untouched): lower in {len(centre)} of {CODES} buckets, by up to {centre_floor:.2f} dB")
    print(f"own band of the coupled channels: lower in {len(own_low)} of {CODES * len(coupled)}, by up to {own_floor:.2f} dB")
    if floor is not None:
        print(f"pairs beyond the centre's floor ({floor:.2f} dB): {len(beyond)}")
    else:
        print("no untouched channel: the floor is not measured")
    print(f"gain range over the coupled channels: {min(gains):.3f} to {max(gains):.3f}")

    if a.json:
        out = {
            "band_hz": list(band), "own_band_hz": list(own), "segments": int(n),
            "coupled": coupled, "untouched": untouched,
            "pairs": CODES * len(coupled), "worse": len(worse), "worse_rounded": len(worse_r),
            "largest_db": None if not worst else round(worst[3] - worst[2], 3),
            "largest_at": None if not worst else [worst[0], worst[1]],
            "centre_lower": len(centre), "centre_floor_db": round(centre_floor, 3),
            "own_lower": len(own_low), "own_floor_db": round(own_floor, 3),
            "beyond_floor": len(beyond) if floor is not None else None,
            "gain_range": [round(min(gains), 4), round(max(gains), 4)],
            "manifest": manifest(work), "table": table,
        }
        Path(a.json).write_text(json.dumps(out, indent=1) + "\n")
    return 0


def cmd_cells(a: argparse.Namespace) -> int:
    """The carrier file of `tools/ecpl_promotion.py` for one model decode."""
    import numpy as np

    work = Path(a.work)
    names, coupled, _, band, _ = geometry(a)
    name = a.name
    model = Path(a.model)
    stem = model.with_suffix("").name
    local = work / f"{stem}.f32"
    if model.resolve() != local.resolve():
        raise SystemExit(f"the model decode must lie in the work folder as {local.name}")
    sp = decodes(work, names, [f"{name}.oadec1", f"{name}.dolby1", stem], [band])
    n = min(v[0].shape[0] for v in sp.values())
    codes = np.arange(n) % CODES
    d = sp[f"{name}.dolby1"][0][:n]
    sig = (np.abs(d) ** 2).sum(axis=1)
    e_amp = (np.abs(sp[f"{name}.oadec1"][0][:n] - d) ** 2).sum(axis=1)
    e_mod = (np.abs(sp[stem][0][:n] - d) ** 2).sum(axis=1)
    cells = []
    for k in range(CODES):
        mask = codes == k
        mask[0] = False
        for nm in coupled:
            ch = names.index(nm)
            s = float(sig[mask, ch].sum())
            if s <= 0:
                continue
            cells.append({"code": k, "channel": nm,
                          "amp_only_db": round(snr_db(s, float(e_amp[mask, ch].sum())), 4),
                          "model_db": round(snr_db(s, float(e_mod[mask, ch].sum())), 4)})
    off = work / f"{name}.model-off.f32"
    off_identical = off.exists() and md5(off) == md5(work / f"{name}.oadec1.f32")
    out = {"carrier": a.carrier, "role": a.role, "layout": a.layout, "cpl_aht": bool(int(a.aht)),
           "source": a.source, "off_identical": off_identical, "model": a.model_name,
           "band_hz": list(band), "cells": cells}
    Path(a.out).write_text(json.dumps(out, indent=1) + "\n")
    print(f"{a.carrier}: {len(cells)} cells, off identical: {off_identical}")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)

    def geo(p):
        p.add_argument("--layout", choices=sorted(LAYOUTS), default="5.1")
        p.add_argument("--band", nargs=2, type=int, default=[109, 193], metavar=("LO", "HI"),
                       help="first and one-past-last coupled bin of the 256 a block has")
        p.add_argument("--coupled", help="the coupled channels, by name, comma separated")
        p.add_argument("--name", default="amp")

    p = sub.add_parser("make")
    p.add_argument("--oadec", required=True)
    p.add_argument("--clip", required=True)
    p.add_argument("--work", required=True)
    p.add_argument("--plan-file")
    p.add_argument("--name", default="amp")
    p = sub.add_parser("dolby")
    p.add_argument("--work", required=True)
    p.add_argument("--clip", required=True)
    p.add_argument("--names", default="amp")
    p.add_argument("--layout", choices=sorted(LAYOUTS), default="5.1")
    p = sub.add_parser("matrix")
    p.add_argument("--oadec", required=True)
    p.add_argument("--clip", required=True)
    p.add_argument("--work", required=True)
    p.add_argument("--plans", required=True)
    p.add_argument("--layout", choices=sorted(LAYOUTS), default="5.1")
    p.add_argument("--keep", action="store_true", help="keep the decodes")
    p = sub.add_parser("analyse")
    p.add_argument("--work", required=True)
    p.add_argument("--json")
    geo(p)
    p = sub.add_parser("cells")
    p.add_argument("--work", required=True)
    p.add_argument("--model", required=True, help="the model's decode, <work>/<stem>.f32")
    p.add_argument("--model-name", default="")
    p.add_argument("--carrier", required=True)
    p.add_argument("--role", choices=["fit", "holdout"], required=True)
    p.add_argument("--source", required=True)
    p.add_argument("--aht", choices=["0", "1"], required=True)
    p.add_argument("--out", required=True)
    geo(p)
    a = ap.parse_args()
    return {"make": cmd_make, "dolby": cmd_dolby, "matrix": cmd_matrix, "analyse": cmd_analyse,
            "cells": cmd_cells}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
