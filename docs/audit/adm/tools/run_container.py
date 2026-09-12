#!/usr/bin/env python3
"""Container validation of an ADM BWF (section 5): our walker plus third-party readers.

    python run_container.py --adm <file.wav> --out <json> [--damf <base>] [--dolby] [--ffprobe]

Records: RIFF/RF64 identity, ds64 fields, fmt, data size and frame count,
chunk table with boundaries and padding, 64-bit consistency, our findings,
and the frame counts / verdicts of ffprobe, bwf_info and atmos_info (tagged
files only for atmos_info, whose provenance refusal is recorded separately).
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from admaudit import axml, caf, dolby, profile, provenance, riff  # noqa: E402


def ffprobe(path: str) -> dict:
    rec = provenance.run(["ffprobe", "-v", "error", "-show_entries", "stream=codec_name,sample_rate,channels,bits_per_sample,duration_ts,duration:format=format_name,size,duration", "-of", "json", path], timeout=1800)
    try:
        parsed = json.loads(rec.stdout)
    except Exception:
        parsed = None
    return {"run": rec.to_json(2000), "parsed": parsed}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--adm", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--damf", default=None)
    ap.add_argument("--dolby", action="store_true")
    ap.add_argument("--ffprobe", action="store_true")
    a = ap.parse_args()
    c = riff.scan(a.adm)
    findings = riff.verify(a.adm, c)
    doc = axml.parse(riff.chunk_bytes(a.adm, c.chunk("axml")))
    chna = axml.parse_chna(riff.chunk_bytes(a.adm, c.chunk("chna")))
    ref_findings = axml.check_references(doc) + axml.check_chna(doc, chna, c.fmt.channels) + profile.check(doc, c.fmt.sample_rate, c.fmt.channels, chna)
    start, end = doc.programme_span(c.fmt.sample_rate)
    rep = {
        "file": provenance.file_record(a.adm) if os.path.getsize(a.adm) < 2_000_000_000 else {"path": os.path.abspath(a.adm), "bytes": os.path.getsize(a.adm), "sha256": None},
        "fourcc": c.fourcc, "riff_size_field": c.riff_size_field, "file_size": c.file_size,
        "ds64": c.ds64.__dict__ if c.ds64 else None,
        "fmt": c.fmt.__dict__ | {"extension": c.fmt.extension.hex()},
        "data": {"size_field": c.data.size_field, "size": c.data.size, "offset": c.data.data_offset, "frames": c.frames, "duration_s": c.frames / c.fmt.sample_rate},
        "chunks": [{"id": k.id, "offset": k.offset, "size": k.size, "size_field": k.size_field, "padded": k.padded, "end": k.data_offset + k.size + (1 if k.padded else 0)} for k in c.chunks],
        "gt_4gib": c.file_size > 0xFFFFFFFF,
        "programme_span_samples": {"start": start, "end": end, "matches_frames": end == c.frames},
        "container_findings": [f.__dict__ for f in findings],
        "reference_and_profile_findings": [f.__dict__ for f in ref_findings],
        "element_counts": doc.counts(),
        "chna": {"num_tracks": chna.num_tracks, "num_uids": chna.num_uids, "entries": len(chna.entries)},
    }
    # overflow checks a 32-bit reader would trip on
    rep["u32_overflow"] = {"riff_size_needed": c.file_size - 8, "exceeds_u32": (c.file_size - 8) > 0xFFFFFFFF, "data_exceeds_u32": c.data.size > 0xFFFFFFFF}
    if a.damf:
        info = caf.read_header(a.damf + ".atmos.audio")
        rep["damf_audio"] = {"frames": info.frames, "channels": info.channels, "sample_rate": info.sample_rate, "frames_equal": info.frames == c.frames}
    if a.ffprobe:
        rep["ffprobe"] = ffprobe(a.adm)
        p = rep["ffprobe"]["parsed"]
        if p and p.get("streams"):
            s = p["streams"][0]
            rep["ffprobe"]["frames"] = int(s.get("duration_ts") or 0)
            rep["ffprobe"]["frames_equal"] = rep["ffprobe"]["frames"] == c.frames
    if a.dolby:
        rep["bwf_info"] = dolby.bwf_info(a.adm)
        m = re.search(r"duration \(in samples\):\s*(\d+)|(\d+)\s*(?:frames|samples)", rep["bwf_info"]["run"]["stdout_tail"], re.I)
        if m:
            m = type("M", (), {"group": staticmethod(lambda i=1, _m=m: _m.group(1) or _m.group(2))})()
        rep["bwf_info"]["frames_reported"] = int(m.group(1)) if m else None
        rep["atmos_info_5.7.2"] = dolby.atmos_info(a.adm, True, "5.7.2")
        rep["atmos_info_1.1"] = dolby.atmos_info(a.adm, True, "1.1")
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(rep, f, indent=1, default=str)
        f.write("\n")
    print(json.dumps({k: rep[k] for k in ("fourcc", "riff_size_field", "file_size", "ds64", "gt_4gib", "programme_span_samples", "u32_overflow")}, default=str))
    print("data:", rep["data"], "| container findings:", rep["container_findings"], "| ref/profile findings:", len(rep["reference_and_profile_findings"]))
    for k in ("ffprobe", "bwf_info", "atmos_info_5.7.2", "atmos_info_1.1"):
        if k in rep:
            r = rep[k].get("run", {})
            print(k, "exit", r.get("exit_code"), "frames", rep[k].get("frames", rep[k].get("frames_reported")), "findings", [x["line"][:100] for x in rep[k].get("findings", [])][:3])
    return 0


if __name__ == "__main__":
    sys.exit(main())
