"""Who produced what, with which binary, in which environment."""
from __future__ import annotations

import datetime as _dt
import hashlib
import importlib
import os
import platform
import subprocess
import sys
import time
from dataclasses import dataclass, field


def utc_now() -> str:
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path: str, block: int = 1 << 24) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            b = f.read(block)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def file_record(path: str) -> dict:
    st = os.stat(path)
    return {"path": os.path.abspath(path), "bytes": st.st_size, "sha256": sha256_file(path),
            "mtime": _dt.datetime.fromtimestamp(st.st_mtime, _dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")}


def env_record() -> dict:
    packages = {}
    for name in ("numpy", "lxml", "scipy", "ear"):
        try:
            mod = importlib.import_module(name)
            packages[name] = getattr(mod, "__version__", None) or _dist_version(name)
        except Exception:
            packages[name] = None
    return {
        "utc": utc_now(),
        "python": sys.version.split()[0],
        "executable": sys.executable,
        "platform": platform.platform(),
        "machine": platform.machine(),
        "packages": packages,
    }


def _dist_version(name: str):
    try:
        from importlib import metadata
        return metadata.version(name)
    except Exception:
        return None


def tool_record(path: str, version_args: list | None = None, timeout: float = 60) -> dict:
    rec = file_record(path)
    if version_args:
        try:
            p = subprocess.run([path] + list(version_args), capture_output=True, text=True, timeout=timeout, errors="replace")
            rec["version"] = (p.stdout or p.stderr).strip().splitlines()[0] if (p.stdout or p.stderr).strip() else None
        except Exception as e:  # pragma: no cover
            rec["version"] = f"error: {e}"
    return rec


def git_record(repo: str) -> dict:
    def g(*args):
        return subprocess.run(["git", "-C", repo] + list(args), capture_output=True, text=True).stdout.strip()
    return {"head": g("rev-parse", "HEAD"), "branch": g("branch", "--show-current"), "describe": g("describe", "--always", "--dirty"),
            "status": g("status", "--short")[:4000]}


@dataclass
class RunRecord:
    argv: list
    cwd: str | None
    exit_code: int | None
    seconds: float
    stdout: str
    stderr: str
    scrubbed: list = field(default_factory=list)
    utc: str = ""
    timed_out: bool = False

    def to_json(self, keep: int = 20000) -> dict:
        return {
            "argv": self.argv, "cwd": self.cwd, "exit_code": self.exit_code, "seconds": round(self.seconds, 3),
            "stdout_sha256": hashlib.sha256(self.stdout.encode("utf-8", "replace")).hexdigest(),
            "stdout_tail": self.stdout[-keep:], "stderr_tail": self.stderr[-keep:],
            "scrubbed_env": self.scrubbed, "utc": self.utc, "timed_out": self.timed_out,
        }


def run(argv: list, cwd: str | None = None, scrub: tuple = ("OADEC_",), timeout: float | None = None, env_extra: dict | None = None) -> RunRecord:
    env = dict(os.environ)
    scrubbed = [k for k in env if any(k.startswith(p) for p in scrub)]
    for k in scrubbed:
        del env[k]
    if env_extra:
        env.update(env_extra)
    t0 = time.monotonic()
    utc = utc_now()
    try:
        p = subprocess.run([str(a) for a in argv], cwd=cwd, capture_output=True, text=True, errors="replace", env=env, timeout=timeout)
        return RunRecord([str(a) for a in argv], cwd, p.returncode, time.monotonic() - t0, p.stdout, p.stderr, scrubbed, utc)
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
        err = e.stderr.decode("utf-8", "replace") if isinstance(e.stderr, bytes) else (e.stderr or "")
        return RunRecord([str(a) for a in argv], cwd, None, time.monotonic() - t0, out, err, scrubbed, utc, True)
