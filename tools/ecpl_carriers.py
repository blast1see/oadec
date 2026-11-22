#!/usr/bin/env python3
"""The enhanced coupling carriers through the round-5 measurement, one step for all.

    python tools/ecpl_carriers.py make  --list CARRIERS.tsv --work DIR --oadec OADEC \\
        --model-decode ECPL_MODEL_DECODE --probe ECPL_CARRIER_PROBE
    python tools/ecpl_carriers.py dolby --list CARRIERS.tsv --work DIR
    python tools/ecpl_carriers.py cells --list CARRIERS.tsv --work DIR

`CARRIERS.tsv` has a line a carrier, tab separated, `#` a comment:

    <stream path>  <name>  <role: fit | holdout | library>  <source>  [<master>]

`master` names what the source was made from when two carriers share it (a
signal at two rates, a 5.1 master and its 2/0 fold), so that
`tools/ecpl_promotion.py` counts them as one holdout. A `library` carrier is
measured like a holdout and written as one, and its file is to be left out
of a promotion while its independence from the fitting carriers is not
known.

`make`, per carrier, in `<work>/<name>`: what `ecpl_carrier_probe` says of
the stream (layout, the coupling region its blocks use most, the coupled
channels, the share of coupled blocks whose coupling channel is coded with
the hybrid transform); the sweep (`amp sweep-frames / angle 0 / chaos 0 /
interp 0 / trans never / seed zero`) and its standard twin (`amp
sweep-frames / seed zero / target standard`) by `oadec eac3-ecpl-inject`;
oadec's default decode of the clip, the sweep (`tools/ecpl_amp_sweep.py
make`) and the twin; the sweep decoded with the model off
(`ecpl_model_decode`); and the candidate model of round 5, the existing
experimental rule over enhanced coupling, which is the twin decoded with
`--decorrelate` (its coefficients are the sweep's, bit for bit). A stream
without coupling is noted and left.

`dolby`: Dolby's decode of the clip, the sweep and the twin
(`ecpl_amp_sweep.dolby_decode`), and whether the sweep and the twin decode
to the same bytes.

`cells`: the carrier file of `tools/ecpl_promotion.py` (`cells.json`) and
the analysis table (`analyse.txt`, `analyse.json`) of every carrier.

Every step writes `<work>/<name>/meta.json`. The paths are arguments; the
tool keeps none of its own.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ecpl_amp_sweep as sw  # noqa: E402

SWEEP = "amp sweep-frames\nangle 0\nchaos 0\ninterp 0\ntrans never\nseed zero\n"
TWIN = "amp sweep-frames\nseed zero\ntarget standard\n"
CODED = {2: ["L", "R"], 7: ["L", "C", "R", "Ls", "Rs"]}
TOOL = Path(__file__).resolve().parent / "ecpl_amp_sweep.py"


def run(cmd) -> str:
    r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True)
    if r.returncode != 0:
        raise SystemExit(f"{Path(str(cmd[0])).name} failed: {r.stdout[-300:]} {r.stderr[-300:]}")
    return r.stdout


def md5(p) -> str:
    return hashlib.md5(Path(p).read_bytes()).hexdigest()


def carriers(path: Path):
    for no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        f = line.split("\t")
        if len(f) not in (4, 5) or f[2] not in ("fit", "holdout", "library"):
            raise SystemExit(f"{path}: line {no}: <stream> <name> <fit|holdout|library> <source> [<master>]")
        yield {"stream": Path(f[0]), "name": f[1], "role": f[2], "source": f[3],
               "master": f[4] if len(f) == 5 else f[3]}


def make(c, work: Path, a) -> dict:
    src = c["stream"]
    probe = json.loads(run([a.probe, src]).strip())
    meta = {"probe": probe, "role": c["role"], "source": c["source"], "master": c["master"],
            "sha256": hashlib.sha256(src.read_bytes()).hexdigest()}
    if not probe["regions"]:
        meta.update({"usable": False, "why": "no coupled block"})
        print(f"{c['name']}: no coupling, not a carrier")
        return meta
    region = max(probe["regions"], key=lambda r: r["blocks"])
    bits = max(probe["chincpl"], key=lambda x: x["blocks"])["bits"]
    coded = CODED[probe["acmod"]]
    meta.update({"usable": True, "layout": "2.0" if probe["acmod"] == 2 else "5.1", "band": region["bins"],
                 "coupled": [coded[i] for i in range(len(coded)) if bits & (1 << i)],
                 "aht_share": round(probe["cpl_aht"] / max(probe["coupled"], 1), 4)})
    (work / "sweep.plan").write_text(SWEEP)
    (work / "twin.plan").write_text(TWIN)
    run([sys.executable, "-I", TOOL, "make", "--oadec", a.oadec, "--clip", src, "--work", work,
         "--plan-file", work / "sweep.plan", "--name", "amp"])
    run([a.oadec, "eac3-ecpl-inject", src, "-o", work / "twin.ec3", "--plan", work / "twin.plan"])
    run([a.oadec, "decode", "--format", "pcm", "-o", work / "twin.oadec.f32", work / "twin.ec3"])
    run([a.model_decode, work / "amp.ec3", work / "amp.model-off.f32"])
    run([a.oadec, "decode", "--format", "pcm", "--decorrelate", "-o", work / "amp.model.f32", work / "twin.ec3"])
    meta["oadec"] = {k: md5(work / f) for k, f in (("amp", "amp.oadec1.f32"), ("twin", "twin.oadec.f32"),
                                                   ("model_off", "amp.model-off.f32"), ("model", "amp.model.f32"))}
    meta["twin_oadec_identical"] = meta["oadec"]["amp"] == meta["oadec"]["twin"]
    meta["model_changes_something"] = meta["oadec"]["model"] != meta["oadec"]["amp"]
    print(f"{c['name']}: {meta['layout']} band {meta['band']} coupled {meta['coupled']} aht {meta['aht_share']:.2f}; "
          f"twin identical in oadec: {meta['twin_oadec_identical']}")
    return meta


def dolby(c, work: Path, meta: dict) -> None:
    for name, s in (("src", c["stream"]), ("amp", work / "amp.ec3"), ("twin", work / "twin.ec3")):
        sw.dolby_decode(s, work / f"{name}.dolby1.f32", meta["layout"])
    meta["dolby"] = {k: md5(work / f"{k}.dolby1.f32") for k in ("src", "amp", "twin")}
    meta["twin_dolby_identical"] = meta["dolby"]["amp"] == meta["dolby"]["twin"]
    print(f"{c['name']}: Dolby sweep == Dolby twin: {meta['twin_dolby_identical']}")


def cells(c, work: Path, meta: dict) -> None:
    lo, hi = meta["band"]
    common = ["--work", work, "--layout", meta["layout"], "--band", lo, hi, "--coupled", ",".join(meta["coupled"])]
    out = run([sys.executable, "-I", TOOL, "cells", *common, "--model", work / "amp.model.f32",
               "--model-name", "standard-decorrelation", "--carrier", c["name"],
               "--role", "fit" if c["role"] == "fit" else "holdout", "--source", c["source"],
               "--master", c["master"], "--aht", "1" if meta["aht_share"] > 0 else "0",
               "--out", work / "cells.json"])
    print(out.strip())
    out = run([sys.executable, "-I", TOOL, "analyse", *common, "--json", work / "analyse.json"])
    (work / "analyse.txt").write_text(out)


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("step", choices=["make", "dolby", "cells"])
    ap.add_argument("--list", type=Path, required=True)
    ap.add_argument("--work", type=Path, required=True)
    ap.add_argument("--oadec")
    ap.add_argument("--model-decode")
    ap.add_argument("--probe")
    a = ap.parse_args()
    if a.step == "make" and not (a.oadec and a.model_decode and a.probe):
        ap.error("make needs --oadec, --model-decode and --probe")
    for c in carriers(a.list):
        work = a.work / c["name"]
        work.mkdir(parents=True, exist_ok=True)
        meta_p = work / "meta.json"
        meta = json.loads(meta_p.read_text()) if meta_p.exists() else {}
        if a.step == "make":
            meta.update(make(c, work, a))
        elif meta.get("usable"):
            meta.update({"role": c["role"], "source": c["source"], "master": c["master"]})
            (dolby if a.step == "dolby" else cells)(c, work, meta)
        meta_p.write_text(json.dumps(meta, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
