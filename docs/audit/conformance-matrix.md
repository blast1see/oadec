# Conformance matrix

Outcomes: **PASS** demonstrated correct · **PASS-TOL** numerically different but
technically justified · **PARTIAL** part of the required function · **FAIL**
demonstrated incorrect · **N/I** not implemented · **N/T** not testable with the
available reference · **UNK** unknown or proprietary.

Evidence paths are relative to `docs/audit/`. Source anchors are
`crate/src/file.rs:line` under `crates/`.

> **Revised after remediation.** Rows the remediation branch changed carry the
> audit's original verdict in brackets, so the before-state stays readable
> without going back through git. The report is
> `decoder-remediation-report.md`; the measurements are under
> `evidence/remediation/`. Nothing moved without one. Line numbers in the
> Implementation column refer to the code as the audit found it and are left
> alone; where a row's code has moved, the new location is named in the text.

## TrueHD

| Feature | Requirement | Implementation | Evidence | Result |
|---|---|---|---|---|
| Access-unit framing, check nibble, substream directory | Dolby high-level description | `oadec-truehd/src/au.rs:75`, `:165` | 6 films clean in the media suite; 220-trial fuzz | PASS |
| Major sync CRC-16 | Dolby | `oadec-truehd/src/sync.rs:181` | negative control: flipped bit gives a CRC failure, exit 7 | PASS |
| Access-unit header parity | Dolby | `oadec-truehd/src/au.rs:194` | negative control as above | PASS |
| Substream parity and CRC-8 | Dolby | `oadec-truehd/src/segment.rs:143`, `:144` | flipped bit at offset 6 000 000 gives 1 parity + 1 CRC failure | PASS |
| Restart header and its CRC-8 | Dolby | `oadec-truehd/src/restart.rs:57`, `:152` | fatal on mismatch; exercised by fuzz | PASS |
| Lossless reconstruction, presentation 2 | lossless by definition | `oadec-truehd/src/decoder.rs` | 40 606 400 samples against FFmpeg, 0 differing | PASS |
| Lossless reconstruction, presentation 3 | lossless by definition | same | 60 909 600 samples against `truehdd`, 0 differing | PASS |
| Lossless check word | Dolby | `oadec-truehd/src/decoder.rs:337` | corruption in substreams 0, 1, 2 and 3 all caught at `-p 3` | PASS |
| `verify` performs a lossless check | — | `oadec-cli/src/scan.rs:411` | `verify --json` exposes no lossless statistic; parity and CRC catch everything tested | PARTIAL |
| Seamless branch and duplicate access units | Dolby | `oadec-truehd/src/timing.rs:64`, `:240` | Braveheart and the synthetic splice clip clean in the media suite | PASS |
| Sampling-rate change at a major sync | Dolby | `oadec-truehd/src/decoder.rs:641` | only the samples-per-access-unit count is compared, so 48 to 44,1 kHz passes and the WAV header keeps the first rate | PARTIAL |
| Mid-stream layout change | Dolby | `oadec-truehd/src/decoder.rs:641` | explicit hard error, "not supported yet" | N/I |
| `block_header_crc` | not public | `oadec-truehd/src/block.rs:255` | recorded, cannot be checked | UNK |
| 24-bit output, sign extension | — | `oadec-cli/src/decode.rs:271` | range over 40.6 M samples −8 217 040 to 7 877 136, none at full scale | PASS |
| 24-bit output, saturation | — | `oadec-cli/src/decode.rs:273` | truncates where `caf.rs:75` and `adm.rs:243` clamp; not reached by this material | PARTIAL |
| WAVE channel mask | WAVE | `oadec-cli/src/decode.rs:152` | indices above 31 are dropped, so `Tsl`/`Tsr` at 36/37 never appear in the mask | FAIL |
| Channel order, interchange | WAVE and FFmpeg order | `oadec-truehd/src/channel.rs:431` | positional match with FFmpeg at zero difference | PASS |
| DRC and dialogue normalisation | A/52-style application | not applied anywhere | every field parsed and discarded; documented in the README | N/I (by design) |

## TrueHD Atmos

| Feature | Requirement | Implementation | Evidence | Result |
|---|---|---|---|---|
| Atmos detection from the bitstream | Dolby | `sync.rs:167`, `restart.rs:72`, `sync.rs:24` | three independent structural signals; Cloud Atlas correctly rejected | PASS |
| Non-Atmos negative control | — | `cli/damf.rs:61` | Cloud Atlas: 3 substreams, no extra channel meaning, object path refuses | PASS |
| Evolution / EMDF payload extraction | Dolby, TS 102 366 Annex H | `truehd/extra.rs:98`, `emdf/container.rs:257` | 1 266 to 3 306 payloads per clip, 0 parse errors | PASS |
| Object audio reconstruction | the 16 channels are the objects | `truehd/decoder.rs` | 60 909 600 samples against Dolby `dlbtruehddec presentation=16`, 0 differing | PASS |
| Bed and object separation | TS 103 420 clause 5 | `truehd/channel.rs:296`, `spatial/program.rs` | LFE bed plus 11 or 15 dynamic objects, matching Dolby's channel assignment | PASS |
| ISF objects | TS 103 420 | `cli/damf.rs:74` | hardcoded to zero; the E-AC-3 path at `eac3_objects.rs:173` handles it | FAIL |
| `--presentation` honoured for object output | — | `cli/damf.rs:193` | silently forced to presentation 3 | FAIL |
| Titles Dolby will confirm | — | — | 3 of 6 accepted. The refusal is specific to the object output — all six decode at `presentation=16` when `out-ch-config` is left alone — and `2ch_control_enabled` is necessary but not sufficient: clearing it turns an accepted title into a refused one, setting it leaves a refused one refused; `evidence/remediation/presentation16-differential.json` | N/T |

## E-AC-3

| Feature | Requirement | Implementation | Evidence | Result |
|---|---|---|---|---|
| Syncframe and bit stream information | TS 102 366 Annex E | `eac3/header.rs:92`, `eac3/bsi.rs` | 17-stream corpus, 0 sync errors | PASS |
| Frame CRC | TS 102 366 clause 6.10.1 | `eac3/frame.rs:288` | flipped bit gives 1 CRC failure in `verify` | PASS |
| CRC failure affects the exit code | — | `cli/integrity.rs`, used by every delivery path | every path now decides with the list `verify` uses; 29 of 33 corpus streams moved from exit 0 to exit 7, each one already non-conformant to `verify`; `evidence/remediation/decode-exit-codes.json` | PASS [was PARTIAL] |
| Independent substream decode | Annex E | `cli/eac3.rs:465` | three-way comparison, closer to Dolby than FFmpeg on every channel | PASS-TOL |
| **Dependent substream decode** | Annex E clause E.2.8.2 | `eac3/program.rs`, `ProgramDecoder` | 40 of 40 eight-channel library tracks decode to 8 channels, FFmpeg agreeing on the count for every one; 320 channel comparisons, 0 mismatched; `evidence/remediation/ddp71-channel-compare.json` | **PASS** [was FAIL] |
| Custom channel map | Annex E clause E.1.3.1.8, table E.1.4 | `eac3/program.rs`, `chanmap_locations` | two maps found in the wild: 0x1a00 (Ls, Rs, Lrs/Rrs → 7.1) on 35 titles and 0xa010 (L, R, Vhl/Vhr → 5.1.2) on 5, both agreeing with FFmpeg's layout | PASS [was N/I] |
| Coupling and default band structure | Annex E, Table E.1.12 | `eac3/frame.rs:880` | absolute subband indexing; stereo corpus decodes | PASS |
| Enhanced coupling, amplitude only | TS 102 366 V1.4.1 | `eac3/frame.rs:1352` | default; injected material decodes, media test passes | PASS |
| Enhanced coupling, angle and chaos | A/52:2018 clause E.3.5.5 | `eac3/ecpl.rs:94`, behind `--ecpl-spec` | no decoder anywhere implements it, so there is no oracle | N/T |
| Spectral extension | Annex E | `eac3/frame.rs:1928` | 17-22 kHz energy 0,0 dB against two Dolby decodes in prior work; media test asserts coverage | PASS-TOL |
| Adaptive hybrid transform | Annex E | `eac3/frame.rs:1769` | media test asserts coverage on the 384 kbit/s clip | PASS-TOL |
| Transient pre-noise processing | Annex E clause E.2.7.2 | `eac3/tpnp.rs:143` | media test bounds the corrected region; cross-fade shape is an implementation choice the clause leaves open | PASS-TOL |
| Delta bit allocation | Annex E | `eac3/frame.rs:1238`, `bitalloc.rs:221` | present in the library sweep but absent from the corpus and from the required-coverage assertion | N/T |
| Dither | A/52 clause 7.3.4 | `eac3/frame.rs:99`, scale 0,5 | `--no-dither` moves oadec ~3 dB closer to Dolby; LFE agrees at 118,8 dB with no dither at all | PASS-TOL |
| Dither sequence | implementation-defined | — | deliberately not recovered | UNK |
| Decoder delay | — | `cli/eac3_objects.rs:33` | Dolby leads by exactly 256 samples, measured | PASS |
| Float output sanity | — | `cli/eac3.rs:895` | 30,5 M samples: no NaN, no infinity, no denormals, nothing above unity | PASS |
| Frame ending inside its own tail | out of spec | `eac3/frame.rs:2103` | decoded and counted separately; `verify` still non-conformant | PASS |
| DRC, `compr`, dialogue normalisation, downmix | A/52 | not applied | documented at `oadec-eac3/src/lib.rs:9` | N/I (by design) |

## EMDF, OAMD, JOC

| Feature | Requirement | Implementation | Evidence | Result |
|---|---|---|---|---|
| EMDF container parse | TS 102 366 Annex H | `emdf/container.rs:190` | container counts agree between `emdf` and `verify` on four clips | PASS |
| EMDF located in the skip field | Annex H clause H.1 | `eac3/frame.rs:1258` | exact, not a byte scan | PASS |
| EMDF in auxiliary data | Annex H clause H.1, clause 4.4.4 | `eac3/frame.rs`, `read_auxdata` | read structurally at fixed offsets, never scanned for; 0 frames carrying auxiliary user bits across the corpus, so the carriage itself is unexercised | N/T [was N/I] |
| EMDF in the last dependent substream | TS 103 420 clause 8.2 | `eac3/program.rs`, `ProgramFrame::metadata_part` | exercised at last, on the configuration 4 streams, which are the first material with both a dependent substream and object metadata: 18 750 dependent frames and 18 750 containers each, and **none** in the independent substream. The encoder agrees with the clause. It also shows what the audit predicted: a decoder that filters dependents out finds no metadata at all in these streams, so their objects were not merely incomplete but absent; `evidence/remediation/library-syntax-scan.json` | PASS [was N/T] |
| EMDF protection words | Annex H clause H.2.2.4 | `emdf/container.rs:231` | read and stored. They cannot be verified: clause H.2.2.4.3 says "calculation of the value of the protection_bits_primary field is implementation dependent and is not defined in the present document", and H.2.2.4.4 says the same of the secondary word. A decoder that does compute them is now visible: Dolby discards any JOC payload rewritten in place and holds the previous matrix, which is what a failed check looks like from outside; `evidence/remediation/payload-rewrite-rejected.json` | UNK [was N/I] |
| Payload configuration constraints | TS 103 420 Table 56 | `emdf/container.rs:160` | all nine fields parsed, none validated | PARTIAL |
| Payload dispatch, 11 and 14 | TS 103 420 Table 55 | `emdf/container.rs:28`, `:31` | discrimination matrix, six inputs, all correct | PASS |
| `addbsi` Atmos declaration | TS 103 420 clause 8.3 | `eac3/bsi.rs:55` | flag true and complexity index 16 on JOC streams, which is 15 objects plus the LFE bed | PASS |
| Complexity index cross-checked against OAMD | clause 8.3.2.2 | reported only, `cli/eac3.rs:746` | no cross-check exists | PARTIAL |
| JOC Huffman tables | Annex A.1 | `emdf/joc_tables.rs` | 582 node pairs identical to ETSI's `ts_103420_tables.c` | PASS |
| QMF prototype | clause 7.4 | `joc/qmf_window.rs` | 640 values identical to ETSI `prot64` as float64 | PASS |
| Parameter band mapping | Table 54 | `joc/lib.rs:19` | all 23 rows and 8 columns match, including the worked example | PASS |
| Dequantisation | clause 6.6.4 | `joc/lib.rs:67` | formula identical to Pseudocode 5 | PASS |
| Coarse quantisation | clause 6.6.3, Annex A.1 | `emdf/joc.rs`, `COARSE_MTX`/`COARSE_VEC` | 92 550 of 170 722 130 object updates use it. It was first checked on a clip carrying 75 coarse objects, at 50 to 57 dB against Dolby's and unchanged by the sparse correction. The clip that settled sparse mode is 5 325 coarse objects against 675 fine and sits at 46,78 dB where the rest of it is 46,69, so coarse is now held by seventy-one times the material; `evidence/remediation/sparse-settled.json` | PASS-TOL [was N/T] |
| Differential decode, dense | clause 6.6.2 Pseudocode 3 | `emdf/joc.rs:244` | identical to the printed pseudocode | PASS |
| Differential decode, sparse | Pseudocode 2 | `emdf/joc.rs`, `SparseReading::Measured` | three corrections are needed and each hides the next: the unselected channel takes the zero-gain code, the channel index accumulates from the resolved previous index, and the coefficient chain runs unbroken across bands. Settled on a clip carrying **825 sparse objects in 55 frames and no steep object at all**, which separates the two for the first time: the printed reading puts those frames at -3,92 dB median against Dolby where the rest of the clip is 45,96, and all three corrections put them at 46,78 against 46,69 -- the level of the clip. Earlier and narrower material is in `sparse-differential.json`; `evidence/remediation/sparse-settled.json` | PASS-TOL [was N/T] |
| Clip gain value and use | clause 6.3.3.2 | `emdf/joc.rs:152`, `cli/eac3_objects.rs:280` | formula matches; the use is measured, not specified | PASS (value) / INFERRED (use) |
| `joc_num_objects`, reserved configurations | clauses 6.3.2.2, 6.3.2.4 | `emdf/joc.rs:141` | 5 to 7 rejected, bits above 15 rejected | PASS |
| Downmix configuration 3 | Table 47 | `cli/eac3_objects.rs:106` | 13 of 15 clips; objects match Dolby | PASS |
| Downmix configuration 0 | Table 47 | same | two streams carry it. Dolby decodes Dredd's to sixteen objects and ours agree to 52,44 dB worst, 54,38 median, correlation structure identical -- the closest agreement of any title measured here. It refuses Snatch's, which is therefore about that stream and not about the configuration; `evidence/remediation/configuration-0-vs-dolby.json` | PASS-TOL [was N/T] |
| Downmix configuration 4 | Table 47 | `eac3/program.rs`, `ChannelLoc::joc_input` | found in five of the library's 118 JOC-carrying tracks, every one an AC-3 core plus an E-AC-3 dependent substream giving 5.1.2. It was structurally unreachable before the dependent-substream work, which is why it had never been seen. Table 47 ends configurations 2 and 4 in the top front pair and not the rear one; reading it as configuration 1 left the height channels unmapped and the decode refused rather than producing wrong output. Fixed and measured: Green Book worst 50,13 dB and median 51,97 against Dolby's objects, 37,20 and 41,96 in the window where the height pair carries signal, and swapping that pair costs 29 dB; `evidence/remediation/library-syntax-scan.json` | PASS-TOL [was N/T] |
| Downmix configurations 1 and 2 | Table 47 | same | still no material, and now the whole library says so: 0 of the 118 Dolby tracks that carry JOC, across 111 films and 226 tracks read one head clip at a time (`evidence/remediation/library-configurations.json`), and 0 of 170 722 130 object updates in the whole-file scans. Relabelling cannot make any, because they size the matrix for seven channels and a five-channel payload relabelled that way runs out of bits in every frame | N/T (implemented, untested) |
| Temporal interpolation, smooth, 1 data point | clause 6.6.5 | `joc/lib.rs:208` | exercised on every clip; objects match Dolby | PASS |
| Temporal interpolation, steep, 1 data point | clause 6.6.5 | `joc/lib.rs`, `SteepReading::Measured` | 737 503 of 32 493 245 object updates. The switch point as clause 6.6.5 prints it is one time slot late: `joc_offset_ts` is one-based per clause 6.3.4.4 and `ts` is not. Measured against Dolby's object decoder on nine titles, five of which carry steep objects: Glass Onion's worst object 25,24 to **49,93 dB** and its median 47,76 to 65,44; The Jackal's, with 840 steep objects, 26,77 to **35,12 dB**; Shaun of the Dead's frame 581 23,51 to 58,90 dB; the four titles with no steep object are bit-identical either way. The optimum over a +/-3 slot sweep is sharp and unique; `evidence/remediation/steep-offset-differential.json` | PASS-TOL [was PASS] |
| Temporal interpolation, 2 data points | clause 6.6.5 | same | **0 of 170 722 130 object updates in 67 JOC streams, whole files**, so the smooth-2 and steep-2 branches of pseudo-code 6 have never run on real material. Both offsets take the same correction as the one-point branch and neither has been measured | N/T |
| `joc_mix_mtx_prev` zero at stream start | clause 6.6.5 | `joc/lib.rs:96` | zero-initialised | PASS |
| Splice reset on a zero sequence counter | clause 6.3.3.3 | `cli/eac3_objects.rs`, `Pipeline::frame` | a zero sequence counter after the first frame forgets the matrix history, as clause 6.6.5 requires of the first frame. A counter that simply does not follow the previous one is counted and reported and not acted on, because the clause makes only the zero a splice: a stream cut at frame 300 reports one such gap and no splice | PASS [was N/I] |
| Object reconstruction | clause 6.6.6 | `joc/lib.rs:264` | six titles, worst 35,25 to 40,75 dB against Dolby, lag 0 | PASS-TOL |
| Matrix-to-timeslot alignment | clause 6.6.6 pairs `ts` with `ts` | `joc/quadrature.rs:52`, offset 10 | sweep: optimum sharp and symmetric at 10, 11,0 dB better than the literal reading | PASS-TOL (deviation, confirmed against one decoder family) |
| 90-degree phase shift for configurations 3 and 4 | not specified | `joc/quadrature.rs:57` | 37-tap filter fitted to Dolby; objects match on six titles including two new ones | INFERRED |
| LFE bypass | Table 47 note | `cli/eac3_objects.rs:165` | LFE taken from the core, matches Dolby at 89 to 118 dB | PASS |
| Object count and distinctness | clause 6.3.2.4 | `cli/eac3_objects.rs:211` | 15 distinct objects; largest object-to-core correlation 0,545; correlation structure matches Dolby to 0,0017 | PASS |
| OAMD field coverage | clause 5.5 | `emdf/oamd.rs:949` | every syntax element read; 1,3 M payloads with 0 parse errors in prior work, re-confirmed on the clips | PASS |
| OAMD event timing | clause 5.3.2 | `spatial/program.rs:331` | 21 of 132 events differ from `truehdd` by exactly +32 samples, the block-offset term | PASS |
| Object 3-D size | clause 5.6.1.2 | `spatial/program.rs:243` | depth and height dropped; ADM re-emits the first axis as all three | FAIL |
| Object distance, divergence, warp mode, trim decibels | clause 5.2 | parsed at `emdf/oamd.rs:474`, `:828`, `:764`, `:735` | never reach any output | PARTIAL |
| Extended-precision position | clause 5.6.6.4 | `emdf/oamd.rs:865` | parsed and applied | PASS |
| DAMF ramp length | clause 5.3.2 | `spatial/damf.rs:341` | real values 32 and 1536 written | PASS |
| ADM interpolation length | clause 5.3.2 | `spatial/adm.rs:513` | fixed 250 samples written instead of the real ramp | FAIL |
| ADM object gain and priority | clause 5.2.3, 5.2.4 | `spatial/adm.rs:490` | zero gain elements and zero importance elements in the `axml` chunk | FAIL |
| `dbmd` chunk | not public | `spatial/dbmd.rs:18` | three verbatim blobs; only bed mask, channel count and LFE flags derive from the stream | UNK |
| Per-object metadata dump for E-AC-3 | — | `cli/oamd.rs:309` refuses E-AC-3; `cli/emdf.rs:266` truncates to 4 objects | the DAMF sidecar is the only complete record | PARTIAL |

## Robustness

| Feature | Requirement | Implementation | Evidence | Result |
|---|---|---|---|---|
| No panic on arbitrary input | production quality | `oadec-bits` returns errors | 220 bit-flip trials, 0 panics | PASS |
| Corruption never silently accepted, TrueHD | production quality | `cli/decode.rs`, `truehd_findings` | 100 of 100 non-zero exit from `decode`, including the two the audit found silent; the extractor summary that every caller but `verify` used to discard is now read | PASS |
| Corruption never silently accepted, JOC objects | production quality | `cli/eac3_objects.rs`, `cli/integrity.rs` | the same 99 sites replayed: 0 silent, 120 of 120 non-zero exit, 0 panics; `evidence/remediation/decode-exit-codes.json` | **PASS** [was FAIL] |
| Reserved JOC extension configuration | clause 6.3.2.5 | `emdf/joc.rs:263` | rejected after the payload is consumed, then stale matrices are held with an anonymous error count | PARTIAL |
| Seek and random access | stateful codec | no seek API | files are always decoded from byte 0 | N/I |
| Environment-independent decode | reproducibility | `cli/eac3_objects.rs:100`, `:203` | three variables silently alter JOC decoding and appear in no output | FAIL |

---

## Before and after

Every `after` here points at a file under `evidence/remediation/`. The report is
`decoder-remediation-report.md`.

| Area | Before | After | Evidence | Remaining gap |
|---|---|---|---|---|
| E-AC-3 dependent substreams | FAIL | **PASS** | `ddp71-channel-compare.json` | Multiple dependents per programme implemented, unexercised |
| DD+ 7.1 output | FAIL | **PASS** | `dependent-substream-before-after.json` | — |
| Dependent channel map | N/I | **PASS** | `ddp71-channel-compare.json` | Whether the map counts the LFE is a documented reading |
| EMDF in the last dependent substream | N/I, unreachable | **PASS** | `library-syntax-scan.json` | The configuration 4 streams have both: 18 750 dependent frames and 18 750 containers, none of them in the independent substream. The encoder agrees with clause 8.2, and a decoder that filters dependents out finds no metadata at all in these streams |
| EMDF in auxiliary data | N/I | N/T | `library-syntax-scan.json` | 0 frames carry auxiliary user bits in any of the 151 streams read whole: 49 in the working directory, 78 across sixteen library titles and 24 streaming ones |
| JOC configuration 4 | N/T, unreachable | **PASS-TOL** | `library-syntax-scan.json`, `library-configurations.json` | Five library tracks carry it, all 5.1.2 through a dependent substream. Its input mapping was wrong and is fixed |
| JOC configuration 1 / 2 | N/T, unreachable | N/T, reachable | `library-configurations.json` | 0 of the library's 118 JOC-carrying tracks, and 0 of 170 722 130 object updates |
| JOC integrity propagation | FAIL | **PASS** | `decode-exit-codes.json` | — |
| Decode CRC exit status | PARTIAL | **PASS** | `decode-exit-codes.json` | — |
| Corruption never silently accepted, TrueHD | PASS with 2 silent | **PASS** | `decode-exit-codes.json` | — |
| Dolby presentation 16 refusal | N/T, "not causal" | N/T, one cause found | `presentation16-differential.json` | `2ch_control_enabled` is necessary, not sufficient; the second reason is unknown |
| Sparse JOC | N/T | **PASS-TOL** | `sparse-settled.json` | 825 sparse objects in 55 frames with no steep object present sit at the level of the rest of their clip, 46,78 dB against 46,69. The one frame that stayed short is the only one that is also steep |
| Steep interpolation switch point | PASS, unmeasured | **PASS-TOL** | `steep-offset-differential.json` | Confirmed against one decoder family, on five titles that carry steep objects |
| Coarse JOC | N/T | **PASS-TOL** | `sparse-settled.json` | Held by 5 325 coarse objects in one clip, against the 75 it was first checked on |
| Two-point interpolation | N/T | N/T | `library-syntax-scan.json` | 0 of 170 722 130 object updates, the corpus having grown five-fold |
| Controlled object positions | N/T | **PASS** | `controlled-atmos-ground-truth.json` | Movement is the encoder resampling a trajectory onto its own grid |
| Controlled object gain | N/T | **PASS** (encoder drops it) | `controlled-atmos-gain-size.json` | Objects authored at 0, -3, -6, -12 and -24 dB all come back with gain 0 and at the same level, in both codecs, and the OAMD payload of the encoded stream carries gain 0 itself. The decoder reads what is there; nothing in the wild carries a non-zero object gain either |
| Controlled object size | N/T | **PASS** (encoder renders it) | `controlled-atmos-gain-size.json` | Size comes back 0 and the object is spread over 7 to 11 encoded objects against 1 for a point source, total energy within 1,2 dB. Rendered, not carried |
| Controlled object divergence | N/T | N/T, unreachable | `controlled-atmos-gain-size.json` | DAMF has no divergence field, so no authored master can carry one; it needs a stream that already does |
| TrueHD bit exactness | PASS | **PASS** | `regression-summary.json` | — |
| JOC objects against Dolby | PASS-TOL | **PASS-TOL** | `joc-objects-vs-dolby.json` | Six titles, none regressed; Glass Onion's worst object moved 25,24 to 49,93 dB with the steep correction. The worst of the six is 35,25 dB and the difference that remains is E-AC-3 dither |
| Measurement flags reach the object path | FAIL | **PASS** | media tests `the_core_options_reach_the_object_path`, `core_only_is_refused_with_an_object_output` | `--no-dither`, `--no-tpnp` and `--ecpl-spec` were dropped on `decode --format damf` for E-AC-3; `--core-only` is now refused there rather than ignored |
| In-place payload rewriting as an instrument | assumed to work | **FAILS against Dolby** | `payload-rewrite-rejected.json` | Dolby discards a rewritten JOC payload and holds the previous matrix whatever the new value says, so `eac3-joc-config` and `eac3-joc-offset` measure only this decoder. Two conclusions drawn from relabelling are withdrawn |
| JOC configuration 0 | N/T, "Dolby refuses it" | **PASS-TOL** | `configuration-0-vs-dolby.json` | A second stream carrying it is decoded to objects by Dolby and agrees with ours to 52,44 dB. The refusal belongs to the other stream, not to the configuration |
| Why Dolby refuses one configuration 0 stream | "the configuration" | UNK, properly posed | `configuration-0-refusal-pair.json` | A control pair that agrees in every field oadec parses; the sequence counter is ruled out by a mid-file cut |
| Media suite reports honestly | FAIL | **PASS** | CI job `media-suite-refuses-to-pass-without-media` | — |
