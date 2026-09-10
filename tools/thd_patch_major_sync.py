#!/usr/bin/env python3
"""Set one bit of every TrueHD major sync and repair the CRC-16.

    python tools/thd_patch_major_sync.py in.thd out.thd <bit> <0|1>

The bit is an absolute offset from the start of the format sync, so any field of
the major sync can be reached; 150 is `2ch_control_enabled`.

**It does not reach Dolby's object decoder.** That path refuses any stream whose
major sync was edited, whatever the field -- the 16-bit `reserved` area, the
peak data rate, the variable-rate flag and the DRC start-up gain all produce
"Selected Dolby TrueHD presentation is not available" with the CRC repaired,
while the same streams decode at presentation 2 and at presentation 16 with the
default channel configuration. So this measures oadec, and a refusal from that
decoder says nothing about the field that was changed. See
`docs/audit/evidence/remediation/major-sync-rewrite-rejected.json`.
"""
import sys

POLY = 0x002D
TBL = []
for i in range(256):
    c = i << 8
    for _ in range(8):
        c = ((c << 1) ^ POLY) & 0xFFFF if c & 0x8000 else (c << 1) & 0xFFFF
    TBL.append(c)


def crc16(data):
    c = 0
    for b in data:
        c = TBL[c >> 8] ^ ((c << 8) & 0xFFFF) ^ b
    return c


src, dst, which, val = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
# `which` is an absolute bit offset from the start of the format sync:
#   0..31 format_sync, 32..63 format_info, 64..79 signature, 80..95 flags,
#   96..111 reserved, 112 variable_rate, 113..127 peak_data_rate,
#   128..131 substreams, 132..135 extended_substream_info,
#   136..143 substream_info, 144.. channel_meaning (150 = 2ch_control_enabled)
BIT = which
d = bytearray(open(src, 'rb').read())
sig = bytes.fromhex('f8726fba')
i, n, fixed, failed, already = -1, 0, 0, 0, 0
while True:
    i = d.find(sig, i + 1)
    if i < 0:
        break
    n += 1
    k = None
    for cand in range(24, 80, 2):
        if i + cand + 2 > len(d):
            break
        if crc16(d[i:i + cand]) == int.from_bytes(d[i + cand:i + cand + 2], 'big'):
            k = cand
            break
    if k is None:
        failed += 1
        continue
    byte = i + BIT // 8
    mask = 1 << (7 - (BIT % 8))
    old = (d[byte] & mask) != 0
    if old == bool(val):
        already += 1
        continue
    d[byte] = (d[byte] | mask) if val else (d[byte] & ~mask)
    crc = crc16(d[i:i + k])
    d[i + k] = crc >> 8
    d[i + k + 1] = crc & 0xFF
    fixed += 1
open(dst, 'wb').write(bytes(d))
print(f"major syncs {n}: {fixed} rewritten, {already} already at {val}, {failed} without a CRC match")
