#!/usr/bin/env python3
"""Which decoders give back the samples a written stream was made from?

`crates/oadec-synth` writes TrueHD and MLP streams from recipes, and knows the
samples of each before any decoder has read it. That makes a stream a test of
a decoder only if the writer is right, and the writer shares its reading of
the format with the decoder of this repository: the same documents, the same
author. So a recipe is taken as a reference here, and nowhere else, and only
on the word of decoders that share nothing with either:

* FFmpeg (`-f truehd` or `-f mlp`, to 24-bit PCM),
* `truehdd` (`decode --strict --presentation 0 --format pcm`),
* Dolby's own decoder, the `dlbtruehddec` element that ships with the Dolby
  Reference Player, on the stream wrapped in MP4 (it takes no raw stream).
  Only where the player is installed, and only for the TrueHD syntax.

Each is run as a program on a file and judged by the bytes it writes. A recipe
is confirmed when at least two of them give the samples of the recipe, every
one of them. What each said goes into the record, the ones that differed
included: a stream on which the decoders disagree is a finding, not a failure
of the tool.

    cargo build --release -p oadec-synth -p oadec-cli
    python tools/synth_accept.py --out oadec-work/synth
    python tools/synth_accept.py --out oadec-work/synth --record

`--record` rewrites `crates/oadec-synth/specs/digests.tsv`, which the tests of
that crate hold the writer to. It refuses when a recipe is not confirmed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from oadec_bin import find as find_oadec  # noqa: E402

DESCRIPTION = "Which decoders give back the samples a written stream was made from?"
ROOT = Path(__file__).resolve().parent.parent
EXE = ".exe" if os.name == "nt" else ""
PLAYER = Path(r"C:\Program Files\Dolby\Dolby Reference Player")
GST = PLAYER / f"gst-launch-1.0{EXE}"
DIGESTS = ROOT / "crates" / "oadec-synth" / "specs" / "digests.tsv"
INDEPENDENT = ("ffmpeg", "truehdd", "dolby")
HEADER = """\
# What `oadec-synth digests` has to print, and which decoders said so.
#
# Written by `tools/synth_accept.py --record` and by nothing else. A line is a
# recipe: its name, the SHA-256 of the stream, the SHA-256 of its samples as
# 24-bit little-endian PCM, the decoders that gave those samples back, the
# ones that gave others, and the ones that would not read the stream (`-` for
# none). A recipe is here because two decoders that share nothing with this
# repository agree on it.
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


def by_ffmpeg(stream: Path, want: bytes, channels: int, work: Path) -> dict:
    out = work / (stream.stem + ".ffmpeg.pcm")
    out.unlink(missing_ok=True)
    fmt = "mlp" if stream.suffix == ".mlp" else "truehd"
    code, said = run(["ffmpeg", "-v", "warning", "-nostdin", "-f", fmt, "-i", stream,
                      "-f", "s24le", "-c:a", "pcm_s24le", "-y", out])
    got = out.read_bytes() if code == 0 and out.is_file() else None
    return judged(got, want, channels, said, code)


def by_truehdd(stream: Path, want: bytes, channels: int, work: Path) -> dict:
    base = work / (stream.stem + ".truehdd")
    out = Path(str(base) + ".pcm")
    out.unlink(missing_ok=True)
    code, said = run(["truehdd", "--loglevel", "warn", "decode", "--strict", "--presentation", "0",
                      "--format", "pcm", "--output-path", base, stream])
    got = out.read_bytes() if code == 0 and out.is_file() else None
    return judged(got, want, channels, said, code)


def by_dolby(stream: Path, want: bytes, channels: int, work: Path) -> dict:
    if stream.suffix != ".thd":
        return {"verdict": "not asked", "said": "the element is given the TrueHD syntax only"}
    mp4 = work / (stream.stem + ".mp4")
    out = work / (stream.stem + ".dolby.s32")
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
                      "dlbtruehdparse", "!", "dlbtruehddec", "presentation=2", "out-ch-config=0", "!",
                      "audio/x-raw,format=S32LE", "!", "filesink", f"location={out.as_posix()}"], env=env)
    if code != 0 or not out.is_file():
        return {"verdict": "refused", "exit": code, "said": said[-600:]}
    words = out.read_bytes()
    # 24 bits at the top of each 32-bit word; anything under them is not the stream's
    if any(words[i] for i in range(0, len(words), 4)):
        return {"verdict": "differs", "exit": code, "said": "bits under the 24 the stream codes"}
    got = b"".join(words[i + 1:i + 4] for i in range(0, len(words), 4))
    return judged(got, want, channels, said, code)


def by_oadec(oadec: str, stream: Path, want: bytes, channels: int, work: Path) -> dict:
    out = work / (stream.stem + ".oadec.pcm")
    out.unlink(missing_ok=True)
    code, said = run([oadec, "decode", "-p", "0", "--format", "pcm", "-o", out, stream])
    got = out.read_bytes() if out.is_file() else None
    result = judged(got, want, channels, "" if code == 0 else said, code)
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
    recipes = []
    for line in said.splitlines():
        name, stream_sha, pcm_sha, rate, channels, units = line.split("\t")
        recipes.append({"name": name, "stream_sha256": stream_sha, "pcm_sha256": pcm_sha,
                        "rate": int(rate), "channels": int(channels), "units": int(units)})
    if args.only:
        recipes = [r for r in recipes if re.search(args.only, r["name"])]
        if not recipes:
            raise SystemExit(f"no recipe matches {args.only!r}")
        if args.record:
            raise SystemExit("--record is for all the recipes or none")

    unconfirmed = []
    for r in recipes:
        stream = next(streams.glob(r["name"] + ".thd"), None) or streams / (r["name"] + ".mlp")
        pcm = streams / (r["name"] + ".pcm")
        if sha256(stream) != r["stream_sha256"] or sha256(pcm) != r["pcm_sha256"]:
            raise SystemExit(f"{r['name']}: the files are not what `digests` names")
        want = pcm.read_bytes()
        r["bytes"] = stream.stat().st_size
        r["decoders"] = {
            "ffmpeg": by_ffmpeg(stream, want, r["channels"], work),
            "truehdd": by_truehdd(stream, want, r["channels"], work),
            "dolby": by_dolby(stream, want, r["channels"], work) if dolby
            else {"verdict": "absent", "said": "the Dolby Reference Player is not installed"},
        }
        r["oadec"] = by_oadec(oadec, stream, want, r["channels"], work)
        r["gave_the_samples"] = [d for d in INDEPENDENT if r["decoders"][d]["verdict"] == "target"]
        r["gave_others"] = [d for d in INDEPENDENT if r["decoders"][d]["verdict"] == "differs"]
        r["refused"] = [d for d in INDEPENDENT if r["decoders"][d]["verdict"] == "refused"]
        r["confirmed"] = len(r["gave_the_samples"]) >= 2
        if not r["confirmed"]:
            unconfirmed.append(r["name"])
        marks = " ".join(f"{d}:{r['decoders'][d]['verdict']}" for d in INDEPENDENT)
        own = (f"oadec:{r['oadec']['verdict']} verify:{r['oadec']['verify_exit']}"
               f" rules:{','.join(r['oadec'].get('rules', ['?'])) or 'none'}")
        print(f"{'ok ' if r['confirmed'] else 'NO '} {r['name']:<26} {marks}  {own}")

    summary = {
        "versions": {
            "ffmpeg": version(["ffmpeg", "-version"]),
            "truehdd": version(["truehdd", "--version"]),
            "dolby": "the dlbtruehddec element of the Dolby Reference Player" if dolby else "absent",
            "oadec": version([oadec, "--version"]),
        },
        "recipes": len(recipes),
        "confirmed": len(recipes) - len(unconfirmed),
        "unconfirmed": unconfirmed,
        "by_decoder": {d: {v: sum(1 for r in recipes if r["decoders"][d]["verdict"] == v)
                           for v in ("target", "differs", "refused", "not asked", "absent")}
                       for d in INDEPENDENT},
        "oadec_gave_the_samples": sum(1 for r in recipes if r["oadec"]["verdict"] == "target"),
        "oadec_verify_clean": sum(1 for r in recipes if r["oadec"]["verify_exit"] == 0),
        "oadec_without_a_finding": sum(1 for r in recipes if r["oadec"].get("rules") == []),
    }
    print(json.dumps(summary, indent=1))
    if args.json:
        args.json.write_text(json.dumps({"summary": summary, "recipes": recipes}, indent=1) + "\n",
                             encoding="utf-8", newline="\n")
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
        print(f"recorded {len(recipes)} recipes in {DIGESTS.relative_to(ROOT).as_posix()}")
    return 1 if unconfirmed else 0


if __name__ == "__main__":
    sys.exit(main())
