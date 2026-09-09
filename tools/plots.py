#!/usr/bin/env python3
"""Draw the measurements that are easier to see than to read.

Three figures, each answering one question the evidence reports answer in
numbers:

* what the transient pre-noise correction actually does to the samples;
* whether the enhanced coupling region lands where the standard coupling
  region did, and how far it is from the Dolby decode;
* whether the clip gain tracks the level of the coded downmix.

    python tools/plots.py --out E:/oadec-work/plots
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).parent))
from tpnp_window import read_wav  # noqa: E402

import matplotlib  # noqa: E402

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

RATE = 48000
INK = "#1b1b1b"
OURS_OFF = "#b0b0b0"
OURS_ON = "#0b6bcb"
DOLBY = "#d1495b"


def style(ax, title, xlabel, ylabel):
    ax.set_title(title, fontsize=11, color=INK, loc="left", pad=8)
    ax.set_xlabel(xlabel, fontsize=9, color=INK)
    ax.set_ylabel(ylabel, fontsize=9, color=INK)
    ax.tick_params(labelsize=8, colors=INK)
    for side in ("top", "right"):
        ax.spines[side].set_visible(False)
    for side in ("left", "bottom"):
        ax.spines[side].set_color("#c8c8c8")
    ax.grid(True, color="#ececec", linewidth=0.8)
    ax.set_axisbelow(True)


def transient(out: Path, on: Path, off: Path, dolby: Path, report: Path) -> None:
    """The samples the correction rewrites, next to the Dolby decode."""
    a = np.fromfile(on, dtype="<f4").reshape(-1, 6).astype(np.float64)
    b = np.fromfile(off, dtype="<f4").reshape(-1, 6).astype(np.float64)
    ref, _ = read_wav(str(dolby))
    tr = json.loads(report.read_text(encoding="utf-8"))["transients"]
    lag, frame = 256, 1536
    # the busiest correction on the left channel, coded channel 0 -> WAVE 0
    best = max((t for t in tr if t["channel"] == 0), key=lambda t: t["len"])
    loc = best["frame"] * frame + best["loc"]
    pn = loc - (loc // 256 - 1) * 256
    tot = pn + best["len"] + 256
    lo, hi = loc - tot - 400, loc + 400
    t = (np.arange(lo, hi) - loc) / RATE * 1000
    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(10, 6.4), sharex=True)
    ax1.plot(t, b[lo:hi, 0], color=OURS_OFF, lw=1.0, label="without the tool")
    ax1.plot(t, a[lo:hi, 0], color=OURS_ON, lw=1.0, label="with it")
    ax1.plot(t, ref[lo - lag:hi - lag, 0], color=DOLBY, lw=0.9, ls="--", label="Dolby")
    ax1.axvspan((lo + 400 - loc) / RATE * 1000, 0, color="#fff3cd", zorder=0)
    style(ax1, "Transient pre-noise: the corrected region, left channel",
          "", "sample value")
    ax1.legend(fontsize=8, frameon=False, ncol=3)
    ax2.plot(t, b[lo:hi, 0] - ref[lo - lag:hi - lag, 0], color=OURS_OFF, lw=1.0,
             label="without the tool")
    ax2.plot(t, a[lo:hi, 0] - ref[lo - lag:hi - lag, 0], color=OURS_ON, lw=1.0,
             label="with it")
    style(ax2, "Distance to the Dolby decode over the same samples",
          "milliseconds from the transient", "difference")
    ax2.legend(fontsize=8, frameon=False, ncol=2)
    fig.tight_layout()
    fig.savefig(out / "transient-pre-noise.png", dpi=140)
    plt.close(fig)


def coupling(out: Path, plain: Path, ecpl: Path, dolby: Path, seconds: float) -> None:
    """The spectrum of a coupled channel, before and after the rewrite."""
    n = 1 << 16
    at = int(seconds * RATE)
    a = np.fromfile(plain, dtype="<f4").reshape(-1, 6).astype(np.float64)[at:at + n, 5]
    b = np.fromfile(ecpl, dtype="<f4").reshape(-1, 6).astype(np.float64)[at:at + n, 5]
    ref, _ = read_wav(str(dolby))
    r = ref[at - 256:at - 256 + n, 5]
    w = np.hanning(n)
    f = np.fft.rfftfreq(n, 1 / RATE) / 1000
    def spec(x):
        return 20 * np.log10(np.maximum(np.abs(np.fft.rfft(x * w)) / n, 1e-12))
    fig, ax = plt.subplots(figsize=(10, 4.6))
    ax.plot(f, spec(a), color=OURS_OFF, lw=0.8, label="standard coupling, as the stream came")
    ax.plot(f, spec(b), color=OURS_ON, lw=0.8, label="rewritten as enhanced coupling")
    ax.plot(f, spec(r), color=DOLBY, lw=0.8, ls="--", label="Dolby, same rewritten stream")
    ax.axvspan(109 * RATE / 512 / 1000, 193 * RATE / 512 / 1000, color="#fff3cd", zorder=0)
    ax.text(0.5 * (109 + 193) * RATE / 512 / 1000, -22, "coupling region",
            ha="center", fontsize=8, color="#8a6d1f")
    ax.set_xlim(0, 24)
    ax.set_ylim(-140, -10)
    style(ax, "Enhanced coupling: the rewritten region, right surround",
          "kHz", "dB")
    ax.legend(fontsize=8, frameon=False)
    fig.tight_layout()
    fig.savefig(out / "enhanced-coupling.png", dpi=140)
    plt.close(fig)


def clipgain(out: Path, core: Path, report: Path) -> None:
    """The clip gain against the level of the downmix it was taken off."""
    d = json.loads(report.read_text(encoding="utf-8"))
    gains = {c["frame"]: c["gain"] for c in d["clipgains"]}
    pcm = np.fromfile(core, dtype="<f4").reshape(-1, 6).astype(np.float64)
    frame = 1536
    frames = len(pcm) // frame
    peak = np.abs(pcm[: frames * frame].reshape(frames, frame, 6)).max(axis=(1, 2))
    g = np.array([gains.get(i, 1.0) for i in range(frames)])
    lo = max(0, int(np.argmax(g)) - 300)
    hi = min(frames, lo + 900)
    t = np.arange(lo, hi) * frame / RATE
    fig, ax = plt.subplots(figsize=(10, 4.6))
    ax.plot(t, peak[lo:hi], color=OURS_OFF, lw=1.0, label="peak of the coded downmix")
    ax.axhline(1.0, color="#c8c8c8", lw=0.8, ls=":")
    ax2 = ax.twinx()
    ax2.plot(t, g[lo:hi], color=DOLBY, lw=1.4, label="clip gain")
    ax2.set_ylabel("clip gain", fontsize=9, color=DOLBY)
    ax2.tick_params(labelsize=8, colors=DOLBY)
    ax2.spines["top"].set_visible(False)
    style(ax, "The clip gain rises exactly where the downmix reaches full scale",
          "seconds", "peak sample")
    lines = ax.get_lines()[:1] + ax2.get_lines()
    ax.legend(lines, [l.get_label() for l in lines], fontsize=8, frameon=False)
    fig.tight_layout()
    fig.savefig(out / "clip-gain.png", dpi=140)
    plt.close(fig)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--work", type=Path, required=True, help="where the decodes are")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    w = args.work
    made = []
    try:
        transient(args.out, w / "tpnp-on.f32", w / "tpnp-off.f32",
                  Path("E:/oadec-work/dee/ddp-decode/lowrate-192/pi-head-192-dolby.wav"),
                  w / "spx192.json")
        made.append("transient-pre-noise.png")
    except (OSError, KeyError, ValueError) as e:
        print("transient figure skipped:", e)
    try:
        coupling(args.out, w / "orig384.f32", w / "pi-ang-dolbymode.f32",
                 Path("E:/oadec-work/dee/ddp-decode/ang/pi-ang-dolby.wav"), 40.0)
        made.append("enhanced-coupling.png")
    except (OSError, KeyError, ValueError) as e:
        print("coupling figure skipped:", e)
    try:
        clipgain(args.out, w / "core-hot.f32", w / "pihot.json")
        made.append("clip-gain.png")
    except (OSError, KeyError, ValueError) as e:
        print("clip gain figure skipped:", e)
    print("wrote:", ", ".join(made) if made else "nothing")
    return 0


if __name__ == "__main__":
    sys.exit(main())
