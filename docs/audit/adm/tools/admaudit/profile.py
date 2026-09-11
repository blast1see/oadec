"""Dolby Atmos Master ADM Profile v1.0 (22 July 2019) rule checks.

Transcribed from the profile document (``E:/oadec-work/specs/
dolby_atmos_master_adm_profile_v1.0.pdf``), tables 9 to 23 and section 2.5.
Every finding names the table it comes from so that a reader can check the
rule against the text.  These are *structural* rules: passing them says the
file is shaped as the profile asks, not that it carries the right scene.
"""
from __future__ import annotations

import re
from fractions import Fraction

from . import axml
from .riff import Finding

# Table 10: speakerLabel -> position (X, Y, Z)
SPEAKERS = {
    "RC_L": (-1.0, 1.0, 0.0), "RC_R": (1.0, 1.0, 0.0), "RC_C": (0.0, 1.0, 0.0), "RC_LFE": (-1.0, 1.0, -1.0),
    "RC_Lss": (-1.0, 0.0, 0.0), "RC_Rss": (1.0, 0.0, 0.0), "RC_Lrs": (-1.0, -1.0, 0.0), "RC_Rrs": (1.0, -1.0, 0.0),
    "RC_Lts": (-1.0, 0.0, 1.0), "RC_Rts": (1.0, 0.0, 1.0), "RC_Ls": (-1.0, -0.36397, 0.0), "RC_Rs": (1.0, -0.36397, 0.0),
}
# Table 8: label -> audioChannelFormatName suffix
SPEAKER_NAMES = {
    "RC_L": "RoomCentricLeft", "RC_R": "RoomCentricRight", "RC_C": "RoomCentricCenter", "RC_LFE": "RoomCentricLFE",
    "RC_Lss": "RoomCentricLeftSideSurround", "RC_Rss": "RoomCentricRightSideSurround",
    "RC_Lrs": "RoomCentricLeftRearSurround", "RC_Rrs": "RoomCentricRightRearSurround",
    "RC_Lts": "RoomCentricLeftTopSurround", "RC_Rts": "RoomCentricRightTopSurround",
    "RC_Ls": "RoomCentricLeftSurround", "RC_Rs": "RoomCentricRightSurround",
}
# Table 16: allowed channel configuration sets, in order
CONFIG_SETS = {
    "2.0": ["RC_L", "RC_R"],
    "3.0": ["RC_L", "RC_R", "RC_C"],
    "5.0": ["RC_L", "RC_R", "RC_C", "RC_Ls", "RC_Rs"],
    "5.1": ["RC_L", "RC_R", "RC_C", "RC_LFE", "RC_Ls", "RC_Rs"],
    "7.0": ["RC_L", "RC_R", "RC_C", "RC_Lss", "RC_Rss", "RC_Lrs", "RC_Rrs"],
    "7.1": ["RC_L", "RC_R", "RC_C", "RC_LFE", "RC_Lss", "RC_Rss", "RC_Lrs", "RC_Rrs"],
    "7.0.2": ["RC_L", "RC_R", "RC_C", "RC_Lss", "RC_Rss", "RC_Lrs", "RC_Rrs", "RC_Lts", "RC_Rts"],
    "7.1.2": ["RC_L", "RC_R", "RC_C", "RC_LFE", "RC_Lss", "RC_Rss", "RC_Lrs", "RC_Rrs", "RC_Lts", "RC_Rts"],
}
# Tables 12 and 13: zone name -> (minX, maxX, minY, maxY, minZ, maxZ)
ZONES = {
    "ZM1": ("-1", "1", "-1", "-0.41934", "-0.499", "0.499"),
    "ZM2L": ("-1", "-0.75806", "-0.41934", "0.83871", "-0.499", "0.499"),
    "ZM2R": ("0.75806", "1", "-0.41934", "0.83871", "-0.499", "0.499"),
    "ZM3L": ("-1", "-0.16129", "0.5", "1", "-0.499", "0.499"),
    "ZM3Lss": ("-1", "-0.51611", "-0.707", "0.49999", "-0.499", "0.499"),
    "ZM3R": ("0.16129", "1", "0.5", "1", "-0.499", "0.499"),
    "ZM3Rss": ("0.51611", "1", "-0.707", "0.49999", "-0.499", "0.499"),
    "ZM4": ("-1", "1", "-1", "0.83871", "-0.499", "0.499"),
    "ZM5": ("-1", "1", "0.5", "1", "-0.499", "0.499"),
    "ZB": ("-1", "1", "-1", "1", "-1", "-0.4995"),
    "ZT": ("-1", "1", "-1", "1", "0.4995", "1"),
}
ALLOWED_OBJECT_SUBELEMENTS = {"cartesian", "position", "gain", "importance", "width", "depth", "height", "diffuse", "channelLock", "jumpPosition", "zoneExclusion"}
FORBIDDEN_OBJECT_SUBELEMENTS = {"objectDivergence", "screenRef"}
INTERPOLATION_SAMPLES = 250
MAX_CHANNELS = 128
MAX_ELEMENTS = 123


def _f(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def check(doc: axml.AdmDoc, fs: int, channels: int, chna=None) -> list:
    out: list[Finding] = []
    F = lambda kind, msg: out.append(Finding(kind, msg))  # noqa: E731

    # ---- section 2.1 general limits
    if channels > MAX_CHANNELS:
        F("profile-channel-count", f"{channels} PCM channels > {MAX_CHANNELS} (general requirements)")
    for tag in ("audioContent", "audioObject", "audioPackFormat"):
        n = len(doc.elements.get(tag, []))
        if n > MAX_ELEMENTS:
            F("profile-element-count", f"{n} {tag} elements > {MAX_ELEMENTS} (table 1)")

    # ---- table 21/22 programme
    progs = doc.elements.get("audioProgramme", [])
    if len(progs) != 1:
        F("profile-id", f"{len(progs)} audioProgramme elements; the profile expects exactly one, APR_1001 (table 21)")
    p_start = p_end = None
    for p in progs:
        if p.id != "APR_1001":
            F("profile-id", f"audioProgrammeID {p.id} != APR_1001 (table 21)")
        if "start" in p.attrs and "end" in p.attrs:
            try:
                p_start = axml.timecode.decode(p.attrs["start"], fs).samples
                p_end = axml.timecode.decode(p.attrs["end"], fs).samples
            except ValueError as e:
                F("profile-timecode", f"audioProgramme {e}")
        else:
            F("profile-programme-span", "audioProgramme without start/end (table 21)")

    # ---- table 19/20 content
    for c in doc.elements.get("audioContent", []):
        if not re.fullmatch(r"ACO_[0-9a-fA-F]{4}", c.id):
            F("profile-id", f"audioContentID {c.id} (table 19)")
        dlg = [n for n in c.node if axml._local(n.tag) == "dialogue"]
        if len(dlg) != 1 or (dlg[0].text or "").strip() != "2" or dlg[0].get("mixedContentKind") != "0":
            F("profile-content-dialogue", f"{c.id}: dialogue subelement must be 2 with mixedContentKind 0 (table 20)")

    # ---- table 17/18 objects
    ao_type = {}
    for ao in doc.elements.get("audioObject", []):
        packs = [t for r, t in ao.refs if r == "audioPackFormatIDRef"]
        uids = [t for r, t in ao.refs if r == "audioTrackUIDRef"]
        ptype = None
        if packs and packs[0] in doc.by_id:
            ptype = axml._type_of(doc.by_id[packs[0]])
        ao_type[ao.id] = ptype
        m = re.fullmatch(r"AO_([0-9a-fA-F]{4})", ao.id)
        if not m:
            F("profile-id", f"audioObjectID {ao.id} (table 17)")
        else:
            v = int(m.group(1), 16)
            lo = 0x100B if ptype == "Objects" else 0x1001
            if not (lo <= v <= 0x1080):
                F("profile-id", f"audioObjectID {ao.id} outside [{lo:#06x},0x1080] for {ptype} (table 17)")
        if len(packs) != 1:
            F("profile-object-pack", f"{ao.id} references {len(packs)} packs (table 18)")
        if ptype == "Objects" and len(uids) != 1:
            F("profile-object-tracks", f"{ao.id} (Objects) references {len(uids)} track UIDs (table 18)")
        if ptype == "DirectSpeakers" and not 1 <= len(uids) <= 10:
            F("profile-object-tracks", f"{ao.id} (DirectSpeakers) references {len(uids)} track UIDs (table 18)")
        start = ao.attrs.get("start", ao.attrs.get("startTime"))
        if start != "00:00:00.00000":
            F("profile-object-start", f"{ao.id} start {start!r} != 00:00:00.00000 (table 17)")
        if "duration" in ao.attrs and p_start is not None and p_end is not None:
            try:
                d = axml.timecode.decode(ao.attrs["duration"], fs).samples
                if d != p_end - p_start:
                    F("profile-object-duration", f"{ao.id} duration {d} samples != programme {p_end - p_start} (table 17)")
            except ValueError as e:
                F("profile-timecode", f"{ao.id} {e}")
        elif "duration" not in ao.attrs:
            F("profile-object-duration", f"{ao.id} has no duration (table 17)")
        for bad in ("audioObjectIDRef", "audioComplementaryObjectIDRef"):
            if any(r == bad for r, _ in ao.refs):
                F("profile-forbidden-subelement", f"{ao.id} carries {bad} (table 18)")

    # ---- table 14/15/16 packs
    for ap in doc.elements.get("audioPackFormat", []):
        t = axml._type_of(ap)
        if not re.fullmatch(r"AP_000[13][0-9a-fA-F]{4}", ap.id):
            F("profile-id", f"audioPackFormatID {ap.id} (table 14)")
        refs = [x for r, x in ap.refs if r == "audioChannelFormatIDRef"]
        if any(r == "audioPackFormatIDRef" for r, _ in ap.refs):
            F("profile-forbidden-subelement", f"{ap.id} nests audioPackFormatIDRef (table 15)")
        if t == "Objects" and len(refs) != 1:
            F("profile-pack-channels", f"{ap.id} (Objects) references {len(refs)} channel formats (table 15)")
        if t == "DirectSpeakers":
            if not 1 <= len(refs) <= 10:
                F("profile-pack-channels", f"{ap.id} (DirectSpeakers) references {len(refs)} channel formats (table 15)")
            labels = []
            for cf in refs:
                try:
                    labels.append(doc.speaker_block(cf).speaker_label)
                except (KeyError, ValueError):
                    labels.append(None)
            if labels not in CONFIG_SETS.values():
                F("profile-bed-configuration", f"{ap.id} channel set {labels} is not one of the table-16 configuration sets")

    # ---- tables 6-13 channel formats and blocks
    interp_s = Fraction(INTERPOLATION_SAMPLES, fs)
    for ac in doc.elements.get("audioChannelFormat", []):
        t = axml._type_of(ac)
        if t == "DirectSpeakers" and not re.fullmatch(r"AC_0001[0-9a-fA-F]{4}", ac.id):
            F("profile-id", f"audioChannelFormatID {ac.id} for DirectSpeakers (table 6)")
        if t == "Objects" and not re.fullmatch(r"AC_0003[0-9a-fA-F]{4}", ac.id):
            F("profile-id", f"audioChannelFormatID {ac.id} for Objects (table 6)")
        blocks = [b for b in ac.node if axml._local(b.tag) == "audioBlockFormat"]
        if t == "DirectSpeakers":
            if len(blocks) != 1:
                F("profile-speaker-blocks", f"{ac.id} has {len(blocks)} blocks; DirectSpeakers takes exactly one (table 7)")
            for b in blocks:
                sb = axml._speaker_block(b)
                if sb.rtime is not None or sb.duration is not None:
                    F("profile-speaker-block-timing", f"{ac.id} {sb.id} carries rtime/duration (table 9)")
                if not sb.id.endswith("_00000001"):
                    F("profile-id", f"{sb.id} DirectSpeakers block id must end in 00000001 (table 9)")
                if sb.speaker_label not in SPEAKERS:
                    F("profile-speaker-label", f"{ac.id} speakerLabel {sb.speaker_label!r} not in table 10")
                else:
                    if tuple(sb.pos) != SPEAKERS[sb.speaker_label]:
                        F("profile-speaker-position", f"{ac.id} {sb.speaker_label} at {sb.pos}, table 10 says {SPEAKERS[sb.speaker_label]}")
                    if ac.name != SPEAKER_NAMES[sb.speaker_label]:
                        F("profile-speaker-name", f"{ac.id} named {ac.name!r}, table 8 says {SPEAKER_NAMES[sb.speaker_label]!r}")
                if sb.cartesian is not True:
                    F("profile-cartesian", f"{ac.id} {sb.id} cartesian != 1 (table 10)")
            continue
        if t != "Objects":
            F("profile-type", f"{ac.id} typeDefinition {t!r}; only DirectSpeakers and Objects are allowed (table 6)")
            continue
        for n, b in enumerate(blocks):
            blk = axml._object_block(b, fs)
            where = f"{ac.id} {blk.id}"
            if blk.rtime is None or blk.duration is None:
                F("profile-block-timing", f"{where} lacks rtime/duration (table 9)")
            if blk.cartesian is not True:
                F("profile-cartesian", f"{where} cartesian != 1 (table 11)")
            if blk.jump is None:
                F("profile-jump-position", f"{where} has no jumpPosition (table 11)")
            elif blk.jump != 1:
                F("profile-jump-position", f"{where} jumpPosition {blk.jump} != 1 (table 11)")
            if blk.interp_len_raw is None:
                F("profile-interpolation-length", f"{where} has no interpolationLength (table 11)")
            else:
                want = Fraction(0) if n == 0 else interp_s
                if abs(blk.interp_len_seconds - want) > Fraction(1, 1_000_000):
                    F("profile-interpolation-length", f"{where} interpolationLength {blk.interp_len_raw} s; table 11 requires {'0' if n == 0 else f'{INTERPOLATION_SAMPLES} samples = {float(interp_s):.6f} s'}")
            g, imp = blk.gain, blk.importance
            if g is not None or imp is not None:
                if g is None or imp is None:
                    F("profile-inactive-encoding", f"{where} carries only one of gain/importance (table 11: both mark an inactive object)")
                elif g != 0.0 or imp != 0:
                    F("profile-gain-on-active", f"{where} gain {blk.gain_raw} importance {imp}: gain/importance are allowed only as 0.0/0 on an inactive object (table 11)")
            if blk.size is not None:
                if not all(blk.size_present):
                    F("profile-size-axes", f"{where} width/depth/height not all present (table 11)")
                elif not (blk.size[0] == blk.size[1] == blk.size[2]):
                    F("profile-size-axes", f"{where} width/depth/height {blk.size} differ (table 11 requires identical values)")
                elif not 0.0 <= blk.size[0] <= 1.0:
                    F("profile-size-range", f"{where} size {blk.size[0]} outside [0,1] (table 11)")
            if not (blk.x_present and blk.y_present):
                F("profile-position", f"{where} lacks X or Y (table 11)")
            for axis, v in zip("XYZ", blk.pos):
                if v is not None and not -1.0 <= v <= 1.0:
                    F("profile-position-range", f"{where} {axis}={v} outside [-1,1] (table 11)")
            if blk.polar:
                F("profile-position", f"{where} carries polar coordinates {sorted(blk.polar)} (table 11: cartesian only)")
            if blk.channel_lock_max_distance is not None:
                F("profile-forbidden-subelement", f"{where} channelLock maxDistance (table 11)")
            for c in b:
                tag = axml._local(c.tag)
                if tag in FORBIDDEN_OBJECT_SUBELEMENTS:
                    F("profile-forbidden-subelement", f"{where} <{tag}> (table 11: shall not be used)")
                elif tag not in ALLOWED_OBJECT_SUBELEMENTS:
                    F("profile-unknown-subelement", f"{where} <{tag}> is not listed in table 11")
                if tag == "position" and c.get("screenEdgeLock") is not None:
                    F("profile-forbidden-subelement", f"{where} position screenEdgeLock (table 11)")
                if tag == "diffuse" and (c.text or "").strip() not in ("0", "1"):
                    F("profile-diffuse", f"{where} diffuse {c.text!r} not 0/1 (table 11)")
            for name, rect in zip(blk.zones, blk.zone_rects):
                if name not in ZONES:
                    F("profile-zone-name", f"{where} zone {name!r} not in tables 12/13")
                    continue
                want = ZONES[name]
                if any(_f(a) is None or abs(_f(a) - float(w)) > 1e-9 for a, w in zip(rect, want)):
                    F("profile-zone-rectangle", f"{where} zone {name} rectangle {rect} != table {want}")
            basic = [z for z in blk.zones if z not in ("ZB", "ZT")]
            if basic and sorted(basic) not in [sorted(v) for k, v in {1: ["ZM1"], 2: ["ZM2L", "ZM2R"], 3: ["ZM3L", "ZM3Lss", "ZM3R", "ZM3Rss"], 4: ["ZM4"], 5: ["ZM5"]}.items()]:
                F("profile-zone-set", f"{where} zones {blk.zones} are not one basic zone set (section 2.5.2)")
            if ("ZB" in blk.zones) != ("ZT" in blk.zones):
                F("profile-zone-set", f"{where} elevation zones ZB/ZT must come together (table 13)")

    # ---- tables 4/5 stream formats, table 23 track UIDs
    for asf in doc.elements.get("audioStreamFormat", []):
        tags = {r for r, _ in asf.refs}
        if not {"audioChannelFormatIDRef", "audioPackFormatIDRef", "audioTrackFormatIDRef"} <= tags:
            F("profile-stream-refs", f"{asf.id} must reference channel, pack and track formats (table 5)")
        if asf.attrs.get("formatLabel") != "0001" or asf.attrs.get("formatDefinition") != "PCM":
            F("profile-stream-format", f"{asf.id} formatLabel/formatDefinition not 0001/PCM (table 4)")
    for atu in doc.elements.get("audioTrackUID", []):
        if atu.attrs.get("sampleRate") != "48000":
            F("profile-sample-rate", f"{atu.id} sampleRate {atu.attrs.get('sampleRate')!r} != 48000 (table 23)")
        if fs != 48000:
            F("profile-sample-rate", f"audio at {fs} Hz; the profile requires 48000 (table 23)")
            break
    if chna is not None and chna.num_tracks != channels:
        F("profile-chna", f"chna numTracks {chna.num_tracks} != PCM channels {channels}")
    # de-duplicate identical findings (the fs loop above may add one per UID otherwise)
    seen = set()
    uniq = []
    for f in out:
        key = (f.kind, f.message)
        if key not in seen:
            seen.add(key)
            uniq.append(f)
    return uniq
