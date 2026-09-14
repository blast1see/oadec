#!/usr/bin/env python3
"""Validation runs of the ADM remediation (branch ``adm-remediation``).

    python run_remediation.py --repo <repo> --work <dir> --stage harness|regression|dolby|render|evidence|all
                              [--harness <adm_harness.exe>] [--oadec <oadec.exe>] [--media E:/oadec-work]
                              [--no-dolby] [--no-ear] [--with-full-film] [--only C02,C09]

The measure is the 2026-09-11 audit toolkit (``docs/audit/adm/tools``), imported
read-only: the same normaliser, ledger, tiling check, profile checker and PCM
comparison that found the defects now judge the fixes.  Stages:

* ``harness``     the audit's writer-level cases C01-C21b plus remediation
                  variants, with an expectation per case (what the fix must
                  change and what must stay);
* ``regression``  every audited decode of the work directory replayed with the
                  new binary and its outputs compared byte for byte with the
                  hashes in ``adm-work-inventory.json``;
* ``dolby``       the Conversion Tool reads oadec's new gains back, Dolby's
                  validators judge the tagged file, Dolby's object numbering
                  is compared;
* ``render``      the EBU ADM Renderer: oadec's gain file against Dolby's, and
                  the real-ramp mode against the audit's real-ramp variant;
* ``evidence``    the raw stage outputs become evidence envelopes under
                  ``docs/audit/evidence/adm-remediation/``.

Exit code 1 when an expectation is not met, so CI can run the harness stage.
"""
from __future__ import annotations

import re
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))


def tools_dir(repo: str) -> str:
    return os.path.join(repo, "docs", "audit", "adm", "tools")


# --------------------------------------------------------------------------- helpers

def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def load(path: str):
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def dump(path: str, obj) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(obj, f, indent=1, default=lambda o: sorted(o) if isinstance(o, set) else str(o))
        f.write("\n")


def axml_text(adm: str) -> str:
    from admaudit import riff
    c = riff.scan(adm)
    return riff.chunk_bytes(adm, c.chunk("axml")).decode("utf-8", "replace")


def file_bytes_contain(path: str, needle: bytes) -> bool:
    with open(path, "rb") as f:
        return needle in f.read()


# --------------------------------------------------------------------------- harness stage

def remediation_cases() -> dict:
    import run_harness
    C = run_harness.cases()
    for heavy in ("C22-rf64", "C22b-riff-max"):
        C.pop(heavy, None)  # the RF64 logic was not touched; the audit's runs stand
    C["C13b-isf-drop"] = dict(C["C13-isf"], case="C13b-isf-drop", isf_drop=True,
                              why="C13 with the drop policy (--isf drop)",
                              expect="written without the ISF elements; ledger isf-dropped 4 (declared loss)")
    C["C15b-96k-allowed"] = dict(C["C15-96k"], case="C15b-96k-allowed", allow_non_profile_rate=True,
                                 why="C15 with --adm-allow-non-profile-rate",
                                 expect="written at 96 kHz, interpolationLength 0.002604, ledger non-profile-sample-rate 1, profile finding stays")
    C["C20b-real-ramps"] = dict(C["C20-ramp-only"], case="C20b-real-ramps", real_ramps=True,
                                why="C20 with --adm-interpolation real",
                                expect="ramp-only changes are blocks, ledger loss.ramp 0, trajectory 0, dbmd marked non-profile")
    C["R01-119-objects"] = {
        "case": "R01-119-objects", "why": "profile table 17 numbers 118 objects; 119 must be refused",
        "program": {"beds": [["LFE"]], "isf_objects": 0, "dynamic_objects": 119}, "frames": 4800, "bed_conform": False,
        "audio": {"hz": [0] * 120}, "sample_rate": 48000,
        "events": [run_harness.ev(3, 0, bed={})] + [run_harness.ev(10 + k, 0, object=run_harness.obj((0, 1, 0))) for k in range(119)],
        "expect": "AdmWriter refuses (TooManyObjects); DAMF is written",
    }
    C["R02-gain-tagged"] = dict(C["C02-gain"], case="R02-gain-tagged", creator="Created using Dolby equipment",
                                why="C02 with the Dolby origin tag, for atmos_info",
                                expect="same file as C02 with the Dolby creator string; the validators accept it")
    return C


def ct_blocks_from_audit(repo: str, case: str) -> dict | None:
    """Dolby's block lists for a harness case, as the audit recorded them."""
    ev = load(os.path.join(repo, "docs", "audit", "evidence", "adm", "adm-harness.json")) or {}
    ct = (ev.get(case) or {}).get("ct") or {}
    blocks = ct.get("adm_blocks")
    if not blocks:
        return None
    return {k: [(b["t"], b["dur"]) for b in v] for k, v in blocks.items()}


def expectations(repo: str, name: str, e: dict) -> list:
    """What the remediated writers must do on each case; returns the failures."""
    f = []

    def eq(label, got, want):
        if got != want:
            f.append(f"{label}: got {got!r}, wanted {want!r}")

    def has(label, text, needle):
        if needle not in text:
            f.append(f"{label}: {needle!r} missing")

    def lacks(label, text, needle):
        if needle in text:
            f.append(f"{label}: {needle!r} present")

    cmp_ = e.get("compare") or {}
    cl = cmp_.get("classes") or {}
    loss = cmp_.get("loss") or {}
    til = cmp_.get("tiling") or {}
    findings = [x["kind"] for x in cmp_.get("adm_findings", [])]
    summ = e.get("summary") or {}
    al = ((summ.get("adm") or {}).get("losses") or {}).get("counts", {})
    dl = ((summ.get("damf") or {}).get("losses") or {}).get("counts", {})
    tl = (summ.get("timeline_losses") or {}).get("counts", {})
    adm_err = summ.get("adm_error") or ""
    damf_err = summ.get("damf_error") or ""
    text = e.get("axml") or ""
    blocks_full = e.get("adm_blocks") or {}
    blocks = {k: [(b["t"], b["dur"]) for b in v] for k, v in blocks_full.items()}

    def tiles_clean():
        eq("tiling gaps", til.get("gaps"), 0)
        eq("tiling overlaps", til.get("overlaps"), 0)
        eq("tiling overruns", til.get("overrun_objects"), 0)
        eq("ends at frames", til.get("ends_at_frames"), True)

    if name == "C01-ramps":
        eq("loss.ramp", loss.get("ramp"), 4)
        tiles_clean()
    elif name in ("C02-gain", "R02-gain-tagged"):
        # R1: gains on active objects are written the way Dolby writes them. The
        # audit ledger compares gain values wherever the ADM carries one
        # (compare_events.inexpressible_diff), so loss.gain == 0 is the toolkit's
        # own confirmation that all five gains agree.
        eq("loss.gain", loss.get("gain", 0), 0)
        eq("loss.importance", loss.get("importance", 0), 0)
        eq("matched", cl.get("matched"), 3)
        # The mid-stream gain change is filed under inexpressible_change because
        # that class is decided from the DAMF side (changed fields within the
        # profile's inexpressible set); its block is checked below.
        eq("gain-only change (DAMF-side class)", cl.get("inexpressible_change"), 1)
        # The muted object: Dolby encodes an active -inf object as <gain>0.0</gain>
        # alone (audit S2); the toolkit's normaliser derives "inactive" from a bare
        # zero gain ("inferred-gain0"), so it reports one value_mismatch on
        # `active` for that block. Exactly that one item is accepted.
        defects = cmp_.get("defects", [])
        eq("defect count", len(defects), 1)
        if defects:
            eq("defect class", defects[0].get("cls"), "value_mismatch")
            eq("defect object", defects[0].get("obj"), 3)
            eq("defect fields", defects[0].get("fields"), ["active"])
        b3 = blocks_full.get("3") or [{}]
        eq("muted object gain present", (b3[0].get("gain") or {}).get("present"), True)
        eq("muted object gain is -inf", (b3[0].get("gain") or {}).get("minus_inf"), True)
        eq("muted object importance absent", (b3[0].get("imp") or {}).get("present"), False)
        for g in ("<gain>0.5011872053</gain>", "<gain>1.4125375748</gain>", "<gain>0.2511886358</gain>", "<gain>0.0000000000</gain>"):
            has("gain text", text, g)
        lacks("muted active object is not the inactive literal", text, "<gain>0.0</gain>")
        b4 = blocks_full.get("4") or []
        eq("gain-change block count", len(b4), 2)
        if len(b4) == 2:
            eq("gain-change block time", b4[1].get("t"), 48000)
            eq("gain-change block gain", (b4[1].get("gain") or {}).get("lin"), 0.2511886358)
            eq("first block carries no gain at 0 dB", (b4[0].get("gain") or {}).get("present"), False)
        # Table 11 read literally: the toolkit flags every block that carries a
        # gain without an importance. Dolby's own S2 output draws the same finding
        # (adm-dolby-reference.json, stimuli/S2-gain-size/{ct,dee}/adm/findings).
        eq("findings", sorted(set(findings)), ["profile-inactive-encoding"])
        eq("finding count (one per gain-bearing block)", len(findings), 4)
        tiles_clean()
    elif name == "C03-importance":
        eq("loss.importance", loss.get("importance"), 2)
    elif name == "C04-size3d":
        w = ((((e.get("adm_blocks") or {}).get("1") or [{}])[0]).get("size") or {}).get("w")
        eq("width written", round(w or -1, 3), 0.2)
        eq("adm size-axes-collapsed", al.get("size-axes-collapsed"), 1)
        eq("damf size-axes-collapsed", dl.get("size-axes-collapsed"), 1)
    elif name == "C05-bed-events":
        eq("bed_changes_lost", cmp_.get("bed_changes_lost"), 2)
        eq("bed-gain-dropped", al.get("bed-gain-dropped"), 1)
        eq("bed-event-dropped", al.get("bed-event-dropped"), 2)
    elif name == "C06-late-first":
        eq("synthetic_block0", cl.get("synthetic_block0"), 2)
        eq("absorbed_by_synthetic", cl.get("absorbed_by_synthetic", 0), 0)
        eq("matched", cl.get("matched"), 2)
        eq("late-first-event-held", al.get("late-first-event-held"), 2)
        tiles_clean()
    elif name == "C07-beyond-end":
        tiles_clean()
        eq("last_end", til.get("last_end"), 96000)
        eq("beyond_end", cl.get("beyond_end"), 2)
        eq("event-beyond-end-dropped", al.get("event-beyond-end-dropped"), 2)
        dolby = ct_blocks_from_audit(repo, name)
        if dolby:
            eq("blocks equal Dolby's (object 1)", blocks.get("1"), dolby.get("1"))
            eq("blocks equal Dolby's (object 2)", blocks.get("2"), dolby.get("2"))
    elif name == "C08-same-pos":
        eq("superseded_same_pos", cl.get("superseded_same_pos"), 1)
        eq("same-position-superseded", al.get("same-position-superseded"), 1)
    elif name == "C09-out-of-order":
        tiles_clean()
        eq("defects", len(cmp_.get("defects", [])), 0)
        eq("matched", cl.get("matched"), 4)
        eq("unsorted objects", til.get("unsorted_objects"), 0)
        eq("damf out-of-order-written-as-is", dl.get("out-of-order-written-as-is"), 1)
        dolby = ct_blocks_from_audit(repo, name)
        if dolby:
            eq("blocks equal Dolby's (object 1)", blocks.get("1"), dolby.get("1"))
    elif name == "C10-zero-objects":
        eq("channels", (summ.get("adm") or {}).get("channels"), 10)
        eq("findings", findings, [])
    elif name == "C11-no-bed":
        eq("profile-id findings", findings.count("profile-id"), 0)
        has("first object id", text, 'audioObjectID="AO_100b"')
    elif name == "C12-118-objects":
        eq("profile-id findings", findings.count("profile-id"), 0)
        eq("channels", (summ.get("adm") or {}).get("channels"), 119)
        eq("other findings", sorted(set(findings)), ["profile-bed-configuration"])
    elif name == "C13-isf":
        has("adm refused", adm_err, "intermediate-spatial-format")
        has("damf refused", damf_err, "intermediate-spatial-format")
    elif name == "C13b-isf-drop":
        eq("adm written", bool(summ.get("adm")), True)
        eq("isf-dropped (adm)", al.get("isf-dropped"), 4)
        eq("isf-dropped (damf)", dl.get("isf-dropped"), 4)
        eq("declared loss", ((summ.get("adm") or {}).get("losses") or {}).get("declared_loss"), True)
        tones = [round(t) for t in (e.get("adm_track_tones") or []) if t]
        eq("ISF tones absent", [t for t in tones if 300 <= t <= 360], [])
    elif name == "C14-bed-tfl":
        has("adm refused", adm_err, "not allowed in a Dolby Atmos master ADM bed")
    elif name == "C15-96k":
        has("adm refused", adm_err, "48 000 Hz")
        eq("damf written", bool(summ.get("damf")), True)
    elif name == "C15b-96k-allowed":
        eq("adm written", bool(summ.get("adm")), True)
        eq("non-profile-sample-rate", al.get("non-profile-sample-rate"), 1)
        eq("profile-sample-rate findings", findings.count("profile-sample-rate"), 2)
        has("scaled constant", text, 'interpolationLength="0.002604"')
    elif name == "C16-active-toggle":
        eq("matched", cl.get("matched"), 4)
    elif name == "C17-zones":
        eq("findings", findings, [])
    elif name == "C18-snap":
        has("channelLock", text, "<channelLock>1</channelLock>")
    elif name == "C19-oamd-extras":
        eq("distance-dropped (timeline)", tl.get("distance-dropped"), 2)
        eq("screen-reference-dropped (adm)", al.get("screen-reference-dropped"), 1)
    elif name == "C20-ramp-only":
        eq("inexpressible_change", cl.get("inexpressible_change"), 1)
        eq("trailing_popped", cl.get("trailing_popped"), 1)
        eq("ramp-replaced counted", (al.get("ramp-replaced") or 0) > 0, True)
    elif name == "C20b-real-ramps":
        # R12: every ramp is written; the toolkit finds no ramp loss and a zero
        # trajectory error. Its ledger still files the two ramp-only states under
        # inexpressible_change (a DAMF-side class), so the written interpolation
        # lengths are checked block by block here.
        eq("loss", loss, {})
        eq("matched", cl.get("matched"), 3)
        eq("ramp-only states (DAMF-side class)", cl.get("inexpressible_change"), 2)
        eq("trailing_popped", cl.get("trailing_popped", 0), 0)
        eq("defects", len(cmp_.get("defects", [])), 0)
        eq("interpolation lengths in samples", [b.get("interp") for b in blocks_full.get("1") or []], [0, 32, 1536, 32])
        eq("ramp-replaced not counted", al.get("ramp-replaced"), None)
        traj = cmp_.get("trajectory_max_e") or {}
        worst = max((max(v.values()) for v in traj.values() if v), default=None)
        eq("trajectory evaluated", bool(traj), True)
        eq("trajectory max_e", worst, 0.0)
        eq("findings", sorted(set(findings)), ["profile-interpolation-length"])
        eq("dbmd marked", e.get("non_profile_mark"), True)
        tiles_clean()
    elif name == "C21-bed-order":
        eq("tones", [round(t) if t else None for t in (e.get("adm_track_tones") or [])][3:6], [100, 600, 500])
    elif name == "C21b-bed-order-nbc":
        eq("tones", [round(t) if t else None for t in (e.get("adm_track_tones") or [])], [500, 600, 100, 440])
        eq("profile-id findings", findings.count("profile-id"), 0)
    elif name == "R01-119-objects":
        has("adm refused", adm_err, "at most 118 objects")
        eq("damf written", bool(summ.get("damf")), True)
    return f


def stage_harness(a) -> dict:
    import run_harness
    from admaudit import normalise
    from run_compare import compare
    only = set(a.only.split(",")) if a.only else None
    cdir = os.path.join(a.work, "harness", "cases")
    odir = os.path.join(a.work, "harness", "out")
    os.makedirs(cdir, exist_ok=True)
    report = {}
    failures = {}
    for name, c in remediation_cases().items():
        if only is not None and name not in only:
            continue
        spec = {k: v for k, v in c.items() if k not in ("why", "expect", "heavy")}
        cpath = os.path.join(cdir, f"{name}.json")
        dump(cpath, spec)
        out = os.path.join(odir, name)
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([a.harness, cpath, out], capture_output=True, text=True, errors="replace")
        entry = {"why": c["why"], "expect": c["expect"], "exit": p.returncode, "stderr": p.stderr.strip()[-800:]}
        sp = os.path.join(out, f"{name}.summary.json")
        summ = load(sp)
        entry["summary"] = summ
        adm = os.path.join(out, f"{name}.wav")
        base = os.path.join(out, name)
        if summ and "adm" in summ and os.path.isfile(base + ".atmos"):
            try:
                rep = compare(base, adm, 0.0, "effective", "full", 10.0, name)
                L = rep["ledger"]
                entry["compare"] = {
                    "classes": L["classes"], "defects": L["defects"][:10], "loss": L["loss"], "identity_ok": L["identity_ok"],
                    "tiling": L["tiling"], "bed_changes_lost": L["bed_changes_lost"],
                    "pcm": [{k: p_[k] for k in ("role", "label", "damf_track", "adm_track", "identical")} for p_ in rep["pcm"]["pairs"]],
                    "adm_findings": list(rep["adm"]["findings"]), "damf_findings": list(rep["damf"]["findings"]),
                    "trajectory_max_e": {k: v["max_e"] for k, v in rep["trajectory_loss"].items()},
                }
                sc = normalise.from_adm(adm)
                entry["adm_blocks"] = {str(o.ordinal): [{"t": ev.t, "dur": ev.dur, "pos": ev.pos, "gain": ev.gain, "imp": ev.importance, "size": ev.size,
                                                    "interp": ev.interp["len_samples"] if ev.interp else None, "active": ev.active} for ev in o.events] for o in sc.objects}
                entry["axml"] = axml_text(adm)
                entry["non_profile_mark"] = file_bytes_contain(adm, b"non-profile: real interpolation lengths")
                if name.startswith(("C21", "C13")):
                    entry["adm_track_tones"] = run_harness.track_tones(adm)
            except Exception as ex:  # noqa: BLE001
                entry["compare_error"] = f"{type(ex).__name__}: {ex}"
        fails = expectations(a.repo, name, entry)
        entry["expectation_failures"] = fails
        if fails:
            failures[name] = fails
        entry.pop("axml", None)  # the file is reproducible; keep the record small
        report[name] = entry
        print(f"{name:20} exit {p.returncode} {'OK' if not fails else 'FAIL ' + '; '.join(fails)}", flush=True)
    dump(os.path.join(a.work, "harness", "remediation-harness.json"), {"cases": report, "failures": failures})
    return failures


# --------------------------------------------------------------------------- regression stage

# The one audited output the remediation is allowed to change, and why. Any
# other difference fails the stage.
EXPECTED_DIFFERENCES = {
    "work/pi-head50m/nbc/pi-head50m.adm.run.json": {
        "fix": "R6",
        "why": "with --no-bed-conform the objects are numbered from AO_100b, as Dolby numbers them "
               "(audit stimulus S11); the audited file numbered them from AO_1002",
        "audited_compare": "work/pi-head50m/nbc/compare.json",
    },
}


def dolby_s11_object_ids(repo: str) -> list:
    """The audioObjectIDs Dolby's Conversion Tool gave the same --no-bed-conform programme."""
    import re
    ref = load(os.path.join(repo, "docs", "audit", "evidence", "adm", "adm-dolby-reference.json")) or {}
    s11 = next((v for k, v in (ref.get("stimuli") or {}).items() if k.startswith("S11")), {})
    return sorted(set(re.findall(r"AO_[0-9a-f]{4}", json.dumps((s11.get("ct") or {}).get("adm") or {}))))


def object_ids(adm: str) -> list:
    import re
    return sorted(set(re.findall(r'audioObjectID="(AO_[0-9a-f]+)"', axml_text(adm))))


def verify_expected_difference(a, inv_files: dict, path: str, new_base: str) -> dict:
    """An output that may differ from the audited bytes must differ only as the fix says.

    The audited DAMF beside it is byte-identical, so the toolkit's own comparison
    of the new ADM against that DAMF must give the ledger the audit recorded for
    the audited ADM: the same classes, the same tiling, no defect, every track
    identical; and the object numbering must be Dolby's.
    """
    from run_compare import compare
    spec = EXPECTED_DIFFERENCES[path]
    audited = (inv_files.get(spec["audited_compare"]) or {}).get("content") or {}
    aL = audited.get("ledger") or {}
    rep_ = compare(new_base, new_base + ".wav", 0.0, "effective", "full", 10.0, "expected-difference")
    L = rep_["ledger"]
    pairs = rep_["pcm"]["pairs"]
    ids = object_ids(new_base + ".wav")
    dolby_ids = dolby_s11_object_ids(a.repo)
    out = {
        "fix": spec["fix"], "why": spec["why"],
        "pcm_pairs": len(pairs), "pcm_all_identical": bool(pairs) and all(p_["identical"] for p_ in pairs),
        "defects": len(L["defects"]), "classes": L["classes"], "audited_classes": aL.get("classes"),
        "classes_equal_audited": L["classes"] == aL.get("classes"),
        "tiling_equal_audited": L["tiling"] == aL.get("tiling"),
        "adm_findings": list(rep_["adm"]["findings"]),
        "object_ids": ids, "dolby_s11_object_ids": dolby_ids,
        "object_ids_equal_dolby": [i for i in ids if int(i[3:], 16) >= 0x100B] == dolby_ids,
    }
    # A non-conformed LFE-only bed is not a table-16 configuration; the profile
    # checker says so for any --no-bed-conform file and that is inherent to the
    # option, not to the fix. No other finding is allowed, and no profile-id one.
    kinds = {f["kind"] for f in out["adm_findings"]}
    out["findings_inherent_to_no_bed_conform"] = kinds <= {"profile-bed-configuration"}
    out["ok"] = (out["pcm_all_identical"] and out["defects"] == 0 and out["classes_equal_audited"] and out["tiling_equal_audited"]
                 and out["object_ids_equal_dolby"] and out["findings_inherent_to_no_bed_conform"])
    return out


def audited_tool_version(inv: dict):
    """The oadec version the audited outputs name, read from a stored `.atmos` header."""
    for r in inv["files"]:
        text = r.get("text")
        if r["path"].endswith(".atmos") and isinstance(text, str) and "creationTool: oadec" in text:
            m = re.search(r"creationToolVersion:\s*(\S+)", text)
            if m:
                return m.group(1)
    return None


def version_patches(path: str, current: str, audited: str):
    """Byte offsets to rewrite so that an output written by `current` reads as if
    `audited` had written it: the `creationToolVersion` line of a `.atmos` header,
    or the tool string inside the `dbmd` chunk of an ADM file together with the
    checksum of the segment that carries it. None when the two versions cannot be
    exchanged byte for byte (different lengths, string not found)."""
    if len(current) != len(audited):
        return None
    if path.endswith(".atmos"):
        with open(path, "rb") as f:
            data = f.read()
        needle = f"creationToolVersion: {current}".encode()
        at = data.find(needle)
        if at < 0:
            return None
        start = at + len(needle) - len(current)
        return {start + i: b for i, b in enumerate(audited.encode())}
    if path.endswith(".wav"):
        size = os.path.getsize(path)
        with open(path, "rb") as f:
            f.seek(max(0, size - (1 << 20)))
            tail_off = f.tell()
            tail = f.read()
        at = tail.rfind(b"dbmd")
        if at < 0:
            return None
        payload_off = at + 8
        length = int.from_bytes(tail[at + 4:at + 8], "little")
        payload = tail[payload_off:payload_off + length]
        needle = b"oadec " + current.encode()
        pos = 4  # the four version bytes
        while pos + 3 <= len(payload) and payload[pos] != 0:
            seg_size = int.from_bytes(payload[pos + 1:pos + 3], "little")
            seg_payload = payload[pos + 3:pos + 3 + seg_size]
            hit = seg_payload.find(needle)
            if hit >= 0:
                patched = bytearray(seg_payload)
                patched[hit + 6:hit + 6 + len(current)] = audited.encode()
                checksum = (256 - ((sum(patched) + seg_size) & 0xFF)) & 0xFF
                base = tail_off + payload_off + pos + 3
                out = {base + hit + 6 + i: b for i, b in enumerate(audited.encode())}
                out[base + seg_size] = checksum
                return out
            pos += 3 + seg_size + 1
        return None
    return None


def sha256_file_patched(path: str, patches: dict) -> str:
    """sha256 of the file with the given byte offsets replaced."""
    h = hashlib.sha256()
    off = 0
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(8 << 20), b""):
            lo, hi = off, off + len(chunk)
            local = {k - lo: v for k, v in patches.items() if lo <= k < hi}
            if local:
                b = bytearray(chunk)
                for k, v in local.items():
                    b[k] = v
                chunk = bytes(b)
            h.update(chunk)
            off = hi
    return h.hexdigest()


def stage_regression(a) -> dict:
    """Every audited decode, replayed with the new binary and compared byte for byte.

    A version bump changes two things in the outputs and nothing else: the
    `creationToolVersion` line of a `.atmos` header, and the tool string (with
    that segment's checksum) inside an ADM file's `dbmd` chunk. An output counts
    as identical when its bytes match the audited hash as written, or after those
    version bytes are put back to the audited version; every other byte must
    match. Expected differences are verified only once every record has been
    decoded, because the verification reads the DAMF a sibling record writes.
    """
    inv = load(os.path.join(a.repo, "docs", "audit", "evidence", "adm", "adm-work-inventory.json"))
    inv_files = {r["path"]: r for r in inv["files"]}
    audited_version = audited_tool_version(inv)
    current_version = None
    try:
        current_version = subprocess.run([a.oadec, "--version"], capture_output=True, text=True).stdout.split()[1]
    except (OSError, IndexError):
        pass
    records = []
    for r in inv["files"]:
        if not r["path"].endswith(".run.json") or "content" not in r:
            continue
        c = r["content"]
        argv = (c.get("run") or {}).get("argv") or []
        if len(argv) < 2 or argv[1] != "decode":
            continue
        if r["path"].startswith("big/") and not a.with_full_film:
            continue
        records.append((r["path"], c))
    runs = {}
    for path, c in sorted(records):
        argv = list(c["run"]["argv"])
        argv[0] = a.oadec
        # the output base is the argument after -o; rebase it under the work directory
        i = argv.index("-o")
        old_base = argv[i + 1].replace("\\", "/")
        rel = old_base.split("/audit/adm/", 1)[1] if "/audit/adm/" in old_base else os.path.basename(old_base)
        new_base = os.path.join(a.work, "regress", rel)
        os.makedirs(os.path.dirname(new_base), exist_ok=True)
        argv[i + 1] = new_base
        env = {k: v for k, v in os.environ.items() if not k.startswith("OADEC_")}
        runs[path] = (argv, new_base, subprocess.run(argv, capture_output=True, text=True, errors="replace", env=env))
    results = []
    identical = 0
    modulo_version = 0
    for path, c in sorted(records):
        argv, new_base, p = runs[path]
        outs = []
        all_same = True
        for o in c.get("outputs", []):
            name = os.path.basename(o["path"])
            new_path = os.path.join(os.path.dirname(new_base), name)
            exists = os.path.isfile(new_path)
            digest = sha256_file(new_path) if exists else None
            same = exists and digest == o["sha256"]
            same_mod = False
            if exists and not same and audited_version and current_version and audited_version != current_version:
                patches = version_patches(new_path, current_version, audited_version)
                same_mod = patches is not None and sha256_file_patched(new_path, patches) == o["sha256"]
            outs.append({"file": name, "audited_sha256": o["sha256"], "audited_bytes": o["bytes"], "identical": same,
                         "identical_modulo_version": same_mod,
                         "bytes": os.path.getsize(new_path) if exists else None, "sha256": digest})
            all_same = all_same and (same or same_mod)
            modulo_version += int(same_mod)
        identical += all_same
        entry = {"record": path, "argv": argv[1:], "audited_exit": c["run"]["exit_code"], "exit": p.returncode,
                 "exit_equal": p.returncode == c["run"]["exit_code"], "outputs": outs, "all_identical": all_same,
                 "stderr_tail": p.stderr[-600:]}
        verdict = "identical" if all_same else "DIFFERENT"
        if all_same and any(o["identical_modulo_version"] for o in outs):
            verdict = "identical (version string only)"
        if not all_same and path in EXPECTED_DIFFERENCES:
            entry["expected_difference"] = verify_expected_difference(a, inv_files, path, new_base)
            verdict = "expected difference, verified" if entry["expected_difference"]["ok"] else "expected difference, NOT as specified"
        results.append(entry)
        print(f"{path:60} exit {p.returncode} (audited {c['run']['exit_code']}) {verdict}", flush=True)
    expected_ok = sum(1 for r in results if (r.get("expected_difference") or {}).get("ok"))
    unexplained = [r["record"] for r in results if not r["all_identical"] and not (r.get("expected_difference") or {}).get("ok")]
    summary = {"records": len(results), "identical": identical, "identical_outputs_modulo_version": modulo_version,
               "audited_tool_version": audited_version, "current_tool_version": current_version,
               "different": len(results) - identical,
               "expected_differences_verified": expected_ok, "unexplained_differences": unexplained,
               "exit_mismatches": sum(1 for r in results if not r["exit_equal"]), "binary_sha256": sha256_file(a.oadec), "results": results}
    dump(os.path.join(a.work, "regression", "byte-identity.json"), summary)
    if not unexplained and summary["exit_mismatches"] == 0:
        return {}
    return {"regression": [f"{len(unexplained)} unexplained differences {unexplained}, {summary['exit_mismatches']} exit codes differ"]}


# --------------------------------------------------------------------------- dolby stage

WANT_C02 = {"1": {0: -6.0}, "2": {0: 3.0}, "3": {0: "-inf"}, "4": {0: 0.0, 48000: -12.0}}
GAIN_TEXTS_C02 = ["0.0000000000", "0.2511886358", "0.5011872053", "1.4125375748"]  # as the Conversion Tool writes them


def damf_gains(base: str) -> dict:
    """Effective gain per object state of a DAMF, in dB rounded to 3 places."""
    from admaudit import normalise
    d = normalise.from_damf(base)
    return {str(o.ordinal): [(ev.t, "-inf" if ev.gain["minus_inf"] else round(ev.gain["db"], 3)) for ev in o.events] for o in d.objects}


def gains_cover(gains: dict, want: dict) -> list:
    """The wanted (object, time, gain) triples missing from a read-back."""
    missing = []
    for k, w in want.items():
        got = dict(gains.get(k, []))
        for t, g in w.items():
            if got.get(t) != g:
                missing.append((k, t, g, got.get(t)))
    return missing


def gain_texts(adm: str) -> list:
    import re
    return sorted(set(re.findall(r"<gain>([^<]+)</gain>", axml_text(adm))))


def stage_dolby(a) -> dict:
    from admaudit import dolby
    fails = {}
    hdir = os.path.join(a.work, "harness", "out")
    rep = {"tools": dolby.tools_present()}
    c02 = os.path.join(hdir, "C02-gain", "C02-gain.wav")
    c02_damf = os.path.join(hdir, "C02-gain", "C02-gain.atmos")
    # (a) the Conversion Tool reads oadec's new gains back into DAMF
    back = os.path.join(a.work, "dolby", "c02-oadec-adm-to-damf")
    r = dolby.conversion_tool(c02, back, "atmos", timeout=1800)
    gains = damf_gains(os.path.join(back, "output")) if os.path.isfile(os.path.join(back, "output.atmos")) else {}
    missing = gains_cover(gains, WANT_C02)
    rep["c02_readback"] = {"run": {k: r["run"][k] for k in ("exit_code", "seconds")}, "findings": r["findings"], "gains_db": gains, "missing": missing,
                           "oadec_gain_texts": gain_texts(c02)}
    if r["run"]["exit_code"] != 0 or missing:
        fails["c02_readback"] = [f"exit {r['run']['exit_code']}, missing {missing}"]
    if rep["c02_readback"]["oadec_gain_texts"] != GAIN_TEXTS_C02:
        fails["c02_gain_texts"] = [f"{rep['c02_readback']['oadec_gain_texts']}"]
    # (b) the control: Dolby's own ADM from the same DAMF, its gain strings, and its read-back
    ctdir = os.path.join(a.work, "dolby", "c02-damf-to-ct-adm")
    r2 = dolby.conversion_tool(c02_damf, ctdir, "wav", timeout=1800)
    ct_wav = os.path.join(ctdir, "output.wav")
    rep["c02_ct"] = {"run": {k: r2["run"][k] for k in ("exit_code", "seconds")}, "findings": r2["findings"]}
    if os.path.isfile(ct_wav):
        rep["c02_ct"]["gain_texts"] = gain_texts(ct_wav)
        rep["c02_ct"]["gain_texts_equal_oadec"] = rep["c02_ct"]["gain_texts"] == rep["c02_readback"]["oadec_gain_texts"]
        if not rep["c02_ct"]["gain_texts_equal_oadec"]:
            fails["c02_ct_gain_texts"] = [f"Dolby wrote {rep['c02_ct']['gain_texts']}"]
        back2 = os.path.join(a.work, "dolby", "c02-ct-adm-to-damf")
        r3 = dolby.conversion_tool(ct_wav, back2, "atmos", timeout=1800)
        g2 = damf_gains(os.path.join(back2, "output")) if os.path.isfile(os.path.join(back2, "output.atmos")) else {}
        rep["c02_ct"]["readback"] = {"run": {k: r3["run"][k] for k in ("exit_code", "seconds")}, "gains_db": g2, "missing": gains_cover(g2, WANT_C02),
                                     "same_as_oadec_readback": g2 == gains}
    else:
        fails["c02_ct"] = [f"the Conversion Tool did not write an ADM (exit {r2['run']['exit_code']})"]
    # (b2) the second Dolby converter: DEE 5.2.1 convert_atmos_mezz, same DAMF, its gain strings
    deedir = os.path.join(a.work, "dolby", "c02-damf-to-dee-adm")
    # DEE writes a file only when the output name carries the extension; without it, a directory.
    r4 = dolby.dee_convert(c02_damf, deedir, "c02-dee.wav", target="adm", temp_dir=os.path.join(a.work, "dolby", "dee-tmp"), timeout=1800)
    dee_wavs = [o["path"] for o in r4["outputs"] if o["path"].lower().endswith(".wav")]
    rep["c02_dee"] = {"run": {k: r4["run"][k] for k in ("exit_code", "seconds")}, "findings": r4["findings"][:6], "outputs": [os.path.basename(o["path"]) for o in r4["outputs"]]}
    if dee_wavs:
        rep["c02_dee"]["gain_texts"] = gain_texts(dee_wavs[0])
        rep["c02_dee"]["gain_texts_equal_oadec"] = rep["c02_dee"]["gain_texts"] == rep["c02_readback"]["oadec_gain_texts"]
    else:
        rep["c02_dee"]["gain_texts"] = None
    # (c) Dolby's validators on the tagged gain file and on the untagged one
    tagged = os.path.join(hdir, "R02-gain-tagged", "R02-gain-tagged.wav")
    rep["validators"] = {}
    for label, path in (("tagged", tagged), ("untagged", c02)):
        if not os.path.isfile(path):
            continue
        v = {"dbmd_creator_tagged": file_bytes_contain(path, b"Created using Dolby")}
        for which in ("5.7.2", "1.1"):
            x = dolby.atmos_info(path, validate=True, which=which)
            v[f"atmos_info_{which}"] = {"exit": x["run"]["exit_code"], "findings": x["findings"][:6]}
        b = dolby.bwf_info(path)
        v["bwf_info"] = {"exit": b["run"]["exit_code"], "findings": b["findings"][:6]}
        rep["validators"][label] = v
    t = rep["validators"].get("tagged") or {}
    if (t.get("atmos_info_5.7.2") or {}).get("exit") != 0:
        fails["validators"] = [f"atmos_info 5.7.2 --validate on the tagged gain file: {t.get('atmos_info_5.7.2')}"]
    # (d) object numbering with --no-bed-conform against Dolby's (audit stimulus S11)
    nbc = os.path.join(a.work, "regress", "work", "pi-head50m", "nbc", "pi-head50m.wav")
    if os.path.isfile(nbc):
        ids = [i for i in object_ids(nbc) if int(i[3:], 16) >= 0x100B]  # the audit record lists Dolby's objects, not its bed
        dolby_ids = dolby_s11_object_ids(a.repo)
        rep["nbc_object_ids"] = {"oadec": ids, "dolby_s11_ct": dolby_ids, "equal": ids == dolby_ids}
        if dolby_ids and ids != dolby_ids:
            fails["nbc_ids"] = [f"{ids} vs Dolby {dolby_ids}"]
    else:
        rep["nbc_object_ids"] = "skipped: the regression stage has not produced pi-head50m/nbc"
    dump(os.path.join(a.work, "dolby", "dolby.json"), rep)
    return fails


# --------------------------------------------------------------------------- render stage

def stage_render(a) -> dict:
    import run_render
    from run_compare import compare
    fails = {}
    rdir = os.path.join(a.work, "render")
    os.makedirs(rdir, exist_ok=True)
    rep = {"comparisons": {}}
    # (a) oadec's C02 (gain on active objects) against Dolby's conversion of the same DAMF
    hdir = os.path.join(a.work, "harness", "out")
    pair = {"oadec": os.path.join(hdir, "C02-gain", "C02-gain.wav"), "dolby-ct": os.path.join(a.work, "dolby", "c02-damf-to-ct-adm", "output.wav")}
    rep["c02_inputs_present"] = {k: os.path.isfile(v) for k, v in pair.items()}
    if all(os.path.isfile(p) for p in pair.values()):
        renders = {}
        for label, path in pair.items():
            compat = os.path.join(rdir, f"c02-{label}__ear.wav")
            run_render.ear_compatible(path, compat)
            for lay in ("0+5+0", "4+7+0"):
                out = os.path.join(rdir, f"c02-{label}__{lay.replace('+', '')}.wav")
                r = run_render.render(compat, lay, out)
                renders[(label, lay)] = out if r["ok"] else None
                rep.setdefault("renders", {})[f"c02/{label}/{lay}"] = {"exit": r["run"]["exit_code"], "seconds": r["run"]["seconds"]}
        for lay in ("0+5+0", "4+7+0"):
            ra, rb = renders.get(("oadec", lay)), renders.get(("dolby-ct", lay))
            if ra and rb:
                c = run_render.compare_renders(ra, rb)
                rep["comparisons"][f"c02 oadec vs dolby-ct @ {lay}"] = {k: c[k] for k in ("all_identical", "min_sdr_db", "max_abs_overall")}
                if not c["all_identical"]:
                    fails[f"c02 render {lay}"] = [f"not identical: min SDR {c['min_sdr_db']}"]
    # (b) the real-ramp mode against the audit's real-ramp variant of the default file
    clip = os.path.join(a.media, "clips", "pi-head50m.thd")
    default_adm = os.path.join(a.work, "regress", "work", "pi-head50m", "default", "pi-head50m.wav")
    damf_base = os.path.join(a.work, "regress", "work", "pi-head50m", "default", "pi-head50m")
    if os.path.isfile(clip) and os.path.isfile(default_adm) and os.path.isfile(damf_base + ".atmos"):
        real_base = os.path.join(rdir, "pi-head50m-real")
        env = {k: v for k, v in os.environ.items() if not k.startswith("OADEC_")}
        p = subprocess.run([a.oadec, "decode", clip, "--format", "adm", "--adm-interpolation", "real", "-o", real_base], capture_output=True, text=True, errors="replace", env=env)
        rep["real_mode_run"] = {"exit": p.returncode, "stderr_tail": p.stderr[-500:]}
        real_adm = real_base + ".wav"
        variant = os.path.join(rdir, "pi-head50m-audit-variant.wav")
        rep["audit_variant"] = run_render.real_ramp_variant(default_adm, damf_base, variant)
        # the toolkit's own ledger and trajectory on the real-mode file
        r = compare(damf_base, real_adm, 0.0, "effective", "none", 10.0, "real")
        L = r["ledger"]
        worst = max((max(v["max_e"].values()) for v in r["trajectory_loss"].values()), default=0.0)
        # ten-decimal seconds round a 32-sample ramp to 0.0006666667 s = 32.000002 samples,
        # which the evaluator sees as a few 1e-8 room units; float32 positions resolve ~6e-8.
        rep["real_mode_ledger"] = {"classes": L["classes"], "loss": L["loss"], "defects": len(L["defects"]), "trajectory_max_e_worst": worst,
                                   "trajectory_tolerance": 1e-6}
        if L["loss"].get("ramp", 0) != 0 or L["defects"] or worst > 1e-6:
            fails["real ledger"] = [f"loss.ramp {L['loss'].get('ramp')} defects {len(L['defects'])} trajectory {worst}"]
        renders = {}
        for label, path in (("real", real_adm), ("audit-variant", variant)):
            compat = os.path.join(rdir, f"{label}__ear.wav")
            run_render.ear_compatible(path, compat)
            for lay in ("0+2+0", "4+7+0"):
                out = os.path.join(rdir, f"{label}__{lay.replace('+', '')}.wav")
                rr = run_render.render(compat, lay, out)
                renders[(label, lay)] = out if rr["ok"] else None
                rep.setdefault("renders", {})[f"real/{label}/{lay}"] = {"exit": rr["run"]["exit_code"], "seconds": rr["run"]["seconds"]}
        for lay in ("0+2+0", "4+7+0"):
            ra, rb = renders.get(("real", lay)), renders.get(("audit-variant", lay))
            if ra and rb:
                c = run_render.compare_renders(ra, rb)
                rep["comparisons"][f"real vs audit-variant @ {lay}"] = {k: c[k] for k in ("all_identical", "min_sdr_db", "max_abs_overall")}
                if not c["all_identical"] and (c["min_sdr_db"] is None or c["min_sdr_db"] < 60.0):
                    fails[f"real render {lay}"] = [f"min SDR {c['min_sdr_db']}"]
    dump(os.path.join(a.work, "render", "render.json"), rep)
    for f in os.listdir(rdir):
        if f.endswith(".wav"):
            os.remove(os.path.join(rdir, f))
    return fails


# --------------------------------------------------------------------------- evidence stage

def stage_evidence(a) -> dict:
    from admaudit import evidence, provenance
    out = os.path.join(a.repo, "docs", "audit", "evidence", "adm-remediation")
    os.makedirs(out, exist_ok=True)
    gen = "docs/audit/adm-remediation/tools/run_remediation.py"
    h = load(os.path.join(a.work, "harness", "remediation-harness.json"))
    if h:
        ok = not h["failures"]
        evidence.write(out, "adm-remediation-harness", h, title="Writer-level cases C01-C21b and the remediation variants through the remediated writers, judged by the audit toolkit",
                       topics=["harness"], evidence_class="MEASURED", structural_result="PASS" if ok else "FAIL", semantic_result="PASS" if ok else "FAIL",
                       classification="every expectation met" if ok else f"{len(h['failures'])} cases failed their expectation",
                       method="run_remediation.py --stage harness (adm_harness + admaudit compare/normalise/profile)", inputs=[], generated_by=gen)
    r = load(os.path.join(a.work, "regression", "byte-identity.json"))
    if r:
        ok = not r["unexplained_differences"] and r["exit_mismatches"] == 0
        evidence.write(out, "adm-remediation-byte-identity", r, title="Every audited decode replayed with the remediated binary: output hashes against adm-work-inventory.json",
                       topics=["regression"], evidence_class="MEASURED", structural_result="PASS" if ok else "FAIL", semantic_result="PASS" if ok else "FAIL",
                       classification=f"{r['identical']} of {r['records']} runs byte-identical, {r['expected_differences_verified']} expected difference verified (R6 object numbering), "
                                      f"{len(r['unexplained_differences'])} unexplained, {r['exit_mismatches']} exit-code differences",
                       method="run_remediation.py --stage regression", inputs=[], generated_by=gen)
    d = load(os.path.join(a.work, "dolby", "dolby.json"))
    if d:
        evidence.write(out, "adm-remediation-dolby", d, title="Dolby Conversion Tool read-back of the new gains, Dolby validators on the tagged file, object numbering against Dolby's",
                       topics=["dolby"], evidence_class="REFERENCE", structural_result=None, semantic_result=None, classification="reference runs",
                       method="run_remediation.py --stage dolby", inputs=[], generated_by=gen)
    rr = load(os.path.join(a.work, "render", "render.json"))
    if rr:
        evidence.write(out, "adm-remediation-render", rr, title="EBU ADM Renderer: oadec's gain file against Dolby's; the real-ramp mode against the audit's real-ramp variant",
                       topics=["render"], evidence_class="REFERENCE", structural_result=None, semantic_result=None, classification="renderer equality is supporting evidence only",
                       method="run_remediation.py --stage render", inputs=[], generated_by=gen)
    from admaudit import dolby
    tools = {"conversion_tool": dolby.CONVERSION_TOOL, "atmos_info_5.7.2": dolby.ATMOS_INFO["5.7.2"], "atmos_info_1.1": dolby.ATMOS_INFO["1.1"], "bwf_info": dolby.BWF_INFO}
    prov = {"git": provenance.git_record(a.repo), "oadec": provenance.file_record(a.oadec) if os.path.isfile(a.oadec) else None,
            "harness": provenance.file_record(a.harness) if os.path.isfile(a.harness) else None, "python": provenance.env_record(),
            "tools": {k: provenance.file_record(v) for k, v in tools.items() if os.path.isfile(v)}}
    evidence.write(out, "00-provenance", prov, title="Provenance of the remediation validation runs", topics=["provenance"], evidence_class="MEASURED",
                   structural_result=None, semantic_result=None, classification="record", method="git, sha256, --version", inputs=[], generated_by=gen)
    m = evidence.manifest(out)
    print(f"wrote {len(m['files'])} evidence files to {out}")
    return {}


# --------------------------------------------------------------------------- main

def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--stage", default="all", choices=["harness", "regression", "dolby", "render", "evidence", "all"])
    ap.add_argument("--harness", default=None)
    ap.add_argument("--oadec", default=None)
    ap.add_argument("--media", default="E:/oadec-work")
    ap.add_argument("--only", default=None)
    ap.add_argument("--no-dolby", action="store_true")
    ap.add_argument("--no-ear", action="store_true")
    ap.add_argument("--with-full-film", action="store_true")
    a = ap.parse_args()
    a.repo = os.path.abspath(a.repo)
    sys.path.insert(0, tools_dir(a.repo))
    exe = ".exe" if os.name == "nt" else ""
    a.harness = a.harness or os.path.join(a.repo, "target", "harness", "release", f"adm_harness{exe}")
    a.oadec = a.oadec or os.path.join(a.repo, "target", "release", f"oadec{exe}")
    os.makedirs(a.work, exist_ok=True)
    stages = ["harness", "regression", "dolby", "render", "evidence"] if a.stage == "all" else [a.stage]
    if a.no_dolby:
        stages = [s for s in stages if s != "dolby"]
    if a.no_ear:
        stages = [s for s in stages if s != "render"]
    failures = {}
    for s in stages:
        print(f"== stage {s}", flush=True)
        failures.update(globals()[f"stage_{s}"](a))
    if failures:
        print("EXPECTATIONS NOT MET:")
        for k, v in failures.items():
            print(f"  {k}: {'; '.join(v)}")
        return 1
    print("all expectations met")
    return 0


if __name__ == "__main__":
    sys.exit(main())
