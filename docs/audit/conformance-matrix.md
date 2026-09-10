# Conformance matrix

Outcomes: **PASS** demonstrated correct · **PASS-TOL** numerically different but
technically justified · **PARTIAL** part of the required function · **FAIL**
demonstrated incorrect · **N/I** not implemented · **N/T** not testable with the
available reference · **UNK** unknown or proprietary.

Evidence paths are relative to `docs/audit/`. Source anchors are
`crate/src/file.rs:line` under `crates/`.

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
| Titles Dolby will confirm | — | — | 3 of 6 accepted; the other 3 refused for a cause not found | N/T |

## E-AC-3

| Feature | Requirement | Implementation | Evidence | Result |
|---|---|---|---|---|
| Syncframe and bit stream information | TS 102 366 Annex E | `eac3/header.rs:92`, `eac3/bsi.rs` | 17-stream corpus, 0 sync errors | PASS |
| Frame CRC | TS 102 366 clause 6.10.1 | `eac3/frame.rs:288` | flipped bit gives 1 CRC failure in `verify` | PASS |
| CRC failure affects the exit code | — | `cli/eac3.rs:541` | `verify` exits 7; `decode` prints the count and exits 0 | PARTIAL |
| Independent substream decode | Annex E | `cli/eac3.rs:465` | three-way comparison, closer to Dolby than FFmpeg on every channel | PASS-TOL |
| **Dependent substream decode** | Annex E | filtered out at `cli/eac3.rs:465`, `cli/eac3_objects.rs:511` | 7.1 track: FFmpeg 8 channels, oadec 6; `verify` says CLEAN, exit 0 | **FAIL** |
| Custom channel map | Annex E | parsed at `eac3/bsi.rs:148`, zero consumers | follows from the row above | N/I |
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
| EMDF in auxiliary data | Annex H | never parsed, `eac3/frame.rs:2088` | no material in the corpus uses it | N/I |
| EMDF in the last dependent substream | TS 103 420 clause 8.2 | dependent substreams are skipped | follows from the dependent-substream failure | N/I |
| EMDF protection words | Annex H clause H.2.2.4 | `emdf/container.rs:231` | read, stored, never verified | N/I |
| Payload configuration constraints | TS 103 420 Table 56 | `emdf/container.rs:160` | all nine fields parsed, none validated | PARTIAL |
| Payload dispatch, 11 and 14 | TS 103 420 Table 55 | `emdf/container.rs:28`, `:31` | discrimination matrix, six inputs, all correct | PASS |
| `addbsi` Atmos declaration | TS 103 420 clause 8.3 | `eac3/bsi.rs:55` | flag true and complexity index 16 on JOC streams, which is 15 objects plus the LFE bed | PASS |
| Complexity index cross-checked against OAMD | clause 8.3.2.2 | reported only, `cli/eac3.rs:746` | no cross-check exists | PARTIAL |
| JOC Huffman tables | Annex A.1 | `emdf/joc_tables.rs` | 582 node pairs identical to ETSI's `ts_103420_tables.c` | PASS |
| QMF prototype | clause 7.4 | `joc/qmf_window.rs` | 640 values identical to ETSI `prot64` as float64 | PASS |
| Parameter band mapping | Table 54 | `joc/lib.rs:19` | all 23 rows and 8 columns match, including the worked example | PASS |
| Dequantisation | clause 6.6.4 | `joc/lib.rs:67` | formula identical to Pseudocode 5 | PASS |
| Differential decode, dense | clause 6.6.2 Pseudocode 3 | `emdf/joc.rs:244` | identical to the printed pseudocode | PASS |
| Differential decode, sparse | Pseudocode 2 | `emdf/joc.rs:202` | literal reading implemented, matches the print; **0 of 231 645 object updates use sparse mode** | N/T |
| Clip gain value and use | clause 6.3.3.2 | `emdf/joc.rs:152`, `cli/eac3_objects.rs:280` | formula matches; the use is measured, not specified | PASS (value) / INFERRED (use) |
| `joc_num_objects`, reserved configurations | clauses 6.3.2.2, 6.3.2.4 | `emdf/joc.rs:141` | 5 to 7 rejected, bits above 15 rejected | PASS |
| Downmix configuration 3 | Table 47 | `cli/eac3_objects.rs:106` | 13 of 15 clips; objects match Dolby | PASS |
| Downmix configuration 0 | Table 47 | same | oadec upmixes per clause 6.6; Dolby refuses. Documented divergence, no reference | N/T |
| Downmix configurations 1, 2, 4 | Table 47 | `emdf/joc.rs:29` | need 7 downmix channels, unreachable while dependent substreams are skipped | N/T |
| Temporal interpolation, smooth, 1 data point | clause 6.6.5 | `joc/lib.rs:208` | exercised on every clip; objects match Dolby | PASS |
| Temporal interpolation, steep, 1 data point | clause 6.6.5 | same | 2 760 object updates in the corpus | PASS |
| Temporal interpolation, 2 data points | clause 6.6.5 | same | **0 occurrences in the corpus** | N/T |
| `joc_mix_mtx_prev` zero at stream start | clause 6.6.5 | `joc/lib.rs:96` | zero-initialised | PASS |
| Splice reset on a zero sequence counter | clause 6.3.3.3 | `joc/lib.rs:118` and three siblings | the reset methods have no callers | N/I |
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
| Corruption never silently accepted, TrueHD | production quality | `truehd/decoder.rs:698` | 98 of 100 flagged by `decode`, 100 of 100 by `verify` | PASS |
| Corruption never silently accepted, JOC objects | production quality | `cli/eac3_objects.rs` | **99 of 120 produced object output with no diagnostic and exit 0**; `verify` caught all 99 | **FAIL** |
| Reserved JOC extension configuration | clause 6.3.2.5 | `emdf/joc.rs:263` | rejected after the payload is consumed, then stale matrices are held with an anonymous error count | PARTIAL |
| Seek and random access | stateful codec | no seek API | files are always decoded from byte 0 | N/I |
| Environment-independent decode | reproducibility | `cli/eac3_objects.rs:100`, `:203` | three variables silently alter JOC decoding and appear in no output | FAIL |
