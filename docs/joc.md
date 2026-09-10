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

  Measured against Dolby's object decoder on nine titles. Five carry steep
  objects; four are the negative control and are bit-identical either way,
  because the reading cannot reach a stream that has none.

  | title | steep objects | worst, as printed | worst, corrected | median |
  |---|---:|---:|---:|---|
  | Glass Onion | 195 | 25,24 dB | **49,93 dB** | 47,76 → 65,44 |
  | The Jackal | 840 | 26,77 dB | **35,12 dB** | 37,89 → 39,34 |
  | Shaun of the Dead | 30 | 40,75 dB | **43,62 dB** | 46,79 → 53,79 |
  | Red Notice | 105 | 36,21 dB | 36,21 dB | unchanged |
  | Extraction | 30 | 50,51 dB | 50,51 dB | unchanged |

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
  samples the analysis bank produces ten slots earlier; the optimum is sharp,
  more than 20 dB per slot either side. Ten slots is 640 samples, the length
  of the analysis prototype. `MATRIX_ALIGN` in `oadec-joc` holds the samples
  back accordingly. Reading it literally costs 25 to 50 dB of object
  accuracy, which no channel-domain comparison can see.
- **Downmix configurations 3 and 4** (table 47, "with 90 degree phase
  shift") carry the surround pair phase-shifted. Clause 6.6 says nothing
  more, and what the shift is the standard never says at all. Rotating Ls
  and Rs by −j in the QMF domain is right above 141 Hz, where our objects
  and the Dolby decoder's agree to the dither floor. It is wrong below it:
  subband 0 spans 0 to 375 Hz, so it straddles direct current, the image of
  the negative frequencies of a real signal sits inside its passband, and
  one rotation turns both halves the same way so that they cancel. The loss
  reaches 14 dB below 50 Hz.

  What Dolby does was measured, bracketed between the two readings (rotate
  every subband, rotate every subband but the lowest): the operator is the
  identity at direct current and reaches −j by about 141 Hz, the same on all
  three titles. One time slot of delay in a subband is 64 output samples
  exactly, so the operator is a real 37-tap filter across time slots;
  `quadrature.rs` carries it and `tools/gen_joc_quadrature.py` regenerates
  it. `--flat-quadrature` restores the plain reading for measurement.
  Configuration 4 behaves like 3, and that is now measured rather than assumed:
  five library tracks carry it and Green Book's objects come out at 50,13 dB
  worst against Dolby's. Configurations 1 and 2 occur in no track of any file --
  0 of the 118 that carry JOC, across 111 films and 226 Dolby tracks.
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
