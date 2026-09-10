# Decoder remediation

What the conformance audit of 2026-09-10 found, what was done about it, and what
is still open. The audit itself is in this directory, committed unedited as the
before-state; where its numbers move, this report says so and the audit stays as
the record of what was measured on the day.

Two defects were demonstrated by the audit and both are fixed. Two more turned
up while closing the coverage gaps: sparse JOC matrices, found when material
that had never been decoded finally was, and the steep interpolation switch,
found while measuring what the sparse corrections left behind. Clause 6.6.2
yielded a third fault of its own on the way.

Conclusions were overturned in both directions, and more of this report's than
the audit's. `2ch_control_enabled` was called necessary here and is neither
necessary nor a correlate. A field pair that isolated two refused streams was
reported as a finding and is what a search of that size produces by chance. A
frame was carried for three rounds as an unexplained outlier and is the tenth
worst of the ninety-nine in its window. Where a claim moved, the sentence that
moved it is here rather than in a rewritten paragraph.

Each of those came from asking one more question of a measurement that already
looked finished: does the instrument hear the change at all, would chance have
produced this, and where does this sit among its neighbours. They are the three
things this round would tell its earlier self.

Beyond the defects the audit left a number of areas untested rather than
failing, and those are treated as what they are: coverage gaps, unknown
proprietary behaviour, or reference-decoder disagreement, each pursued on its
own terms and none promoted to a pass without evidence. Three are now settled
against authored ground truth rather than against another decoder; two questions
that had been chased on six titles were put to the whole library instead, which
changed the answer to both; and one instrument that failed here was replaced by
a better one — instead of editing a finished stream, have Dolby's encoder write
the shape under test.

Evidence is under `evidence/remediation/`. Every status change points at a file
there.

That the audit itself was not edited is checkable rather than asserted. The
baseline commit is the one that added the audit report, and since then the only
audit file this branch modifies is `conformance-matrix.md`, whose rows carry
their before-state in `[was …]` brackets:

```
git diff --name-status $(git log --oneline --diff-filter=A     -- docs/audit/2026-09-10-conformance-audit.md | tail -1 | cut -d' ' -f1)..HEAD     -- docs/audit/
```

Everything else it lists is an addition under `evidence/remediation/`.

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

### Signals that name themselves

Everything above compares oadec against another decoder, which shows that two
implementations do the same thing and not that either puts a channel where the
stream says it belongs. That needs a source whose channels can be told apart by
listening to them.

Twelve seconds, eight channels, one sine per channel at −20 dBFS — 400, 630,
1000, 55, 1600, 2500, 4000 and 6300 Hz — encoded by DEE's own
`pcm_to_ddp` at `encoder_mode` ddp71, 1024 kbit/s, loudness measured and not
corrected. Out comes exactly the structure defect 1 was about: an independent
substream at `acmod` 7 + LFE and a dependent at `acmod` 5 with `chanmap` 0x1a00,
375 frames each, no CRC failure and no decode error.

**Every decoded channel carries one tone at −20,0 dBFS**, its authored level to
the first decimal, with the loudest tone belonging to another channel between
108 and 191 dB below it. FFmpeg gives the identical assignment from the same
stream. The channel map is no longer a matter of two decoders agreeing.

And the same clip shows what the defect actually delivered. `--core-only`, the
5.1 a decoder that stops at the independent substream produces:

| the 5.1 core's left surround | dBFS |
|---|---:|
| the back-left tone | −20,0 |
| the side-left tone | −21,2 |
| the side-right tone | −26,2 |

Three signals in one channel. After the merge that same channel carries a single
tone at −20,0 with everything else more than 150 dB down, and the back pair
arrives as two new channels. That is clause E.2.8.2's replace-and-add, measured
rather than argued — and it is why the old behaviour was worse than "two
channels missing".

The clip is checked in as material and the check as a media test: every channel
within 1 dB of −20 and at least 60 dB clear of any other channel's tone.
Swapping the side and back pairs in the expectation makes it fail by 197 dB.

One thing was nearly misread as a decoder fault along the way. DEE reads an
eight-channel WAV as L, R, C, LFE, Ls, Rs, Lrs, Rrs — side pair fifth and sixth,
back pair seventh and eighth — where the WAVE channel mask orders them the other
way round. Encoding the same audio again with the mask set to zero places the
tones identically, so the encoder is not reading the mask at all. It was the
FFmpeg control that separated "the encoder's input convention" from "the
decoder's output order".
`evidence/remediation/ddp71-authored-tones.json`.

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

One more thing in the same parser, which the audit's backlog asked about and
which is not a defect: the first parameter band's channel index was being taken
modulo the channel count where clause 6.6.2 applies no modulo. Three
transmitted bits can name a channel a five-channel downmix does not have, and
the wrap put such a band's coefficient onto a real channel. It no longer does:
a band whose index names no channel selects none of them. This cannot change a
conforming stream — all 150 sparse objects in the library transmit 0 to 4 with
five channels, and all five sparse clips decode byte-identically either way —
so it is pinned by a unit test rather than by a measurement.

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

The material that settles it came later, from a streaming collection the user
pointed at: 3 375 sparse objects across eleven streams, where the whole corpus
before it had 150. One clip of Extraction 2 holds 825 of them in 55 frames and
carries **no steep object at all**, which separates the two for the first time.

| | sparse frames | the rest of the clip |
|---|---:|---:|
| as printed | −3,92 dB median, −10,94 worst | 45,96 dB |
| all three corrections | **46,78 dB median, 39,18 worst** | 46,69 dB |

The sparse frames sit at the level of the rest of the clip. There is nothing
left to explain in sparse decoding itself, and the row moves to `PASS-TOL`.

What remains is one frame, and it is the one that is also steep with offset 23:
34,6 dB where its clip sits at 44,4. **That reading did not survive being asked
one more question**; the section below says so.

**The combination is not the cause either.** `verify --json` now counts objects
that are sparse and steep at once and reports where they are, and Extraction 2
carries 120 of them across eight frames. A clip around two of them measures
51,32 and 49,57 dB against a clip median of 48,76 -- at or above their clip's
level, with neighbours at 56 to 63. Nor is a large switch offset the cause: both
clips carry offsets across the whole range, 23 and 24 included.

So the remaining frame is an outlier of one. It is not sparse decoding, not
steep decoding, not the two together, not a large offset and not dither.
`evidence/remediation/sparse-settled.json`.

### One check that does not need Dolby

Everything above is a differential, and a differential cannot tell "this decoder
is right" from "this decoder repeats the same mistake as the one it is compared
with". There is one property of the answer that comes from the audio instead: an
object's level is continuous across a frame boundary unless the content changes,
and a matrix wrong for one frame makes it step.

Decoding the settled clip twice, once under each reading, and measuring that
step 256 samples either side of every boundary:

| | sparse boundaries (95) | worst | every other boundary (403) |
|---|---:|---:|---:|
| the reading in use | 1,54 dB | 4,22 dB | 1,81 dB |
| as clause 6.6.2 prints it | 1,73 dB | **8,02 dB** | 1,81 dB |

The other boundaries are the control and are identical between the two decodes,
because the reading touches only sparse frames. Under the reading in use the
sparse boundaries are quieter than the ordinary ones; under the printed reading
they are noisier and the worst is nearly twice as big. Three smaller clips point
the same way, Red Notice most clearly at 2,60 against 4,59.

This is worth much less than the 50 dB the corrections are worth against Dolby,
and it answers something that measurement cannot. The effect is small — a fifth
of a decibel in the median — because most objects at a sparse boundary are not
badly affected either way. `evidence/remediation/sparse-continuity.json`.

### There is no outlier

The frame is Extraction (2020) frame 63325 — not Glass Onion, whose sparse
frames carry no steep object at all and measure 50 to 68 dB from four different
clip windows. It is sparse, steep and coarse at once, fifteen objects all three,
which is why every correction moved it: as decoded 34,55 dB, with the printed
steep reading 18,43, with the printed sparse reading 7,54, with both 1,05. That
last figure is what the first round recorded before any of them. Dither is not
it either, at 34,90 with dither off.

What was never asked is where the frame ranks among its neighbours. A median
says what a typical frame does and nothing about the spread, and this passage's
spread is wide:

| window | median | frame 63325 | its rank, worst first |
|---|---:|---:|---|
| 99 frames | 40,53 dB | 34,55 dB | **10 of 99** |
| 299 frames | 41,65 dB | 34,89 dB | **48 of 299** |

Nine frames of the first window are worse, down to 22,85 dB, and none of them is
sparse; forty-seven are worse in the second, down to 21,37. The passage as a
whole sits at 40 to 42 dB where Extraction 2's sparse clip sits at 46,78 and
Glass Onion's at 58. A frame six decibels under its window's median, with
forty-seven frames beneath it, is not something left to explain.

The mistake was comparing one frame with a clip median and never asking where it
stood among its neighbours. There is no outlier, and this open item closes.
`evidence/remediation/sparse-outlier-dissolved.json`.

### What is still unexercised

Across 170 722 130 object updates in 67 JOC streams, the corpus having grown
five-fold since: **two data points never occur**, so two of the four
branches of clause 6.6.5 pseudo-code 6 — smooth with two points and steep with
two points — have never run on real material. Downmix configurations 1 and 2
never occur; configuration 4 does, in three library titles, and has a section
of its own below. No EMDF container was found in `auxdata` in any stream.

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

**The streams say the same thing without being asked.** A frame is 24 time
slots, so a zero-based index into it takes the values 0 to 23 and a one-based
one takes 1 to 24. Over three whole streams and 515 205 steep objects:

| | steep objects | offsets seen | at 24 |
|---|---:|---|---:|
| Glass Onion | 181 080 | every value 1 to 24, none outside | 7 410 |
| Extraction | 143 730 | every value 1 to 24, none outside | 4 620 |
| Red Notice | 190 395 | every value 1 to 24, none outside | 7 920 |

Zero never occurs and 24 does, which is what a one-based field looks like and
not what a zero-based one looks like.

It also shows the printed reading discarding transmitted data rather than merely
mistiming it. Under that reading the switch happens when `ts` reaches the
transmitted value, and for a value of 24 in a frame whose slots are 0 to 23 that
never happens: the data point the encoder sent is never applied and the object
holds the previous frame's matrix for the whole frame. That is 19 950 of the
515 205 steep objects here, between 3,2 and 4,2 per cent of each stream's. A
reading that throws away transmitted data on one steep object in twenty-five is
wrong on its own terms, before any decoder is consulted.
`evidence/remediation/steep-offset-is-one-based.json`.

### Reproduction and reference result

`evidence/remediation/steep-offset-differential.json`. Each clip is decoded
twice from the same binary, once with `--steep-as-printed` and once without,
and both are compared per object against Dolby's object decoder in the window
`tools/objcmp.py` already used.

Ten titles. Six carry steep objects; four are the negative control and are
bit-identical under the two readings, because a stream with no steep object
cannot reach the code that changed. The sixth came last, from the streaming
collection: an Extraction 2 clip carrying 1 020 steep objects, a different
encoder generation from the disc remuxes, better on every measure and with its
inter-object correlation structure halved.

| Title | Steep objects | Worst, as printed | Worst, corrected | Median |
|---|---:|---:|---:|---|
| Glass Onion | 195 | 25,24 dB | **49,93 dB** | 47,76 → 65,44 |
| The Jackal | 840 | 26,77 dB | **35,12 dB** | 37,89 → 39,34 |
| Shaun of the Dead | 30 | 40,75 dB | **43,62 dB** | 46,79 → 53,79 |
| Red Notice | 105 | 36,21 dB | 36,21 dB | unchanged |
| Extraction | 30 | 50,51 dB | 50,51 dB | unchanged |
| Extraction 2 | 1 020 | 21,74 dB | **30,30 dB** | 27,11 → 36,70 |

The Jackal came last, from the sweep that measured which outputs the corrections
changed: it carries more steep objects than any title measured before it and had
no reference until then. Two of its damaged frames go 26,51 → 37,76 and
31,46 → 39,18 dB while the rest do not move, which is the same shape as
Glass Onion's.

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

### Material where the branch is the rule, not the exception

Every measurement above is a few frames against a clip, because steep objects
are two to four per cent of a film's object updates. That can be changed by
asking the encoder for them.

Three scenes were authored and encoded, each eight objects over ten seconds and
4 695 object updates:

| the scene | steep objects | where |
|---|---:|---|
| a sweep every half frame | 15 | the fast scene, built for two data points |
| teleporting between opposite corners once a frame | 45 | frames 0, 1 and 2 only |
| **objects arriving out of silence mid-file** | **1 410** | from frame 63 to the end |

So this encoder answers movement — however discontinuous — with smooth
interpolation, and the arrival of level with the steep branch. Teleporting
produced steep only in the first three frames, where the encoder has no previous
matrix to ramp from, which is the same situation an onset creates. That is why
film soundtracks carry steep objects at all: things start.

On the third of those, where thirty per cent of the object updates take the
branch under test, against Dolby's decode of the stream its own encoder wrote:

| | median | worst |
|---|---:|---:|
| the reading in use | **51,39 dB** | **43,45 dB** |
| the printed reading | 45,93 dB | 38,08 dB |

Better on five elements of five, by 3,39 to 5,72 dB. The margin is smaller than
the films' 25 to 50 because there the steep frames are a few per cent and drag a
whole-clip figure with them, while here the figure *is* the steady state.
Unanimous either way, and it is now a media test: the reading in use must beat
the printed one by at least a decibel on every element carrying audio, and
swapping the two makes it fail at element 1 with 38,08 against 43,45.
`evidence/remediation/steep-authored-material.json`, with the scene it was
authored from in `evidence/remediation/controlled-atmos-onset-scene.json`.

Two things that scene could not be made to produce. **Sparse matrices**: not at
any of the six data rates, not from eight point sources parked exactly on the
downmix speakers, not from any other scene. DEE 5.2.1 writes none, so sparse
stays settled against Dolby's decoder on real material and nothing better is
available. **Two data points**: still zero, including in the scene built to
provoke them.

### Remaining limitation

This is agreement with one decoder family, as the matrix alignment and the
sparse corrections are. It is `PASS-TOL`, not `PASS` — though the material is
no longer only other people's: the last measurement is on a stream authored
here, and the encoder that made it is Dolby's. Steep interpolation is 6 352 123
of 170 722 130 object updates — 3,7 per cent — so this reaches most streams, but
the two-data-point steep branch still has no material and stays `N/T`.

---

## Controlled ground truth, second round: object gain and object size

The first round authored a scene, encoded it with Dolby's own encoder both ways
and got seven static positions back exactly, at event offset zero. It named
movement, gain, size and divergence as follow-ups. Two of those are now
measured, and a third turns out not to be measurable through this pipeline at
all.

A second scene holds everything still except the two fields under test:
`evidence/remediation/controlled-atmos-gain-size-scene.json`. Eight objects,
each a tone at its own frequency so it can be identified from its audio, five
of them point sources at 0, −3, −6, −12 and −24 dB and three of them at
gain 0 with sizes 0,25, 0,5 and 1,0. The positions repeat the scene that
already recovered exactly, so they are the control. `atmos_info --validate 1`
accepts the master, exit 0.

### Object gain does not survive the encoder

| | authored | recovered metadata | essence level | total tone power |
|---|---|---|---|---|
| point source | gain 0 | gain 0 | as authored | −23,02 dBFS |
| point source | gain −3 | **gain 0** | as authored | −23,02 dBFS |
| point source | gain −6 | **gain 0** | as authored | −23,02 dBFS |
| point source | gain −12 | **gain 0** | as authored | −23,02 dBFS |
| point source | gain −24 | **gain 0** | as authored | −23,02 dBFS |

The figures are the TrueHD encode; the E-AC-3 one agrees within 0,08 dB. The
gain is not in the metadata and it is not in the audio: the object authored at −24 dB comes back at the same level as the one
authored at 0. It is not this decoder dropping it either — `oadec emdf --dump 1`
reads `gain Db(0)` straight out of the OAMD payload of the encoded stream,
before any DAMF is written.

### Object size is rendered into the spatial coding

| authored size | elements carrying the tone | loudest element | total tone power against authored |
|---|---:|---:|---:|
| 0 (point source) | 1 | −23,02 dBFS | −0,01 dB |
| 0,25 | 7 | −26,83 dBFS | −0,31 dB |
| 0,5 | 8 | −28,39 dBFS | −0,61 dB |
| 1,0 | 11 | −33,42 dBFS | −1,21 dB |

The metadata comes back with size 0, but the energy is not lost: a sized object
is spread over seven to eleven encoded objects where a point source needs one,
and the total is within about a decibel of what went in. The element it is
loudest in reports a position at the room edge rather than the authored one,
which is what a spread across the room gives. So size is not carried; it is
rendered, and the decoder is reading correctly what the encoder wrote.

Neither is peculiar to authored material. Pi decoded from TrueHD and Disclosure
decoded from E-AC-3 JOC carry gain 0 and size 0,0 on every object of every
event, so nothing in the wild exercises these fields either.

### Divergence cannot be authored at all

OAMD carries `object_divergence` in the extended object element and oadec parses
it. The Dolby Atmos Master Format has no divergence field, so a DAMF master
cannot express one and this pipeline cannot produce a stream that carries it.
It stays `N/T` for a reason that no amount of authoring will change: it needs a
stream that already has it.

### What this leaves

The control holds in both codecs: five point sources, positions exact, event
offset zero, one element each. `tools/ground_truth.py` now reads gain and size
back and measures the essence level, so the same scene shape answers all three
questions at once. Movement was measured in the first round and is unchanged:
the encoder resamples a trajectory onto its own grid.
`evidence/remediation/controlled-atmos-gain-size.json`.

---

## The Dolby refusal, chased again

Dolby's TrueHD decoder opens the object presentation on Pi, Talk to Me and
Braveheart and refuses it on Shaun of the Dead, Knives Out and Kingsman. The
audit found `2ch_control_enabled` perfectly correlated across the six, set it in
all 383 major syncs of a Shaun clip with the CRC repaired, saw no change, and
concluded the field was not causal.

Three things came out of going back to it. Read them as the six-title round they
were: two sections below, the same question is put to 194 object presentations
and the flag does not survive it.

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

**And the field looked causal.** The audit tested one direction. Testing the
other gave this, which two later sections take apart:

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

That reading does not survive as stated. It holds only if a refusal after
editing one field says something about that field, and the object path refuses
edits to fields that cannot be causal.

Nine more bits were changed the same way, in every major sync of the same file,
with the CRC-16 repaired and `oadec verify` reporting the same nothing:

| edited | object path |
|---|---|
| `2ch_control_enabled`, cleared | refused |
| `reserved`, made non-zero | refused |
| `flags`, an undefined bit set | refused |
| `peak_data_rate`, one unit lower | refused |
| `variable_rate`, 1 to 0 | refused |
| `heavy_drc_start_up_gain` | refused |
| `eightch_dialogue_norm` | refused |
| `extended_substream_info`, 3 to 11 | refused |
| **`twoch_dialogue_norm`, another legal level** | **refused** |
| **`extended_substream_info`, 3 to 2** | **accepted** |

The last line is what stops this being "any edit refuses": a legal value change,
through the same patcher and the same wrapping, is taken. The machinery works.
Most of what refuses looks like values the decoder does not expect — a reserved
field made non-zero, an undefined flag, a peak rate below the stream's own, a
variable-rate stream declaring itself constant, a four-bit field given a fifth
bit.

But `twoch_dialogue_norm` is the clear line. It is a legal
dialogue-normalisation level, it belongs to the **two-channel** presentation,
and changing it to another legal level makes the **sixteen-channel** object
output refuse. A field of one presentation cannot determine whether another
presentation exists.

The refusal is specific to the object path, which is what makes it look like the
real thing. The same edited streams decode at presentation 2, and at
presentation 16 with the default channel configuration; only
`out-ch-config=21` refuses. That is the shape all six titles show, and it is
also the shape an edited stream shows.

This needed the control it is itself about. The patched streams are wrapped in
MP4, because the Dolby element will not take a raw elementary stream, so the
wrapping could have been the cause. The unpatched stream wrapped with the same
command in the same session is accepted, sixteen object channels. The wrapping
is innocent and the refusal follows the edit.

So the object path reads the major sync far more strictly than ordinary decoding
does, and the defined CRC-16 is not what it is reading: `oadec verify` derives
the CRC span from the parse rather than by searching, and reports every patched
file clean.

What that leaves is a middle position, which is where the evidence stood until
the stimulus was authored rather than edited. The six unmodified titles
correlate perfectly and that is untouched: every accepted one has
`2ch_control_enabled` set, every refused one has it clear. The mutation cannot
carry that to necessity, because the same refusal follows a change to the
two-channel presentation's dialogue normalisation, which cannot gate the
sixteen-channel one.

Two sections below settles it the other way: DEE writes the flag clear when
`presentation_2ch/drc_default_on` is false, and Dolby opens that stream. The
flag is a perfect correlate that is **not sufficient**, and the refusal after
the hand edit came from the edit.

There is no legal alternative for E-AC-3 either. DEE's own stream editor,
`edit_ddp`, is described in its schema as "trim/join Dolby Digital Plus
streams" and takes a start timecode, an end timecode and a join order -- nothing
that touches a metadata field. What worked for TrueHD, having the encoder write
the shape under test, has no counterpart here: the JOC syntax under question is
not exposed by any encoder option.

A fourth has since joined them, and it is the one that looked safest: `dialnorm`
is a header field, not a payload, and its frame CRC is recomputable, so a patched
stream is well-formed rather than merely edited -- the patcher round-trips byte
for byte. Dolby refuses it anyway, and refuses it for a legal value that is not
the one under suspicion, which is the arm that turns a false positive into a
dead instrument.

It is the third instrument to come back with a limit on it, after in-place
rewriting of EMDF payloads, which this decoder discards outright. This one is
not useless -- a legal change is taken -- but it is too blunt to single out one
field, and any question about these titles that needs a clean mutation is
answered by that, not by the mutation.

None of this says oadec is wrong. oadec decodes all six; at the time of writing,
the three whose object output had never been confirmed were the three Dolby will
not open in object mode. That sentence is now false, and the section after next
says why. `evidence/remediation/major-sync-rewrite-rejected.json`,
`evidence/remediation/presentation16-differential.json`.

---

## The refusal is not three titles

Everything above treats the `presentation=16` refusal as a property of three
streams. Six titles is enough to find a field that correlates perfectly and not
enough to know whether it means anything, and the sweep that made the point for
Dolby Digital Plus makes it here too: twenty seconds from the head of every
TrueHD track in the library that declares a sixteen-channel presentation,
through the same command.

| | |
|---|---:|
| files | 186 |
| object presentations | 194 |
| opened | 105 |
| **refused** | **89, or 46 per cent** |

Shaun of the Dead and Knives Out sit alongside A Quiet Place and its sequel,
three John Wick films, three Thor films, Aliens, Apocalypse Now, District 9,
Alita, The Northman, Zombieland and Dredd. What looked like three odd streams is
most of a catalogue.

The verdict belongs to the stream and not to the measurement. The same file
gives the same answer three runs running; A Quiet Place is refused at 20, 60 and
180 seconds of input; and it is specific to the object mode, exactly as it was
for the three — the same clip decodes at `presentation=16` with the default
channel configuration and at `presentation=2` with `out-ch-config=21`, and only
the two together are refused. `truehdd` opens it and finds twelve elements.

### The refused set is not the unconfirmed set

The reason the refusal mattered to this project was never Dolby's opinion of the
streams: it was that the object output for those titles had nothing to check it
against. That is no longer so. The same cuts the sweep took were decoded twice,
by this decoder and by `truehdd`, and the two object audio files compared by
hash.

| | compared | byte-identical |
|---|---:|---:|
| refused by Dolby | 89 | **89** |
| opened by Dolby, as a control | 14 | 14 |

103 of 103, over 3,9 GB of object audio. TrueHD is lossless and presentation 3
carries the objects as coded channels, so two correct decoders must agree
exactly — and they do, on every title Dolby will not open.

The refusal says nothing about whether this decoder is right about those
streams. It does not say Dolby is wrong either: two non-Dolby decoders agreeing
is weaker than either agreeing with Dolby, and why that mode refuses them is
still unknown. What it retires is the last claim in this report that treated the
refused set as unconfirmed.
`evidence/remediation/refused-titles-vs-truehdd.json`.

### What that does to `2ch_control_enabled`

**The correlation does not survive.** Across six titles the flag was set in all
three Dolby opened and clear in all three it refused. Across 194:

| | opened | refused |
|---|---:|---:|
| `2ch_control_enabled` clear | 76 | 83 |
| `2ch_control_enabled` set | 29 | 6 |

Clear is very nearly a coin toss. Set leans towards opening and is a minority of
both. The field predicts almost nothing, and the perfect correlation across six
titles was the six titles. This report has now weakened that reading twice and
can retire it: it was **not sufficient**, by the encoder experiment, and it is
**not a correlate either**, by the sweep.

### What is left is one number, and it is still a correlation

Over every field `info --json` reports — and it now reports the channel meaning,
which it did not before this sweep and which is why the first pass came back
saying nothing at all about these fields — nothing takes disjoint values on the
two sides. **Nor does any pair**: of the 3 486 pairs formable from the 84 fields
that are neither counts nor near-unique, none splits them either. One field
nearly does on its own: `twoch_dialogue_norm`, the two-channel presentation's
own dialogue normalisation.

| | values |
|---|---|
| opened | 37 (×72), 33 (×25), 34 (×3), 36 (×2), 32 (×2), 31 (×1) |
| refused | 63 (×53), 31 (×30), 37 (×5), 33 (×1) |

A value between 32 and 37 goes with opening; 63, the field's maximum and what is
written when nothing was, or 31, which is what the other presentations carry,
goes with refusal. That agrees with the verdict on **187 of 194** streams, seven
exceptions.

Unlike the configuration-0 conjunction, this one is not what a search produces
by chance, and the way to see that is to rank every field by how many rows its
best partition cannot classify:

| field | rows it cannot classify |
|---|---:|
| `twoch_dialogue_norm` | **7** |
| `twoch_mix_level` | 60 |
| `drc_start_up_gain` | 61 |
| `sixch_dialogue_norm` | 73 |
| median of all 84 | 89 |

A field that says nothing about the verdict leaves about 89 rows unclassifiable
here, because that is the size of the smaller side, and the median field does
exactly that. The best one leaves seven, and the second best leaves sixty. That
is not a search finding one candidate among many; it is one field standing an
order of magnitude clear of every other.

It is the same family as the old reading — the flag says whether to honour the
two-channel presentation's own control data and the dialogue norm is that data —
which is why on six titles the two moved together and the flag looked like the
answer.

And it is a correlation, not a cause, for the reason it has always been: the
object path refuses any stream whose major sync was edited, whatever the edit
says, including edits to fields that cannot be causal. A correlation over 194
streams is worth more than one over six. It is still a correlation.

**The classification this earns is Dolby-tool-unsupported**, not spec-invalid
and not oadec-invalid. An output mode that opens 54 per cent of a catalogue's
Atmos titles and refuses the rest, with no bitstream field separating them and
two other decoders opening all of them, is more plausibly a property of that
mode than of the streams.
`evidence/remediation/truehd-object-sweep.json`, and the field-by-field pass
itself in `evidence/remediation/truehd-object-sweep-split.json`.

---

## A second opinion that is not ours and not Dolby's

Every reading above rests on one engine saying no and this one saying yes. DEE,
the Reference Player and Plex EasyAudioEncoder are the same decoder wearing
three names, so the count was one against one. There is a third: `truehdd`
0.6.1, a TrueHD decoder written by other people, in a separate codebase, which
this project already used as the presentation-3 PCM baseline.

**It opens all six.** `truehdd decode --presentation 3` exits 0 on Pi, Talk to
Me, Braveheart, Shaun of the Dead, Knives Out and Kingsman, writing a DAMF set
with the same element count oadec reports on each -- 12, 16, 12, 12, 12 and 14 --
and reporting `Dolby Atmos true` with an independent presentation 3 for all six.
The one structural difference it shows is that Kingsman's presentation 1 is
independent where the other five downmix it from presentation 2, and the other
two refused titles do not share that, so it separates nothing.

That does not say why Dolby refuses, and a permissive decoder opening a stream
is not proof that a stricter one is wrong. What it does is separate two claims
that had been travelling together: *Dolby refuses these three* and *these three
are not object programmes*. The second is now contradicted by an implementation
that shares no code with this one.

**The audio is byte-identical.** Not close: the same MD5 on all six
`.atmos.audio` files, same sample count, same channel count, same CAF header,
zero differing samples in 212 527 200 element-samples. That is what two correct
decoders of a lossless format must produce, and it widens the presentation-3
gate from three titles to six.

### The one thing the two decoders disagree about

Of the metadata, exactly one field differs in value: `samplePos`, and it differs
by exactly 32 samples and by nothing else, on 131 of the 800 events written
across the six titles. `pos` differs on 573 events in printed form only -- the
largest difference in value is 7,1e-08, because `truehdd` prints the full double
and oadec prints seven digits.

Clause 5.3.2 of TS 103 420 is explicit:

> The start_sample values t_n shall be determined by the following equation:
> t_n = so + 32 × bo_n. In this equation so is the value of sample_offset and
> bo_n is the value of block_offset_factor for the corresponding property
> update.

oadec computes `base + container_offset + sample_offset + 32 ×
block_offset_factor`. `truehdd` computes `sample_pos + sample_offset +
evo_sample_offset` and never adds the block offset; it reads
`block_offset_factor_bits` and uses it only to decide whether a payload restates
the current state. Every stream measured carries the factor as 0 or 1, one
payload in five at 1, which is why the difference is always exactly 32.

The specification settles it, but the streams settle it too, and independently.
The same clause says the decoder adds 1 536 to `frame_offset` each codec frame,
so take every written event position modulo 1536:

| | distinct residues over seven streams | shape |
|---|---:|---|
| oadec | **4** | one or two per stream, the second being the one-per-element opening event |
| `truehdd` | 7 | the same residues, with one of them split in two, 32 apart |

Braveheart's 201 events are all at residue 0 under oadec and split 167/34 under
`truehdd`. Shaun's 338 sit at 1152 and split 252/74. On the authored master --
the one case where a regular grid is expected, because Dolby's encoder snapped
its events to frame boundaries -- all 45 of oadec's positions are exact
multiples of 1536 and 19 of `truehdd`'s are 32 short of one.

So this is a reference-decoder disagreement where the specification is explicit
and oadec follows it. Nothing here changes. The equation is now pinned by a unit
test over seven combinations including `block_offset_factor` 63; removing the
term from the decoder makes it fail with left 0, right 32. What is not measured
is the multiplier itself: no material anywhere carries a factor above 1, so 32
is normative rather than observed.
`evidence/remediation/truehdd-second-opinion.json`.

---

## Authoring the stimulus instead of editing it

The mutation instrument is too blunt to ask whether `2ch_control_enabled` is
what Dolby refuses on, because the object path refuses edits to fields that
cannot be causal. There is a way round that the earlier rounds did not take:
stop editing a finished stream and have Dolby's own encoder write the shape you
want to test.

DEE has the knob. `encode_to_dthd/presentation_2ch/drc_default_on` set to false
makes it write `2ch_control_enabled` clear — the shape of all three refused
titles. Four other options in the same block were swept and none moves the flag:
`dolby_surround_encoded`, `dolby_headphone_encoded`,
`legacy_authoring_compatibility` false and the speech DRC profile all leave it
set.

The pair is as clean as this gets outside a bit patcher. Both encodes of the
controlled scene are 3 354 856 bytes. The whole of `oadec info` differs in one
line, `2ch true` becoming `2ch false`. 362 bytes differ in three and a third
megabytes: one byte per major sync at offset +18 — the byte the field-level
table already named as the only whole byte separating the accepted titles from
the refused — two bytes of recomputed CRC-16 twelve bytes later, and 28 clusters
of the two-channel presentation's own DRC data, which is what the option
controls. **The object audio is byte-identical**: the same 23 040 000 bytes out
of `decode -p 3 --format pcm` from both, so nothing the object programme carries
moved.

Dolby opens it.

| | `2ch_control_enabled` | Dolby object path |
|---|---|---|
| the scene as authored | set | accepted |
| **the scene with `drc_default_on` false** | **clear** | **accepted** |
| the scene, surround-encoded 2ch | set | accepted |
| the scene, headphone-encoded 2ch | set | accepted |
| the scene, legacy compatibility off | set | accepted |
| the scene, speech DRC on the 2ch | set | accepted |
| Shaun of the Dead, Knives Out, Kingsman | clear | refused |
| Pi | set | accepted |

The last two rows are the positive control, run in the same session through the
same command: an experiment whose every arm is accepted has to show the
instrument can still say no.

**So a clear `2ch_control_enabled` is not sufficient.** The six-title
correlation is untouched, and so is the possibility that the flag matters
together with something else. But the flag on its own is now ruled out, and the
mutation result is explained rather than merely doubted: clearing the flag by
hand makes Dolby refuse, clearing it at the encoder does not, so the refusal
after the hand edit came from the edit. That is what the dynamic-range gain
control suggested and this settles it. This report's earlier "suggestive but
unclean" becomes **not sufficient**.

### The same question, asked of the other refusal

Dolby also gives one configuration 0 stream six channels and another sixteen
objects: Snatch is refused, Dredd is accepted, and they agree in every field
oadec parses. The mutation instrument is unavailable there too, for a different
reason — Dolby discards a payload rewritten in place. But the re-encode is
available, and it is the same experiment:

| | Dolby, as it stands | Dolby, its own objects re-encoded |
|---|---:|---:|
| Snatch, head | 6 channels | **16** |
| Snatch, a mid-file cut | 6 channels | **16** |
| Dredd, the control | 16 | 16 |

The re-encode is a real object stream and not sixteen channels of something:
oadec and Dolby agree on all fifteen active objects of it at 43,81 to 111,05 dB,
correlation 0,999 98 or better, the sixteenth silent in both. So that refusal is
not about the object programme either.
`evidence/remediation/configuration-0-refusal-pair.json`.

### Asking the whole library instead of one pair

The configuration-0 refusal had been chased with a control pair: one stream
refused, one accepted, and a list of everything that does not differ between
them. Two streams are enough to list what is the same and not enough to find
what is not. Dolby's decoder is on this machine and five seconds of audio is
cheap, so the question can be put to every Dolby Digital Plus track in the
library at once.

| | |
|---|---:|
| files | 210 |
| tracks carrying JOC | 226 |
| opened by Dolby as sixteen objects | 223 |
| refused | **2** |

**Snatch is not a singleton.** The King (2019), a streaming release made by a
different chain on a different day, gets the same six channels. Whatever the
property is, it is not one disc's accident.

A third came back at 15 channels and was not a refusal: F1 The Movie, from a
five-second head that yielded under a second of audio, where the plugin's
decoder-delay padding makes the ratio of bytes to samples land between the
integers. Twenty seconds from ten minutes in gives 15,976, which is sixteen. The
sweep now records the raw ratio and says *check by hand* rather than rounding a
short clip into a refusal.

### One conjunction, and no way to test it

With nine configuration-0 streams instead of two — seven opened, two refused —
there is enough to ask which field splits them. Exactly one does, out of
everything `info`, `emdf` and `oamd` report: **`dialnorm`**, 31 in both refused
and 23 to 27 in every accepted one.

Neither half of that is the answer on its own:

| | |
|---|---|
| configuration 0 alone | 9 streams, Dolby opens 7 |
| `dialnorm` 31 alone | 90 of the 223 streams Dolby opens carry it |
| configuration 0 **and** `dialnorm` 31 | 2 streams, both refused |

No stream in the library is configuration 0 with `dialnorm` 31 and opened, and
none is configuration 0 with a lower `dialnorm` and refused.

**That is worth less than it reads.** There are two refused streams and about
25 200 ways to choose two rows from 225. A search over the 13 530 field pairs
formable from what `info` reports is of the same order, so a conjunction that
isolates exactly those two is what such a search is *expected* to turn up by
chance. It is a candidate, not a finding, and it stays one until a third refused
stream appears. The TrueHD side is the counter-example that makes the point:
there the refused set is 89 rather than 2, and no pair of 3 486 splits it.

And unlike everything else this report has wanted to test, `dialnorm` looked
reachable. It is not in an EMDF payload — which Dolby discards when rewritten —
but five bits of `bsi()` at bit 45 of an E-AC-3 syncframe, with a recomputable
frame CRC. `tools/ec3_patch_dialnorm.py` round-trips: patching Dredd from 25 to
31 and back to 25 reproduces the original file byte for byte, so the repair is
exact, and `oadec verify` calls every patched file clean.

| | `dialnorm` | Dolby |
|---|---:|---|
| Dredd, as it stands | 25 | 16 objects |
| Dredd, patched to 31 | 31 | **6 channels** |
| **Dredd, patched to 20** | **20** | **6 channels** |
| Togo, as it stands | 27 | 16 objects |
| Togo, patched to 31 | 31 | 6 channels |
| Snatch, as it stands | 31 | 6 channels |
| Snatch, patched to 27 | 27 | 6 channels |
| The King, as it stands | 31 | 6 channels |
| The King, patched to 27 | 27 | 6 channels |

The third row is the experiment. Without it the reading is "`dialnorm` 31 causes
the refusal", supported by two streams turning refused when set to 31 — and it
would be wrong. **Every edited stream is refused whatever the new value says,
and no edit makes a refused stream accepted.** The object path is reacting to
the edit.

So a header field of E-AC-3 is no more testable on this decoder than a payload
field is, and this is the fourth instrument to come back with a limit on it. The
open item is better posed and no closer to an answer: two streams out of 226
rather than one, a conjunction with no counterexample in the library, and no way
to ask whether the conjunction is the reason.
`evidence/remediation/dolby-object-sweep.json`.

### And the content is not it either

The same tools answer the next question. The objects of the refused titles are
recoverable — oadec decodes them and `truehdd` confirms them byte for byte — so
they can be handed back to Dolby's encoder and the question asked again.
`decode -p 3 --format damf --no-bed-conform`, then `encode_to_dthd` from that
set with `spatial_clusters` matched to the element count, then the same object
path:

| | elements | re-encode |
|---|---:|---|
| Shaun of the Dead | 12 | accepted |
| Knives Out | 12 | accepted |
| Kingsman | 14 | accepted |
| Pi, the control | 12 | accepted |

So the refusal is not about the object programme: not the objects, not their
count, not their metadata. A re-encode keeps the content and replaces the
container, which is the point of the experiment and also its boundary — it says
the content is acceptable, not which field of the original is the trigger.

What is left is the same question with three fewer answers. The streams are not
undecodable, the flag that correlates is not sufficient, and the content is
acceptable. Nothing in the 74 fields oadec reports separates the sets;
`truehdd verify`, a conformance checker written by other people, does not
separate them either — Shaun and Kingsman draw DRC warnings, Knives Out draws
none, and Talk to Me draws the same warnings while being accepted. It is a
property of how those three streams were written, and no instrument here reaches
it. `evidence/remediation/twoch-control-authored.json`.

---

## A smaller one of the same family

Three of `decode`'s measurement flags never reached the object path.
`--no-dither`, `--no-tpnp` and `--ecpl-spec` all set fields on the core
decoder's options, and `decode --format damf` on an E-AC-3 stream built those
options with `Default::default()` and dropped what the command line said. The
flags read as applied and were not, which is the same shape as defect 2: a
decision taken and then not carried to the thing that produces the output.

It surfaced while testing whether the last sparse frame's residual was dither
noise amplified by a large matrix coefficient. That question could not be asked
until the flag worked. With it working the answer is no: our own dither's weight
on that frame is 61,70 dB, in line with its neighbours at 59,58 to 80,90 and
with the clip's median of 59,38. A residual at 34,55 dB is not dither.

`--core-only` is now refused with an object output instead of being ignored,
because the object programme is the whole programme and the JOC downmix needs
every channel of it. Two media tests hold both: one asks for `--no-dither` and
requires the objects to move, the other requires the contradictory combination
to fail and to name the flag.

---

## The harness that let both defects through

`cargo test --release -p oadec-cli --test real -- --ignored` with `OADEC_MEDIA`
unset reported "10 passed" in 0.00 s, because every test returned `Ok` after
printing "skipping", and individual files skipped the same way. A green run
proved nothing. Asking for `--ignored` is asking for the conformance suite, so a
run that cannot reach the media now fails, and CI runs the check that it does.

---

## Configuration 4, found by scanning the library rather than the corpus

The counts behind three open rows came from 31 streams in the working
directory. The library holds 111 more object-carrying tracks, every count that
closed or failed to close a row is a per-frame encoder decision, and head clips
are what hid sparse mode the first time. So sixteen titles were scanned whole,
chosen across sources rather than at random: 112 of the library's 113 object
tracks use configuration 3 with fifteen objects and unity clip gain, and most
are UHD disc remuxes from one encoder chain, so diversity of source is worth
more than count.

That added 77 858 940 object updates to the 32 493 245 already scanned. Two
data points: still zero. EMDF in auxiliary data: still zero. Two negatives did
fall, though. Sparse matrices and coarse quantisation are not a streaming habit —
Dangal, a Blu-ray remux, carries fifteen sparse objects and eight streams
across discs and streaming carry coarse quantisation. And **downmix
configuration 4 exists**: three of the sixteen titles carry it, where nothing
measured before ever had.

A second, much cheaper sweep then asked the configuration question of the whole
library. The configuration is a header field and constant within a stream, so
ten seconds off the front of a track answers it — what a head clip does not
excuse is reading one track per file, which is what the earlier sweep did and
why it reported 112 of 113 at configuration 3 with one at 0. Reading every
Dolby track of every file, 226 of them across 111 films, 118 carry JOC:
**configuration 3 in 112, configuration 0 in one, configuration 4 in five, and
configurations 1 and 2 in none at all**. Five titles carry configuration 4, not
three, and each of the five discs also carries a configuration 3 track, which is
exactly how a one-track-per-file sweep misses them.
`evidence/remediation/library-configurations.json`.

### Why it had never been seen

Every configuration 4 stream found is an AC-3 core plus an E-AC-3 dependent
substream with `chanmap` 0xa010, giving `L C R Ls Rs LFE Vhl Vhr` — a 5.1.2
programme. Configuration 4 needs a seven-channel downmix, a seven-channel
downmix needs a dependent substream, and until defect 1 those decoded as their
5.1 core. The configuration was not rare; it was unreachable.

### The mapping was written for configuration 1 alone

Table 47 gives the downmix channels per configuration. Configuration 1 ends
`Lb, Rb`, the rear surround pair that table E.1.4 of TS 102 366 calls `Lrs` and
`Rrs`. Configurations 2 and 4 end `Tfl, Tfr`, the top front pair it calls `Vhl`
and `Vhr`. `ChannelLoc::joc_input` knew only the first reading, so a
configuration 4 stream's height channels mapped to nothing.

It did not decode them wrongly. The object pipeline refused: "the programme
channels [...] do not cover the 7-channel JOC downmix". The bail that has sat
there since the substream work did its job on the first real stream to reach it.

### What it measures now

| | worst | median |
|---|---:|---:|
| Green Book, the window the comparison tool uses by default | 50,13 dB | 51,97 dB |
| Green Book, the window where the height pair carries signal | 37,20 dB | 41,96 dB |
| Dangal, likewise, thirteen objects with signal | 34,19 dB | 39,91 dB |

Dangal's other two objects read 5,65 and 5,30 dB and carry an RMS of 0,000001 in
both decoders, so that figure is the ratio of two noise floors and not a
measurement.

**The order within the pair is measured.** Swapping `Tfl` and `Tfr` costs 29 dB
in the window where the height channels carry signal — 1,66 dB worst against
37,20. In the default window it costs nothing at all, to the decimal, because
Green Book's height channels are digitally silent there. A control run in the
wrong window would have said the two mappings were the same thing.

This also settles something left open: the 90-degree phase correction of
configurations 3 and 4 was implemented for 3 and assumed to hold for 4, with the
note "no stream to test". There is a stream now, and it holds.
`evidence/remediation/library-syntax-scan.json`.

---

## An instrument that does not work, and what rested on it

Every conclusion in this project that came from rewriting a field inside an EMDF
payload and handing the result to Dolby's decoder rests on an assumption nobody
had tested: that the decoder honours the rewritten value. It does not.

The test came out of trying to strengthen defect 4. A sweep over an unmodified
stream shows where the steep switch point's optimum is; it does not show that
the optimum belongs to the field, since the same curve would appear if the whole
matrix stream were misaligned for some other reason. Moving the field and
watching both decoders follow it is the experiment that settles that, so
`oadec eac3-joc-offset` was written to move it: five bits per steep data point,
frame check redone, nothing else touched. `oadec verify` reads the new value
back on all 195 of Glass Onion's steep objects and calls the stream clean.

Dolby does not follow it.

| Frame | rewritten to 4 | to 12 | to 20 |
|---|---:|---:|---:|
| 628 | 62,1 dB | 62,1 dB | 62,1 dB |
| 824 | 31,0 dB | 31,0 dB | **bit-identical** |
| 840 | 26,8 dB | 26,8 dB | 26,8 dB |
| 907 | 46,3 dB | 46,3 dB | **bit-identical** |
| 929 | 46,4 dB | 46,4 dB | **bit-identical** |

Those are Dolby's own decodes measured against Dolby's decode of the original.
A decoder following the field would move a different distance for a switch at
slot 3 and a switch at slot 11; this one moves the same distance to one decimal.
And the frames that come back bit-identical under 20 are exactly the three whose
fifteen objects already transmitted 19, so the tool did not change them: 45 of
the 195 fields carry `joc_offset_ts` 20. What Dolby does depends on whether the
payload was touched, not on what was written into it.

What it does instead is hold the previous matrix. Sweeping our own transmitted
value against Dolby's decode of the stream that says 11 gives no optimum at 11
or anywhere near it:

| ours transmits | 0 | 4 | 8 | 11 | 14 | 18 | 22 | 26 | 31 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| dB against Dolby's 11 | 22,8 | 23,7 | 24,9 | 26,6 | 29,0 | 36,9 | **60,9** | 61,0 | 61,0 |

Agreement rises monotonically and saturates at 22, which is where a 24-slot
frame's switch stops firing at all.

The likely mechanism is the EMDF protection words. `container.rs` parses
`protection_bits_primary` and cannot verify it, because clause H.2.2.4.3 of
TS 102 366 says its calculation "is implementation dependent and is not defined
in the present document". A decoder that does compute it finds every in-place
payload rewrite invalid. Frame-level rewriting is not the problem:
`eac3-ecpl-inject` rewrites audio data and redoes the same frame check, and
Dolby decodes those streams normally. It is the container that is protected.
The measurement shows the behaviour, not the field that causes it.

**What this withdraws.** Two earlier conclusions came from relabelling
`joc_dmx_config_idx` and handing the result to Dolby: that configuration 0 alone
turns its upmix off, and that the library's one configuration 0 stream has a
second reason for being refused because relabelling it to 3 does not help.
Neither is supported by that experiment, because dropping to the core's six
channels is also what discarding the payload gives.

Neither survives. A second stream carrying configuration 0 unmodified turned up
later, in material the earlier sweeps had not read: Dolby decodes **Dredd's**
configuration 0 to sixteen objects, and ours agree with its to 52,44 dB at worst
and 54,38 median, with an inter-object correlation structure identical to four
decimal places. The configuration does not turn Dolby's upmix off. Something
about the other stream does, and that is where the question goes back to.

**What it does not touch.** Defect 4 rests on unmodified streams from five
titles decoded by both decoders. So do the sparse corrections, the matrix
alignment and the quadrature. None of them involves a rewrite.

`evidence/remediation/payload-rewrite-rejected.json`.

---

## The gates, re-run at the end

Every change of this round touches the JOC decode, and the last of them --
the first parameter band's channel index -- changes it only for malformed
input, which is exactly what a bit-flip campaign produces. So both campaigns
were replayed against the final binary rather than assumed:

| Campaign | verify | `decode --format pcm` | `decode --format damf` |
|---|---|---|---|
| JOC, 120 sites | 0 silent, 120 non-zero, 0 panics | 0 silent, 120 non-zero | 0 silent, 120 non-zero |
| TrueHD, 100 sites | 0 silent, 100 non-zero, 0 panics | 0 silent, 100 non-zero | — |

Site for site identical to what the same replay recorded before the JOC work.

The blast radius was measured rather than argued, against hashes taken before
the corrections. All 21 corpus PCM decodes still on disk are byte-identical, as
they must be: the corrections are in the JOC matrix and no PCM decode touches
it. Of the fourteen object decodes, the five with no steep object are
byte-identical in all three files of the set, and the nine whose object audio
moved are exactly the nine that carry steep objects, from 30 of them to 1 200.
The three TrueHD baselines are still bit-exact, the six-title object comparison
regressed on none of them, the controlled scene reproduces its figures to the
digit, and the media suite passes 18 of 18 in 1 329 s. The two newest are the ones this
round could not have written before it had the material: the steep switch
measured on a stream that is thirty per cent steep, and a title Dolby will not
open checked against an independent decoder byte for byte. The presentation-3 baseline is no
longer three titles: `truehdd` was run over all six and the object audio is
byte-identical on every one, 212 527 200 element-samples with the same MD5 — and
then over every title in the library that Dolby's object mode refuses, 89 of 89
identical with 14 it opens as a control, 3,9 GB in all. Across the 420 streams
the two sweeps read, this decoder reported no failure of any kind.
`evidence/remediation/decode-exit-codes.json`,
`evidence/remediation/regression-summary.json`.

---

## What is still open

- **Why Dolby's object mode opens some TrueHD Atmos titles and not others.**
  It opens 105 of the library's 194 object presentations and refuses 89, so this
  is not three odd streams but 46 per cent of a catalogue. The verdict belongs to
  the stream — stable across runs and cut lengths — and to the object mode alone:
  the same clip decodes at `presentation=16` with the default channel
  configuration and at `presentation=2` with `out-ch-config=21`.

  Four answers were removed and none was found. The streams are not undecodable:
  `truehdd` opens all 89 and this decoder's object audio matches it byte for
  byte, 103 of 103 including controls. The content is not it: each refused
  title's own objects, re-encoded by DEE, are opened. `2ch_control_enabled` is
  retired twice over — DEE writes a stream with the flag clear when
  `presentation_2ch/drc_default_on` is false and Dolby opens that, so it is not
  sufficient, and across 194 streams it is clear in 76 Dolby opens and 83 it
  refuses, so it is not a correlate either. And nothing in the bitstream
  separates the sets: no field of the 84 usable ones, and no pair of the 3 486
  formable from them.

  What is left is an association: `twoch_dialogue_norm` between 32 and 37 goes
  with opening and 63 or 31 with refusal, agreeing on 187 of 194 where the next
  best field manages 134 and the median field manages nothing. That is not a
  search artefact — the arithmetic that killed the configuration-0 conjunction
  clears this one — but it is an association, because no mutation reaches it:
  the object path refuses any stream whose major sync was edited, whatever the
  edit says.

  One avenue is prepared and not taken. An Atmos sink is on this machine and its
  EDID advertises TrueHD and Dolby Digital Plus at eight channels, so a refused
  title can be sent to a renderer that is neither Dolby's software nor this one.
  It cannot answer in a machine-readable way — a sink reports what it decoded on
  its own display and there is no return path over HDMI — so it needs a person
  at the device. `tools/atmos_sink_check.ps1` sends six titles to it in turn and
  pauses after each. `evidence/remediation/truehd-object-sweep.json`,
  `evidence/remediation/refused-titles-vs-truehdd.json`,
  `evidence/remediation/atmos-sink.json`.
- **Why Dolby refuses two configuration 0 streams and accepts seven.** A
  library-wide sweep of its object path over 226 Dolby Digital Plus tracks finds
  two refusals: Snatch and The King (2019), a streaming release, so it is not
  one disc's accident either. They agree with the seven it opens in every field
  this decoder parses — frame header, programme, EMDF container version and key,
  payload configuration, the small payloads, the OAMD shape, the JOC header.

  Two answers are excluded. The sequence counter: Snatch's 48 zeros are all in
  its first 48 frames, a mid-file cut has none, and Dolby refuses that cut too.
  The content: Snatch's own objects, re-encoded by DEE, come back as sixteen
  objects Dolby opens, on the head clip and the mid-file cut alike, with Dredd
  at sixteen either way.

  What is left is one conjunction and it is a candidate rather than a finding.
  The two refused streams are the only ones anywhere that are configuration 0
  **and** carry `dialnorm` 31; neither half is enough alone, since Dolby opens
  seven configuration-0 streams and 90 streams with `dialnorm` 31. But there are
  two refused streams among 225 and about 25 200 ways to choose two rows, against
  13 530 field pairs searched — the same order — so a pair that isolates exactly
  those two is what such a search produces by chance. It will stay a candidate
  until a third refusal appears.

  No experiment reaches it. Dolby discards any payload rewritten in place, and a
  header field is no better: every stream whose `dialnorm` is rewritten is
  refused whatever the new value says, including a legal value that is not the
  one under suspicion. `evidence/remediation/configuration-0-refusal-pair.json`,
  `evidence/remediation/dolby-object-sweep.json`.
- **Two-data-point interpolation**, and with it the smooth-2 and steep-2
  branches of clause 6.6.5: 0 of 170 722 130 object updates, and no way to make
  any that could be measured. A synthetic payload carrying two data points is
  writable -- the syntax is small and the Huffman encoder already exists for the
  tests -- but Dolby discards a rewritten payload, so it could only ever be
  checked against this decoder, which is what the unit tests already do. Dolby's encoder writes one data point per frame at every data rate it
  offers, even for an object authored to move four times within a frame. The
  branches are covered by a unit test against the printed pseudo-code, which
  proves the implementation matches clause 6.6.5 and not that Dolby agrees
  with it — and defect 4 is exactly a case where it does not, on the branch
  that could be measured. The two-point branches carry the same offset field
  and the same correction is applied to it, unmeasured.
- **Downmix configurations 1 and 2**: reachable, and no material anywhere in
  the library — 0 of the 118 Dolby tracks that carry JOC, across 111 films.
  Relabelling cannot make any, because they size the matrix for seven channels;
  neither can the encoder. That was read off its generated job description
  before; it is now read off its XML schema, where the three Atmos
  Dolby Digital Plus filters share one option set — data rate, dynamic range,
  downmix levels, trims — with no core layout and no downmix configuration
  anywhere in it. It writes configuration 3 at all six of its data rates.
  Configuration 4 is no longer on this list.
- **EMDF in `auxdata`**: implemented and unexercised. No frame of any of the 151
  streams read whole carries auxiliary user bits at all, so there is nothing for
  a container to sit in. EMDF in a dependent substream is no longer on this
  list: the configuration 4 streams carry all of theirs there.
- **Controlled divergence**, which cannot be authored through either route
  Dolby's encoder accepts. DAMF has no field for it, and the Dolby Atmos Master
  ADM Profile v1.0 lists `objectDivergence` at quantity **0** with "this
  subelement shall not be used" — so writing an ADM BWF, which oadec can do and
  DEE can read, does not reach it either. It needs a stream that already carries
  one, and none on hand does.
- **Object gain and object size in the wild.** Both are dropped by Dolby's
  encoders and neither appears in any real stream measured. The parse of a
  non-zero value is unit-tested against hand-built payloads, including the reuse
  and differential forms; what has never happened is a real stream carrying one.
- **EMDF protection words** are parsed and not verified, and cannot be:
  clause H.2.2.4.3 says "calculation of the value of the
  `protection_bits_primary` field is implementation dependent and is not
  defined in the present document", and H.2.2.4.4 says the same of the
  secondary word. The audit's backlog item asking for them to be verified is
  not actionable as written; the status moves from N/I to UNK. What is new is
  that a decoder which does compute them is now visible from the outside:
  Dolby discards any JOC payload rewritten in place, whatever the new value
  says, which is what a failed check looks like. That does not recover the
  algorithm, and it costs this project an instrument.

## Status

See `conformance-matrix.md` for the feature-by-feature table and the final
before-and-after summary at the end of it.
