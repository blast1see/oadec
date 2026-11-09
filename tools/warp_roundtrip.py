#!/usr/bin/env python3
"""Real streams through the chain, with and without their warp mode.

E-AC-3 -> oadec -> DAMF or ADM -> Dolby's encoder -> oadec. For each title this asks
what the warp mode becomes without `--warp-mode`, whether `--warp-mode stream` keeps it
through both outputs, and whether the outputs without the flag are the bytes they were
(give `--old` a binary from before). `--render` plays the two encodes through the
Reference Player's own renderer and reports what the mode does to the sound.

    python tools/warp_roundtrip.py work/census/census.jsonl films/ more-films/ \
        --work work/rt --render

The census is what `tools/metadata_census.py` wrote; it says which tracks declare what.
Needs a Dolby Media Encoder installation and, for `--render`, the Reference Player.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from metadata_census import public_title  # noqa: E402
from oadec_bin import find as oadec_binary  # noqa: E402

DEE = Path(os.environ.get("DME_DEE", r"C:\Program Files\Dolby Media Encoder\resources\dee"))
LICENCE = Path(os.environ.get("APPDATA", "")) / "Dolby Media Encoder" / "license.lic"
DRP = Path(os.environ.get("DRP_DIR", r"C:\Program Files\Dolby\Dolby Reference Player"))
GST = dict(os.environ, GST_PLUGIN_PATH=str(DRP / "gst-plugins"), PATH=str(DRP) + os.pathsep + os.environ.get("PATH", ""))


def run(cmd: list[str], env: dict | None = None) -> tuple[int, str, str]:
    r = subprocess.run(cmd, capture_output=True, text=True, errors="replace", timeout=3600, env=env,
                       stdin=subprocess.DEVNULL)
    return r.returncode, r.stdout, r.stderr


def sha(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()[:16]


def encode(atmos: Path, out: Path) -> bool:
    return run([str(DEE / "dee_ddpjoc_encoder.exe"), "-l", str(LICENCE), "--input-format", "atmos_mezz",
                "-i", str(atmos), "-o", str(out), "--overwrite", "1", "--start", "0"])[0] == 0


def read_wave(path: Path):
    import numpy as np

    d = path.read_bytes()
    pos, channels, bits = 12, 0, 16
    while pos + 8 <= len(d):
        cid, n = d[pos:pos + 4], struct.unpack("<I", d[pos + 4:pos + 8])[0]
        if cid == b"fmt ":
            channels = struct.unpack("<H", d[pos + 10:pos + 12])[0]
            bits = struct.unpack("<H", d[pos + 22:pos + 24])[0]
        if cid == b"data":
            x = np.frombuffer(d[pos + 8:pos + 8 + n], dtype={32: "<f4", 16: "<i2"}[bits]).reshape(-1, channels).astype(float)
            return x if bits == 32 else x / 32768
        pos += 8 + n + (n & 1)
    raise ValueError(f"no data chunk in {path}")


def render(src: Path, layout: str, out: Path):
    run([str(DRP / "gst-launch-1.0.exe"), "filesrc", f"location={src.as_posix()}", "!", "dlbac3parse",
         "enable-metadata=true", "!", "dlbaudiodecbin", f"out-ch-config={layout}", "!", "audioconvert", "!",
         "wavenc", "!", "filesink", f"location={out.as_posix()}"], env=GST)
    return read_wave(out)


def compare_renders(kept: Path, plain: Path, work: Path) -> list[dict]:
    """Level as played, and what is left after matching each channel's level."""
    import numpy as np

    rows = []
    for layout in ("5.1", "7.1", "2.0"):
        a = render(kept, layout, work / f"kept{layout}.wav")
        b = render(plain, layout, work / f"plain{layout}.wav")
        n = min(len(a), len(b))
        level, residual = [], []
        for c in range(a.shape[1]):
            ra, rb = float(np.sqrt(np.mean(a[:n, c] ** 2))), float(np.sqrt(np.mean(b[:n, c] ** 2)))
            if ra < 1e-7 or rb < 1e-7:
                level.append(None)
                residual.append(None)
                continue
            level.append(round(20 * float(np.log10(ra / rb)), 2))
            left = a[:n, c] - b[:n, c] * (ra / rb)
            residual.append(round(20 * float(np.log10(max(float(np.sqrt(np.mean(left * left))), 1e-12) / ra)), 1))
        rows.append({"layout": layout, "level_db_kept_vs_plain": level, "residual_db": residual})
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("census", type=Path)
    ap.add_argument("roots", nargs="+", type=Path)
    ap.add_argument("--work", type=Path, required=True)
    ap.add_argument("--seconds", type=int, default=30)
    ap.add_argument("--per-mode", type=int, default=2, help="titles per declared warp mode")
    ap.add_argument("--old", help="a binary from before, to compare the output without the flag")
    ap.add_argument("--render", action="store_true")
    args = ap.parse_args()
    new = oadec_binary()

    def warp(path: Path):
        return json.loads(run([new, "oamd", "--json", str(path)])[1]).get("warp_modes")

    rows = [json.loads(ln) for ln in args.census.read_text(encoding="utf-8").splitlines() if ln.strip()]
    picked: list[tuple[dict, list[Path]]] = []
    seen: dict[str, int] = {}
    for r in rows:
        modes = (r.get("oamd") or {}).get("warp_modes")
        if r.get("codec") != "eac3" or not modes:
            continue
        mode = next(iter(modes))
        # one file name can be in two folders, with different tracks
        sources = [root / r["file"] for root in args.roots if (root / r["file"]).exists()]
        if sources and seen.get(mode, 0) < args.per_mode:
            seen[mode] = seen.get(mode, 0) + 1
            picked.append((r, sources))

    results = []
    for r, sources in picked:
        d = args.work / f"{len(results):03d}"
        d.mkdir(parents=True, exist_ok=True)
        head = d / "head.ec3"
        for source in sources:
            run(["ffmpeg", "-v", "fatal", "-y", "-i", str(source), "-map", f"0:{r['track']}", "-c", "copy",
                 "-t", str(args.seconds), "-f", "eac3", str(head)])
            if head.exists() and head.stat().st_size > 0:
                break
        else:
            continue
        row: dict = {"title": public_title(r["file"]), "source_warp": warp(head)}
        run([new, "decode", str(head), "--format", "damf", "--no-bed-conform", "-o", str(d / "plain" / "x")])
        if args.old:
            run([args.old, "decode", str(head), "--format", "damf", "--no-bed-conform", "-o", str(d / "old" / "x")])
            row["no_flag_identical"] = all(
                sha(d / "old" / f"x{e}") == sha(d / "plain" / f"x{e}")
                for e in (".atmos", ".atmos.metadata", ".atmos.audio"))
        row["warp_without_flag"] = warp(d / "plain.ec3") if encode(d / "plain" / "x.atmos", d / "plain.ec3") else None
        run([new, "decode", str(head), "--format", "damf", "--no-bed-conform", "-o", str(d / "kept" / "x"),
             "--warp-mode", "stream"])
        row["warp_with_flag_damf"] = warp(d / "kept.ec3") if encode(d / "kept" / "x.atmos", d / "kept.ec3") else None
        run([new, "decode", str(head), "--format", "adm", "--no-bed-conform", "-o", str(d / "adm" / "x"),
             "--warp-mode", "stream", "--dolby-origin-tag"])
        row["warp_with_flag_adm"] = warp(d / "adm.ec3") if encode(d / "adm" / "x.wav", d / "adm.ec3") else None
        if (d / "kept.ec3").exists() and (d / "adm.ec3").exists():
            row["damf_and_adm_encode_alike"] = sha(d / "kept.ec3") == sha(d / "adm.ec3")
        if args.render and (d / "kept.ec3").exists() and (d / "plain.ec3").exists():
            row["render"] = compare_renders(d / "kept.ec3", d / "plain.ec3", d)
        results.append(row)
        print(json.dumps(row, ensure_ascii=False), flush=True)
    (args.work / "roundtrip.json").write_text(json.dumps(results, indent=1, ensure_ascii=False), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
