# TrueHD as `oadec` reads it

Notes in our own words on the parts of the format the code relies on, with the
evidence that pinned each convention. There is no public TrueHD specification;
the syntax was re-derived from the public decoders (FFmpeg `mlpdec.c`, LGPL;
`truehdd`, Apache-2.0), the Dolby high-level bitstream description and the AES
paper on MLP, and then checked against real discs. Where two sources disagree,
the real streams decide.

## Framing

* An access unit (AU) starts with `check_nibble:4`, `access_unit_length:12` (in
  16-bit words, header included) and `input_timing:16`. The nibble makes the XOR
  of the header and directory bytes, folded to four bits, equal `0xF`.
* A major sync (`0xF8726FBA`) may follow; it ends with a CRC-16 (polynomial
  `0x002D`, shift-then-XOR register from 0) over everything before it.
* The DVD-Audio form (`0xF8726FBB`, FBB) has no extension: its major sync is
  always 28 bytes, the same CRC-16 over the first 26 at bytes 26-27. Three
  DVD-Audio streams verify this way at that offset and no other, one written by
  Meridian's own encoder (evidence
  `docs/audit/evidence/verification-upgrade-2026-11/01-fbb-major-sync-crc.json`).
  The first major sync that verifies, of either form, decides which the stream
  is (see *FBB (DVD-Audio)* below). Once the extractor has locked onto a
  stream, only that stream's pattern opens a major sync; the other pattern is
  data like any other byte, so damage that leaves it at the start of a unit is
  judged by the unit's parity and recovered from by a resync.
* One directory entry per substream: `extra_word:1, restart_nonexistent:1,
  crc_present:1, reserved:1, end_ptr:12` (words from the start of the segments),
  plus `drc_gain_update s9, drc_time_update:3, reserved:4` when `extra_word`.
* Segments follow in order; the block of extra data (Evolution frames with the
  Object Audio Metadata) sits after the last segment with its own header nibble
  and parity byte.
* The major sync's `audio_sampling_frequency_1` gives the rate: codes 0-2 are
  48/96/192 kHz and codes 8-10 are 44,1/88,2/176,4 kHz. An access unit holds
  `40 * (fs / 44100)` samples with the division truncated, so the count is 40,
  80 or 160 and **cannot identify the rate**: each count belongs to two rates,
  one from each family. That is why a configuration change is checked against
  the rate itself and not against the count -- otherwise a stream can cross
  between the families mid-file, decode correctly, and be written out with the
  wrong rate on its header. Only 48 and 96 kHz have ever been read here: one
  track in 221 across a 525-file library is 96 kHz, and none is anything else.

## Substream segments

Blocks until `last_block_in_segment`, then 16-bit alignment, an optional
termination word (`0x348D3` in 18 bits, `zero_samples_indicated:1`, then either
`zero_samples:13` or `0x1234`), and with `crc_present` a parity byte and a CRC
byte. The segment ends exactly at the directory's end pointer.

* Parity byte = XOR of every segment byte before it, XOR `0xA9`.
* CRC byte = CRC-8, polynomial `0x63`, shift-then-XOR register starting at
  `0xA2`, fed with every segment byte before the parity byte **only**. Feeding
  the parity byte as well (an equally plausible reading of the other decoders)
  fails on every real segment; the data-only form passes on 24,232,208 segments
  of Pi.

## Restart header

`sync:14` (`0x31EA` for the two lower substreams with MLP-style noise channels,
`0x31EB` with the dither table, `0x31EC` for the object presentation in
substream 3), `output_timing:16, min_chan:4, max_chan:4, max_matrix_chan:4,
dither_shift:4, dither_seed:23, max_shift:4, max_lsbs:5, max_bits:5, max_bits
(repeat):5, error_protect:1, lossless_check:8, hires_output_timing:1,
reserved:2, heavy_drc_present:1` (only meaningful with major-sync flag bit 13;
then `gain s9, time:3`, else 12 reserved bits), `ch_assign:6` per matrix
channel (a permutation), `crc:8` (polynomial `0x1D`, register from 0, over the
bits from the sync word to just before the CRC).

A restart header is therefore an entry point. Applying one re-initialises the
substream: the channel assignment, the dither, the matrices and the FIR and IIR
histories of every channel, which is what carries audio state from one access
unit to the next. A decode that begins at an access unit carrying a major sync
and restart headers is thus not an approximation of a full decode from byte 0 --
it is the same samples. `oadec decode --start <seconds>` uses that: it swallows
access units until the requested time, begins at the first major sync at or
after it, and says which sample that was, because the snap forward can be as
long as one major sync interval. A test in
`crates/oadec-cli/tests/cli_truehd.rs`,
`a_decode_that_starts_inside_the_stream_is_the_tail_of_a_full_one`, holds it by
comparing the bytes. A start the stream never reaches is refused with the length
it actually has, rather than with the complaint for a file that carried no
stream at all.

How far the snap can be was measured, not assumed: across four titles and 56
starts, every one of them a byte-identical tail, the worst snap was 126, 123,
127 and 122 access units -- about 105 ms. It is bounded by the maximum major
sync interval `oadec info` reports as `max_major_sync_interval`, 128 in all four
streams, but the landing points are not on a fixed grid, because a cut clip's
sync points are irregular: pi-head50m carries one at access unit 1 and
shaun-head20m at unit 18. Evidence:
`docs/audit/evidence/verification-2026-09-14/21-seek-corpus.json` and
`22-seek-grid.json`.

## Block header, matrices, filters, samples

A segment is a run of blocks. A block opens with `block_header_exists:1`.
When it is set, `restart_header_exists:1` follows, then the restart header if
that is set, then the block header: a run of fields, each behind a flag, and
the flag itself only in the stream while its guard bit is set.

| Guard bit | Flag, then the field |
|---|---|
| 0 | `guards:8`, the guard bits from here on |
| 7 | `block_size:9`, 8 to the samples of an access unit |
| 6 | the matrices (below) |
| 5 | `output_shift` s4 for each matrix channel |
| 4 | `quant_step_size:4` for each channel up to the last coded one |
| | for each coded channel, a flag and then its parameters: |
| 3 | the coefficients of filter A |
| 2 | the coefficients of filter B |
| 1 | `huff_offset` s15 |
| | `huff_type:2`, `huff_lsbs:5` (no flag: they close the parameters) |

A restart header leaves every guard bit set, a block size of 8, no matrix, no
output shift and no quantiser step, and for every channel no filter, an offset
of 0, no code book and `huff_lsbs` 24. A block says only what changes.

**A coefficient set** is `order:4`, at most 8 for filter A and 4 for filter B,
and 8 for the two of a channel together. A set of order 0 ends there. Otherwise
`coeff_q:4` (8 to 15, the fractional bits; the two filters of a channel carry
the same), `coeff_bits:5` (1 to 16), `coeff_shift:3` (bits and shift together
at most 16), and each coefficient in `coeff_bits` bits of two's complement, to
be shifted left by `coeff_shift`; -32768 is not a coefficient. Then
`new_states:1`, which only filter B may set, followed by `state_bits:4`,
`state_shift:4` and a state for each tap in `state_bits` bits, shifted left
by `state_shift` -- or nothing at all for a width of 0, which is a state of
noughts. The first state is the most recent.

**The block data** is, for each sample, the bypassed LSBs of each matrix that
sends them and then, for each coded channel, the code of its book and
`huff_lsbs - quant_step_size` further bits. The three books have one shape
(`crates/oadec-truehd/src/huffman.rs`): a leading `1` and two, one or no
bits for the values from 0 up; `00`, `n` noughts and a `1` for `-(n + 1)`;
`01`, `n` noughts and a `1` for `base + n`, the base 4, 2 or 1. No code is
longer than nine bits, so the books reach -7 to 10, 8 and 7. Without a book
there is no code and it counts as 0. The sample is

    ((code << lsb_bits) + lsbs + huff_offset - sign) << quant_step_size

where `sign` centres the range: `1 << (lsb_bits - 1)` without a book, and
with one `1 << (lsb_bits + 2 - huff_type)`; a shift that would be negative
makes it 0. With `error_protect` the data is preceded by `block_data_bits:16`
and followed by `block_header_crc:8`; what the first counts and how the second
is computed no public source says (*Probes*, below). `last_block_in_segment:1`
closes the block.

**The matrices** of a `0x31EA` substream are `primitive_matrices:4` and then,
for each, `matrix_ch:4` (the channel it writes), `frac_bits:4` (0 to 14),
`lsb_bypass_used:1`, and a coefficient for each channel up to the last matrix
channel and for two noise channels after them: a flag, and behind a flag that
is set the coefficient in `frac_bits + 2` bits of two's complement, in units
of `2^-frac_bits`. A flag of 0 is a coefficient of 0. All the matrices are
sent again when any changes, and a restart header leaves none. A matrix with
`lsb_bypass_used` adds one bit to every sample of the block data, in front of
the codes of the channels: the matrices in order, a bit each.

A `0x31EB` substream has no noise channels and a `dither_scale:4` behind each
matrix. The syntax of `0x31EC` (`cf_mask`, `cf_shift_code`,
`lsb_bypass_bit_count`, delta coefficients) is documented inline in
`crates/oadec-truehd/src/matrix.rs`.

A segment of a `0x31EA` substream holds four blocks at most, one of the other
two kinds two (Dolby's description, 4.6); major syncs come eight access units
apart at least and 128 at most (2.5). `verify` warns on either.

The Huffman books are decoded through a 512-entry table per book. The
structural decoder (leading `1` → centre group, `00`/`01` → zero-run chains)
is kept as the `const fn` that builds the tables; replacing its data-dependent
branches by one lookup took the parser from 173× to 286× real time on a cached
excerpt, which is how badly the branch predictor fares on Huffman codes.

## Decoding

* Recorrelation: `pred = (Σ fir·hist + Σ iir·hist) >> coeff_q`, `out = residual
  + (pred & ~((1 << qss) − 1))`, IIR history takes `out − pred`. Histories
  reset at a restart header; IIR states are loaded from the coefficient set
  when it carries them. That the histories are empty after a restart header
  is something no real stream shows and a written one does: see *Streams
  written from a recipe* below.
* Rematrixing (18 fractional bits, `i64` accumulators): `0x31EA` feeds two noise
  samples per position from the 23-bit register `r`: the byte from bit 15 up
  and the byte from bit 7 up, each a signed number shifted left by
  `dither_shift`, after which `r` becomes `((r >> 7) ^ ((r >> 7) << 5) ^
  (r << 16)) & 0x7FFFFF`. A restart header states the register, and the first
  sample after it is drawn from what it states. `0x31EB` and `0x31EC` add a
  dither value from a 256-entry table indexed by
  `((P − pmi)(2n + 1) + n) mod 2^k`, shifted by `11 + dither_scale`; `0x31EC`
  adds `(Σ in·D >> 18) · n · ((65536 / spa) << 2)` and does `M += D` at the end
  of the access unit. Output: `((acc >> 18) & ~((1 << qss) − 1)) +
  bypassed_lsb`.
* The 256-entry noise table is part of the format; it appears identically in
  FFmpeg (`noise_table`) and truehdd (`DITHER_LUT`); `oadec` carries it in
  `crates/oadec-truehd/src/dither.rs`.
* Remap: `out[ch_assign[ch]] = in[ch] << output_shift[ch]` (right shift for
  negative). Lossless check: XOR of `(out & 0xFFFFFF) << (ch & 7)` over every
  output of every access unit since the previous restart header, folded 32 → 8
  bits and compared with the next restart header's `lossless_check`.

Evidence (whole films, `oadec compare`): presentations 0, 1 and 2 of Pi, Talk to
Me, Knives Out, Shaun of the Dead, Braveheart and The King's Man are
sample-identical to FFmpeg; presentation 3 of Pi (12 channels) is
sample-identical to truehdd's CAF output, with every lossless check passing.
See `docs/evidence/`.

## Channel order

Output channels are produced in stream order (front pair, centre, LFE, side
pair, back pair, …). FFmpeg and WAVE files use the interchange order (back pair
before side pair); `ChannelLabel::interchange_order` gives the permutation, and
`oadec decode` writes it by default. FFmpeg's 5.1 output for a 7.1 stream needs
`-downmix "5.1(side)"`, because its 5.1 default uses back channels that are not
a subset of the stream's side layout.

## Blu-ray dumps that keep the AC-3 core in the same file

A Blu-ray TrueHD track carries two elementary streams in one PES: the MLP
access units and an AC-3 core for players that cannot decode TrueHD. A
demultiplexer that copies the payload without separating them writes a file
that is neither, and the extension usually still says `.thd`.

Such a file defeats every tool tried here. FFmpeg answers "Invalid data found
when processing input" and MediaInfo prints nothing. A TrueHD parser fares
little better: it locks on at each major sync, loses framing at the next core
frame and hunts for the next one, so it resynchronises tens of thousands of
times and throws away most of the stream.

They separate without guessing, because each stream declares its own length:
an AC-3 syncframe in its header, a TrueHD access unit in its first two bytes.
Walking the file and taking whichever parses recovers both. `oadec thd-demux`
does it. Measured on a 4,99 GB dump of a 2:51:47 film: 12 368 306 access
units and 322 092 core frames, **no byte left over**, 23 seconds.

Two traps worth naming:

- **A file that opens with the AC-3 sync word can still be TrueHD.** The core
  frame comes first in such a dump, so sniffing two bytes routes the file to
  the wrong decoder. `is_eac3` now looks for a TrueHD major sync with an
  access-unit chain behind it before it answers.
- **Take a core frame only when something parses after it.** A four-byte
  pattern turns up in audio data eventually; the one-unit lookahead is what
  keeps an access unit that happens to open `0B 77` from being eaten.

## Bit-exact against Dolby

The lossless check inside the format is self-validating, and `truehdd` is not
a Dolby decoder, so until now nothing said that this decoder agrees with
Dolby's. It does, exactly. Plex ships a licensed Dolby engine
(`EasyAudioEncoder`, driven through `Plex Transcoder -c:a truehd_eae`); on
`pi-head50m.thd` decoded to the eight-channel presentation, oadec and that
engine agree on **all 40 606 400 samples**, zero lag, no sample different at
24 bits. See `docs/evidence/2026-09-09-b.md`.
- **The object presentation is bit-exact against Dolby.** Dolby's TrueHD
  element refuses a raw elementary stream but takes the same audio in an MP4,
  and `out-ch-config=21` with `presentation=16` then writes the coded objects
  as PCM. Against that, Pi's twelve objects and Talk to Me's sixteen come out
  identical, every sample: 60 909 600 and 30 720 000 of them, worst difference
  zero. TrueHD invents nothing at decode time, so two correct decoders have
  nothing to differ about. `docs/dolby-tools.md` has the pipeline and
  `docs/evidence/2026-09-10.md` the numbers.

## Diagnostics

Every check `verify` makes is a rule in [`conformance-rules.md`](conformance-rules.md):
a stable name, a default severity, and the source the rule rests on. The
severity follows the source. A clause of Dolby's high-level description, a
measurement recorded under `docs/audit/evidence/`, or a clause of the ETSI
specifications for the metadata may fail a stream; a rule that only another
implementation states warns at most.

A finding carries its place: the access unit (counted as `verify` counts them),
the byte offset, the substream, and for a segment that does not parse the bit
the reader stood at when it gave up -- the end of the last field it read. Time
is the unit index times the samples per unit, so it is nominal: a gap or a
branch earlier in the stream is not accounted for.

A damage region follows from how the format restarts. A substream is decoded
from its restart headers on, and a restart header re-initialises everything its
decoding carries, so a fault in a substream leaves it untrustworthy until that
substream's next restart header, and a presentation is damaged where any
substream it reads is. A fault in the access unit as a whole damages every
substream. Bytes the extractor passed over are lost, and the unit it found sync
again at carries a major sync and so is a clean entry point by itself. A fault
in the extra data damages the object metadata of that unit and none of the
audio. `crates/oadec-cli/tests/cli_rules.rs` checks the reading against the
decoder: a damaged and an undamaged copy decode identically outside the region,
and a presentation that reads no damaged substream decodes identically
everywhere.

## What a stream uses of the syntax

The parser reads every element the descriptions define. Which of them a
stream is coded with is a separate question, and `verify` answers it: the
`Coding tools:` line (`coverage` in `--json`, which `info --json` carries
too) names the prediction filters of either kind, each code book, quantiser
steps, output shifts, the matrix syntaxes and their options, heavy DRC, and
the rest of the list `oadec_truehd::Census::tools` holds. A tool is named when
the stream was coded with it, not when a field for it was present: a filter
of order zero is no filter.

`verify --census` reports the count the names are drawn from: how often each
element occurs and with which values, first for the major syncs and the unit
directories, then for each substream the stream carries (restart headers,
blocks, filters, entropy coding, matrices). With `--json` every counter is
listed, the ones at zero included, and a field of 32 values or fewer lists
them all; as text, one line a group, with the counters that are not zero.

Two kinds of count are kept and the name says which. A counter ending in
`_sent` counts the syntax: how often the element was transmitted. Every other
counts the state in force: the blocks, or the blocks times channels, coded
under each value. A filter sent once and kept for a film is one `fir_sent`
and millions of `fir_order`.

The census is what makes "no material carries it" a number. One stream says
what one encoder did with one programme; `tools/sweep.py` records the census
of every TrueHD track of a library and `tools/census.py` adds them up and
lists the counters that are zero in every one. It is also the order of work
for anything that would judge a reserved field: count how often real streams
set it first, give the rule a severity after.

`tools/cov_regions.py` asks the same question of the code instead of the
syntax. It sorts every line of the decoder by what reaches it: the unit tests
(a bitstream built by hand to reach it), the corpus (what an encoder writes),
both, or neither.

## Streams written from a recipe

An encoder chooses, and what no encoder chooses no stream carries. For that
part of the syntax `crates/oadec-synth` writes the streams: a recipe is a
layout, a plan that says what codes each block, and the samples, and
`oadec-synth write <recipe> <stream> [<pcm>]` gives the stream and the samples
a decoder has to give back for it. The samples are known before any decoder
has read a byte, the signals are integer arithmetic, and a recipe is the same
bytes on every machine.

The writer shares no code with the decoder: it holds the syntax and the
arithmetic of this page a second time, from the encoding side, and takes only
the CRC and parity primitives from `oadec-bits`. It still shares an author
and a reading of the format, so that the decoder gives back the samples of a
stream the writer wrote says little. What a recipe is worth is decided
outside the repository. `tools/synth_accept.py` puts every stream in front of
FFmpeg, `truehdd` and Dolby's own decoder (the `dlbtruehddec` element of the
Dolby Reference Player, on the stream wrapped in MP4), each run as a program
and judged by the bytes it writes, and records in
`crates/oadec-synth/specs/digests.tsv` the digest of each stream, the digest
of its samples, and which of the three gave those samples. A recipe is
recorded when two of them did. The tests of the crate hold the writer to the
record, so a change that alters a stream fails until the stream has been in
front of the decoders again; and they hold the decoder to the samples, `verify`
to finding nothing, and the census of each stream to the tools its recipe
says it carries.

What the 41 recipes cover so far is one substream of one or two channels in
the TrueHD and the DVD-Audio syntax. Under the channels: no filter and no
book, the three books, the offset at both ends of its range, `huff_lsbs` wider
than the samples need, filter A at every order and precision, filter B at
every order with and without a state, both filters on a channel in every
split of eight taps, coefficient shifts, quantiser steps with and without a
filter over them, one to four blocks to a unit with the coding changing
inside it, the six sampling rates, major syncs 8 and 128 units apart, digital
silence, full scale, and segments without their check bytes. Over the
channels: one to eight matrices at 0 to 14 fractional bits, a channel kept or
turned over, coefficients on the two noise channels with the register running
on or set anew at every restart header, the noise shifted, a bypassed bit, a
quantiser step under a matrix, output shifts of 1 to 7 bits up and 1 to 8
down, and the two channels sent to each other's output. In the headers: guard
bits sent again, cleared and given up, and the update of the heavy dynamic
range control. Several substreams and the matrices of the other two sync
words are not written yet.

Measured (`docs/audit/evidence/engine-round-2026-11/10-synthetic-truehd.json`
for the first 26, `11-synthetic-matrices.json` for the 12 over the channels,
`12-probes.json` for the last three and the probes): every recipe is
confirmed by two decoders at least, 34 of the 41 by every decoder asked.
Dolby's decoder is not asked for the four in the DVD-Audio syntax. Each of
the other seven is a fact about one decoder:

| Recipe | The decoder that reads it otherwise | What it does |
|---|---|---|
| `silence` | Dolby's | no output: of 16 access units of ten bytes the parser element in front of the decoder hands on 7 |
| `blocks-four` | `truehdd` | refuses four blocks to a unit; it takes three |
| `filter-at-restart` | FFmpeg | keeps the filter histories across a restart header (below) |
| `matrix-noise-shifted` | `truehdd` | decodes to samples that fail its own lossless check: it reads the noise channels right at a shift of 0 and not at any other |
| `bypass-over-step` | Dolby's | does not cut the sum to the quantiser step (below) |
| `output-shift-down` | FFmpeg | takes an output shift below nought for none, and says it has not implemented it |
| `heavy-drc` | `truehdd` | refuses bit 13 of the major sync's `flags` as reserved |

### The filter histories after a restart header

A restart header is an entry point: a decoder that starts there has nothing
before it. Whether a decoder that was already running keeps its filter
histories across one is not something a real stream can say. Of 282,022
restart headers in 46 real streams, every one sits in a block of eight samples
in which no channel has a filter, and those eight samples fill both
histories: a decoder that clears them at the header and one that keeps them
go on from the same state.

The recipe `filter-at-restart` has filters in force in the very block of the
restart header, after noise. `truehdd` and Dolby's decoder give its samples,
which are the ones a cleared history predicts. FFmpeg's decoder gives others
from the first sample after each restart header and reports the lossless
check of the section as failed: it keeps the histories. `oadec` clears them
(`SubstreamDecoder::restart`), which is now a measured reading and not only
the plausible one, and a mutation that keeps them is caught by this recipe
and by nothing that was in the suite before it.

Every other recipe codes the eight samples at a restart header as encoders
do, without a filter (`AtRestart::Unfiltered`), so that all three decoders
can judge what it is about.

### A bypassed bit over a quantiser step

A matrix writes its channel as the sum cut to the quantiser step of that
channel, and with `lsb_bypass_used` one bit from the stream is added. What
happens when a matrix has both, a step to cut to and a bit to add, is again
something no real stream says: of 242.8 million matrices in force in the
blocks of 55 streams, 1.8 million send a bypassed bit and 65 million write a
channel with a step, and none does both.

The recipe `bypass-over-step` does both. FFmpeg and `truehdd` cut the sum and
add the bit, `(sum & mask) + bit`, as this decoder does, and give the samples
of the recipe. Dolby's decoder gives the sum as it is with the bit added,
`sum + bit`: on four streams written for the purpose, with the bit clear and
with it set, with the step on one channel and on both, that reading gives
every one of its samples, 640 of 640 each time, and the other gives 33 to 39.
With a step and no bypassed bit, and with a bypassed bit and no step, the
four decoders agree (`matrix-steps`, `matrix-bypass`).

So on this one point the decoder of the format's owner stands against the two
public decoders this one was derived from, and `oadec` is on the side of the
two. It stays there for now, and the recipe is recorded with Dolby's decoder
among those that gave other samples: no stream that exists is decoded
differently either way, and which reading is the format's is not for a count
of decoders to settle. `11-synthetic-matrices.json` has the measurements.

### Probes

A recipe keeps within what the format and its encoders do, and is a reference
because outside decoders gave its samples back. A probe is the other kind of
written stream (`oadec_synth::PROBES`, names beginning `probe-`): it steps
past a bound of the format, or into a field whose meaning is not public, to
see what a decoder does there. Nobody's agreement makes a probe right. What
the three outside decoders did with each is recorded in
`crates/oadec-synth/specs/probes.tsv`; what this decoder does is held by the
tests as what it does today.

| Probe | `verify` | FFmpeg | `truehdd` | Dolby's decoder |
|---|---|---|---|---|
| `probe-five-blocks`: five blocks to a unit, the bound being four | warns, `segment.block_count` | the samples | refuses (it takes three) | the samples |
| `probe-sync-every-four`, `probe-sync-every-unit`: major syncs closer than eight units | warns, `sync.major_sync_spacing` | the samples | refuses | the samples |
| `probe-nine-matrices` | nothing | refuses | the samples | refuses |
| `probe-protected`: `error_protect`, the size counting the block data alone, the check byte 0 | nothing: the byte is kept and cannot be checked | the samples, and a complaint about the size | the samples | refuses |
| `probe-protected-counting-itself`: the size counting its own 16 bits too | fails, `block.data_bits` | the samples, no complaint | refuses | refuses |

`oadec decode` writes the samples each probe was written from, with the exit
code of `verify`. Three things follow.

*Dolby's decoder is not strict where the description is.* It reads five
blocks to a segment and major syncs in every unit; the two rules stay
warnings, which is what they were set to before anything could show it.

*Nine matrices pass `verify` and Dolby's decoder refuses them.* FFmpeg's
decoder stops at eight too. A stream that the format owner's decoder will not
read should not come out of `verify` without a word, and there is no rule
for it yet: the bound has first to be counted on real streams, for each kind
of substream.

*What `block_data_bits` counts is still open.* FFmpeg's decoder takes it to
count its own 16 bits; `truehdd` and this decoder take it to count the block
data alone, and `block.data_bits` fails a stream on that reading. Dolby's
decoder refuses a protected block either way, most likely over the check
byte, whose definition is not public, so it cannot say which. No real stream
sets `error_protect`: 0 of the restart headers of 157 library tracks.

## Decoder buffer

Dolby's high-level description (section 2.7) gives the model: an access unit
enters the decoder's buffer whole at its `input_timing`, and the data of a
frame leaves when the frame's output comes -- the `output_timing` of the
restart headers at a major sync, one frame later for each unit after it. The
encoder keeps the buffer from running over or under, holds the delay through
it, and keeps the delivery within the peak data rate; every decoder has at
least 120 000 bytes of buffer, and no unit is delivered faster than
18 Mbit/s.

What the description leaves open was measured against the verification report
Dolby's own TrueHD encoder prints for each stream it writes
(`docs/audit/evidence/verification-upgrade-2026-11/06-fifo-model.json`,
`tools/fifo_oracle.py`):

* a unit leaves one frame after its output time;
* the delay through the buffer is the output time less the input time;
* a substream's depth counts its segments; a decoder's depth counts the
  segments and directory entries of the substreams it reads, plus the header,
  major sync and extra data of every unit; the decoder of a copied
  presentation reads the substreams of the one it copies;
* a substream may fill 15 000 bytes for each channel its restart header spans,
  as the encoder's report states for every substream. On white noise, which
  leaves the buffer nothing to smooth, this is the bound the encoder fills to:
  one substream reaches 91 to 98 per cent of its allowance while no decoder
  comes past 94 per cent of 15 000 bytes a channel;
* a unit's delivery rate is its bits over the input interval to the next unit,
  and the peak is the first unit that reaches it.

On twelve encodes -- 48 and 96 kHz, one to four substreams, objects, a
rate-optimised encode, white noise up to 17.4 Mbit/s -- every figure of the
report comes out the same.
`verify` prints the peaks (`Buffer:`; `buffer` in `--json`) and judges the
bounds as warnings: delay over 75 ms or under one frame, a substream or the
stream over its allowance, delivery over the declared peak or over 18 Mbit/s.
On real material they fire only inside damage regions. `verify --trace-fifo
FILE` writes what every unit puts into the buffer and when, as CSV.

Not checked: a delay held constant when `flags` bit 15 says so (Dolby, Table
12). No stream in the corpus sets the bit -- 63 streams, encoder deliveries
included -- so there is nothing to measure the rule against at a branch.

## Disc formats

`verify` says whether the stream could go on a disc as it is (`Disc:`; `disc`
in `--json`): one row a format, and for a row that says no, the reason and the
source of the bound.

| Row | What it holds the stream to | Source |
|---|---|---|
| BD | the FBA syntax; 48, 96 or 192 kHz; at 192 kHz no presentation over six channels | Dolby's description, section 1, names FBA the Blu-ray syntax; BDA white paper, version 2.5, Figure 3-4 |
| HD DVD | at 176.4 and 192 kHz no presentation over two channels; an FBB stream with both its two- and its six-channel presentations | the verifier messages of Dolby's TrueHD encoder (secondary) |
| DVD-A | the FBB syntax; `flags` bit 14; at 176.4 and 192 kHz no presentation over two channels; delivery within 9.6 Mbit/s | Dolby's description, section 1; the verifier messages of Dolby's TrueHD encoder (secondary); 9.6 Mbit/s is the peak Meridian's own encoder declares for a 96 kHz six-channel stream (secondary) |

The BD and HD DVD rows also hold the stream to its delivery: a finding of the
buffer or rate rules above is a reason for both, since a stream Dolby's own
verifier would refuse goes on neither disc.

What the rows leave out, and why:

* Figure 3-4 gives eight channels at 48 and 96 kHz. The 8-channel presentation
  cannot have more, and the 16-channel object presentation came after the
  figure; Dolby's encoder marks its object streams valid for Blu-ray, and so
  does the row.
* Figure 3-4 caps a Dolby Lossless stream at 18.64 Mbit/s. On a disc that
  stream carries an AC-3 stream along with the TrueHD data, and the white paper
  does not say what the figure counts. Every TrueHD unit is held to 18 Mbit/s
  anyway (`rate.over_ceiling`, Dolby section 2.7), which is lower.
* The word length (16 to 24 bits on Blu-ray): an FBA stream does not declare
  it.
* The rates HD DVD takes are in no public source. At 44.1, 88.2 and 176.4 kHz
  its row says `unjudged`; at 48 and 96 kHz Dolby's encoder marks its streams
  valid for HD DVD, and its two-channel bound at 192 kHz says the rate itself
  is taken.
* DVD-Audio's own buffer -- the FIFO sizes of the DVD-Audio decoder -- is not
  modelled: the buffer model is the FBA one Dolby's encoder reports, and on an
  FBB stream only its delivery rate is read.

`verify --require-disc bd` (or `hddvd`, `dvda`, comma-separated) makes a row
part of the verdict: a required row that says no, or cannot judge, is one
finding under `disc.required`, an error, and the run exits 7.

The rows were held to Dolby's encoder, which prints a stream validity table for
every stream it writes, and to streams it will not write
(`docs/audit/evidence/verification-upgrade-2026-11/07-disc-formats.json`).

## Damaged access units

A decode writes every access unit, damaged or not, so the programme keeps its
length and what follows a fault stays where it belongs in time (`decode
--on-error conceal`, the default):

* An access unit that cannot be read, or whose segment fails its parity, CRC,
  end-pointer or sample-count check, is one access unit of silence. The
  substream it damaged is read again from its next restart header, and the
  presentation is silent until every one of its substreams has been -- the
  damage region `verify` reports, unit for unit. Nothing before the fault and
  nothing after the restart changes: a restart header re-initialises
  everything its substream's decoding carries. The lossless check word of a
  section that took silence is skipped, not failed.
* A stretch of the file the extractor passed over is filled with the silence
  it took. The output clock -- the `output_timing` of the restart header the
  stream resumes at, against the units before it -- gives its length modulo
  65 536 samples. The bytes passed over, at the stream's average unit size,
  have to put it well inside one wrap of that clock (under half, even if they
  underestimate it twice over) and agree with it within a factor of two. Bytes
  damaged where they lie do; bytes missing from the file do not, and such a
  gap is left as it is, the output that much shorter, and said.
* A change of configuration still ends the output where it happens (exit 7),
  and a fault before the first sample of output is still an error (exit 2).

The run says what it did -- `concealed N access units with silence (X ms) in
K regions; the first at ...`, `filled N gaps ...`, `left a gap ... unfilled:
why` -- and exits 7. `decode --on-error stop` is the behaviour before: the
decode ends at the first access unit it cannot read (exit 2), and a gap leaves
the output shorter. `compare` and `verify --decode` always stop, because both
are about the stream as it is.

Dolby's own decoder -- the `dlbtruehddec` GStreamer element of the Dolby
Reference Player -- conceals differently. On a real stretch of 19 damaged
units it repeated the last good access unit 18 times, so what follows came
out one unit early, and its `plc` property changed nothing on TrueHD
(`docs/audit/evidence/verification-upgrade-2026-11/08-concealment.json`).
Repeating a 40-sample unit is a buzz at 1.2 kHz; silence is a gap the ear
reads as one.

## FBB (DVD-Audio)

DVD-Audio carries MLP in the FBB syntax, the one TrueHD's FBA grew out of.
Dolby's description leaves it out (section 1), so what oadec reads of it rests
on these, and on measurement:

* **The major sync** is 28 bytes, the CRC-16 of the FBA one over the first 26
  (measured, `01-fbb-major-sync-crc.json`). After the sync word come the word
  length codes of channel groups 1 and 2 (0, 1, 2 = 16, 20, 24 bits), their
  sampling-frequency codes (the FBA codes; 0xF: no group 2), 11 bits, and a
  5-bit channel assignment; then the signature, `flags`, 16 reserved bits,
  the variable-rate bit and the peak data rate, the substream count, four bits,
  and `substream_info`, where FBA has them. The layout is the one FFmpeg's MLP
  parser reads (`ff_mlp_read_major_sync`). The eight bytes before the CRC are
  a channel meaning laid out otherwise than FBA's; they are kept and not read.
  A Meridian-encoded stream sets `flags` bit 14, as Dolby's encoder demands of
  an FBB stream for DVD-Audio.
* **Substreams.** At most two, both restarting on 0x31EA (FFmpeg's MLP
  decoder). With two, substream 0 carries a two-channel downmix and both
  together the whole programme, presentations 0 and 1; with one, substream 0
  carries all of it. `substream_info` says which, as in FBA. The access unit,
  the block and the decoding are those of a TrueHD 0x31EA substream.
* **Channel assignments** 0 to 20 name the channels in two groups, in this
  order in the stream (the surrounds are labelled back channels, a single one
  `Cb`, as FFmpeg's layouts have them):

  | Assignment | Group 1 | Group 2 |
  |---|---|---|
  | 0 | C (mono) | |
  | 1 | L R | |
  | 2 to 6 | L R | Cb; Lb Rb; LFE; LFE Cb; LFE Lb Rb |
  | 7 to 12 | L R | C; C Cb; C Lb Rb; C LFE; C LFE Cb; C LFE Lb Rb |
  | 13 to 17 | L R C | Cb; Lb Rb; LFE; LFE Cb; LFE Lb Rb |
  | 18 to 20 | L R Lb Rb | LFE; C; C LFE |

  The sets and their sizes are FFmpeg's (`mlp_channels`, `mlp_layout`); the
  order is the group order, which FFmpeg's decoder confirms by moving the
  surround pair of 18 to 20 behind the centre and the LFE, and which
  Meridian's encoder prints for assignment 20.

`decode` takes the widest presentation an FBB stream carries when none is
named; the object outputs refuse one, which has no object presentation. A
stream whose second channel group runs at another rate than the first is
refused by name (exit 2): the decoder has one output rate.

Measured (`docs/audit/evidence/verification-upgrade-2026-11/09-fbb.json`): the
96 kHz, 24-bit, six-channel stream Meridian's encoder wrote (assignment 20, two
substreams) decodes identical to FFmpeg on every channel and every sample, and
every one of its 4 499 lossless check words holds in both presentations. Five
channels are identical to their source files over all 30 seconds; the sixth
is one sample apart from its file, which FFmpeg's decode and the check words
place in the file, not the decode. The downmix matches the coefficients the
encoder printed to within -82 dB. FFmpeg's FATE sample, a 16-bit stereo
sample (identical to its source but for the very last sample, where FFmpeg
agrees with oadec), and the four fixtures FFmpeg's encoder wrote for the tests
decode identical to FFmpeg.
