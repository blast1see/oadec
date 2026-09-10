# Decoder remediation

What the conformance audit of 2026-09-10 found, what was done about it, and what
is still open. The audit itself is in this directory, committed unedited as the
before-state; where its numbers move, this report says so and the audit stays as
the record of what was measured on the day.

Two defects were demonstrated. Both are fixed. Beyond them the audit left a
number of areas untested rather than failing, and those are treated as what they
are: coverage gaps, unknown proprietary behaviour, or reference-decoder
disagreement, each pursued on its own terms and none of them promoted to a pass
without evidence.

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

## The harness that let both defects through

`cargo test --release -p oadec-cli --test real -- --ignored` with `OADEC_MEDIA`
unset reported "10 passed" in 0.00 s, because every test returned `Ok` after
printing "skipping", and individual files skipped the same way. A green run
proved nothing. Asking for `--ignored` is asking for the conformance suite, so a
run that cannot reach the media now fails, and CI runs the check that it does.

---

## Status

See `conformance-matrix.md` for the feature-by-feature table and the final
before-and-after summary at the end of it.
