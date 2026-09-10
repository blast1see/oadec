# Decoder remediation

What the conformance audit of 2026-09-10 found, what was done about it, and what
is still open. The audit itself is in this directory, committed unedited as the
before-state; where its numbers move, this report says so and the audit stays as
the record of what was measured on the day.

Two defects were demonstrated by the audit and both are fixed. A third turned up
while closing one of the coverage gaps, a fourth while measuring what was left
of the third, and a third fault in the same clause as the third defect turned up
when the material that had never been decoded finally was. One of the audit's
own conclusions turned out to be wrong as well. Beyond them the audit left a number of areas untested
rather than failing, and those are treated as what they are: coverage gaps,
unknown proprietary behaviour, or reference-decoder disagreement, each pursued
on its own terms and none of them promoted to a pass without evidence.

Evidence is under `evidence/remediation/`. Every status change points at a file
there.

---

## Defect 1 — E-AC-3 dependent substreams were never decoded

### Symptom

A real Dolby Digital Plus 7.1 programme decoded to 5.1 and `verify` called the
file clean. On a 60-second cut of the 1917 remux's secondary track:

| Decoder | samples | channels |
|---|---:|---:|
| FFmpeg | 2 880 000 | 8 |
| oadec | 2 880 000 | 6 |

`verify` printed `Substreams: type 0 id 0: 1875, type 1 id 0: 1875` — it counted
the dependent substream — and then `Result: CLEAN`, exit 0. Forty eight-channel
tracks in the library sweep had been recorded clean on that basis.

### Reproduction

`evidence/remediation/dependent-substream-before-after.json`, produced by
`tools/ec3_structure.py` and `tools/pairing.py`, both written to read the
bitstream independently of the decoder so their account is a check on oadec
rather than a restatement of it.

The structure walker reads 1 875 frame groups, each an AC-3 5.1 core at
640 kbit/s of 2 560 bytes immediately followed by an E-AC-3 dependent substream
with `strmtyp` 1, `substreamid` 0, `acmod` 5 (four coded channels), `lfeon` 0,
`chanmape` 1 and `chanmap` 0x1a00, of 3 584 bytes.

The pairing matrix compares every oadec channel against every FFmpeg channel
rather than trusting position, and that turned out to matter. The audit read the
surround residual as a rear-pair match; the matrix shows oadec's `Ls` pairs with
FFmpeg's `SL` at 16.86 dB and sits at −16.49 dB against `BL`. What oadec emitted
there was the core's matrixed surround, and the rear channels were simply absent.
L, R, C and LFE came out at 63.80, 65.86, 53.39 and 141.94 dB — the audit's own
figures — so the two agree wherever the decode was whole.

### Root cause

Five of six frame-consuming paths dropped every dependent frame before it was
parsed:

```rust
if header.stream_type == StreamType::Dependent || header.substream_id != 0 {
    return Ok(());
}
```

`Bsi::chanmap` was parsed and had no consumer anywhere in the repository.
`is_clean` did not consider the dependent substream count, so a truncated
programme was clean by construction. `oadec-eac3`'s `Decoder` is per-substream
by design and there was no type above it that could hold a programme.

### What the specification requires

- **E.1.3.1.2** — "Dependent substreams shall immediately follow the independent
  substream with which they are associated." Association is bitstream position;
  ids are assigned sequentially in that order; an AC-3 stream inside an
  Enhanced AC-3 stream is independent substream 0.
- **E.2.8.2** — with `chanmape` clear the dependent substream's `acmod` and
  `lfeon` name its channels and those overwrite the corresponding channels of
  the independent substream; with it set, `chanmap` names them and matching
  locations replace while the rest are additional. At most 16 channels per
  programme.
- **E.1.3.1.8, table E.1.4** — the channel map, bit 0 in the most significant
  bit, pair bits standing for two adjacent coded channels, the location count
  equal to the coded channel count.
- **E.2.8.3** — a second independent substream is a second programme; the
  default is programme 1 and the rest are skipped.
- **TS 103 420 clause 8.2** — with dependent substreams present, the EMDF
  container carrying OAMD and JOC is in the last dependent substream.

### Implementation

`crates/oadec-eac3/src/program.rs`, new. `ChannelLoc` names a channel location
and carries three derived facts with it — the WAVE mask bit, the interchange
rank, the JOC downmix input — so the three cannot drift apart the way three
separate `match` ladders on channel-name strings eventually would.
`ProgramLayout::merge` is clause E.2.8.2 and nothing else. `ProgramDecoder`
keeps one `Decoder` per substream and pairs their output by **frame group
index**, not by arrival: enhanced coupling makes a decoder hold a frame back and
transient pre-noise processing holds samples back, so a substream using either
releases a group later than one that does not, and pairing by arrival would
misalign the programme the moment an encoder switched a tool on in the core and
not in the dependent substream.

The merge is a permutation, so no samples are copied; a programme channel is a
borrow of one substream's decoded channel. A group whose channel set differs
from the programme's is reported, counted and silence-filled rather than acted
on, because `decode` writes the WAVE header from the first group and afterwards
only patches its length.

Interchange order became the order of the WAVE mask bits. Today's hand-written
ranks were already in that order, so the rewrite changed the numbers and not the
permutation — every existing output stayed byte-identical — and 7.1 fell out
without a further decision: Lrs and Rrs at 0x10 and 0x20 sort ahead of Ls and Rs
at 0x200 and 0x400, giving FL FR FC LFE BL BR SL SR and mask 0x63f, which is
what FFmpeg produces for the same stream.

`--core-only` writes the independent substream's channels alone, which is what a
decoder limited to 5.1 produces and clause E.2.8.2 allows.

**One documented deviation.** Clause E.1.3.1.8 requires the map to name exactly
as many channels as the substream codes. If it names the full-bandwidth channels
of a substream that also carries an LFE, oadec appends the LFE and flags the
frame rather than refusing the file: whether the map counts the LFE is not
settled by any stream on hand, and a whole file is not worth failing over it.
Every other count mismatch is an error. The fallback has not fired on any stream
measured.

### The risk this had to retire first

Dependent-substream `bsi` and `audblk` syntax had never executed on real
material, because the filter sat before `Frame::parse`. That matters more than
it sounds: `Frame::parse` seeks the audio blocks to the bit position the `bsi`
ended at, so one bit wrong in the `chanmape`/`chanmap` read does not give a
slightly wrong frame, it gives a garbage frame — and the frame CRC will not
catch it, because the CRC is over the bytes and never touches the parse.

So the merge was split from the parse. One commit decoded the dependent
substreams, merged nothing, and reported: 1 875 frames, 0 CRC failures, 0 decode
errors, and **18 to 1 281 bits unread** per frame. The floor of exactly 18 is the
mandatory `auxdatae` and error check that closes every syncframe, and the ceiling
sits inside what a known-good E-AC-3 substream shows. `eac3-blocks --frame 0
--part 1` read one of those frames by hand: `chanmap` 0x1a00, four coded
channels, 217 coefficients each, sensible exponents. Only then was the merge
wired up.

### Reference result

`evidence/remediation/dependent-substream-before-after.json`. All eight
programme channels pair with FFmpeg's, each beating its runner-up by at least
53 dB:

| Channel | Reference | Before | After |
|---|---|---:|---:|
| L | FL | 63.80 dB | 63.80 dB |
| R | FR | 65.86 dB | 65.86 dB |
| C | FC | 53.39 dB | 53.39 dB |
| LFE | LFE | 141.94 dB | 141.94 dB |
| Ls | SL | 16.86 dB | **66.61 dB** |
| Rs | SR | 16.79 dB | **67.56 dB** |
| Lrs | BL | absent | **59.92 dB** |
| Rrs | BR | absent | **60.12 dB** |

`decode --format pcm` gives 92 160 000 bytes, which is 2 880 000 samples of
eight 32-bit channels. `--core-only` reproduces the pre-change 5.1 decode byte
for byte.

### More than one title

`evidence/remediation/ddp71-channel-compare.json`, from `tools/ddp71_sweep.py`
over all 40 eight-channel E-AC-3 tracks in the library: ten seconds from ten
minutes into each, because the first minutes of a film are often digital silence
and two decoders' dither is not a measurement.

All 40 decode to eight channels and FFmpeg agrees on the count for every one. Of
320 channel comparisons, 271 matched by at least 10 dB, 5 won by less, 44 were
silent, and **none was mismatched**. The five narrow cases are content rather
than placement: Jurassic Park's two height channels carry nearly the same signal,
and Vertigo's surrounds sit between −66 and −76 dBFS where most of what is
compared is the two decoders' noise.

Two custom channel maps occur in the wild, and the second was a surprise:

| `chanmap` | locations | programme | titles | FFmpeg says |
|---|---|---|---:|---|
| 0x1a00 | Ls, Rs, Lrs/Rrs | 7.1 | 35 | 7.1 |
| 0xa010 | L, R, Vhl/Vhr | 5.1.2 | 5 | 5.1.2 |

A track whose file name says 7.1 is quite capable of being 5.1.2. The channel
map, not the title, says what a dependent substream carries.

### Tests added

`a_seven_one_programme_decodes_to_eight_channels` in the media suite requires
the eight channels by name, a clean verify with every failure counter at zero,
both substreams named in the programme report, eight channels of PCM from
`decode` and six from `--core-only`. Twelve unit tests in `program.rs` cover the
channel-map table against its names, all sixteen `(acmod, lfeon)` pairs against
a hardcoded table of the previous output, the measured 0x1a00 expansion, the
merge and its sources, the sixteen-channel cap, and the interchange order and
mask of a 7.1 programme.

### Remaining limitation

Multiple dependent substreams per programme, and dependent substreams of a
second programme, are implemented and unexercised: every stream measured has
exactly one dependent substream, which is also what TS 103 420 clause 9 allows
for object audio. The LFE fallback above is a documented reading, not a
confirmed one.

---

## Defect 2 — corruption never reached the exit code

### Symptom

The audit injected 120 single-bit errors into a JOC stream and 100 into a
TrueHD stream.

| Path | trials | non-zero exit | flagged but exit 0 | silent |
|---|---:|---:|---:|---:|
| `verify` (JOC) | 120 | 120 | 0 | 0 |
| `decode --format pcm` (JOC) | 120 | 21 | 99 | 0 |
| `decode --format damf` (JOC) | 120 | 21 | 0 | **99** |
| `verify` (TrueHD) | 100 | 100 | 0 | 0 |
| `decode -p 3` (TrueHD) | 100 | 95 | 3 | **2** |

Ninety-nine corrupted streams became Atmos objects and object metadata with
nothing said and exit 0.

### Reproduction

`evidence/remediation/decode-exit-codes.json`, from `tools/replay_fuzz.py`.

The audit kept only totals, but both site lists are recoverable: `fuzz2.py` drew
them from `random.seed` over the file size, and the seeds are 1 for the JOC run,
as recorded in the evidence, and 7 for the TrueHD run, found by searching for
the seed whose hundred sites contain both stored silent accepts. The 99 sites
stored in `evidence/09-01-fuzz-joc.json` are exactly the 99 this replay finds
silent, with no overlap with the 21 that exit non-zero, so the campaign is
reproduced site for site rather than in aggregate.

### Root cause

Detection was never missing. It was expressed as booleans on parse structs and
then re-derived, differently and more narrowly, by each command that delivered
something.

```text
E-AC-3
  frame::crc_ok() -> Frame.crc_ok -> Decoded.crc_ok        (never an Err)
      -> eac3.rs account()  -> Pass.crc_failures -> is_clean() -> verify exit 7   OK
      -> eac3.rs decode()   -- is_clean NOT CALLED; only decode_errors gated      LOST
      -> eac3.rs compare()  -- crc_failures printed, not in the verdict           LOST
      -> eac3_objects.rs    -- d.crc_ok NEVER READ; no counter existed            LOST
  for_each_frame -> (frames, sync_errors, skipped)
      -> eac3_objects.rs    -- tuple discarded                                    LOST

TrueHD
  Extractor -> ExtractStats{resyncs, skipped_bytes, major_sync_crc_failures}
      -> input::for_each_unit -> PassSummary
              -> scan::scan     -- kept -> Failures -> verify exit 7              OK
              -> decode::run    -- DISCARDED                                      LOST
              -> damf::run      -- DISCARDED                                      LOST
              -> compare::run   -- DISCARDED                                      LOST
  Segment::parse -> parity_ok, crc_ok, end_ok, sample_count_ok, tail_ok
      -> scan.rs        -- eight separate counters -> Failures                    OK
      -> truehd/decoder -- collapsed into one `segment_problems` no caller gated  LOST
  Decoder -> DecodeStats{lossless_mismatches, segment_problems, ...}
      -> decode.rs / damf.rs -- ONLY lossless_mismatches reached `bail!`          PARTIAL

EMDF / OAMD / JOC
  parse errors -> typed Err
      -> eac3.rs, emdf.rs, oamd.rs, damf.rs -- counted with their text            OK
      -> eac3_objects.rs -- `Err(_) => errors += 1`, one opaque total that
                            never gated the exit                                  LOST
  emdf_protection words -- parsed, never verified anywhere                        OPEN
```

### Implementation

`crates/oadec-cli/src/integrity.rs`. `Findings` collects the faults, `report`
prints them, and every path that delivers audio or metadata returns whether it
was clean. The list of faults is the one `verify` already used, so the two
cannot drift.

`docs/exit-codes.md` writes the policy down: 0 clean, 2 fatal, 7 an integrity
fault affecting what was delivered. Output is still written — throwing away
audio that decodes is the wrong trade, and this project already declined it once
over a frame in *The 400 Blows* that FFmpeg, Dolby and oadec all agree on. What
changed is that the run says what it found and does not exit clean. There is no
flag to turn the verdict off.

Two things are deliberately not faults: a second programme, which is legal and
should be skipped, and a frame that ends inside its own tail.

### Reference result

`evidence/remediation/decode-exit-codes.json`, both campaigns replayed site for
site:

| Path | silent before | silent after | non-zero before | non-zero after |
|---|---:|---:|---:|---:|
| `decode --format pcm` (JOC) | 0 | 0 | 21 | **120** |
| `decode --format damf` (JOC) | **99** | **0** | 21 | **120** |
| `decode -p 3` (TrueHD) | **2** | **0** | 95 | **100** |

No panics in either campaign, before or after.

Consistency was the claim, and it is measurable: across the 33 AC-3 and E-AC-3
streams in the work directory, 29 decode exit codes moved from 0 to 7, every one
of them on a file `verify` already called non-conformant. No `verify` result
changed and every byte of PCM and DAMF output is identical.

### Tests added

`corrupted_joc_never_decodes_to_a_silent_success` replays the 99 recorded sites
and requires every one to exit non-zero without panicking.

### Remaining limitation

EMDF protection words are still parsed and not verified. They are a second,
independent check on the container and would catch corruption the frame CRC
misses; nothing uses them yet.

---

## Defect 3 — sparse JOC matrices, found while closing a coverage gap

Not in the audit. The audit filed sparse mode as untested — the literal reading
matched the printed pseudo-code, no stream in the corpus used it — and closing
that gap turned it into a demonstrated defect.

### Symptom

Frames carrying sparse matrices decode to objects that bear almost no relation
to Dolby's. Per-object median distance on the first sparse material found, the
three frames of two Extraction clips, against the frames either side of them:

| Frame | oadec | frames either side |
|---|---:|---:|
| Extraction 63 325 | 1,05 dB | 38-48 dB |
| Extraction 208 315 | −3,23 dB | 55-58 dB |
| Extraction 208 333 | −13,48 dB | 54-56 dB |

Seven more sparse frames were cut later — one more from Extraction and the
whole of Glass Onion's and Red Notice's — and they behave the same way:
−16 to +6 dB where the same clips run at 52 to 60 elsewhere.

### Why the audit could not see it

It scanned head clips. Sparse matrices occur 150 times in 32 493 245 object
updates across the whole library, only in streaming material, tens of thousands
of frames in. Three megabytes off the front of fifteen clips contains none of
it, and neither does any other head clip. `tools/joc_coverage.py` reads whole
files; `verify --json` reports the frames where each rare branch occurs, and
`tools/ec3_cut.py` turns one of those frame numbers into a clip that starts on
a syncframe boundary and contains it.

### Root cause

Clause 6.6.2 pseudo-code 2 is wrong in three places, and each hides the next.

It puts the value `offset`, 50 coarse or 100 fine, on the channels a band does
not select. Clause 6.6.4 dequantises with `(q - nquant/2) * 820 /
(4096 (1 + quant_idx))`, so 50 and 100 both come out at 0,4004 — every
unselected channel contributes four tenths of a downmix channel to the object,
which is the opposite of what a sparse representation is for. The code that
means zero gain is 48/96, the same value dense mode starts from.

It forms the channel index of band `pb` from the **transmitted** previous
value rather than the resolved one. That reading was already in the source,
behind `SparseIndexMode::Cumulative`, unused and unsettled — because on its own
it is worth nothing measurable, the off-channel error swamping it entirely.

And it accumulates the selected channel's coefficient from
`joc_mix_mtx_q[obj][dp][ch][pb-1]` — the same channel's previous band — which
is the offset whenever the previous band selected a different channel. So the
printed reading restarts the chain at the offset every time the channel moves.
Dolby's runs it unbroken across the bands, whatever channel each one lands on,
the way dense mode's per-channel chain runs unbroken across its own. This one
only became visible after the first two, and it is what the material from the
two titles that had never been decoded shows most clearly.

The seed of the chain is the printed 50/100 and stays there. Reading it as
48/96, so that sparse and dense agree on where they start, is the obvious
fourth correction and it is wrong by 50 dB: one constant, two uses, and only
one of the two misprinted.

### Reference result

`evidence/remediation/sparse-differential.json`. **Every** sparse frame in the
library is measured: ten frames, 150 object updates, three streaming titles, five
clips cut around the frames `verify --json` names. Glass Onion and Red Notice
had never been decoded against Dolby on their sparse frames at all, and one of
Extraction's four had been missed; the first round of this work had three of
the ten.

The figure below is the worst sparse frame of a clip against that same clip's
median away from its sparse frames, so the comparison is with the ordinary
frames of the same decode of the same material:

| Clip | Sparse frames | as printed | + zero gain, resolved index | + unbroken chain |
|---|---|---:|---:|---:|
| Extraction A | 1 | −36,9 dB | −13,8 dB | **−9,9 dB** |
| Extraction B | 2 | −71,3 dB | −12,1 dB | **−4,7 dB** |
| Extraction C | 1 | −46,3 dB | −19,7 dB | **−5,7 dB** |
| Glass Onion | 5 | −75,2 dB | −50,1 dB | **−9,2 dB** |
| Red Notice | 1 | −58,7 dB | −10,7 dB | **−4,9 dB** |

Glass Onion is where the third correction shows: two corrections leave its five
frames 50 dB below their clip, all three bring them to 9. In absolute terms the
ten sparse frames end at 34,6 to 68,4 dB against clip levels of 44 to 60.

Frames carrying no sparse object are bit-identical under all three readings,
and all 33 work streams decode to byte-identical PCM and DAMF.
`--sparse-as-printed` keeps the printed reading reachable so the difference
stays measurable.

Coarse quantisation, tested the same way on a clip carrying 75 coarse objects,
was already right: 50 to 57 dB, unchanged.

### Remaining limitation

One frame of the ten is still the worst: 34,6 dB where its clip sits at 44,4
and its immediate neighbours at 38,5 and 48,2. It is the frame this
investigation started from, and it has moved three times — 16,5 dB after the
first two corrections, 30,6 after defect 4 below, 34,6 after the third. What is left
is confined to two QMF time slots, at the point where the sparse matrix is
applied, and that frame is the only one in the library that is both sparse and
steep with a large offset. Open.

### What is still unexercised

Across 32 493 245 object updates in 31 JOC streams: **two data points never
occur**, so two of the four branches of clause 6.6.5 pseudo-code 6 — smooth
with two points and steep with two points — have never run on real material.
Downmix configurations 1, 2 and 4 never occur; 28 streams use configuration 3
and three use 0. No EMDF container was found in `auxdata` in any of 49 streams.

---

## Defect 4 — the steep interpolation switched one time slot too late

Not in the audit either, and not in the list of things this work set out to do.
It was found while measuring what was left of defect 3: the frame that would not
come right is the only frame in that clip carrying steep objects, and the
question "what else is different about it" had one answer.

### Symptom

On frames where an object's matrix changes sharply, the objects diverge from
Dolby's for one QMF time slot, and the filter bank spreads that slot over about
ten. On the worst frames measured the per-object median falls to 23 to 34 dB
where the frames either side are 60 to 65.

### Root cause

Clause 6.3.4.4 defines `joc_offset_ts = joc_offset_ts_bits + 1`. The offset is
one-based: the smallest value the five transmitted bits can carry names the
first time slot. The `ts` of clause 6.6.5 counts from zero, and its pseudo-code
compares the two directly:

```
if (ts < joc_offset_ts[obj][0]) {
  joc_mix_mtx_interp[obj][ts][ch][sb] = joc_mix_mtx_prev[obj][ch][sb];
}
```

so the previous matrix is held for one slot too many. Dolby's decoder switches
at the slot the offset names, which is `ts < joc_offset_ts - 1`. Nothing else
about the syntax changes: the parse still reports `joc_offset_ts` exactly as
clause 6.3.4.4 defines it, and `verify --json` still counts the transmitted
values.

### Reproduction and reference result

`evidence/remediation/steep-offset-differential.json`. Each clip is decoded
twice from the same binary, once with `--steep-as-printed` and once without,
and both are compared per object against Dolby's object decoder in the window
`tools/objcmp.py` already used.

Eight titles. Four carry steep objects; the other four are the negative control
and are bit-identical under the two readings, because a stream with no steep
object cannot reach the code that changed.

| Title | Steep objects | Worst, as printed | Worst, corrected | Median |
|---|---:|---:|---:|---|
| Glass Onion | 195 | 25,24 dB | **49,93 dB** | 47,76 → 65,44 |
| Shaun of the Dead | 30 | 40,75 dB | **43,62 dB** | 46,79 → 53,79 |
| Red Notice | 105 | 36,21 dB | 36,21 dB | unchanged |
| Extraction | 30 | 50,51 dB | 50,51 dB | unchanged |

Per frame it is sharper. Of Glass Onion's thirteen frames with steep objects,
four were damaged and nine were already right:

| Frame | as printed | corrected |
|---|---:|---:|
| 752 | 48,50 dB | 63,39 dB |
| 824 | 33,82 dB | 63,29 dB |
| 907 | 32,54 dB | 63,99 dB |
| 929 | 34,09 dB | 64,37 dB |
| the other nine | 58 to 65 dB | identical |

and Shaun of the Dead's frame 581 goes from 23,51 dB to 58,90.

Where the two matrices either side of the switch are nearly equal the reading
makes no audible difference, which is why two titles that do carry steep
objects do not move: their two decodes differ by at most 1,5·10⁻⁴ and 1,1·10⁻⁶.
That is the shape a one-slot correction should have.

### Why this is not a metric tuned on one sample

The switch position is a discrete parameter, so it can be swept. Moving it over
±3 slots on the title with the most steep objects gives a single sharp optimum
with nothing near it:

| Shift, slots | −3 | −2 | **−1** | 0 (as printed) | +1 | +2 |
|---|---:|---:|---:|---:|---:|---:|
| Worst object, dB | 24,69 | 27,09 | **49,93** | 25,24 | 21,64 | 19,28 |
| Median, dB | 46,88 | 49,23 | **65,44** | 47,76 | 43,64 | 41,96 |
| Correlation-structure delta | 0,0007 | 0,0007 | **0,0001** | 0,0007 | 0,0012 | 0,0023 |

Both immediate neighbours are more than 22 dB worse. And the reading is not
free-floating: it is what a one-based offset against a zero-based slot index
gives, with no tunable quantity in it.

### Tests added

`steep_switches_one_slot_before_the_printed_reading` in `oadec-joc` pins both
readings and the one slot they disagree about.
`every_interpolation_branch_matches_pseudocode_6` now asks for
`SteepReading::AsPrinted` explicitly, so the printed pseudo-code stays pinned
as printed and the deviation stays visible as a deviation.

### Remaining limitation

This is agreement with one decoder family, as the matrix alignment and the
sparse corrections are. It is `PASS-TOL`, not `PASS`. Steep interpolation is
737 503 of 32 493 245 object updates — 2,3 per cent — so this reaches most
streams, but the two-data-point steep branch still has no material and stays
`N/T`.

---

## The Dolby refusal, chased again

Dolby's TrueHD decoder opens the object presentation on Pi, Talk to Me and
Braveheart and refuses it on Shaun of the Dead, Knives Out and Kingsman. The
audit found `2ch_control_enabled` perfectly correlated across the six, set it in
all 383 major syncs of a Shaun clip with the CRC repaired, saw no change, and
concluded the field was not causal.

Three things came out of going back to it.

**What the refusal is.** The decoder says "Selected Dolby TrueHD presentation is
not available" and the pipeline fails to preroll, so the decision is taken from
the major sync before any audio is decoded. And it is specific to the object
output: all three refused titles decode at `presentation=16` when
`out-ch-config` is left alone. Only `out-ch-config=21` with `presentation=16` is
refused.

**Nothing in the header separates the sets.** Across all 74 fields oadec reports
from the major sync and the 16-channel declaration, not one separates the three
accepted from the three refused. Shaun of the Dead and Knives Out are
indistinguishable from Pi and Braveheart: four substreams,
`extended_substream_info` 3, flags 0x1000, presentation masks 1/3/7/15, and a
16-channel declaration of 12 channels, 11 dynamic objects, dialogue norm 31,
mix level 35. One is accepted and the other refused.

Byte by byte over the first 96 bytes of the major sync, exactly one whole byte
separates them: byte +18, 0x02 in all three accepted and 0x00 in all three
refused. Byte +18 opens `channel_meaning` — six bits of
`heavy_drc_start_up_gain`, then `2ch_control_enabled` — so the differential
finds the audit's field again, independently.

**And the field is causal after all.** The audit tested one direction. Testing
the other:

| Title | `2ch_control_enabled` | Dolby |
|---|---|---|
| Pi | 1, as authored | accepted |
| Pi | 0, cleared | **refused** |
| Braveheart | 1, as authored | accepted |
| Braveheart | 0, cleared | **refused** |
| Shaun of the Dead | 0, as authored | refused |
| Shaun of the Dead | 1, set | refused |
| Knives Out | 0 → 1 | refused |
| Kingsman | 0 → 1 | refused |

The flag is flipped in every major sync and the CRC-16 repaired; `verify` on the
patched Pi reports nothing but the 84 trailing bytes the 50 MB cut already had,
so the patch is clean. Clearing the flag turns an accepted title into a refused
one, on both accepted titles tested.

So `2ch_control_enabled` is **necessary and not sufficient**. Setting it on a
refused title changes nothing, which is what the audit measured and why testing
only that direction could not have found the necessity. The refused titles have
at least one further reason, it is taken before any audio is decoded, and it is
not in the major sync.

None of this says oadec is wrong. oadec decodes all six; the three whose object
output has never been confirmed are the three Dolby will not open in object
mode. `evidence/remediation/presentation16-differential.json`.

---

## The harness that let both defects through

`cargo test --release -p oadec-cli --test real -- --ignored` with `OADEC_MEDIA`
unset reported "10 passed" in 0.00 s, because every test returned `Ok` after
printing "skipping", and individual files skipped the same way. A green run
proved nothing. Asking for `--ignored` is asking for the conformance suite, so a
run that cannot reach the media now fails, and CI runs the check that it does.

---

## What is still open

- **Sparse JOC**, on one of the library's ten sparse frames: 34,6 dB where its
  clip sits at 44,4. The disagreement is two QMF slots wide and sits where the
  sparse matrix is applied. That frame has moved from 1,05 to 16,5 to 30,6 to
  34,6 dB across three separate corrections; the other nine end within 5,7 dB
  of their clips' own level.
- **The second reason Dolby refuses three titles.** `2ch_control_enabled` is
  necessary and not sufficient; the rest is taken before any audio is decoded
  and is not in the major sync.
- **Two-data-point interpolation**, and with it the smooth-2 and steep-2
  branches of clause 6.6.5: 0 of 32 493 245 object updates, and no way to make
  any. Dolby's encoder writes one data point per frame at every data rate it
  offers, even for an object authored to move four times within a frame. The
  branches are covered by a unit test against the printed pseudo-code, which
  proves the implementation matches clause 6.6.5 and not that Dolby agrees
  with it — and defect 4 is exactly a case where it does not, on the branch
  that could be measured. The two-point branches carry the same offset field
  and the same correction is applied to it, unmeasured.
- **Downmix configurations 1, 2 and 4**: reachable, and no material.
  Relabelling cannot make any, because they size the matrix for seven channels;
  neither can the encoder, whose job description has no core-layout or
  downmix-configuration option and which writes configuration 3 at all six of
  its data rates.
- **EMDF in `auxdata`**, and **EMDF in a dependent substream**: both implemented
  and neither exercised, because no stream on hand carries either.
- **Controlled movement, gain, size and divergence.** The authoring pipeline
  works and the scene file is one edit away; only positions and timing were
  measured this round.
- **EMDF protection words** are parsed and not verified, and cannot be:
  clause H.2.2.4.3 says "calculation of the value of the
  `protection_bits_primary` field is implementation dependent and is not
  defined in the present document", and H.2.2.4.4 says the same of the
  secondary word. The audit's backlog item asking for them to be verified is
  not actionable as written; the status moves from N/I to UNK.

## Status

See `conformance-matrix.md` for the feature-by-feature table and the final
before-and-after summary at the end of it.
