#!/usr/bin/env python3
"""Compare a decoded Atmos programme with the scene it was authored from.

Everything else this project measures is differential: oadec against truehdd,
oadec against Dolby. That answers "do two decoders agree" and never "is either
right". This asks the second question. A scene description names each object's
position and the samples at which it moves; `oadec atmos-author` writes a master
from it, Dolby's encoder encodes it, oadec decodes it back, and the recovered
metadata is compared with the numbers that went in.

Objects are identified by their audio, not by their index: an encoder's spatial
coding renumbers and reclusters freely, so each authored object carries a tone
at its own frequency, or a train of impulses at named samples, and the recovered
element that carries that signal is the one to compare against.

    python tools/ground_truth.py scene.json decoded --out truth.json
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from caf import read_caf  # noqa: E402

FIRST_OBJECT_ID = 10


def read_events(path: Path) -> dict[int, list[dict]]:
    """The metadata file as, per element id, its states in sample order.

    Later events carry only the fields that changed, so each state is the
    previous one updated.
    """
    text = path.read_text(encoding="utf-8", errors="replace")
    events: dict[int, list[dict]] = {}
    state: dict[int, dict] = {}
    cur_id = None
    cur: dict = {}

    def flush():
        if cur_id is None:
            return
        merged = dict(state.get(cur_id, {}))
        merged.update(cur)
        state[cur_id] = merged
        events.setdefault(cur_id, []).append(dict(merged))

    for line in text.splitlines():
        m = re.match(r"\s*-\s*ID:\s*(\d+)", line)
        if m:
            flush()
            cur_id = int(m.group(1))
            cur = {}
            continue
        m = re.match(r"\s*(\w+):\s*(.+?)\s*$", line)
        if m and cur_id is not None:
            key, raw = m.group(1), m.group(2)
            if raw.startswith("["):
                cur[key] = [float(v) for v in raw.strip("[]").split(",")]
            elif re.fullmatch(r"-?\d+", raw):
                cur[key] = int(raw)
            elif re.fullmatch(r"-?\d*\.\d+", raw):
                cur[key] = float(raw)
            elif raw in ("true", "false"):
                cur[key] = raw == "true"
            else:
                cur[key] = raw
    flush()
    return events


def identify(audio: np.ndarray, rate: int, scene: dict) -> dict[int, int]:
    """Which decoded channel carries which authored object.

    A tone is found by the strongest bin of its channel's spectrum; an impulse
    train by where the channel's energy is concentrated. Both are properties of
    the audio, so neither depends on the encoder keeping object order.
    """
    n = min(len(audio), rate * 4)
    spec = np.abs(np.fft.rfft(audio[:n] * np.hanning(n)[:, None], axis=0))
    freqs = np.fft.rfftfreq(n, 1.0 / rate)
    peak_hz = freqs[np.argmax(spec, axis=0)]
    energy = np.sqrt((audio ** 2).mean(0))
    used: set[int] = set()
    out: dict[int, int] = {}
    for i, obj in enumerate(scene["objects"]):
        sig = obj["signal"]
        best, score = None, -1e18
        for ch in range(audio.shape[1]):
            if ch in used or energy[ch] <= 0:
                continue
            if sig["kind"] == "tone":
                s = -abs(peak_hz[ch] - sig["hz"])
            elif sig["kind"] == "impulses":
                # an impulse train is flat in frequency and sparse in time
                x = np.abs(audio[:, ch])
                loud = x > 0.2 * x.max()
                s = -float(loud.mean()) * 1e6
            else:
                s = -1e17
            if s > score:
                best, score = ch, s
        if best is not None:
            used.add(best)
            out[i] = best
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("scene")
    ap.add_argument("base", help="decoded DAMF base name, without .atmos")
    ap.add_argument("--out", required=True)
    ap.add_argument("--tolerance", type=float, default=0.05,
                    help="position tolerance in room units")
    a = ap.parse_args()

    scene = json.load(open(a.scene, encoding="utf-8"))
    rate = scene.get("sample_rate", 48000)
    base = Path(a.base)
    audio, fmt = read_caf(str(base) + ".atmos.audio")
    events = read_events(Path(str(base) + ".atmos.metadata"))

    # the bed occupies the first channels; the objects follow
    bed = len(scene.get("bed", ["LFE"]))
    objects_audio = audio[:, bed:]
    mapping = identify(objects_audio, rate, scene)

    rows = []
    for i, obj in enumerate(scene["objects"]):
        ch = mapping.get(i)
        row = {"authored": obj["name"], "signal": obj["signal"], "decoded_channel": ch}
        if ch is None:
            row["verdict"] = "not identified"
            rows.append(row)
            continue
        eid = FIRST_OBJECT_ID + ch
        seq = events.get(eid, [])
        row["element_id"] = eid
        row["recovered_events"] = len(seq)
        checks = []
        for ev in obj["events"]:
            # the state in force at the authored sample
            at = [e for e in seq if e.get("samplePos", 0) <= ev["sample"]]
            got = at[-1] if at else None
            pos = got.get("pos") if got else None
            err = (max(abs(p - q) for p, q in zip(pos, ev["pos"]))
                   if pos and len(pos) == 3 else None)
            # the nearest recovered event to the authored one
            near = min((e for e in seq), key=lambda e: abs(e.get("samplePos", 0) - ev["sample"]),
                       default=None)
            checks.append({
                "authored_sample": ev["sample"],
                "authored_pos": ev["pos"],
                "recovered_pos": pos,
                "position_error": None if err is None else round(err, 6),
                "nearest_event_sample": None if near is None else near.get("samplePos"),
                "event_offset_samples": (None if near is None
                                         else near.get("samplePos", 0) - ev["sample"]),
                "position_matches": bool(err is not None and err <= a.tolerance),
            })
        # A moving object is judged differently, because the encoder is free to
        # resample a trajectory onto its own grid: what can be asked of the
        # decoder is that every authored position comes back, and what is then
        # reported is where in time the encoder put it.
        if len(obj["events"]) > 1:
            for c in checks:
                near = min(
                    (e for e in seq if e.get("pos")),
                    key=lambda e: max(abs(p - q) for p, q in zip(e["pos"], c["authored_pos"])),
                    default=None)
                if near is None:
                    continue
                err = max(abs(p - q) for p, q in zip(near["pos"], c["authored_pos"]))
                c["closest_recovered_pos"] = near["pos"]
                c["closest_position_error"] = round(err, 6)
                c["closest_at_sample"] = near.get("samplePos")
                c["encoder_time_shift_samples"] = near.get("samplePos", 0) - c["authored_sample"]
                c["position_matches"] = err <= a.tolerance
        row["checks"] = checks
        row["verdict"] = ("PASS" if all(c["position_matches"] for c in checks)
                          else "POSITION DIFFERS")
        row["static"] = len(obj["events"]) == 1
        rows.append(row)

    identified = sum(1 for r in rows if r.get("decoded_channel") is not None)
    passed = sum(1 for r in rows if r.get("verdict") == "PASS")
    offsets = [c["event_offset_samples"] for r in rows for c in r.get("checks", [])
               if c["event_offset_samples"] is not None]
    errors = [(c.get("closest_position_error") if not r.get("static", True)
               else c["position_error"])
              for r in rows for c in r.get("checks", [])
              if (c.get("closest_position_error") if not r.get("static", True)
                  else c["position_error"]) is not None]
    shifts = [c["encoder_time_shift_samples"] for r in rows for c in r.get("checks", [])
              if c.get("encoder_time_shift_samples") is not None]
    summary = {
        "scene": a.scene,
        "decoded": str(base),
        "sample_rate": fmt["sr"],
        "decoded_channels": int(audio.shape[1]),
        "authored_objects": len(scene["objects"]),
        "identified_by_signal": identified,
        "objects_with_every_position_recovered": passed,
        "static_objects": sum(1 for r in rows if r.get("static")),
        "static_positions_exact": sum(
            1 for r in rows if r.get("static")
            for c in r.get("checks", []) if c["position_error"] == 0.0),
        "worst_position_error": None if not errors else max(errors),
        "event_offsets_samples": sorted(set(offsets)),
        "encoder_time_shift_samples": sorted(set(shifts)),
    }
    json.dump({"summary": summary, "objects": rows}, open(a.out, "w"), indent=1)
    print(json.dumps(summary, indent=1))
    for r in rows:
        print(f"  {r['authored'][:34]:34s} -> channel {r.get('decoded_channel')} "
              f"{r['verdict']}")
        for c in r.get("checks", []):
            if "closest_position_error" in c:
                print(f"      authored {c['authored_pos']} at {c['authored_sample']:>7}: "
                      f"recovered {c['closest_recovered_pos']} at "
                      f"{c['closest_at_sample']} (err {c['closest_position_error']}, "
                      f"encoder shifted it {c['encoder_time_shift_samples']:+d})")
            else:
                print(f"      authored {c['authored_pos']} at {c['authored_sample']:>7}: "
                      f"recovered {c['recovered_pos']} err {c['position_error']} "
                      f"event offset {c['event_offset_samples']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
