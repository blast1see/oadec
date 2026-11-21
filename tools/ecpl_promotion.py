#!/usr/bin/env python3
"""Whether an enhanced coupling model may be promoted to a hidden flag.

    python tools/ecpl_promotion.py CARRIER.json [CARRIER.json ...] [--json OUT]
    python tools/ecpl_promotion.py --self-test

A model (`oadec_eac3::ecpl::Model`, decoded by the example
`ecpl_model_decode`) is laid over the default, amplitude-only, reading of
enhanced coupling. It is promoted to `--ecpl-model <name>` -- never to the
default -- only when all of this holds (`docs/eac3.md`):

1. on every carrier, the decode with the model off is byte-identical to
   the default decode (`off_identical`);
2. on each of at least two carriers that took no part in fitting the model
   (`role: holdout`), of the (amplitude code, coupled channel) cells:
   - in at least 95 per cent the model is closer to Dolby's decode than the
     amplitude-only reading (`model_db > amp_only_db`, both the SNR against
     Dolby over the coupling band of the same frames), and
   - in none is it further by more than 0.5 dB;
3. the holdout carriers include one 2/0 programme and one whose coupling
   channel is coded without the hybrid transform (`cpl_aht: false`), and
   none of them was encoded from a source signal a fitting carrier was
   encoded from.

The fitting carriers are reported by the same figures, and do not decide.

One file a carrier, written by the measurement (Durak 2):

    {"carrier": "st128-ffmpeg-noise",       a name
     "role": "fit" | "holdout",
     "layout": "2.0" | "5.1",
     "cpl_aht": false,                       the coupling channel's AHT
     "source": "noise-bands-A",              the source signal's name
     "off_identical": true,
     "cells": [{"code": 8, "channel": "Ls",
                "amp_only_db": 0.30, "model_db": 2.10}, ...]}

The exit code is the verdict: 0 promoted, 1 not, 2 the input will not do.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

SHARE = 0.95
WORST_DB = 0.5
MIN_HOLDOUT = 2


class Bad(Exception):
    pass


def load(path: Path) -> dict:
    try:
        c = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise Bad(f"{path}: {e}") from e
    return check(c, str(path))


def check(c: dict, where: str) -> dict:
    for key, kind in (("carrier", str), ("role", str), ("layout", str), ("cpl_aht", bool),
                      ("source", str), ("off_identical", bool), ("cells", list)):
        if not isinstance(c.get(key), kind):
            raise Bad(f"{where}: '{key}' missing or not {kind.__name__}")
    if c["role"] not in ("fit", "holdout"):
        raise Bad(f"{where}: role is '{c['role']}', not fit or holdout")
    if not c["cells"]:
        raise Bad(f"{where}: no cells")
    seen = set()
    for cell in c["cells"]:
        key = (cell.get("code"), cell.get("channel"))
        if key in seen:
            raise Bad(f"{where}: cell {key} twice")
        seen.add(key)
        for k in ("amp_only_db", "model_db"):
            if not isinstance(cell.get(k), (int, float)):
                raise Bad(f"{where}: cell {key} has no number '{k}'")
    return c


def judge(c: dict) -> dict:
    gains = [cell["model_db"] - cell["amp_only_db"] for cell in c["cells"]]
    better = sum(1 for g in gains if g > 0)
    worst = min(gains)
    share = better / len(gains)
    return {
        "carrier": c["carrier"], "role": c["role"], "layout": c["layout"],
        "cpl_aht": c["cpl_aht"], "source": c["source"], "cells": len(gains),
        "better": better, "share": round(share, 4), "worst_db": round(worst, 3),
        "median_db": round(sorted(gains)[len(gains) // 2], 3),
        "off_identical": c["off_identical"],
        "passes": share >= SHARE and worst >= -WORST_DB,
    }


def verdict(carriers: list[dict]) -> tuple[bool, dict]:
    rows = [judge(c) for c in carriers]
    hold = [r for r in rows if r["role"] == "holdout"]
    fit_sources = {r["source"] for r in rows if r["role"] == "fit"}
    reasons = []
    if not all(r["off_identical"] for r in rows):
        reasons.append("off is not byte-identical on: " + ", ".join(r["carrier"] for r in rows if not r["off_identical"]))
    leaked = [r["carrier"] for r in hold if r["source"] in fit_sources]
    if leaked:
        reasons.append("holdout encoded from a fitting source: " + ", ".join(leaked))
    clean = [r for r in hold if r["source"] not in fit_sources]
    if len(clean) < MIN_HOLDOUT:
        reasons.append(f"{len(clean)} holdout carriers, {MIN_HOLDOUT} needed")
    if not any(r["layout"] == "2.0" for r in clean):
        reasons.append("no 2/0 holdout carrier")
    if not any(not r["cpl_aht"] for r in clean):
        reasons.append("no holdout carrier without AHT on the coupling channel")
    failing = [r["carrier"] for r in clean if not r["passes"]]
    if failing:
        reasons.append("holdout carriers that do not pass: " + ", ".join(failing))
    ok = not reasons
    return ok, {"promoted": ok, "reasons": reasons, "share_needed": SHARE,
                "worst_allowed_db": -WORST_DB, "carriers": rows}


def self_test() -> int:
    def carrier(name, role, layout, aht, source, deltas, off=True):
        return {"carrier": name, "role": role, "layout": layout, "cpl_aht": aht,
                "source": source, "off_identical": off,
                "cells": [{"code": i, "channel": "L", "amp_only_db": 1.0, "model_db": 1.0 + d}
                          for i, d in enumerate(deltas)]}

    good = [0.3] * 40
    cases = {
        "promoted": ([carrier("a", "fit", "5.1", True, "s1", good),
                      carrier("b", "holdout", "2.0", True, "s2", good),
                      carrier("c", "holdout", "5.1", False, "s3", good)], True),
        "one cell 0.6 dB worse": ([carrier("b", "holdout", "2.0", True, "s2", good[:-1] + [-0.6]),
                                   carrier("c", "holdout", "5.1", False, "s3", good)], False),
        "one cell 0.5 dB worse is allowed": ([carrier("b", "holdout", "2.0", True, "s2", good[:-1] + [-0.5]),
                                              carrier("c", "holdout", "5.1", False, "s3", good)], True),
        "94 per cent better": ([carrier("b", "holdout", "2.0", True, "s2", [0.3] * 47 + [-0.1] * 3),
                                carrier("c", "holdout", "5.1", False, "s3", good)], False),
        "95 per cent better": ([carrier("b", "holdout", "2.0", True, "s2", [0.3] * 38 + [-0.1] * 2),
                                carrier("c", "holdout", "5.1", False, "s3", good)], True),
        "a tie is not better": ([carrier("b", "holdout", "2.0", True, "s2", [0.0] * 40),
                                 carrier("c", "holdout", "5.1", False, "s3", good)], False),
        "no stereo holdout": ([carrier("b", "holdout", "5.1", True, "s2", good),
                               carrier("c", "holdout", "5.1", False, "s3", good)], False),
        "no AHT-less holdout": ([carrier("b", "holdout", "2.0", True, "s2", good),
                                 carrier("c", "holdout", "5.1", True, "s3", good)], False),
        "holdout from a fitting source": ([carrier("a", "fit", "5.1", True, "s2", good),
                                           carrier("b", "holdout", "2.0", True, "s2", good),
                                           carrier("c", "holdout", "5.1", False, "s3", good)], False),
        "one holdout": ([carrier("b", "holdout", "2.0", False, "s2", good)], False),
        "off not identical": ([carrier("a", "fit", "5.1", True, "s1", good, off=False),
                               carrier("b", "holdout", "2.0", True, "s2", good),
                               carrier("c", "holdout", "5.1", False, "s3", good)], False),
        "a failing fit carrier does not decide": ([carrier("a", "fit", "5.1", True, "s1", [-3.0] * 40),
                                                   carrier("b", "holdout", "2.0", True, "s2", good),
                                                   carrier("c", "holdout", "5.1", False, "s3", good)], True),
    }
    wrong = 0
    for name, (carriers, want) in cases.items():
        got, out = verdict([check(c, name) for c in carriers])
        mark = "ok" if got == want else "WRONG"
        wrong += got != want
        print(f"{mark:5} {name}: {'promoted' if got else 'not'} {out['reasons']}")
    for bad in ({}, {"carrier": "x", "role": "test", "layout": "2.0", "cpl_aht": True, "source": "s",
                     "off_identical": True, "cells": [{"code": 0, "channel": "L", "amp_only_db": 0, "model_db": 0}]}):
        try:
            check(bad, "bad")
            print("WRONG a bad carrier was taken")
            wrong += 1
        except Bad as e:
            print(f"ok    refused: {e}")
    print(f"self-test: {'pass' if not wrong else f'{wrong} wrong'}")
    return 1 if wrong else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("carriers", nargs="*", type=Path)
    ap.add_argument("--json", type=Path)
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if not a.carriers:
        ap.error("no carrier files")
    try:
        carriers = [load(p) for p in a.carriers]
    except Bad as e:
        print(f"ecpl_promotion: {e}", file=sys.stderr)
        return 2
    ok, out = verdict(carriers)
    for r in out["carriers"]:
        print(f"{r['role']:8} {r['carrier']:28} {r['layout']} aht={str(r['cpl_aht']):5} "
              f"better {r['better']}/{r['cells']} ({100 * r['share']:.1f} %) worst {r['worst_db']:+.2f} dB "
              f"median {r['median_db']:+.2f} dB off-identical={r['off_identical']} {'PASS' if r['passes'] else 'fail'}")
    print("PROMOTED" if ok else "NOT PROMOTED: " + "; ".join(out["reasons"]))
    if a.json:
        a.json.write_text(json.dumps(out, indent=1) + "\n", encoding="utf-8")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
