"""Negative controls: inject one exact defect into a copy of an ADM BWF or DAMF set.

Every ADM mutation rebuilds the container so that chunk sizes, the RIFF size
and (for RF64) the ``ds64`` fields stay consistent -- the injected defect is
the *only* thing that changes.  XML edits are textual so that formatting,
ordering and precision are preserved.
"""
from __future__ import annotations

import os
import re
import shutil
import struct

import numpy as np

from . import riff, timecode

# ----------------------------------------------------------------------------- ADM container


def rebuild(src: str, out: str, replace: dict | None = None, pcm_transform=None, data_delta: int = 0, drop: set | None = None) -> None:
    replace = replace or {}
    drop = drop or set()
    c = riff.scan(src)
    payloads = []
    with open(src, "rb") as f:
        for ch in c.chunks:
            if ch.id in drop:
                continue
            if ch.id in replace:
                payloads.append((ch.id, replace[ch.id]))
                continue
            f.seek(ch.data_offset)
            if ch.id == "data" and (pcm_transform is not None or data_delta):
                raw = f.read(ch.size)
                if pcm_transform is not None:
                    raw = pcm_transform(raw, c.fmt)
                if data_delta < 0:
                    raw = raw[:data_delta]
                elif data_delta > 0:
                    raw = raw + b"\x00" * data_delta
                payloads.append(("data", raw))
            else:
                payloads.append((ch.id, f.read(ch.size)))
    rf64 = c.fourcc != "RIFF"
    body = b""
    data_size = 0
    for cid, payload in payloads:
        if cid == "ds64":
            continue
        if cid == "data":
            data_size = len(payload)
        size_field = 0xFFFFFFFF if (rf64 and cid == "data") else len(payload)
        body += struct.pack("<4sI", cid.encode("latin-1"), size_field) + payload + (b"\x00" if len(payload) & 1 else b"")
    if rf64:
        frames = data_size // c.fmt.block_align if c.fmt and c.fmt.block_align else 0
        ds64 = struct.pack("<QQQI", 0, data_size, frames, 0)
        ds64_chunk = struct.pack("<4sI", b"ds64", len(ds64)) + ds64
        riff_size = 4 + len(ds64_chunk) + len(body)
        ds64 = struct.pack("<QQQI", riff_size, data_size, frames, 0)
        ds64_chunk = struct.pack("<4sI", b"ds64", len(ds64)) + ds64
        blob = struct.pack("<4sI4s", c.fourcc.encode("latin-1"), 0xFFFFFFFF, b"WAVE") + ds64_chunk + body
    else:
        blob = struct.pack("<4sI4s", b"RIFF", 4 + len(body), b"WAVE") + body
    with open(out, "wb") as f:
        f.write(blob)


def edit_axml(src: str, out: str, fn) -> None:
    c = riff.scan(src)
    ax = c.chunk("axml")
    text = riff.chunk_bytes(src, ax).decode("utf-8")
    new = fn(text)
    if new == text:
        raise ValueError("axml edit changed nothing")
    rebuild(src, out, replace={"axml": new.encode("utf-8")})


def swap_tracks(src: str, out: str, i: int, j: int) -> None:
    def tf(raw: bytes, fmt) -> bytes:
        a = np.frombuffer(raw, dtype=np.uint8).reshape(-1, fmt.channels, 3).copy()
        a[:, [i, j], :] = a[:, [j, i], :]
        return a.tobytes()
    rebuild(src, out, pcm_transform=tf)


def set_data_size(src: str, out: str, delta: int) -> None:
    rebuild(src, out, data_delta=delta)


def truncate(src: str, out: str, inside: str = "axml") -> None:
    c = riff.scan(src)
    ch = c.chunk(inside)
    if ch is None:
        raise KeyError(inside)
    cut = ch.data_offset + max(1, ch.size // 2)
    with open(src, "rb") as f:
        blob = f.read(cut)
    with open(out, "wb") as f:
        f.write(blob)


# ----------------------------------------------------------------------------- axml text surgery

_BLOCK_RE = re.compile(r"<audioBlockFormat\b[^>]*>.*?</audioBlockFormat>", re.S)


def _channel_span(text: str, ac_id: str) -> tuple:
    m = re.search(r'<audioChannelFormat\b[^>]*audioChannelFormatID="%s"[^>]*>.*?</audioChannelFormat>' % re.escape(ac_id), text, re.S)
    if not m:
        raise KeyError(ac_id)
    return m.start(), m.end()


def _blocks(text: str, ac_id: str) -> list:
    s, e = _channel_span(text, ac_id)
    return [(s + m.start(), s + m.end()) for m in _BLOCK_RE.finditer(text[s:e])]


def _replace_span(text: str, span: tuple, new: str) -> str:
    return text[: span[0]] + new + text[span[1]:]


def _set_attr(block: str, attr: str, value: str) -> str:
    head_end = block.index(">")
    head = block[:head_end]
    if re.search(r'\b%s="' % attr, head):
        head = re.sub(r'(\b%s=")[^"]*(")' % attr, lambda m: m.group(1) + value + m.group(2), head, count=1)
    else:
        head = head + f' {attr}="{value}"'
    return head + block[head_end:]


def _get_attr(block: str, attr: str) -> str | None:
    m = re.search(r'\b%s="([^"]*)"' % attr, block[: block.index(">")])
    return m.group(1) if m else None


def edit_blocks(src: str, out: str, ac_id: str, fn) -> None:
    """``fn(index, block_text) -> new block_text`` over the blocks of one channel format."""
    def apply(text: str) -> str:
        spans = _blocks(text, ac_id)
        for k, span in reversed(list(enumerate(spans))):
            new = fn(k, text[span[0]:span[1]])
            if new is not None:
                text = _replace_span(text, span, new)
        return text
    edit_axml(src, out, apply)


def shift_block(src: str, out: str, ac_id: str, index: int, delta: int, fs: int) -> None:
    def fn(k, block):
        if k == index:
            t = timecode.decode(_get_attr(block, "rtime"), fs).samples + delta
            d = timecode.decode(_get_attr(block, "duration"), fs).samples - delta
            block = _set_attr(block, "rtime", timecode.encode(t, fs))
            return _set_attr(block, "duration", timecode.encode(d, fs))
        if k == index - 1:
            d = timecode.decode(_get_attr(block, "duration"), fs).samples + delta
            return _set_attr(block, "duration", timecode.encode(d, fs))
        return None
    edit_blocks(src, out, ac_id, fn)


def _set_interp_in_block(block: str, samples: int, fs: int) -> str:
    value = f"{samples / fs:.6f}"
    if "interpolationLength=" in block:
        return re.sub(r'interpolationLength="[^"]*"', f'interpolationLength="{value}"', block, count=1)
    return block.replace("<jumpPosition", f'<jumpPosition interpolationLength="{value}"', 1)


def set_interpolation(src: str, out: str, spec, fs: int) -> None:
    """``spec``: samples for every object block, or ``{ac_id: {block index: samples}}``."""
    def apply(text: str) -> str:
        if isinstance(spec, dict):
            for ac_id, per_block in spec.items():
                spans = _blocks(text, ac_id)
                for k in sorted(per_block, reverse=True):
                    text = _replace_span(text, spans[k], _set_interp_in_block(text[spans[k][0]:spans[k][1]], per_block[k], fs))
            return text
        return _BLOCK_RE.sub(lambda m: _set_interp_in_block(m.group(0), spec, fs) if "rtime=" in m.group(0) else m.group(0), text)
    edit_axml(src, out, apply)


def remove_child(src: str, out: str, ac_id: str, index: int, tag: str) -> None:
    """``tag`` may carry an attribute selector, e.g. ``position coordinate="Z"``."""
    name = tag.split()[0]

    def fn(k, block):
        if k != index:
            return None
        new = re.sub(r"\s*<%s(?=[\s>/])[^>]*>.*?</%s>" % (re.escape(tag), name), "", block, count=1, flags=re.S)
        if new == block:
            raise KeyError(f"block {index} of {ac_id} has no <{tag}>")
        return new
    edit_blocks(src, out, ac_id, fn)


def add_child(src: str, out: str, ac_id: str, index: int, fragment: str) -> None:
    def fn(k, block):
        if k != index:
            return None
        return block.replace("</audioBlockFormat>", fragment + "</audioBlockFormat>", 1)
    edit_blocks(src, out, ac_id, fn)


def delete_block(src: str, out: str, ac_id: str, index: int) -> None:
    def apply(text: str) -> str:
        span = _blocks(text, ac_id)[index]
        start = span[0]
        while start > 0 and text[start - 1] in " \t":
            start -= 1
        if start > 0 and text[start - 1] == "\n":
            start -= 1
        return text[:start] + text[span[1]:]
    edit_axml(src, out, apply)


_POS_RE = re.compile(r'(<position coordinate=")([XYZ])(">)([^<]*)(</position>)')


def _object_blocks_only(text: str, fn) -> str:
    return _BLOCK_RE.sub(lambda m: fn(m.group(0)) if "rtime=" in m.group(0) else m.group(0), text)


def invert_axis(src: str, out: str, axis: str) -> None:
    def inv(block: str) -> str:
        return _POS_RE.sub(lambda m: m.group(1) + m.group(2) + m.group(3) + (_neg(m.group(4)) if m.group(2) == axis else m.group(4)) + m.group(5), block)
    edit_axml(src, out, lambda t: _object_blocks_only(t, inv))


def _neg(v: str) -> str:
    v = v.strip()
    return v[1:] if v.startswith("-") else "-" + v


def swap_axes(src: str, out: str, a: str, b: str) -> None:
    def sw(block: str) -> str:
        return _POS_RE.sub(lambda m: m.group(1) + ({a: b, b: a}.get(m.group(2), m.group(2))) + m.group(3) + m.group(4) + m.group(5), block)
    edit_axml(src, out, lambda t: _object_blocks_only(t, sw))


def duplicate_uid(src: str, out: str, uid: str) -> None:
    def apply(text: str) -> str:
        m = re.search(r'<audioTrackUID\b[^>]*UID="%s"[^>]*>.*?</audioTrackUID>' % re.escape(uid), text, re.S)
        if not m:
            raise KeyError(uid)
        return text[: m.end()] + "\n" + m.group(0) + text[m.end():]
    edit_axml(src, out, apply)


def break_chna(src: str, out: str, entry_index: int, uid: str | None = None, track_ref: str | None = None, pack_ref: str | None = None, track_index: int | None = None) -> None:
    c = riff.scan(src)
    payload = bytearray(riff.chunk_bytes(src, c.chunk("chna")))
    off = 4 + 40 * entry_index
    idx, u, t, p = struct.unpack("<H12s14s11sx", payload[off : off + 40])
    if track_index is not None:
        idx = track_index
    if uid is not None:
        u = uid.encode()
    if track_ref is not None:
        t = track_ref.encode()
    if pack_ref is not None:
        p = pack_ref.encode()
    payload[off : off + 40] = struct.pack("<H12s14s11sx", idx, u, t, p)
    rebuild(src, out, replace={"chna": bytes(payload)})


# ----------------------------------------------------------------------------- DAMF text surgery

_EV_START = re.compile(r"^\s*-\s*ID:\s*(\d+)\s*$")


def _split_metadata(text: str) -> tuple:
    """(header lines, [(id, sample_pos, [lines])])."""
    lines = text.splitlines()
    header = []
    events = []
    cur = None
    for line in lines:
        m = _EV_START.match(line)
        if m:
            cur = [int(m.group(1)), None, [line]]
            events.append(cur)
            continue
        if cur is None:
            header.append(line)
            continue
        cur[2].append(line)
        sm = re.match(r"^\s*samplePos:\s*(\d+)\s*$", line)
        if sm:
            cur[1] = int(sm.group(1))
    return header, events


def _join(header: list, events: list) -> str:
    out = list(header)
    for _id, _pos, ls in events:
        out.extend(ls)
    return "\n".join(out) + "\n"


def _fmt_value(v) -> str:
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (tuple, list)):
        return "[" + ", ".join(_fmt_value(x) for x in v) + "]"
    if isinstance(v, float):
        return repr(v)
    return str(v)


def damf_copy(base: str, out: str) -> None:
    name = os.path.basename(out)
    with open(base + ".atmos", encoding="utf-8") as f:
        text = f.read()
    text = re.sub(r"(^\s*metadata:\s*).*$", lambda m: m.group(1) + name + ".atmos.metadata", text, flags=re.M)
    text = re.sub(r"(^\s*audio:\s*).*$", lambda m: m.group(1) + name + ".atmos.audio", text, flags=re.M)
    with open(out + ".atmos", "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    shutil.copyfile(base + ".atmos.metadata", out + ".atmos.metadata")
    if os.path.exists(base + ".atmos.audio"):
        shutil.copyfile(base + ".atmos.audio", out + ".atmos.audio")


def _edit_metadata(base: str, out: str, fn) -> None:
    damf_copy(base, out)
    with open(out + ".atmos.metadata", encoding="utf-8") as f:
        header, events = _split_metadata(f.read())
    header, events = fn(header, events)
    with open(out + ".atmos.metadata", "w", encoding="utf-8", newline="\n") as f:
        f.write(_join(header, events))


def damf_set(base: str, out: str, element_id: int, event_index: int | None, key: str, value) -> None:
    def fn(header, events):
        k = -1
        for ev in events:
            if ev[0] != element_id:
                continue
            k += 1
            if event_index is not None and k != event_index:
                continue
            indent = re.match(r"^(\s*)", ev[2][1]).group(1) if len(ev[2]) > 1 else "    "
            new_line = f"{indent}{key}: {_fmt_value(value)}"
            for i, line in enumerate(ev[2]):
                if re.match(r"^\s*%s:" % re.escape(key), line):
                    ev[2][i] = new_line
                    break
            else:
                ev[2].insert(2 if len(ev[2]) > 1 else 1, new_line)
        return header, events
    _edit_metadata(base, out, fn)


def damf_move_event(base: str, out: str, element_id: int, event_index: int, new_sample_pos: int) -> None:
    def fn(header, events):
        k = -1
        for ev in events:
            if ev[0] != element_id:
                continue
            k += 1
            if k == event_index:
                ev[1] = new_sample_pos
                ev[2] = [re.sub(r"^(\s*samplePos:\s*)\d+", lambda m: m.group(1) + str(new_sample_pos), l) for l in ev[2]]
        events.sort(key=lambda e: e[1] if e[1] is not None else -1)
        return header, events
    _edit_metadata(base, out, fn)


def damf_delete_event(base: str, out: str, element_id: int, event_index: int) -> None:
    def fn(header, events):
        k = -1
        keep = []
        for ev in events:
            if ev[0] == element_id:
                k += 1
                if k == event_index:
                    continue
            keep.append(ev)
        return header, keep
    _edit_metadata(base, out, fn)


def damf_set_header(base: str, out: str, key: str, value: str) -> None:
    damf_copy(base, out)
    with open(out + ".atmos", encoding="utf-8") as f:
        text = f.read()
    new = re.sub(r"(^\s*%s:\s*).*$" % re.escape(key), lambda m: m.group(1) + value, text, count=1, flags=re.M)
    if new == text:
        raise KeyError(key)
    with open(out + ".atmos", "w", encoding="utf-8", newline="\n") as f:
        f.write(new)


def poke_sample(src: str, out: str, track: int, frame: int, delta: int) -> None:
    """Add ``delta`` to one 24-bit sample of one track."""
    def tf(raw: bytes, fmt) -> bytes:
        a = np.frombuffer(raw, dtype=np.uint8).reshape(-1, fmt.channels, 3).copy()
        b = a[frame, track, :]
        v = int(b[0]) | (int(b[1]) << 8) | (int(b[2]) << 16)
        if v >= 1 << 23:
            v -= 1 << 24
        v += delta
        a[frame, track, :] = np.frombuffer((v & 0xFFFFFF).to_bytes(3, "little"), dtype=np.uint8)
        return a.tobytes()
    rebuild(src, out, pcm_transform=tf)


def damf_edit_text(base: str, out: str, fn) -> None:
    """Free-form edit of the ``.atmos.metadata`` text of a copied DAMF set."""
    damf_copy(base, out)
    with open(out + ".atmos.metadata", encoding="utf-8") as f:
        text = f.read()
    new = fn(text)
    if new == text:
        raise ValueError("metadata edit changed nothing")
    with open(out + ".atmos.metadata", "w", encoding="utf-8", newline="\n") as f:
        f.write(new)
