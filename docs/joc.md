# JOC format notes

Facts about joint object coding (ETSI TS 103 420 V1.2.1 clause 6 and 7)
that the decoder relies on. Clause numbers refer to that document.

- **What real streams actually use.** Whole-file scan of 49 streams in the
  working directory, sixteen library titles chosen across sources and twelve
  streaming titles, 67 streams carrying JOC between them, **170 722 130 object
  updates**. Sparse 3 540 and coarse 92 550 of those; smooth slope with one data point and steep with
  one account for all of them. **Two data points: zero.** So two of the four
  branches of clause 6.6.5 pseudo-code 6, smooth-2 and steep-2, have still
  never been seen. Downmix configurations: 3 everywhere except three streams at
  0 and **three at 4**; 1 and 2 are still unseen. Scanning head clips finds
  none of the rare syntax: the same library scanned three megabytes at a time
  gives zero sparse and zero coarse. Nor is the rare syntax a streaming habit --
  Dangal, a Blu-ray remux, carries sparse matrices, and coarse quantisation
  turns up in eight streams across discs and streaming.
  `docs/audit/evidence/remediation/library-syntax-scan.json`.

- **Where the side information lives.** EMDF payload 14 in the skip fields
  of the E-AC-3 frame, next to the OAMD payload 11 (clause 8.2); both once
  per frame in every stream measured.
- **Huffman trees.** The trees of Annex A.1 are shipped as C tables next to
  the specification (`ts_103420v010201p0.zip`); `tools/gen_joc_tables.py`
  turns them into `crates/oadec-emdf/src/joc_tables.rs`. A child value `c <= 0`
  is the leaf `-c - 1` (clause 6.6.3); every value round-trips in the tests.
- **Differential decoding** (clause 6.6.2): dense matrices start from 48
  (coarse) or 96 (fine) and accumulate modulo 96/192 across bands.

  **Sparse mode is printed wrong, in three places.** Pseudo-code 2 puts the
  offset 50/100 on the channels a band does not select, forms the channel
  index of band `pb` as `(joc_channel_idx[pb-1] + joc_channel_idx[pb]) % nch`
  from the *transmitted* previous value, and accumulates the selected
  channel's coefficient from `joc_mix_mtx_q[ch][pb-1]`, which is that same
  offset whenever the previous band chose a different channel. All three are
  wrong, and each only becomes measurable once the one before it is fixed.

  50 and 100 do not dequantise to zero. Clause 6.6.4 gives
  `(q - nquant/2) * 820 / (4096 (1 + quant_idx))`, so 50 and 100 both come out
  at 0,4004 — an unselected channel would contribute four tenths of a downmix
  channel to every object, which is the opposite of what "sparse" means. The
  code that dequantises to zero is 48/96, the same value dense mode starts
  from. The index accumulates from the *resolved* previous index, not the
  transmitted one. And the coefficient chain runs unbroken across the bands
  whatever channel each one selects, the way dense mode's per-channel chain
  does, instead of restarting at the offset every time the channel changes.

  The seed of that chain is the printed 50/100 and stays there. Reading it as
  48/96, so that sparse and dense start from the same place, is the obvious
  fourth correction and it is wrong: the sparse frames of one clip fall from
  53 dB to −0,4 dB. One constant, two uses, and only one of them misprinted.

  **The first band's index takes no modulo.** Clause 6.6.2 applies
  `% joc_num_channels` from the second parameter band on and not to the first,
  where three transmitted bits can name a channel a five-channel downmix does
  not have. Such a band selects none of them and every channel keeps its
  unselected value; wrapping it onto a real channel would invent one. It cannot
  happen on conforming material — all 150 sparse objects in the library
  transmit 0 to 4 with five channels, and all five sparse clips decode
  byte-identically either way — so this is malformed-stream behaviour and is
  pinned by a unit test rather than measured.

  Measured against Dolby's object decoder on **all** the sparse material there
  is: 150 object updates in ten frames of three streaming titles, out of
  32 493 245 object updates in the whole library. Five clips, each cut around
  the frames `verify --json` names. The number below is the worst sparse frame
  of a clip against that same clip's own median away from those frames, so it
  compares the sparse frames with the ordinary frames of the same decode of the
  same material:

  | clip | sparse frames | as printed | + zero gain and resolved index | + unbroken chain |
  |---|---|---:|---:|---:|
  | Extraction A | 1 | −36,9 dB | −13,8 dB | **−9,9 dB** |
  | Extraction B | 2 | −71,3 dB | −12,1 dB | **−4,7 dB** |
  | Extraction C | 1 | −46,3 dB | −19,7 dB | **−5,7 dB** |
  | Glass Onion | 5 | −75,2 dB | −50,1 dB | **−9,2 dB** |
  | Red Notice | 1 | −58,7 dB | −10,7 dB | **−4,9 dB** |

  Glass Onion is where the third correction shows: two corrections leave its
  five frames 50 dB down, all three bring them to 9. In absolute terms the
  sparse frames end up at 34,6 to 68,4 dB against clip levels of 44 to 60.
  `--sparse-as-printed` restores the printed reading, and a stream with no
  sparse object is bit-identical under all three. Evidence:
  `docs/audit/evidence/remediation/sparse-differential.json`.

  **Settled on material that separates sparse from steep.** The streaming
  collection carries 3 375 sparse objects where the whole corpus before it had
  150, and one clip of Extraction 2 holds 825 of them in 55 frames with **no
  steep object anywhere in it**. The printed reading puts those frames at
  −3,92 dB median against Dolby where the rest of the clip is 45,96; all three
  corrections put them at **46,78 dB against 46,69** -- the level of the clip.
  There is nothing left to explain in sparse decoding.

  The frame that stayed short in the narrower evidence is the one that is also
  steep, with offset 23, and its two-slot residual belongs to that combination.
  `docs/audit/evidence/remediation/sparse-settled.json`.
- **Dequantization** (clause 6.6.4): `(q - nquant/2) * 820 / (4096 (1 +
  quant_idx))`, range about ±9.6.
- **Band mapping** (table 54): 23/15/12/9/7/5/3/1 parameter bands over the
  64 QMF subbands; `band_map` in `oadec-joc` reproduces the table's example
  (15 bands, subband 24 → band 13).
- **Interpolation** (clause 6.6.5): smooth slope interpolates from the
  previous frame's last data point over the 24 time slots of a six-block
  frame (12 + 12 with two data points); steep slope switches at the
  transmitted slot offset. Objects absent from a payload keep their matrix.

  **The steep switch is one slot late as printed.** Clause 6.3.4.4 defines
  `joc_offset_ts = joc_offset_ts_bits + 1`, so the offset is one-based: the
  smallest value it can carry names the first time slot. The `ts` of
  clause 6.6.5 counts from zero, and its pseudo-code compares the two
  directly — `if (ts < joc_offset_ts)` — which holds the previous matrix for
  one slot too many. Dolby's decoder switches at the slot the offset names,
  which is `ts < joc_offset_ts - 1`.

  Measured against Dolby's object decoder on ten titles. Six carry steep
  objects; four are the negative control and are bit-identical either way,
  because the reading cannot reach a stream that has none. The sixth is a
  streaming encode rather than a disc remux.

  | title | steep objects | worst, as printed | worst, corrected | median |
  |---|---:|---:|---:|---|
  | Glass Onion | 195 | 25,24 dB | **49,93 dB** | 47,76 → 65,44 |
  | The Jackal | 840 | 26,77 dB | **35,12 dB** | 37,89 → 39,34 |
  | Shaun of the Dead | 30 | 40,75 dB | **43,62 dB** | 46,79 → 53,79 |
  | Red Notice | 105 | 36,21 dB | 36,21 dB | unchanged |
  | Extraction | 30 | 50,51 dB | 50,51 dB | unchanged |
  | Extraction 2 | 1 020 | 21,74 dB | **30,30 dB** | 27,11 → 36,70 |

  Per frame it is sharper still: of Glass Onion's 13 frames with steep
  objects, four were damaged and nine were already right — 33,82 → 63,29 and
  32,54 → 63,99 and 34,09 → 64,37 and 48,50 → 63,39 dB, the rest identical.
  Shaun's frame 581 goes 23,51 → 58,90 dB. The switch position is a discrete
  parameter and its optimum is sharp: sweeping it over ±3 slots gives 24,7,
  27,1, **49,9**, 25,2, 21,6 and 19,3 dB. Where the two matrices at the switch
  are nearly equal the reading is inert, which is why two titles with steep
  objects do not move at all: they differ by 1,5·10⁻⁴ and 1,1·10⁻⁶ at most.

  Steep is not rare — 737 503 of 32 493 245 object updates, 2,3 per cent.
  `--steep-as-printed` restores the printed reading for measurement, and the
  unit tests pin both. Evidence:
  `docs/audit/evidence/remediation/steep-offset-differential.json`.
- **QMF bank** (clause 7): 64 bands, 640-tap prototype QWIN, analysis
  `Q[sb] = sum_j u[j] exp(i pi (sb + 1/2)(j - 1/2)/64)`. The synthesis phase
  must be `(j - 2n + 1/2)` as in the matrix equation of clause 7.3, not the
  `(2j - 2n - 1)` of its pseudo-code; only the former reconstructs (78 dB).
  Analysis plus synthesis delays by 577 samples.
- **Matrix alignment.** Clause 6.6.6 multiplies the subband samples of time
  slot `ts` by the matrix of time slot `ts` and says nothing about the filter
  bank that produced them. Taken literally that is ten time slots out.
  Measured against the object output of the Dolby decoder on three titles
  from three encoders, the matrix of slot `ts` belongs with the subband
  samples the analysis bank produces ten slots earlier. On the mean over
  objects the optimum is symmetric and about 1 dB per slot: sweeping the
  offset against Dolby's objects gives 37,2 dB at 0, 47,2 at 9, 48,2 at 10,
  47,2 at 11 and 38,5 at 18, 11 dB better than the literal reading; a second,
  independent guard on a clip the constant was never fitted to gave 44,7 dB
  in use against 42,0 and 41,8 one slot either side. Ten slots is 640
  samples, the length of the analysis prototype, and it is what the operator
  on the lowest subband delays by (below). A decoder that holds every subband
  back by that filter and applies the matrix of a slot to what the filter
  hands over in that slot comes to this alignment and no other, and that is
  how this one is built since the operator was measured: the samples are held
  back ten slots and the matrices not at all. The guard stands at 46,2 dB in
  use against 42,0 and 41,8. Reading the clause literally costs 11 dB on the
  mean and far more on the objects whose bands the core decode gets exactly,
  which no channel-domain comparison can see.
- **Downmix configurations 3 and 4** (table 47, "with 90 degree phase
  shift") carry the surround pair phase-shifted. Clause 6.6 says nothing
  more, and what the shift is the standard never says at all. Rotating Ls
  and Rs by −j in the QMF domain is right in every subband but the lowest,
  where our objects and the Dolby decoder's agree to the dither floor. It is
  wrong in the lowest: subband 0 spans 0 to 375 Hz, so it straddles direct
  current, the image of the negative frequencies of a real signal sits inside
  its passband, and one rotation turns both halves the same way so that they
  cancel. The loss reaches 14 dB below 50 Hz.

  What Dolby does there is a filter across time slots, and it is measured.
  One time slot of delay in a subband is 64 output samples exactly, and a
  decode is linear in the taps of a filter put on the lowest subband of the
  two channels. So the taps can be solved for: decode a stream with no filter
  there and then with one tap at a time, and take the combination of those
  that comes closest to the objects Dolby's decoder writes for the stream,
  below 450 Hz, by least squares. `tools/gen_joc_quadrature.py` does that,
  and the hidden `--joc-low-fir` is what lets it: it puts an operator from a
  file in the place of the measured one. Fourteen streams of a library, ten
  of configuration 3 and four of configuration 4, forty seconds of each, give
  the same 21 complex taps, the same either side of the middle one. Half a
  stream alone gives them to a hundred-thousandth, and most halves to a few
  parts in ten million, wherever what goes through the operator is more than
  a ten-thousandth of what the objects have below 450 Hz; one half of the 28
  has next to nothing there and says nothing. The taps of all fourteen
  together leave that band 96,3 to 130,5 dB from Dolby's, 121,1 at the
  median. Nothing less does: held to 19 taps the median is 62,1 dB, to 17
  taps 52,7, to real taps 13,3. `quadrature.rs` carries the taps as they
  were measured; no closed form was found for them.

  From 169 Hz up the operator is the turn by −j of the other subbands, to
  four parts in a thousand, on the image of the negative frequencies as on
  the signal. Below, it is no turn: 1,0022 − 2,4512j at direct current.
  Signal and image meet again in the synthesis, and a real tone comes through
  at its own level, within a third of a per cent, with a phase that goes from
  nothing at direct current through 45 degrees at 25 Hz to the quarter turn
  at 80 Hz. The operator reaches ten slots either side of its middle, and
  those are the ten slots of the matrix alignment above.

  Until 2026-11 the decoder carried something else there: the turn, and a
  real filter of 37 taps fitted between two readings (rotate every subband,
  rotate every subband but the lowest) on three titles. It does to a steady
  real tone what the measured operator does, to 0,2 degrees and 0,6 per cent,
  which is why it measured well on what it was fitted to, and it is another
  operator all the same: 1 at direct current, where the measured one is
  1,0022 − 2,4512j. Within the 21 slots nothing of its form, a turn and a
  real correction times 1 + j, comes nearer to Dolby than 31,9 dB at the
  median. `--flat-quadrature` and the hidden `--joc-low-band untouched` are
  the two readings, kept for measurement.
  Configuration 4 behaves like 3, and that is now measured rather than assumed:
  five library tracks carry it and Green Book's objects come out at 50,13 dB
  worst against Dolby's. Configurations 1 and 2 occur in no track of any file --
  0 of the 118 that carry JOC, across 111 films and 226 Dolby tracks.


  **What each leaves, over a library.** `tools/joc_bands.py` compares two
  decodes of the objects of a stream band by band of 375 Hz, and gives the
  ratio after the best linear fit beside the plain one. Over the 127 tracks
  of a library that Dolby's decoder gives objects for, forty seconds of
  each, with our dither left out, the filter of 37 taps left the lowest band
  at 55,9 dB at the median where the bands from 375 Hz to 9 kHz were at
  71,4, more than 10 dB short of them on 71 tracks and more than 20 dB on 36.
  The best filter on an object gained 1,9 dB of that, so the distance was not
  a gain or a delay of what was written, and inside the band it was largest
  at the bottom: 27,9 dB at 6 Hz and 48,5 at 53 Hz
  (`docs/audit/evidence/engine-round-2026-11/48-where-the-objects-are-far-from-dolby.json`).

  With the measured operator the same tracks stand at 115,2 dB in the lowest
  band at the median, 106,8 to 124,5 between the quarters, and at 90,2 dB at
  6 Hz. That is where the LFE of those tracks is, 115,9 dB, which comes of
  the core and goes through nothing of JOC: over every object heard the
  median is 116,0. The lowest band is short of the bands above it by more
  than 10 dB on no track. The band next to it, 375 to 750 Hz, gains 36,5 dB
  at the median, the whole band stands at 53,5 dB against 49,3, and with
  our dither at 50,8 against 47,0. Of the 1080 objects heard none is
  further from Dolby's than it was, in the lowest band or over the whole
  band.

  **With Dolby's dither in the core.** Above the lowest bands what was left
  between the two decoders was the dither of the mantissas without bits,
  which each drew for itself. The core draws the values Dolby's decoder
  draws since they were read off it (`docs/eac3.md`), and the objects are
  made of the core. Over the same tracks, forty seconds of each, with the
  dither on as a decode has it: the whole band stands at 104,2 dB at the
  median, 96,2 to 112,4 between the quarters, where it stood at 52,3; the
  band at 12 kHz at 74,1 where it stood at 42,0, and the band at 18 kHz at
  61,4 where it stood at -2,8. Every one of the 1080 objects heard on the
  127 tracks is nearer to Dolby's over the whole band than it was.
  `docs/audit/evidence/engine-round-2026-11/51-the-dither-dolbys-decoder-draws.json`.

  **On tracks that were in no corpus.** The operator, the place of the clip
  gain and the dither were each read off what Dolby's decoder writes for
  streams of one library, and measured on that library. Four whole tracks of
  streaming services at 768 kbit/s came after all of it, which nothing here
  had read. Each was cut twice, forty seconds from its head and the forty in
  which its clip gain is most at work, and held to Dolby's object decode the
  same way. The two decoders are at the same sample in all eight excerpts.
  The median object of an excerpt stands at 101,9 to 117,5 dB over the whole
  band and at 118,2 to 131,5 in the lowest band, and the LFE at 113,4 to
  128,0. The stretches with the gain at work, up to 198 frames of 1 250 with
  a gain other than 1 and fourteen different gains, stand no lower than the
  heads. Of 105 objects two are under 90 dB over the whole band, at 60,8 and
  73,3, both in the head of one track.
  `docs/audit/evidence/engine-round-2026-11/61-tracks-that-were-in-no-corpus.json`.

  **The core the objects are made of is the plain one.** Decoding an E-AC-3
  stream to channels, Dolby's decoders decorrelate the channels above the
  coupling begin frequency (`docs/eac3.md`), which this decoder does not do.
  Decoding it to objects, they do not. A JOC stream whose core uses coupling
  stands at -0,9 to 4,5 dB from this decoder's in two bands of the coupled
  region of four of its five channels, and at 43 and 23 dB in the fifth,
  which is not decorrelated; its eleven objects stand at 42 to 48 and at 17
  to 25 dB in the same two bands.
  `docs/audit/evidence/engine-round-2026-11/52-what-dolbys-decoders-do-to-coupled-channels.json`.

  Fourteen tracks stayed under 90 dB in the lowest band with it, and they
  are the fourteen whose clip gain changes within the forty seconds: this
  decoder stepped the gain where the objects left it, 1 216 samples before
  Dolby's decoder begins to move it. The shorter hold of the operator had
  moved that step 512 samples later, nearer to where Dolby has it: on thirteen
  of the fourteen the bands above the lowest gained by it, 1,4 dB at the
  median of the fourteen, and on one they lost 1,3 dB. The clip gain below is
  what was left of them.
  `docs/audit/evidence/engine-round-2026-11/49-the-operator-on-the-lowest-subband.json`.
- **Configuration 0 is rare, and Dolby does upmix it.** It appears twice in the
  material read so far: Snatch, a hybrid disc remux, and Dredd. Dolby gives
  Snatch six channels rather than sixteen objects -- `Channel-based decoding
  joc_enable(1), jocd_out_mode(1)` -- and that single observation used to carry
  the claim that the configuration itself turns its upmix off. It does not.
  Dolby decodes **Dredd's** configuration 0 to sixteen objects, and ours agree
  with its to **52,44 dB** at worst and 54,38 median, the closest agreement of
  any title measured here, with an inter-object correlation structure identical
  to four decimal places. So whatever makes Dolby refuse Snatch belongs to
  Snatch. oadec follows clause 6.6, which draws no distinction, and reconstructs
  the objects either way.
  `docs/audit/evidence/remediation/configuration-0-vs-dolby.json`.

  **Relabelling does not test this, and neither does any other in-place
  rewrite.** A configuration 3 stream relabelled as 0 also drops to six
  channels, and so does a configuration 0 stream relabelled as 3 -- but a
  decoder that discards a JOC payload falls back to the core's six channels
  too, and Dolby discards any payload that has been rewritten. Rewriting
  `joc_offset_ts_bits` to 4 and to 12 moves Dolby's output away from the
  original by exactly the same distance in every frame, to one decimal, while
  the three frames the tool happened not to change come back bit-identical;
  and a sweep of our own switch position against Dolby's decode of a rewritten
  stream has no optimum, rising monotonically to 60,95 dB where our switch
  stops firing at all. Dolby holds the previous matrix on a rewritten payload.
  The likely mechanism is the EMDF protection words, which clause H.2.2.4.3 of
  TS 102 366 leaves implementation-dependent and which no third party can
  recompute. Frame-level rewriting is fine -- `eac3-ecpl-inject` redoes the
  same frame check and Dolby decodes its output normally -- so it is the
  container that is protected.
  `docs/audit/evidence/remediation/payload-rewrite-rejected.json`.
- **The seven-channel configurations do not all end in the same pair.**
  Table 47 gives the downmix channels: configuration 1 ends `Lb, Rb`, the rear
  surround pair that table E.1.4 of TS 102 366 calls `Lrs` and `Rrs`, while
  configurations 2 and 4 end `Tfl, Tfr`, the top front pair it calls `Vhl` and
  `Vhr`. Reading all three as if they ended in the rear pair leaves a real
  stream's height channels unmapped, and every configuration 4 stream found so
  far is exactly that shape: an AC-3 core plus an E-AC-3 dependent substream
  with `chanmap` 0xa010, giving `L C R Ls Rs LFE Vhl Vhr`. The order within the
  pair is measured, not assumed: swapping `Tfl` and `Tfr` costs 29 dB in a
  window where the height channels carry signal -- and nothing at all in one
  where they are digitally silent, which is where the comparison tool's default
  window happens to fall on that title.
- **Clip gain** (clause 6.3.3.2): `(1 + y/32) 2^(x-4)`, over [1; 8,75]. The
  standard defines the value and never uses it again: the word does not
  appear in clause 6.6, which specifies the whole decode. It is the gain the
  encoder took off the downmix, and the decoder puts it back on the objects.

  Measured, because the standard would not say. The same Atmos master was
  encoded twice, once as it stands and once scaled by exactly three. The
  unscaled encode carries 1,000 in all 3 305 frames; the scaled one carries
  360 frames from 1,031 to 1,813. Decoding both cores and taking the
  per-frame ratio gives `scale / clipgain` in every bucket:

  | clip gain | frames | hot/ref | x gain |
  |---|---:|---:|---:|
  | 1,0000 | 408 | 2,9995 | 2,9995 |
  | 1,0312 | 125 | 2,9100 | 3,0009 |
  | 1,0625 | 110 | 2,8234 | 2,9999 |
  | 1,0938 | 30 | 2,7422 | 2,9992 |
  | 1,3125 | 3 | 2,2866 | 3,0011 |

  All six coded channels follow it, LFE included, each within 0,3 % of three.
  So oadec multiplies the object program, bed and objects alike, by the clip
  gain, and leaves the backwards-compatible core exactly as coded.
  `--no-clip-gain` turns it off.

  Where it is multiplied back, and how it gets from one value to the next,
  was read off the object output of Dolby's decoder, which is run as it
  stands (`tools/joc_clip_gain.py`). The decode without the gain is what the
  gain is applied to, so Dolby's objects over it are the gain as Dolby applies
  it, and around a change that stands alone they show the way from one value
  to the other. Forty seconds of a film in which the gain takes
  39 values and changes at 76 frames have 65 such changes. In steps of 16
  samples, counted from the first sample the frame with the new gain decodes
  to, the LFE is 0,029, 0,092 and 0,154 of the way in the first three steps,
  0,967 in the sixteenth, and there from sample 256: a straight line over the
  first 256 samples of the frame, from the gain of the frame before. Where
  the gain holds it is the value of the payload, to four parts in a million
  in every one of 1 255 frames. The objects give the same line
  (0,033, 0,085, 0,154 and on to 0,961).

  The line does not say where in the decoder the gain is. Put on what leaves
  the decoder, with that line, it brings the LFE of the stream to Dolby's
  (135,8 dB) and the lowest band of the objects only to 57,1 dB, short of
  the bands above it: the objects are made of the downmix by a filter bank
  and, in the lowest subband, by a filter 21 slots long, and a gain that
  moves is not the same thing in front of them and behind. Put into the
  downmix as the core decodes it, before the objects are made of it, it
  brings the lowest band of the objects of that stream to 125,9 dB from
  Dolby's, where the step left 31,0. That is where oadec puts it, and what
  its own objects then show of the gain is what Dolby's show, step for step.

  Over the 127 tracks of the library the fourteen whose gain changes stood at
  49,0 dB in the lowest band at the median and 45,0 over the whole band, and
  stand at 126,4 and 58,5; their LFE went from 59,2 to 139,2 dB. The
  lowest band is under 90 dB on no track where it was on 14. Every figure
  of the other 113 tracks is what it was.

  The last samples of an object are what the banks still hold when the last
  frame is in. They came without the gain while it was put on frame by frame
  where the objects left the decoder; made of a downmix that has the gain,
  they have it.
  `docs/audit/evidence/engine-round-2026-11/50-the-clip-gain-in-the-downmix.json`.

  Two other readings are refuted. It is not a matrix range extender: the peak
  quantized coefficient already sits at the 9,61 ceiling when the gain is 1,
  and gain times peak runs past the ceiling when it is not. It is not
  unrelated to level: on a two-hour streaming title the mean core peak is
  0,091 where the gain is 1 and 0,38 to 0,74 where it is not, with the
  maximum pinned at full scale in every bucket.
- **The LFE** is not part of JOC (table 47, note); it comes from the core
  decode and is aligned with the objects by delaying the objects' start by
  the filter bank delay.
- **Object order.** The OAMD program lists beds, then ISF, then dynamic
  objects; JOC objects fill that order minus the LFE. Complexity index 16
  in the streams measured means 15 JOC objects plus the LFE bed.
- **The Dolby decoder writes objects too**, which is what settled the two
  points above. Its GStreamer element `dlbac3dec` takes an undocumented
  `out-ch-config=21` ("RAW") and then emits the coded objects as PCM instead
  of a channel bed, and `dlboar` renders them to up to 9.1.6.
  `docs/dolby-tools.md` has the pipeline; `docs/evidence/2026-09-09-c.md` has
  the measurements.
