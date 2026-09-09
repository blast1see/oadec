# JOC format notes

Facts about joint object coding (ETSI TS 103 420 V1.2.1 clause 6 and 7)
that the decoder relies on. Clause numbers refer to that document.

- **Where the side information lives.** EMDF payload 14 in the skip fields
  of the E-AC-3 frame, next to the OAMD payload 11 (clause 8.2); both once
  per frame in every stream measured.
- **Huffman trees.** The trees of Annex A.1 are shipped as C tables next to
  the specification (`ts_103420v010201p0.zip`); `tools/gen_joc_tables.py`
  turns them into `crates/oadec-emdf/src/joc_tables.rs`. A child value `c <= 0`
  is the leaf `-c - 1` (clause 6.6.3); every value round-trips in the tests.
- **Differential decoding** (clause 6.6.2): dense matrices start from 48
  (coarse) or 96 (fine) and accumulate modulo 96/192 across bands; sparse
  mode places one value per band on one channel and the offset 50/100
  elsewhere. The sparse channel index of band `pb` is
  `(joc_channel_idx[pb-1] + joc_channel_idx[pb]) % nch` with the
  *transmitted* previous index, as printed; an alternative cumulative
  reading is kept behind `SparseIndexMode::Cumulative`. Sparse objects are
  rare (60 of 3.3 million object updates in one stream) and untested.
- **Dequantization** (clause 6.6.4): `(q - nquant/2) * 820 / (4096 (1 +
  quant_idx))`, range about ±9.6.
- **Band mapping** (table 54): 23/15/12/9/7/5/3/1 parameter bands over the
  64 QMF subbands; `band_map` in `oadec-joc` reproduces the table's example
  (15 bands, subband 24 → band 13).
- **Interpolation** (clause 6.6.5): smooth slope interpolates from the
  previous frame's last data point over the 24 time slots of a six-block
  frame (12 + 12 with two data points); steep slope switches at the
  transmitted slot offset. Objects absent from a payload keep their matrix.
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
  Configuration 4 is assumed to behave like 3 (no stream to test).
- **Configuration 0 is all but extinct, and Dolby will not upmix it.** Of the
  113 object-carrying E-AC-3 tracks in the library, 112 use configuration 3
  and one uses 0. On that one the Dolby decoder reports `joc_enable(1)` and
  then chooses channel output: six channels, no objects, where every
  configuration 3 stream gives sixteen. Its JOC parses clean and its OAMD
  carries a bed and fifteen placed objects like any other, so there is
  nothing visibly wrong with it. oadec follows clause 6.6, which draws no
  distinction between the configurations, and reconstructs the objects. The
  path therefore has no Dolby reference; `docs/evidence/2026-09-09-c.md`
  records it as an open divergence.
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
