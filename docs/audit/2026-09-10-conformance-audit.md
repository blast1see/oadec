# oadec conformance audit

**Date** 2026-09-10 · **Artefact** `oadec 0.2.0`, commit `860c76d`
(`v0.2.0-2-g152dc62`), worktree clean · **Binary** SHA-256
`9dca54db61aeb91844e3e841e830768bf720c14cc52c3a489ab1c912d7ca5165`, built with
rustc 1.98.1 · Provenance: `evidence/00-provenance.json`.

**Scope** Does this decoder reconstruct the audio, channel structure, Atmos
objects, object metadata and timing carried by TrueHD, TrueHD+Atmos, E-AC-3 and
E-AC-3+JOC bitstreams? The reference model is in `reference-model.md`; the
feature-by-feature table is in `conformance-matrix.md`.

**Method** Falsification. Prior claims in `docs/evidence/` were treated as
hypotheses and re-measured with independently written tools rather than with the
decoder's own `compare` command. No result is recorded as PASS without a stored
artefact. Reference decoders used: FFmpeg 2026-09-02, `truehdd` b54f209, Dolby
Reference Player 3.2.0 including its GStreamer object decoders, and Dolby
Encoding Engine 5.2.1. DEE, the Reference Player and Plex EasyAudioEncoder share
one engine and are counted as **one** opinion, not three.

---

## 1. Headline

The base decoders are correct, and both Atmos paths are real. TrueHD is
bit-exact against two independent decoders including Dolby's own. E-AC-3 JOC
performs genuine object-domain reconstruction whose output matches Dolby's
object decoder to 35-41 dB worst case per object, with an inter-object
correlation structure indistinguishable from Dolby's. None of the fake-Atmos
patterns is present in either path.

Two defects are serious. **E-AC-3 dependent substreams are never decoded**, so a
7.1 programme silently becomes 5.1 and `verify` still says CLEAN. And **the
object decode path reports no integrity problems at all**: 99 of 120 injected
bit errors produced object output with exit 0 and no diagnostic, every one of
which `verify` catches.

Beyond that, large parts of the JOC syntax have never been exercised by any
available material, and are recorded as untested rather than passing.

---

## 2. What was measured

### 2.1 Test suites

`cargo test --workspace --locked`: 144 passed, 0 failed, 10 ignored. The media
suite with the real corpus: 10 passed in 716 s.

**Harness defect.** With `OADEC_MEDIA` unset the same media suite reports
"10 passed" in 0.00 s, because each test returns `Ok` after printing
"OADEC_MEDIA not set; skipping". Individual tests also skip silently when a file
is missing. A green run therefore proves nothing, and CI never runs this suite
at all. Every media result below was produced by hand for that reason.

### 2.2 Normative tables (no audio involved)

Regenerating the JOC tables from ETSI's own `ts_103420_tables.c` into a sandbox
and comparing against the committed source: the six Huffman trees are identical
across all 582 node pairs, and the QMF prototype is identical across all 640
values when compared as float64 (the committed text differs only in exponent
formatting). Table 54 matches row for row including the specification's worked
example, where 15 bands and subband 24 give band 13. `joc_num_objects`,
`joc_clipgain`, dequantisation, Tables 47, 48, 50, 51 and 53, the clause 6.2
field order, and both differential-decoding pseudocodes match the printed text
exactly.

One implementation choice: for the first parameter band in sparse mode oadec
takes the transmitted channel index modulo the channel count where the
specification applies no modulo. For a conforming stream the two agree; for a
malformed one oadec silently wraps instead of rejecting.

### 2.3 TrueHD

| Test | Result |
|---|---|
| Presentation 2 (7.1) against FFmpeg, `pi-head50m.thd` | **40 606 400 samples, 0 differing, maximum difference 0** |
| Presentation 3 (objects) against `truehdd` | **60 909 600 samples, 0 differing** (the two CAF files are byte-identical) |
| Presentation 3 (objects) against Dolby `dlbtruehddec presentation=16 out-ch-config=21` | **60 909 600 samples, 0 differing**; Dolby pads channels 12 to 15 with digital silence |
| Sample range | minimum −8 217 040, maximum 7 877 136 over 40.6 M samples; none at full scale, so the truncating 24-bit writer is not reached by this material |

The 16-channel presentation is decoded, not merely detected, and it is exactly
what Dolby's own decoder produces. That is the whole of TrueHD Atmos object
audio: the format carries the objects as lossless channels, so there is no
further reconstruction step to get wrong.

**Integrity, with negative controls.** A single flipped bit deep inside a
substream is reported by `verify` as a parity and CRC failure with exit 7, and
by `decode -p 3` as one failed lossless check and one segment problem.
Corruption placed in substreams 0, 1 or 2 is also caught when decoding
presentation 3, because that presentation's check word covers the combined
output. The code-level concern that only the top substream is verified is
therefore real in letter but not in effect. `verify` performs no lossless check
of its own and reports no lossless statistic; it relies on parity and CRC, which
caught every corruption tested.

**Atmos detection is structural, and the negative control passes.** Cloud Atlas,
whose filename says Atmos, is correctly reported as three substreams with no
16-channel presentation and no Evolution payloads, and the object path refuses
it with a clear message.

### 2.4 E-AC-3 base

Three-way comparison on `pi-head-dee.ec3`, 100 s, against Dolby's `ddp_decode`
and FFmpeg. Dolby's decode leads by exactly 256 samples, as documented; that
offset was measured, not assumed.

| Channel | oadec against Dolby | oadec `--no-dither` against Dolby | FFmpeg against Dolby |
|---|---|---|---|
| L | 48.82 dB | 51.85 dB | 47.09 dB |
| R | 48.72 dB | 51.75 dB | 46.32 dB |
| C | 58.50 dB | 61.49 dB | 56.72 dB |
| LFE | 118.78 dB | 118.78 dB | 118.78 dB |
| Ls | 45.02 dB | 48.07 dB | 43.26 dB |
| Rs | 44.61 dB | 47.59 dB | 42.84 dB |

oadec is closer to Dolby than FFmpeg is on every channel. The LFE carries no
dither and all three decoders agree there to about 19 bits, which is the sharp
test and it passes. Disabling dither moves oadec about 3 dB closer, confirming
that the residual on the other channels is dominated by the dither sequence,
which the standard leaves to the implementation. Float output contains no NaN,
no infinity, no denormals and nothing outside unity.

### 2.5 E-AC-3 dependent substreams — FAIL

Every decode path filters out stream type 1 and any substream identifier other
than zero, and the parsed custom channel map has no consumer. The corpus could
not show this because all 18 existing E-AC-3 files are 5.1 or stereo. A 7.1
track was extracted from the user's own library to test it.

On a 60 s cut of a real Dolby Digital Plus 7.1 track:

| Decoder | samples | channels |
|---|---|---|
| FFmpeg | 2 880 000 | **8** |
| oadec | 2 880 000 | **6** |

L, R, C and LFE match FFmpeg at 63.8, 65.9, 53.4 and 141.9 dB. The two remaining
oadec channels correspond to FFmpeg's rear pair at only 17.0 dB, and FFmpeg's
side pair is absent from oadec's output entirely. So oadec produces the
backward-compatible 5.1 core faithfully, and drops the 7.1 programme.

`oadec verify` on that file prints `Substreams: type 0 id 0: 1875, type 1 id 0:
1875`, so it counts the dependent substream, and then reports **`Result: CLEAN`,
exit 0**. The library sweep contains **40 such 8-channel E-AC-3 tracks, all
previously recorded as clean**.

Three consequences follow. JOC downmix configurations 1, 2 and 4 need seven
downmix channels, which can only arrive through a dependent substream, so those
configurations are structurally unreachable rather than merely untested. Clause
8.2 places the EMDF container in the last dependent substream whenever one
exists, and oadec never looks there; JOC works today only because 5.x streams
have no dependent substream. And EMDF is searched only in the skip fields, never
in the auxiliary data.

### 2.6 JOC detection

Detection comes from the bitstream, not from a label. The `addbsi` extension of
clause 8.3 is parsed and reported, and `complexity_index_type_a` reads 16 on JOC
streams, which is the 15 dynamic objects plus the LFE bed, consistent with
clause 8.3.2.2. The value is reported but never cross-checked against the OAMD
object count.

| Input | JOC extension | payloads | object output |
|---|---|---|---|
| AC-3 5.1 | none | 0 | refused, exit 2 |
| E-AC-3 5.1, no objects | none | 0 | refused, exit 2 |
| E-AC-3 2.0 | none | 0 | refused, exit 2 |
| E-AC-3 JOC, config 3 | flag true, index 16 | 976 OAMD + 976 JOC | produced |
| E-AC-3 JOC, config 0 | flag true, index 16 | 1250 + 1250 | produced |
| E-AC-3 7.1, core plus dependent | none | 0 | refused, exit 2 |

The nine payload-configuration constraints of Table 56 are parsed and never
validated, and the EMDF protection words are read and never verified.

### 2.7 JOC object reconstruction — the central test

oadec's objects were compared against Dolby's own object decoder
(`dlbac3dec out-ch-config=21`, dynamic range suppressed) on six titles, **two of
which appear in neither the fit set nor the held-out set of the earlier work**.
Objects quieter than −80 dBFS are excluded, because comparing two noise floors
is not a measurement.

| Title | lag | worst | median | correlation-structure difference |
|---|---|---|---|---|
| Disclosure Day | 0 | 35.25 dB | 40.24 dB | 0.0010 |
| The Fifth Element (new) | 0 | 35.96 dB | 47.62 dB | 0.0017 |
| Kingsman | 0 | 35.54 dB | 39.59 dB | 0.0001 |
| Knives Out | 0 | 35.72 dB | 41.23 dB | 0.0007 |
| A Quiet Place (new) | 0 | 35.85 dB | 39.88 dB | 0.0012 |
| Shaun of the Dead | 0 | 40.75 dB | 46.79 dB | 0.0002 |

The last column is the point. Reconstructed JOC objects **are** strongly
correlated with one another: 15 objects are built from 5 channels, so they live
in a five-dimensional subspace and must be. On Talk to Me, 31 of the 120 object
pairs exceed 0.9 and the mean absolute correlation is 0.559 — and Dolby's
decoder gives **31 pairs and 0.559 as well**, the two correlation matrices
differing by 0.000 to three decimals. High inter-object correlation is a
property of joint object coding, not a defect, and oadec reproduces the
reference structure exactly.

Against the fake-Atmos checklist: no object matches a base channel, the largest
object-to-core correlation being 0.545; objects have distinct levels and
distinct silence fractions; the LFE is uncorrelated with everything and bypasses
JOC as the note to Table 47 requires; and the object count follows
`joc_num_objects` with genuinely distinct audio in each. None of the failure
patterns is present.

**The ten-timeslot matrix alignment.** Clause 6.6.6 pairs timeslot `ts` with
timeslot `ts`. oadec holds the samples back ten slots, a departure justified in
the project's notes only by a fit against Dolby. Because ten slots is also
exactly the analysis prototype length in slots, "we are compensating for our own
filter bank" was a live alternative. Sweeping the offset against Dolby's objects
reproduces the finding independently:

| matrix offset in slots | mean distance to Dolby |
|---|---|
| 0 (literal specification) | 37.18 dB |
| 6 | 42.68 dB |
| 9 | 47.23 dB |
| **10 (shipped)** | **48.21 dB** |
| 11 | 47.20 dB |
| 14 | 42.56 dB |
| 18 | 38.53 dB |

The optimum is at ten, is symmetric about it, and is 11.0 dB better than reading
the clause literally. The measured sharpness is about 1 dB per slot on this
metric, not the 15 to 20 dB per slot the source comment claims; that comment
overstates it. The alignment is confirmed against one decoder family and remains
a documented deviation from the printed specification.

### 2.8 Object metadata timing

oadec and `truehdd` were run on the same clip and their DAMF metadata compared
event by event. Both emit 132 events. **Exactly 21 differ, every one of them by
exactly +32 samples.** That is the `32 x block_offset_factor` term of clause
5.3.2, which `truehdd` omits and oadec applies; the block-offset histogram for
this clip is 2 645 events at zero and 661 at one, and one times 32 is 32. oadec
follows the printed equation. Positions, gains and every other field agree; the
only other difference is that oadec prints positions as 32-bit floats where
truehdd prints 64-bit, which is ample for a value quantised in steps of 1/62.

### 2.9 Robustness

220 random single-bit flips, 120 on a JOC stream and 100 on a TrueHD stream.

**No panics anywhere**, despite `Crc8::update_bits` containing the one assertion
on the bitstream path.

| Path | trials | panics | non-zero exit | flagged but exit 0 | no diagnostic at all |
|---|---|---|---|---|---|
| TrueHD `decode -p 3` | 100 | 0 | 95 | 3 | 2 |
| E-AC-3 JOC `decode --format damf` | 120 | 0 | 21 | 0 | **99** |

`verify` flags **100 %** of the cases the object path passed over in silence,
and both TrueHD cases as well. The detection exists; it is simply not wired into
the path that produces the deliverable. A worked example: one flipped bit gives
`verify` "1 CRC failures, frame 22, Result: PROBLEMS" and exit 7; gives
`decode --format pcm` the same CRC line but **exit 0**; and gives
`decode --format damf` **no output at all and exit 0**, so the corrupted frame
becomes object audio and object metadata with nothing said.

### 2.10 Coverage gaps found by scanning the corpus

Across 15 JOC clips and 231 645 object updates:

| JOC feature | occurrences |
|---|---|
| dense matrices | 231 645 |
| **sparse mode** | **0** |
| fine quantisation | 231 645 |
| **coarse quantisation** | **0** |
| steep slope | 2 760 |
| **two data points** | **0** |
| downmix configuration 3 | 13 clips |
| downmix configuration 0 | 2 clips |
| **downmix configurations 1, 2, 4** | **0** |

So the sparse Huffman tables, both coarse Huffman tables, and two of the four
interpolation branches of Pseudocode 6 are never exercised by any real stream
available. Sparse mode additionally has two competing readings in the source, of
which only the literal one is reachable; the literal reading is the one the
specification prints, but nothing decides it against a reference.

### 2.11 The Dolby refusal, chased

Dolby's TrueHD decoder refuses `presentation=16` on some Atmos titles that oadec
decodes. The earlier note said "not chased". It was chased here.

| Title | 2-channel control enabled | Dolby presentation 16 |
|---|---|---|
| Pi | true | accepted |
| Talk to Me | true | accepted |
| Braveheart | true | accepted |
| Shaun of the Dead | false | refused |
| Knives Out | false | refused |
| Kingsman | false | refused |

The correlation is perfect across six titles. It is **not** causal: setting the
flag in all 383 major syncs of a Shaun clip and repairing each CRC-16 leaves
Dolby's refusal unchanged. Two further hypotheses were eliminated. Shaun's first
OAMD payload is malformed, its first element overrunning the declared 34 bytes;
cutting the stream to start after it removes the size mismatch and Dolby still
refuses. And the declared 16-channel structure is identical to Pi's: same
`substream_info` 0xFC, same `extended_substream_info` 3, same flags 0x1000, same
12 channels of LFE plus 11 dynamic objects, same payload cadence, same 50-byte
payloads.

The consequence for this audit is the important part: **the three titles whose
object output has never been checked against Dolby are exactly the three Dolby
refuses.** oadec's object reconstruction for them is unverified, not wrong.

### 2.12 Output defects confirmed by inspection

Decoding a TrueHD Atmos clip to ADM BWF and reading the `axml` chunk: the
interpolation length takes only two values, 0.000000 and 0.005208 seconds. The
second is 250 samples, the fixed constant in the writer. The real ramp durations
in that stream are 32 and 1536 samples, and the DAMF output carries them
correctly, so the ADM path discards the real ramp. The same chunk contains
**zero** gain elements and **zero** importance elements: per-object gain and
priority never reach the ADM output at all.

---

## 3. Verdicts

| Area | Score | Basis |
|---|---:|---|
| TrueHD decoding | 96 | Bit-exact against FFmpeg over 40.6 M samples; every integrity check enforced and negative-controlled. Held back by the undetected sampling-rate change, the truncating 24-bit writer and the channel-mask defect. |
| TrueHD Atmos decoding | 92 | Bit-exact against `truehdd` and against Dolby over 60.9 M samples. Held back by the hardcoded ISF count, the forced presentation and three titles Dolby will not confirm. |
| E-AC-3 decoding | 62 | The core is closer to Dolby than FFmpeg on every channel and exact on the LFE, but dependent substreams are not decoded at all, so anything above 5.1 is silently truncated. |
| E-AC-3 JOC Atmos decoding | 85 | Genuine object-domain reconstruction, 35-41 dB worst case against Dolby on six titles, reference correlation structure reproduced exactly. Held back by unreachable configurations and untested syntax. |
| Atmos metadata accuracy | 80 | OAMD parsing is thorough and the timing equation is right where `truehdd` is wrong. Held back by 3-D size collapsed to one scalar, and by gain, priority and ramp lost in ADM. |
| Atmos object audio reconstruction | 90 | Both paths reconstruct real objects. TrueHD is exact; JOC matches the reference decoder including its correlation structure. |
| Timing and synchronisation | 90 | Zero lag against Dolby's objects on six titles; the 32-sample OAMD term applied correctly; the 256-sample E-AC-3 decoder delay understood and measured. |
| Robustness and error handling | 55 | No panics in 220 corruption trials, and `verify` catches everything. But the object path reported nothing on 99 of 120 corrupted streams, and `decode` never sets a non-zero exit on a CRC failure. |

**Overall classification: functionally complete with minor defects**, with one
material exception. For TrueHD, TrueHD Atmos and 5.x E-AC-3 JOC the description
holds without qualification. For E-AC-3 above 5.1 the tool is a base-codec
decoder that silently discards part of the programme, and it should say so.

---

## 4. The eleven questions

1. **Is the TrueHD decoder bit-correct?** Yes. 40 606 400 samples against FFmpeg
   with zero differences on presentation 2, and 60 909 600 against `truehdd` and
   against Dolby on the object presentation.
2. **Is the E-AC-3 decoder correct?** For 5.1 and below, yes, within the limit
   the format allows: closer to Dolby than FFmpeg on every channel, exact on the
   dither-free LFE. Above 5.1 it is incomplete rather than incorrect, decoding
   only the independent substream without saying so.
3. **Does TrueHD Atmos parsing genuinely recover Atmos information?** Yes. The
   16-channel presentation is decoded losslessly and matches Dolby's own object
   output exactly, and OAMD is parsed field by field with correct event timing.
4. **Does JOC reconstruction genuinely reconstruct object-domain audio?** Yes.
   Real QMF-domain reconstruction, matching Dolby per object at 35-41 dB worst
   case, with the reference decoder's inter-object correlation structure
   reproduced to three decimals.
5. **Is OAMD parsed correctly?** Substantially. Every syntax element of clause
   5.5 is read, and the block-offset timing term is applied where another public
   decoder omits it. But object distance, divergence, warp mode and the trim
   decibel values are parsed and discarded, and 3-D size is collapsed to a
   single scalar before output.
6. **Are object audio and metadata synchronised?** Yes. Zero lag against Dolby's
   objects on six titles; metadata events agree with `truehdd` except for the 21
   that carry a non-zero block offset, where oadec is right.
7. **Are bed, channel and object concepts separated?** Yes. The LFE is taken
   from the core and bypasses JOC per Table 47; beds, ISF and dynamic objects
   are ordered from the OAMD program. One asymmetry: the TrueHD path hardcodes
   the ISF count to zero while the E-AC-3 path handles it.
8. **Can it reproduce known controlled Atmos test cases?** Not established. No
   controlled material with known object positions was available, and building
   it was outside what this run could complete. Everything above is
   reference-differential, not ground-truth.
9. **Which parts are proven correct?** TrueHD lossless reconstruction and object
   output; the normative JOC tables; the JOC object reconstruction chain; OAMD
   event timing; JOC detection and its negative controls; the E-AC-3 core at 5.1
   and below.
10. **Which parts are inferred or unsupported?** The ten-slot matrix alignment
    and the low-band quadrature filter are measured against one decoder family
    and are not derivable from the standard. Sparse mode, coarse quantisation
    and two-data-point interpolation have no material. Downmix configurations 1,
    2 and 4 are unreachable. The E-AC-3 dither sequence is deliberately not
    recovered. Three TrueHD Atmos titles cannot be confirmed because Dolby
    refuses them.
11. **What must change?** See the backlog below.

---

## 5. Backlog, in priority order

1. **Decode E-AC-3 dependent substreams, or refuse the file.** `cli/eac3.rs:465`
   and `cli/eac3_objects.rs:511`. Until then `verify` must not return CLEAN on a
   stream carrying a dependent substream. Test: the 7.1 extract must give eight
   channels, or a non-zero exit and a stated reason.
2. **Report integrity in the object path.** `cli/eac3_objects.rs` counts no CRC
   failures. Test: the 99 corrupted streams in `evidence/09-01-fuzz-joc.json`
   must each produce a diagnostic.
3. **Make `decode` exit non-zero on a CRC failure**, as `verify` does.
4. **Make the media suite fail when media is missing** rather than returning
   `Ok`, and run it in CI. `cli/tests/real.rs:14`.
5. **Carry all three size axes to the output.** `spatial/program.rs:243` keeps
   only the first; `adm.rs:504` re-emits it as all three.
6. **Write the real ramp and the real object gain and priority to ADM.**
   `adm.rs:24`, `adm.rs:513`, `adm.rs:490`.
7. **Honour a zero JOC sequence counter** by calling the four `reset()` methods
   that currently have no callers, and reset JOC state after a core decode
   error.
8. **Stop hardcoding the ISF count to zero** in the TrueHD object program,
   `cli/damf.rs:74`; the E-AC-3 path already does it correctly.
9. **Stop silently ignoring `--presentation` for `--format damf` and `adm`**,
   `cli/damf.rs:193`.
10. **Fix the WAVE channel mask** for top-side and wide channels,
    `cli/decode.rs:152`.
11. **Clamp rather than truncate in the 24-bit writer**, `cli/decode.rs:273`, to
    match CAF and ADM.
12. **Detect a sampling-rate change** that leaves the samples-per-access-unit
    count unchanged, `truehd/decoder.rs:641`.
13. **Verify EMDF protection words**, and honour `discard_unknown_payload`.
14. **Surface the three JOC environment variables** in the output, or remove
    them; a decode silently altered by the ambient environment is not
    reproducible.
15. **Correct the source comment** at `quadrature.rs:48`: the measured sharpness
    of the matrix-alignment optimum is about 1 dB per slot on the
    mean-over-objects metric, not 15 dB.

## 6. Not testable with what is available

- Controlled Atmos material with known object positions, trajectories and
  per-object tones. This is the single largest gap in the evidence: everything
  here compares against another decoder, never against authored ground truth.
- Sparse JOC mode, coarse quantisation, two-data-point interpolation.
- JOC downmix configurations 1, 2 and 4.
- Enhanced coupling angle and chaos: no decoder anywhere implements them, so
  `--ecpl-spec` has no oracle at all.
- The E-AC-3 dither sequence.
- Why Dolby refuses three of the six TrueHD Atmos titles.
