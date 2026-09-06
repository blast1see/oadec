"""Extract raw TrueHD / E-AC-3 / AC-3 elementary streams from the test MKVs.

Reads each MKV once (mkvextract can pull several tracks in one pass) and writes a
manifest with sizes.  Real media never enters the repository; this only fills the
work directory named by OADEC_MEDIA (default E:\oadec-work).
"""
from __future__ import annotations

import glob
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

MKVMERGE = r"C:\temp\mkvmerge.exe"
MKVEXTRACT = r"C:\temp\mkvextract.exe"
WORK = Path(os.environ.get("OADEC_MEDIA", r"E:\oadec-work"))

# (mkv path, [(codec_id, language, destination relative to WORK)])
JOBS = [
    (r"E:\Pi (1998).mkv",
     [("A_TRUEHD", "eng", "thd/pi.thd"), ("A_AC3", "eng", "ec3/pi-dd51.ac3")]),
    (r"E:\series\The Day Of The Jackal S01\The Day Of The Jackal S01E07.mkv",
     [("A_EAC3", "eng", "ec3/jackal-s01e07.ec3")]),
    (r"E:\Disclosure Day (2026).mkv",
     [("A_EAC3", "eng", "ec3/disclosure-web.ec3")]),
    (r"E:\films\Talk to Me (2023).mkv",
     [("A_TRUEHD", "eng", "thd/talktome.thd"), ("A_EAC3", "eng", "ec3/talktome-joc.ec3"), ("A_AC3", "eng", "ec3/talktome-dd51.ac3")]),
    (r"E:\films\Shaun of the Dead (2004).mkv",
     [("A_TRUEHD", "eng", "thd/shaun.thd"), ("A_EAC3", "eng", "ec3/shaun-joc1024.ec3")]),
    (r"E:\films\Knives Out (2019).mkv",
     [("A_TRUEHD", "eng", "thd/knivesout.thd"), ("A_EAC3", "eng", "ec3/knivesout-joc.ec3")]),
    (r"E:\films\The Kings Man (2021).mkv",
     [("A_TRUEHD", "eng", "thd/kingsman.thd"), ("A_EAC3", "eng", "ec3/kingsman-joc.ec3")]),
    (r"E:\Children of Men (2006).mkv",
     [("A_EAC3", None, "ec3/childrenofmen-ddp51.ec3")]),
]

COPIES = [
    (r"E:\samples\Nightcrawler (2014)-English.ec3", "ec3/nightcrawler.ec3"),
    (r"E:\samples\Nightcrawler (2014)-Turkish.ac3", "ec3/nightcrawler-dd20.ac3"),
    (r"E:\samples\Hannibal (2001) - 2 - E-AC3, [tur], 2.0 channels, 224kbps, 48kHz.eac3", "ec3/hannibal-ddp20.eac3"),
]
BRAVEHEART_GLOB = r"%USERPROFILE%\Braveheart*\*.thd"


def identify(mkv: str) -> list[dict]:
    out = subprocess.run([MKVMERGE, "-J", mkv], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if out.returncode != 0:
        raise RuntimeError(f"mkvmerge -J failed for {mkv}: {out.stderr[:400]}")
    return json.loads(out.stdout)["tracks"]


def pick(tracks: list[dict], codec_id: str, lang: str | None) -> dict | None:
    cands = [t for t in tracks if t["type"] == "audio" and t["properties"].get("codec_id") == codec_id]
    if lang:
        pref = [t for t in cands if t["properties"].get("language") == lang]
        cands = pref or cands
    return cands[0] if cands else None


def main() -> int:
    manifest_path = WORK / "ref" / "corpus.json"
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest: dict = {"created": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "streams": {}}
    only = set(sys.argv[1:])

    for mkv, wanted in JOBS:
        if only and not any(Path(dst).stem in only for _, _, dst in wanted):
            continue
        if not Path(mkv).exists():
            print(f"MISSING  {mkv}", flush=True)
            continue
        tracks = identify(mkv)
        args = []
        for codec_id, lang, dst in wanted:
            dst_path = WORK / dst
            if dst_path.exists() and dst_path.stat().st_size > 0:
                print(f"SKIP     {dst} (exists)", flush=True)
                continue
            t = pick(tracks, codec_id, lang)
            if t is None:
                print(f"NOTRACK  {codec_id}/{lang} in {Path(mkv).name}", flush=True)
                continue
            args.append((t["id"], dst_path, t))
        if not args:
            continue
        cmd = [MKVEXTRACT, "tracks", mkv] + [f"{tid}:{p}" for tid, p, _ in args]
        t0 = time.time()
        print(f"EXTRACT  {Path(mkv).name} -> {[str(p.name) for _, p, _ in args]}", flush=True)
        res = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
        dt = time.time() - t0
        if res.returncode != 0:
            print(f"FAILED   {res.stderr[:400]}", flush=True)
            continue
        for tid, p, t in args:
            manifest["streams"][p.name] = {
                "source": mkv, "track_id": tid, "codec_id": t["properties"].get("codec_id"),
                "language": t["properties"].get("language"), "track_name": t["properties"].get("track_name"),
                "bytes": p.stat().st_size, "seconds": round(dt, 1),
            }
            print(f"DONE     {p.name}  {p.stat().st_size/1e9:.3f} GB  ({dt:.0f} s)", flush=True)

    for src, dst in COPIES + [(g, "thd/braveheart.thd") for g in glob.glob(BRAVEHEART_GLOB)[:1]]:
        dst_path = WORK / dst
        if dst_path.exists() and dst_path.stat().st_size > 0:
            print(f"SKIP     {dst} (exists)", flush=True)
            continue
        if not Path(src).exists():
            print(f"MISSING  {src}", flush=True)
            continue
        t0 = time.time()
        shutil.copyfile(src, dst_path)
        manifest["streams"][dst_path.name] = {"source": src, "bytes": dst_path.stat().st_size, "seconds": round(time.time() - t0, 1)}
        print(f"COPIED   {dst}  {dst_path.stat().st_size/1e9:.3f} GB", flush=True)

    manifest_path.write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print(f"MANIFEST {manifest_path}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
