"""DAMF reader: ``.atmos`` header and ``.atmos.metadata`` delta events.

DAMF has no public specification; the reader implements the YAML subset the
Dolby tools and oadec write as a strict line grammar (no YAML library, so a
malformed line is a finding rather than a silently coerced value).  Events are
deltas: the first event of an element carries its full state, later events
carry only the fields that changed.  ``reconstruct`` turns them into full
states and records which fields really changed between consecutive states.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field

from .riff import Finding

_KV = re.compile(r"^(\s*)(-\s*)?([A-Za-z_][A-Za-z0-9_]*):\s*(.*?)\s*$")
_EVENT = re.compile(r"^\s*-\s*ID:\s*(\d+)\s*$")
_LIST = re.compile(r"^\[(.*)\]$")


class DamfError(Exception):
    pass


def _scalar(text: str):
    if text == "":
        return ""
    if text == "true":
        return True
    if text == "false":
        return False
    if text in ("-inf", "inf", "-Inf", "Inf", ".inf", "-.inf"):
        return text
    m = _LIST.match(text)
    if m:
        parts = [p.strip() for p in m.group(1).split(",") if p.strip()]
        return tuple(float(p) for p in parts)
    if re.fullmatch(r"-?\d+", text):
        return int(text)
    if re.fullmatch(r"-?\d+\.\d*(e-?\d+)?|-?\d*\.\d+(e-?\d+)?|-?\d+e-?\d+", text):
        return float(text)
    return text


@dataclass
class Header:
    path: str
    version: str | None = None
    fps: str | None = None
    offset: str | None = None
    metadata_file: str | None = None
    audio_file: str | None = None
    creation_tool: str | None = None
    creation_tool_version: str | None = None
    sc_bed_configuration: str | None = None
    bed_channels: list = field(default_factory=list)   # [(name, id)] in file order
    object_ids: list = field(default_factory=list)
    raw: str = ""

    def kind(self, id_: int) -> str | None:
        if any(i == id_ for _n, i in self.bed_channels):
            return "bed"
        if id_ in self.object_ids:
            return "object"
        return None

    def element_order(self) -> list:
        """Element ids in DAMF audio channel order: bed channels, then objects."""
        return [i for _n, i in self.bed_channels] + list(self.object_ids)


def read_atmos(path: str) -> Header:
    h = Header(path)
    with open(path, "r", encoding="utf-8") as f:
        h.raw = f.read()
    section = None
    pending_channel = None
    for line in h.raw.splitlines():
        m = _KV.match(line)
        if not m:
            continue
        _indent, _dash, key, value = m.groups()
        if key == "bedInstances":
            section = "beds"
            continue
        if key == "objects":
            section = "objects"
            continue
        if key == "presentations":
            section = None
            continue
        if section == "beds":
            if key == "channel":
                pending_channel = value
            elif key == "ID" and pending_channel is not None:
                h.bed_channels.append((pending_channel, int(value)))
                pending_channel = None
            elif key == "channels":
                pass
            else:
                section = None
        if section == "objects":
            if key == "ID":
                h.object_ids.append(int(value))
                continue
            section = None
        if key == "version" and h.version is None:
            h.version = value
        elif key == "fps" and h.fps is None:
            h.fps = value
        elif key == "offset" and h.offset is None:
            h.offset = value
        elif key == "metadata" and h.metadata_file is None:
            h.metadata_file = value
        elif key == "audio" and h.audio_file is None:
            h.audio_file = value
        elif key == "creationTool" and h.creation_tool is None:
            h.creation_tool = value
        elif key == "creationToolVersion" and h.creation_tool_version is None:
            h.creation_tool_version = value
        elif key == "scBedConfiguration" and h.sc_bed_configuration is None:
            h.sc_bed_configuration = value
    return h


@dataclass
class RawEvent:
    id: int
    sample_pos: int | None
    fields: dict
    present: set
    line_no: int


def read_metadata(path: str) -> tuple:
    """``(sample_rate, [RawEvent])`` in file order."""
    fs = None
    events: list[RawEvent] = []
    cur: RawEvent | None = None
    with open(path, "r", encoding="utf-8") as f:
        for n, line in enumerate(f, 1):
            line = line.rstrip("\r\n")
            if not line.strip():
                continue
            m = _EVENT.match(line)
            if m:
                cur = RawEvent(int(m.group(1)), None, {}, set(), n)
                events.append(cur)
                continue
            m = _KV.match(line)
            if not m:
                raise DamfError(f"{path}:{n}: cannot parse {line!r}")
            _indent, dash, key, value = m.groups()
            if cur is None or (dash and key != "ID"):
                if key == "sampleRate":
                    fs = int(value)
                    continue
                if key == "events":
                    continue
                raise DamfError(f"{path}:{n}: unexpected {key!r} outside an event")
            if key == "samplePos":
                cur.sample_pos = int(value)
            elif key == "ID":
                raise DamfError(f"{path}:{n}: ID inside an event body")
            else:
                cur.fields[key] = _scalar(value)
                cur.present.add(key)
    if fs is None:
        raise DamfError(f"{path}: no sampleRate")
    return fs, events


REQUIRED = {"object": {"active", "pos"}, "bed": {"active"}}
EXPECTED = {
    "object": {"active", "pos", "snap", "elevation", "zones", "size", "importance", "gain", "rampLength", "trimBypass"},
    "bed": {"active", "importance", "gain", "rampLength", "trimBypass"},
}


@dataclass
class State:
    id: int
    kind: str
    t: int
    values: dict
    present: set
    changed: set
    line_no: int


def reconstruct(raw: list, header: Header) -> tuple:
    """``({element id: [State]}, [Finding])``."""
    states: dict = {}
    last_pos: dict = {}
    findings: list[Finding] = []
    for ev in raw:
        kind = header.kind(ev.id)
        if kind is None:
            findings.append(Finding("damf-unknown-id", f"line {ev.line_no}: ID {ev.id} is not in the .atmos header"))
            continue
        if ev.sample_pos is None:
            findings.append(Finding("damf-no-samplepos", f"line {ev.line_no}: ID {ev.id} event without samplePos"))
            continue
        if ev.id not in states:
            missing = REQUIRED[kind] - ev.present
            if missing:
                findings.append(Finding("damf-first-event-incomplete", f"line {ev.line_no}: ID {ev.id} first event lacks {sorted(missing)}"))
            sparse = EXPECTED[kind] - ev.present
            if sparse and not missing:
                findings.append(Finding("damf-first-event-sparse", f"line {ev.line_no}: ID {ev.id} first event has no {sorted(sparse)}"))
            values = dict(ev.fields)
            changed = set(ev.present)
        else:
            prev = states[ev.id][-1].values
            if ev.sample_pos < last_pos[ev.id]:
                findings.append(Finding("damf-out-of-order", f"line {ev.line_no}: ID {ev.id} samplePos {ev.sample_pos} < previous {last_pos[ev.id]}"))
            values = dict(prev)
            values.update(ev.fields)
            changed = {k for k in ev.present if prev.get(k, object()) != values[k]}
        states.setdefault(ev.id, []).append(State(ev.id, kind, ev.sample_pos, values, set(ev.present), changed, ev.line_no))
        last_pos[ev.id] = ev.sample_pos
    return states, findings
