#!/usr/bin/env python3
"""Structural comparison of two ADM BWF files (Dolby Atmos master profile).

Compares the RIFF chunk list, the format chunk, the `chna` entries, the ADM
element inventory (ids, names, references), every object's audioBlockFormat
sequence (times, positions and the other subelements, with a tolerance) and
the `dbmd` segments. Prints the differences and exits 1 when any were found.

    python tools/adm_diff.py ours.wav reference.wav [--time-tolerance 0.00002]
"""

import argparse
import re
import struct
import sys
import xml.etree.ElementTree as ET

NS = {"e": "urn:ebu:metadata-schema:ebuCore_2016"}


def chunks(path):
    out = []
    with open(path, "rb") as f:
        head = f.read(12)
        riff, size, wave = struct.unpack("<4sI4s", head)
        pos = 12
        data = {}
        while True:
            f.seek(pos)
            h = f.read(8)
            if len(h) < 8:
                break
            cid, csize = struct.unpack("<4sI", h)
            if cid in (b"ds64",):
                payload = f.read(csize)
                csize_real = struct.unpack("<Q", payload[8:16])[0] if len(payload) >= 16 else csize
                data[cid] = payload
                out.append((cid.decode(), csize))
                pos += 8 + csize + (csize & 1)
                continue
            if cid == b"data" and csize == 0xFFFFFFFF and b"ds64" in data:
                real = struct.unpack("<Q", data[b"ds64"][8:16])[0]
                out.append(("data", real))
                pos += 8 + real + (real & 1)
                continue
            if cid in (b"fmt ", b"axml", b"chna", b"dbmd", b"bext"):
                data[cid] = f.read(csize)
            out.append((cid.decode(), csize))
            pos += 8 + csize + (csize & 1)
    return riff.decode(), out, data


def parse_axml(xml_bytes):
    root = ET.fromstring(xml_bytes)
    afe = root.find(".//e:audioFormatExtended", NS)
    inv = {}
    for tag in [
        "audioProgramme",
        "audioContent",
        "audioObject",
        "audioPackFormat",
        "audioChannelFormat",
        "audioStreamFormat",
        "audioTrackFormat",
        "audioTrackUID",
    ]:
        inv[tag] = afe.findall(f"e:{tag}", NS)
    return afe, inv


def elem_summary(el):
    attrs = {k: v for k, v in el.attrib.items()}
    refs = [(c.tag.split("}")[1], c.text) for c in el if c.tag.split("}")[1].endswith("IDRef")]
    return attrs, refs


def block_summary(b):
    d = {"id": b.get("audioBlockFormatID"), "rtime": b.get("rtime"), "duration": b.get("duration")}
    for c in b:
        t = c.tag.split("}")[1]
        if t == "position":
            d["pos_" + c.get("coordinate")] = float(c.text)
        elif t == "jumpPosition":
            d["jump"] = (c.text, c.get("interpolationLength"))
        elif t == "zoneExclusion":
            d["zones"] = sorted(z.text for z in c)
        elif t == "speakerLabel":
            d["speaker"] = c.text
        else:
            d[t] = c.text
    return d


def tc_to_seconds(tc):
    if tc is None:
        return None
    h, m, s = tc.split(":")
    return int(h) * 3600 + int(m) * 60 + float(s)


def compare(a_path, b_path, time_tol, pos_tol):
    problems = []
    ra, ca, da = chunks(a_path)
    rb, cb, db = chunks(b_path)
    print(f"A: {a_path}\n   {ra} chunks {ca}")
    print(f"B: {b_path}\n   {rb} chunks {cb}")
    if [c for c, _ in ca] != [c for c, _ in cb]:
        problems.append(f"chunk order differs: {[c for c, _ in ca]} vs {[c for c, _ in cb]}")
    fa, fb = da.get(b"fmt "), db.get(b"fmt ")
    if fa[:16] != fb[:16]:
        problems.append(f"fmt differs: {struct.unpack('<HHIIHH', fa[:16])} vs {struct.unpack('<HHIIHH', fb[:16])}")
    else:
        print("fmt:", struct.unpack("<HHIIHH", fa[:16]))
    if dict(ca).get("data") != dict(cb).get("data"):
        problems.append(f"data size differs: {dict(ca).get('data')} vs {dict(cb).get('data')}")
    # chna
    for name, d in (("A", da), ("B", db)):
        c = d.get(b"chna")
        n, u = struct.unpack("<HH", c[:4])
        print(f"chna {name}: tracks {n} uids {u}")
    if da.get(b"chna") != db.get(b"chna"):
        ca_e = [da[b"chna"][4 + 40 * i : 4 + 40 * (i + 1)] for i in range(struct.unpack("<H", da[b"chna"][2:4])[0])]
        cb_e = [db[b"chna"][4 + 40 * i : 4 + 40 * (i + 1)] for i in range(struct.unpack("<H", db[b"chna"][2:4])[0])]
        for i, (x, y) in enumerate(zip(ca_e, cb_e)):
            if x != y:
                problems.append(f"chna entry {i} differs: {x} vs {y}")
        if len(ca_e) != len(cb_e):
            problems.append(f"chna entry count differs: {len(ca_e)} vs {len(cb_e)}")
    # axml
    afa, ia = parse_axml(da[b"axml"])
    afb, ib = parse_axml(db[b"axml"])
    for tag in ia:
        if len(ia[tag]) != len(ib[tag]):
            problems.append(f"{tag} count differs: {len(ia[tag])} vs {len(ib[tag])}")
        else:
            print(f"{tag}: {len(ia[tag])}")
        for ea, eb in zip(ia[tag], ib[tag]):
            sa, sb = elem_summary(ea), elem_summary(eb)
            if tag == "audioChannelFormat":
                pass
            if sa != sb:
                problems.append(f"{tag} differs:\n    A {sa}\n    B {sb}")
    # blocks per channel format
    for ea, eb in zip(ia["audioChannelFormat"], ib["audioChannelFormat"]):
        ba = [block_summary(b) for b in ea.findall("e:audioBlockFormat", NS)]
        bb = [block_summary(b) for b in eb.findall("e:audioBlockFormat", NS)]
        name = ea.get("audioChannelFormatID")
        if len(ba) != len(bb):
            problems.append(f"{name}: block count {len(ba)} vs {len(bb)}")
        for x, y in zip(ba, bb):
            keys = set(x) | set(y)
            for k in sorted(keys):
                va, vb = x.get(k), y.get(k)
                if k in ("rtime", "duration"):
                    ta, tb = tc_to_seconds(va), tc_to_seconds(vb)
                    if (ta is None) != (tb is None) or (ta is not None and abs(ta - tb) > time_tol):
                        problems.append(f"{name} {x['id']}: {k} {va} vs {vb}")
                elif k.startswith("pos_"):
                    if va is None or vb is None:
                        if (va or 0.0) != (vb or 0.0):
                            problems.append(f"{name} {x['id']}: {k} {va} vs {vb}")
                    elif abs(va - vb) > pos_tol:
                        problems.append(f"{name} {x['id']}: {k} {va} vs {vb}")
                elif k == "id":
                    if va != vb:
                        problems.append(f"{name}: block id {va} vs {vb}")
                elif va != vb:
                    problems.append(f"{name} {x['id']}: {k} {va!r} vs {vb!r}")
    # dbmd
    def segs(d):
        pos = 4
        out = []
        while pos < len(d) and d[pos] != 0:
            sid = d[pos]
            size = int.from_bytes(d[pos + 1 : pos + 3], "little")
            out.append((sid, d[pos + 3 : pos + 3 + size]))
            pos += 4 + size
        return out

    sa, sb = segs(da[b"dbmd"]), segs(db[b"dbmd"])
    print("dbmd segments A", [(s, len(p)) for s, p in sa], "B", [(s, len(p)) for s, p in sb])
    for (s1, p1), (s2, p2) in zip(sa, sb):
        if s1 != s2 or len(p1) != len(p2):
            problems.append(f"dbmd segment {s1}/{s2} size {len(p1)}/{len(p2)}")
            continue
        diff = [i for i in range(len(p1)) if p1[i] != p2[i]]
        if diff:
            # tool name strings are expected to differ in segment 9
            if s1 == 9 and all(i < 96 for i in diff):
                print(f"dbmd segment 9: only the tool strings differ ({len(diff)} bytes)")
            else:
                problems.append(f"dbmd segment {s1}: {len(diff)} bytes differ at {diff[:12]}")
    print()
    if problems:
        print(f"{len(problems)} differences:")
        for p in problems[:80]:
            print(" -", p)
        return 1
    print("no structural differences")
    return 0


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("a")
    ap.add_argument("b")
    ap.add_argument("--time-tolerance", type=float, default=0.00002)
    ap.add_argument("--position-tolerance", type=float, default=1e-6)
    args = ap.parse_args()
    sys.exit(compare(args.a, args.b, args.time_tolerance, args.position_tolerance))
