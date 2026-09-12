"""Evidence envelopes: every stored measurement says what it is and what it proves."""
from __future__ import annotations

import json
import os

from .provenance import sha256_file, utc_now

EVIDENCE_CLASSES = ("SPEC", "REFERENCE", "MEASURED", "INFERRED", "IMPLEMENTATION CHOICE", "UNKNOWN")
RESULTS = ("PASS", "PASS-TOL", "PARTIAL", "FAIL", "N/I", "N/T", "UNK", None)


def _default(o):
    if isinstance(o, set):
        return sorted(o)
    if hasattr(o, "to_json"):
        return o.to_json()
    if hasattr(o, "__dict__"):
        return o.__dict__
    return str(o)


def write(directory: str, id_: str, payload: dict, *, title: str, topics: list, evidence_class: str,
          structural_result, semantic_result, classification: str, method: str, inputs: list,
          generated_by: str | None = None, tolerance=None) -> str:
    if evidence_class not in EVIDENCE_CLASSES:
        raise ValueError(f"evidence class {evidence_class!r}")
    for r in (structural_result, semantic_result):
        if r not in RESULTS:
            raise ValueError(f"result {r!r}")
    doc = {
        "id": id_, "title": title, "topics": list(topics), "evidence_class": evidence_class,
        "structural_result": structural_result, "semantic_result": semantic_result,
        "classification": classification, "method": method, "inputs": list(inputs),
        "tolerance": tolerance, "generated_by": generated_by, "utc": utc_now(),
    }
    for k, v in payload.items():
        if k in doc:
            raise ValueError(f"payload key {k!r} collides with the envelope")
        doc[k] = v
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, f"{id_}.json")
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        json.dump(doc, f, indent=2, default=_default, ensure_ascii=False)
        f.write("\n")
    return path


def read(path: str) -> dict:
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def manifest(directory: str, write_file: bool = True) -> dict:
    files = []
    for name in sorted(os.listdir(directory)):
        if name == "manifest.json" or not name.endswith((".json", ".csv")):
            continue
        p = os.path.join(directory, name)
        entry = {"file": name, "bytes": os.path.getsize(p), "sha256": sha256_file(p)}
        if name.endswith(".json"):
            try:
                d = read(p)
                entry["id"] = d.get("id")
                entry["title"] = d.get("title")
                entry["structural_result"] = d.get("structural_result")
                entry["semantic_result"] = d.get("semantic_result")
            except Exception:
                pass
        files.append(entry)
    m = {"utc": utc_now(), "files": files}
    if write_file:
        with open(os.path.join(directory, "manifest.json"), "w", encoding="utf-8", newline="\n") as f:
            json.dump(m, f, indent=2)
            f.write("\n")
    return m
