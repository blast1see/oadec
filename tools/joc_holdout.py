#!/usr/bin/env python3
"""The held-out half of the JOC measurements: which groups of tracks a fit may
look at and which it may not.

The unit that is split is the group, never the track: everything made from the
same content or the same master is one group (every encoding, bit rate,
language and cut of a film, every clip of it, every episode of a series that
shares its mix, every encoding of one ADM master), and in doubt two things are
one group. A whole group goes to `development` or to `holdout`, about 60/40.
What anything was ever fitted on, the gate clips, the two outlying objects and
whatever is of unknown content stay in development.

The repository keeps only a salted digest of each group and its side
(`tools/joc_holdout.json`); no name of a film enters it. The names, the salt
and the map from an item to its group stay in the work folder.

    python tools/joc_holdout.py build --items <private items.json> --salt-file <salt>
        --seed <n> --out tools/joc_holdout.json --private-out <private map.json>
    python tools/joc_holdout.py check --list tools/joc_holdout.json
        [--map <private map.json> --salt-file <salt>]

`check` runs before every scoring: without the map it checks the list alone;
with it, that every item's group is recomputed from its name under the salt,
is on the list, is on one side only, and that every item that must be in
development is.

Private items: `[{"item": "library:001", "name": "<file or title>",
"group_key": "<optional, overrides the name>", "dev_reason": "<optional>"}]`.

Exit codes: 0 held, 1 a check failed, 2 bad arguments.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import re
import sys
from pathlib import Path

SCHEMA = 1
SIDES = ("development", "holdout")
HOLDOUT_SHARE = 0.40
# the development share of items a list may have and still be "about 60/40"
DEV_SHARE_RANGE = (0.50, 0.70)

_RES = re.compile(r"\b(2160p|1080p|720p|uhd|blu-?ray|remux|web-?dl|hybrid)\b")
_EPISODE = re.compile(r"\bs\d{1,2}e\d{1,3}\b")
_YEAR = re.compile(r"\b(19\d{2}|20[0-3]\d)\b")


def title_key(name: str) -> str:
    """The content a file or clip name stands for: its title, lower case, words
    only, without the year, the episode, the edition or the release."""
    s = Path(name).name.lower()
    s = re.sub(r"\.(mkv|mp4|m2ts|ec3|eac3|ac3|thd|mlp)$", "", s)
    s = re.sub(r"[._\-]+", " ", s)
    cut = len(s)
    for pattern in (_RES, _EPISODE):
        m = pattern.search(s)
        if m:
            cut = min(cut, m.start())
    head = s[:cut]
    years = list(_YEAR.finditer(head))
    if years:
        # the year is the last one before the release words: "1999 2019",
        # "robot runner 2049 2017"; a title that is only a year keeps it
        last = years[-1]
        if head[: last.start()].strip():
            head = head[: last.start()]
    return " ".join(re.sub(r"[^a-z0-9 ]+", "", head).split())


def group_id(salt: bytes, key: str) -> str:
    return hashlib.sha256(salt + b"\0" + key.encode("utf-8")).hexdigest()[:16]


def read_salt(path: Path) -> bytes:
    salt = path.read_bytes().strip()
    if len(salt) < 16:
        raise SystemExit(f"{path}: a salt of at least 16 bytes is needed")
    return salt


def build(args) -> int:
    items = json.loads(args.items.read_text(encoding="utf-8"))
    salt = read_salt(args.salt_file)
    groups: dict[str, dict] = {}
    for it in items:
        key = it.get("group_key") or title_key(it["name"])
        if not key:
            raise SystemExit(f"{it['item']}: no title in its name")
        g = groups.setdefault(key, {"id": group_id(salt, key), "items": [], "dev": []})
        g["items"].append(it)
        if it.get("dev_reason"):
            g["dev"].append(it["dev_reason"])
    total = len(items)
    side = {k: "development" for k, g in groups.items() if g["dev"]}
    free = sorted(k for k in groups if k not in side)
    random.Random(args.seed).shuffle(free)
    held = 0
    for k in free:
        n = len(groups[k]["items"])
        # into the holdout while it brings the holdout nearer its share
        if abs(held + n - HOLDOUT_SHARE * total) < abs(held - HOLDOUT_SHARE * total):
            side[k] = "holdout"
            held += n
        else:
            side[k] = "development"
    listing = sorted(({"group": g["id"], "side": side[k]} for k, g in groups.items()),
                     key=lambda r: r["group"])
    out = {
        "schema": SCHEMA,
        "what": "JOC measurement groups by content and common master; a group is a salted "
                "digest of its title, the names stay outside the repository (tools/joc_holdout.py)",
        "salt_sha256": hashlib.sha256(salt).hexdigest(),
        "seed": args.seed,
        "holdout_share_aimed_at": HOLDOUT_SHARE,
        "items": {"development": total - held, "holdout": held},
        "groups": listing,
    }
    args.out.write_text(json.dumps(out, indent=1) + "\n", encoding="utf-8")
    private = {"salt_sha256": out["salt_sha256"], "items": []}
    for k, g in groups.items():
        for it in g["items"]:
            private["items"].append({**it, "group_key": k, "group": g["id"], "side": side[k]})
    private["items"].sort(key=lambda r: r["item"])
    args.private_out.write_text(json.dumps(private, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"{len(groups)} groups, {total} items: {total - held} development, {held} holdout")
    return check_list(out, None, None)


def check_list(listing: dict, private: dict | None, salt: bytes | None) -> int:
    problems = []
    if listing.get("schema") != SCHEMA:
        problems.append(f"schema {listing.get('schema')}, {SCHEMA} expected")
    sides: dict[str, set] = {}
    for row in listing.get("groups", []):
        if row.get("side") not in SIDES:
            problems.append(f"group {row.get('group')}: side {row.get('side')!r}")
        if not re.fullmatch(r"[0-9a-f]{16}", str(row.get("group"))):
            problems.append(f"group {row.get('group')!r} is not a digest")
        sides.setdefault(row.get("group"), set()).add(row.get("side"))
    both = [g for g, s in sides.items() if len(s) > 1]
    if both:
        problems.append(f"{len(both)} groups on both sides: {both}")
    dupes = len(listing.get("groups", [])) - len(sides)
    if dupes:
        problems.append(f"{dupes} groups listed more than once")
    counts = listing.get("items", {})
    n = sum(counts.get(s, 0) for s in SIDES)
    if n:
        dev = counts.get("development", 0) / n
        if not DEV_SHARE_RANGE[0] <= dev <= DEV_SHARE_RANGE[1]:
            problems.append(f"development holds {dev:.0%} of the items, not about 60%")
    if private is not None:
        if private.get("salt_sha256") != listing.get("salt_sha256"):
            problems.append("the map was made with another salt than the list")
        if salt is not None and hashlib.sha256(salt).hexdigest() != listing.get("salt_sha256"):
            problems.append("the salt given is not the salt of the list")
        side_of = {g: next(iter(s)) for g, s in sides.items()}
        seen = {s: 0 for s in SIDES}
        for it in private["items"]:
            key = it.get("group_key") or title_key(it["name"])
            if "group_key" in it and it["group_key"] != key:
                problems.append(f"{it['item']}: group key differs")
            if salt is not None and group_id(salt, key) != it["group"]:
                problems.append(f"{it['item']}: its group does not come from its name under the salt")
            listed = side_of.get(it["group"])
            if listed is None:
                problems.append(f"{it['item']}: group not on the list")
                continue
            if listed != it["side"]:
                problems.append(f"{it['item']}: the map says {it['side']}, the list {listed}")
            if it.get("dev_reason") and listed != "development":
                problems.append(f"{it['item']}: must be in development ({it['dev_reason']})")
            seen[listed] += 1
        if seen != {s: counts.get(s) for s in SIDES}:
            problems.append(f"the map counts {seen}, the list {counts}")
        # two items of one title in two groups would split a group
        by_key: dict[str, set] = {}
        for it in private["items"]:
            by_key.setdefault(title_key(it["name"]), set()).add(it["group"])
        split = [k for k, gs in by_key.items() if len(gs) > 1 and k]
        if split:
            problems.append(f"{len(split)} titles in more than one group")
    for p in problems:
        print(f"FAIL {p}")
    if not problems:
        what = "list and map" if private is not None else "list"
        print(f"ok   {what}: {len(sides)} groups, no group on both sides, items {counts}")
    return 1 if problems else 0


def check(args) -> int:
    listing = json.loads(args.list.read_text(encoding="utf-8"))
    private = json.loads(args.map.read_text(encoding="utf-8")) if args.map else None
    salt = read_salt(args.salt_file) if args.salt_file else None
    return check_list(listing, private, salt)


def self_test() -> int:
    cases = {
        "1999.2019.UHD.BluRay.2160p.TrueHD.Atmos.7.1.DV.HEVC.REMUX-X.mkv": "1999",
        "Robot Runner 2049 2017 Hybrid 2160p UHD Blu-ray Remux.mkv": "robot runner 2049",
        "Show.Name.S01E03.Some.Title.2160p.WEB-DL.mkv": "show name",
        "Some Film 1994 Hybrid 2160p UHD Blu-ray Remux.mkv": "some film",
        "Some.Film.1994.UHD.BluRay.2160p.TrueHD.mkv": "some film",
    }
    bad = {k: (title_key(k), v) for k, v in cases.items() if title_key(k) != v}
    for k, (got, want) in bad.items():
        print(f"FAIL title_key({k!r}) = {got!r}, {want!r} expected")
    if not bad:
        print(f"ok   title_key: {len(cases)} cases")
    return 1 if bad else 0


def main() -> int:
    ap = argparse.ArgumentParser(description="Group-wise holdout of the JOC measurements.")
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("build")
    b.add_argument("--items", type=Path, required=True)
    b.add_argument("--salt-file", type=Path, required=True)
    b.add_argument("--seed", type=int, required=True)
    b.add_argument("--out", type=Path, required=True)
    b.add_argument("--private-out", type=Path, required=True)
    c = sub.add_parser("check")
    c.add_argument("--list", type=Path, default=Path(__file__).resolve().parent / "joc_holdout.json")
    c.add_argument("--map", type=Path)
    c.add_argument("--salt-file", type=Path)
    sub.add_parser("self-test")
    args = ap.parse_args()
    if args.cmd == "build":
        return build(args)
    if args.cmd == "check":
        return check(args)
    return self_test()


if __name__ == "__main__":
    sys.exit(main())
