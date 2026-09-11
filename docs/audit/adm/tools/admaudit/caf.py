"""Core Audio Format reader for DAMF ``.atmos.audio`` (24-bit integer LPCM)."""
from __future__ import annotations

import os
import struct
from dataclasses import dataclass

import numpy as np


class CafError(Exception):
    pass


@dataclass(frozen=True)
class CafInfo:
    sample_rate: int
    channels: int
    bits: int
    frames: int
    data_offset: int
    big_endian: bool
    is_float: bool
    bytes_per_frame: int
    size_unknown: bool


def read_header(path: str) -> CafInfo:
    size = os.path.getsize(path)
    with open(path, "rb") as f:
        magic = f.read(8)
        if magic[:4] != b"caff":
            raise CafError("not a CAF file")
        desc = None
        pos = 8
        while pos + 12 <= size:
            f.seek(pos)
            ctype, csize = struct.unpack(">4sq", f.read(12))
            if ctype == b"desc":
                rate, fmt, flags, bpp, fpp, cpf, bpc = struct.unpack(">d4sIIIII", f.read(32))
                if fmt != b"lpcm":
                    raise CafError(f"format {fmt!r} is not lpcm")
                desc = (rate, flags, bpp, fpp, cpf, bpc)
            elif ctype == b"data":
                if desc is None:
                    raise CafError("data chunk before desc chunk")
                rate, flags, bpp, fpp, cpf, bpc = desc
                data_offset = pos + 12 + 4  # edit count
                if csize == -1:
                    payload = size - data_offset
                    unknown = True
                else:
                    payload = csize - 4
                    unknown = False
                    if data_offset + payload > size:
                        raise CafError("data chunk runs past end of file")
                if fpp != 1 or bpp != cpf * ((bpc + 7) // 8):
                    raise CafError(f"unsupported packing: {bpp} bytes/packet, {fpp} frames/packet, {cpf} ch, {bpc} bits")
                return CafInfo(int(rate), cpf, bpc, payload // bpp, data_offset, not (flags & 2), bool(flags & 1), bpp, unknown)
            if csize < 0:
                raise CafError(f"chunk {ctype!r} with unknown size before data")
            pos += 12 + csize
    raise CafError("no data chunk")


def read_track(path: str, info: CafInfo, track: int, block_frames: int = 1 << 20):
    if info.bits != 24 or info.is_float:
        raise CafError("only 24-bit integer CAF is supported")
    if not 0 <= track < info.channels:
        raise IndexError(track)
    bpf = info.bytes_per_frame
    with open(path, "rb") as f:
        done = 0
        while done < info.frames:
            n = min(block_frames, info.frames - done)
            f.seek(info.data_offset + done * bpf)
            raw = np.frombuffer(f.read(n * bpf), dtype=np.uint8).reshape(n, info.channels, 3)[:, track, :]
            if info.big_endian:
                v = (raw[:, 0].astype(np.int32) << 16) | (raw[:, 1].astype(np.int32) << 8) | raw[:, 2].astype(np.int32)
            else:
                v = raw[:, 0].astype(np.int32) | (raw[:, 1].astype(np.int32) << 8) | (raw[:, 2].astype(np.int32) << 16)
            yield np.where(v >= 1 << 23, v - (1 << 24), v).astype(np.int32)
            done += n


def read_track_all(path: str, info: CafInfo, track: int) -> np.ndarray:
    blocks = list(read_track(path, info, track))
    return np.concatenate(blocks) if blocks else np.zeros(0, dtype=np.int32)
