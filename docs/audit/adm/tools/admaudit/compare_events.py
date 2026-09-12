"""Reconciliation ledger: every DAMF state and every ADM block ends in one class.

Classes that are lossy by construction of the Dolby Atmos Master ADM profile
(or of oadec's writer) are counted, not treated as defects:

  bed_event             DAMF bed states; the profile allows no time-varying bed blocks
  inexpressible_change  a DAMF state whose only changes are fields ADM cannot carry
                        (ramp, gain, importance, trim, screen, depth); the ADM shows a
                        duplicate block
  trailing_popped       the same at the end of the object's list; no ADM block at all
  superseded_same_pos   two DAMF states at one sample; only the last survives
  beyond_end            a DAMF state at or after the end of the audio
  synthetic_block0      an ADM block at 0 with no DAMF state at 0
  absorbed_by_synthetic a late first state held from 0 by that block and then dropped:
                        the state survives, the time of its arrival does not

Defect classes: time_mismatch, value_mismatch, unexplained_missing,
unexplained_extra.  Independently of the classes, differences in gain,
importance and ramp between a matched pair are tallied in ``loss`` -- they are
the semantic loss the profile imposes, and the audit measures them rather than
hiding them inside "matched".
"""
from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field

import numpy as np

from .normalise import ObjEvent, Scene

INEXPRESSIBLE = {"rampLength", "gain", "importance", "trimBypass", "screenFactor", "depthFactor", "dialog", "music", "headTrackMode", "binauralRenderMode"}
EXPRESSIBLE = {"active", "pos", "size", "zones", "snap", "elevation"}
DEFECT_CLASSES = ("time_mismatch", "value_mismatch", "unexplained_missing", "unexplained_extra")
TIME_WINDOW = 1536  # samples within which a same-content block is a time mismatch rather than missing+extra
SIZE_TOL = 1e-9


@dataclass
class LedgerItem:
    cls: str
    obj: int
    t: int | None
    damf_ref: str | None
    adm_ref: str | None
    t_damf: int | None
    t_adm: int | None
    fields: set = field(default_factory=set)
    detail: dict = field(default_factory=dict)


@dataclass
class Ledger:
    classes: Counter = field(default_factory=Counter)
    items: list = field(default_factory=list)
    defects: list = field(default_factory=list)
    loss: Counter = field(default_factory=Counter)
    loss_items: list = field(default_factory=list)
    identity_ok: bool = True
    identity: dict = field(default_factory=dict)
    tiling: dict = field(default_factory=dict)
    bed_changes_lost: int = 0
    time_lost_events: int = 0
    presence_differences: list = field(default_factory=list)
    per_object: dict = field(default_factory=dict)

    def to_json(self) -> dict:
        return {
            "classes": dict(self.classes),
            "defects": [_item_json(i) for i in self.defects],
            "loss": dict(self.loss),
            "identity_ok": self.identity_ok,
            "identity": self.identity,
            "tiling": self.tiling,
            "bed_changes_lost": self.bed_changes_lost,
            "time_lost_events": self.time_lost_events,
            "presence_differences": self.presence_differences,
            "per_object": self.per_object,
            "items": [_item_json(i) for i in self.items],
        }


def _item_json(i: LedgerItem) -> dict:
    return {"cls": i.cls, "obj": i.obj, "t": i.t, "damf": i.damf_ref, "adm": i.adm_ref, "t_damf": i.t_damf, "t_adm": i.t_adm, "fields": sorted(i.fields), "detail": i.detail}


def _f32(v) -> float:
    """Both formats serialise float32 values (DAMF: shortest repr, ADM: ten decimals of
    the widened value); comparing at float32 precision makes the two spellings of one
    number equal while one float32 ulp still differs."""
    return float(np.float32(v or 0.0))


def _pos_diff(a: tuple, b: tuple) -> float:
    return max(abs(_f32(x) - _f32(y)) for x, y in zip(a, b))


def expressible_diff(d: ObjEvent, a: ObjEvent, pos_tol: float = 0.0) -> set:
    """Fields an ADM block *can* carry that differ from the DAMF state."""
    out = set()
    if _pos_diff(d.pos, a.pos) > pos_tol:
        out.add("pos")
    if bool(d.active) != bool(a.active):
        out.add("active")
    ds, as_ = d.size, a.size
    if any(abs(_f32(ds[k]) - _f32(as_[k])) > SIZE_TOL for k in ("w", "d", "h")):
        out.add("size")
    if set(d.zones["names"]) != set(a.zones["names"]):
        out.add("zones")
    if bool(d.snap) != bool(a.snap):
        out.add("snap")
    return out


def inexpressible_diff(d: ObjEvent, a: ObjEvent, first_block: bool) -> set:
    """Semantic differences the profile gives the writer no way to express."""
    out = set()
    if d.active:
        if d.gain["present"] and (d.gain["minus_inf"] or abs(d.gain["lin"] - a.gain["lin"]) > 1e-9):
            out.add("gain")
        if d.importance["present"] and d.importance["value"] is not None and d.importance["value"] != 1.0 and a.importance["value"] == 10:
            out.add("importance")
    if not first_block and d.ramp is not None and a.interp is not None:
        if a.interp.get("len_samples") != d.ramp:
            out.add("ramp")
    return out


def presence_diff(d: ObjEvent, a: ObjEvent) -> set:
    out = set()
    if d.gain["present"] != a.gain["present"]:
        out.add("gain")
    if d.importance["present"] != a.importance["present"]:
        out.add("importance")
    if d.size["present"] != a.size["present"]:
        out.add("size")
    if d.z_present != a.z_present:
        out.add("z")
    return out


def _tiling(blocks: list, frames: int | None) -> dict:
    bs = sorted(blocks, key=lambda b: b.t)
    gaps = overlaps = 0
    tile_dec = Counter()
    for x, y in zip(bs, bs[1:]):
        if x.end is not None:
            if x.end < y.t:
                gaps += 1
            elif x.end > y.t:
                overlaps += 1
        if x.t_units is not None and x.dur_units is not None and y.t_units is not None:
            tile_dec[(x.t_units + x.dur_units) - y.t_units] += 1
    return {
        "blocks": len(bs),
        "starts_at_zero": bool(bs) and bs[0].t == 0,
        "gaps": gaps,
        "overlaps": overlaps,
        "last_end": bs[-1].end if bs else None,
        "ends_at_frames": bool(bs) and frames is not None and bs[-1].end == frames,
        "overrun": (bs[-1].end - frames) if bs and frames is not None and bs[-1].end is not None and bs[-1].end > frames else 0,
        "tile_dec": {str(k): v for k, v in sorted(tile_dec.items())},
        "sorted": all(x.t <= y.t for x, y in zip(bs, bs[1:])),
        "min_dur": min((b.dur for b in bs if b.dur is not None), default=None),
        "blocks_shorter_than_interp": sum(1 for b in bs if b.dur is not None and b.interp and b.interp.get("len_samples") is not None and b.dur < b.interp["len_samples"]),
    }


def _default_state_like(a: ObjEvent) -> bool:
    return (not a.active) and a.pos == (0.0, 0.0, 0.0)


def reconcile_object(k: int, D: list, A: list, frames: int | None, mode: str, pos_tol: float, ledger: Ledger) -> None:
    items: list[LedgerItem] = []
    D = sorted(D, key=lambda e: e.t)
    A = sorted(A, key=lambda b: b.t)
    # 1. same-position states: only the last survives
    effective = []
    for i, s in enumerate(D):
        if i + 1 < len(D) and D[i + 1].t == s.t:
            items.append(LedgerItem("superseded_same_pos", k, s.t, s.ref, None, s.t, None))
        else:
            effective.append(s)
    # 2. states at or after the end
    remaining = []
    for s in effective:
        if frames is not None and s.t >= frames:
            items.append(LedgerItem("beyond_end", k, s.t, s.ref, None, s.t, None, detail={"frames": frames}))
        else:
            remaining.append(s)
    blocks_at = {}
    for b in A:
        blocks_at.setdefault(b.t, []).append(b)
    used = set()
    # 3. trailing states whose only changes are inexpressible and that have no block
    while remaining:
        s = remaining[-1]
        if s.changed is not None and s.changed and s.changed <= INEXPRESSIBLE and s.t not in blocks_at:
            items.append(LedgerItem("trailing_popped", k, s.t, s.ref, None, s.t, None, fields=set(s.changed)))
            remaining.pop()
        else:
            break
    # 4. match by exact time
    unmatched_states = []
    for s in remaining:
        cands = [b for b in blocks_at.get(s.t, []) if id(b) not in used]
        if not cands:
            unmatched_states.append(s)
            continue
        b = cands[0]
        used.add(id(b))
        first_block = A and b is A[0]
        ex = expressible_diff(s, b, pos_tol)
        inex = inexpressible_diff(s, b, bool(first_block))
        if mode == "strict":
            pres = presence_diff(s, b)
            if pres:
                ledger.presence_differences.append({"obj": k, "t": s.t, "fields": sorted(pres)})
        for f in inex:
            ledger.loss[f] += 1
            ledger.loss_items.append({"obj": k, "t": s.t, "field": f, "damf": _field_value(s, f), "adm": _field_value(b, f)})
        if ex:
            items.append(LedgerItem("value_mismatch", k, s.t, s.ref, b.ref, s.t, b.t, fields=ex, detail={"pos_delta": _pos_diff(s.pos, b.pos), "damf_pos": s.pos, "adm_pos": b.pos, "inexpressible": sorted(inex)}))
        elif s.changed is not None and s.changed and s.changed <= INEXPRESSIBLE:
            items.append(LedgerItem("inexpressible_change", k, s.t, s.ref, b.ref, s.t, b.t, fields=set(s.changed)))
        else:
            items.append(LedgerItem("matched", k, s.t, s.ref, b.ref, s.t, b.t, detail={"inexpressible": sorted(inex)} if inex else {}))
    # 5a. a late first state that the writer holds from sample 0 (synthetic block) and then drops as a
    #     duplicate: the state survives, its time does not
    absorbed = []
    if A and A[0].t == 0 and D and D[0].t > 0:
        for s in list(unmatched_states):
            if not expressible_diff(s, A[0], pos_tol) and s.t not in blocks_at:
                items.append(LedgerItem("absorbed_by_synthetic", k, s.t, s.ref, A[0].ref, s.t, 0, detail={"time_lost_samples": s.t}))
                unmatched_states.remove(s)
                absorbed.append(s)
                ledger.time_lost_events += 1
    # 5. states without a block: time mismatch if an unused block of equal content lies within the window
    for s in unmatched_states:
        best = None
        for b in A:
            if id(b) in used:
                continue
            dt = b.t - s.t
            if abs(dt) <= TIME_WINDOW and not expressible_diff(s, b, pos_tol):
                if best is None or abs(dt) < abs(best[1]):
                    best = (b, dt)
        if best is not None:
            b, dt = best
            used.add(id(b))
            items.append(LedgerItem("time_mismatch", k, s.t, s.ref, b.ref, s.t, b.t, detail={"delta_samples": dt}))
        else:
            items.append(LedgerItem("unexplained_missing", k, s.t, s.ref, None, s.t, None, fields=set(s.changed or ())))
    # 6. blocks without a state
    first_state = D[0] if D else None
    for b in A:
        if id(b) in used:
            continue
        if b.t == 0 and (first_state is None or first_state.t > 0):
            if first_state is not None and not expressible_diff(first_state, b, pos_tol):
                holds = "first-state"
            elif _default_state_like(b):
                holds = "default"
            else:
                holds = "other"
            items.append(LedgerItem("synthetic_block0", k, 0, None, b.ref, None, 0, detail={"holds": holds, "active": b.active}))
        else:
            items.append(LedgerItem("unexplained_extra", k, b.t, None, b.ref, None, b.t, detail={"pos": b.pos}))
    order = {"synthetic_block0": 0}
    items.sort(key=lambda i: (i.t if i.t is not None else -1, order.get(i.cls, 1)))
    counts = Counter(i.cls for i in items)
    expected_blocks = len(D) - counts["superseded_same_pos"] - counts["beyond_end"] - counts["trailing_popped"] - counts["absorbed_by_synthetic"] + counts["synthetic_block0"]
    ident = {"damf_states": len(D), "adm_blocks": len(A), "expected_blocks": expected_blocks, "ok": expected_blocks == len(A) and counts["unexplained_missing"] == 0 and counts["unexplained_extra"] == 0}
    ledger.per_object[k] = {"classes": dict(counts), "identity": ident, "tiling": _tiling(A, frames)}
    ledger.items.extend(items)
    ledger.classes.update(counts)
    if not ident["ok"]:
        ledger.identity_ok = False


def _field_value(e: ObjEvent, f: str):
    if f == "gain":
        return e.gain
    if f == "importance":
        return e.importance
    if f == "ramp":
        return e.ramp if e.ramp is not None else (e.interp or {}).get("len_samples")
    return None


def reconcile(damf: Scene, adm: Scene, mode: str = "effective", pos_tol: float = 0.0, frames: int | None = None) -> Ledger:
    ledger = Ledger()
    if frames is None:
        frames = damf.source.get("frames") if damf.source.get("frames") is not None else adm.source.get("frames")
    # beds: every DAMF bed event is by construction unrepresentable as a time-varying ADM block
    lost = 0
    for bed in damf.beds:
        for i, ev in enumerate(bed.events):
            ledger.items.append(LedgerItem("bed_event", -1, ev.t, ev.ref, None, ev.t, None, fields=set(ev.changed or ()), detail={"bed": bed.element_id}))
            ledger.classes["bed_event"] += 1
            if i > 0 and ev.changed and (ev.changed & {"active", "gain", "importance"}):
                lost += 1
    ledger.bed_changes_lost = lost
    adm_by_ordinal = {o.ordinal: o for o in adm.objects}
    for o in damf.objects:
        a = adm_by_ordinal.get(o.ordinal)
        reconcile_object(o.ordinal, list(o.events), list(a.events) if a else [], frames, mode, pos_tol, ledger)
    for k, a in adm_by_ordinal.items():
        if not any(o.ordinal == k for o in damf.objects):
            reconcile_object(k, [], list(a.events), frames, mode, pos_tol, ledger)
    ledger.defects = [i for i in ledger.items if i.cls in DEFECT_CLASSES]
    tilings = [v["tiling"] for v in ledger.per_object.values()]
    ledger.tiling = {
        "objects": len(tilings),
        "gaps": sum(t["gaps"] for t in tilings),
        "overlaps": sum(t["overlaps"] for t in tilings),
        "starts_at_zero": all(t["starts_at_zero"] for t in tilings) if tilings else None,
        "ends_at_frames": all(t["ends_at_frames"] for t in tilings) if tilings else None,
        "last_end": max((t["last_end"] for t in tilings if t["last_end"] is not None), default=None),
        "overrun_objects": sum(1 for t in tilings if t["overrun"]),
        "blocks_shorter_than_interp": sum(t["blocks_shorter_than_interp"] for t in tilings),
        "tile_dec": dict(sum((Counter(t["tile_dec"]) for t in tilings), Counter())),
        "unsorted_objects": sum(1 for t in tilings if not t["sorted"]),
    }
    ledger.identity = {"ok": ledger.identity_ok, "objects": {k: v["identity"] for k, v in ledger.per_object.items()}}
    return ledger
