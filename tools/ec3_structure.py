#!/usr/bin/env python3
"""Walk an AC-3 / E-AC-3 elementary stream and report its substream structure.

Written from the bitstream, independently of oadec, so it can be used as a check
on oadec rather than a restatement of it. Reads the syncframe headers only:
strmtyp, substreamid, frmsiz, fscod, numblkscod, acmod, lfeon, bsid, dialnorm,
compr, and for dependent substreams chanmape and chanmap (TS 102 366 clause
E.1.3.1, table E.1.4).

    python ec3_structure.py stream.ec3 [--frames N] [--json out.json]
"""
from __future__ import annotations
import argparse, json, sys
from collections import Counter

# Table 4.1: channels per acmod, front/surround naming from table 4.3.
NFCHANS = [2, 1, 2, 3, 3, 4, 4, 5]
CHANNEL_ORDER = [["Ch1", "Ch2"], ["C"], ["L", "R"], ["L", "C", "R"],
                 ["L", "R", "S"], ["L", "C", "R", "S"],
                 ["L", "R", "Ls", "Rs"], ["L", "C", "R", "Ls", "Rs"]]
# Table E.1.4, bit 0 in the most significant bit of the 16-bit field.
CHANMAP = [["L"], ["C"], ["R"], ["Ls"], ["Rs"], ["Lc", "Rc"], ["Lrs", "Rrs"],
           ["Cs"], ["Ts"], ["Lsd", "Rsd"], ["Lw", "Rw"], ["Vhl", "Vhr"],
           ["Vhc"], ["Lts", "Rts"], ["LFE2"], ["LFE"]]
# AC-3 table 5.18, words per frame at 48 kHz, indexed by frmsizecod >> 1.
AC3_BITRATE = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256,
               320, 384, 448, 512, 576, 640]


class Bits:
    def __init__(self, data: bytes, pos: int = 0):
        self.d, self.p = data, pos

    def u(self, n: int) -> int:
        v = 0
        for _ in range(n):
            v = (v << 1) | ((self.d[self.p >> 3] >> (7 - (self.p & 7))) & 1)
            self.p += 1
        return v


def parse_frame(data: bytes, off: int) -> dict | None:
    if data[off:off + 2] != b"\x0b\x77":
        return None
    bsid = (data[off + 5] >> 3) & 0x1F
    r = Bits(data, (off + 2) * 8)
    if bsid <= 10:                                   # AC-3 syntax
        r.u(16)                                      # crc1
        fscod, frmsizecod = r.u(2), r.u(6)
        r.u(5)                                       # bsid
        bsmod = r.u(3)
        acmod = r.u(3)
        if acmod not in (0, 1) and acmod & 1:
            r.u(2)                                   # cmixlev
        if acmod & 4:
            r.u(2)                                   # surmixlev
        if acmod == 2:
            r.u(2)                                   # dsurmod
        lfeon = r.u(1)
        dialnorm = r.u(5)
        if frmsizecod >> 1 >= len(AC3_BITRATE) or fscod == 3:
            return None
        words = {0: 2, 1: 2, 2: 2}[fscod] * 0 + 0     # placeholder, filled below
        rate = AC3_BITRATE[frmsizecod >> 1]
        # clause 5.3.3: words per syncframe at 48/44,1/32 kHz
        words = {0: rate * 2, 2: rate * 3}.get(fscod)
        if fscod == 1:
            words = (rate * 1536 // 44100 + (frmsizecod & 1)) * 2
        return {"syntax": "AC-3", "strmtyp": 0, "substreamid": 0, "bsid": bsid,
                "bytes": words * 2, "fscod": fscod, "blocks": 6, "acmod": acmod,
                "lfeon": lfeon, "bsmod": bsmod, "dialnorm": -dialnorm,
                "bit_rate": rate * 1000, "chanmape": None, "chanmap": None,
                "channels": CHANNEL_ORDER[acmod] + (["LFE"] if lfeon else [])}
    # E-AC-3 syntax
    strmtyp, ssid, frmsiz, fscod = r.u(2), r.u(3), r.u(11), r.u(2)
    fscod2 = None
    if fscod == 3:
        fscod2, blocks = r.u(2), 6
    else:
        blocks = [1, 2, 3, 6][r.u(2)]
    acmod, lfeon = r.u(3), r.u(1)
    r.u(5)                                            # bsid
    dialnorm = r.u(5)
    compr = r.u(8) if r.u(1) else None
    if acmod == 0:
        r.u(5)
        if r.u(1):
            r.u(8)
    chanmape = chanmap = None
    if strmtyp == 1:
        chanmape = r.u(1)
        if chanmape:
            chanmap = r.u(16)
    if chanmap is None:
        chans = CHANNEL_ORDER[acmod] + (["LFE"] if lfeon else [])
    else:
        chans = [c for b in range(16) if chanmap & (1 << (15 - b)) for c in CHANMAP[b]]
    return {"syntax": "E-AC-3", "strmtyp": strmtyp, "substreamid": ssid,
            "bsid": bsid, "bytes": (frmsiz + 1) * 2, "fscod": fscod,
            "fscod2": fscod2, "blocks": blocks, "acmod": acmod, "lfeon": lfeon,
            "dialnorm": -dialnorm, "compr": compr, "chanmape": chanmape,
            "chanmap": chanmap, "channels": chans}


def walk(path: str, limit: int | None):
    data = open(path, "rb").read()
    off, frames, groups, group = 0, [], [], []
    while off + 6 <= len(data):
        f = parse_frame(data, off)
        if f is None or f["bytes"] <= 0 or off + f["bytes"] > len(data):
            break
        f["offset"] = off
        if f["strmtyp"] == 1:
            group.append(f)
        else:
            if group:
                groups.append(group)
            group = [f]
        frames.append(f)
        off += f["bytes"]
        if limit and len(groups) >= limit:
            break
    if group:
        groups.append(group)
    return data, frames, groups, off


def describe(path: str, limit: int | None) -> dict:
    data, frames, groups, consumed = walk(path, limit)
    shapes = Counter()
    for g in groups:
        shapes[tuple((p["syntax"], p["strmtyp"], p["substreamid"], p["acmod"],
                      p["lfeon"], p["chanmap"], p["bytes"]) for p in g)] += 1
    first = groups[0] if groups else []
    merged, sources = [], []
    for i, part in enumerate(first):
        for ci, name in enumerate(part["channels"]):
            if name in merged:
                sources[merged.index(name)] = (i, ci)      # replace, clause E.2.8.2
            else:
                merged.append(name); sources.append((i, ci))
    return {
        "file": path, "bytes": len(data), "bytes_consumed": consumed,
        "frames": len(frames), "groups": len(groups),
        "independent_frames": sum(1 for f in frames if f["strmtyp"] != 1),
        "dependent_frames": sum(1 for f in frames if f["strmtyp"] == 1),
        "substreams": {f"type {k[0]} id {k[1]}": v for k, v in
                       sorted(Counter((f["strmtyp"], f["substreamid"]) for f in frames).items())},
        "distinct_group_shapes": len(shapes),
        "first_group": [{k: v for k, v in p.items() if k != "offset"} for p in first],
        "programme_channels": merged,
        "programme_sources": sources,
        "group_bytes": first and sum(p["bytes"] for p in first),
    }


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("file"); ap.add_argument("--groups", type=int, default=0)
    ap.add_argument("--json", default="")
    a = ap.parse_args()
    d = describe(a.file, a.groups or None)
    print(json.dumps(d, indent=1))
    if a.json:
        open(a.json, "w").write(json.dumps(d, indent=1))
