"""RIFF / RF64 / BW64 container walker and 24-bit PCM track reader.

Written for the audit from the container specifications (EBU Tech 3285 for
BWF, EBU Tech 3306 for RF64/``ds64``, ITU-R BS.2088 for BW64); it shares no
code with oadec's writer.  Every size is taken from the file itself, and the
``data`` chunk size ``0xFFFFFFFF`` is resolved only through ``ds64``.
"""
from __future__ import annotations

import os
import struct
from dataclasses import dataclass, field

import numpy as np

MINUS_ONE = 0xFFFFFFFF
OUTER_IDS = ("RIFF", "RF64", "BW64")


class ContainerError(Exception):
    """The file cannot be walked as a RIFF-family container."""


@dataclass(frozen=True)
class Finding:
    kind: str
    message: str


@dataclass(frozen=True)
class Chunk:
    id: str
    size: int          # payload bytes as resolved (ds64 applied)
    offset: int        # offset of the 8-byte header
    data_offset: int   # offset of the payload
    padded: bool       # a pad byte follows the payload
    size_field: int    # the raw 32-bit size field


@dataclass(frozen=True)
class Fmt:
    format_tag: int
    channels: int
    sample_rate: int
    byte_rate: int
    block_align: int
    bits_per_sample: int
    extension: bytes = b""


@dataclass(frozen=True)
class Ds64:
    riff_size: int
    data_size: int
    sample_count: int
    table: tuple = ()


@dataclass
class Container:
    path: str
    file_size: int
    fourcc: str
    riff_size_field: int
    chunks: list = field(default_factory=list)
    fmt: Fmt | None = None
    data: Chunk | None = None
    ds64: Ds64 | None = None

    @property
    def frames(self) -> int | None:
        if self.fmt is None or self.data is None or self.fmt.block_align == 0:
            return None
        return self.data.size // self.fmt.block_align

    def chunk(self, cid: str) -> Chunk | None:
        for c in self.chunks:
            if c.id == cid:
                return c
        return None


def _parse_fmt(payload: bytes) -> Fmt:
    if len(payload) < 16:
        raise ContainerError(f"fmt chunk is {len(payload)} bytes, need 16")
    tag, ch, rate, byte_rate, block, bits = struct.unpack("<HHIIHH", payload[:16])
    return Fmt(tag, ch, rate, byte_rate, block, bits, payload[16:])


def _parse_ds64(payload: bytes) -> Ds64:
    if len(payload) < 28:
        raise ContainerError(f"ds64 chunk is {len(payload)} bytes, need 28")
    riff_size, data_size, sample_count, table_len = struct.unpack("<QQQI", payload[:28])
    table = []
    pos = 28
    for _ in range(table_len):
        if pos + 12 > len(payload):
            raise ContainerError("ds64 table runs past the chunk")
        cid, size = struct.unpack("<4sQ", payload[pos : pos + 12])
        table.append((cid.decode("latin-1"), size))
        pos += 12
    return Ds64(riff_size, data_size, sample_count, tuple(table))


def scan(path: str) -> Container:
    """Walk the chunk list; raise ``ContainerError`` on anything unwalkable."""
    file_size = os.path.getsize(path)
    with open(path, "rb") as f:
        head = f.read(12)
        if len(head) < 12:
            raise ContainerError("file shorter than a RIFF header")
        fourcc_b, riff_size_field, wave = struct.unpack("<4sI4s", head)
        fourcc = fourcc_b.decode("latin-1")
        if fourcc not in OUTER_IDS:
            raise ContainerError(f"outer chunk id {fourcc!r} is not RIFF/RF64/BW64")
        if wave != b"WAVE":
            raise ContainerError(f"form type {wave!r} is not WAVE")
        c = Container(path, file_size, fourcc, riff_size_field)
        pos = 12
        while pos + 8 <= file_size:
            f.seek(pos)
            cid_b, size_field = struct.unpack("<4sI", f.read(8))
            cid = cid_b.decode("latin-1")
            size = size_field
            if cid == "ds64":
                payload = f.read(min(size_field, file_size - pos - 8))
                c.ds64 = _parse_ds64(payload)
            elif size_field == MINUS_ONE:
                if c.ds64 is None:
                    raise ContainerError(f"chunk {cid!r} declares size -1 but no ds64 chunk precedes it")
                if cid == "data":
                    size = c.ds64.data_size
                else:
                    sizes = dict(c.ds64.table)
                    if cid not in sizes:
                        raise ContainerError(f"chunk {cid!r} declares size -1 and ds64 has no table entry for it")
                    size = sizes[cid]
            data_offset = pos + 8
            if data_offset + size > file_size:
                raise ContainerError(
                    f"chunk {cid!r} at {pos} declares {size} bytes but only {file_size - data_offset} remain"
                )
            padded = bool(size & 1)
            chunk = Chunk(cid, size, pos, data_offset, padded, size_field)
            c.chunks.append(chunk)
            if cid == "fmt ":
                f.seek(data_offset)
                c.fmt = _parse_fmt(f.read(size))
            elif cid == "data":
                c.data = chunk
            pos = data_offset + size + (1 if padded else 0)
    return c


def chunk_bytes(path: str, chunk: Chunk) -> bytes:
    with open(path, "rb") as f:
        f.seek(chunk.data_offset)
        return f.read(chunk.size)


def verify(path: str, c: Container) -> list[Finding]:
    """Container-level checks; an empty list means nothing was found."""
    out: list[Finding] = []
    ids = [k.id for k in c.chunks]
    if c.fourcc == "RIFF":
        if c.riff_size_field + 8 != c.file_size:
            out.append(Finding("riff-size", f"RIFF size field {c.riff_size_field} + 8 != file size {c.file_size}"))
        if c.ds64 is not None:
            out.append(Finding("ds64-in-riff", "a RIFF file carries a ds64 chunk"))
        if c.data is not None and c.data.size_field == MINUS_ONE:
            out.append(Finding("riff-data-minus-one", "RIFF file with data size -1"))
    else:
        if c.riff_size_field != MINUS_ONE:
            out.append(Finding("rf64-size-field", f"{c.fourcc} size field is {c.riff_size_field}, expected -1"))
        if c.ds64 is None:
            out.append(Finding("ds64-missing", f"{c.fourcc} file without a ds64 chunk"))
        else:
            if ids[:1] != ["ds64"]:
                out.append(Finding("ds64-not-first", f"ds64 is not the first chunk (order: {ids})"))
            if c.ds64.riff_size + 8 != c.file_size:
                out.append(Finding("ds64-riff-size", f"ds64 riffSize {c.ds64.riff_size} + 8 != file size {c.file_size}"))
            if c.data is not None and c.data.size_field != MINUS_ONE:
                out.append(Finding("rf64-data-size-field", f"data size field is {c.data.size_field}, expected -1"))
            if c.frames is not None and c.ds64.sample_count not in (0, c.frames):
                out.append(Finding("ds64-sample-count", f"ds64 sampleCount {c.ds64.sample_count} != frames {c.frames}"))
    if c.fmt is None:
        out.append(Finding("fmt-missing", "no fmt chunk"))
    else:
        f = c.fmt
        if f.format_tag not in (1, 0xFFFE):
            out.append(Finding("fmt-tag", f"format tag {f.format_tag} is neither PCM (1) nor EXTENSIBLE (0xFFFE)"))
        if f.bits_per_sample != 24:
            out.append(Finding("fmt-bits", f"{f.bits_per_sample} bits per sample"))
        if f.block_align != f.channels * ((f.bits_per_sample + 7) // 8):
            out.append(Finding("fmt-block-align", f"block_align {f.block_align} != channels*bytes {f.channels * ((f.bits_per_sample + 7) // 8)}"))
        if f.byte_rate != f.sample_rate * f.block_align:
            out.append(Finding("fmt-byte-rate", f"byte_rate {f.byte_rate} != rate*block_align {f.sample_rate * f.block_align}"))
        if f.format_tag == 0xFFFE and len(f.extension) < 24:
            out.append(Finding("fmt-extension", "EXTENSIBLE fmt without a 22-byte extension"))
    if c.data is None:
        out.append(Finding("data-missing", "no data chunk"))
    elif c.fmt is not None and c.fmt.block_align and c.data.size % c.fmt.block_align:
        out.append(Finding("data-partial-frame", f"data size {c.data.size} is not a multiple of block_align {c.fmt.block_align}"))
    if "fmt " in ids and "data" in ids and ids.index("fmt ") > ids.index("data"):
        out.append(Finding("fmt-after-data", "fmt chunk follows the data chunk"))
    if c.chunks:
        last = c.chunks[-1]
        end = last.data_offset + last.size + (1 if last.padded else 0)
        if end < c.file_size:
            out.append(Finding("trailing-bytes", f"{c.file_size - end} bytes after the last chunk"))
        if last.padded and last.data_offset + last.size >= c.file_size:
            out.append(Finding("missing-pad", f"odd chunk {last.id!r} has no pad byte"))
    return out


def read_track(path: str, c: Container, track: int, block_frames: int = 1 << 20):
    """Yield ``numpy.int32`` blocks of one channel's 24-bit samples."""
    if c.fmt is None or c.data is None:
        raise ContainerError("no fmt/data")
    ch = c.fmt.channels
    if not 0 <= track < ch:
        raise IndexError(f"track {track} of {ch}")
    frames = c.frames or 0
    with open(path, "rb") as f:
        done = 0
        while done < frames:
            n = min(block_frames, frames - done)
            f.seek(c.data.data_offset + done * c.fmt.block_align)
            raw = np.frombuffer(f.read(n * c.fmt.block_align), dtype=np.uint8)
            raw = raw.reshape(n, ch, 3)[:, track, :]
            v = raw[:, 0].astype(np.int32) | (raw[:, 1].astype(np.int32) << 8) | (raw[:, 2].astype(np.int32) << 16)
            v = np.where(v >= 1 << 23, v - (1 << 24), v)
            yield v.astype(np.int32)
            done += n


def read_track_all(path: str, c: Container, track: int) -> np.ndarray:
    blocks = list(read_track(path, c, track))
    return np.concatenate(blocks) if blocks else np.zeros(0, dtype=np.int32)
