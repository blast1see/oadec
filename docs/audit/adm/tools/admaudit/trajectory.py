"""What an object's parameter actually does over time, under each format's rules.

OAMD / DAMF (ETSI TS 103 420 V1.2.1 clause 5.6.2, figure 4): a property
update at ``start_sample`` with ``ramp_duration`` R interpolates from the
values in force to the new values, reaching them at ``start_sample + R``
("damf", start-anchored).  ``damf-d2`` is the end-anchored reading kept only
as a sensitivity variant.

ADM (ITU-R BS.2076-3 section 9.3 and figure A1-8): with ``jumpPosition=1`` and
``interpolationLength`` L the parameter is interpolated over the first L
seconds of the block and then held; L = 0 or absent is an instant jump; with
``jumpPosition=0`` the interpolation spans the whole block.

Both are the same shape -- a ramp that starts at the event -- which is why the
faithful image of a DAMF event is ``interpolationLength = ramp / fs``.
"""
from __future__ import annotations

import bisect
import copy
from dataclasses import dataclass, field

from .normalise import ObjEvent

AXES = {"x": 0, "y": 1, "z": 2}


def _target(e: ObjEvent, key: str) -> float:
    if key in AXES:
        v = e.pos[AXES[key]]
        return float(v) if v is not None else 0.0
    if key == "gain":
        return float(e.gain["lin"])
    if key in ("w", "d", "h"):
        return float(e.size[key])
    if key == "size":
        return float(e.size["w"])
    raise KeyError(key)


def _ramp(e: ObjEvent, semantics: str) -> int:
    if semantics.startswith("damf"):
        return int(e.ramp or 0)
    ip = e.interp or {}
    if ip.get("jump") == 1:
        return int(ip.get("len_samples") or 0) if ip.get("len_present") else 0
    return int(e.dur or 0)


def segments(events: list, semantics: str, key: str, carry_mid_ramp: bool = True) -> list:
    """``[(start, from_value, to_value, ramp)]`` in time order."""
    evs = sorted(events, key=lambda e: e.t)
    out = []
    cur = None
    for e in evs:
        target = _target(e, key)
        r = _ramp(e, semantics)
        start = e.t - r if semantics == "damf-d2" else e.t
        if cur is None:
            out.append((start, target, target, 0))
            cur = target
            continue
        prev_start, prev_from, prev_to, prev_r = out[-1]
        if carry_mid_ramp:
            frac = 1.0 if prev_r == 0 else min(max((start - prev_start) / prev_r, 0.0), 1.0)
            from_value = prev_from + (prev_to - prev_from) * frac
        else:
            from_value = prev_to
        out.append((start, from_value, target, r))
    return out


def value_from_segments(segs: list, starts: list, t: int) -> float:
    """Value at ``t`` given ``segments()`` output and its list of start times."""
    if not segs:
        raise ValueError("no events")
    if t < starts[0]:
        return segs[0][1]
    i = bisect.bisect_right(starts, t) - 1
    start, v0, v1, r = segs[i]
    if r <= 0:
        return v1
    frac = min(max((t - start) / r, 0.0), 1.0)
    return v0 + (v1 - v0) * frac


def value_at(events: list, t: int, semantics: str, key: str, carry_mid_ramp: bool = True) -> float:
    segs = segments(events, semantics, key, carry_mid_ramp)
    return value_from_segments(segs, [s[0] for s in segs], t)


@dataclass
class Probe:
    t: int
    axis: str
    v_damf: float
    v_adm: float
    e: float
    kind: str


@dataclass
class LossReport:
    probes: list = field(default_factory=list)
    max_e: dict = field(default_factory=dict)
    mean_e: dict = field(default_factory=dict)
    p95_e: dict = field(default_factory=dict)
    over: dict = field(default_factory=dict)
    n_probes: int = 0
    threshold: float = 0.01

    def to_json(self) -> dict:
        return {"max_e": self.max_e, "mean_e": self.mean_e, "p95_e": self.p95_e, "over_threshold": self.over, "threshold": self.threshold, "n_probes": self.n_probes}


def loss(damf_events: list, adm_events: list, frames: int | None, fractions=(0.25, 0.5, 0.75), keys=("x", "y", "z"), damf_semantics: str = "damf", threshold: float = 0.01, carry_mid_ramp: bool = True) -> LossReport:
    rep = LossReport(threshold=threshold)
    D = sorted(damf_events, key=lambda e: e.t)
    A = sorted(adm_events, key=lambda e: e.t)
    if not D or not A:
        return rep
    times = []
    for i, e in enumerate(D):
        nxt = D[i + 1].t if i + 1 < len(D) else (frames if frames is not None else e.t + 1536)
        gap = max(nxt - e.t, 0)
        for f in fractions:
            times.append((int(e.t + f * gap), "fraction"))
        r = int(e.ramp or 0)
        if 0 < r < gap:
            times.append((e.t + r, "damf-ramp-end"))
    for b in A:
        L = (b.interp or {}).get("len_samples") or 0
        if L and b.dur and L < b.dur:
            times.append((b.t + L, "adm-ramp-end"))
    seen = set()
    per_axis = {k: [] for k in keys}
    segs_d = {k: segments(D, damf_semantics, k, carry_mid_ramp) for k in keys}
    segs_a = {k: segments(A, "adm", k, carry_mid_ramp) for k in keys}
    starts_d = {k: [s[0] for s in segs_d[k]] for k in keys}
    starts_a = {k: [s[0] for s in segs_a[k]] for k in keys}
    for t, kind in sorted(times):
        for k in keys:
            if (t, k) in seen:
                continue
            seen.add((t, k))
            vd = value_from_segments(segs_d[k], starts_d[k], t)
            va = value_from_segments(segs_a[k], starts_a[k], t)
            e = abs(vd - va)
            rep.probes.append(Probe(t, k, vd, va, e, kind))
            per_axis[k].append(e)
    for k, es in per_axis.items():
        if not es:
            rep.max_e[k] = 0.0
            rep.mean_e[k] = 0.0
            rep.p95_e[k] = 0.0
            rep.over[k] = 0
            continue
        s = sorted(es)
        rep.max_e[k] = s[-1]
        rep.mean_e[k] = sum(s) / len(s)
        rep.p95_e[k] = s[min(len(s) - 1, int(0.95 * (len(s) - 1)))]
        rep.over[k] = sum(1 for e in es if e > threshold)
    rep.n_probes = len(rep.probes)
    return rep


def faithful_adm(damf_events: list, frames: int | None) -> list:
    """The ADM block list that would carry each DAMF ramp exactly (profile aside)."""
    D = sorted(damf_events, key=lambda e: e.t)
    out = []
    for i, e in enumerate(D):
        b = copy.deepcopy(e)
        nxt = D[i + 1].t if i + 1 < len(D) else frames
        b.dur = (nxt - e.t) if nxt is not None else None
        b.end = nxt
        L = 0 if i == 0 else int(e.ramp or 0)
        b.interp = {"jump": 1, "len_present": True, "len_s": None, "len_samples": L, "len_exact": True}
        b.ramp = None
        out.append(b)
    return out


def displacements(events: list, keys=("x", "y", "z")) -> dict:
    D = sorted(events, key=lambda e: e.t)
    gaps = [b.t - a.t for a, b in zip(D, D[1:])]
    disp = [max(abs(_target(b, k) - _target(a, k)) for k in keys) for a, b in zip(D, D[1:])]
    return {"gaps": gaps, "displacements": disp, "ramps": [int(e.ramp or 0) for e in D]}
