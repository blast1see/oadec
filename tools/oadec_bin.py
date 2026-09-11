#!/usr/bin/env python3
"""Where the decoder is, without asking the machine it was written on.

Several tools here carried an absolute path to one developer's build directory
as their default. That works on one machine and produces a confusing "file not
found" on every other, which is a poor way to greet someone reading the evidence
and trying to reproduce it.

Resolution order, first hit wins:

1. `$OADEC_BIN`, for a build that is somewhere else entirely.
2. `target/release/oadec[.exe]` beside this file's repository, which is what
   `cargo build --release` produces.
3. `target/debug/oadec[.exe]`, for someone who has only built once.
4. `oadec` on `PATH`, for an installed copy.

Raises with all four places named when none of them has it, because a tool that
runs for ten minutes and then reports nothing is worse than one that stops.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path

EXE = "oadec.exe" if os.name == "nt" else "oadec"


def candidates() -> list[Path]:
    root = Path(__file__).resolve().parent.parent
    out = []
    env = os.environ.get("OADEC_BIN")
    if env:
        out.append(Path(env))
    out.append(root / "target" / "release" / EXE)
    out.append(root / "target" / "debug" / EXE)
    found = shutil.which("oadec")
    if found:
        out.append(Path(found))
    return out


def find(required: bool = True) -> str:
    for p in candidates():
        if p.is_file():
            return str(p)
    if not required:
        return str(Path(__file__).resolve().parent.parent / "target" / "release" / EXE)
    raise SystemExit(
        "the oadec binary is not where any of these say it is:\n  "
        + "\n  ".join(str(p) for p in candidates())
        + "\nbuild it with `cargo build --release -p oadec-cli`, or point $OADEC_BIN at it"
    )


if __name__ == "__main__":
    print(find())
