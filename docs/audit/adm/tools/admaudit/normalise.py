"""One representation for the Atmos scene, whichever file carries it.

``from_adm`` reads an ADM BWF (via ``riff``/``axml``); ``from_damf`` reads a
DAMF set (via ``damf``/``caf``).  Both produce a ``Scene`` whose events keep
*absence* and *effective value* apart: every optional quantity is a mapping
with a ``present`` flag next to the value that a reader applying the BS.2076
defaults would use.  Time is the integer sample everywhere.

Zone tables come from the Dolby Atmos Master ADM Profile v1.0, tables 12 and
13 (SPEC), and the DAMF zone spellings from the files oadec and the Dolby
tools write; an unknown spelling is kept verbatim with ``index: None``.
"""
from __future__ import annotations

import dataclasses
import hashlib
import math
import os
from dataclasses import dataclass, field
from fractions import Fraction

from . import axml, caf, damf, riff

# Dolby Atmos Master ADM Profile v1.0, table 12 (basic zones) and table 13 (elevation).
ZONE_NAMES = {
    0: [],
    1: ["ZM1"],
    2: ["ZM2L", "ZM2R"],
    3: ["ZM3L", "ZM3Lss", "ZM3R", "ZM3Rss"],
    4: ["ZM4"],
    5: ["ZM5"],
}
ELEVATION_ZONES = ["ZB", "ZT"]
DAMF_ZONE_INDEX = {"all": 0, "no back": 1, "no sides": 2, "center back": 3, "screen only": 4, "surround only": 5}

# ITU-R BS.2076-3 audioBlockFormat (Objects) defaults applied when a subelement is absent.
DEFAULT_GAIN_LIN = 1.0
DEFAULT_IMPORTANCE = 10
DEFAULT_SIZE = 0.0


@dataclass
class Track:
    index: int
    role: str
    label: str | None
    element_id: int | None = None
    uid: str | None = None
    track_format: str | None = None
    channel_format: str | None = None
    pack: str | None = None
    audio_object: str | None = None
    problems: list = field(default_factory=list)


@dataclass
class ObjEvent:
    t: int
    dur: int | None
    end: int | None
    active: bool | None
    active_derivation: str
    pos: tuple
    z_present: bool
    gain: dict
    importance: dict
    size: dict
    interp: dict | None
    ramp: int | None
    zones: dict
    snap: bool | None
    screen_factor: float | None
    depth_factor: float | None
    changed: set | None
    extra: list
    ref: str
    t_raw: str | None = None
    dur_raw: str | None = None
    t_units: int | None = None
    dur_units: int | None = None
    t_exact: bool | None = None
    dur_exact: bool | None = None
    present: set | None = None


@dataclass
class BedEvent:
    t: int
    active: bool | None
    gain: dict
    importance: dict
    ramp: int | None
    trim_bypass: bool | None
    changed: set | None
    ref: str


@dataclass
class Bed:
    element_id: int | None
    label: str | None
    track: int
    pos: tuple | None
    events: list
    channel_format: str | None = None


@dataclass
class Obj:
    element_id: int | None
    ordinal: int
    name: str | None
    track: int | None
    channel_format: str | None
    events: list
    uid: str | None = None
    audio_object: str | None = None


@dataclass
class Scene:
    source: dict
    tracks: list
    beds: list
    objects: list
    flags: dict = field(default_factory=dict)
    findings: list = field(default_factory=list)

    def to_json(self) -> dict:
        return _plain(dataclasses.asdict(self))

    def object_by_ordinal(self, k: int) -> Obj | None:
        for o in self.objects:
            if o.ordinal == k:
                return o
        return None


def _plain(v):
    if isinstance(v, dict):
        return {str(k): _plain(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_plain(x) for x in v]
    if isinstance(v, set):
        return sorted(_plain(x) for x in v)
    if isinstance(v, Fraction):
        return str(v)
    if isinstance(v, float) and (math.isnan(v) or math.isinf(v)):
        return str(v)
    if dataclasses.is_dataclass(v):
        return _plain(dataclasses.asdict(v))
    return v


def sha256_file(path: str, block: int = 1 << 24) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(block)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def gain_from_db(db) -> dict:
    if db is None:
        return {"present": False, "lin": DEFAULT_GAIN_LIN, "db": 0.0, "minus_inf": False}
    if isinstance(db, str):
        if db.lstrip("-").lower().lstrip(".") in ("inf",) and db.startswith("-"):
            return {"present": True, "lin": 0.0, "db": None, "minus_inf": True}
        raise ValueError(f"unrecognised gain {db!r}")
    return {"present": True, "lin": 10 ** (float(db) / 20), "db": db, "minus_inf": False}


def gain_from_adm(lin, unit) -> dict:
    if lin is None:
        return {"present": False, "lin": DEFAULT_GAIN_LIN, "db": 0.0, "minus_inf": False}
    if unit is not None and unit.lower() == "db":
        return {"present": True, "lin": 10 ** (float(lin) / 20), "db": float(lin), "minus_inf": False}
    lin = float(lin)
    if lin <= 0.0:
        return {"present": True, "lin": lin, "db": None, "minus_inf": True}
    return {"present": True, "lin": lin, "db": 20 * math.log10(lin), "minus_inf": False}


def zones_from_names(names: list) -> dict:
    basic = [n for n in names if n not in ELEVATION_ZONES]
    elevation = not all(z in names for z in ELEVATION_ZONES)
    index = None
    for k, v in ZONE_NAMES.items():
        if sorted(v) == sorted(basic):
            index = k
            break
    return {"names": list(names), "index": index, "elevation": elevation}


def zones_from_damf(name, elevation: bool) -> dict:
    index = DAMF_ZONE_INDEX.get(name) if isinstance(name, str) else None
    names = list(ZONE_NAMES.get(index, [])) if index is not None else [f"damf:{name}"]
    if not elevation:
        names = names + list(ELEVATION_ZONES)
    return {"names": names, "index": index, "elevation": bool(elevation)}


def _dbmd_strings(payload: bytes) -> tuple:
    """(creator, tool) from segment 9 of the Dolby audio metadata chunk, if present."""
    pos = 4
    while pos + 3 <= len(payload) and payload[pos] != 0:
        sid = payload[pos]
        size = int.from_bytes(payload[pos + 1 : pos + 3], "little")
        body = payload[pos + 3 : pos + 3 + size]
        if sid == 9 and len(body) >= 96:
            creator = body[0:32].split(b"\x00", 1)[0].decode("latin-1")
            tool = body[32:96].split(b"\x00", 1)[0].decode("latin-1")
            return creator, tool
        pos += 4 + size
    return None, None


_ADM_COMPARE_KEYS = ("active", "pos", "gain", "importance", "size", "zones", "snap")


def _adm_changed(prev: ObjEvent | None, cur: ObjEvent) -> set:
    if prev is None:
        return set(_ADM_COMPARE_KEYS)
    out = set()
    for k in _ADM_COMPARE_KEYS:
        if getattr(prev, k) != getattr(cur, k):
            out.add(k)
    return out


def from_adm(path: str, with_sha256: bool = False) -> Scene:
    c = riff.scan(path)
    findings = list(riff.verify(path, c))
    ax = c.chunk("axml")
    ch = c.chunk("chna")
    if ax is None:
        raise ValueError("no axml chunk")
    doc = axml.parse(riff.chunk_bytes(path, ax))
    findings += axml.check_references(doc)
    channels = c.fmt.channels if c.fmt else 0
    fs = c.fmt.sample_rate if c.fmt else 48000
    chna = axml.parse_chna(riff.chunk_bytes(path, ch)) if ch is not None else None
    if chna is None:
        findings.append(riff.Finding("chna-missing", "no chna chunk"))
        links = []
    else:
        findings += axml.check_chna(doc, chna, channels)
        links = axml.track_chain(doc, chna, channels)
    creator = tool = None
    db = c.chunk("dbmd")
    if db is not None:
        creator, tool = _dbmd_strings(riff.chunk_bytes(path, db))

    tracks: list[Track] = []
    by_track_index = {}
    for link in links:
        role = "unknown"
        label = None
        t = doc.channel_format_type(link.channel_format) if link.channel_format else None
        if t == "DirectSpeakers":
            role = "bed"
            try:
                label = doc.speaker_block(link.channel_format).speaker_label
            except (KeyError, ValueError):
                label = None
        elif t == "Objects":
            role = "object"
            el = doc.element(link.channel_format)
            label = el.name if el else None
        tr = Track(link.track_index - 1, role, label, None, link.uid, link.track_format, link.channel_format, link.pack_from_chna, link.audio_object, list(link.problems))
        tracks.append(tr)
        by_track_index[tr.index] = tr

    beds: list[Bed] = []
    for tr in tracks:
        if tr.role == "bed":
            sb = doc.speaker_block(tr.channel_format)
            beds.append(Bed(None, sb.speaker_label, tr.index, sb.pos, [], tr.channel_format))

    objects: list[Obj] = []
    ordinal = 0
    time_notation = None
    uid_track = {tr.uid: tr for tr in tracks}
    for ao in doc.elements.get("audioObject", []):
        packs = [t for r, t in ao.refs if r == "audioPackFormatIDRef"]
        uids = [t for r, t in ao.refs if r == "audioTrackUIDRef"]
        if not packs:
            continue
        pel = doc.element(packs[0])
        ptype = (pel.attrs.get("typeDefinition") if pel else None) or axml.TYPE_LABELS.get(pel.attrs.get("typeLabel", "")) if pel else None
        if ptype != "Objects":
            continue
        ordinal += 1
        tr = uid_track.get(uids[0]) if uids else None
        cf = tr.channel_format if tr else None
        if cf is None and pel is not None:
            refs = [t for r, t in pel.refs if r == "audioChannelFormatIDRef"]
            cf = refs[0] if refs else None
        events: list[ObjEvent] = []
        prev = None
        notation = None
        if cf is not None:
            for b in doc.object_blocks(cf, fs):
                gain = gain_from_adm(b.gain, b.gain_unit)
                imp = {"present": b.importance is not None, "scale": "adm-0-10", "value": b.importance if b.importance is not None else DEFAULT_IMPORTANCE}
                if b.gain is not None and gain["lin"] == 0.0 and b.importance == 0:
                    active, deriv = False, "inferred-gain0-importance0"
                elif b.gain is not None and gain["lin"] == 0.0:
                    active, deriv = False, "inferred-gain0"
                else:
                    active, deriv = True, "inferred-default"
                if b.size is not None:
                    w, d, h = (x if x is not None else DEFAULT_SIZE for x in b.size)
                    size = {"present": True, "w": w, "d": d, "h": h, "uniform": (w == d == h)}
                else:
                    size = {"present": False, "w": DEFAULT_SIZE, "d": DEFAULT_SIZE, "h": DEFAULT_SIZE, "uniform": None}
                interp = {
                    "jump": b.jump,
                    "len_present": b.interp_len_raw is not None,
                    "len_s": b.interp_len_seconds,
                    "len_samples": b.interp_len_samples,
                    "len_exact": b.interp_len_exact,
                }
                if notation is None and b.rtime is not None:
                    notation = "bs2076-2-samples" if "S" in b.rtime else "bs2076-1-decimal"
                ev = ObjEvent(
                    t=b.t, dur=b.dur, end=b.end, active=active, active_derivation=deriv,
                    pos=b.pos, z_present=b.z_present, gain=gain, importance=imp, size=size,
                    interp=interp, ramp=None, zones=zones_from_names(b.zones), snap=b.channel_lock,
                    screen_factor=None, depth_factor=None, changed=None, extra=list(b.extra), ref=b.id,
                    t_raw=b.rtime, dur_raw=b.duration, t_units=b.t_units, dur_units=b.dur_units,
                    t_exact=b.t_exact, dur_exact=b.dur_exact,
                    present={k for k, v in (("gain", b.gain is not None), ("importance", b.importance is not None), ("size", b.size is not None), ("z", b.z_present), ("interp_len", b.interp_len_raw is not None)) if v},
                )
                ev.changed = _adm_changed(prev, ev)
                events.append(ev)
                prev = ev
        objects.append(Obj(None, ordinal, ao.name, tr.index if tr else None, cf, events, uids[0] if uids else None, ao.id))
        if notation and time_notation is None:
            time_notation = notation

    start, end = doc.programme_span(fs)
    source = {
        "kind": "adm-bwf", "path": os.path.abspath(path), "bytes": c.file_size,
        "sha256": sha256_file(path) if with_sha256 else None,
        "fourcc": c.fourcc, "chunks": [(k.id, k.size) for k in c.chunks],
        "sample_rate": fs, "frames": c.frames, "channels": channels,
        "bits": c.fmt.bits_per_sample if c.fmt else None, "format_tag": c.fmt.format_tag if c.fmt else None,
        "rf64": c.fourcc != "RIFF", "ds64": dataclasses.asdict(c.ds64) if c.ds64 else None,
        "time_notation": time_notation, "programme_start": start, "programme_end": end,
        "dbmd_creator": creator, "dbmd_tool": tool, "xml_namespace": doc.ns,
        "element_counts": doc.counts(),
    }
    return Scene(source, tracks, beds, objects, {}, findings)


def _damf_event(state: damf.State) -> ObjEvent:
    v = state.values
    pos = v.get("pos")
    pos = tuple(float(x) for x in pos) if isinstance(pos, tuple) else (None, None, None)
    elev = v.get("elevation", True)
    size_v = v.get("size")
    return ObjEvent(
        t=state.t, dur=None, end=None,
        active=v.get("active"), active_derivation="explicit" if "active" in v else "absent",
        pos=pos, z_present=True,
        gain=gain_from_db(v.get("gain")),
        importance={"present": "importance" in v, "scale": "damf-0-1", "value": v.get("importance")},
        size={"present": size_v is not None, "w": size_v if size_v is not None else DEFAULT_SIZE, "d": size_v if size_v is not None else DEFAULT_SIZE, "h": size_v if size_v is not None else DEFAULT_SIZE, "uniform": True if size_v is not None else None},
        interp=None, ramp=v.get("rampLength"),
        zones=zones_from_damf(v.get("zones", "all"), bool(elev) if isinstance(elev, bool) else True),
        snap=v.get("snap"), screen_factor=v.get("screenFactor"), depth_factor=v.get("depthFactor"),
        changed=set(state.changed), extra=[], ref=f"line {state.line_no}", present=set(state.present),
    )


def from_damf(base: str, with_sha256: bool = False) -> Scene:
    atmos_path = base + ".atmos"
    header = damf.read_atmos(atmos_path)
    d = os.path.dirname(atmos_path)
    md_path = os.path.join(d, header.metadata_file) if header.metadata_file else base + ".atmos.metadata"
    au_path = os.path.join(d, header.audio_file) if header.audio_file else base + ".atmos.audio"
    fs, raw = damf.read_metadata(md_path)
    states, findings = damf.reconstruct(raw, header)
    info = caf.read_header(au_path) if os.path.exists(au_path) else None
    order = header.element_order()
    if info is not None and info.channels != len(order):
        findings.append(riff.Finding("damf-channel-count", f"header lists {len(order)} elements, audio has {info.channels} channels"))
    if info is not None and info.sample_rate != fs:
        findings.append(riff.Finding("damf-rate", f"metadata sampleRate {fs} != audio {info.sample_rate}"))
    tracks = []
    for i, eid in enumerate(order):
        kind = header.kind(eid)
        label = next((n for n, x in header.bed_channels if x == eid), None) if kind == "bed" else f"object {eid}"
        tracks.append(Track(i, kind or "unknown", label, eid))
    beds = []
    for name, eid in header.bed_channels:
        evs = []
        for s in states.get(eid, []):
            v = s.values
            evs.append(BedEvent(s.t, v.get("active"), gain_from_db(v.get("gain")), {"present": "importance" in v, "scale": "damf-0-1", "value": v.get("importance")}, v.get("rampLength"), v.get("trimBypass"), set(s.changed), f"line {s.line_no}"))
        beds.append(Bed(eid, name, order.index(eid), None, evs))
    objects = []
    for k, eid in enumerate(header.object_ids, 1):
        evs = [_damf_event(s) for s in states.get(eid, [])]
        for a, b in zip(evs, evs[1:]):
            a.dur = b.t - a.t
            a.end = b.t
        if evs and info is not None:
            evs[-1].dur = max(info.frames - evs[-1].t, 0)
            evs[-1].end = info.frames
        objects.append(Obj(eid, k, None, order.index(eid), None, evs))
    source = {
        "kind": "damf", "path": os.path.abspath(base), "metadata_path": os.path.abspath(md_path), "audio_path": os.path.abspath(au_path),
        "sha256": {"metadata": sha256_file(md_path), "audio": sha256_file(au_path) if info else None} if with_sha256 else None,
        "sample_rate": fs, "frames": info.frames if info else None, "channels": info.channels if info else None,
        "bits": info.bits if info else None, "fps": header.fps, "version": header.version, "offset": header.offset,
        "creation_tool": header.creation_tool, "creation_tool_version": header.creation_tool_version,
        "raw_events": len(raw), "element_order": order,
    }
    return Scene(source, tracks, beds, objects, {}, findings)
