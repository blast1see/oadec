# oadec ADM BWF semantic-fidelity audit

**Date** 2026-09-11/12 · **Artefact** `oadec` built from the production sources as merged at
`796e022` (unchanged on the `adm-audit` branch), binary SHA-256
`292f5f91b7b26052957dc6dd83f778829400c8dd545de780e4f359e54147506b` · **Evidence**
`docs/audit/evidence/adm/` (17 files, manifest with hashes) · **Matrix**
`docs/audit/adm-conformance-matrix.md` (generated from the evidence) · **Tools**
`docs/audit/adm/` (toolkit, harness, reproduction commands in its README).

**Question** Does the ADM BWF that oadec writes represent the same Atmos scene that oadec decoded:
the same object and bed PCM, the same positions, gain, size, importance, ramps and event times,
attached to the right audio elements, in a container and XML that other tools read the same way?
Structural validity and semantic fidelity are judged separately throughout.

**Method** Falsification. The ADM writer's own input (the DAMF written from the same decode) is
the primary differential reference; the raw OAMD dump is the second; Dolby's own DAMF-to-ADM
converters, Dolby's validators and the EBU ADM Renderer are references, never proof. Every claim
below is reproducible from a stored evidence file. The toolkit (ADM/BW64 walker, timecode
arithmetic, ADM XML and DAMF parsers, semantic normaliser, reconciliation ledger, trajectory
evaluator, PCM comparator, defect injector, profile checker) was written for this audit and
imports nothing from oadec; 134 self-tests pass. 22 injected defects were all detected before any
real file was judged. No decoder or writer source was modified.

Result codes: PASS, PASS-TOL, PARTIAL, FAIL, N/I, N/T, UNK. Evidence classes: SPEC, REFERENCE,
MEASURED, INFERRED, IMPLEMENTATION CHOICE, UNKNOWN / PROPRIETARY.

---

## 1. Executive verdict

**Structural ADM verdict: MOSTLY VALID.** Every default file (bed conformed to 7.1.2, 48 kHz)
passes the container, reference-graph, chna and Dolby Atmos Master ADM Profile v1.0 checks with
zero findings, and Dolby's `atmos_info` 5.7.2 `--validate` and 1.1 accept it once the origin tag
is set. Two options leave the profile silently: `--no-bed-conform` numbers objects below
`AO_100b` when the bed has fewer than ten channels, and a 96 kHz or 44.1 kHz programme is written
against the profile's 48 kHz rule without a word.

**Atmos semantic fidelity verdict: MINOR SEMANTIC LOSS** on every decoded stream tested. Object
and bed PCM are sample-identical between DAMF and ADM on all 590 track pairs of 25 inputs,
including the whole of Pi (1:24:08, 242 322 080 frames, 15.3 GB RF64). Every one of the 36 434
metadata events reconciles with an ADM block at the exact sample, with exact float32 position,
and the raw OAMD update times, including the `32 x block_offset_factor` term, are reproduced
without exception. The ADM is behaviourally identical to what Dolby's Conversion Tool 2.1.2 and
DEE 5.2.1 produce from the same DAMF: same block inventory, same interpolation pattern, and
sample-identical EBU ADM Renderer output in 7 of 7 layout comparisons. The one loss on real
material is the profile's fixed interpolation length: every ramp of 1536 (rarely 32, 768, 1152,
1472) samples becomes 250 samples. That is what the profile mandates and what Dolby writes, but it
is a real deviation from the OAMD trajectory (up to 1.67 room units inside a block; rendered,
36.5-63 dB below signal on film material and 6.9-17.4 dB on a fast synthetic scene) and oadec
never says so.

**For scenes the corpus does not contain** the writer loses more, and still silently: a non-zero
object gain (Dolby's converters keep it), ISF objects (audio and metadata dropped), the second and
third size axes (collapsed in the programme model), importance and time-varying bed gain (both
profile-mandated). No corpus stream carries any of these today; the harness proves the losses at
the writer level.

**One clear statement.** For the TrueHD Atmos and E-AC-3 JOC streams oadec decodes today, its ADM
BWF is a faithful Dolby-profile interchange representation of the decoded scene, equal to Dolby's
own conversion of the same DAMF, with one profile-imposed and undeclared approximation (the
250-sample ramp). As a carrier of arbitrary OAMD scenes it is only a structurally valid
approximation: gain, ISF, 3-D size, importance and bed events do not survive, and nothing tells
the user.

---

## 2. Scope and methodology

### 2.1 What was compared against what

| Layer | Reference | Claim it supports |
|---|---|---|
| DAMF written from the same decode | primary differential (`run_compare.py`, ledger) | what the ADM stage itself loses, alters or invents |
| Raw OAMD (`oadec oamd --dump`) | second differential, TrueHD only | event times incl. block offset; positions to the dump's 4 decimals |
| Dolby Conversion Tool 2.1.2, DEE 5.2.1 `convert_atmos_mezz` | REFERENCE (same DAMF in) | whether a loss is Dolby practice or oadec's |
| Dolby `atmos_info` 5.7.2 / 1.1, `bwf_info` | REFERENCE validators | interoperability only |
| EBU ADM Renderer 2.1.0 (BS.2127) | REFERENCE renderer | rendered consequence of metadata differences |
| Authored scenes (`atmos-author` DAMF, DEE-encoded TrueHD/JOC) | controlled truth from the previous audit | ground-truth positions, gains, sizes, timing |
| Writer harness (`docs/audit/adm/harness`, Rust) | oadec's own public `AdmWriter`/`DamfWriter` driven with explicit events and OAMD structs | behaviours no stream in the corpus can trigger |

Dolby's DRP cannot open ADM or DAMF, and no Dolby Atmos Renderer is installed; the renderer leg is
EAR plus Dolby's converters and validators. This is stated wherever it limits a result.

### 2.2 Material

25 inputs were analysed end to end (decode to DAMF and to ADM, normalise, reconcile, compare PCM):

| Group | Inputs | Notes |
|---|---|---|
| TrueHD Atmos, real | pi-head50m (default and `--no-bed-conform`), talktome-head, shaun-head20m, kingsman-h, knivesout-h, braveheart-h, **Pi whole film** | head cuts end in a truncated access unit, so oadec exits 7 and writes the output; the whole film exits 0 |
| TrueHD Atmos, authored | scene-thd, gainsize-thd, fast-thd (`.mlp` from DEE) | known positions, gains, sizes, trajectories |
| E-AC-3 JOC, real | talktome, kingsman, disclosure-web, greenbook-cfg4, snatch, joc-steep-jackal, extraction-nf, knivesout, glassonion-nf heads | six exit 7 (skipped bytes at the cut), three exit 0 |
| E-AC-3 JOC, authored | scene-joc, gainsize-joc, fast-joc, onset-768, teleport-768 | as above, JOC path |
| Not analysable | thd-96k-revenant | 96 kHz TrueHD without Atmos: oadec exits 2, correctly |

Every corpus bed is LFE-only; every corpus object gain is 0 dB, importance 1.0, size 0; no stream
carries ISF, distance, divergence or out-of-order events. Those semantics were tested with the
harness and Dolby's converters, and are marked N/T for real material where that is all the
evidence there is.

### 2.3 Tolerances

Time is compared in integer samples (timecodes decoded with exact rationals; the 10 µs notation
re-encoded and compared as strings separately). Positions are compared at float32 precision: DAMF
prints the shortest representation of the float32, the ADM prints 10 decimals of the same value.
Gain is compared at 1e-6 relative; Dolby's own float32 drift (5.96e-8) is recorded, not tolerated
away, for Dolby outputs. Absent Z is read as 0 by the profile rule but its absence is recorded.
An ADM `<gain>0.0</gain>` is minus infinity and never equals 0 dB.

---

## 3. Source/reference model

| Semantic | OAMD (ETSI TS 103 420) | DAMF (`.atmos.metadata`) | Dolby Atmos Master ADM Profile v1.0 (SPEC, `01-spec-extracts.json`) |
|---|---|---|---|
| Event time | `sample_offset + 32 x block_offset_factor` inside the access unit (clause 5.6.2) | `samplePos` (integer) | `rtime`/`duration` hh:mm:ss.fffff; blocks are discrete events (section 2.5.1) |
| Ramp | `ramp_duration`, start-anchored (figure 4) | `rampLength` samples | `jumpPosition=1`, `interpolationLength` **0 on the first block, 250 samples afterwards** (table 11) |
| Position | 6-bit x, y and 5-bit signed z codes, room 0..1 | `pos` in -1..1 | cartesian `X/Y/Z`, Z may be omitted when 0 (table 11) |
| Gain | `object_gain` dB / mute | `gain` dB | **only on inactive objects** (`gain 0.0` + `importance 0`); forbidden on active objects (table 11) |
| Importance | `object_priority` 0..1 | `importance` 0..1 | only the inactive marker (table 11); BS.2076 default 10 |
| Size | `object_size` 1 or 3 axes | one `size` | `width=depth=height` identical (table 11) |
| Distance, divergence, screen | `object_distance`, `divergence`, screen flags | `screenFactor`, `depthFactor` | `absoluteDistance`, `objectDivergence`, `screenRef`, `screenEdgeLock` shall not be used |
| Zones, snap | zone constraints, snap | `zones`, `snap`, `elevation` | zone exclusion rectangles (fixed table), `channelLock` |
| Beds | bed channel assignment, bed updates | bed instances, bed events | DirectSpeakers blocks **without** rtime/duration, fixed positions (tables 9/10); configuration sets 2.0…7.1.2 (table 16) |
| Structure | - | `.atmos` presentation | one programme/content, bed `AO_1001`, objects `AO_100b..AO_1080`, sampleRate 48000, at most 128 tracks / 123 elements (tables 17, 21, 23) |

BS.2076-3 section 9.3 and TS 103 420 figure 4 describe the same shape: a linear ramp that starts
at the event and reaches the target after the ramp length, then holds. The faithful ADM image of a
DAMF event is therefore `interpolationLength = rampLength / 48000` with `jumpPosition=1`; the
profile forbids exactly that.

Not available: ITU-R BS.2088 (the `BW64` FourCC; RF64 structure taken from EBU Tech 3306) and
profile v1.1. Both gaps are recorded in `00-provenance.json`.

---

## 4. Internal Atmos to ADM pipeline

```mermaid
flowchart LR
  A[TrueHD bitstream] --> B[cli/damf.rs::run<br/>bed from major sync, isf_objects = 0 :86]
  A2[E-AC-3 JOC bitstream] --> B2[cli/eac3_objects.rs::run<br/>bed + ISF from OAMD, 256-sample DECODER_DELAY :523-528]
  B --> C[Oamd::parse<br/>oadec-emdf/src/oamd.rs:989]
  B2 --> C
  C --> D[Timeline::push<br/>oadec-spatial/src/program.rs:286<br/>time = base + container_offset + sample_offset + 32*bof :332-335<br/>damf_position X=2x-1, Y=2(0.5-y), Z=z :225-227<br/>size = r.size[0] :242]
  D --> E[Event{id, sample_pos, ObjectState|BedState}]
  E --> F[Sink<br/>cli/damf.rs:92-189; presentation forced :205]
  F --> G[DamfWriter<br/>oadec-spatial/src/damf.rs; fps 24 :42]
  F --> H[AdmWriter<br/>oadec-spatial/src/adm.rs]
  H --> I[RIFF/RF64: JUNK->ds64, fmt(tag 1), data, axml, chna, dbmd :186-316]
```

| Responsibility | Location | What the audit found there |
|---|---|---|
| Bed identification and labels | `adm.rs:87-101` (`bed_profile`), `program.rs:56-57` | only the 7.1.2 label set is writable; any other bed channel makes `AdmWriter::create` fail explicitly (harness C14) |
| Dynamic objects, ISF, ordering | `program.rs:136-143`, `adm.rs:178-180` | ISF slots become `Slot::None` in both writers: audio and metadata dropped (C13) |
| Object IDs | `adm.rs` (`AO_` + `0x1001 + bed + k - 1`) | correct with the 10-channel conformed bed; below `AO_100b` otherwise (C11, C12, C21b) |
| Coordinates | `program.rs:225-227` | exact (section 7) |
| Gain, importance | `adm.rs:488-492` | written only for inactive objects, as the profile says; the active gain is lost (C02) |
| Size / 3-D size | `program.rs:242`, `adm.rs:504-509` | first axis only, re-emitted three times (C04) |
| Distance, divergence, warp, trim | `oamd.rs:605-613, 881-904, 808-836` | parsed, never stored in `ObjectState` (C19) |
| Interpolation | `adm.rs:24` (`INTERPOLATION_SAMPLES = 250`), `:437`, `:513-516` | fixed 0 / 250; `ObjectState.ramp` never read by the ADM writer |
| Timing | `program.rs:332-335`, `adm.rs:660-676` | sample-exact; rtime and duration rounded independently to 10 µs |
| Bed events | `adm.rs:252-259` | never stored (profile: no rtime/duration on DirectSpeakers) |
| First block / late first event | `adm.rs:445-466` | synthetic block at 0 holding the first state (C06) |
| Same position / beyond end / out of order | `adm.rs:471-480` | last wins (C08); overrun past the end (C07); overlap (C09) |
| Track UIDs, chna, axml | `adm.rs:576-578` (UID sample rate from the stream), `:186-316` | chain complete on every file; UID sampleRate follows the stream (C15) |
| Diagnostics | `oadec-spatial` has no logging at all | no warning for any lossy path; exit codes reflect bitstream integrity only |

Everything in this table was reproduced by measurement in the sections below; the line numbers are
those of the unchanged production sources.

---

## 5. PCM fidelity

Every declared DAMF track was compared with its ADM track sample by sample over the whole length
(one-pass interleaved read, SHA-256 of the samples, differing-sample count, first difference, peak,
RMS, silence percentage), and a full pairing matrix over a 20 s window was built to catch swaps,
duplicates and offsets independently of the declaration (`adm-object-pcm-compare.json`).

| | TrueHD path (11 inputs) | JOC path (14 inputs) | Total |
|---|---:|---:|---:|
| Track pairs compared | 240 | 350 | 590 |
| Sample-identical | 240 | 350 | 590 |
| Pairing matrix: declared pairs confirmed, non-silent pairs unique | all | all | all |

The whole film (21 tracks x 242 322 080 frames) is included: 21 of 21 identical. The nine
conformed bed channels other than LFE are digital silence in both files, as they should be. No
swap, duplication, drop, offset, interleave error, padding or truncation was found on any input.
The negative controls M-A4 (tracks 11 and 13 swapped), M-A8 (data chunk 3 bytes short) and M-A14
(+1 LSB at sample 1000) all fired.

For the JOC path this proves that the ADM carries **the decoder's reconstructed object signals**
exactly; it does not prove that those signals equal the authoring master (section 14, section 29
of the brief). **Result: PASS (MEASURED).**

---

## 6. Object/bed mapping

Track mapping was derived from each file's own declaration (DAMF header order; ADM `chna` ->
`audioTrackUID` -> `audioTrackFormat` -> `audioStreamFormat` -> `audioChannelFormat` ->
`audioPackFormat` -> `audioObject`) and then confirmed by the PCM pairing matrix
(`adm-track-mapping.json`, `adm-reference-graph.json`).

- Every chna entry resolves through a unique chain; no dangling, duplicate, orphan or ambiguous
  node in any of the 25 files. `numTracks = numUIDs = channels` everywhere.
- Beds stay DirectSpeakers in one custom pack `AP_00011001`; objects are one `Objects` pack and
  one channel format each; the hierarchy `APR_1001 -> ACO_1001 -> AO_1001 + AO_100b..` is the same
  element inventory Dolby's converters produce from the same DAMF.
- Bed channel order is honoured: harness C21 codes the bed as Rss, Ls, LFE with tones 500/600/100 Hz
  and the ADM carries them under `RC_Rss`, `RC_Lss`, `RC_LFE` with conform on and off.
- LFE is sample-identical on every input and sits under `RC_LFE` at the profile's fixed position
  [-1, 1, -1]. A BS.2127 renderer does not recognise that label as LFE (EAR warns); Dolby's own files
  share the limitation because it is the profile's labelling.
- Counts agree between OAMD programme, DAMF and ADM for beds, dynamic objects and total elements on
  every input (all corpus beds are LFE-only; the conformed bed adds nine silent channels by design).
  ISF is the exception: see section 18.

Concept separation, as required: codec channels (TrueHD 16-channel presentation, E-AC-3 core 6/8)
are never counted as objects; JOC reconstructed signals are the decoder's output; OAMD programme
objects define the elements; ADM `audioObject`s are one bed object plus one per dynamic object;
speaker-rendered channels appear only in section 15. **Result: PASS (MEASURED)**, PARTIAL for the
count row because ISF is dropped.

---

## 7. Coordinate fidelity

Positions were parsed numerically on both sides and compared at float32 precision over all 36 434
matched events (`adm-coordinate-diff.json`): **0 value mismatches**, no axis swap, inversion,
origin shift, 0..1 mapping error, clamping or quantisation. Absent Z occurs only where the value
is 0.

Against the raw OAMD dump on six TrueHD clips (`adm-event-timing.json`, `raw_oamd_crosscheck`):
the mapping X = 2x-1, Y = 2(0.5-y), Z = z holds within 9.7e-5 room units, which is the dump's
four-decimal printing, on 711 DAMF states and 694 ADM blocks. The negative controls M-A2
(X += 0.001), M-A3 (block lists of two objects swapped) and M-A9a (non-zero Z removed) fired; the
tolerance-sanity variant M-A2t did not fire at 0.01, as intended.

The metadata-domain **trajectory** is a different question from the block values and is treated
in section 10. **Result: PASS (MEASURED).**

---

## 8. Gain, size and importance fidelity

No corpus stream carries a non-zero gain, a size or an importance other than 1.0
(`docs/audit/evidence/remediation/object-gain-and-size.json`), so these three were driven through
the writer and through Dolby's converters (`adm-gain-diff.json`, `adm-harness.json`,
`adm-dolby-reference.json`).

| Semantic | DAMF | oadec ADM | Dolby CT / DEE ADM (same DAMF) | Classification |
|---|---|---|---|---|
| Gain -6 dB, +3 dB, -inf on active objects (C02) | kept | **omitted** (3 losses, the mid-stream -12 dB change popped) | `<gain>` written on the non-unity objects (4 blocks in C02; 4 of 5 objects in S2, the 0 dB one omitted) | **avoidable loss**: the profile text forbids it, Dolby writes it anyway |
| Importance 0.5 / 0.0 on active objects (C03, S3) | kept | omitted (2 losses) | omitted (0 present) | profile rule, Dolby-identical; absent = BS.2076 default 10 = full priority = corpus value |
| Inactive object (C16, S5) | `active: false` | `gain 0.0` + `importance 0`, position kept | same marker, position zeroed | equivalent; oadec keeps more |
| Uniform size 0.25 / 0.5 / 1.0 (C04, S2) | kept | `width=depth=height` exact | same triple plus `diffuse 1` | PASS |
| 3-D size [0.2, 0.5, 0.8] (C04, via `Timeline::push`) | **0.2** (one scalar) | 0.2 x 3 | n/a (DAMF already collapsed) | collapse happens in the programme model (`program.rs:242`); neither DAMF nor the profile can carry three axes; the choice of the first axis, and the silence, are oadec's |
| Bed gain -6 / -12 dB and inactive bed (C05, S8) | 3 bed events kept | none (LFE PCM unscaled) | none | profile rule; bed changes lost: 2 (harness), 0 on real material |

The user-visible consequence of the gain row: a muted or attenuated active object becomes audible
at full level in any consumer that applies ADM gain, while Dolby's own conversion of the same DAMF
would have played it correctly. It does not occur on any stream decoded so far. A side finding: the
profile checker flags Dolby's own gain-only elements on active objects (`profile-inactive-encoding`
on the Conversion Tool's C02 output), so on this point Dolby's practice and the profile text
disagree, and oadec followed the text.

**Results:** gain FAIL (MEASURED + REFERENCE); importance PASS-TOL (SPEC + REFERENCE); scalar size
PASS; 3-D size PARTIAL (IMPLEMENTATION CHOICE); bed events PASS-TOL (SPEC + REFERENCE).

---

## 9. Event timing

Canonical timebase: integer samples. ADM `rtime`/`duration` were decoded with exact rational
arithmetic to the nearest sample; the 10 µs strings were re-encoded and compared separately
(`adm-event-timing.json`).

| Measurement | Value |
|---|---:|
| Matched DAMF states / ADM blocks (25 inputs) | 36 434 |
| Time mismatches (samples, µs, ms) | 0 / 0 / 0 |
| Timecodes decoded | 73 014 |
| Re-encode mismatches | 0 |
| Block tiling: gaps / overlaps / overruns past the end | 0 / 0 / 0 |
| Objects starting at 0 and ending at the programme end | all |
| Independent-rounding signature (`tile_dec` in 10 µs units) | 0 on every block |
| Whole film: first / last event (samples) | 0 / 241 923 136 (objects 2 and 11); every object first at 0 |
| Whole film: drift between DAMF and ADM over 1:24:08 | 0 samples |

Against the raw OAMD dump (six TrueHD clips, 99 091 updates): 711 of 711 DAMF state times and 694
of 694 ADM block times are OAMD update times; 0 fall outside the set. The DAMF-to-ADM identity
`blocks = states - superseded - beyond_end - trailing_popped - absorbed + synthetic` holds on all
25 inputs (73 duplicate blocks and 59 trailing pops, all ramp-only changes, all on the TrueHD
path; 0 on JOC). Negative controls M-A1 (+1 sample), M-A5 (interior block deleted), M-A11 (+10 µs
inside half a sample) and M-D1 (samplePos deleted) fired.

The 256-sample JOC decoder-delay compensation is applied before both writers, so the ADM inherits
whatever the DAMF has; its absolute correctness was established in the remediation report with the
onset/teleport truth sets, not re-measured here (INFERRED for ADM). **Result: PASS (MEASURED).**

### 9.1 The 32 x block_offset_factor term

21 506 of the 99 091 OAMD updates in the six clips carry `block_offset_factor = 1`. 80 ADM blocks
sit on times that exist only with the +32 term (e.g. 4672, 12352, 20032 in pi-head50m), and every
such DAMF state has its ADM block at the identical sample. The term survives ADM export
unquantised. **Result: PASS (MEASURED).**

---

## 10. Interpolation and ramp fidelity

**Hypothesis A reproduced and classified.** Of 36 434 matched events, 36 158 carry an ADM
`interpolationLength` that is not the DAMF `rampLength` (`adm-interpolation-diff.json`). The
source ramps are 1536 samples in 36 453 events, 32 in 33, 768 in 45, 1152 in 11, 1472 in 24; the
ADM writes 0 on each object's first block and 0.005208 s = 250 samples on every later block,
regardless of the source. The constant is `INTERPOLATION_SAMPLES = 250` at `adm.rs:24`; the
writer never reads `ObjectState.ramp`.

Classification, following the decision tree agreed in the plan:

1. **SPEC.** Profile table 11 mandates exactly this: `interpolationLength` 0 on the first block
   and 250 samples on subsequent blocks, `jumpPosition = 1`.
2. **REFERENCE.** Dolby's Conversion Tool and DEE write the same 0/250 pattern for a DAMF whose
   ramps cycle 0/32/250/480/1536/2048 (S1: histogram {0: 8, 250: 3278}, identical for both tools).
3. **REFERENCE.** Dolby's reader ignores the value: every ADM converted back to DAMF by the
   Conversion Tool returns `rampLength 0` for every event, whether the ADM came from oadec, from a
   variant carrying the real 1536-sample ramps (M-A6 style), or from Dolby's own converter
   (3 286 of 3 286 events in S1, 128 of 128 in the pi-head50m variants).
4. **MEASURED loss.** The trajectory evaluator (validated by M-T1: a synthetic ADM carrying the
   real ramps scores exactly 0) probes each transition at 25/50/75 % and at both ramp ends under
   the start-anchored semantics of TS 103 420 and BS.2076-3 section 9.3. Worst deviation 1.674 room
   units in X and Y and 0.837 in Z (a full-width move probed when the 250-sample ramp has finished
   and the 1536-sample ramp is at 16 %); 105 580 of 1 465 155 axis probes exceed 0.01 room units.
   Rendered with EAR against a real-ramp variant of the same file: minimum SDR 36.5-63.0 dB across
   layouts on pi-head50m, 6.9-17.4 dB on the fast authored scene with lags of 218-982 samples
   between the two renders.

Interpolation **mode** is right: both formats describe a ramp that starts at the event, and
`jumpPosition = 1` is the correct ADM expression of it. Interpolation **length** is PASS-TOL
(SPEC + REFERENCE + MEASURED): the difference is mandated by the chosen profile and identical to
Dolby's output, and its magnitude is now measured rather than assumed. The defect that remains
oadec's is that the loss is silent (section 19). The previous audit's FAIL on this row is therefore
reclassified, with the profile clause and Dolby's files as the reason.

---

## 11. ADM hierarchy and references

Reference graph and profile rules were checked on every produced file (`adm-reference-graph.json`):
`audioProgramme -> audioContent -> audioObject -> audioPackFormat -> audioChannelFormat`,
`audioTrackUID -> audioTrackFormat -> audioStreamFormat -> audioChannelFormat/audioPackFormat`,
chna entries, ID uniqueness and ranges, type consistency, unknown sub-elements. Default files: 0
findings on all inputs including the whole film (12 audioObjects, 12 packs, 21 channel/stream/
track formats, 21 UIDs). `--no-bed-conform` files: `profile-id` (objects from `AO_1002`) and
`profile-bed-configuration` (an LFE-only bed is not a table-16 set), the latter also true of
Dolby's conversion of the same DAMF. Negative controls M-A7 (dangling chna track format) and
M-A13 (UID sample rate 44100) fired. **Result: PASS (MEASURED)** for the default output, FAIL on
the ID rule for the `--no-bed-conform` path.

---

## 12. BW64/CHNA/AXML validity

Container walker plus ffprobe, `bwf_info` and `atmos_info` (`adm-container-validation.json`):

| File | FourCC | Size (bytes) | ds64 riffSize / dataSize / sampleCount | Frames agreed by | Findings |
|---|---|---:|---|---|---|
| Pi whole film, 21 ch | RF64 | 15 278 169 042 | 15 278 169 034 / 15 266 291 040 / 242 322 080 | walker, ds64, ffprobe, bwf_info, DAMF CAF: 242 322 080 | 0 |
| Harness C22 (data > 4 GiB, 2 ch) | RF64 | 4 294 972 786 | 4 294 972 778 / 4 294 967 298 / 715 827 883 | walker = DAMF CAF | 0 container findings |
| Harness C22b (data < 4 GiB, RIFF size > u32) | RF64 | 4 294 972 430 | 4 294 972 422 / 4 294 967 292 / 715 827 882 | walker = DAMF CAF = bwf_info | 0 container findings |
| 24 RIFF-sized files | RIFF | - | JUNK(28) placeholder | walker = DAMF | 0 |

Chunk order is `ds64` (in place of the 28-byte JUNK), `fmt ` (tag 1, 24-bit, blockAlign and
byteRate consistent), `data` (size field 0xFFFFFFFF resolved only through ds64), `axml`, `chna`,
`dbmd`, with correct pad bytes; the `data` and RIFF sizes exceed u32 exactly where they must and
both promotion thresholds are handled. oadec writes the EBU Tech 3306 `RF64` FourCC rather than
BS.2088's `BW64`; BS.2088 was not obtainable, every reader used accepts the file, and the point is
recorded as INFERRED. Negative controls M-A8 (partial frame) and M-A12 (truncated inside axml)
fired. **Result: PASS (MEASURED)**, with the FourCC question open.

---

## 13. TrueHD-specific results

11 inputs on this path (three Pi variants, five film heads, three authored scenes).

- Ledger: 32 945 matched, 0 defects, 32 879 ramp losses, 73 duplicate blocks and 59 trailing pops
  (ramp-only changes the profile cannot express), 22 bed events (static LFE), identity holds.
- PCM: 240 of 240 pairs identical. Timecodes: 66 036, 0 re-encode mismatches.
- Raw OAMD cross-check on six clips: every DAMF state and ADM block on an OAMD update time,
  positions within the dump's precision, +32 term intact.
- TrueHD head cuts end in a truncated access unit; oadec exits 7, writes the file and says the
  output "is not trustworthy" in the stderr summary. The ADM written in that case is complete and
  consistent with the DAMF written from the same run.
- The TrueHD driver hardcodes the ISF count to 0 (`cli/damf.rs:86`); no TrueHD stream in the
  library carries ISF, so this is N/T on real material and a known asymmetry.
- Whole film: 6 058 052 access units, 157 763 OAMD payloads, 30 589 events, 0 restating payloads,
  0 out-of-order events, 0 payload errors; ADM in 190.9 s, DAMF in 184.9 s.

---

## 14. E-AC-3 JOC-specific results

14 inputs on this path (nine film heads, five authored scenes).

- Ledger: 3 489 matched, 0 defects, 3 279 ramp losses, 0 duplicate blocks, 0 pops, 14 bed events,
  identity holds. JOC programmes never produced a ramp-only change in this corpus.
- PCM: 350 of 350 pairs identical between DAMF and ADM.
- Programme length is the E-AC-3 frame grid minus the 256-sample decoder delay (480 512 frames for
  the 10 s authored scenes); the last block of every object ends exactly there.
- The greenbook `cfg4` head is a downmix configuration, not a 5.1.2 bed: OAMD declares one LFE bed
  and 15 objects, and DAMF and ADM agree.
- What this proves: the ADM carries the **reconstructed** JOC object signals and the OAMD metadata
  exactly as decoded. It does not prove the reconstruction equals the authoring master; that layer
  (35-41 dB against Dolby's decoder) belongs to the previous audits. No same-programme TrueHD/JOC
  cross-comparison was made in this audit; the two paths were tested separately, as required, and
  neither showed a failure the other did not.

---

## 15. Renderer interoperability

**Dolby validators** (`adm-dolby-reference.json`, `validate`): `atmos_info` 5.7.2 `--validate 1`
and 1.1 refuse every untagged oadec file with "Content was not authored with Dolby tools" and
accept the same file written with `--dolby-origin-tag` (exit 0, no warnings, both versions); they
also accept the real-ramp variant. `bwf_info` accepts all of them and reports the right frame
count. The Conversion Tool converts every oadec ADM back to DAMF with no findings, 128 of the 130
DAMF states returned (the two missing are the bed events), positions within 5e-8, and
`rampLength 0` throughout. DEE's `convert_atmos_mezz` warns only about the absent `.atmos.dbmd`
side file and range conversion; neither warning concerns the content.

**EBU ADM Renderer 2.1.0.** EAR refuses Dolby-profile files, oadec's and Dolby's alike, because the
profile requires `audioStreamFormat` to reference both a channel format and a pack format while
BS.2076 allows one. With the pack reference removed identically from every input (22 references in
pi-head50m, 26 in fast-thd) all renders succeed (`adm-render-comparison.json`):

| Scene | Layouts | oadec vs Dolby CT | oadec (250-sample ramp) vs real-ramp variant |
|---|---|---|---|
| pi-head50m (1:45) | 0+2+0, 0+5+0, 4+7+0 | sample-identical, lag 0 | min SDR 63.0 / 47.0 / 36.5 dB |
| fast-thd (10 s authored) | 0+2+0, 0+5+0, 4+5+0, 4+7+0 | sample-identical, lag 0 | min SDR 17.4 / 8.8 / 8.8 / 6.9 dB, lags 218-982 samples |

Renderer equality is supporting evidence: EAR reads exactly the fields the profile keeps, so it
cannot see gain or importance that were never written. **Result: PASS-TOL (REFERENCE)**; Dolby
Atmos Renderer N/T.

---

## 16. Controlled ground-truth tests

The authored scenes from the previous audit (`atmos-author` DAMF -> DEE -> TrueHD `.mlp` and
E-AC-3 JOC `.ec3`) were decoded to DAMF and ADM on both paths:

| Scene | What it fixes | Result through ADM |
|---|---|---|
| scene (static positions) | seven known positions | positions exact at float32 in DAMF and ADM on both codecs |
| gainsize (gains 0/-3/-6/-12/-24 dB, sizes 0.25/0.5/1.0) | authored gain and size | DEE already applies gain into the object PCM and drops it from OAMD, and renders size; the decoded DAMF carries 0 dB and size 0, so the ADM cannot be tested against the authored gain through an encoder. The writer-level test is harness C02/C04 and the Dolby reference S2. |
| fast (continuous motion, 1536-sample ramps) | trajectory | ADM blocks at the exact event samples; the only deviation is the 250-sample ramp (section 10) |
| onset-768, teleport-768 (JOC) | event timing against an impulse | DAMF and ADM identical; absolute alignment inherited from the remediation report |

New for this audit, the Dolby reference stimuli S0-S13 (edited DAMF text through the Conversion
Tool and DEE) establish what Dolby's own converters do with ramps, gains, importance, inactive
objects, late first events, zones/snap, bed gain, screen factor, fps 23.976, a trailing ramp-only
event and two events at one sample position. Ground truth here is the DAMF text itself, not an
authoring master.

---

## 17. Negative controls

22 defects were injected into copies of the pi-head50m ADM/DAMF pair before the real files were
judged (`adm-negative-controls.json`); each names the detector expected to fire, and all 22 fired
with no specificity violation. Positive control P-0 (the unmutated pair) reports 0 defects and
590 identical tracks.

| Control | Injected | Detector that fired |
|---|---|---|
| M-A1 | rtime +1 sample | ledger time_mismatch |
| M-A2 / M-A2t | X += 0.001; same file at tolerance 0.01 | value_mismatch{pos} fires / does not fire |
| M-A3 | block lists of two objects swapped | value_mismatch on both, structure unchanged |
| M-A4 | PCM of tracks 11 and 13 swapped | identical pairs -2, pairing matrix not as declared |
| M-A5 | interior block deleted | tiling gap, unexplained_missing |
| M-A6 | interpolationLength := 1536 | ramp loss -1, trajectory statistics change, profile finding |
| M-A7 | chna entry to a non-existent track format | chna-dangling |
| M-A8 | data chunk 3 bytes short | riff data-partial-frame |
| M-A9a / M-A9b / M-A9bs | Z removed; gain 1.0 added on an active block | value_mismatch{pos}; no new effective defect; profile-gain-on-active |
| M-A11 | rtime +10 µs (< half a sample) | re-encode mismatch while the sample still matches |
| M-A12 | file truncated inside axml | ContainerError |
| M-A13 | UID sampleRate 44100 | uid-sample-rate |
| M-A14 | +1 LSB at sample 1000 | one pair not identical, first_diff 1000 |
| M-D1 / M-D2 / M-D3 | DAMF samplePos deleted; first event without pos; sampleRate 44100 | damf-no-samplepos; damf-first-event-incomplete; damf-rate |
| M-T1 | synthetic ADM carrying the real ramps | evaluator returns exactly 0, non-zero on the written file |
| M-A10 | inactive importance 0 -> 5 | N/T on this file (no inactive block); covered by harness C16 |

The brief's "shift by 1536 samples", "replace all ramps by 250", "remove gains/importance",
"invert X", "exchange X and Y" and "duplicate a TrackUID" are strict supersets of M-A1, M-A6,
M-A9b, M-A2, M-A3 and M-A7 with the same detectors, and the mutator implements them; the smaller
perturbations were run because a detector that catches +1 sample catches +1536.

---

## 18. Coverage gaps

- **ISF.** No stream in the library carries ISF; the TrueHD driver hardcodes 0 and the JOC driver
  maps ISF slots to nothing. Harness C13 shows four ISF objects vanish from DAMF and ADM without a
  diagnostic. N/T on real material; FAIL at the writer level.
- **Multi-channel beds.** Every corpus bed is LFE-only. Bed labels, positions and order were
  verified with the harness (C21, C14); a 5.1 bed's `Ls/Rs` would be labelled as side surrounds
  (`RC_Lss/RC_Rss`, y = 0) where the profile's 5.1 set uses `RC_Ls/RC_Rs` (y = -0.36397): untested,
  INFERRED.
- **Gain, importance, size, distance, divergence, warp mode, trim** never occur in the corpus;
  tested only at the writer level and through Dolby's converters.
- **96 kHz Atmos** does not exist in the library; harness C15 only. The 44.1 kHz Pi encode was not
  decoded in this audit.
- **Dolby Atmos Renderer** is not installed; DRP cannot open ADM/DAMF. Renderer evidence is EAR
  plus Dolby's converters and validators.
- **BS.2088** and **profile v1.1** were not obtainable.
- **Whole-film JOC** (>4 GiB on the JOC path) was not generated; the RF64 logic is codec-agnostic
  and was exercised by the TrueHD whole film and the two harness threshold files.
- **`--presentation`** for DAMF/ADM was not re-measured (prior audit: silently forced to 3).
- **dbmd** content is opaque; only Dolby's acceptance of the tagged file is known.

---

## 19. Defects by severity

No P0. Nothing corrupts or misrepresents the scene of any decoded stream.

| # | Priority | Defect | Origin | User-visible consequence | Evidence |
|---|---|---|---|---|---|
| D1 | P1 | Object gain of active objects is not written (`adm.rs:488-492`) | writer; Dolby's converters write it | a muted or attenuated object plays at full level in an ADM consumer; not triggered by any corpus stream (all 0 dB) | C02, S2 |
| D2 | P1 | ISF objects dropped from DAMF and ADM, audio and metadata, without a diagnostic (`Slot::None`; TrueHD count hardcoded 0) | model / drivers | whole audio elements disappear from the export; no corpus material | C13 |
| D3 | P1 | No diagnostic for any lossy path: ramp, gain, importance, 3-D size, bed events, ISF, non-48 kHz, out-of-order input; exit 0 | product | the user cannot know what the ADM lost; the brief's "silent lossy conversion is a product defect" applies | all harness cases, batch |
| D4 | P2 | Fixed 250-sample interpolation length (`adm.rs:24`) | **profile table 11; Dolby-identical** | object motion in the ADM differs from the OAMD trajectory within each block: 36.5-63 dB below signal on film, 6.9-17.4 dB on a fast scene; DAW import with >30 ms event spacing shows short ramp then hold instead of a 32 ms glide | section 10 |
| D5 | P2 | 3-D size collapsed to the first axis in the programme model (`program.rs:242`) | model; DAMF and profile cannot carry three axes | an object authored 0.2 wide and 0.8 high is exported 0.2 in every axis; axis choice arbitrary; no corpus material | C04 |
| D6 | P2 | Non-48 kHz programmes written against the profile's sampleRate rule without warning; 250-sample constant scaled by rate | writer | a 44.1 kHz Atmos programme yields a profile-non-conformant ADM whose acceptance by Dolby tools is unknown | C15 |
| D7 | P2 | Object IDs below `AO_100b` with `--no-bed-conform` and fewer than ten bed channels | writer (`0x1001 + bed + k - 1`) | profile violation (table 17); Dolby numbers objects from `AO_100b` regardless; semantic content unaffected | C11, C12, C21b, S11 |
| D8 | P3 | Block duration computed from an event beyond the programme end: block overruns the end (`adm.rs:477-487`) | writer | invalid tiling on a hypothetical input; never observed (all real programmes end at `frames`) | C07 |
| D9 | P3 | Out-of-order events produce overlapping blocks and a skipped state | writer never sorts | invalid tiling; 0 out-of-order events on every decoded stream | C09 |
| D10 | P3 | Late first event: first state held from 0, arrival time of the first state lost (Dolby writes a default (0,0,0) block instead) | writer (`adm.rs:445-466`) | an object that should appear at 2 s is placed from 0 s; not observed on real material | C06, S6 |
| D11 | P3 | Importance and time-varying bed gain not carried | **profile**, Dolby-identical | priority-based renderer decisions and bed automation are lost; corpus values are the defaults | C03, C05, S3, S8 |
| D12 | P3 | Distance, divergence, warp mode and trim parsed and discarded | model; not representable in DAMF or the profile | none on the corpus (`unspecified` only) | C19, code |
| D13 | P3 | `--presentation` silently ignored for DAMF/ADM; DAMF `fps` fixed at 24 | CLI / DAMF header | cosmetic for ADM, whose timing is sample-based (proved by 0 drift); Dolby's tool accepts fps 23.976 | prior audit, S10 |
| D14 | P3 | `RF64` FourCC rather than BS.2088 `BW64`; `bed_mask_bit` collision for Lw/Rw/LFE2 (unreachable: those channels are refused) | writer | none observed; every reader accepts RF64 | container, code |

---

## 20. Conformance matrix

`docs/audit/adm-conformance-matrix.md` (35 rows, generated by `build_matrix.py` from the
evidence; also `adm-conformance-matrix.json`). Structural and semantic results are separate
columns. Summary of the required rows:

| Feature | Structural | Semantic | Class |
|---|---|---|---|
| Object PCM, bed PCM/LFE, channel assignment, object identity | PASS | PASS | MEASURED |
| Object count and classification | PASS | PARTIAL (ISF) | MEASURED |
| X, Y, Z | PASS | PASS | MEASURED |
| Distance, divergence | PASS | N/T | IMPLEMENTATION CHOICE / SPEC |
| Gain | PASS | **FAIL** | MEASURED + REFERENCE |
| Size (scalar) | PASS | PASS | MEASURED |
| Width / height / depth | PASS | PARTIAL | MEASURED |
| Importance | PASS | PASS-TOL | MEASURED + REFERENCE |
| Interpolation mode | PASS | PASS | SPEC + MEASURED |
| Interpolation length | PASS | PASS-TOL | SPEC + REFERENCE + MEASURED |
| Event timing, block offset, duration | PASS | PASS / PASS / PASS-TOL | MEASURED |
| Start time | PASS | PASS-TOL | MEASURED + REFERENCE |
| ISF | PASS | **FAIL** (N/T on real material) | MEASURED |
| Bed configuration, bed events | PASS | PASS-TOL | MEASURED + REFERENCE |
| Programme hierarchy, TrackUID/chna, axml, BW64 structure | PASS | PASS | MEASURED |
| Object IDs with `--no-bed-conform`; sample rate | **FAIL** | PASS | MEASURED |
| dbmd | UNK | N/T | UNKNOWN / PROPRIETARY |
| Diagnostics | PASS | **FAIL** | MEASURED |

---

## 21. Final scores

| Score | Value | Basis |
|---|---:|---|
| PCM fidelity | 100 | 590/590 track pairs sample-identical, whole film included |
| Object mapping | 100 | chna chain and pairing matrix confirmed on every input; identity stable over 1:24:08 |
| Bed mapping | 90 | LFE beds exact; multi-channel beds proven only in the harness; 5.1 `Ls/Rs` labelling untested |
| Coordinate fidelity | 100 | 0 mismatches at float32; raw-OAMD mapping within 9.7e-5 |
| Gain fidelity | 40 | real material unaffected; the writer drops gains and mutes that Dolby keeps |
| Size fidelity | 70 | uniform size exact; 3-D size collapsed to the first axis; corpus never carries size |
| Timing fidelity | 100 | sample-exact against DAMF and raw OAMD including +32; 0 drift |
| Interpolation fidelity | 50 | mode right, length fixed by the profile and identical to Dolby; rendered loss 6.9-63 dB SDR |
| ADM structural validity | 95 | 0 findings on every default file; two options leave the profile silently |
| ADM semantic fidelity | 80 | everything the profile can carry is carried exactly; the profile's losses are silent; gain and ISF are lost avoidably |
| Renderer interoperability | 85 | Dolby validators accept (tagged); EAR bit-identical to Dolby after the profile-conflict edit; no Dolby Atmos Renderer |
| **Overall ADM export** | **82** | a faithful Dolby-profile ADM of the decoded scene, equal to Dolby's own conversion, with undeclared profile losses and three writer-level defects on inputs the corpus never produces |

The scores are not averages; a missing critical semantic (gain) is scored on its own row and pulls
the semantic and overall rows down regardless of the many exact rows.

---

## 22. Production-readiness verdict

**Structural: MOSTLY VALID.** **Semantic: MINOR SEMANTIC LOSS** for the decoded corpus.

The ADM export can be used today for what it has been used for: handing a decoded TrueHD Atmos
or JOC programme to DEE or another Dolby-profile consumer, where it behaves exactly like Dolby's
own DAMF-to-ADM conversion. It is **not** yet production-ready as a general Atmos interchange
writer, for three reasons that are oadec's to fix: it drops object gain that Dolby keeps, it drops
ISF elements, and it says nothing when it loses anything. Until D1-D3 are addressed, every ADM
should be understood as "the Dolby master profile's view of the scene, minus gain, minus ISF",
and the tool should say so.

---

## 23. Prioritised remediation backlog

Recommendations only; nothing was changed during the audit. Warnings versus failures follow the
rule that a file must be correct or the tool must say why it is not.

1. **Write `<gain>` on active objects** when the gain is not unity (`adm.rs:488-492`), as Dolby's
   converters do; keep the inactive marker as is. Test: harness C02 must reconcile with 0 gain
   losses and the Conversion Tool must return -6/+3/-inf/-12 dB.
2. **Emit a diagnostic for every lossy path** and consider a non-zero exit class for "output written
   with declared semantic loss": ramp length replaced (count and source values), gain/importance
   omitted, 3-D size collapsed (axes), bed events dropped (count), ISF dropped (count, this one
   should be an error), non-48 kHz programme, out-of-order events, events beyond the end.
   `oadec-spatial` needs a logging path first.
3. **Stop dropping ISF**: either represent ISF elements (as DirectSpeakers or as objects with a
   documented mapping) or fail explicitly; remove the hardcoded 0 in the TrueHD driver
   (`cli/damf.rs:86`).
4. **Carry all three size axes to the programme model** (`program.rs:242`) and choose the ADM
   value deliberately (the profile requires three identical values; document the reduction and
   warn when the axes differ).
5. **Refuse or warn on non-48 kHz programmes** for `--format adm` (`adm.rs:576-578`); the profile
   allows only 48 000.
6. **Number objects from `AO_100b` regardless of bed size** (table 17), matching Dolby.
7. **Clamp the last block to the programme end** and drop or warn about events at or beyond
   `frames` (`adm.rs:477-487`); **sort or reject out-of-order events** before writing.
8. **Late first event**: either write Dolby's default block at 0 followed by the real event, or
   document the hold-from-zero behaviour; today the arrival time is lost.
9. **Offer a real-ramp mode** (`interpolationLength = rampLength`) as an explicit, non-default
   option for consumers that honour BS.2076 interpolation, clearly labelled as outside the Dolby
   profile; the M-A6 variant passes Dolby's validators.
10. **Honour `--presentation` for DAMF/ADM or reject the flag**; write the DAMF `fps` from the
    stream or the user rather than a constant.
11. **Document the `RF64` choice** or write `BW64` per BS.2088 once the text is available; fix the
    `bed_mask_bit` collision even though the channels are currently unreachable.
12. **Run `--format adm` in CI** on at least one clip with the toolkit's ledger, PCM and
    container checks as the regression gate; today no test exercises the ADM writer end to end.

---

## Appendix A. The twenty-seven questions

1. **Does ADM contain the exact object PCM recovered by the decoder?** Yes. 590 of 590 track pairs
   are sample-identical, including the whole film.
2. **Are beds and objects mapped to the correct PCM tracks?** Yes. Declaration-derived chains
   confirmed by the pairing matrix on every file; harness C21 confirms label order with tones.
3. **Does every Atmos object retain its identity?** Yes. Gapless, sorted block tiling per object
   and identical PCM under each object's channel format from first to last sample over 1:24:08.
4. **Are X/Y/Z coordinates preserved correctly?** Yes. 0 mismatches at float32 over 36 434 events;
   the OAMD-to-room mapping verified against the raw dump.
5. **Is distance preserved?** No, and it cannot be: parsed, not stored, no field in DAMF, forbidden
   by the profile. The corpus never specifies it. N/T.
6. **Is object gain preserved?** No. Active-object gain is omitted; Dolby's converters write it.
   Real material is unaffected because every gain is 0 dB. FAIL.
7. **Is object priority/importance preserved?** No, by profile rule, identically to Dolby; the
   absent value equals the corpus value. PASS-TOL.
8. **Is object size preserved?** A uniform size, yes, exactly.
9. **Are width, height and depth individually preserved where present?** No. The programme model
   keeps the first axis; DAMF and the profile cannot carry three. PARTIAL.
10. **Is divergence preserved?** No; forbidden by the profile, absent from DAMF, absent from the
    corpus. N/T.
11. **Are metadata event sample positions preserved?** Yes, exactly, on all 36 434 events and on
    the raw OAMD times.
12. **Is the `32 x block_offset_factor` term preserved?** Yes. 80 ADM blocks sit on times that
    exist only with the term; none is missing or moved.
13. **Are interpolation mode and duration preserved?** Mode yes; duration no: fixed 0 then 250
    samples by profile rule.
14. **Does ADM introduce a fixed ramp duration?** Yes, 250 samples (0.005208 s), as the profile
    mandates and as Dolby writes; measured consequence in section 10.
15. **Are any OAMD fields parsed but silently discarded before ADM?** Yes: distance, divergence,
    warp mode, trim configuration, second and third size axes, ISF elements; ramp length, gain,
    importance and bed events are carried to DAMF and discarded by the ADM stage.
16. **Does DAMF preserve information that ADM loses?** Yes: ramp length, active-object gain,
    importance, bed events, screen factor. DAMF itself loses distance, divergence and 3-D size.
17. **Are CHNA, TrackUID and AXML relationships correct?** Yes on every file; FAIL only on the
    object-ID range with `--no-bed-conform`.
18. **Is ADM PCM channel order correct?** Yes: beds first in profile order, objects in element
    order, confirmed by PCM identity and tones.
19. **Does the ADM file remain semantically correct on long programmes?** Yes. Whole film: 0
    ledger defects, 0 drift, RF64/ds64 consistent, all readers agree on 242 322 080 frames.
20. **Does Dolby Atmos Renderer import it without warnings?** Not testable (not installed).
    `atmos_info` 5.7.2/1.1 accept the tagged file without warnings and refuse the untagged one on
    provenance alone; the Conversion Tool imports it with no findings.
21. **Does renderer import actually exercise all metadata fields?** No. EAR and Dolby's readers
    consume what the profile keeps; Dolby's reader returns rampLength 0 for every interpolation
    length, so import proves nothing about ramps, gain or importance.
22. **Does ADM render equivalently to DAMF?** Equivalently to Dolby's ADM of the same DAMF, yes
    (sample-identical). Equivalently to the DAMF's own ramps, no: 6.9-63 dB SDR depending on how
    fast the scene moves. No tool renders DAMF directly here.
23. **Is TrueHD Atmos to ADM faithful?** Faithful to the decoded scene within the profile:
    PCM, positions, timing, identity exact; ramp length lost; ISF count hardcoded.
24. **Is E-AC-3 JOC to ADM faithful?** Same answer for the decoded (reconstructed) scene; the
    reconstruction-versus-master layer is outside this audit.
25. **What exact metadata is currently impossible to recover or validate?** dbmd content (opaque);
    the BW64 FourCC question (no BS.2088); 5.1-bed labelling, ISF, non-zero gain/size/importance,
    distance and divergence on real streams (no material); Dolby Atmos Renderer behaviour.
26. **Which claims remain differential rather than ground-truth?** All PCM claims (DAMF versus ADM,
    both from oadec); JOC object signals versus the master; the 256-sample delay; the timing
    claims are ground-truth against the raw OAMD, and the static-position claims against authored
    scenes.
27. **What must be fixed before ADM output can be called production-ready?** Backlog items 1-3
    (gain, diagnostics, ISF); items 4-7 before the `--no-bed-conform` and non-48 kHz paths can be
    offered.

## Appendix B. Reclassification of the previous audit's ADM rows

The 2026-09-10 audit marked the 250-sample ramp, the missing gain/importance, the missing bed
events and the 3-D size as FAIL from code reading. With the profile text (SPEC) and Dolby's own
converters (REFERENCE) in hand: the ramp, importance and bed-event rows are profile-mandated and
Dolby-identical and become PASS-TOL with a measured loss; the 3-D size row is PARTIAL because the
collapse happens before both writers and neither target can carry three axes; the gain row stays
FAIL because Dolby carries gain and oadec does not. The silence about all of them is a new,
separate defect (D3).
