#!/usr/bin/env python3
"""The enhanced coupling amplitude sweep against Dolby's decoder, written down.

Round 4 measured the default (amplitude-only) reading of enhanced coupling
against the Dolby Reference Player's `dlbac3dec` with a script that never
entered the repository; this is that measurement, step by step, so it can be
run again on any carrier with standard coupling.

    python tools/ecpl_amp_sweep.py make    --oadec OADEC --clip SRC.ec3 --work DIR
    python tools/ecpl_amp_sweep.py dolby   --work DIR
    python tools/ecpl_amp_sweep.py analyse --work DIR [--json OUT.json]

`make` rewrites the clip's standard coupling as enhanced coupling with
`oadec eac3-ecpl-inject --plan` (plan `amp sweep-frames / angle 0 / chaos 0 /
interp 0 / trans never`: frame f carries amplitude code f % 32 in every band
of every coupled channel) and decodes the clip and the rewritten stream with
`oadec decode --format pcm`, each twice. `dolby` decodes the same two streams
with `dlbac3dec out-ch-config=7` (DRC off, `drop-delay=true`), each twice;
the player is found under `OADEC_DOLBY_PLAYER`, else under
`%ProgramFiles%/Dolby/Dolby Reference Player`. `analyse` needs numpy and
nothing else, and reads only the four decodes.

What `analyse` reports, over the coupling region of the clip (10218,75 to
18093,75 Hz, bins 109 to 193 of a 512-point transform at 48 kHz):

- per amplitude code (the frame's number modulo 32) and channel, the SNR of
  oadec's decode of the rewritten stream against Dolby's decode of it,
  beside the same SNR for the clip itself (standard coupling) over the same
  frames;
- the pairs of 32 codes and the coupled channels where enhanced coupling
  comes out further from Dolby than standard coupling, how far, and how many
  of them are beyond what the centre channel, which the rewrite does not
  touch, moves between the two decodes (the floor of the measurement);
- per code and channel, the least-squares gain of Dolby's decode on oadec's
  (a complex projection of the short-time spectra over the band) and the SNR
  left after that gain.

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
CH = 6
NAMES = ["L", "R", "C", "LFE", "Ls", "Rs"]
COUPLED = ["L", "R", "Ls", "Rs"]
UNTOUCHED = "C"
CODES = 32


def band_of(begin_bin: int, end_bin: int) -> tuple[float, float]:
    return (begin_bin * SR / 512, end_bin * SR / 512)


BAND = band_of(109, 193)


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


# ---------------------------------------------------------------- make


def cmd_make(a: argparse.Namespace) -> int:
    work = Path(a.work)
    work.mkdir(parents=True, exist_ok=True)
    clip = Path(a.clip)
    m = manifest(work)
    m["clip"] = {"name": clip.name, "bytes": clip.stat().st_size, "sha256": sha256(clip)}
    m["oadec"] = {"version": run([a.oadec, "--version"]).strip(), "sha256": sha256(Path(a.oadec))}
    plan = work / "amp.plan"
    plan.write_text(PLAN_AMP)
    amp = work / "amp.ec3"
    run([a.oadec, "eac3-ecpl-inject", str(clip), "-o", str(amp), "--plan", str(plan)])
    m["amp"] = {"plan": PLAN_AMP, "md5": md5(amp), "sha256": sha256(amp)}
    streams = {"src": clip, "amp": amp}
    for name, s in streams.items():
        sums = []
        for k in (1, 2):
            out = work / f"{name}.oadec{k}.f32"
            run([a.oadec, "decode", "--format", "pcm", "-o", str(out), str(s)])
            sums.append(md5(out))
        m.setdefault("decodes", {})[f"{name}.oadec"] = {"md5": sums[0], "repeat_same": sums[0] == sums[1]}
        print(f"{name}: oadec {sums[0]} (repeat {'same' if sums[0] == sums[1] else 'DIFFERENT'})")
    print(f"amp.ec3 md5 {m['amp']['md5']}")
    save_manifest(work, m)
    return 0


# ---------------------------------------------------------------- dolby


def player() -> Path:
    env = os.environ.get("OADEC_DOLBY_PLAYER")
    if env:
        return Path(env)
    return Path(os.environ.get("ProgramFiles", "")) / "Dolby" / "Dolby Reference Player"


def dolby_decode(src: Path, out: Path) -> None:
    p = player()
    gst = p / "gst-launch-1.0.exe"
    if not gst.exists():
        raise SystemExit(f"no gst-launch-1.0.exe under {p}; set OADEC_DOLBY_PLAYER")
    env = dict(os.environ)
    env["GST_PLUGIN_PATH"] = str(p / "gst-plugins")
    env["PATH"] = str(p) + os.pathsep + env.get("PATH", "")
    fwd = lambda x: str(x).replace("\\", "/")  # noqa: E731
    run([str(gst), "-q", "filesrc", f"location={fwd(src)}", "!", "dlbac3parse", "!", "dlbac3dec",
         "out-ch-config=7", "drc-suppress=true", "drc-mode=custom-0", "drc-cut=0", "drc-boost=0",
         "drop-delay=true", "!", "audio/x-raw,format=F32LE", "!", "filesink", f"location={fwd(out)}"], env)


def cmd_dolby(a: argparse.Namespace) -> int:
    work = Path(a.work)
    m = manifest(work)
    srcs = {"amp": work / "amp.ec3"}
    if a.clip:
        srcs["src"] = Path(a.clip)
    else:
        raise SystemExit("--clip is needed: the standard coupling stream is the clip itself")
    for name, s in srcs.items():
        sums = []
        for k in (1, 2):
            out = work / f"{name}.dolby{k}.f32"
            dolby_decode(s, out)
            sums.append(md5(out))
        m.setdefault("decodes", {})[f"{name}.dolby"] = {"md5": sums[0], "repeat_same": sums[0] == sums[1]}
        print(f"{name}: dolby {sums[0]} (repeat {'same' if sums[0] == sums[1] else 'DIFFERENT'})")
    m["dolby"] = {"element": "dlbac3dec out-ch-config=7 drc-suppress=true drc-mode=custom-0 drc-cut=0 drc-boost=0 drop-delay=true"}
    save_manifest(work, m)
    return 0


# ---------------------------------------------------------------- analyse


def cmd_analyse(a: argparse.Namespace) -> int:
    import numpy as np

    work = Path(a.work)

    def load(name: str):
        return np.fromfile(work / f"{name}.f32", dtype="<f4").reshape(-1, CH).astype(np.float64)

    def spectra(x):
        n = x.shape[0] // FRAME
        seg = x[: n * FRAME].reshape(n, FRAME, CH) * np.hanning(FRAME)[None, :, None]
        s = np.fft.rfft(seg, axis=1)
        f = np.fft.rfftfreq(FRAME, 1 / SR)
        return s[:, (f >= BAND[0]) & (f < BAND[1]), :]

    def aligned(stream: str):
        o, d = load(f"{stream}.oadec1"), load(f"{stream}.dolby1")
        n = min(o.shape[0] - LATE, d.shape[0])
        return spectra(o[LATE:LATE + n]), spectra(d[:n])

    so_e, sd_e = aligned("amp")
    so_s, sd_s = aligned("src")
    n = min(so_e.shape[0], so_s.shape[0])
    so_e, sd_e, so_s, sd_s = so_e[:n], sd_e[:n], so_s[:n], sd_s[:n]
    codes = np.arange(n) % CODES

    def energy(s):
        return (np.abs(s) ** 2).sum(axis=1)  # [segment, channel]

    sig_e, err_e = energy(sd_e), energy(so_e - sd_e)
    sig_s, err_s = energy(sd_s), energy(so_s - sd_s)

    table = []
    for k in range(CODES):
        mask = codes == k
        mask[0] = False
        row = {"code": k, "channels": {}}
        for ch, name in enumerate(NAMES):
            if name == "LFE":
                continue
            se, ee = sig_e[mask, ch].sum(), err_e[mask, ch].sum()
            ss, es = sig_s[mask, ch].sum(), err_s[mask, ch].sum()
            cell: dict = {}
            if ss > 0 and se > 1e-12 * ss:
                cell["enhanced_db"] = float(10 * np.log10(se / ee)) if ee > 0 else float("inf")
                cell["standard_db"] = float(10 * np.log10(ss / es)) if es > 0 else float("inf")
            a_, b_ = so_e[mask, :, ch].ravel(), sd_e[mask, :, ch].ravel()
            ea = float(np.vdot(a_, a_).real)
            if ea > 0:
                g = np.vdot(a_, b_) / ea
                r = b_ - g * a_
                cell["gain"] = float(abs(g))
                cell["gain_snr_db"] = float(10 * np.log10(np.vdot(b_, b_).real / max(np.vdot(r, r).real, 1e-300)))
            row["channels"][name] = cell
        table.append(row)

    def diffs(names):
        out = []
        for row in table:
            for nm in names:
                c = row["channels"][nm]
                if "enhanced_db" in c and c["enhanced_db"] < c["standard_db"]:
                    out.append((row["code"], nm, c["enhanced_db"], c["standard_db"]))
        return out

    worse = diffs(COUPLED)
    centre = diffs([UNTOUCHED])
    floor = max((s - e for _, _, e, s in centre), default=0.0)
    gains = [row["channels"][nm]["gain"] for row in table for nm in COUPLED if "gain" in row["channels"][nm]]

    # rounded to one decimal before comparing, as the tables are read
    worse_r = [w for w in worse if round(w[2], 1) < round(w[3], 1)]
    beyond = [w for w in worse if w[3] - w[2] > round(floor, 1)]

    print("SNR in the coupling band (dB), oadec default against dlbac3dec,")
    print("enhanced (amp code k) / standard coupling (same frames)")
    print("code " + "".join(f"{c:>16}" for c in NAMES if c != "LFE"))
    for row in table:
        line = f"{row['code']:4} "
        for nm in NAMES:
            if nm == "LFE":
                continue
            c = row["channels"][nm]
            line += f"{c['enhanced_db']:7.1f} /{c['standard_db']:6.1f} " if "enhanced_db" in c else f"{'-':>16}"
        print(line)
    print("\nleast-squares |gain| of Dolby on oadec, SNR after it (dB)")
    for row in table:
        line = f"{row['code']:3}"
        for nm in ("L", "R", "C", "Ls", "Rs"):
            c = row["channels"][nm]
            line += f"  {nm} {c['gain']:6.3f} {c['gain_snr_db']:5.1f}" if "gain" in c else f"{'-':>18}"
        print(line)
    worst = max(worse, key=lambda w: w[3] - w[2]) if worse else None
    print(f"\npairs (code x coupled channel) with enhanced < standard: {len(worse)} of {CODES * len(COUPLED)}"
          f" ({len(worse_r)} where the rounded figures differ)")
    if worst:
        print(f"largest: {worst[3] - worst[2]:.2f} dB at code {worst[0]} {worst[1]}")
        for w in sorted(worse, key=lambda w: w[2] - w[3])[:6]:
            print(f"   code {w[0]:2} {w[1]:2}: {w[2]:6.2f} / {w[3]:6.2f}  ({w[3] - w[2]:.2f} dB)")
    print(f"centre (untouched): lower in {len(centre)} of {CODES} buckets, by up to {floor:.2f} dB")
    print(f"pairs beyond the centre's floor: {len(beyond)}")
    print(f"gain range over the coupled channels: {min(gains):.3f} to {max(gains):.3f}")

    if a.json:
        out = {
            "band_hz": list(BAND),
            "segments": int(n),
            "pairs": CODES * len(COUPLED),
            "worse": len(worse),
            "worse_rounded": len(worse_r),
            "largest_db": None if not worst else round(worst[3] - worst[2], 3),
            "largest_at": None if not worst else [worst[0], worst[1]],
            "centre_lower": len(centre),
            "centre_floor_db": round(floor, 3),
            "beyond_floor": len(beyond),
            "gain_range": [round(min(gains), 4), round(max(gains), 4)],
            "manifest": manifest(work),
            "table": table,
        }
        Path(a.json).write_text(json.dumps(out, indent=1) + "\n")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("make")
    p.add_argument("--oadec", required=True)
    p.add_argument("--clip", required=True)
    p.add_argument("--work", required=True)
    p = sub.add_parser("dolby")
    p.add_argument("--work", required=True)
    p.add_argument("--clip", required=True)
    p = sub.add_parser("analyse")
    p.add_argument("--work", required=True)
    p.add_argument("--json")
    a = ap.parse_args()
    return {"make": cmd_make, "dolby": cmd_dolby, "analyse": cmd_analyse}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
