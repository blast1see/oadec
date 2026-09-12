"""Parser for the text of ``oadec oamd <file> --dump N``.

The dump is oadec's own reading of the raw OAMD, printed before any programme
model or writer touches it: gain in dB, priority, the OAMD room position
(x, y in 0..1, z in -1..1, four decimals), all three size axes, zone index,
elevation/snap flags, distance and divergence when present, plus the timing
words (``sample_offset`` and per block ``(block_offset_factor, ramp_duration)``).
The audit uses it as a third source for *timing* and for the fields DAMF and
ADM cannot carry; positions are only four decimals here.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field

_UNIT = re.compile(r"^access unit (\d+): OAMD v(\d+) (\d+) objects, container sample offset (\S+), (\d+) elements, padding (\d+) bits")
_FRAME = re.compile(r"^frame (\d+)")
_PROGRAM = re.compile(r"^\s*program: dyn_only (\w+) beds (\[.*?\]) isf (\S+) dynamic (\d+)")
_ELEMENT = re.compile(r"^\s*object element \((\d+) bytes, padding (\d+)\): sample_offset (\d+) blocks \[(.*)\]")
_BLOCKS = re.compile(r"\((\d+), (\d+)\)")
_OBJ = re.compile(
    r"^\s*obj\s+(\d+) blk (\d+):\s*(bed)?\s*gain (\S+) dB prio (\S+) \[(\w+)/(\w+)\] pos \(([-\d.]+), ([-\d.]+), ([-\d.]+)\) size \(([-\d.]+), ([-\d.]+), ([-\d.]+)\) zone (\d+)(.*)$"
)


@dataclass
class ObjLine:
    obj: int
    blk: int
    bed: bool
    gain_db: object
    prio: float
    basic_status: str
    render_status: str
    pos_oamd: tuple
    size: tuple
    zone: int
    elevation: bool = True
    snap: bool = False
    distance: str | None = None
    divergence: str | None = None
    differential: bool = False
    active: bool = True
    flags: list = field(default_factory=list)


@dataclass
class Unit:
    au: int
    version: int
    object_count: int
    container_offset: int | None
    elements: int
    padding_bits: int
    program: dict = field(default_factory=dict)
    sample_offset: int | None = None
    blocks: list = field(default_factory=list)  # [(block_offset_factor, ramp)]
    objects: dict = field(default_factory=dict)  # (obj, blk) -> ObjLine


@dataclass
class Dump:
    units: list = field(default_factory=list)
    footer: dict = field(default_factory=dict)


def _num(s: str):
    if s in ("-inf", "inf"):
        return s
    try:
        return int(s)
    except ValueError:
        return float(s)


def parse(text: str) -> Dump:
    d = Dump()
    cur: Unit | None = None
    for line in text.splitlines():
        m = _UNIT.match(line)
        if m:
            au, ver, n, coff, el, pad = m.groups()
            cur = Unit(int(au), int(ver), int(n), None if coff == "None" else int(coff), int(el), int(pad))
            d.units.append(cur)
            continue
        if cur is None:
            continue
        m = _PROGRAM.match(line)
        if m:
            dyn, beds, isf, dynamic = m.groups()
            cur.program = {"dyn_only": dyn == "true", "beds": beds, "isf": None if isf == "None" else int(isf), "dynamic": int(dynamic)}
            continue
        m = _ELEMENT.match(line)
        if m:
            _bytes, _pad, so, blocks = m.groups()
            cur.sample_offset = int(so)
            cur.blocks = [(int(a), int(b)) for a, b in _BLOCKS.findall(blocks)]
            continue
        m = _OBJ.match(line)
        if m:
            obj, blk, bed, gain, prio, bs, rs, x, y, z, w, dd, h, zone, rest = m.groups()
            o = ObjLine(int(obj), int(blk), bed is not None, _num(gain), float(prio), bs, rs,
                        (float(x), float(y), float(z)), (float(w), float(dd), float(h)), int(zone))
            toks = rest.split()
            i = 0
            while i < len(toks):
                t = toks[i]
                if t == "no-elev":
                    o.elevation = False
                elif t == "snap":
                    o.snap = True
                elif t == "diff":
                    o.differential = True
                elif t in ("inactive", "not-active"):
                    o.active = False
                elif t == "dist" and i + 1 < len(toks):
                    o.distance = toks[i + 1]
                    i += 1
                elif t == "div" and i + 1 < len(toks):
                    o.divergence = toks[i + 1]
                    i += 1
                else:
                    o.flags.append(t)
                i += 1
            cur.objects[(o.obj, o.blk)] = o
            continue
        fm = re.match(r"^([A-Za-z /]+):\s+(.*)$", line)
        if fm and not line.startswith(" "):
            d.footer[fm.group(1).strip()] = fm.group(2).strip()
    return d


@dataclass
class Event:
    obj: int
    t: int
    au: int
    blk: int
    sample_offset: int
    block_offset_factor: int
    ramp: int
    bed: bool
    gain_db: object
    prio: float
    pos_oamd: tuple
    pos_damf: tuple
    size: tuple
    zone: int
    elevation: bool
    snap: bool
    distance: str | None
    divergence: str | None
    active: bool


def damf_position(p: tuple) -> tuple:
    x, y, z = p
    return ((x - 0.5) * 2.0, (0.5 - y) * 2.0, z)


def events(dump: Dump, au_samples: int = 40, base_offset: int = 0) -> list:
    """Per-object update events with ``t = au*au_samples + container_offset + sample_offset + 32*block_offset_factor``."""
    out = []
    for u in dump.units:
        if u.sample_offset is None:
            continue
        base = base_offset + u.au * au_samples + (u.container_offset or 0)
        for blk, (bof, ramp) in enumerate(u.blocks):
            t = base + u.sample_offset + 32 * bof
            for (obj, b), o in sorted(u.objects.items()):
                if b != blk:
                    continue
                out.append(Event(obj, t, u.au, blk, u.sample_offset, bof, ramp, o.bed, o.gain_db, o.prio, o.pos_oamd,
                                 damf_position(o.pos_oamd), o.size, o.zone, o.elevation, o.snap, o.distance, o.divergence, o.active))
    out.sort(key=lambda e: (e.obj, e.t, e.au, e.blk))
    return out
