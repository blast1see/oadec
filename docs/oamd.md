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
* **Element sizes are byte-tight.** `oa_element_size` covers the alternate id,
  the discard flag, the element and its padding; on 1,344,146 payloads of the
  six-film corpus every element ends within 7 zero bits of its declared size,
  which is the strongest available check that every field was read at the
  right width. One exception: the very first payload of Shaun of the Dead
  declares 34 bytes for an object element that needs 47. The parser tolerates
  an overrun of the last element (flagged as `size_ok = false`).
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
