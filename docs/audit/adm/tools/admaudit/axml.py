"""ADM (``axml`` chunk) parser, reference graph and ``chna`` chunk reader.

Written for the audit from ITU-R BS.2076 and EBU Tech 3285 s7.  The parser is
namespace-agnostic (ebuCore 2014/2016/2017 all occur in the wild) and keeps
every child element it does not model in ``Block.extra`` so that nothing an
implementation writes can hide.
"""
from __future__ import annotations

import struct
import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from fractions import Fraction

from . import timecode
from .riff import Finding

ID_ATTR = {
    "audioProgramme": "audioProgrammeID",
    "audioContent": "audioContentID",
    "audioObject": "audioObjectID",
    "audioPackFormat": "audioPackFormatID",
    "audioChannelFormat": "audioChannelFormatID",
    "audioStreamFormat": "audioStreamFormatID",
    "audioTrackFormat": "audioTrackFormatID",
    "audioTrackUID": "UID",
}
REF_TARGET = {
    "audioContentIDRef": "audioContent",
    "audioObjectIDRef": "audioObject",
    "audioPackFormatIDRef": "audioPackFormat",
    "audioChannelFormatIDRef": "audioChannelFormat",
    "audioStreamFormatIDRef": "audioStreamFormat",
    "audioTrackFormatIDRef": "audioTrackFormat",
    "audioTrackUIDRef": "audioTrackUID",
    "audioComplementaryObjectIDRef": "audioObject",
}
TYPE_LABELS = {"0001": "DirectSpeakers", "0002": "Matrix", "0003": "Objects", "0004": "HOA", "0005": "Binaural"}


def _local(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


@dataclass
class Element:
    tag: str
    id: str
    name: str
    attrs: dict
    refs: list          # [(ref tag, target id)]
    node: object


@dataclass
class Block:
    id: str
    rtime: str | None
    duration: str | None
    t: int | None
    dur: int | None
    t_exact: bool | None
    dur_exact: bool | None
    t_units: int | None
    dur_units: int | None
    cartesian: bool | None
    pos: tuple
    x_present: bool
    y_present: bool
    z_present: bool
    polar: dict
    gain: float | None
    gain_raw: str | None
    gain_unit: str | None
    importance: int | None
    size: tuple | None
    size_present: tuple
    jump: int | None
    interp_len_raw: str | None
    interp_len_seconds: Fraction | None
    interp_len_samples: int | None
    interp_len_exact: bool | None
    channel_lock: bool
    channel_lock_max_distance: str | None
    zones: list
    zone_rects: list
    extra: list

    @property
    def end(self) -> int | None:
        return None if self.t is None or self.dur is None else self.t + self.dur


@dataclass
class SpeakerBlock:
    id: str
    speaker_label: str | None
    pos: tuple
    rtime: str | None
    duration: str | None
    cartesian: bool | None
    extra: list


@dataclass
class AdmDoc:
    ns: str
    elements: dict = field(default_factory=dict)   # tag -> [Element]
    by_id: dict = field(default_factory=dict)      # id -> Element (first)
    duplicates: list = field(default_factory=list)  # (id, tag)
    block_ids: dict = field(default_factory=dict)  # block id -> channel format id

    def counts(self) -> dict:
        return {tag: len(v) for tag, v in self.elements.items()}

    def element(self, id_: str) -> Element | None:
        return self.by_id.get(id_)

    def channel_format_type(self, ac_id: str) -> str | None:
        e = self.by_id.get(ac_id)
        if e is None or e.tag != "audioChannelFormat":
            return None
        return e.attrs.get("typeDefinition") or TYPE_LABELS.get(e.attrs.get("typeLabel", ""))

    def programme_span(self, fs: int) -> tuple:
        progs = self.elements.get("audioProgramme", [])
        if not progs:
            return (None, None)
        a = progs[0].attrs
        start = timecode.decode(a["start"], fs).samples if "start" in a else None
        end = timecode.decode(a["end"], fs).samples if "end" in a else None
        return (start, end)

    def object_blocks(self, ac_id: str, fs: int) -> list:
        e = self.by_id.get(ac_id)
        if e is None:
            raise KeyError(ac_id)
        return [_object_block(b, fs) for b in e.node if _local(b.tag) == "audioBlockFormat"]

    def speaker_block(self, ac_id: str) -> SpeakerBlock:
        e = self.by_id.get(ac_id)
        if e is None:
            raise KeyError(ac_id)
        blocks = [b for b in e.node if _local(b.tag) == "audioBlockFormat"]
        if not blocks:
            raise ValueError(f"{ac_id} has no audioBlockFormat")
        return _speaker_block(blocks[0])


def _position(children):
    pos = [0.0, 0.0, 0.0]
    present = [False, False, False]
    polar = {}
    for c in children:
        coord = c.get("coordinate")
        if coord in ("X", "Y", "Z"):
            i = "XYZ".index(coord)
            pos[i] = float(c.text)
            present[i] = True
        elif coord is not None:
            polar[coord] = float(c.text)
    return tuple(pos), tuple(present), polar


def _object_block(b, fs: int) -> Block:
    rtime = b.get("rtime")
    duration = b.get("duration")
    t = dur = t_units = dur_units = None
    t_exact = dur_exact = None
    if rtime is not None:
        d = timecode.decode(rtime, fs)
        t, t_exact, t_units = d.samples, d.exact, d.decimal_units
    if duration is not None:
        d = timecode.decode(duration, fs)
        dur, dur_exact, dur_units = d.samples, d.exact, d.decimal_units
    cartesian = None
    gain = gain_raw = gain_unit = None
    importance = None
    size = [None, None, None]
    jump = None
    interp_raw = None
    channel_lock = False
    channel_lock_max = None
    zones: list = []
    zone_rects: list = []
    extra: list = []
    positions = []
    for c in b:
        tag = _local(c.tag)
        text = (c.text or "").strip()
        if tag == "cartesian":
            cartesian = text == "1"
        elif tag == "position":
            positions.append(c)
        elif tag == "gain":
            gain_raw = text
            gain = float(text)
            gain_unit = c.get("gainUnit")
        elif tag == "importance":
            importance = int(text)
        elif tag in ("width", "depth", "height"):
            size[("width", "depth", "height").index(tag)] = float(text)
        elif tag == "jumpPosition":
            jump = int(text) if text else None
            interp_raw = c.get("interpolationLength")
        elif tag == "channelLock":
            channel_lock = text == "1"
            channel_lock_max = c.get("maxDistance")
        elif tag == "zoneExclusion":
            for z in c:
                zones.append((z.text or "").strip())
                zone_rects.append(tuple(z.get(k) for k in ("minX", "maxX", "minY", "maxY", "minZ", "maxZ")))
        else:
            extra.append((tag, text))
    pos, present, polar = _position(positions)
    interp_seconds = interp_samples = None
    interp_exact = None
    if interp_raw is not None:
        interp_seconds = Fraction(interp_raw)
        x = interp_seconds * fs
        interp_samples = (x + Fraction(1, 2)).__floor__()
        interp_exact = x.denominator == 1
    size_present = tuple(s is not None for s in size)
    return Block(
        id=b.get("audioBlockFormatID", ""), rtime=rtime, duration=duration, t=t, dur=dur,
        t_exact=t_exact, dur_exact=dur_exact, t_units=t_units, dur_units=dur_units,
        cartesian=cartesian, pos=pos, x_present=present[0], y_present=present[1], z_present=present[2],
        polar=polar, gain=gain, gain_raw=gain_raw, gain_unit=gain_unit, importance=importance,
        size=tuple(size) if any(size_present) else None, size_present=size_present,
        jump=jump, interp_len_raw=interp_raw, interp_len_seconds=interp_seconds,
        interp_len_samples=interp_samples, interp_len_exact=interp_exact,
        channel_lock=channel_lock, channel_lock_max_distance=channel_lock_max,
        zones=zones, zone_rects=zone_rects, extra=extra,
    )


def _speaker_block(b) -> SpeakerBlock:
    label = None
    cartesian = None
    positions = []
    extra = []
    for c in b:
        tag = _local(c.tag)
        text = (c.text or "").strip()
        if tag == "speakerLabel":
            label = text
        elif tag == "cartesian":
            cartesian = text == "1"
        elif tag == "position":
            positions.append(c)
        else:
            extra.append((tag, text))
    pos, _present, _polar = _position(positions)
    return SpeakerBlock(b.get("audioBlockFormatID", ""), label, pos, b.get("rtime"), b.get("duration"), cartesian, extra)


def parse(xml_bytes: bytes) -> AdmDoc:
    root = ET.fromstring(xml_bytes)
    ns = root.tag[1:].split("}")[0] if root.tag.startswith("{") else ""
    afe = None
    for node in root.iter():
        if _local(node.tag) == "audioFormatExtended":
            afe = node
            break
    if afe is None:
        raise ValueError("no audioFormatExtended element")
    doc = AdmDoc(ns)
    for tag in ID_ATTR:
        doc.elements[tag] = []
    for node in afe:
        tag = _local(node.tag)
        if tag not in ID_ATTR:
            doc.elements.setdefault(tag, []).append(Element(tag, "", "", dict(node.attrib), [], node))
            continue
        id_ = node.get(ID_ATTR[tag], "")
        name = node.get(tag + "Name", "")
        refs = [(_local(c.tag), (c.text or "").strip()) for c in node if _local(c.tag).endswith("IDRef") or _local(c.tag) == "audioTrackUIDRef"]
        el = Element(tag, id_, name, dict(node.attrib), refs, node)
        doc.elements[tag].append(el)
        if id_ in doc.by_id:
            doc.duplicates.append((id_, tag))
        else:
            doc.by_id[id_] = el
        if tag == "audioChannelFormat":
            for b in node:
                if _local(b.tag) == "audioBlockFormat":
                    bid = b.get("audioBlockFormatID", "")
                    if bid in doc.block_ids:
                        doc.duplicates.append((bid, "audioBlockFormat"))
                    else:
                        doc.block_ids[bid] = id_
    return doc


def _type_of(el: Element) -> str | None:
    return el.attrs.get("typeDefinition") or TYPE_LABELS.get(el.attrs.get("typeLabel", ""))


def check_references(doc: AdmDoc) -> list[Finding]:
    out: list[Finding] = []
    for id_, tag in doc.duplicates:
        out.append(Finding("duplicate-id", f"{tag} id {id_} occurs more than once"))
    for tag, els in doc.elements.items():
        for el in els:
            for ref_tag, target in el.refs:
                want = REF_TARGET.get(ref_tag)
                got = doc.by_id.get(target)
                if got is None:
                    out.append(Finding("dangling", f"{tag} {el.id} -> {ref_tag} {target} does not exist"))
                elif want and got.tag != want:
                    out.append(Finding("wrong-target-type", f"{tag} {el.id} -> {ref_tag} {target} is a {got.tag}"))
    # type consistency pack -> channel formats
    for ap in doc.elements.get("audioPackFormat", []):
        t = _type_of(ap)
        for ref_tag, target in ap.refs:
            if ref_tag == "audioChannelFormatIDRef" and target in doc.by_id:
                tc = _type_of(doc.by_id[target])
                if t and tc and t != tc:
                    out.append(Finding("type-mismatch", f"pack {ap.id} ({t}) references channel {target} ({tc})"))
        lbl = ap.attrs.get("typeLabel")
        if lbl and t and TYPE_LABELS.get(lbl) != t:
            out.append(Finding("type-label", f"pack {ap.id} typeLabel {lbl} vs typeDefinition {t}"))
    for ac in doc.elements.get("audioChannelFormat", []):
        lbl = ac.attrs.get("typeLabel")
        t = ac.attrs.get("typeDefinition")
        if lbl and t and TYPE_LABELS.get(lbl) != t:
            out.append(Finding("type-label", f"channel {ac.id} typeLabel {lbl} vs typeDefinition {t}"))
        if not [b for b in ac.node if _local(b.tag) == "audioBlockFormat"]:
            out.append(Finding("no-blocks", f"channel {ac.id} has no audioBlockFormat"))
    # stream formats must point at a channel, a pack and a track format; track formats back at a stream
    for asf in doc.elements.get("audioStreamFormat", []):
        tags = {r for r, _ in asf.refs}
        for need in ("audioChannelFormatIDRef", "audioTrackFormatIDRef"):
            if need not in tags:
                out.append(Finding("stream-incomplete", f"stream {asf.id} lacks {need}"))
    for atf in doc.elements.get("audioTrackFormat", []):
        if "audioStreamFormatIDRef" not in {r for r, _ in atf.refs}:
            out.append(Finding("track-incomplete", f"track format {atf.id} lacks audioStreamFormatIDRef"))
    # every track UID must be owned by exactly one object
    owners: dict = {}
    for ao in doc.elements.get("audioObject", []):
        for ref_tag, target in ao.refs:
            if ref_tag == "audioTrackUIDRef":
                owners.setdefault(target, []).append(ao.id)
    for atu in doc.elements.get("audioTrackUID", []):
        n = len(owners.get(atu.id, []))
        if n == 0:
            out.append(Finding("orphan-uid", f"track UID {atu.id} is referenced by no audioObject"))
        elif n > 1:
            out.append(Finding("shared-uid", f"track UID {atu.id} is referenced by {owners[atu.id]}"))
    # programme -> content -> object closure
    reachable = set()
    for apr in doc.elements.get("audioProgramme", []):
        for _r, aco in apr.refs:
            reachable.add(aco)
            e = doc.by_id.get(aco)
            if e is not None:
                for _r2, ao in e.refs:
                    reachable.add(ao)
    for tag in ("audioContent", "audioObject"):
        for el in doc.elements.get(tag, []):
            if el.id not in reachable:
                out.append(Finding("unreachable", f"{tag} {el.id} is not reachable from any audioProgramme"))
    return out


@dataclass(frozen=True)
class ChnaEntry:
    track_index: int
    uid: str
    track_ref: str
    pack_ref: str


@dataclass
class Chna:
    num_tracks: int
    num_uids: int
    entries: list


def parse_chna(payload: bytes) -> Chna:
    if len(payload) < 4:
        raise ValueError("chna chunk shorter than 4 bytes")
    num_tracks, num_uids = struct.unpack("<HH", payload[:4])
    entries = []
    pos = 4
    while pos + 40 <= len(payload):
        idx, uid, tref, pref = struct.unpack("<H12s14s11sx", payload[pos : pos + 40])
        entries.append(ChnaEntry(idx, uid.rstrip(b"\x00").decode("latin-1"), tref.rstrip(b"\x00").decode("latin-1"), pref.rstrip(b"\x00").decode("latin-1")))
        pos += 40
    return Chna(num_tracks, num_uids, entries)


@dataclass
class TrackLink:
    track_index: int
    uid: str
    track_format: str | None
    stream_format: str | None
    channel_format: str | None
    pack_from_chna: str
    pack_from_uid: str | None
    audio_object: str | None
    problems: list


def track_chain(doc: AdmDoc, chna: Chna, channels: int) -> list[TrackLink]:
    uid_owner = {}
    for ao in doc.elements.get("audioObject", []):
        for ref_tag, target in ao.refs:
            if ref_tag == "audioTrackUIDRef":
                uid_owner[target] = ao.id
    links = []
    for e in sorted(chna.entries, key=lambda x: x.track_index):
        problems = []
        atu = doc.by_id.get(e.uid)
        tf = ss = cf = pack_uid = None
        if atu is None or atu.tag != "audioTrackUID":
            problems.append(f"UID {e.uid} not in axml")
        else:
            refs = dict((r, t) for r, t in atu.refs)
            tf = refs.get("audioTrackFormatIDRef")
            pack_uid = refs.get("audioPackFormatIDRef")
            if tf != e.track_ref:
                problems.append(f"chna trackRef {e.track_ref} != UID's {tf}")
        tfe = doc.by_id.get(e.track_ref)
        if tfe is None or tfe.tag != "audioTrackFormat":
            problems.append(f"trackRef {e.track_ref} not in axml")
        else:
            ss = dict(tfe.refs).get("audioStreamFormatIDRef")
            sse = doc.by_id.get(ss or "")
            if sse is None:
                problems.append(f"stream {ss} not in axml")
            else:
                cf = dict(sse.refs).get("audioChannelFormatIDRef")
                if cf not in doc.by_id:
                    problems.append(f"channel {cf} not in axml")
        if e.pack_ref not in doc.by_id:
            problems.append(f"packRef {e.pack_ref} not in axml")
        elif pack_uid and pack_uid != e.pack_ref:
            problems.append(f"chna packRef {e.pack_ref} != UID's {pack_uid}")
        links.append(TrackLink(e.track_index, e.uid, e.track_ref, ss, cf, e.pack_ref, pack_uid, uid_owner.get(e.uid), problems))
    return links


def check_chna(doc: AdmDoc, chna: Chna, channels: int) -> list[Finding]:
    out: list[Finding] = []
    if chna.num_tracks != channels or len(chna.entries) != channels:
        out.append(Finding("chna-track-count", f"chna numTracks {chna.num_tracks}, entries {len(chna.entries)}, PCM channels {channels}"))
    if chna.num_uids != len(chna.entries):
        out.append(Finding("chna-uid-count", f"chna numUIDs {chna.num_uids} != entries {len(chna.entries)}"))
    idx = sorted(e.track_index for e in chna.entries)
    if idx != list(range(1, len(chna.entries) + 1)):
        out.append(Finding("chna-track-index", f"track indices {idx} are not 1..{len(chna.entries)}"))
    if len({e.uid for e in chna.entries}) != len(chna.entries):
        out.append(Finding("chna-duplicate-uid", "a UID occurs twice in chna"))
    for link in track_chain(doc, chna, channels):
        for p in link.problems:
            kind = "chna-dangling" if "not in axml" in p else "chna-inconsistent"
            out.append(Finding(kind, f"track {link.track_index}: {p}"))
        if link.audio_object is None:
            out.append(Finding("chna-orphan", f"track {link.track_index} UID {link.uid} belongs to no audioObject"))
    return out
