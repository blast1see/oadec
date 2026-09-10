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

## Transient pre-noise processing

Both ETSI clause E.2.7.2 and ATSC clause E.3.7.2 give the same pseudo-code,
and A/52 adds that "the reference decoder shall implement" it. FFmpeg parses
the fields and applies nothing.

- **The parameters reach forward and backward.** `transprocloc` is ten bits of
  four samples, so a frame's transient can sit up to 4 092 samples past that
  frame's first sample, in a frame not decoded yet; the substitution source
  starts `2*TC1 + 2*pnlen` (up to 1 536) samples before the transient, in a
  frame already decoded. Output can only be released once no future frame can
  still rewrite it.
- **`transproclen` is measured backwards from the pre-noise**, not from the
  transient: clause E.2.3.2.23 says otherwise but the pseudo-code and clause
  E.3.7.2 agree, and the reading was confirmed by measurement (below).
- **The pre-noise starts at the leading edge of the audio block before the one
  holding the transient**, so `pnlen` runs from 256 to 512 samples.
- **The cross-fade shape is left open** ("nearly any pair of constant
  amplitude cross-fade windows", with Hanning suggested). Measured against the
  Dolby decode of `pi-head-spx192.ec3`, a linear ramp lands closer than a
  raised cosine on every channel that carries a transient, by 0,3 to 0,6 dB,
  so oadec uses the ramp.
- **Measured effect.** On that stream (24 transients over 18 frames), inside
  the corrected regions the distance to the Dolby decode drops by 3,79 dB on
  L, 3,05 dB on R and 4,00 dB on Rs. Dolby applies the tool; FFmpeg does not.
  `tools/tpnp_window.py` is the measurement.
- **The copy is not the whole tool.** Over the substituted stretch itself the
  printed reading tracks the Dolby decode to the line, so the source offset,
  the region and the length are right. The entire residual sits in the first
  cross-fade, and the variant that measures best there is the one that starts
  the substitution abruptly, which puts a step at the splice that Dolby does
  not have. Dolby reaches the splice already matching the original, which is
  what "time scaling synthesis" in the clause title implies and what its
  pseudo-code, a plain copy, does not do. oadec keeps the printed cross-fade.
  See `docs/evidence/2026-09-09-b.md` and `tools/plots.py`.

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
