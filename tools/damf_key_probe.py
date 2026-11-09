#!/usr/bin/env python3
"""Ask Dolby's encoders which keys of a DAMF set they read.

Knowing a key is not listening to it. For each variant this copies an authored set,
adds the key, runs `atmos_info` and the encoder, and reads what came out back with
oadec. Encoding the unchanged set twice is the control: the encoder is deterministic,
so any changed byte is the key's doing.

    oadec atmos-author crates/oadec-cli/tests/fixtures/authored-scene.json -o base/base
    python tools/damf_key_probe.py base variants.json out --encoder joc

A variant is {"name", "atmos": [lines for the header], "dbmd": "text of a sidecar",
"metadata": [[old, new]], "args": [encoder arguments], "start": "0" or null}.
`docs/audit/evidence/metadata-bridge-2026-11-09/02-damf-key-probe.json` holds the
variants and results behind `docs/dolby-tools.md`. Needs a Dolby Media Encoder
installation and its licence file; the licence is passed to the encoder and not read.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as oadec_binary  # noqa: E402

DEE = Path(os.environ.get("DME_DEE", r"C:\Program Files\Dolby Media Encoder\resources\dee"))
LICENCE = Path(os.environ.get("APPDATA", "")) / "Dolby Media Encoder" / "license.lic"


def run(cmd: list[str], timeout: int = 900) -> tuple[int, str]:
    r = subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=timeout)
    return r.returncode, r.stdout + r.stderr


def probe(base: Path, stem: str, out: Path, v: dict, encoder: str, oadec: str) -> dict:
    d = out / v["name"]
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    shutil.copy(base / f"{stem}.atmos.audio", d)
    text = (base / f"{stem}.atmos").read_text(encoding="utf-8")
    if v.get("atmos"):
        lines = "".join(f"    {x}\n" for x in v["atmos"])
        text = text.replace("    bedInstances:", lines + "    bedInstances:", 1)
    (d / f"{stem}.atmos").write_text(text, encoding="utf-8", newline="\n")
    meta = (base / f"{stem}.atmos.metadata").read_text(encoding="utf-8")
    for old, new in v.get("metadata", []):
        meta = meta.replace(old, new)
    (d / f"{stem}.atmos.metadata").write_text(meta, encoding="utf-8", newline="\n")
    if "dbmd" in v:
        (d / f"{stem}.atmos.dbmd").write_text(v["dbmd"], encoding="utf-8", newline="\n")

    info_rc, info = run([str(DEE / "atmos_info.exe"), "-i", str(d / f"{stem}.atmos")])
    y = d / ("y.ec3" if encoder == "joc" else "y.mlp")
    if encoder == "joc":
        cmd = [str(DEE / "dee_ddpjoc_encoder.exe"), "-l", str(LICENCE), "--input-format", "atmos_mezz",
               "-i", str(d / f"{stem}.atmos"), "-o", str(y)]
    else:
        cmd = [str(DEE / "dee_dthd_encoder.exe"), "-l", str(LICENCE), "--presentation", "atmos",
               "--input", str(d / f"{stem}.atmos"), "-o", str(y)]
    if v.get("start", "0") is not None:
        cmd += ["--start", v.get("start", "0")]
    cmd += v.get("args", [])
    rc, log = run(cmd)
    row: dict = {
        "name": v["name"],
        "atmos_info_exit": info_rc,
        "render_warp_mode": next((ln.split("Render warp mode: ")[-1].strip() for ln in info.splitlines()
                                  if "Render warp mode" in ln), None),
        "dbmd_warnings": [ln.split("WARNING: ")[-1].strip() for ln in info.splitlines() if "WARNING: name:" in ln],
        "encoder_exit": rc,
    }
    if not y.exists():
        row["error"] = next((ln.strip() for ln in reversed(log.splitlines()) if "ERROR" in ln), "")[:160]
        return row
    row["sha256"] = hashlib.sha256(y.read_bytes()).hexdigest()[:16]
    oamd = json.loads(run([oadec, "oamd", "--json", str(y)])[1].split("\n{", 1)[-1] if False else subprocess.run(
        [oadec, "oamd", "--json", str(y)], capture_output=True, text=True, errors="replace").stdout)
    row["warp_modes"] = oamd.get("warp_modes")
    row["trim_configs"] = oamd.get("trim_configs")
    info_json = json.loads(subprocess.run([oadec, "info", "--json", str(y)], capture_output=True, text=True,
                                          errors="replace").stdout)
    row["dialnorm"] = info_json.get("dialnorm")
    meaning = info_json.get("channel_meaning")
    if meaning:
        row["presentation_dialogue_norms"] = {k: meaning.get(k) for k in (
            "twoch_dialogue_norm", "sixch_dialogue_norm", "eightch_dialogue_norm")}
    return row


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("base", type=Path, help="directory holding the authored DAMF set")
    ap.add_argument("variants", type=Path)
    ap.add_argument("out", type=Path)
    ap.add_argument("--encoder", choices=("joc", "thd"), default="joc")
    args = ap.parse_args()
    stem = next(args.base.glob("*.atmos")).stem
    variants = json.loads(args.variants.read_text(encoding="utf-8"))
    oadec = oadec_binary()
    rows = []
    for v in variants:
        row = probe(args.base, stem, args.out, v, args.encoder, oadec)
        rows.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
    (args.out / "probe.json").write_text(json.dumps(rows, indent=1, ensure_ascii=False), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
