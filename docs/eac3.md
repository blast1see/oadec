# E-AC-3 format notes

Facts about AC-3 / Enhanced AC-3 (ETSI TS 102 366 V1.4.1) that the decoder
relies on and that took a measurement or a second reading to pin down.
Clause numbers refer to that document unless another is named; "measured"
means checked on the streams listed under `docs/evidence/`.

Two coding tools are not described the same way in every edition of the
standard, so this file names the document each time: **ETSI TS 102 366**
V1.2.1 (2008) and V1.4.1 (2017), and **ATSC A/52** :2012 and :2018. The
four are the same specification, and for enhanced coupling they disagree.

- **Syntax detection.** `bsid` sits at bit 40 of every syncframe for both
  syntaxes (AC-3 after `syncword crc1 fscod frmsizecod`, E-AC-3 after
  `syncword strmtyp substreamid frmsiz fscod numblkscod acmod lfeon`).
  `bsid <= 8` is AC-3, `11..=16` is E-AC-3, 9 and 10 must not be decoded
  (clause E.1.3.1.6).
- **Reduced sample rates.** V1.4.1 reserves `fscod == 3`; earlier revisions
  put `fscod2` in the two bits that otherwise carry `numblkscod`, with six
  blocks per frame. The header parser keeps that reading.
- **Frame CRC.** `crc2` checks when the CRC-16 (`x^16 + x^15 + x^2 + 1`,
  register 0) over the frame without its sync word ends at zero; for AC-3
  the first 5/8 of the frame (`(words >> 1) + (words >> 3)` words) checks
  the same way for `crc1` (clause 6.10.1). Measured: 0 failures on 3.3
  million frames.
- **SNR offset.** `snroffset = (((csnroffst - 15) << 4) + fsnroffst) << 2`;
  the text of clause 6.2.2.1 leaves the precedence to the reader.
- **Default coupling band structure (E-AC-3).** Table E.1.12 is indexed by
  the absolute coupling sub-band number; the transmitted `cplbndstrc[bnd]`
  is relative to `cplbegf`. With `cplbegf = 11` the default gives 2 coupling
  bands out of 4 sub-bands. Copying the table by relative index desynchronised
  every frame of the stereo stream (measured, then fixed).
- **Dither.** Zero-bit mantissas take random values when `dithflag` is set;
  the sequence is the implementation's (clause 6.3.4). Two conforming
  decoders therefore never produce the same samples; they agree to 35–55 dB
  depending on the content, and to float precision where no bin is
  dithered. The dither of coupled bins is scaled by the coupling coordinate
  (up to 8x), which is where most of the residual of stereo streams lies.

  The *scaling* is not free, though, and it is measurable without knowing
  anyone's sequence. ATSC A/52 clause 7.3.4 calls 0,707 optimum, 0,75 close
  enough and 0,5 also acceptable. Decoding a stream with the dither on and
  off gives our dither by subtraction; subtracting the same silent decode
  from the Dolby one gives theirs. The power ratio is 2,00 on fifteen
  channels of three streams, so Dolby scales by 0,5, and oadec now does too.
  It is worth 1,76 dB on every channel of every stream. The Dolby decode is
  also repeatable to the byte, so their sequence is seeded rather than
  sampled; recovering it, and with it bit-exactness, has not been tried.
- **Decoder delay.** The first 256 output samples are the first block's
  half window over silence. The Dolby decoder drops them; FFmpeg and oadec
  keep them. Object metadata counts time from the first frame's first
  sample, so the object output drops them too.
- **Block switching.** The pair of 256-sample transforms takes the even
  coefficients for the first half and the odd ones for the second, as
  clause 6.9.4.2 says; swapping them ruins every switched block (tried).
  Clause 6.9 is self-consistent: the forward transform of clause 7.2.3.2
  followed by the inverse reconstructs to 1e-9 in the unit tests.
- **Window.** The Kaiser-Bessel-derived window with alpha 5 computed in
  double precision matches the five-decimal table 6.33 within 6e-6.
- **AHT.** The inverse DCT of clause E.2.4.5 is `C(k,m) = sqrt(2) sum_j
  R_j X(k,j) cos(j(2m+1)pi/12)` with `R_0 = 1/sqrt(2)`; the text extraction
  of the PDF garbles it.
- **Gain-adaptive quantization.** The helper array of clause E.2.4.2 marks
  a bin as gain-coded from its `hebap` alone, but clause E.2.4.4.2 says
  `gaqmod == 0` transmits no gain words and uses the plain quantizer
  throughout. Reading a gain word for those bins reads the wrong bits (and
  crashed the decoder before the low-rate material was made).
- **Getting AHT and spectral extension to appear.** No film in the corpus
  used either. Encoding 5.1 material with DEE `pcm_to_ddp` at 384 kbit/s
  gives AHT in almost every frame, and at 192 kbit/s spectral extension in
  every frame as well; below 192 the encoder refuses 5.1. Measured in
  `docs/evidence/2026-09-09.md`.
- **Spectral extension noise** is drawn by the decoder, like the dither of
  clause 6.3.4, so decoders differ audibly less than the raw ratio
  suggests: on a stream with SPX in every frame two conforming decoders sit
  15-19 dB apart on the surround channels. The *level* is right, though, and
  that is checkable: against both Dolby decoders on `pi-head-spx192.ec3` the
  17-22 kHz band is 0,0 dB energy-weighted over the whole file and 0,0 dB
  median per frame, with the two envelopes correlated at 0,997 in log energy.
  Measure it over long windows sampled sparsely and it appears to be 25 dB
  out, because most of what that adds up is two decoders' noise floors; a
  ratio of two silences is not a measurement.
- **EMDF placement.** JOC streams carry one EMDF container per frame in the
  skip field of an audio block (clause H.1); parsing the skip fields is exact,
  scanning the frame bytes for `0x5838` is not (false syncs in audio data).
## Substreams, and the programme they make

A programme of more than 5.1 channels is an independent substream carrying a
5.1-compatible downmix followed by the dependent substreams that replace and
extend it. Four clauses settle everything a decoder needs.

- **Association is bitstream position.** Clause E.1.3.1.2: "dependent
  substreams shall immediately follow the independent substream with which they
  are associated." Substream ids are assigned sequentially in that order, an
  AC-3 stream inside an Enhanced AC-3 stream is treated as independent substream
  0, and independent substream 0 is always first. Nothing is inferred from ids
  or from file order.
- **Replace or add.** Clause E.2.8.2: with `chanmape` clear, the dependent
  substream's own `acmod` and `lfeon` name its channels and those overwrite the
  matching channels of the independent substream; with `chanmape` set, the
  custom channel map names them, matching locations replace and the rest are
  additional. At most 16 channels are rendered for one programme.
- **The map.** Clause E.1.3.1.8, table E.1.4, bit 0 stored in the most
  significant bit of the 16-bit field: 0 L, 1 C, 2 R, 3 Ls, 4 Rs, 5 Lc/Rc,
  6 Lrs/Rrs, 7 Cs, 8 Ts, 9 Lsd/Rsd, 10 Lw/Rw, 11 Vhl/Vhr, 12 Vhc, 13 Lts/Rts,
  14 LFE2, 15 LFE. A pair bit stands for two adjacent coded channels, and the
  number of locations shall equal the substream's coded channel count.
- **A second programme is legal.** Clause E.2.8.3: another independent substream
  is another programme. The default is programme 1, and the rest are skipped,
  dependents included.

**Two maps occur in the wild, and the file name does not say which.** Sweeping
the 40 eight-channel E-AC-3 tracks in one library, ten seconds from ten minutes
into each:

| `chanmap` | locations | programme | titles |
|---|---|---|---|
| 0x1a00 | Ls, Rs, Lrs/Rrs | 7.1 | 35 |
| 0xa010 | L, R, Vhl/Vhr | 5.1.2 | 5 |

Both replace two of the core's channels and add two. FFmpeg reports the same two
layouts on the same titles. A track whose file name says 7.1 is quite capable of
being 5.1.2, so the map is what to read.

A typical group, measured on a Blu-ray remux: an AC-3 5.1 core at 640 kbit/s,
2 560 bytes, immediately followed by an E-AC-3 dependent substream with
`strmtyp` 1, `substreamid` 0, `acmod` 5 (four coded channels), `lfeon` 0,
`chanmape` 1 and `chanmap` 0x1a00, 3 584 bytes. The dependent substream's Ls and
Rs replace the core's matrixed pair and its Lrs and Rrs are added, giving
L C R Ls Rs LFE Lrs Rrs.

- **Interchange order is the order of the WAVE mask bits**, which is why the
  rear pair comes out before the side pair: BL 0x10 and BR 0x20 before SL 0x200
  and SR 0x400, giving FL FR FC LFE BL BR SL SR and mask 0x63f. That is FFmpeg's
  order for the same stream. Deriving the order from the mask rather than from a
  second table is what stops the two disagreeing.
- **Release must be keyed on the frame group, not on arrival.** Each substream
  has its own decoder, and enhanced coupling makes a decoder hold a frame back
  while transient pre-noise processing holds samples back. A substream using
  either releases a group later than one that does not, so pairing whatever came
  out of one decoder with whatever came out of another misaligns the programme
  as soon as an encoder switches a tool on in the core and not in the dependent
  substream.
- **Dependent-substream `bsi` had never run on real material** before this,
  because the filter that dropped those frames sat before the frame was parsed.
  It matters more than it sounds: `Frame::parse` seeks the audio blocks to the
  bit the `bsi` ended at, so one bit wrong there misreads the whole frame, and
  the frame CRC will not catch it because the CRC is over the bytes. The check
  that settled it was bit slack, `frame_bytes * 8 - used_bits`: on the 1917
  extract the dependent substream leaves 18 to 1 281 bits unread over 1 875
  frames, and the floor of exactly 18 is the mandatory `auxdatae` and error
  check that closes every frame.

- **EMDF has two legal homes.** Annex H clause H.1 says the container "may be
  carried within an AC-3 or Enhanced AC-3 syncframe using one of the reserved
  data spaces ... for example the auxdata field located at the end of the
  syncframe, or the skip fields present at the end of each audio block". TS
  103 420 clause 8.2 narrows it for object audio: with dependent substreams
  present, the container carrying OAMD and JOC is in the **last dependent
  substream**. `auxdata` is read structurally -- `auxdatae` 18 bits from the end
  of the frame, `auxdatal` in the 14 before it, the user data ending where
  `auxdatal` begins -- and never scanned for. No stream measured so far carries
  any: 0 frames with auxiliary user bits across the corpus.

- **Coded channel order** is table 4.3 (`L C R Ls Rs`, LFE last); the WAVE
  and FFmpeg order is `L R C LFE Ls Rs`.

## Enhanced coupling: four documents, two different tools

ETSI TS 102 366 **V1.4.1** clause E.2.5.5 has three subclauses and reduces
the whole tool to one line,

```
chmant[ch][bin] = ecplmant[bin] * ampbnd[ch][sbnd];
```

with no carrier reconstruction and no angle, and its audblk table prints the
coordinate field as `reserved ... 9 x (necplbnd - 1)`.

ETSI TS 102 366 **V1.2.1** clause E.2.5.5, ATSC **A/52:2012** and
**A/52:2018** clause E.3.5.5 carry four subclauses and a complex process:
reconstruct a non-aliased carrier `Z[k]` from the previous, current and next
blocks, rotate it per band by `ecplangle`, de-correlate it per bin by
`ecplchaos`, then project back. They name the fields and loop them over every
band, `9 x necplbnd`. The three agree word for word; V1.4.1 is the odd one
out, and it is nine bits short.

**What Dolby does.** Rewriting a real stream's standard coupling as enhanced
coupling (`oadec eac3-ecpl-inject`, see `docs/evidence/2026-09-09-b.md`) makes
the question answerable. Two streams were built from the same source, one with
every `ecplangle` and `ecplchaos` zero and one with a full spread of both:

- the Dolby Encoding Engine 5.2.1 `ddp_decode` filter returns **bit-identical**
  audio for the two;
- Dolby Reference Player 3.2.0 returns **bit-identical** audio for the two;
- Plex's EasyAudioEncoder, the licensed Dolby engine its transcoder is built
  against, returns **bit-identical** audio for the two.

Neither Dolby decoder implements the angle or the chaos. oadec therefore
follows the amplitude-only reading by default, which is what the ETSI V1.4.1
text describes, and `--ecpl-spec` selects the full ATSC process. The parser
always reads `9 x necplbnd`, because that is the field the encoder wrote and
V1.4.1's count would desynchronise the block.

- **Enhanced coupling amplitudes.** `ecplamp == 31` is minus infinity;
  otherwise `ecplampmanttab / 32` shifted down by `ecplampexptab` (table
  E3.10), giving 0 dB to -45,01 dB in roughly 1,5 dB steps.
- **The sub-band tables line up.** `ecplsubbndtab[4 + k] = 37 + 12k`, exactly
  the standard coupling grid `37 + 12*cplbegf`, and `ecplsubbndtab[ecplendf +
  7]` is `37 + 12*(cplendf + 3)`. A stream can therefore be moved from one
  tool to the other without touching an exponent or a mantissa.
- **The overlap-add of clause E.3.5.5.1 step 3 is printed without a factor
  two.** The main body's overlap-add (clause 6.9.4) carries it. Measured: as
  printed, a unit-amplitude zero-angle channel comes back at exactly
  0.500000 of the coupling channel, 6,02 dB down. oadec applies the factor,
  and `ecpl::tests::unit_amplitude_and_zero_angle_return_the_carrier` pins it.
- **The angle interpolation of clause E.3.5.5.3 indexes two different
  arrays** under one name: `angle[ch][bnd]` is read while `angle[ch][bin]` is
  written, and the bin indices are relative to the start of the region. Read
  as one array the band values would be overwritten before they are used.

## The bit allocation where enhanced coupling begins at its lowest sub-band

With `ecplbegf` 0 the coupling channel begins at bin 13, which is band 13 of
the masking model, and the channels coupled into it end there. Standard
coupling cannot begin below bin 37, and no channel of AC-3 but the LFE ends
below band 22. So this is the one configuration that asks the excitation
function of clause 6.2.2.4 two things, and the pseudo-code of the clause
answers neither the way an encoder does:

- **The coupling channel has the low-frequency compensation.** The clause
  starts a coupling channel in the loop that has none (`begin = bndstrt`),
  which is the same thing for one that begins at band 31 or later. One that
  begins at band 13 is compensated like any other channel while its bands are
  below 22: `calc_lowcomp` runs, from zero, and the excitation is
  `max(fastleak - lowcomp, slowleak)`.
- **The last band of a channel looks at no band after it.** `calc_lowcomp`
  compares a band with the one after it. The clause leaves the comparison out
  at the last band of the LFE channel; a channel that ends at band 13 leaves
  it out at its last band as well.

Neither is in a document. Both were found on the streams of an encoder that
couples from the lowest sub-band at 64 kbit/s, stereo (MainConcept TotalCode
Studio 5.3, its Dolby Digital Plus Professional encoder). Read as the clause
prints it, 74 of the 626 frames of the first stream ended in a mantissa code
that does not exist, in a group above its range or past the end of the frame,
and the Dolby decoder read every one of them. With the first rule alone 28
frames fail, with the second alone 48, with both none, and none of the 2 879
frames of four streams: pink noise in level steps, two independent noises,
tones with a sweep, and bursts. Below 1,2 kHz, where each channel codes its
own bins, a frame of the two streams that carry no transient is then within
89 dB of the Dolby decode at the median, the channel that carries the sweep
within 68 dB; the two decodes have the same energy to 0,03 dB in every
region, and what they differ by above is the dither of the bins that got no
bits. `tools/band_compare.py` is the measurement, and
`docs/audit/evidence/engine-round-2026-11/28-coupling-from-the-lowest-band.json`
the record. The exponents and the parameters of two blocks of those streams
are kept as tests, with the pointers their mantissas are packed with
(`bitalloc::tests`).

## Transient pre-noise processing

Both ETSI clause E.2.7.2 and ATSC clause E.3.7.2 give the same pseudo-code,
and A/52 adds that "the reference decoder shall implement" it. FFmpeg parses
the fields and applies nothing.

- **The frame begins one block into the samples a decoder hands out.** The
  transient is counted from "the first decoded PCM sample contained in the
  audio frame" (clause E.2.7.1). A decoder that hands out everything its
  transforms make begins a stream with the half block the first transform
  fades in over, as oadec and FFmpeg do. The Dolby decoders drop that half
  block: the decode filter of the Dolby Encoding Engine starts 256 samples
  later for it. Their frame begins one block later in the same samples, and
  that is where the count starts. Measured on both Dolby decoders, to the
  sample: a correction ends 256 samples after `transprocloc` counted from the
  first sample oadec hands out for the frame, and on the two streams whose
  transients are sharpest the fit is 3 to 7 dB worse one sample either way.
  `tools/tpnp_fit.py` is the measurement.
- **The parameters reach forward and backward.** `transprocloc` is ten bits of
  four samples, so a frame's transient can sit up to 4 092 samples past the
  start of that frame, in a frame not decoded yet; the substitution source
  starts `2*TC1 + 2*pnlen` (up to 1 536) samples before the transient, in a
  frame already decoded. Output can only be released once no future frame can
  still rewrite it.
- **`transproclen` is measured backwards from the pre-noise**, not from the
  transient: clause E.2.3.2.23 says otherwise but the pseudo-code and clause
  E.3.7.2 agree, and the fit below is with that reading.
- **The pre-noise starts at the leading edge of the audio block before the one
  holding the transient**, so `pnlen` runs from 256 to 512 samples.
- **Both fades are raised cosines.** The clause leaves the shape open ("nearly
  any pair of constant amplitude cross-fade windows", with Hanning suggested).
  Where a correction gives way to a loud transient, the Dolby decode is the
  decode without the tool under the window, so the window can be read off it:
  a raised cosine fits to 40 and 42 dB on two streams, a straight line to 18
  and 20. The fade into a correction fits the same way where the sound before
  the transient is loud enough to tell: 33 to 37 dB against 22 to 24 on three
  streams.
- **Measured effect.** Over the corrected samples of a stream, the distance to
  the Dolby decode, in dB below it, without the tool and with it: 0,9 and
  21,8 on `pi-head-spx192.ec3`, a 5.1 stream of Dolby's own encoder with 24
  transients; -6,1 and 29,5 on pink noise in level steps (4) and 1,9 and 26,7
  on bursts (18), both stereo at 64 kbit/s from another encoder; -1,6 and
  35,5 (5) and 2,4 and 26,0 (35) on the heads of two film tracks, stereo and
  5.1. With the correction one block early the five were 3 to 6 dB below it.
  Counted the way `tools/tpnp_window.py` counts, over the samples the tool
  changes, the first stream is 20,9 dB closer to the Dolby decode with the
  tool than without: 21,5 dB on L, 20,7 on R, 16,5 on Ls and 18,0 on Rs.
  Dolby applies the tool; FFmpeg does not.
- **What this section said until 2026-11.** That the tool brought that stream
  3 to 4 dB closer, that a straight line measured 0,3 to 0,6 dB closer than a
  raised cosine, and that the whole residual sat in the first cross-fade and
  was smallest with no fade at all, "which puts a step at the splice that
  Dolby does not have". All three were one thing: the correction was one
  block early. Two decoders whose corrections are a block apart agree where
  the two overlap, because the substitution copies the same samples either
  way; ours began where Dolby had not begun, and ended a block before the
  transient, with the loudest of the pre-noise still in place. A residual
  with a shape is a finding, and this one was written down as a property of
  the reference.

## A frame that ends inside its own tail

A frame closes with at least `auxdatae` and the error check, eighteen bits. On
`The 400 Blows` (1959, AC-3 2.0 at 192 kbit/s) one frame in 1 875 ends three
bits inside that space. Its CRC checks, and FFmpeg, the Dolby decoder and this
one all produce audio for it that agrees to the usual dither floor: measured
against FFmpeg, frame 224 sits at 44,5 dB, its neighbours at 50,1 and 50,4.

So the frame is out of spec but perfectly decodable. Refusing it threw away
32 ms of audio that three decoders agree on, which is the wrong trade. It is
now decoded and counted separately, and `verify` still reports the file as
non-conformant. Found by sweeping 1 368 tracks out of 361 films; it was one of
two failures, and the only one that was ours.

## Delta bit allocation, on frames written for it

Delta bit allocation (clause 6.2.2.6) lets an encoder move the masking curve
of a channel by 6 to 24 dB, up or down, over segments of the fifty bands the
bit allocation works in, and so put bits where its own model of hearing wants
them. A block says one of three things of each channel in two bits
(`deltbae`): new segments follow, the state of the block before goes on, or
there is none.

No stream examined does it. The fields are in 221 of 1 345 library tracks,
every one of them AC-3, and each time they say "none"; Dolby's own encoder
writes nothing else
(`audit/evidence/verification-upgrade-2026-11/13-delta-allocation.json`). So
the path stood on unit tests of two functions, and no decoder had been asked
to agree with it. Nor was any fixture an AC-3 stream: nothing in the tests
decoded an AC-3 frame from its sync word to its samples.

`crates/oadec-synth` writes such frames, and not as an encoder would: there is
no transform in it and no model of hearing. A block is told its exponents and
what every mantissa is to be. The bit allocation of clause 6.2.2 is worked out
a second time, the delta included, from tables the tests hold equal to the
decoder's, and each mantissa is quantised to the quantiser its pointer names.
What a decoder has to make of a frame is then known twice. Exactly, as
exponents, pointers and transform coefficients, which the tests compare with
what the frame parser read. And as samples, to the accuracy of floating
point, through the inverse transform of clause 6.9.4 written from its
definition: a sum of 256 cosines for each sample, windowed and overlapped.
The part of the syntax written is narrow: `bsid` 8, two channels, no
coupling, no rematrixing, long blocks, exponents sent one by one (D15) or
reused, no dither.

The mantissas are what makes such a frame a test. The width of a mantissa
follows from the bit allocation, so a decoder that moved the curve otherwise
than the writer did reads other bits from the first pointer that differs, and
has lost the rest of the block.

### The references

Eight streams, whose names begin `ac3-` (`oadec-synth list`):

| Stream | What it carries |
|---|---|
| `ac3-plain` | no delta; every quantiser from no bits to sixteen |
| `ac3-delta` | every state a block can leave a channel in: new segments, the same again, none, a block that says nothing, on two channels out of step |
| `ac3-delta-codes` | each of the eight codes of a segment, from 24 dB down to 24 dB up |
| `ac3-delta-no-length` | segments of no length: every field of the tool, and no band moved |
| `ac3-delta-eight-segments` | eight segments a channel, the most a block sends, the last ending at the fiftieth band and one beginning at an offset of 31, over channels of the widest bandwidth |
| `ac3-delta-silent-first-block` | a frame whose first block says nothing, after a frame that ended with a delta in force: there is none (clause 4.4.3.47) |
| `ac3-delta-44100`, `ac3-delta-32000` | the states again at the other two sampling rates, where the hearing threshold under the curve and the size of a frame are others |

FFmpeg's decoder and Dolby's give the samples of every one, and so does this
decoder. That is the first time an applied delta has been compared with
anything. Dolby's is the `dlbac3dec` element of the Dolby Reference Player,
run as a program on a file; a second front end to the same engine is within
165 dB of the element on every stream the element decodes.
`tools/synth_accept.py` records what the two
decoders made of each stream in `crates/oadec-synth/specs/ac3.tsv`, and the
tests hold the streams to the record.

Agreement is a signal-to-noise ratio here, since nothing about AC-3 is
lossless, and the tool asks for 80 dB on every channel. That is far from both
sides. A decoder that took one mantissa at another width is under 20 dB, and
at about 0 dB when it happens in every frame. One that read every mantissa is
as near as its arithmetic: this decoder at 152 dB of the writer's samples,
which is what 32-bit float leaves (written to a file, its samples are the
writer's bit for bit), FFmpeg's at 138 to 139 dB, and Dolby's between 103 and
138 dB.

The spread of Dolby's has one cause, and rounding found it. Its decoder holds
the levels of the five symmetric quantisers, the thirds, fifths, sevenths,
elevenths and fifteenths of tables 6.19 to 6.23, to fifteen fractional bits.
Against the writer's samples with those levels rounded so, it is at 138,5 to
138,9 dB on every channel of every reference; against the fractions it is
nearest where the fewest mantissas are that coarse, and by the arithmetic a
stream with no others would leave it at 96 dB. This decoder keeps the
fractions, as FFmpeg's does.

### The questions

Seven more streams, `probe-ac3-…`, are frames the standard forbids, or leaves
to be read two ways, or frames nobody can read. What each decoder made of
them:

| The frame | FFmpeg's decoder | Dolby's decoder | This decoder |
|---|---|---|---|
| The delta is laid out in the mantissas and the fields say "none" | other samples | decodes no frame | decodes no frame |
| "Reuse" after "none" in one frame | applies the segments sent before the "none" | "none" goes on | "none" goes on |
| "Reuse" in the first block of a frame | applies the segments of the frame before | does not decode the frame | applies nothing |
| The reserved code of `deltbae` | gives the frame up | goes on with no delta | goes on with no delta |
| A segment that runs past the fiftieth band | gives the frame up | does not decode the frame | applies the part within the bands |

The first is the control: nobody can read mantissas laid out for a curve the
frame keeps to itself, and nobody does.

*"Reuse" after "none".* The pseudo code of clause 6.2.2.6 applies the stored
segments whenever the code is 0 or 1, and FFmpeg's decoder does as it says.
Table 4.11 calls code 0 "reuse previous state", and the state before is
"none": Dolby's decoder reads it so. It gives the samples of the frames laid
out with the delta off (`probe-ac3-reuse-after-none-kept`) and decodes none
of the frames laid out with it on. This decoder read the sequence as the
pseudo code has it until the frames had been put to Dolby's. It keeps the
state now: for each channel it holds whether a delta is in force, apart from
the code the block sent, and a "reuse" leaves that as it was. It gives the
samples Dolby's decoder gives and refuses the frames Dolby's refuses. No
stream examined has the sequence, none of them applying a delta at all, and
over the corpus the binary before and the binary after write the same bytes
(`audit/evidence/engine-round-2026-11/18-reuse-keeps-none.json`).

*"Reuse" in a first block.* Clause 4.4.3.49 forbids it and so does not say
what it would mean. Three decoders, three readings. The probe has two
layouts, one for FFmpeg's reading and one for this decoder's
(`…-first-block-empty`), and their mantissas differ. Dolby's decoder writes
the same bytes for both: it turns such a frame away before it comes to the
mantissas.

*The reserved code.* Clause 4.4.3.48 says of the coupling channel's field,
whose table the channels' field shares, that a decoder should not decode
audio and should mute. FFmpeg's decoder does that. Dolby's does not, and
neither does this one.

*A segment past the last band.* The pseudo code has no bound and would write
past its array. The two outside decoders agree that such a frame is not to be
decoded.

For the last three `verify` has no rule yet: laid out as this decoder reads
them, it finds the streams clean and `decode` writes their samples with exit
code 0. The tests hold the commands to exactly that, so that the rules, when
they are written, change it on purpose.

### What it gave the tests

Eighteen mutations of the AC-3 path went into `tools/mutants.py`: eleven of
the delta (its codes, where a segment begins and ends, whose segments are
whose, what a block that says nothing leaves in force) and seven of the rest
of an AC-3 frame (the span of `crc1`, the fields of a block, the dither flag,
the hearing threshold and the frame size at the other sampling rates).
Fifteen of them survived the tests as they were. None survives now, and
the tests over the written streams catch all eighteen by themselves.

Not written: the delta of the coupling channel (`cpldeltbae`), which needs
coupling; E-AC-3 frames, which switch the fields on with `dbaflde`; more than
two channels, an LFE, grouped exponents (D25 and D45), short blocks and
rematrixing. Measured:
`audit/evidence/engine-round-2026-11/16-synthetic-ac3-delta.json`.

## What a metadata container declares, and the JOC sequence counter

An EMDF container says three things of itself, around its payloads: the
version of its syntax (TS 102 366 clause H.2.2.2.2: 0 for the syntax of the
annex), a key identifier (H.2.2.2.3: which key its protection words were
computed with, the values "implementation dependent"), and the lengths of
its two protection words (H.2.2.4.1 and H.2.2.4.2, where 00 for the primary
length is reserved). The words are keyed and their computation is not public
(H.2.2.4.3), so oadec does not check them, and nothing described here reads
one. What a container *declares* is public syntax, and it is counted:
`verify --json` has it as `emdf.protection` (`containers`, `key_ids`,
`versions`, `primary_bytes`, `secondary_bytes`, `reserved_primary_code`, and
`changes`, the containers that declare another key identifier or another
length of a word than the container before them), the report in words has a
`Protection:` line where the stream has a container, and three rules file
what stands out, at its frame:

| Rule | Fires on |
|---|---|
| `emdf.syntax_version` | a container whose version is not 0 |
| `emdf.protection_reserved` | a container whose primary length is coded 00 |
| `emdf.protection_changed` | a container that declares another key identifier, or another length of a word, than the container before it |

The other thing a stream says of where it comes from is the sequence counter
of its JOC payloads. TS 103 420 clause 6.3.3.3: it goes up by one with every
frame, to 1 023, and starts again at 1; 0 is the first frame of a bitstream,
or the first after a splice. The object decode has followed it for as long
as it has reset the matrices at a splice. `verify` follows it too, and files:

| Rule | Fires on |
|---|---|
| `joc.sequence_restart` | a payload after the first whose counter is 0 |
| `joc.sequence_gap` | a payload whose counter is neither 0 nor the one after the counter of the payload before it |

`verify --json` has `seq_count_first`, `seq_count_restarts` and
`seq_count_gaps` in its `joc` object, beside `seq_count_zero`. The line the
object decode prints (`splices: … restarted the sequence counter, … did not
follow the previous one`) counts the same two things, and a test holds the
two to each other on a stream put behind itself and on one with a frame
taken out.

All five are at `info`. They fail nothing but a `verify --fail-on info`, and
a decode keeps its verdict. What each is worth is a question for a count of
streams, which is what `info` is for until one has been made:

- **The corpus**, 76 streams of the family, 46 of them with containers
  (79 598 containers). Every stream declares one thing from its first
  container to its last: key identifier 0 in 25 streams and 1 in 21, a
  primary word of four bytes and a secondary of one, version 0. Of the 24
  streams with JOC, 23 begin with a counter of 0 and one with 11, a track
  that does not begin where its encoder began; none has a gap.
- **A library**, the first minute of each of 1 260 tracks (755 AC-3 and 505
  E-AC-3, of 347 titles), 370 of them with containers (688 011 containers).
  The same: key identifier 1 on 226 tracks and 0 on 144, four bytes and one,
  version 0; no track names two keys, and not one container declares anything
  else than the container before it. Of 128 tracks with JOC the counter
  starts again in three, and in none of them at a splice: each holds the
  counter at 0 over its first frames (31, 47 and 375 of them, the longest
  twelve seconds) and counts on from there. The head of the third is kept in
  the corpus, where it is the one stream on which a rule of the five fires.
  No counter fails to follow.

So three of the five have never fired on a real stream, and would mark one
whose containers changed hands; `joc.sequence_restart` fires on streams that
are sound, where every frame of a lead-in says it is the first; and
`joc.sequence_gap` has yet to be seen outside a stream with a frame taken
out. A minute is the head of a track: what a whole one does at a seamless
branch or a change of reel is not in this count, and is why none of the five
is more than information yet.

Measured: `audit/evidence/engine-round-2026-11/31-ac3-census-rules.json`.
