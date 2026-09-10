#!/usr/bin/env python3
"""Decode the head of every Dolby Digital Plus 7.1 track in a library and check
each channel against FFmpeg.

Certifying dependent-substream support on one title is not certifying it. This
takes a short head out of each track listed in a manifest, decodes it with both
decoders, and reports for every programme channel which FFmpeg channel it
matches and by how much it beats the runner-up -- so a channel that landed in
the wrong place shows up as a small margin rather than as a plausible number.

The manifest is one line per track, "<stream index> <bytes> <file name>", which
is what a library scan for eight-channel E-AC-3 produces.

    python tools/ddp71_sweep.py manifest.txt --roots E:/films D:/films \\
        --out sweep.json --seconds 10

Heads and float dumps are deleted as soon as they are measured: eight channels
of ten seconds is 15 MB per title in each decoder's output, and only the
numbers are worth keeping.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ec3_structure  # noqa: E402

FFMPEG = shutil.which("ffmpeg") or r"C:\ffmpeg\bin\ffmpeg.exe"
FFPROBE = shutil.which("ffprobe") or r"C:\ffmpeg\bin\ffprobe.exe"


def find(name: str, roots: list[Path]) -> Path | None:
    for root in roots:
        p = root / name
        if p.exists():
            return p
        hit = next(root.rglob(name), None) if root.exists() else None
        if hit:
            return hit
    return None


def run(cmd: list[str], timeout: int = 600) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)


def layout_of(path: Path) -> tuple[int, str]:
    """FFmpeg's channel count and layout name for a stream."""
    res = run([FFPROBE, "-v", "error", "-select_streams", "a:0", "-show_entries",
               "stream=channels,channel_layout", "-of", "csv=p=0", str(path)])
    line = (res.stdout or "").strip().splitlines()
    if not line:
        return 0, ""
    parts = line[0].split(",")
    try:
        return int(parts[0]), (parts[1] if len(parts) > 1 else "")
    except ValueError:
        return 0, ""


# FFmpeg's channel order for the layouts a Dolby Digital Plus programme reaches.
# A dependent substream can add a height pair as easily as a rear pair, so a
# track advertised as 7.1 in its file name may well decode to 5.1.2.
LAYOUTS = {
    "5.1": ["FL", "FR", "FC", "LFE", "BL", "BR"],
    "5.1(side)": ["FL", "FR", "FC", "LFE", "SL", "SR"],
    "7.1": ["FL", "FR", "FC", "LFE", "BL", "BR", "SL", "SR"],
    "7.1(wide)": ["FL", "FR", "FC", "LFE", "BL", "BR", "FLC", "FRC"],
    "5.1.2": ["FL", "FR", "FC", "LFE", "BL", "BR", "TFL", "TFR"],
    "5.1.4": ["FL", "FR", "FC", "LFE", "BL", "BR", "TFL", "TFR", "TBL", "TBR"],
    "7.1.2": ["FL", "FR", "FC", "LFE", "BL", "BR", "SL", "SR", "TFL", "TFR"],
}

# Which FFmpeg channel each channel location should land in. The surround pair
# is the one place it depends on the layout: FFmpeg calls it side left/right
# when a rear pair is also present and back left/right when it is not.
NAME_MAP = {
    "L": ["FL"], "R": ["FR"], "C": ["FC"], "LFE": ["LFE"],
    "Ls": ["SL", "BL"], "Rs": ["SR", "BR"],
    "Lrs": ["BL"], "Rrs": ["BR"],
    "Lc": ["FLC"], "Rc": ["FRC"], "Cs": ["BC"], "S": ["BC"],
    "Vhl": ["TFL"], "Vhr": ["TFR"], "Vhc": ["TFC"],
    "Lts": ["TBL"], "Rts": ["TBR"], "Ts": ["TC"],
}


def expected(name: str, names_b: list[str]) -> str | None:
    for candidate in NAME_MAP.get(name, []):
        if candidate in names_b:
            return candidate
    return None


def compare(a: Path, ach: int, b: Path, bch: int, names_a, names_b):
    """Every channel of `a` against every channel of `b`, and a verdict.

    The verdict is not "which reference channel does this look most like": with
    two identical reference channels, or with one at the dither floor, the
    argmax is arbitrary and would report a fault where there is none. It is
    "does the channel this one is supposed to be beat every channel it is not",
    with silent references excluded because comparing two decoders' dither
    says nothing at all.
    """
    import numpy as np

    def load(p, ch):
        x = np.fromfile(p, dtype="<f4")
        n = len(x) // ch
        return x[: n * ch].reshape(n, ch).astype(np.float64)

    A, B = load(a, ach), load(b, bch)
    n = min(len(A), len(B))
    A, B = A[:n], B[:n]
    levels = [None if (B[:, j] ** 2).mean() == 0
              else round(float(10 * np.log10((B[:, j] ** 2).mean())), 1)
              for j in range(bch)]

    def sdr(i, j):
        err = float(np.sqrt(((A[:, i] - B[:, j]) ** 2).mean()))
        sig = float(np.sqrt((B[:, j] ** 2).mean()))
        if sig == 0:
            return None
        return float("inf") if err == 0 else round(20 * np.log10(sig / err), 2)

    matrix = [[sdr(i, j) for j in range(bch)] for i in range(ach)]
    rows = []
    for i, name in enumerate(names_a):
        want = expected(name, names_b)
        row = {"a": name, "expected": want}
        if want is None:
            row["verdict"] = "no reference channel for this location"
            rows.append(row)
            continue
        j = names_b.index(want)
        row["sdr_db"] = matrix[i][j]
        row["reference_rms_dbfs"] = levels[j]
        if levels[j] is None or levels[j] <= -80.0:
            row["verdict"] = "silent"
            rows.append(row)
            continue
        # the best channel this is not supposed to be, ignoring silent ones and
        # any reference channel that is a copy of the intended one
        others = []
        for k in range(bch):
            if k == j or levels[k] is None or levels[k] <= -80.0:
                continue
            same = float(np.dot(B[:, k], B[:, j]) /
                         (np.linalg.norm(B[:, k]) * np.linalg.norm(B[:, j]) + 1e-30))
            if same > 0.999:
                continue
            others.append((matrix[i][k] if matrix[i][k] is not None else -999.0, names_b[k]))
        best_other = max(others, default=(-999.0, None))
        row["runner_up"] = best_other[1]
        row["runner_up_sdr_db"] = None if best_other[0] == -999.0 else best_other[0]
        margin = (row["sdr_db"] or -999.0) - best_other[0]
        row["margin_db"] = round(margin, 2)
        # Three outcomes, and only one of them is a fault. The channel is in
        # the wrong place if some other reference channel fits it better. It is
        # in the right place but not provably so if it wins by little, which is
        # what happens when the reference pair is nearly the same signal, or
        # when the channel sits close enough to the dither floor that most of
        # what is being compared is two decoders' noise.
        row["verdict"] = ("mismatched" if margin < 0
                          else "matched" if margin >= 10
                          else "narrow")
        rows.append(row)
    return matrix, rows, int(n), levels


# FFmpeg's channel order for the layouts a DD+ programme can reach.
FFMPEG_ORDER = {
    6: ["FL", "FR", "FC", "LFE", "BL", "BR"],
    8: ["FL", "FR", "FC", "LFE", "BL", "BR", "SL", "SR"],
}


def one(entry: tuple[int, str], roots, binary, work: Path, seconds: float, start: float) -> dict:
    index, name = entry
    row: dict = {"title": name, "stream_index": index}
    path = find(name, roots)
    if path is None:
        row["error"] = "not found"
        return row
    head = work / "head.ec3"
    head.unlink(missing_ok=True)
    cut = run([FFMPEG, "-v", "error", "-y", "-ss", str(start), "-i", str(path),
               "-t", str(seconds), "-map", f"0:{index}", "-c", "copy", "-f", "eac3", str(head)])
    if cut.returncode != 0 or not head.exists() or head.stat().st_size == 0:
        row["error"] = f"extract failed: {cut.stderr.strip()[:200]}"
        return row
    row["head_bytes"] = head.stat().st_size

    try:
        row["structure"] = {k: v for k, v in ec3_structure.describe(str(head), None).items()
                            if k in ("frames", "groups", "independent_frames", "dependent_frames",
                                     "substreams", "distinct_group_shapes", "first_group",
                                     "programme_channels", "group_bytes")}
    except (OSError, IndexError, KeyError, ValueError) as e:
        row["structure_error"] = str(e)

    v = run([binary, "verify", "--json", str(head)])
    try:
        report = json.loads(v.stdout)
    except json.JSONDecodeError:
        row["error"] = f"verify produced no JSON: {v.stderr.strip()[:200]}"
        return row
    row["verify_exit"] = v.returncode
    row["oadec"] = {
        "channels": report["channels"],
        "clean": report["clean"],
        "failures": {k: n for k, n in report["failures"].items() if n},
        "bit_rate": report["bit_rate"],
        "program_bit_rate": sum(p["bit_rate"] or 0 for p in report.get("program", [])),
        "dialnorm": report["dialnorm"],
        "samples": report["samples"],
        "program": report.get("program", []),
        "first_error": report.get("first_error"),
    }

    ours, ref = work / "ours.f32", work / "ref.f32"
    d = run([binary, "decode", "--order", "stream", "--format", "pcm", "-o", str(ours), str(head)])
    row["decode_exit"] = d.returncode
    f = run([FFMPEG, "-v", "error", "-y", "-drc_scale", "0", "-i", str(head),
             "-f", "f32le", "-acodec", "pcm_f32le", str(ref)])
    bch, layout = layout_of(head)
    row["ffmpeg_channels"] = bch
    row["ffmpeg_layout"] = layout
    if ours.exists() and ref.exists() and f.returncode == 0:
        ach = len(row["oadec"]["channels"])
        names_b = LAYOUTS.get(layout, [f"ch{i}" for i in range(bch)])
        if len(names_b) != bch:
            names_b = [f"ch{i}" for i in range(bch)]
        try:
            matrix, pairing, n, levels = compare(
                ours, ach, ref, bch, row["oadec"]["channels"], names_b)
            row["compared_samples"] = n
            row["reference_rms_dbfs"] = dict(zip(names_b, levels))
            row["pairing"] = pairing
            row["matrix_sdr_db"] = matrix
            verdicts = [p["verdict"] for p in pairing]
            row["measurable_channels"] = verdicts.count("matched") + verdicts.count("narrow")
            row["silent_channels"] = verdicts.count("silent")
            row["narrow_channels"] = verdicts.count("narrow")
            row["mismatched_channels"] = verdicts.count("mismatched")
            row["unambiguous"] = "mismatched" not in verdicts
            row["worst_sdr_db"] = min((p["sdr_db"] for p in pairing
                                       if p.get("verdict") in ("matched", "narrow")
                                       and p.get("sdr_db") is not None), default=None)
        except (ValueError, ZeroDivisionError) as e:
            row["compare_error"] = str(e)
    for p in (head, ours, ref):
        p.unlink(missing_ok=True)
    return row


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("manifest")
    ap.add_argument("--roots", nargs="+", required=True)
    ap.add_argument("--binary", default=r"target\release\oadec.exe")
    ap.add_argument("--out", required=True)
    ap.add_argument("--seconds", type=float, default=10.0)
    ap.add_argument("--start", type=float, default=600.0,
                    help="seconds into the title to cut from; the first minutes of a film are "
                         "often digital silence, and two decoders' dither is not a measurement")
    ap.add_argument("--limit", type=int, default=0)
    a = ap.parse_args()

    entries = []
    for line in open(a.manifest, encoding="utf-8"):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        idx, _size, name = line.split(" ", 2)
        entries.append((int(idx), name))
    if a.limit:
        entries = entries[: a.limit]

    roots = [Path(r) for r in a.roots]
    work = Path(tempfile.mkdtemp(prefix="ddp71-"))
    rows = []
    for i, e in enumerate(entries, 1):
        print(f"[{i}/{len(entries)}] {e[1][:70]}", flush=True)
        rows.append(one(e, roots, a.binary, work, a.seconds, a.start))
        json.dump({"titles": rows}, open(a.out, "w"), indent=1)
    shutil.rmtree(work, ignore_errors=True)

    done = [r for r in rows if "pairing" in r]
    eight = [r for r in done if len(r["oadec"]["channels"]) == 8]
    summary = {
        "titles": len(rows),
        "measured": len(done),
        "not_found": sum(1 for r in rows if r.get("error") == "not found"),
        "eight_channel_out": len(eight),
        "matching_ffmpeg_channel_count": sum(
            1 for r in done if len(r["oadec"]["channels"]) == r["ffmpeg_channels"]),
        "titles_with_every_channel_in_the_right_place": sum(1 for r in done if r.get("unambiguous")),
        "channels_matched": sum(r.get("measurable_channels", 0) - r.get("narrow_channels", 0) for r in done),
        "channels_narrow": sum(r.get("narrow_channels", 0) for r in done),
        "channels_silent": sum(r.get("silent_channels", 0) for r in done),
        "channels_mismatched": sum(r.get("mismatched_channels", 0) for r in done),
        "below_the_dither_floor": sum(1 for r in done if r.get("measurable_channels") == 0),
        "layouts": {},
        "worst_sdr_db": min((r["worst_sdr_db"] for r in done
                             if r.get("worst_sdr_db") is not None), default=None),
    }
    from collections import Counter
    summary["layouts"] = dict(Counter(r.get("ffmpeg_layout", "?") for r in done))
    summary["channel_maps"] = dict(Counter(
        hex(p["chanmap"]) if p.get("chanmap") is not None else "none"
        for r in done for p in r.get("structure", {}).get("first_group", [])[1:]))
    json.dump({"summary": summary, "titles": rows}, open(a.out, "w"), indent=1)
    print(json.dumps(summary, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
