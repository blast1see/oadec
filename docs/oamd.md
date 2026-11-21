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

## Intermediate spatial format

A programme can carry intermediate spatial format (ISF) objects between its
beds and its dynamic objects: signals for loudspeaker positions on stacked
rings, middle (M), upper (U), lower (L) and zenith (Z). What a format is
called, how many channels each ring has and in what order the channels come
is public; where a ring is, and where a channel sits on it, is not.

| Source | What it says | What it does not |
|---|---|---|
| ETSI TS 103 420 V1.2.1, 5.6.0.11, table 11b | `intermediate_spatial_format_idx` 0 to 5: SR3.1.0.0, SR5.3.0.0, SR7.3.0.0, SR9.5.0.0, SR7.5.3.0, SR15.9.5.1 (4, 8, 10, 14, 15 and 30 objects); 6 and 7 reserved. The objects in MULZ order, `M1 M2 M3 U1` to `M1 … M15 U1 … U9 L1 … L5 Z1` (notes 1 and 2: "SR" is stacked ring, MULZ the order the signals come in). | Any azimuth, elevation or coordinate. |
| Dolby TrueHD High-Level Bitstream Description (2018), 4.4, tables 17 and 19 | `16ch_intermediate_spatial_format` of the major sync: `010b` BH7.3.0.0 (10 channels), `011b` BH9.5.0.0 (14), `100b` BH7.5.3.0 (15); `000b`, `001b` and `101b` to `111b` reserved. The channels in M.U.L.Z. order, after the loudspeaker feeds and before the dynamic objects; together they are the channels of `16ch_channel_count`. The example: content `0111b`, C and LFE, `010b`, two objects is "C LFE M1 M2 M3 M4 M5 M6 M7 U1 U2 U3 Obj1 Obj2". | Any coordinate. |
| US 9,807,538 B2 (Dolby), figures 4 and 5 | The rings and the BH notation; the channels of a ring "uniformly spread", channel 1 at 0° (front), the rest anticlockwise, azimuth (n − 1)/N · 360°. | Any ring elevation. It is a patent's example ("one form of speaker arrangement"), and its own list for BH9.5.0.1 does not match its counting rule. |
| US 11,322,164 B2 and US 2018/0254047 A1 (Dolby) | The BH notation; channels "may be uniformly distributed"; BH3.1.0.0 "approximately" C, Ls, Rs and zenith. | Any number. |

So `oadec` names the ISF channels and gives their samples, and puts them nowhere:

* `decode` writes the 16-channel presentation with the ISF channels labelled
  `M1` … in MULZ order after the bed; the dynamic objects after them stay
  `obj1` …. WAVE has no speaker bit for an ISF channel, so the channel mask
  is unassigned, as it is for objects.
* `info` names the format and its channels (`ISF SR7.5.3.0 M1 … L3 (15)`),
  and `info --json` gives `sixteen_channel.isf`: index, name, whether the
  value is reserved, channels and labels. A stream without ISF channels has
  no such field.
* `oamd` and `metadata` give the name and the objects of the ISF type of the
  programme (`ISF objects:`), all six of table 11b; the payload parser counts
  thirty objects for SR15.9.5.1 though no 16-channel presentation can carry
  them.
* DAMF and the Dolby Atmos master ADM profile have no place for them:
  `--isf error` (the default) refuses the output, `--isf drop` writes the beds
  and dynamic objects without them and declares the loss. There is no
  `--isf objects`, and a valid ADM file is no evidence that ISF channels were
  placed or converted without loss.
* The major sync can declare only the three formats of table 19. A stream that
  declares 0, 1 or 5, which TS 103 420 gives to other formats, is read as one
  that declares 6 or 7: it decodes, `info` says
  `ISF 16ch_intermediate_spatial_format 0 (reserved)`, and DAMF and ADM refuse
  it with that name. A major sync whose bed, ISF channels and dynamic objects
  do not come to `16ch_channel_count` is refused by DAMF and ADM by name.

The streams `crates/oadec-cli/tests/cli_isf.rs` writes with `oadec-synth`
(SR7.3.0.0 alone and with C, LFE and two objects, SR9.5.0.0 with LFE and an
object, SR7.5.3.0 with LFE) decode to the samples they were written from;
`truehdd` gives the same samples of the 16-channel presentation. FFmpeg's
decoder reads no 16-channel presentation, and Dolby's decoder, without an
Evolution frame of object metadata in the stream, gives two channels; neither
was a check of these.

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

## What an encoder writes

The streams Dolby's encoders made of 51 authored scenes, each five ways
(`tools/interop.py`; `docs/dolby-tools.md`, *A matrix of round trips*), say the same
as the library of the fields above and add what a library cannot: what went in.
None of 269 streams, and none of the 66 of the probes beside them, carries an
object gain, a size, a lock or a zone, though the masters of 35 of the 51 rows
said one of them of an object, as 27 of the 33 probes did: the encoder renders the
lock, the zone and the size and drops the gain (`docs/dolby-tools.md`, *What the
encoders do with what a master says of an object*). The programme is an LFE and
dynamic objects in every stream. Every E-AC-3 stream has the trim element and no
TrueHD one has; an E-AC-3 payload has a sample offset of 0 and a block offset
factor of 0 where a TrueHD one goes round 0, 8, 16 and 24 with a factor of 0 or 1;
and a ramp is 1536 samples but for one shorter one in 6 E-AC-3 streams of 51 and in
49 TrueHD ones of 51.

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

## Field by field: what reaches DAMF and ADM

*Draft of round 5 (P3, stop 1). Nothing below changes what oadec writes; it says
what it writes today and what the loss ledger says of it. "Not measured" means
no evidence file of this repository answers the question.*

Eight questions per field: how the bitstream carries it; whether oadec parses it
right; whether the programme model (`crates/oadec-spatial/src/program.rs`) keeps
it; whether DAMF can carry it and whether the ADM profile (Dolby Atmos Master ADM
Profile v1.0, `docs/audit/evidence/adm/01-spec-extracts.json`) can; what Dolby's
tools do with it; whether the loss can be avoided or is a limit of the format;
and whether the ledger (`crates/oadec-spatial/src/loss.rs`, `--loss-report`,
`docs/schemas/loss-report.schema.json`) reports it. Line numbers are of the
commit that adds this section; `oamd.rs` is `crates/oadec-emdf/src/oamd.rs`, the others are in
`crates/oadec-spatial/src/`.

| Field | Bitstream (TS 103 420) | Parsed | Model | DAMF | ADM | Dolby tools | Avoidable? | Ledger |
|---|---|---|---|---|---|---|---|---|
| Position, extended precision | `object_render_info` 6+6+1+4 bits or a 3-bit differential; `ext_prec_pos` steps in element 5 | yes, `oamd.rs:612-633`, steps `oamd.rs:940-950`; the bit order of `ext_prec_pos_presence` measured (above) | `ObjectState.pos`, `program.rs:350` | `pos`, exact (`damf.rs:410`) | cartesian X/Y, Z omitted at 0 (`adm.rs:722-731`) | what sounds alone comes back at its place, 296 of 296 (`dolby-tools.md`, *A matrix of round trips*) | nothing is lost | nothing to report |
| Distance | `b_object_distance_specified`, at infinity or `distance_factor` (table 15) | yes, `oamd.rs:634-641` | **no**: dropped in `Timeline::push`, `program.rs:562-580` | no key known | `absoluteDistance` shall not be used | not measured; no stream of the library or of the 269 encodes carries one | profile limit; BS.2076 has `absoluteDistance` (outside the profile) | `distance-dropped` (unrepresentable), `program.rs:578`, per event and per change |
| Divergence | element 5 `object_divergence`, tables 41/42 | yes, `oamd.rs:910-931` | **no**: `program.rs:582-592` | no key known | `objectDivergence` shall not be used | not measured | profile limit; BS.2076 has `objectDivergence` | `divergence-dropped` (unrepresentable), `program.rs:590` |
| Trim configurations | trim element, `global_trim_mode == 2`, nine configurations | yes, `oamd.rs:833-865` | **no**; only the per-object bypass (below) | trim keys accepted, encoder output unchanged in every layout tried (`02-damf-key-probe.json`) | no field | `atmos_info` reports auto trims whatever the set says | a format limit as far as measured | `trim-config-dropped` (unrepresentable), once per change, `program.rs:508` |
| Trim bypass | `b_disable_trim_per_obj`, or `global_trim_mode == 1` | yes, `oamd.rs:867-875` | `trim_bypass`, `program.rs:490-497` | `trimBypass`, exact (`damf.rs:425-431`); whether an encoder reads it: not measured | no field | not measured | profile limit | `trim-bypass-dropped` (profile reduction) for objects, `adm.rs:844`; **a bed's first state is not counted** (below) |
| Gain | `object_gain`: 0 dB, mute, 6-bit dB, or the previous object's | yes, `oamd.rs:520-529` | `gain`, exact | `gain` in dB | as Dolby's converters write it, ten decimals of the float32 factor (`adm.rs:715`) | the encoders drop it; the Conversion Tool carries -6 dB as `0.5011872053` (`adm-remediation-dolby.json`) | nothing lost by oadec | nothing to report |
| Gain ramp | `ramp_duration` (table 25) or 11 bits | yes | `ramp`, exact | `rampLength`, exact | 250 samples after the first block (profile table 11), or the source with `--adm-interpolation real` (outside the profile) | Dolby's converters write 0 then 250 (`adm-dolby-reference.json`, S1) | profile limit | `ramp-replaced` (profile reduction) with the source lengths, `adm.rs:704` |
| Importance | `object_priority`, 5 bits or 1.0 | yes, `oamd.rs:531-536` | `importance`, exact | `importance` | only the inactive marker `<importance>0</importance>` | the Conversion Tool writes none on active objects (S3) | profile limit | `importance-omitted` (profile reduction), `adm.rs:838`, active objects only; **a bed's first importance is not counted** |
| Size, three axes | `object_size_idx` 1 (one value) or 2 (three) | yes, `oamd.rs:649-661` | three axes, `program.rs:267` | one `size`, the width (`damf.rs:421`) | `width`=`depth`=`height` of the width (`adm.rs:733-737`); the profile requires the three equal | the encoders render a size into elements; the Conversion Tool writes the same triple and `diffuse 1`; a three-axis size: not measured | DAMF: format limit; ADM: profile limit, three axes outside it | `size-axes-collapsed` (approximation), both writers, per event |
| Screen reference | `b_object_use_screen_ref`, `screen_factor` 3 bits, `depth_factor` table 16 | yes, `oamd.rs:664-672` | as `screen_factor` (0 when absent) and `depth_factor` (0.25 when absent), `program.rs:359-360`; no flag of its own, though `screen_factor` is never 0 when present | `screenFactor`, `depthFactor` (`damf.rs:437-452`); whether an encoder reads them: not measured | `screenRef` shall not be used | not measured; no stream carries one | ADM: profile limit; DAMF: carried as far as oadec knows | ADM: `screen-reference-dropped` (profile reduction), `adm.rs:841`; DAMF: nothing, and nothing is lost unless an encoder ignores the keys |
| Snap | `b_object_snap` | yes, `oamd.rs:673` | `snap` | `snap` | `channelLock` 1 (`adm.rs:740`) | the encoder renders the lock into a place (`dolby-tools.md`, *What the encoders do*) | nothing lost | nothing to report |
| Zones 0 to 5, elevation | `zone_constraints_idx` 3 bits (table 20), `b_enable_elevation` | yes, `oamd.rs:645-646` | `zones`, `elevation` | `zones` by name, `elevation` | zone rectangles of profile tables 12 and 13 (`adm.rs:746-759`) | the encoder renders a zone into places; the Conversion Tool's zones agree (harness C17) | nothing lost | nothing to report |
| Zones 6 and 7 | reserved values of table 20 | read as given | kept as 6 or 7 | written `zones: all` (`program.rs:245`) | an empty `<zoneExclusion>`, or only the elevation zones (`adm.rs:746`) | not measured | a reading of a reserved value | **nothing**; proposed `reserved-zone-read-as-all` (approximation) |
| Warp mode | trim element `warp_mode`, 2 bits | yes, `oamd.rs:833` | per payload, `program.rs:498-505` | `warpMode` with `--warp-mode stream`, none by default | `dbmd` segment 9 byte 152 with `--warp-mode stream` | measured: `02-damf-key-probe.json`, `03-warp-roundtrip.json`, `04-warp-render.json` (`dolby-tools.md`, *The warp mode*) | avoidable on request; the default keeps its bytes | `warp-mode-dropped` (approximation), once per change, `program.rs:503` |
| ISF | `content_description` bit 1, `intermediate_spatial_format_idx` (table 11b) | yes, `oamd.rs:323-329` | counted (`Program.isf_objects`), no position: `program.rs:533` | none: refused, or left out with `--isf drop` | none: likewise | none | not avoidable: no public source places an ISF channel (above) | `isf-dropped` (declared loss, exit 4), `damf.rs:150`, `adm.rs:293` |
| `dbmd` chunk | not in OAMD; ADM only | n/a | n/a | n/a | segment 7 constant; segment 9 tool names, bed mask and warp mode; segment 10 channel flags; the rest copied from a Dolby conversion (`dbmd.rs`) | learned from two Conversion Tool outputs; the meaning of the copied bytes is not public | unknown (UNK) | not a ledger matter; no byte is made up |
| `additional_table_data` | per object info block, `b_additional_table_data_exists`, 4-bit size and bytes | yes, `oamd.rs:763-768` | **no** | no place | no place | not measured; census 0 | format limit | **nothing**; proposed `additional-table-data-dropped` |
| Alternate object data | `b_alternate_object_data_present`, `alternate_object_data_id_idx` per element | yes, `oamd.rs:1095-1099` | **no**: the first object, trim and extended elements are read (`oamd.rs:1182-1206`), the others are not | no place | no place | not measured; census 0 | format limit | **nothing**; proposed `alternate-object-data-dropped` |
| Elements 3, 4 and reserved ids | `oa_element_id_idx` without a syntax here (`truehdd` names 3 and 4 headphone and object description) | kept as bytes, `oamd.rs:1018-1028` | **no** | no place | no place | not measured; census 0 | format limit | **nothing**; proposed `unknown-element-dropped` |
| `b_bed_chan_distribute` | programme assignment, with beds | yes, `oamd.rs:293` | **no** (`Program` keeps the beds only) | `bedDistribution` accepted, encoder output unchanged (`02-damf-key-probe.json`, on a set whose bed is the LFE alone, so not conclusive) | no field | see DAMF | a format limit as far as measured | **nothing**; proposed `bed-distribute-dropped` |
| `reserved_data`, object element `reserved` | `content_description` bit 3, 4-bit size and bytes; 5 reserved bits | yes, `oamd.rs:337-340`, `oamd.rs:711-715` | **no** | no place | no place | census 0 | format limit | **nothing**; proposed `reserved-data-dropped` |

What the ledger does not say today, and what stop 2 adds, all of classes that
leave the exit code at 0 (`crates/oadec-spatial/tests/ledger_completeness.rs`
holds the red tests):

- the six fields marked **nothing** above, each a kind of its own;
- the first state of a bed channel in ADM: a DirectSpeakers block has no
  importance and no trim bypass, and `bed-gain-dropped` asks only of the gain
  and of activity (`adm.rs:412-418`), so an LFE whose importance is not 1 or
  whose trim is bypassed loses it without a line. They go under
  `importance-omitted` and `trim-bypass-dropped`.

None of the committed fixtures carries any of these fields, so no recorded loss
report changes with them (`crates/oadec-cli/tests/loss-golden/`).
