"""Sample-exact comparison of PCM tracks, track pairing and lag search."""
from __future__ import annotations

import hashlib
from dataclasses import dataclass

import numpy as np


def sha256_samples(a: np.ndarray) -> str:
    """Hash of the samples as little-endian int32 -- container independent."""
    return hashlib.sha256(np.ascontiguousarray(a, dtype="<i4").tobytes()).hexdigest()


@dataclass
class TrackCompare:
    samples_a: int
    samples_b: int
    length_delta: int
    compared: int
    differing: int
    first_diff: int | None
    max_abs: int
    rms_error: float
    peak_a: int
    peak_b: int
    rms_a: float
    rms_b: float
    silence_pct_a: float
    silence_pct_b: float
    corr: float | None
    sha256_a: str
    sha256_b: str
    identical: bool

    def to_json(self) -> dict:
        return dict(self.__dict__)


def _rms(x: np.ndarray) -> float:
    return float(np.sqrt(np.mean(x.astype(np.float64) ** 2))) if x.size else 0.0


def compare_tracks(a: np.ndarray, b: np.ndarray) -> TrackCompare:
    a = np.asarray(a, dtype=np.int64)
    b = np.asarray(b, dtype=np.int64)
    n = min(a.size, b.size)
    d = a[:n] - b[:n]
    nz = np.nonzero(d)[0]
    differing = int(nz.size)
    first = int(nz[0]) if nz.size else None
    if a.size != b.size:
        differing += abs(a.size - b.size)
        if first is None:
            first = n
    with np.errstate(invalid="ignore", divide="ignore"):
        corr = None
        if n > 1 and a[:n].std() > 0 and b[:n].std() > 0:
            corr = float(np.corrcoef(a[:n].astype(np.float64), b[:n].astype(np.float64))[0, 1])
    return TrackCompare(
        samples_a=int(a.size), samples_b=int(b.size), length_delta=int(b.size - a.size), compared=int(n),
        differing=differing, first_diff=first, max_abs=int(np.max(np.abs(d))) if n else 0,
        rms_error=_rms(d), peak_a=int(np.max(np.abs(a))) if a.size else 0, peak_b=int(np.max(np.abs(b))) if b.size else 0,
        rms_a=_rms(a), rms_b=_rms(b),
        silence_pct_a=float(100.0 * np.mean(a == 0)) if a.size else 100.0,
        silence_pct_b=float(100.0 * np.mean(b == 0)) if b.size else 100.0,
        corr=corr, sha256_a=sha256_samples(a), sha256_b=sha256_samples(b),
        identical=(a.size == b.size and differing == 0),
    )


def distance_matrix(tracks_a: list, tracks_b: list) -> np.ndarray:
    """RMS difference between every track of A and every track of B."""
    m = np.zeros((len(tracks_a), len(tracks_b)), dtype=np.float64)
    for i, a in enumerate(tracks_a):
        a64 = np.asarray(a, dtype=np.float64)
        for j, b in enumerate(tracks_b):
            b64 = np.asarray(b, dtype=np.float64)
            n = min(a64.size, b64.size)
            m[i, j] = np.sqrt(np.mean((a64[:n] - b64[:n]) ** 2)) if n else np.inf
    return m


@dataclass
class Match:
    i: int
    j: int
    distance: float
    runner_up: float
    margin: float


def best_matches(m: np.ndarray) -> list:
    out = []
    for i in range(m.shape[0]):
        row = m[i]
        order = np.argsort(row)
        j = int(order[0])
        runner = float(row[order[1]]) if row.size > 1 else float("inf")
        out.append(Match(i, j, float(row[j]), runner, runner - float(row[j])))
    return out


def permutation_is_identity(matches: list) -> bool:
    return all(mt.i == mt.j for mt in matches)


def lag(a: np.ndarray, b: np.ndarray, max_lag: int = 4096) -> int:
    """Lag (in samples) by which ``b`` trails ``a`` (positive: b is later)."""
    a = np.asarray(a, dtype=np.float64)
    b = np.asarray(b, dtype=np.float64)
    n = min(a.size, b.size)
    a = a[:n] - a[:n].mean()
    b = b[:n] - b[:n].mean()
    best, best_v = 0, -np.inf
    for k in range(-max_lag, max_lag + 1):
        if k >= 0:
            v = np.dot(a[: n - k], b[k:]) if n - k > 0 else -np.inf
        else:
            v = np.dot(a[-k:], b[: n + k]) if n + k > 0 else -np.inf
        if v > best_v:
            best, best_v = k, v
    return best


class _StreamAcc:
    def __init__(self):
        self.n = 0
        self.differing = 0
        self.first = None
        self.max_abs = 0
        self.sq_err = 0.0
        self.peak_a = 0
        self.peak_b = 0
        self.sq_a = 0.0
        self.sq_b = 0.0
        self.zero_a = 0
        self.zero_b = 0
        self.sum_a = 0.0
        self.sum_b = 0.0
        self.sum_ab = 0.0
        self.psq_a = 0.0
        self.psq_b = 0.0
        self.ha = hashlib.sha256()
        self.hb = hashlib.sha256()
        self.na = 0
        self.nb = 0

    def feed(self, a: np.ndarray, b: np.ndarray):
        a = np.asarray(a, dtype=np.int64)
        b = np.asarray(b, dtype=np.int64)
        self.ha.update(np.ascontiguousarray(a, dtype="<i4").tobytes())
        self.hb.update(np.ascontiguousarray(b, dtype="<i4").tobytes())
        self.na += a.size
        self.nb += b.size
        if a.size:
            self.peak_a = max(self.peak_a, int(np.max(np.abs(a))))
            self.sq_a += float(np.dot(a, a))
            self.zero_a += int(np.count_nonzero(a == 0))
        if b.size:
            self.peak_b = max(self.peak_b, int(np.max(np.abs(b))))
            self.sq_b += float(np.dot(b, b))
            self.zero_b += int(np.count_nonzero(b == 0))
        n = min(a.size, b.size)
        if n:
            d = a[:n] - b[:n]
            nz = np.nonzero(d)[0]
            if nz.size and self.first is None:
                self.first = self.n + int(nz[0])
            self.differing += int(nz.size)
            self.max_abs = max(self.max_abs, int(np.max(np.abs(d))))
            self.sq_err += float(np.dot(d, d))
            af = a[:n].astype(np.float64)
            bf = b[:n].astype(np.float64)
            self.sum_a += float(af.sum())
            self.sum_b += float(bf.sum())
            self.sum_ab += float(np.dot(af, bf))
            self.psq_a += float(np.dot(af, af))
            self.psq_b += float(np.dot(bf, bf))
            self.n += n


def compare_streams(blocks_a, blocks_b) -> TrackCompare:
    """Same result as ``compare_tracks`` over two iterables of blocks, in bounded memory."""
    acc = _StreamAcc()
    ita, itb = iter(blocks_a), iter(blocks_b)
    bufa = np.zeros(0, dtype=np.int64)
    bufb = np.zeros(0, dtype=np.int64)
    done_a = done_b = False
    while True:
        while bufa.size == 0 and not done_a:
            try:
                bufa = np.asarray(next(ita), dtype=np.int64)
            except StopIteration:
                done_a = True
        while bufb.size == 0 and not done_b:
            try:
                bufb = np.asarray(next(itb), dtype=np.int64)
            except StopIteration:
                done_b = True
        if bufa.size == 0 and bufb.size == 0:
            break
        n = min(bufa.size, bufb.size) if (bufa.size and bufb.size) else max(bufa.size, bufb.size)
        acc.feed(bufa[:n], bufb[:n])
        bufa, bufb = bufa[n:], bufb[n:]
    return _finish(acc)


def _finish(acc: _StreamAcc) -> TrackCompare:
    n = acc.n
    differing = acc.differing + abs(acc.na - acc.nb)
    first = acc.first
    if acc.na != acc.nb and first is None:
        first = n
    corr = None
    if n > 1:
        va = acc.psq_a / n - (acc.sum_a / n) ** 2
        vb = acc.psq_b / n - (acc.sum_b / n) ** 2
        if va > 0 and vb > 0:
            corr = (acc.sum_ab / n - (acc.sum_a / n) * (acc.sum_b / n)) / np.sqrt(va * vb)
            corr = float(corr)
    return TrackCompare(
        samples_a=acc.na, samples_b=acc.nb, length_delta=acc.nb - acc.na, compared=n,
        differing=differing, first_diff=first, max_abs=acc.max_abs,
        rms_error=float(np.sqrt(acc.sq_err / n)) if n else 0.0,
        peak_a=acc.peak_a, peak_b=acc.peak_b,
        rms_a=float(np.sqrt(acc.sq_a / acc.na)) if acc.na else 0.0, rms_b=float(np.sqrt(acc.sq_b / acc.nb)) if acc.nb else 0.0,
        silence_pct_a=float(100.0 * acc.zero_a / acc.na) if acc.na else 100.0,
        silence_pct_b=float(100.0 * acc.zero_b / acc.nb) if acc.nb else 100.0,
        corr=corr, sha256_a=acc.ha.hexdigest(), sha256_b=acc.hb.hexdigest(),
        identical=(acc.na == acc.nb and differing == 0),
    )


def wav_frames(path: str, container, block_frames: int = 1 << 18):
    """Yield ``(n, ch)`` int32 arrays of interleaved 24-bit LE frames from a RIFF/RF64 file."""
    fmt = container.fmt
    ch = fmt.channels
    frames = container.frames or 0
    with open(path, "rb") as f:
        done = 0
        while done < frames:
            n = min(block_frames, frames - done)
            f.seek(container.data.data_offset + done * fmt.block_align)
            raw = np.frombuffer(f.read(n * fmt.block_align), dtype=np.uint8).reshape(n, ch, 3)
            v = raw[:, :, 0].astype(np.int32) | (raw[:, :, 1].astype(np.int32) << 8) | (raw[:, :, 2].astype(np.int32) << 16)
            yield np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.int32)
            done += n


def caf_frames(path: str, info, block_frames: int = 1 << 18):
    """Yield ``(n, ch)`` int32 arrays of interleaved 24-bit frames from a CAF file."""
    ch = info.channels
    bpf = info.bytes_per_frame
    with open(path, "rb") as f:
        done = 0
        while done < info.frames:
            n = min(block_frames, info.frames - done)
            f.seek(info.data_offset + done * bpf)
            raw = np.frombuffer(f.read(n * bpf), dtype=np.uint8).reshape(n, ch, 3)
            if info.big_endian:
                v = (raw[:, :, 0].astype(np.int32) << 16) | (raw[:, :, 1].astype(np.int32) << 8) | raw[:, :, 2].astype(np.int32)
            else:
                v = raw[:, :, 0].astype(np.int32) | (raw[:, :, 1].astype(np.int32) << 8) | (raw[:, :, 2].astype(np.int32) << 16)
            yield np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.int32)
            done += n


def compare_interleaved(frames_a, frames_b, pairs: list) -> list:
    """One pass over two interleaved frame streams; ``pairs`` = [(track in a, track in b)].

    Returns one ``TrackCompare`` per pair, identical to ``compare_tracks`` on the
    extracted tracks, but the files are read once instead of once per track.
    """
    accs = [_StreamAcc() for _ in pairs]
    ita, itb = iter(frames_a), iter(frames_b)
    bufa = bufb = None
    done_a = done_b = False
    while True:
        while (bufa is None or bufa.shape[0] == 0) and not done_a:
            try:
                bufa = next(ita)
            except StopIteration:
                done_a = True
                bufa = None
        while (bufb is None or bufb.shape[0] == 0) and not done_b:
            try:
                bufb = next(itb)
            except StopIteration:
                done_b = True
                bufb = None
        na = 0 if bufa is None else bufa.shape[0]
        nb = 0 if bufb is None else bufb.shape[0]
        if na == 0 and nb == 0:
            break
        n = min(na, nb) if (na and nb) else max(na, nb)
        for acc, (ta, tb) in zip(accs, pairs):
            a = bufa[:n, ta] if na else np.zeros(0, dtype=np.int32)
            b = bufb[:n, tb] if nb else np.zeros(0, dtype=np.int32)
            acc.feed(a, b)
        bufa = bufa[n:] if na else None
        bufb = bufb[n:] if nb else None
    return [_finish(acc) for acc in accs]
