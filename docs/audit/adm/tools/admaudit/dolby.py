"""Wrappers around the Dolby tools present on this workstation.

Every call goes through ``provenance.run`` so that the exact command, exit
code and log are recorded; the log is scanned for warnings and errors so that
"the tool accepted the file" and "the tool accepted the file with a warning"
never collapse into one result.
"""
from __future__ import annotations

import os
import re

from . import provenance

CONVERSION_TOOL = r"C:\Program Files\Dolby\Dolby Atmos Conversion Tool\cmdline_atmos_conversion_tool.exe"
DEE = r"C:\dee\dee.exe"
DEE_DIR = r"C:\dee"
ATMOS_INFO = {
    "5.7.2": r"C:\Program Files\Dolby Media Encoder\resources\dee\atmos_info.exe",
    "1.1": r"C:\dee\atmos_info.exe",
}
BWF_INFO = r"C:\dee\bwf_info.exe"
DEE_TEMP = r"E:\oadec-work\audit\adm\work\dee-tmp"

_LEVEL = re.compile(r"\b(WARNING|WARN|ERROR|FATAL)\b\s*:?", re.I)
_FAILED = re.compile(r"\b(failed|failure|rejected|not supported|unsupported|invalid|cannot|could not)\b", re.I)


def tools_present() -> dict:
    return {name: os.path.isfile(p) for name, p in [("conversion_tool", CONVERSION_TOOL), ("dee", DEE), ("atmos_info_5.7.2", ATMOS_INFO["5.7.2"]), ("atmos_info_1.1", ATMOS_INFO["1.1"]), ("bwf_info", BWF_INFO)]}


def log_findings(text: str) -> list:
    out = []
    for line in text.splitlines():
        m = _LEVEL.search(line)
        if m:
            lvl = m.group(1).upper()
            out.append({"level": "ERROR" if lvl in ("ERROR", "FATAL") else "WARNING", "line": line.strip()})
        elif _FAILED.search(line) and not line.lstrip().startswith(("INFO", "[")) or ("failed" in line.lower() and "validation" in line.lower()):
            out.append({"level": "ERROR", "line": line.strip()})
    return out


# ------------------------------------------------------------------ Dolby Atmos Conversion Tool

def conversion_tool_argv(master: str, outdir: str, fmt: str) -> list:
    return [CONVERSION_TOOL, "-i", master, "-o", outdir, "-f", fmt]


def conversion_tool(master: str, outdir: str, fmt: str, timeout: float | None = None) -> dict:
    os.makedirs(outdir, exist_ok=True)
    rec = provenance.run(conversion_tool_argv(master, outdir, fmt), timeout=timeout)
    outputs = [provenance.file_record(os.path.join(outdir, n)) for n in sorted(os.listdir(outdir))]
    return {"run": rec.to_json(), "findings": log_findings(rec.stdout + "\n" + rec.stderr), "outputs": outputs}


# ------------------------------------------------------------------ Dolby Encoding Engine

def dee_convert_job_xml(in_dir: str, in_name: str, out_dir: str, out_name: str, target: str, temp_dir: str) -> str:
    return f"""<?xml version="1.0"?>
<job_config>
  <input>
    <audio>
      <atmos_mezz version="1">
        <file_name>{in_name}</file_name>
        <timecode_frame_rate>not_indicated</timecode_frame_rate>
        <offset>auto</offset>
        <ffoa>auto</ffoa>
        <storage>
          <local>
            <path>{in_dir}</path>
          </local>
        </storage>
      </atmos_mezz>
    </audio>
  </input>
  <filter>
    <audio>
      <convert_atmos_mezz version="2">
        <target_frame_rate>auto</target_frame_rate>
        <ffoa>auto</ffoa>
        <verbosity>normal</verbosity>
        <timecode_frame_rate>not_indicated</timecode_frame_rate>
        <start>first_frame_of_action</start>
        <end>end_of_file</end>
        <time_base>file_position</time_base>
        <prepend_silence_duration>0</prepend_silence_duration>
        <append_silence_duration>0</append_silence_duration>
        <target_format>{target}</target_format>
      </convert_atmos_mezz>
    </audio>
  </filter>
  <output>
    <atmos_mezz version="1">
      <file_name>{out_name}</file_name>
      <storage>
        <local>
          <path>{out_dir}</path>
        </local>
      </storage>
    </atmos_mezz>
  </output>
  <misc>
    <temp_dir>
      <clean_temp>true</clean_temp>
      <path>{temp_dir}</path>
    </temp_dir>
  </misc>
</job_config>
"""


def dee_convert(master: str, out_dir: str, out_name: str, target: str = "adm", temp_dir: str = DEE_TEMP, timeout: float | None = None) -> dict:
    """DAMF (or ADM) master -> ``target`` via DEE's ``convert_atmos_mezz`` filter."""
    master = os.path.abspath(master)
    out_dir = os.path.abspath(out_dir)
    os.makedirs(out_dir, exist_ok=True)
    os.makedirs(temp_dir, exist_ok=True)
    xml = dee_convert_job_xml(os.path.dirname(master), os.path.basename(master), out_dir, out_name, target, temp_dir)
    job = os.path.join(out_dir, f"convert-{target}.xml")
    with open(job, "w", encoding="utf-8", newline="\n") as f:
        f.write(xml)
    rec = provenance.run([DEE, "--xml", job], cwd=DEE_DIR, timeout=timeout)
    with open(os.path.join(out_dir, f"convert-{target}.log"), "w", encoding="utf-8", newline="\n") as f:
        f.write(rec.stdout)
        f.write(rec.stderr)
    outputs = [provenance.file_record(os.path.join(out_dir, n)) for n in sorted(os.listdir(out_dir)) if not n.startswith("convert-")]
    return {"run": rec.to_json(), "job": job, "findings": log_findings(rec.stdout + "\n" + rec.stderr), "outputs": outputs}


# ------------------------------------------------------------------ validators

def atmos_info_argv(path: str, validate: bool = True, which: str = "5.7.2") -> list:
    exe = ATMOS_INFO[which]
    if which == "5.7.2":
        return [exe, "-i", path, "--validate", "1" if validate else "0"]
    return [exe, "-i", path] + ([] if validate else ["-s"])


def atmos_info(path: str, validate: bool = True, which: str = "5.7.2", timeout: float | None = 1800) -> dict:
    rec = provenance.run(atmos_info_argv(path, validate, which), timeout=timeout)
    return {"run": rec.to_json(), "findings": log_findings(rec.stdout + "\n" + rec.stderr), "which": which, "validate": validate}


def bwf_info(path: str, timeout: float | None = 1800) -> dict:
    rec = provenance.run([BWF_INFO, "-i", path], timeout=timeout)
    return {"run": rec.to_json(), "findings": log_findings(rec.stdout + "\n" + rec.stderr)}
