# Object Audio Metadata as `oadec` reads it

The OAMD payload (ETSI TS 103 420 V1.2.1, clause 5.5 syntax, clause 5.6
semantics) travels as payload id 11 inside Evolution frames (TrueHD extra data)
and EMDF containers (E-AC-3 skip fields). `crates/oadec-emdf/src/oamd.rs`
implements the whole syntax; `oadec oamd` walks a TrueHD stream and reports.

## Conventions pinned on real streams

* **Flag arrays are transmitted from element 0.** `content_description[]`,
  `obj_basic_info[]`, `obj_render_info[]`, `trim_balance_presence[]`,
  `ext_prec_pos_presence[]`, `bed_channel_assignment[]` and
  `nonstd_bed_channel_assignment[]` are read as unsigned integers; element `k`
  of an `n`-bit array is bit `n − 1 − k`. With the opposite reading the object
  element of every payload overruns its size.
* **Two presence arrays name their elements from the top.** The syntax asks
  `ext_prec_pos_presence[2]` for x and `[0]` for z, and `trim_balance_presence[4]`
  for the centre trim and `[0]` for the surround balance (table 34). By the rule
  above element 2 of three is bit 0, so x is bit 0 and z bit 2. These fields do not
  change how many bits are read, so no size check can tell, and neither can a
  pattern like `0b101`. The code had x at the top bit until the one stream in 144
  titles with extended precision showed otherwise: read that way 1 762 of its 4 052
  positions land outside the room, against none, and z moves with eight times the
  jitter. `truehdd` reads it the same way and agrees to the sample.
* **Element sizes are byte-tight.** `oa_element_size` covers the alternate id,
  the discard flag, the element and its padding; on 1,344,146 payloads of the
  six-film corpus every element ends within 7 zero bits of its declared size,
  which is the strongest available check that every field was read at the
  right width. The exception is the first payload of a segment, at the start of a
  stream or at a seamless branch, whose size belongs to some other payload: Shaun of
  the Dead declares 34 bytes for an object element that needs 47, and seven of 144
  TrueHD Atmos titles declare 9, 34 or 51 for one that fits in fewer, 4 to 28 bits past
  the end of the payload. This was once held to be a payload cut short, because `truehdd`
  warns about it, but that warning is the same size check. The content is all there:
  every such element ends less than a byte of zero fill before the end of its payload,
  and `truehdd` reads the same events. So the parser reads a last element either way and
  flags it (`size_ok = false`), and `verify` says so. That reading is earned per element:
  one past the payload is refused if its bits run out, if more than that fill follows it,
  or if its id has no syntax to ask. An element with another after it has to be right,
  because the next one starts where it ends.
* **Reuse never crosses payloads.** Block 0 of every object always carries a
  full update (status 01 is implied), so differential positions and reuse
  statuses only refer to the previous block of the same object in the same
  payload; the parser needs no state between payloads.
* **Beds come first.** Objects are ordered bed channels, ISF objects, dynamic
  objects. In a dynamic-object-only program with `b_lfe_present` the LFE is
  object 0 and counts as a bed channel (no render info is coded for it).

## Timing

`md_update_info` carries `sample_offset` (0, a table value 8/16/18/24, or 5
bits) and one `block_update_info` per update block (`block_offset_factor:6`,
`ramp_duration`). Per clause 5.3 the start of update `n` is
`sample_offset + 32 · block_offset_factor_n` samples after the first sample the
payload applies to; the interpolation to the new values takes `ramp_duration`
samples. Every stream of the corpus uses one block per payload with
`block_offset_factor` 0 or 1 and `ramp_duration` 1536 (the codec frame), one
payload per 1536 samples, `sample_offset` cycling through 0, 8, 16, 24.

## What a library carries

Six films say little about what a writer will ever meet, so
`tools/metadata_census.py` read the head of all 1 446 Dolby tracks of a library:
277 object tracks, 144 TrueHD and 133 E-AC-3
(`docs/audit/evidence/metadata-bridge-2026-11-09/01-metadata-census.json`).

| Field | Where it is found |
|---|---|
| Trim element | 130 of 133 E-AC-3, **0 of 144 TrueHD** |
| `warp_mode` | normal 78, warping 28, Lo/Ro 24, Pro Logic IIx 0 |
| Explicit trim configurations | 2 |
| Extended object element | 1, a TrueHD stream |
| Snap, screen reference, object gain, size, additional table data, alternate object data, `bed_chan_distribute`, unknown elements | 0 |
| Element ids 3 and 4 (`truehdd` names them headphone and object description) | 0 |

A head is a sample: a field that first appears later is not counted. Beside these
the E-AC-3 tracks carry EMDF payloads 1 (programme loudness, 372 tracks) and 2
(programme information, 140); `oadec emdf` reads both, and `docs/dolby-tools.md` says
what they are worth.

## What the corpus exercises

| Film | Payloads | Objects | Elements | Notes |
|---|---:|---:|---|---|
| Pi | 157,763 | 1 LFE + 11 | object only | one 32-sample ramp at the start |
| Talk to Me | 177,988 | 1 LFE + 15 | object only | |
| Shaun of the Dead | 186,209 | 1 LFE + 11 | object only | first element size wrong |
| Knives Out | 244,157 | 1 LFE + 11 | object only | one 1472-sample ramp |
| Braveheart | 333,078 | 1 LFE + 11 | object only | one 480-sample ramp |
| The King's Man | 244,951 | 1 LFE + 13 | object only | |

All updates are full (no reuse, no differential positions, no inactive
objects, no snap, no zone constraints, no screen reference, no distance, no
trim or extended elements). Those paths are covered by hand-built payloads in
the unit tests only.

## Oracles

The Dolby Reference Player's `--metadata-directory` writes a CSV of bitstream
and presentation information (DRC gains, presentation types, element counts)
but no per-object metadata, so it cannot check positions or gains. The
cross-checks for the object metadata are therefore truehdd's `.atmos.metadata`
events and the round trips through the Dolby encoder (see the DAMF notes).
The presentation types it prints ("Downmix of 6-channel presentation",
"Independent") match `oadec info`.
