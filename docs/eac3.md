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
- **Dither.** Zero-bit mantissas take random values when `dithflag` is set,
  and the standard leaves the sequence to the decoder (clause 6.3.4). Two
  decoders with sequences of their own never write the same samples: they
  agree as far down as the dither of either, 35 to 55 dB below the signal.
  Dolby's decoder draws the same values every time it decodes a stream, so
  its sequence can be read off what it writes, and oadec draws it
  (`crates/oadec-eac3/src/dither.rs`). With the dither on, a decode here is
  then the decode of Dolby's decoder wherever the two read the coded
  coefficients alike. Nothing of that decoder was opened: it was run, on
  streams written for the purpose and on streams of a library, and what it
  wrote was read.

  What it draws:

  - **The generator.** A state of 16 bits, `x -> x * 47989 + 3`. A value
    takes two draws `a` and `b`, read as signed numbers, and is
    `(((a >> 1) + (b >> 1)) * 23170) >> 15` in units of `2^-15`, times
    `2^-exponent`. That is the mean of two draws times `1/sqrt(2)`: within
    0,7071, spread as a triangle, with the power of an even spread within
    0,5.
  - **Sixty states.** For each of the six blocks of a frame and each of the
    five full-bandwidth channels, one for the channel's own coefficients and
    one for those it takes from the coupling channel. A state moves by what
    is drawn from it and by nothing else. A channel whose `dithflag` is clear
    draws nothing, and the LFE is not dithered.
  - **AC-3, and a dependent substream.** The states go on from frame to
    frame: block 0 of a channel takes up where block 0 of the frame before
    stopped. They start as the run of a second generator, `g -> g * 47989 +
    1` from 1: block by block and channel by channel, a step for the own
    state and a step for the coupled one.
  - **An independent E-AC-3 substream.** Every frame seeds all sixty anew,
    from the 16 bits that end 18 bits before its end: what stands before
    `auxdatae`, the bit after it and `crc2`. The second generator runs from
    that word, through the thirty own states and then the thirty coupled
    ones. The encoders behind a library write a different word there in
    every frame and the same run of words in every stream (`17f4 150a b7ab
    e15e` ...); FFmpeg's encoder leaves nought there, and Dolby's decoder
    seeds from the nought. A dependent substream is not seeded from its
    frames, whatever stands there: 226 of its frames in two streams have
    something, and it goes on through them.
  - **The adaptive hybrid transform.** A coefficient without bits takes no
    dither before the transform across the blocks. In each block it takes
    the next draw of that block's own state, `2 * ((s * 28378) >> 15)` with
    `s = (a >> 1) + (b >> 1)`: the ordinary value times `sqrt(6)`, the size
    the transform would have given six of them. Before, this decoder put six
    values through the transform, as FFmpeg's does.

  How it was read. `oadec-synth` writes an AC-3 stream in which no mantissa
  has a bit, every exponent is 8 and every block asks for dither (`cargo run
  -p oadec-synth --example dither_probe`), so that what a decoder writes for
  it is its dither and nothing else. Taken back through the transform, which
  windowed overlap-add makes exact, Dolby's decode of it is whole numbers of
  `2^-15`. The run of them closes after 16 384 values. Two readings of one
  place differ by 23 170 or not at all, and the values modulo that are one
  for every place: the states of a generator that visits each state once.
  Their ranks go `q -> 9081 q + 8719` modulo 16 384, which is two steps of a
  generator whose multiplier squares to 9081, and of the 64 generators of 16
  bits that the rule allows one gives every block of every channel of the
  decode as a run of its own, 474 of 474 (`tools/dither_probe.py read`). The
  states the blocks of the first frame start from are then 12, 14, 22, 24 ...
  steps of the second generator from 0, and a block is 253 values on one
  frame later, or 133, or 73: as many as its channel has coefficients. On a
  real stream, Dolby's decode less this decoder's decode without dither is
  Dolby's dither alone. The coupled states were read that way on an AC-3
  stream, the tail word by trying every bit offset of the frame against the
  states of eight frames (one offset fits all eight), and the hybrid rule on
  952 values, all of which it gives.

  What it gives, against Dolby's decode. On the stream without a bit, every
  sample to the rounding of a 32-bit float: 138 dB. On a minute of each of
  45 streams of the corpus (11 AC-3 and the 34 E-AC-3 that use neither
  coupling nor spectral extension nor the hybrid transform; two, six and
  eight channels), the band from 12 to 18 kHz stood at -3,0 to 31,4 dB at
  the median of a stream's channels and stands at 77,2 to 107,8, and the
  band from 6 to 12 kHz at 86,9 to 119,0 where it stood at 2,1 to 115,2.
  Over the whole band a channel of the four clips of the media test, the
  LFE aside, stood at 31 to 64 dB and stands at 95 to 139. What this decoder
  adds where a stream has no bits is at the level of what Dolby's adds to a
  hundredth of a dB in every channel, and of Dolby's the part that is not
  this decoder's is 68 to 90 dB under it, which is the rounding of Dolby's
  floats (`tools/dither_probe.py against`). One sample of one clip is apart
  for another reason: Dolby's decoder holds a sample at full scale where
  this one writes 1,0054. The objects of a JOC stream are made of these
  channels, and stand with them (`docs/joc.md`).
  `docs/audit/evidence/engine-round-2026-11/51-the-dither-dolbys-decoder-draws.json` is the record.

  What it does not reach:

  - **The coupled region of an E-AC-3 stream.** The dither is right there
    too: what a channel gets in a sub-band is its own run times its coupling
    coordinate. What Dolby's decode has more there is not dither. Its
    decoders decorrelate the coupled channels of an E-AC-3 stream, and this
    one does not: the next point. The 10 streams of the corpus that use
    coupling in E-AC-3, most of them with spectral extension or the hybrid
    transform as well, gain in the channels' own region and stand where
    they stood above it.
  - **The noise of spectral extension** is still this decoder's own. It is
    another run of it than it was: the dither drew from the same generator
    and no longer does. Where a few transients carry a band, a figure
    against Dolby's decode is the luck of that run. The band above 18 kHz of
    the left channel of `pi-head-spx192.ec3` stands at 21 to 32 dB over
    eight decodes that start at different frames, and stood at 19 to 35.
  - **A coupling channel coded with the hybrid transform** gets no dither
    for its coefficients without bits, as before; what Dolby's decoder gives
    them was not read.
  - **Frames of fewer than six blocks** and **a substream converted from
    AC-3** are in no stream at hand. The first take the states of the blocks
    they have; the second is taken as the independent substream it is.

  Before this the decoder drew an even spread within 0,5 from a generator of
  its own. The scale had been measured, as a power ratio of 2,00 to a spread
  within 0,707, and read as "Dolby scales by 0,5". The power was right and
  the reading was not: a triangle within 0,707 has the power of an even
  spread within 0,5.
- **Decorrelation of the coupled channels, which Dolby's decoders do to an
  E-AC-3 stream and this decoder does not.** Above the coupling begin
  frequency the channels of a stream are the coupling channel times a
  coordinate for each (clause 7.4), and that is all this decoder makes of
  them, as FFmpeg's does. Dolby's decoders make more of them when the stream
  is E-AC-3, and nothing more when it is AC-3. For a coupled coefficient `X`
  of the plain decode they write

      a X + b D(X),    a^2 + b^2 = 1

  where `D` is a filter along the blocks, on the same coefficient in the
  blocks before: a delay of one block and an all-pass of three poles,
  `z^-1 (c3 + c2 z^-1 + c1 z^-2 + z^-3) / (1 + c1 z^-1 + c2 z^-2 + c3 z^-3)`
  with `c = (-0,0944, 0,0242, -0,3957)`, which is a real pole at 0,7555 and
  a pair at radius 0,7237 and 117,18 degrees either way. `X` is the
  coefficient as the plain decode has it, with its coordinate on it and its
  dither in it. The pair `(a, b)` is another for every block and for every
  coupling sub-band of twelve coefficients, and of the two channels of a
  stereo stream the second takes `b` with the other sign. It is a
  decorrelation: the level and the spectrum of a channel stay, and the two
  channels are no longer one waveform.

  How it was found. With the dither of both decoders the same, one source
  encoded by FFmpeg as AC-3 and as E-AC-3 stood at 79 to 92 dB against
  Dolby's decode in the coupled region as AC-3 and at 7 to 9 as E-AC-3.
  Fifteen test signals, one thing apart from the next
  (`tools/coupling_mix.py signals`), encoded both ways at 128 kbit/s, say
  what sets it off. A band of noise the two channels share above 5 kHz over
  a tone of 440 Hz, at levels that stay, move slowly or fast, come in bursts
  or with other noise in each channel, or twelve sines in its place: 92 to
  114 dB in the coupled region, as E-AC-3 and as AC-3. The same with noise
  of each channel's own between 1 and 4,5 kHz, under the coupling begin
  frequency: 0,1 to 6,3 dB as E-AC-3 and 89 to 106 as AC-3. With that noise
  30 dB lower: 90 dB again. With it in the left channel only: 5,7 dB in the
  left and 90,5 in the right. What a channel has of its own under the
  coupled region decides what is done to it in the coupled region. One of
  the first kind, whose levels jump every 100 ms, stands at 54 dB in one
  channel: one block of 2 063 holds all of the difference, a block a jump
  falls in.

  What it is. `tools/coupling_mix.py mix` fits the filter on the
  coefficients of the two decodes, and then the pair of every block and
  sub-band. On four of the test streams the fitted taps are the all-pass
  above to 0,00002, the same in every sub-band and in both channels, and
  with the pairs the model gives Dolby's coefficients to 89 to 95 dB. On
  music, whose coordinates move from block to block, it gives them to 81 dB
  with the all-pass fitted on the music and to 89,5 with the one above; with
  the filter put before the coordinate, to 32. `a^2 + b^2` is 1,0000 at the
  median. `a` falls with the sub-band (0,93 in the first to 0,81 in the
  eighth where only the left channel has noise of its own), spreads by 0,02
  to 0,10 from block to block, and is 1 in every block for a channel that
  has none. The same left channel stands at 0,72 to 0,61 when the right
  channel has other noise of its own: the pair of a channel depends on the
  other channel too.

  Both of Dolby's decoders here do it, and do the same: the decode filter of
  the Dolby Encoding Engine and the element of the Reference Player stand at
  110 to 129 dB from each other in the coupled region of four streams, which
  is the 24 bits of the first, and at 0,8 to 8,4 dB from this decoder in the
  channels that are decorrelated.

  More than two channels. On a 5.1 stream of Dolby's encoder the left and
  the right take the filter with opposite signs, the centre takes none, and
  the surround pair takes it with opposite signs again. On a 7.1 stream of a
  library the left and the right take it the same way and the four surround
  channels take another all-pass of the same kind,
  `c = (0,3628, 0,2547, 0,4212)`: a real pole at -0,7588 and a pair at
  radius 0,7451 and 74,59 degrees. Both streams code the coupling channel
  with the hybrid transform, whose coefficients without bits are open
  (above), so the pairs of single blocks could not be read on them, and
  which channel takes which filter was not worked out.

  The objects are not touched by it. A JOC stream whose core uses coupling
  (`pi-head-joc384.ec3`), decoded by Dolby's decoder to channels, stands at
  -0,9 to 4,5 dB from this decoder's in the coupled region of the left, the
  right and the surrounds, and at 43 and 23 dB in two bands of the centre,
  which is not decorrelated. Decoded to objects, its eleven objects stand at
  42 to 48 and at 17 to 25 dB in the same two bands, where the centre
  stands: the objects are made of the plain core.

  What is not known is the rule for `a`. Dolby's patents US 9,489,956 and
  US 9,830,916 describe a decoder that does this to the coupled channels of
  an AC-3 or E-AC-3 stream: an all-pass along the blocks after a fixed
  delay, the second channel with the other sign, and a weight estimated,
  block by block, from how alike each channel is to the sum of the channels
  in the bands under the coupling begin frequency, lowered with frequency,
  with noise added to it. What was measured here has that shape, and the
  weight moves from block to block as a weight with noise in it would. It
  was not read, and nothing of Dolby's decoders was opened; so this decoder
  leaves the coupled region as the standard has it. A decode here and
  Dolby's then differ, in the coupled region of an E-AC-3 stream whose
  channels have wideband content of their own, by as much as two
  decorrelated copies of one signal do: 0 to 9 dB.
  `docs/audit/evidence/engine-round-2026-11/52-what-dolbys-decoders-do-to-coupled-channels.json` is the record.

  An experimental mode does it (`oadec_eac3::decorr`,
  `Decoder::set_decorrelation`), off by default. It is not Dolby's
  decorrelation, and a decode with it is not claimed to equal Dolby's: the
  filter and the signs are taken as measured above, and the weight `a` is
  this decoder's own estimate, with no randomness in it.

  Where it runs: an E-AC-3 independent substream that the caller says is
  the whole programme (`ProgrammeScope::CoreOnly`), in 2/0 without the LFE
  or 3/2 with or without it, in a block with standard coupling whose
  coupling channel is not coded with the hybrid transform. The left and the
  right take `b` with opposite signs, the surround pair too, and the centre
  and the LFE are left alone. AC-3, a dependent substream, a programme that
  has one (which channel of a 7.1 programme takes which filter was not
  worked out), a substream converted from AC-3, 2/0 with the LFE, enhanced
  coupling, a coupling channel coded with the hybrid transform and a
  concealed frame are left as the standard has them, and
  `Decoder::decorrelation_status` says why.

  The weight, per block, per channel and per coupling sub-band:
  `rho` is the share of what the channel has under the coupling begin
  frequency, from bin 2, that the other coupled channels have too, taken
  bin by bin (`s = clamp(X S / X^2, 0, 1)` with `S` the sum of the others,
  weighted by `X^2`), and 1 for a channel with nothing there; it is smoothed
  along the blocks by one half. Then `a = cos t` and `b = sin t`, so that
  `a^2 + b^2 = 1`, with `t = (1 - rho) t0(f) (1 + 0,677 (1 - rho_others))`,
  `rho_others` the mean of the other channels' `rho`, and `t` held to 60
  degrees. `t0` rises with the centre bin `f` of the sub-band along the line
  through `acos 0,93` at bin 54,5 and `acos 0,81` at bin 138,5. Those two
  figures and the 0,677 are fitted on what Dolby's decoder gave the left
  channel of the test signals `low-left` and `low-both` (above, and
  evidence 52), with the left channel's `rho` taken as 0 there and the
  right's as 1 or 0; the bins left out, the smoothing and the 60 degrees are
  choices. They are estimates of this decoder, not Dolby's rule.

  The filter state is zero at the start, after a reset, a change of layout,
  mode or scope, and across a concealed frame. A bin that is not coupled in
  a block starts again from zero; when the coupling range changes, the bins
  that stay keep their past. The first block of a run of coupled blocks of
  a bin comes out as the plain decode and only fills the filter, which is
  this decoder's choice: `a X` alone would take the level of that block
  down by `a^2`. A state value under 1e-30 is taken as zero, so that silence
  does not run the recursion through subnormal numbers.

  What `a^2 + b^2 = 1` keeps is the power of a channel where `X` and
  `D(X)` are uncorrelated along the blocks and the pair stands still while
  `D` rings out. On the stereo fixture `eac3-20-coupled.ec3`, whose coupled
  region is noise, every channel and coupling sub-band keeps its power to
  0,014 dB in the coefficients and in the samples.

  What the mode does not claim:

  - That it equals Dolby's decode, or that its weight is Dolby's rule.
  - That a channel keeps its power after a coupled transient. When a
    channel's own region goes silent, its `rho` goes to 1 and `b` to 0
    while `D` still rings with the transient, so the tail of the transient
    loses its filtered part: the channel keeps about `a^2` of the power the
    transient had. On the authored scene, whose coupled region is silence
    but for one transient in its second to last block, the left channel
    loses 1,8 to 3,1 dB there by sub-band, the right 1,0 to 1,7, the right
    surround 0,3 to 0,6. That is a property of this decoder's rule, not of
    the stream and not something measured of Dolby's decoders.
  - That the first block of a run of coupled blocks is decorrelated: it is
    the plain decode, by this decoder's choice.

  `tools/coupling_mix.py measure` sets a decode with the mode on against
  another decode of the same stream, a reference decoder's among them, by
  channel and sub-band.
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
- **Spectral extension noise** is drawn by the decoder, and is not the one
  Dolby's decoder draws (the dither of clause 6.3.4 is, since it was read
  off that decoder), so decoders differ audibly less than the raw ratio
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
- **Choosing a programme.** `ProgramConfig::programme` names the programme by
  the `substreamid` of its independent substream. `None` is programme 0, the
  default of clause E.2.8.3, and its absence is not an error: a stream without
  it decodes to nothing and its frames are counted as another programme's.
  `Some(n)` is an explicit choice, `Some(0)` included; a programme asked for
  that never comes is `ProgrammeNotFound` at the end of the stream, with the
  ids that did come, ascending. A dependent substream belongs to the
  independent substream before it, whatever its own id, and frames of a
  programme not chosen reach no decoder. On the command line it is
  `--programme N` on `decode`, `compare`, `info` and `verify`; a programme the
  stream does not carry ends the run with exit 2, names the ones it does, and
  leaves no output (to standard output nothing is written, not even a header).
- **A programme whose channels change.** With `LayoutPolicy::Fixed` the
  programme keeps the channels of its first group; a group with another set is
  flagged and counted in `layout_changes`, the channels it lacks are silent and
  the ones it adds are left out. With `LayoutPolicy::Follow` each group comes
  out with its own channels, flagged where they change and counted in
  `layout_follows`, which is not a fault; a substream that leaves the programme
  is flushed at the group it is missing from, so its last frame is delivered
  without waiting for the pending window.

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

That was a head of one track. A shelf read whole has seven tracks with such
frames, each the Turkish track of its file, two channels at 192 kbit/s: 1 472
frames of 1 565 517, from one in a track to 1 309 in the track above. Held
frame by frame against Dolby's decoder while the noise of the two is in step,
the 251 of them that can be so held stand as the frames beside them do:
93,2 dB at the least and 106,8 to 119,8 dB at the median of a track, where the
other frames are at 111,2 to 118,0 dB and their lowest tenth under 101,6 to
105,5 dB. The frames are decoded as Dolby's decoder decodes them, and the
rule says something of an encoder
(`audit/evidence/engine-round-2026-11/62-a-library-read-from-end-to-end.json`).

## A frame that will not decode

A frame of the independent substream that will not decode keeps its place. A
frame of silence is written for it, so a decode is as long as its stream and
what follows the frame is where it belongs in time. The silence is a frame of
coefficients that are all zero, taken through the transform like any other:
the frame before it fades out over the block the two share, and the frame
after it fades in over its first. It is what a TrueHD access unit that does
not decode gets (`docs/truehd.md`), and for the same reason.

The frame is counted, `frame.decode`, and its checksum is still read: a frame
that would not decode and fails its checksum is reported as both, which tells
a frame damaged on the way from one that was written so. It says nothing
else. Its level, its downmix and its coding tools are not counted, and the
metadata container that went with its audio is not a second loss: over the
frame the matrices of the frame before hold, and the objects are as long as
the stream. A decode says what it wrote, and exits 7:

```text
Concealed:         2 frames of silence (64.0 ms) where one would not decode, the first frame 212896
```

A frame of a dependent substream that will not decode is left out of its
group, as it was: the programme falls back on its core there, which is the
better audio.

Until this a frame that would not decode was left out of what was written. A
shelf of a library read whole had three such frames in two tracks: two side
by side whose second checksum fails, and one with both checksums right and a
group of three-level mantissas whose code is above 26, which the standard
gives no meaning and which FFmpeg and Dolby's decoder decode. The decode of
each track came out 1536 samples a frame shorter than its stream, and
everything after the frame that much early, where the two others write 1536
samples for every frame. Decoded again, each track has the frames of its
file, stands where FFmpeg's decode stands along its whole length, and is
sample for sample what it was: the same before the first such frame, and
after each the same a frame later
(`audit/evidence/engine-round-2026-11/63-a-frame-that-will-not-decode-keeps-its-place.json`).
What Dolby's decoder writes in the place of a frame it cannot decode is not
silence: something of its own in the first of the two frames side by side,
and the same again in the second. It was measured and is not followed, as its
repeating of the last good access unit of a TrueHD stream is not.

## A mantissa code above its range

Five of the quantisers have codes the standard gives no meaning (clauses
6.3.3 and 6.3.5). Three 3-level mantissas share a group of five bits, which
is 0 to 26 and can say 27 to 31; three 5-level ones a group of seven bits, 0
to 124; two 11-level ones a group of seven bits, 0 to 120; and a 7-level and
a 15-level mantissa have the codes 7 and 15 to spare. An encoder has no
reason to write one, and a frame with one was refused.

One track of a library has such a frame. Its audio blocks run into the bits
that close the frame, as those of 1 309 other frames of the track do, and the
last group of 3-level mantissas of its last block is read from there: 27. Its
checksums are right, and FFmpeg and Dolby's decoder decode it. Refused, it
cost the track a frame, and with the frame the step of the noise this decoder
draws with Dolby's: from that frame to the end of the track the two stood at
55,0 dB where they had stood at 118,0.

What Dolby's decoder makes of such a code was measured on a frame of a clean
stream. A group or a code of each quantiser in the middle of the frame was
written over with each of the seventeen codes above a range in turn, and the
checksums made right again. A decode is linear in a mantissa, so a decode
with nothing in the mantissas of the code and one with 1 in each of them give
every other, and the values that bring the frame nearest to Dolby's decode
are the values Dolby's decoder gave them. For every one of the seventeen it
is the top level of the quantiser in every mantissa of the code: 2/3 three
times, 4/5 three times, 6/7, 10/11 twice, 14/15. With those values the frame
stands against Dolby's decode at 128,7 dB, where the frames before it stand
at 127,5. FFmpeg takes the code apart as it stands and gets levels no
quantiser has, 4/3 for the first mantissa of a 3-level group, and nothing for
the two codes that are one mantissa.

This decoder reads a code above its range as the highest of the range, which
comes to the same, and says among the coding tools of the stream that it has
one (`mantissa-code-above-range`). It is no finding of a rule: the one frame
seen with one ends inside its tail, and is a finding of that. The track now
decodes with every frame in its place, and stands against Dolby's decode at
118,5 dB at the median from its first frame to its last, its 1 310 frames
that end inside their tail at 108,5
(`audit/evidence/engine-round-2026-11/64-mantissa-codes-above-their-range.json`).

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

## What a stream says of its downmix

A stream of more than two channels can say how it would be mixed down to two:
which downmix its maker prefers, Lt/Rt or Lo/Ro, and for each of the two the
level of the centre channel and the level of the surround channels. AC-3 has
the five fields in the first extra word of its alternate syntax (TS 102 366
clauses D.1.2 and D.1.3, `bsid` 6), E-AC-3 in its mixing metadata (clause
E.1.2.2), where a mode without a centre or without a surround channel leaves
the level of that channel out. Nothing in a decode to the channels of the
stream depends on them, oadec makes no downmix, and until now it read past
them. They are what a second encode of a decode has to be told, so `info` and
`verify` report them: a `Downmix:` line, and `downmix` in the JSON, a list
with an entry for each thing the frames of the first programme say and the
number of frames that say it. A decode prints the line too, with the rest of
what it says of the stream it read.

```text
Downmix:           Lt/Rt preferred, Lt/Rt centre -4.5 dB surround -1.5 dB, Lo/Ro centre -1.5 dB surround -6.0 dB x282
```

An entry has `preferred` ("Lt/Rt", "Lo/Ro", "not indicated" or "reserved",
table D.1.1), the four levels in dB as tables D.1.2 to D.1.5 have them
(`ltrt_centre_db`, `ltrt_surround_db`, `loro_centre_db`, `loro_surround_db`;
strings, since one of the levels is "-inf"), and `codes`, the fields as they
stand in the stream. A level is null where the stream does not say it, and
where it says one of the three surround codes the tables reserve, which
`codes` then shows. The alternate syntax of AC-3 carries all five fields
whatever the mode, and the notes of the tables reserve the meaning of a field
for a mode without the channels it speaks of: such a field is not reported.
A stream none of whose frames says any of it has an empty list, and `none
said` in its line.

The reading is held to what an encoder was told. Dolby's encoders take the
five as options, so a stream they write is one whose fields are known without
reading it. Eight settings have every value of every option among them, the
Lt/Rt and the Lo/Ro value of a channel never the same, and each went through
three encoders: E-AC-3 with JOC from a master, E-AC-3 and AC-3 from a file of
5.1. 22 streams came of it, the AC-3 ones in the alternate syntax, and every
one says, in every frame, the levels its encoder was told and the downmix it
was told to prefer. Three things beside that. Told `ltrt-pl2`, the two E-AC-3
encoders write the code that table D.1.1 reserves, 3, which is reported as
reserved with its number. The AC-3 encoder refuses that option. And an encode
of two channels says nothing in either syntax, though the AC-3 one carries
the word. Measured:
`audit/evidence/engine-round-2026-11/58-what-a-stream-says-of-its-downmix.json`.

A track need not say one thing for its length, and the number of frames
beside an entry is there to be read. Of the 1 088 tracks of AC-3 and E-AC-3
of one shelf, read whole, 727 say nothing of a downmix and 359 say one thing
in every frame. One says its downmix in 96 657 of 217 123 frames, the others
being frames of the plain syntax, in 31 runs. One says a second downmix,
every code zero, in 2 530 of its 285 382 frames, four stretches of 16 to 24
seconds. And one says a second dialogue normalisation, -31 for -27, in four
frames of its last three seconds. Each is an encoding joined of pieces, and
none of the three showed in the first minute of its track
(`audit/evidence/engine-round-2026-11/62-a-library-read-from-end-to-end.json`).

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

### The same container in a TrueHD access unit

The Evolution frame of a TrueHD access unit is this container without the
sync word and the length a skip field puts before it, and it says the same
three things of itself. `verify` has them as `extra.protection` and files
three rules at the access unit: `evo.syntax_version` and
`evo.protection_changed`, at `info` like the two of this family they mirror,
and `evo.protection_reserved`, which is older and a warning. A test makes
each of the three from the fixture, by changing the bits of one frame that
say it and putting the parity of the extra data right; a frame with no
primary word and a secondary word of eight bits is as long as the frame it
was made from, so the reserved code can be shown with nothing else of the
frame moved.

The count is shorter than the one above, because there is one answer in it.
In the corpus, 41 of 55 streams carry Evolution frames, 227 395 of them; in
a library, the first minute of 137 of 157 TrueHD tracks does, 256 309
frames. Every frame of every one names key identifier 0, a primary word of
one byte and no secondary word, at version 0, and none declares anything
else than the frame before it. The key identifier 1 that half of the E-AC-3
streams name is in no TrueHD stream. So none of the three rules has fired on
a real stream; each would mark one whose frames changed hands.

Measured: `audit/evidence/engine-round-2026-11/33-what-an-evolution-frame-declares.json`.
