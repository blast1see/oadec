"""Capture the --help output of the Dolby command-line tools into docs/dolby-tools.md."""
from __future__ import annotations

import subprocess
import time
from pathlib import Path

DEE57 = Path(r"C:\Program Files\Dolby Media Encoder\resources\dee")
CMDS = [
    [r"C:\dee\dee.exe", "--help"],
    [r"C:\dee\dee.exe", "--print-stages"],
    [r"C:\dee\atmos_info.exe", "--help"],
    [r"C:\dee\bwf_info.exe", "--help"],
    [str(DEE57 / "dee_dthd_encoder.exe"), "--help"],
    [str(DEE57 / "dee_dthd_encoder.exe"), "--morehelp", "input-format"],
    [str(DEE57 / "dee_dthd_encoder.exe"), "--morehelp", "presentation"],
    [str(DEE57 / "dee_ddpjoc_encoder.exe"), "--help"],
    [str(DEE57 / "dee_ddpjoc_encoder.exe"), "--morehelp", "input-format"],
    [str(DEE57 / "dee_ddpjoc_encoder.exe"), "--morehelp", "examples"],
    [str(DEE57 / "atmos_info.exe"), "--help"],
    [r"C:\Program Files\Dolby\Dolby Reference Player\drp.exe", "--help"],
    [r"C:\Program Files\Dolby\Dolby Reference Player\drp.exe", "--version"],
]

out = ["# Dolby tool command lines on the development machine", "",
       f"Captured {time.strftime('%Y-%m-%d')} by `tools/capture_dolby_help.py`. These outputs",
       "are the ground truth for the flags used by the verification scripts.", ""]
for cmd in CMDS:
    res = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    text = (res.stdout + res.stderr).replace("\r", "").strip()
    out += [f"## `{Path(cmd[0]).name} {' '.join(cmd[1:])}`", "", "```text", text[:12000], "```", ""]
Path("docs/dolby-tools.md").write_text("\n".join(out), encoding="utf-8")
print("wrote docs/dolby-tools.md", sum(len(x) for x in out), "chars")
