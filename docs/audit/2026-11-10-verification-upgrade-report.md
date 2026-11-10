# Verification upgrade, 2026-11

The owner asked whether oadec could take on five things: DVD-Audio (MLP, the FBB
syntax); bitstream conformance with detailed diagnostics and the decoder-buffer
rules; Evolution frame protection; much better error diagnosis; and whether a
stream could go on a disc as it is. Another decoder has some of them, and the
owner asked that none of its code be ported. The logic was studied and checked
against a standard or another public document, and the code written for oadec's
own structures. This report is what was built, how it was measured, and what is
still open. Every number comes from a stored measurement under
`evidence/verification-upgrade-2026-11/`.

The work landed in three pull requests, merged on 2026-11-10 and released as
0.8.0: [#3](https://github.com/blast1see/oadec/pull/3) (named rules, the buffer
model, the disc rows), [#4](https://github.com/blast1see/oadec/pull/4) (silence
for a damaged unit) and [#5](https://github.com/blast1see/oadec/pull/5)
(DVD-Audio).

## 1. Verdict

| Question | Answer |
|---|---|
| DVD-Audio (FBB) | Decoded. The six-channel 96 kHz stream Meridian's own encoder wrote decodes identical to FFmpeg on every channel, and five of the six identical to the source files it was encoded from; the sixth is one sample apart, an offset FFmpeg's decode and the stream's check words both place in the file. FFmpeg's two published samples and four fixtures written for the repository are identical to FFmpeg too (09). A stream whose two channel groups run at different rates is refused by name. |
| Conformance, with detailed diagnostics | Every check `verify` makes on TrueHD is a named rule with a default severity and its source. A finding says where it is, and `verify` reports damage regions with the unit the stream is clean again at. On 48 streams no verdict, field or line an earlier build reported moved (03). |
| The decoder-buffer rules | The model gives the figures Dolby's own TrueHD encoder reports, 84 of 84 over twelve encodes, the unit each peak is first reached at included (06). Its bounds warn, and on real material they fire only inside damage regions. |
| Evolution frame protection | Reported, not checked. How the protection word is computed is implementation dependent and the key is not public (ETSI TS 102 366, clause H.2.2.4.3). All 696 673 Evolution frames of 44 TrueHD streams name key 0 with an 8-bit word (05). |
| Error diagnosis | Unit, byte, bit, substream and time for every finding; damage regions; `oadec rules`; `oadec au FILE --index N`; `--fail-on`, whose default is the verdict `verify` has always given (03). |
| Disc authorability | Rows for Blu-ray, HD DVD and DVD-Audio, each no with its reason and the source of the bound. They match the validity table of Dolby's encoder, 36 of 36, and 49 of 51 corpus streams fit Blu-ray; the two that do not are the damaged titles (07). `--require-disc` puts the fit in the verdict. |
| Not asked, found and done | A unit oadec cannot read now decodes to silence in its place, where a decode used to stop or shift everything after the damage (08). The DVD-Audio sync pattern inside a damaged TrueHD stream no longer ends the run (02). |

## 2. How it was done

**Sources.** Dolby's *TrueHD (MLP) High-Level Bitstream Description* (2018), the
BDA *BD-ROM Audio Visual Application Format* white paper 2.5 (2011), ETSI TS 102
366 Annex H and TS 103 420. FFmpeg's MLP parser and decoder were read for facts
about the FBB syntax, cited in `docs/truehd.md`; truehdd was read for facts only.
No code, rule names or messages came from either.

**Dolby's tools as black boxes.** The TrueHD encoder of Dolby Media Encoder 5.7.2
prints a verification report with every stream it writes: the buffer figures and
a validity table for three disc formats. Its messages were read as strings of the
program. The Dolby Reference Player's TrueHD decoder was run on real damage.
Nothing was decompiled. The logs name the machine and stay out of the repository;
only numbers are in the evidence.

**The corpus.** 40 raw TrueHD heads of library titles, two clips, the committed
fixture, six heads of tracks Dolby's encoder wrote for delivered remuxes, and three
whole titles: Coco (2017) and The Incredibles (2004), which carry real damage,
and Up (2009), which branches 18 times and is clean. That is 52 streams and 27.8
million access units (04).

**Gates on every step.** `tools/old_new.py` runs the previous build and the new
one over the corpus and reports every changed exit code, old JSON field, `verify`
line and decoded byte. Every new classifier has entries in `tools/mutants.py`: 32
new mutations, 90 in all, every one caught by the unit tests, one only after a
test was added for it. Format, lint, 401 tests, the ADM toolkit's checks and
`cargo deny` pass on Linux and Windows.

## 3. Step by step

### 3.1 The DVD-Audio pattern in a damaged TrueHD stream (01, 02)

The extractor took either sync pattern for a major sync and refused the FBB one
outright, so four bytes in the wrong place of a damaged unit made every command
exit 2 with no report. A stream now locks onto the pattern whose major sync
verifies first, and after that only its own pattern opens a major sync. An FBB
major sync is 28 bytes, with the FBA CRC-16 over the first 26 stored big-endian
at offset 26; no other offset matches on any of three published FBB streams (01).

On 43 real streams every exit code, old JSON field, old text line and decoded
byte is unchanged. On a copy of the fixture with the FBB pattern written into a
unit, `verify` and `decode` go from exit 2 to 7 and report the damage (02).

### 3.2 Named rules, located findings and damage regions (03)

Each check is a rule in `oadec-truehd/src/rules.rs`, with a default severity and
its source: a clause of a public specification, a measurement in the repository,
or another implementation. A rule only another implementation states cannot fail
a stream by default; a test holds the catalog to that, and another holds
`docs/conformance-rules.md` to the catalog. Every counter `verify --json` has
always reported is a rule whose findings add up to it, which a test checks on
five kinds of damage.

Over 48 streams, the three whole titles among them, nothing an earlier build
reported moved apart from the change in 3.1; the JSON gains `diagnostics`,
`foreign_syncs` and `substream_stats`. Coco reports 21 fatal and 17 error
findings in 7 regions. Each region's clean re-entry is a seamless branch: six of
them valid, and one the invalid branch `verify` also reports. The Incredibles
reports 6 fatal findings in 2 regions; Up is clean.

A region was held to a decode. The fixture with one byte of substream 1 of unit
1500 inverted gives a region of units 1500 to 1537 in presentations 1, 2 and 3,
clean again at 1538. Decoded against the undamaged fixture, presentation 0 is
identical, and presentations 1 to 3 differ in samples 60 002 to 61 519, units 1500
to 1537, and nowhere else.

The diagnostics cost 1.3 and 1.9 per cent on two whole titles. With the buffer
model of 3.5, `verify` is about 3 per cent slower on a whole title.

### 3.3 New rules, counted before they were given a severity (04)

The rules from Dolby's description that oadec did not check were run over the
corpus first: 27 823 168 access units, 217 673 major syncs, 109 931 752 segments
and 696 736 Evolution frames. The rules are major syncs more than 128 units
apart or closer than eight, a minor sync whose directory begins with `0xF872`,
more blocks than the restart sync allows, a termination word claiming a whole
unit, restart headers of one unit disagreeing on `output_timing`, bytes after the
extra data, an Evolution frame longer than its container's syntax, and object
metadata describing another programme. None fires on any stream. Over nine streams
run through `verify --decode`, no sample needs more bits than `max_bits` allows and
none is clipped.

Two first readings were wrong and were corrected before anything shipped:

- The spacing rule fired 30 times in the whole titles, every time on the unit after
  a branch. The clip a branch joins opens with two major syncs in a row, which the
  description permits where a bitstream opens, so the unit after a branch is now
  excused the same way.
- A rule for set reserved bits found the reserved bit of a directory entry set in
  all 52 streams, 13 070 270 times. The description says parsers should ignore
  reserved bitfields (section 3.2), and the encoders set this one, so the rule was
  withdrawn rather than shipped as noise.

The new rules warn, except two that describe bytes nothing accounts for and fail
the stream: an Evolution frame longer than its container's syntax, and bytes after
the extra data inside a unit.

### 3.4 Protection (05)

Over 51 TrueHD streams, 44 carry Evolution frames: 696 673 containers, every one
with key 0, an 8-bit primary word, no secondary word and version 0. That holds on
both sides of the proxy that separates the titles Dolby's object path opens from
the ones it refuses, so that refusal does not follow the key or the length of the
word. In 52 E-AC-3 and AC-3 heads, 33 carry containers: key 0 in 13 and key 1 in
20, a 4-byte primary and a 1-byte secondary word. No stream codes the reserved
primary length, which is now reported as `evo.protection_reserved`. The words are
not computed: the clause leaves the method to the implementation and the key is
not public.

### 3.5 The decoder buffer, against Dolby's encoder (06)

Dolby's description (section 2.7) gives the buffer and leaves its accounting
open. The open readings were measured against the report Dolby's encoder prints.
It encoded synthetic programmes at 48 and 96 kHz, with one to four substreams,
objects, a rate-optimised encode, pink noise gated against a tone, and white noise
up to 17.4 Mbit/s.

The model that came out of the measurement:

- A unit arrives whole at its `input_timing` and leaves one frame after its output
  time.
- A substream's depth counts its segments, and not its directory entry.
- A decoder's depth counts the segments and directory entries of the substreams it
  reads, and every unit's header, major sync and extra data. A copied presentation's
  decoder reads the substreams of the presentation it copies.
- Each substream may hold 15 000 bytes for each channel its restart header spans.

The first encode refuted two other readings: removal at the output time, and the
directory entry counted in a substream's own depth.

On twelve encodes every figure of the report comes out the same, 84 comparisons,
down to the unit a peak is first reached at. White noise shows the bound the encoder
works to. In every white-noise encode one substream comes nearer its allowance
(90.9 to 97.6 per cent) than any decoder comes to 15 000 bytes a channel (73.5 to
93.9 per cent). On 57 real streams the bounds fire only in Coco and The
Incredibles, and only inside their damage regions: all 340 delay and 25 rate
findings of Coco are there. None fire on the 55 others, the 18 branches of Up and
six encoder-written deliveries included.

### 3.6 Disc rows (07)

| Row | Bound | Source |
|---|---|---|
| Blu-ray | 48, 96 or 192 kHz; eight channels, six at 192 kHz; 18.64 Mbit/s | BDA white paper 2.5, Figure 3-4 |
| Blu-ray, DVD-Audio | the FBA syntax for Blu-ray, the FBB syntax for DVD-Audio | Dolby, section 1 |
| HD DVD | at most two channels in an FBA presentation at 176.4 and 192 kHz; both presentations in an FBB stream | the messages of Dolby's encoder (secondary) |
| DVD-Audio | flags bit 14; two channels at 176.4 and 192 kHz | the messages of Dolby's encoder (secondary) |
| DVD-Audio | 9.6 Mbit/s | the peak Meridian's encoder declares for a 96 kHz six-channel stream (secondary) |
| Blu-ray, HD DVD | within the delivery bounds of 3.5 | the buffer and rate rules, each with its own source |

No public source gives the rates HD DVD takes, so that row says `unjudged` where
the rate decides.

On the twelve encodes of 3.5 the rows match the encoder's validity table, 36 of 36.
The encoder refuses 44.1 and 192 kHz input, so FFmpeg's TrueHD encoder wrote seven
streams for the rows Dolby's cannot reach:

| Stream | Blu-ray | HD DVD | DVD-Audio |
|---|---|---|---|
| 2.0, 176.4 kHz | no | unjudged | no |
| 2.0, 192 kHz | yes | yes | no |
| 5.1, 44.1 kHz | no | unjudged | no |
| 5.1, 48 kHz | yes | yes | no |
| 5.1, 96 kHz, pink noise | no | no | no |
| 5.1, 192 kHz, a tone | yes | no | no |
| 5.1, 192 kHz, pink noise | no | no | no |

The pink-noise streams are kept off every disc by their delivery: FFmpeg's encoder
declares a peak near 9.6 Mbit/s whatever it writes and delivers more. Of the 51
corpus streams, 49 fit Blu-ray. The two that do not are Coco and The Incredibles:
365 and 121 delivery findings respectively, none outside a damage region.

### 3.7 Silence where a unit is damaged (08)

Two clips were cut around real damage regions, from three major syncs before to
three after, and decoded by oadec and by the TrueHD decoder of the Dolby Reference
Player.

| | Coco | The Incredibles |
|---|---|---|
| Damage region | units 364 to 382 | units 352 to 382 |
| oadec, before (`--on-error stop`) | stops at unit 364, exit 2 | stops at unit 352, exit 2 |
| oadec, now | 767 units, 19 of them silence, exit 7 | 767 units, 31 of them silence, exit 7 |
| Dolby | repeats the last good unit 18 times, then drops one unit | repeats it 30 times, then drops one unit |

oadec's units before the damage are what they were, and from the restart header
on the samples are those of a clean decode, in the same place. Dolby's decoder
writes 766 units, so what follows comes out 40 samples early, and its `plc`
property changes nothing on TrueHD.

The whole of Coco, 7 562 407 units and 1:45:02, used to stop at unit 394 396. It
now decodes to its end with 360 units of silence in 7 regions. These are the 7
regions `verify` reports for that presentation, unit for unit, and all 59 083
check words hold.

The stored bit-flip campaign (seed 7, 100 sites on the fixture) still passes. The
12 decodes that stopped now finish with exit 7, and `verify` is unchanged. Over 52
streams the only changes are the decodes of the two damaged titles: exit 2 to 7,
at full length.

### 3.8 DVD-Audio (09)

`God Save The Queen.mlp` was written by Meridian's MLP encoder 1.23 and is published
with the encoder's log and a WavPack file for each input channel. It declares 96
kHz, 24 bits, channel assignment 20 in two substreams, flags `0x4000` and a 9.6
Mbit/s peak, over 36 000 units and 30 seconds.

- **Channels.** The six channels come out as L R Lb Rb C LFE, the group order the
  encoder's log prints. All six are identical to FFmpeg, sample for sample.
- **Against the source files.** L, Lb, Rb, C and LFE are identical to their files.
  R is one sample apart from its file; FFmpeg decodes it as oadec does, and every
  check word holds, so the offset is in the file.
- **Check words.** 4 499 in each presentation, none failed.
- **Downmix.** The two-channel downmix is within 34 LSB rms of the coefficients the
  encoder printed, under a signal near 400 000 LSB rms. The printed coefficients are
  rounded; the coded ones are exact.
- **Disc rows.** `verify` is clean: Blu-ray no, HD DVD yes, DVD-Audio yes.

FFmpeg's `440hz.mlp` and `luckynight-partial.mlp` are identical to FFmpeg, with
1 370 and 1 114 check words holding; the second is cut part way through a unit and
exits 7. The four fixtures FFmpeg's MLP encoder wrote for the repository put one
tone on each channel through an explicit map, and `crates/oadec-cli/tests/cli_fbb.rs` holds each
tone to its label and each decode to FFmpeg's.

## 4. Found along the way

- FFmpeg's TrueHD encoder declares a peak data rate near 9.6 Mbit/s whatever it
  writes. Its 5.1 at 96 and 192 kHz delivers more, and at 192 kHz more than 18
  Mbit/s (07).
- No stream of the 63 measured sets the constant-delay flag (major sync `flags`
  bit 15), so that rule is documented and not checked.
- Dolby's TrueHD decoder conceals damage by repetition and lets the timeline move
  by a unit (08).
- The encoders set a reserved bit of every directory entry (04).

## 5. Still open

- DVD-Audio's own decoder buffer is not modelled; on an FBB stream only the delivery
  rate is read.
- An FBB stream whose groups run at different rates is refused by name: the decoder
  has one output rate.
- The limits for the decoders of the 6-, 8- and 16-channel presentations are not
  checked. No public source gives them.
- The rates HD DVD takes are not known, so its row is `unjudged` where they decide.
- The constant-delay flag is not checked, for want of a stream that sets it.
- A damaged stretch whose length the skipped bytes and the output clock do not agree
  on is left unfilled and reported, not guessed.
- 23 of the 28 real-media tests cannot run: their clips and references no longer
  exist. The other five pass. `tools/media_regression.sh` was not run.

## 6. Reproducing

```text
python tools/old_new.py --old old/oadec.exe --new new/oadec.exe --decode \
    --out old-new.json E:/work/thd E:/work/clips
python tools/rule_census.py --oadec target/release/oadec.exe \
    --out census.json E:/work/thd E:/work/clips
python tools/fifo_oracle.py --oadec target/release/oadec.exe --out fifo.json \
    E:/work/dme/*.mlp
python tools/replay_fuzz.py
python tools/mutants.py
```

`fifo_oracle.py` needs each encode's text log beside it, written by Dolby's
encoder with `--extras log_format=txt`. It reads only numbers from the log. The
paths are examples; the streams themselves are not in the repository.

## Addendum, 2026-11-10: the 44.1 kHz family and 192 kHz (10)

The matrix had TrueHD at 44.1, 88.2 and 176.4 kHz, and at 192 kHz, as not
testable: no title in the library carries them. FFmpeg's TrueHD encoder writes
all six rates, so it wrote five-second programmes at each, 2.0 and 5.1, from a
tone per channel and a quiet sweep at 24 bits.

- On all twelve streams oadec's decode of the widest presentation is the
  encoder's input, sample for sample, and FFmpeg's decode too. No check word
  fails.
- `verify` passes ten and gives 7 on the two 5.1 streams at 176.4 and 192 kHz.
  The encoder declares a peak near 9.6 Mbit/s and delivers up to 12.45 and 12.90
  Mbit/s, the encoder fault 3.6 records; the decode is exact there.
- Four 0.1-second streams of the same kind, tones only, are fixtures.
  `crates/oadec-cli/tests/cli_rates.rs` holds them to FFmpeg's samples, each
  tone to its label, and the rate to what `info --json` and the WAVE header say.

This is encoder-written material, not disc material, and the matrix row says so.
