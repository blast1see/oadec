"""ADM timecode <-> integer sample position.

Two notations are accepted (ITU-R BS.2076):
  * ``hh:mm:ss.zzzzz``        (BS.2076-1; five or more decimal digits)
  * ``hh:mm:ss.nnnnnSfffff``  (BS.2076-2 and later; ``n`` samples at ``f`` Hz
    inside the second)

The canonical timebase of the audit is the integer sample.  A decimal timecode
is mapped to the *nearest* sample (half up).  At 44.1, 48 and 96 kHz a five
decimal timecode always identifies one sample uniquely (the 10 us grid is
finer than half a sample period); at 192 kHz it does not -- see ``selftest``.
All arithmetic is exact (``fractions.Fraction`` or integers); no floats.
"""
from __future__ import annotations

import re
from dataclasses import dataclass
from fractions import Fraction

_DECIMAL = re.compile(r"^(\d{2}):(\d{2}):(\d{2})\.(\d{5,})$")
_SAMPLES = re.compile(r"^(\d{2}):(\d{2}):(\d{2})\.(\d+)S(\d+)$")

DECIMAL_UNIT = Fraction(1, 100_000)  # 10 us


@dataclass(frozen=True)
class Decoded:
    samples: int
    exact: bool
    notation: str
    decimal_units: int | None
    seconds: Fraction
    raw: str


def to_seconds(tc: str) -> Fraction:
    """The exact rational number of seconds a timecode string denotes."""
    m = _DECIMAL.match(tc)
    if m:
        hh, mm, ss, frac = m.groups()
        return Fraction(int(hh) * 3600 + int(mm) * 60 + int(ss)) + Fraction(int(frac), 10 ** len(frac))
    m = _SAMPLES.match(tc)
    if m:
        hh, mm, ss, num, den = m.groups()
        if int(den) == 0:
            raise ValueError(f"zero sample rate in timecode {tc!r}")
        return Fraction(int(hh) * 3600 + int(mm) * 60 + int(ss)) + Fraction(int(num), int(den))
    raise ValueError(f"not an ADM timecode: {tc!r}")


def decode(tc: str, fs: int) -> Decoded:
    """Nearest integer sample at ``fs`` for timecode ``tc``."""
    sec = to_seconds(tc)
    x = sec * fs
    samples = (x + Fraction(1, 2)).__floor__()
    if _DECIMAL.match(tc):
        notation = "bs2076-1-decimal"
        units = sec / DECIMAL_UNIT
        decimal_units = int(units) if units.denominator == 1 else None
    else:
        notation = "bs2076-2-samples"
        decimal_units = None
    return Decoded(samples, x.denominator == 1, notation, decimal_units, sec, tc)


def encode(samples: int, fs: int) -> str:
    """``hh:mm:ss.zzzzz`` with the fraction rounded half up to 10 us.

    This mirrors the convention oadec's writer uses (five digits, half away
    from zero) but with exact arithmetic; the writer rounds in f64.  Whether
    the two ever disagree on a real file is itself measured (every real
    timecode is re-encoded and compared).
    """
    if samples < 0:
        raise ValueError("negative sample position")
    total = Fraction(samples, fs)
    whole = total.__floor__()
    units = ((total - whole) / DECIMAL_UNIT + Fraction(1, 2)).__floor__()
    if units >= 100_000:
        whole += 1
        units = 0
    return f"{whole // 3600:02}:{(whole // 60) % 60:02}:{whole % 60:02}.{units:05}"


def selftest(fs: int, n_max: int) -> list[int]:
    """Sample positions in ``0..=n_max`` that do not survive encode->decode.

    Integer arithmetic equivalent of ``decode(encode(n))`` so that millions of
    positions can be checked quickly.
    """
    bad = []
    for n in range(n_max + 1):
        units = (2 * n * 100_000 + fs) // (2 * fs)          # half up
        back = (2 * units * fs + 100_000) // (2 * 100_000)  # nearest sample
        if back != n:
            bad.append(n)
    return bad
