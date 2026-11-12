#!/usr/bin/env python3
"""Which decoders give back the samples a written stream was made from?

`crates/oadec-synth` writes TrueHD and MLP streams from recipes, and knows the
samples of each before any decoder has read it. That makes a stream a test of
a decoder only if the writer is right, and the writer shares its reading of
the format with the decoder of this repository: the same documents, the same
author. So a recipe is taken as a reference here, and nowhere else, and only
on the word of decoders that share nothing with either:

* FFmpeg (`-f truehd` or `-f mlp`, to 24-bit PCM),
* `truehdd` (`decode --strict --presentation <index> --format pcm`),
* Dolby's own decoder, the `dlbtruehddec` element that ships with the Dolby
  Reference Player, on the stream wrapped in MP4 (it takes no raw stream).
  Only where the player is installed (`$DOLBY_REFERENCE_PLAYER` says where,
  if it is not where the system keeps its programs), and only for the TrueHD
  syntax.

Each is run as a program on a file and judged by the bytes it writes. A recipe
is confirmed when at least two of them give the samples of the recipe, every
one of them. What each said goes into the record, the ones that differed
included: a stream on which the decoders disagree is a finding, not a failure
of the tool.

A stream of several substreams has a presentation for each: two channels, six
and eight. Each is asked of every decoder and has a line of its own in the
record, and the recipe is confirmed when all of them are. The samples of a
presentation are compared in the order of the stream, which is front pair,
centre, LFE, side pair, back pair: FFmpeg and Dolby's decoder write eight
channels with the back pair in front of the side pair, and theirs are put
back first.

The crate writes AC-3 streams too, about delta bit allocation. Those are not
lossless, so no decoder gives a writer's samples bit for bit: each is judged
by the signal-to-noise ratio of what it writes against the samples the writer
says the stream comes to, channel by channel, and gives them when every
channel is over 80 dB. That is a question of reading and not of arithmetic,
and the two are far apart. A decoder that took one mantissa at another width
than the writer laid it out at has lost the rest of the block: it is under
20 dB, and at about 0 dB when it happens in every frame. A decoder that read
every mantissa is as near as its arithmetic: FFmpeg's is at 138 to 139 dB,
and Dolby's between 103 and 138 dB, because it holds the levels of the five
symmetric quantisers (thirds, fifths, sevenths, elevenths, fifteenths) to
fifteen fractional bits: against the writer's samples with the levels
rounded so, it is at 138 dB on every recipe. That costs it nothing where a
mantissa has bits enough for a plain quantiser, and by the arithmetic leaves
it at 96 dB on a stream that had no others. Two decoders from outside read
AC-3 here, FFmpeg's (`-drc_scale 0`) and Dolby's (the
`dlbac3dec` element, which delays its output by 32 samples), and an AC-3
recipe is confirmed when both give its samples. They are recorded in
`crates/oadec-synth/specs/ac3.tsv`, the probes with them.

    cargo build --release -p oadec-synth -p oadec-cli
    python tools/synth_accept.py --out oadec-work/synth
    python tools/synth_accept.py --out oadec-work/synth --record

`--record` rewrites `crates/oadec-synth/specs/digests.tsv`, which the tests of
that crate hold the writer to. It refuses when a recipe is not confirmed.

The crate also writes probes: streams that step past a bound of the format or
into a field whose meaning is not public. They go in front of the same
decoders, and what each did is recorded in `specs/probes.tsv` beside the
recipes. A probe needs nobody's confirmation: it is a question.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import subprocess
import sys
from array import array
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402

DESCRIPTION = "Which decoders give back the samples a written stream was made from?"
ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""
# where the Dolby Reference Player is: $DOLBY_REFERENCE_PLAYER, or the place the
# system keeps its programs
PLAYER = Path(os.environ.get("DOLBY_REFERENCE_PLAYER")
              or Path(os.environ.get("ProgramFiles", "")) / "Dolby" / "Dolby Reference Player")
GST = PLAYER / f"gst-launch-1.0{EXE}"
DIGESTS = ROOT / "crates" / "oadec-synth" / "specs" / "digests.tsv"
AC3 = ROOT / "crates" / "oadec-synth" / "specs" / "ac3.tsv"
AC3_HEADER = """\
# What the decoders made of the AC-3 streams of `oadec-synth`.
#
# Written by `tools/synth_accept.py --record` and by nothing else. A line is a
# stream: its name, `reference` or `probe`, the SHA-256 of the stream, the
# decoders that gave the samples the writer says it comes to (every channel
# over 80 dB of signal to noise against them), the ones that gave others,
# and the ones that would not read it (`-` for none). The decoders are
# FFmpeg's and Dolby's. A reference is here because both gave its samples; a
# probe is a question, and its lists say what each decoder does and not what
# is right.
"""
# What a decoder has to reach on every channel to have given the samples of
# an AC-3 stream, in dB of signal to noise: under what the coarsest
# arithmetic of the two leaves (about 96 dB, see above) and far over what a
# misread block does (under 20 dB). And the delays at which a decoder's
# output is looked for: none, and the 32 samples Dolby's adds.
AC3_AGREES_DB = 80.0
AC3_DELAYS = (0, 32)
AC3_OUTSIDE = ("ffmpeg", "dolby")
PROBES = ROOT / "crates" / "oadec-synth" / "specs" / "probes.tsv"
PROBES_HEADER = """\
# What the decoders did with the probes of `oadec-synth`.
#
# Written by `tools/synth_accept.py --record` and by nothing else. A line is a
# probe: its name, the SHA-256 of the stream, the SHA-256 of the samples the
# writer made it from as 24-bit little-endian PCM, the decoders that gave
# those samples, the ones that gave others, and the ones that would not read
# the stream (`-` for none). A probe is past a bound of the format or in a
# field whose meaning is not public: the lists say what each decoder does
# there and not what is right.
#
# A probe of several substreams has a line for each presentation, named as
# the recipes' are: `<name>.p<index>` is the presentation of the substream at
# that index, under the widest.
"""
INDEPENDENT = ("ffmpeg", "truehdd", "dolby")
# Where FFmpeg and Dolby's decoder put the channels of the stream's order when
# there are eight: back pair before side pair, as a WAVE file has them.
INTERCHANGE = {8: (0, 1, 2, 3, 6, 7, 4, 5)}
# What FFmpeg is asked for when the presentation is not the widest of the
# stream, by its channels; and what Dolby's decoder is to write, in the
# numbers of its element.
FFMPEG_LAYOUT = {2: "stereo", 6: "5.1(side)"}
DOLBY_CONFIG = {2: 0, 6: 7, 8: 11}
HEADER = """\
# What `oadec-synth digests` has to print, and which decoders said so.
#
# Written by `tools/synth_accept.py --record` and by nothing else. A line is a
# recipe: its name, the SHA-256 of the stream, the SHA-256 of its samples as
# 24-bit little-endian PCM, the decoders that gave those samples back, the
# ones that gave others, and the ones that would not read the stream (`-` for
# none). A recipe is here because two decoders that share nothing with this
# repository agree on it.
#
# A stream of several substreams has a line for each presentation: the name
# alone is the widest, the samples the stream was written from, and
# `<name>.p<index>` the presentation of the substream at that index under it,
# whose samples are what the writer worked out its matrices come to. Every
# line of a recipe has its two decoders.
"""


def run(cmd, env=None, timeout=600):
    r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, errors="replace",
                       env=env, timeout=timeout)
    return r.returncode, (r.stdout + r.stderr).strip()


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def first_difference(got: bytes, want: bytes, channels: int):
    """The first sample at which two 24-bit streams part, and how many do."""
    frame = 3 * channels
    differing = sum(1 for i in range(0, min(len(got), len(want)), 3) if got[i:i + 3] != want[i:i + 3])
    first = next((i // frame for i in range(0, min(len(got), len(want)), 3)
                  if got[i:i + 3] != want[i:i + 3]), None)
    return first, differing


def judged(got: bytes | None, want: bytes, channels: int, said: str, code: int) -> dict:
    """What a decoder made of a stream, against the samples of the recipe."""
    said = said[-600:]
    if got is None:
        return {"verdict": "refused", "exit": code, "said": said}
    if got == want:
        out = {"verdict": "target", "exit": code}
        if said:
            out["said"] = said
        return out
    first, differing = first_difference(got, want, channels)
    return {
        "verdict": "differs",
        "exit": code,
        "samples_written": len(got) // (3 * channels),
        "samples_expected": len(want) // (3 * channels),
        "first_differing_sample": first,
        "differing_samples": differing,
        "said": said,
    }


def stream_order(pcm: bytes | None, channels: int) -> bytes | None:
    """24-bit samples a decoder wrote in the interchange order, in the stream's."""
    order = INTERCHANGE.get(channels)
    if pcm is None or order is None:
        return pcm
    frame = 3 * channels
    out = bytearray(len(pcm))
    for start in range(0, len(pcm) - frame + 1, frame):
        for ch, source in enumerate(order):
            out[start + 3 * ch:start + 3 * ch + 3] = pcm[start + 3 * source:start + 3 * source + 3]
    return bytes(out)


def by_ffmpeg(stream: Path, r: dict, want: bytes, work: Path) -> dict:
    channels = r["channels"]
    out = work / (r["name"] + ".ffmpeg.pcm")
    out.unlink(missing_ok=True)
    fmt = "mlp" if stream.suffix == ".mlp" else "truehd"
    # the widest presentation is what it decodes when asked for nothing
    ask = [] if r["widest"] else ["-downmix", FFMPEG_LAYOUT[channels]]
    code, said = run(["ffmpeg", "-v", "warning", "-nostdin", *ask, "-f", fmt, "-i", stream,
                      "-f", "s24le", "-c:a", "pcm_s24le", "-y", out])
    got = out.read_bytes() if code == 0 and out.is_file() else None
    return judged(stream_order(got, channels), want, channels, said, code)


def by_truehdd(stream: Path, r: dict, want: bytes, work: Path) -> dict:
    base = work / (r["name"] + ".truehdd")
    out = Path(str(base) + ".pcm")
    out.unlink(missing_ok=True)
    code, said = run(["truehdd", "--loglevel", "warn", "decode", "--strict", "--presentation",
                      r["presentation"], "--format", "pcm", "--output-path", base, stream])
    got = out.read_bytes() if code == 0 and out.is_file() else None
    return judged(got, want, r["channels"], said, code)


def by_dolby(stream: Path, r: dict, want: bytes, work: Path) -> dict:
    if stream.suffix != ".thd":
        return {"verdict": "not asked", "said": "the element is given the TrueHD syntax only"}
    channels = r["channels"]
    mp4 = work / (stream.stem + ".mp4")
    out = work / (r["name"] + ".dolby.s32")
    out.unlink(missing_ok=True)
    code, said = run(["ffmpeg", "-v", "error", "-nostdin", "-f", "truehd", "-i", stream, "-c:a", "copy",
                      "-strict", "-2", "-f", "mp4", "-y", mp4])
    if code != 0:
        return {"verdict": "refused", "exit": code, "said": "wrapping in MP4: " + said[-400:]}
    env = dict(os.environ)
    env["GST_PLUGIN_PATH"] = str(PLAYER / "gst-plugins")
    env["PATH"] = str(PLAYER) + os.pathsep + env.get("PATH", "")
    # forward slashes: the property parser of gst-launch takes a backslash for an escape
    code, said = run([GST, "-q", "filesrc", f"location={mp4.as_posix()}", "!", "qtdemux", "!",
                      "capssetter", "caps=audio/x-true-hd", "replace=true", "join=false", "!",
                      "dlbtruehdparse", "!", "dlbtruehddec", f"presentation={channels}",
                      f"out-ch-config={DOLBY_CONFIG[channels]}", "!",
                      "audio/x-raw,format=S32LE", "!", "filesink", f"location={out.as_posix()}"], env=env)
    if code != 0 or not out.is_file():
        return {"verdict": "refused", "exit": code, "said": said[-600:]}
    words = out.read_bytes()
    # 24 bits at the top of each 32-bit word; anything under them is not the stream's
    if any(words[i] for i in range(0, len(words), 4)):
        return {"verdict": "differs", "exit": code, "said": "bits under the 24 the stream codes"}
    got = b"".join(words[i + 1:i + 4] for i in range(0, len(words), 4))
    return judged(stream_order(got, channels), want, channels, said, code)


def by_oadec(oadec: str, stream: Path, r: dict, want: bytes, work: Path) -> dict:
    out = work / (r["name"] + ".oadec.pcm")
    out.unlink(missing_ok=True)
    code, said = run([oadec, "decode", "-p", r["presentation"], "--order", "stream", "--format", "pcm",
                      "-o", out, stream])
    got = out.read_bytes() if out.is_file() else None
    result = judged(got, want, r["channels"], "" if code == 0 else said, code)
    vcode, vsaid = run([oadec, "verify", "--json", stream])
    result["verify_exit"] = vcode
    try:
        report = json.loads(vsaid)
        result["tools"] = report.get("coverage")
        # every finding, the warnings and the notes with the errors
        result["rules"] = sorted({f["rule"] for f in report["diagnostics"]["findings"]})
    except (json.JSONDecodeError, KeyError):
        result["verify_said"] = vsaid[-300:]
    return result


def ac3_snr(want: bytes, got: bytes, delay: int) -> list[float]:
    """Signal to noise in dB of each of two channels of 32-bit float `got`,
    read `delay` samples in, against `want`."""
    w = array("f")
    w.frombytes(want[:len(want) // 8 * 8])
    g = array("f")
    g.frombytes(got[:len(got) // 8 * 8])
    out = []
    for ch in range(2):
        a, b = w[ch::2], g[ch::2][delay:]
        n = min(len(a), len(b))
        signal = sum(x * x for x in a[:n])
        noise = sum((x - y) * (x - y) for x, y in zip(a[:n], b[:n]))
        if n == 0 or signal == 0:
            out.append(-999.0)
        elif noise == 0:
            out.append(999.0)
        else:
            out.append(round(10 * math.log10(signal / noise), 1))
    return out


def ac3_judged(got: bytes | None, want: bytes, said: str, code: int) -> dict:
    """What a decoder made of an AC-3 stream, against the writer's samples."""
    said = said[-400:]
    if got is None or len(got) < 8:
        return {"verdict": "refused", "exit": code, "said": said}
    snr, delay = max((ac3_snr(want, got, delay), delay) for delay in AC3_DELAYS)
    verdict = "target" if min(snr) >= AC3_AGREES_DB else "differs"
    out = {"verdict": verdict, "exit": code, "snr_db": snr, "delay": delay,
           "samples_written": len(got) // 8, "samples_expected": len(want) // 8}
    if said and verdict != "target":
        out["said"] = said
    return out


def ac3_by_ffmpeg(stream: Path, want: bytes, work: Path) -> dict:
    out = work / (stream.stem + ".ffmpeg.f32")
    out.unlink(missing_ok=True)
    code, said = run(["ffmpeg", "-v", "warning", "-nostdin", "-drc_scale", "0", "-f", "ac3", "-i", stream,
                      "-f", "f32le", "-c:a", "pcm_f32le", "-y", out])
    got = out.read_bytes() if code == 0 and out.is_file() else None
    return ac3_judged(got, want, said, code)


def ac3_by_dolby(stream: Path, want: bytes, work: Path) -> dict:
    out = work / (stream.stem + ".dolby.f32")
    out.unlink(missing_ok=True)
    env = dict(os.environ)
    env["GST_PLUGIN_PATH"] = str(PLAYER / "gst-plugins")
    env["PATH"] = str(PLAYER) + os.pathsep + env.get("PATH", "")
    code, said = run([GST, "-q", "filesrc", f"location={stream.as_posix()}", "!", "dlbac3parse", "!",
                      "dlbac3dec", "drc-suppress=true", "out-ch-config=0", "!",
                      "audio/x-raw,format=F32LE", "!", "filesink", f"location={out.as_posix()}"], env=env)
    got = out.read_bytes() if code == 0 and out.is_file() else None
    return ac3_judged(got, want, said, code)


def ac3_by_oadec(oadec: str, stream: Path, want: bytes, work: Path) -> dict:
    out = work / (stream.stem + ".oadec.wav")
    out.unlink(missing_ok=True)
    code, said = run([oadec, "decode", "--format", "wav", "-o", out, stream])
    got = None
    if out.is_file():
        wav = out.read_bytes()
        at = wav.find(b"data")
        if at >= 0:
            got = wav[at + 8:at + 8 + int.from_bytes(wav[at + 4:at + 8], "little")]
    result = ac3_judged(got, want, "" if code == 0 else said, code)
    vcode, _ = run([oadec, "verify", stream])
    result["verify_exit"] = vcode
    return result


def version(cmd) -> str:
    try:
        code, said = run(cmd, timeout=60)
    except (OSError, subprocess.TimeoutExpired):
        return "absent"
    line = said.splitlines()[0] if said else ""
    return re.sub(r"\s+", " ", line)[:120] if code == 0 else "absent"


def main() -> int:
    ap = argparse.ArgumentParser(description=DESCRIPTION)
    ap.add_argument("--out", required=True, type=Path, help="a directory for the streams and the decodes")
    ap.add_argument("--synth", default=str(ROOT / "target" / "release" / f"oadec-synth{EXE}"))
    ap.add_argument("--oadec", default=None, help="the decoder under test (default: as tools/oadec_bin.py finds it)")
    ap.add_argument("--only", default="", help="a regular expression: the recipes to run")
    ap.add_argument("--record", action="store_true", help=f"rewrite {DIGESTS.relative_to(ROOT).as_posix()}")
    ap.add_argument("--json", type=Path, help="write everything that was measured here")
    args = ap.parse_args()
    if not Path(args.synth).is_file():
        raise SystemExit(f"{args.synth} is not there; build it with `cargo build --release -p oadec-synth`")
    oadec = args.oadec or find_oadec()
    for tool in ("ffmpeg", "truehdd"):
        if not shutil.which(tool):
            raise SystemExit(f"`{tool}` is not on PATH, and a recipe needs two decoders from outside")
    dolby = GST.is_file()
    streams = args.out / "streams"
    work = args.out / "decoded"
    for d in (streams, work):
        d.mkdir(parents=True, exist_ok=True)

    code, said = run([args.synth, "all", streams])
    if code != 0:
        raise SystemExit(f"oadec-synth all: {said}")
    code, said = run([args.synth, "digests"])
    if code != 0:
        raise SystemExit(f"oadec-synth digests: {said}")
    # A line is a presentation. The name of a recipe alone is its widest, and
    # `<recipe>.p<index>` the one of the substream at that index under it.
    recipes = []
    for line in said.splitlines():
        name, stream_sha, pcm_sha, rate, channels, units, kind, presentation = line.split("\t")
        recipes.append({"name": name, "recipe": name.split(".p")[0], "presentation": presentation,
                        "widest": ".p" not in name,
                        "stream_sha256": stream_sha, "pcm_sha256": pcm_sha,
                        "rate": int(rate), "channels": int(channels), "units": int(units),
                        "probe": kind == "probe"})
    if args.only:
        recipes = [r for r in recipes if re.search(args.only, r["recipe"])]
        if args.record:
            raise SystemExit("--record is for all the recipes or none")

    unconfirmed = []
    for r in recipes:
        stream = next(streams.glob(r["recipe"] + ".thd"), None) or streams / (r["recipe"] + ".mlp")
        pcm = streams / (r["name"] + ".pcm")
        if sha256(stream) != r["stream_sha256"] or sha256(pcm) != r["pcm_sha256"]:
            raise SystemExit(f"{r['name']}: the files are not what `digests` names")
        want = pcm.read_bytes()
        r["bytes"] = stream.stat().st_size
        r["decoders"] = {
            "ffmpeg": by_ffmpeg(stream, r, want, work),
            "truehdd": by_truehdd(stream, r, want, work),
            "dolby": by_dolby(stream, r, want, work) if dolby
            else {"verdict": "absent", "said": "the Dolby Reference Player is not installed"},
        }
        r["oadec"] = by_oadec(oadec, stream, r, want, work)
        r["gave_the_samples"] = [d for d in INDEPENDENT if r["decoders"][d]["verdict"] == "target"]
        r["gave_others"] = [d for d in INDEPENDENT if r["decoders"][d]["verdict"] == "differs"]
        r["refused"] = [d for d in INDEPENDENT if r["decoders"][d]["verdict"] == "refused"]
        r["confirmed"] = len(r["gave_the_samples"]) >= 2
        if not r["confirmed"] and not r["probe"]:
            unconfirmed.append(r["name"])
        marks = " ".join(f"{d}:{r['decoders'][d]['verdict']}" for d in INDEPENDENT)
        own = (f"oadec:{r['oadec']['verdict']} verify:{r['oadec']['verify_exit']}"
               f" rules:{','.join(r['oadec'].get('rules', ['?'])) or 'none'}")
        mark = "?? " if r["probe"] else "ok " if r["confirmed"] else "NO "
        print(f"{mark} {r['name']:<36} {marks}  {own}")

    # The AC-3 streams: FFmpeg's decoder and Dolby's, by signal to noise
    # against the samples the writer says each stream comes to.
    code, said = run([args.synth, "ac3-digests"])
    if code != 0:
        raise SystemExit(f"oadec-synth ac3-digests: {said}")
    ac3 = []
    for line in said.splitlines():
        name, stream_sha, rate, frames, kind, read = line.split("\t")
        ac3.append({"name": name, "stream_sha256": stream_sha, "rate": int(rate), "frames": int(frames),
                    "probe": kind == "probe", "this_decoder_is_to_give": read})
    if args.only:
        ac3 = [r for r in ac3 if re.search(args.only, r["name"])]
        if not recipes and not ac3:
            raise SystemExit(f"no recipe matches {args.only!r}")
    for r in ac3:
        stream = streams / (r["name"] + ".ac3")
        if sha256(stream) != r["stream_sha256"]:
            raise SystemExit(f"{r['name']}: the file is not what `ac3-digests` names")
        want = (streams / (r["name"] + ".f32")).read_bytes()
        r["bytes"] = stream.stat().st_size
        r["decoders"] = {
            "ffmpeg": ac3_by_ffmpeg(stream, want, work),
            "dolby": ac3_by_dolby(stream, want, work) if dolby
            else {"verdict": "absent", "said": "the Dolby Reference Player is not installed"},
        }
        r["oadec"] = ac3_by_oadec(oadec, stream, want, work)
        r["gave_the_samples"] = [d for d in AC3_OUTSIDE if r["decoders"][d]["verdict"] == "target"]
        r["gave_others"] = [d for d in AC3_OUTSIDE if r["decoders"][d]["verdict"] == "differs"]
        r["refused"] = [d for d in AC3_OUTSIDE if r["decoders"][d]["verdict"] == "refused"]
        r["confirmed"] = len(r["gave_the_samples"]) >= 2
        if not r["confirmed"] and not r["probe"]:
            unconfirmed.append(r["name"])

        def shown(d):
            snr = d.get("snr_db")
            return d["verdict"] + (f" {min(snr):.0f} dB" if snr else "")
        marks = " ".join(f"{d}:{shown(r['decoders'][d])}" for d in AC3_OUTSIDE)
        mark = "?? " if r["probe"] else "ok " if r["confirmed"] else "NO "
        print(f"{mark} {r['name']:<36} {marks}  oadec:{shown(r['oadec'])} verify:{r['oadec']['verify_exit']}")

    probes = [r for r in recipes if r["probe"]]
    recipes = [r for r in recipes if not r["probe"]]
    summary = {
        "versions": {
            "ffmpeg": version(["ffmpeg", "-version"]),
            "truehdd": version(["truehdd", "--version"]),
            "dolby": "the dlbtruehddec and dlbac3dec elements of the Dolby Reference Player" if dolby else "absent",
            "oadec": version([oadec, "--version"]),
        },
        # a recipe is a stream; a stream of several substreams has several presentations
        "recipes": len({r["recipe"] for r in recipes}),
        "presentations": len(recipes),
        "confirmed": len(recipes) - len(unconfirmed),
        "unconfirmed": unconfirmed,
        "by_decoder": {d: {v: sum(1 for r in recipes if r["decoders"][d]["verdict"] == v)
                           for v in ("target", "differs", "refused", "not asked", "absent")}
                       for d in INDEPENDENT},
        "oadec_gave_the_samples": sum(1 for r in recipes if r["oadec"]["verdict"] == "target"),
        "oadec_verify_clean": sum(1 for r in recipes if r["oadec"]["verify_exit"] == 0),
        "oadec_without_a_finding": sum(1 for r in recipes if r["oadec"].get("rules") == []),
        "probes": len({r["recipe"] for r in probes}),
        "presentations_of_probes": len(probes),
        "ac3_recipes": sum(1 for r in ac3 if not r["probe"]),
        "ac3_confirmed": sum(1 for r in ac3 if not r["probe"] and r["confirmed"]),
        "ac3_probes": sum(1 for r in ac3 if r["probe"]),
        "ac3_this_decoder_gave_the_samples": sum(1 for r in ac3 if r["oadec"]["verdict"] == "target"),
    }
    print(json.dumps(summary, indent=1))
    if args.json:
        args.json.write_text(json.dumps({"summary": summary, "recipes": recipes, "probes": probes,
                                         "ac3": ac3}, indent=1)
                             + "\n", encoding="utf-8", newline="\n")
    if args.record:
        if unconfirmed:
            raise SystemExit("not recorded: " + ", ".join(unconfirmed) + " lack two decoders")
        lines = [HEADER]
        for r in recipes:
            lines.append("\t".join([r["name"], r["stream_sha256"], r["pcm_sha256"],
                                    ",".join(r["gave_the_samples"]), ",".join(r["gave_others"]) or "-",
                                    ",".join(r["refused"]) or "-"]) + "\n")
        DIGESTS.parent.mkdir(parents=True, exist_ok=True)
        DIGESTS.write_text("".join(lines), encoding="utf-8", newline="\n")
        print(f"recorded {len(recipes)} presentations of {summary['recipes']} recipes in "
              f"{DIGESTS.relative_to(ROOT).as_posix()}")
        lines = [PROBES_HEADER]
        for r in probes:
            lines.append("\t".join([r["name"], r["stream_sha256"], r["pcm_sha256"],
                                    ",".join(r["gave_the_samples"]) or "-", ",".join(r["gave_others"]) or "-",
                                    ",".join(r["refused"]) or "-"]) + "\n")
        PROBES.write_text("".join(lines), encoding="utf-8", newline="\n")
        print(f"recorded {len(probes)} presentations of {summary['probes']} probes in "
              f"{PROBES.relative_to(ROOT).as_posix()}")
        lines = [AC3_HEADER]
        for r in ac3:
            lines.append("\t".join([r["name"], "probe" if r["probe"] else "reference", r["stream_sha256"],
                                    ",".join(r["gave_the_samples"]) or "-", ",".join(r["gave_others"]) or "-",
                                    ",".join(r["refused"]) or "-"]) + "\n")
        AC3.write_text("".join(lines), encoding="utf-8", newline="\n")
        print(f"recorded {len(ac3)} AC-3 streams in {AC3.relative_to(ROOT).as_posix()}")
    return 1 if unconfirmed else 0


if __name__ == "__main__":
    sys.exit(main())
