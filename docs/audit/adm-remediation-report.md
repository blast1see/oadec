# ADM remediation report

**Date:** 2026-09-14. **Branch:** `adm-remediation`, opened from `main` at `7b07189` (the
merge of the 2026-09-11 ADM semantic-fidelity audit, PR #2). **Plan:**
`docs/audit/adm-remediation-plan.md` (commit `83115c7`), approved with its fourteen decisions
as recommended. **Baseline:** the audit report, its conformance matrix and its evidence under
`docs/audit/evidence/adm/`, all unchanged by this branch. **Evidence of this remediation:**
`docs/audit/evidence/adm-remediation/`; driver and reproduction:
`docs/audit/adm-remediation/README.md`.

## 1. Summary

The audit's fourteen findings are closed. The three P1 defects are fixed: an active object's
gain is written, intermediate-spatial-format (ISF) elements are refused or dropped on explicit
request instead of vanishing, and every lossy mapping is counted and printed, with a new exit
code 4 for a file written with a declared loss. The P2 defects that were oadec's to fix are
fixed: three size axes reach the programme model, a non-48 kHz ADM is refused unless asked for,
objects are numbered from `AO_100b` whatever the bed. The P3 tiling, presentation and frame-rate
items are fixed. The 250-sample interpolation length stays, as the profile requires and as
Dolby's converters write it; it is now declared on every run, and an opt-in, marked, non-profile
mode writes the stream's own ramps.

For the streams oadec decodes today nothing changed but the words on stderr: of the 51 decodes
the audit recorded (25 inputs, DAMF and ADM, default and `--no-bed-conform`), 50 produce
byte-identical output with the remediated binary, and the one that differs, the
`--no-bed-conform` ADM of pi-head50m, differs only in its object IDs, which are now the ones
Dolby's Conversion Tool gives the same programme. Its audio, its block inventory and its ledger
against the unchanged DAMF are the audit's.

For scenes the corpus does not contain the writers now do what the audit asked: the Conversion
Tool reads -6, +3, -inf and -12 dB back from oadec's gain file, the Conversion Tool's and the
Encoding Engine's own ADM of the same scene carry the same four gain strings, Dolby's validators
accept the tagged file, and the EBU ADM Renderer renders oadec's file and Dolby's identically.

The validation corrected one thing the plan had wrong. Decision 3 described Dolby's muted active
gain as `0.0`; the Conversion Tool writes `0.0000000000`, and the remediation's own Dolby stage
was the first measurement of that text (the audit's records keep parsed values, not strings).
The writer was changed to match, in its own commit, before the final runs.

## 2. Rules the work followed

- The merged audit is the baseline; nothing was re-audited. Its reports, evidence and toolkit are
  immutable; the toolkit is imported read-only by the remediation driver, and the harness crate
  under `docs/audit/adm/harness` was updated in place only to compile against the new writer
  API and to expose the new options (decision 11).
- The 250-sample interpolation length is Dolby-profile-conformant (profile table 11; Dolby's
  Conversion Tool and Encoding Engine write the same 0 / 0.005208 pattern; Dolby's reader
  returns 0 for any value). It is not replaced. The real-ramp mode is opt-in and marked.
- Decoder guarantees are preserved and proved by the byte-identity replay; no decoder code was
  touched.
- Every fix was written test first (the failing test is in the commit that makes it pass);
  `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and
  `cargo test --workspace --locked` pass at every commit.
- The crate version stays 0.2.0 so the `dbmd` tool string, and with it the byte-identity gate,
  hold; the bump to 0.3.0 is a separate release commit after the merge (decision 12).

## 3. The defects, before and after

| Audit item | Fix | Before | After | Classification | Where it is checked |
|---|---|---|---|---|---|
| D1 (P1) active object gain not written | R2, `e31d9e3`, `feca038` | a muted or attenuated object came out at full level | `<gain>` written as the Conversion Tool writes it: linear, ten decimals of float32 arithmetic (`0.5011872053` for -6 dB, `1.4125375748` for +3 dB, `0.2511886358` for -12 dB, `0.0000000000` for -inf), nothing at 0 dB; a gain-only change gets its own block; the inactive marker (`0.0` with importance 0) unchanged | interoperability-driven: follows Dolby's tools against the letter of table 11; Dolby's validators accept it | unit tests `active_gain_is_written_as_dolby_prints_it`, `a_gain_only_change_gets_its_own_block`, `the_inactive_marker_is_unchanged`; harness C02, R02; Dolby read-back and gain strings; EAR |
| D2 (P1) ISF elements dropped in silence | R3, `3a8c412` | audio and metadata of ISF objects vanished from both outputs; the TrueHD driver hard-coded the count to 0 | the TrueHD ISF count comes from the major sync; `--format damf|adm` refuses the programme with exit 2, naming the ISF type (TS 103 420 table 11b) and count; `--isf drop` writes the rest and exits 4 with `isf-dropped` in the ledger | explicit refusal; declared loss on request; a positioned mapping stays UNK (ring counts only, no reference) | unit tests in `program.rs`, `adm.rs`, `damf.rs`, `cli/damf.rs`; harness C13, C13b |
| D3 (P1) no diagnostic for any lossy path | R1, `b23717e` | every loss left with exit 0 and no message | a loss ledger of seventeen kinds in four classes, printed once per run, one line per class; `--loss-report FILE` writes it as JSON; exit 4 for declared losses; integrity 7 still wins; policy in `docs/exit-codes.md` | reporting | unit tests in `loss.rs`, `integrity.rs`; CLI fixture tests; every harness case (ledger in the harness summary) |
| D4 (P2) fixed 250-sample interpolation | kept; R1 declares it; R10 `81e0b83` optional | silent | `adm: profile reductions: N interpolation lengths replaced by 250 samples (1536 xA, 32 xB)` on every run; `--adm-interpolation real` writes the source ramps (ten-decimal seconds, 0 on the first block), keeps ramp-only changes as blocks, marks the `dbmd` tool string, says so on stderr and refuses `--dolby-origin-tag` | default profile-conformant and Dolby-identical; real mode a marked non-profile extension | harness C01, C20, C20b; pi-head50m real mode against the audit's real-ramp variant (ledger, trajectory, EAR) |
| D5 (P2) 3-D size collapsed to the first axis in the model | R4, `7188002` | `size: r.size[0]` before either writer | `ObjectState.size: [f32; 3]`; both writers write the width (both formats carry one value) and count `size-axes-collapsed` per event whose axes differ | profile-conformant reduction with a diagnostic | unit tests in `program.rs`, `adm.rs`, `damf.rs`; harness C04 |
| D6 (P2) non-48 kHz ADM written against table 23 | R5, `c025c6c` | written, 250-sample constant scaled | refused with exit 2; `--adm-allow-non-profile-rate` writes it, counts `non-profile-sample-rate`, exits 4; the profile checker's finding stays on the override | profile-conformant refusal; declared loss on request | unit test; harness C15, C15b |
| D7 (P2) object IDs below `AO_100b` with `--no-bed-conform` | R6, `a1a6d33` | `0x1001 + bed + k - 1` | objects numbered from `AO_100b` regardless of the bed (table 17), more than 118 refused, 128 tracks refused; wide and second-LFE bed channels pinned as refused | profile-conformant, Dolby-identical (audit S11) | unit tests; harness C11, C12, C21b; replayed nbc ADM against Dolby's S11 IDs |
| D8 (P3) last block overruns the programme end | R7, `a9ee661` | duration from an event beyond `frames` | events at or beyond the end dropped and counted; the last block ends at `frames`; block lists equal the Conversion Tool's | profile-conformant tiling | unit test; harness C07 against Dolby's recorded blocks |
| D9 (P3) out-of-order events overlap | R7, `a9ee661` | writer never sorted | ADM sorts (stable) and tiles cleanly, block lists equal the Conversion Tool's; DAMF writes as delivered and declares `out-of-order-written-as-is` (exit 4) rather than buffering a film | profile-conformant tiling; DAMF declared loss | unit tests; harness C09 |
| D10 (P3) late first event held from 0, arrival time lost | R7, `a9ee661` | synthetic block absorbed the real first event | the hold-from-zero block stays (blocks must start at 0) and the real first event keeps its own block; counted as `late-first-event-held`; Dolby's default (0,0,0) block deliberately not adopted (decision 6) | oadec's documented choice within the profile | unit test; harness C06 (`absorbed_by_synthetic` 0) |
| D11 (P3) importance and time-varying bed gain not carried | R1 declares | silent | `importance-omitted`, `bed-gain-dropped`, `bed-event-dropped` counted and printed as profile reductions | profile-conformant (tables 9-11), Dolby-identical (S3, S8) | harness C03, C05 |
| D12 (P3) distance, divergence, warp mode, trim discarded | R1 declares | silent | counted in the timeline (`distance-dropped`, `divergence-dropped`, `warp-mode-dropped`, `trim-config-dropped`) and in the ADM writer (`screen-reference-dropped`, `trim-bypass-dropped`), printed as "not representable in DAMF or the ADM profile" | unrepresentable in both formats; a BS.2076 non-profile mode is deferred | harness C19; the JOC fixture prints `1 warp mode setting` |
| D13 (P3) `--presentation` ignored; DAMF `fps` fixed | R8, `92f2114` | accepted and ignored; 24 | `--presentation` optional (WAV/PCM default 2), refused with the object formats unless 3, exit 2 before anything is written; `--fps` 23.976/24/25/29.97/30 for the DAMF header, default 24 | CLI hygiene | unit tests; CLI fixture test |
| D14 (P3) `RF64` FourCC; `bed_mask_bit` collision | deferred | as audited | unchanged: BS.2088 was not obtainable and every reader accepts RF64; the colliding channels are refused before the mask is built, and a test pins that refusal | DEFERRED / UNK | unit test `unsupported_wide_and_second_lfe_channels_are_refused` |

The audit's backlog item 12 (`--format adm` in CI) is R11, `4aa40b3`: a writer-level round trip
read back by an independent RIFF/chna/axml walker, a CLI test on two committed Dolby Encoding
Engine encodes of one synthetic scene (TrueHD and JOC separately), a media-gated gate that the
default ADM of pi-head50m stays byte-identical to the audited file, and a CI job that runs the
audit toolkit's self-tests and its writer-level cases with expectations.

## 4. What a user now sees

The ledger prints one line per class after the run's own summary. The committed fixtures
show the two common shapes:

```text
adm: profile reductions: 13 interpolation lengths replaced by 250 samples (1536 x12, 768 x1)
```

```text
adm: profile reductions: 17 interpolation lengths replaced by 250 samples (...)
adm: not representable in DAMF or the ADM profile: 1 warp mode setting
```

A declared loss adds a line and changes the exit code:

```text
adm: written with loss: 4 intermediate-spatial-format elements dropped (--isf drop)
```

| Class | Kinds | Exit |
|---|---|---|
| profile reductions | `ramp-replaced`, `importance-omitted`, `bed-event-dropped`, `bed-gain-dropped`, `screen-reference-dropped`, `trim-bypass-dropped` | 0 |
| not representable in DAMF or the ADM profile | `distance-dropped`, `divergence-dropped`, `warp-mode-dropped`, `trim-config-dropped` | 0 |
| approximations | `size-axes-collapsed`, `late-first-event-held`, `same-position-superseded`, `event-beyond-end-dropped` | 0 |
| written with loss | `isf-dropped`, `out-of-order-written-as-is`, `non-profile-sample-rate` | 4 |

New flags: `--loss-report FILE`, `--isf <error|drop>`, `--adm-allow-non-profile-rate`,
`--adm-interpolation <profile|real>`, `--fps`. Refusals before anything is written (exit 2):
an ISF programme without `--isf drop`, a non-48 kHz ADM without the override, more than 118
objects or 128 tracks, `--presentation` other than 3 with the object formats, and
`--adm-interpolation real` together with `--dolby-origin-tag`.

## 5. Validation

The measure is the audit toolkit, not the writers' own tests: the same normaliser,
reconciliation ledger, tiling check, profile checker and PCM comparison that found the defects.
The driver `docs/audit/adm-remediation/tools/run_remediation.py` adds an expectation per case
(what the fix must have changed and what must have stayed) and exits non-zero when any is not
met. Two readings of the toolkit's output are deliberate, documented in the driver and
repeated here so that no one mistakes them for hidden tolerance:

- The ledger classifies a metadata state from the DAMF side: a state whose only changed fields
  are in the profile's inexpressible set is filed under `inexpressible_change` even when the
  block now carries the value. The value is checked by the ledger's own `loss` counters (the
  toolkit compares a gain wherever the ADM carries one, so `loss.gain 0` is its confirmation
  that the gains agree) and by the driver block by block.
- The normaliser derives "inactive" from a bare zero gain (`inferred-gain0`), so Dolby's
  encoding of an active muted object draws one `value_mismatch` on `active`; exactly that one
  item, with its object, time and field pinned, is accepted on C02. The profile checker
  likewise flags every gain-bearing block (`profile-inactive-encoding`, table 11 read
  literally), as it did on Dolby's own S2 files in the audit's evidence.

### 5.1 Writer-level cases (27 of 27)

The audit's C01-C21b through the remediated writers, plus five remediation variants. Evidence:
`adm-remediation-harness.json`.

| Case | What must hold | Result |
|---|---|---|
| C01 ramps | four ramps replaced, tiling clean | as expected |
| C02 gain, R02 tagged | `loss.gain 0`, `loss.importance 0`; strings `0.5011872053`, `1.4125375748`, `0.2511886358`, `0.0000000000`; muted object gain present and importance absent; the gain change at 48000 its own block with `0.2511886358`; one accepted `value_mismatch{active}` on the muted object; four `profile-inactive-encoding` findings and nothing else | as expected |
| C03 importance | two importance losses (profile) | as expected |
| C04 size 3-D | width 0.2 written; `size-axes-collapsed` 1 in ADM and DAMF | as expected |
| C05 bed events | `bed_changes_lost` 2; `bed-gain-dropped` 1, `bed-event-dropped` 2 | as expected |
| C06 late first | two synthetic blocks, `absorbed_by_synthetic` 0, both real events matched, `late-first-event-held` 2, tiling clean | as expected |
| C07 beyond end | tiling clean, last end 96000, two events dropped and counted, block lists equal the Conversion Tool's | as expected |
| C08 same position | one superseded, counted | as expected |
| C09 out of order | tiling clean, no unsorted object, four matched, DAMF `out-of-order-written-as-is` 1, block list equals the Conversion Tool's | as expected |
| C10 zero objects | 10 bed tracks, no finding | as expected |
| C11 no bed, C12 118 objects, C21b nbc | no `profile-id` finding; first object `AO_100b`; 119 channels for 118 objects | as expected |
| C13 ISF default | both writers refuse, message names the ISF type | as expected |
| C13b `--isf drop` | written; `isf-dropped` 4 in both ledgers; declared loss; no ISF tone in any track | as expected |
| C14 Tfl bed | refused | as expected |
| C15 96 kHz default | ADM refused, DAMF written | as expected |
| C15b override | written; `non-profile-sample-rate` 1; the profile checker's two rate findings remain; constant 0.002604 | as expected |
| C16 active toggle, C17 zones, C18 snap, C19 extras | unchanged behaviour; `distance-dropped` 2 in the timeline, `screen-reference-dropped` 1 in the ADM | as expected |
| C20 ramp-only | profile mode: one duplicate block, one popped, ramps counted | as expected |
| C20b real ramps | `loss` empty, four blocks with lengths 0/32/1536/32 samples, no defect, trajectory error 0, only `profile-interpolation-length` findings, `dbmd` marked | as expected |
| C21 bed order | tones under the right labels | as expected |
| R01 119 objects | ADM refused (`at most 118 objects`), DAMF written | as expected |

### 5.2 Byte identity of every audited decode

Every decode run record embedded in `adm-work-inventory.json` was replayed with the remediated
binary (SHA-256 `0689ba53b797ae3109b660da2329c3d5a126b5aec66bc5260222a871dd4e961e`) and each
output hashed against the audited hash. Evidence: `adm-remediation-byte-identity.json`.

| Records | Byte-identical | Expected difference, verified | Unexplained | Exit codes as audited |
|---|---|---|---|---|
| 51 (25 inputs; 28 JOC, 18 TrueHD, 5 pi-head50m) | 50 | 1 | 0 | 51 of 51 |

The one difference is `work/pi-head50m/nbc/pi-head50m.adm.run.json`, the `--no-bed-conform`
ADM (182 798 334 bytes, as audited; SHA-256 now
`ddde17a31528fc4e736e0af25a0d1f9d6e4ff5c4029f5e9cd2f09b062315bfb0`, audited
`495146cb9395dabd17c3d7ac2eecf87353528bc1683293ea87ba9b18a1b5f3d9`). Against the unchanged DAMF beside it the toolkit gives the
ledger the audit recorded for the audited file (`matched 119`, `inexpressible_change 9`,
`trailing_popped 2`, `bed_event 2`, no defect, the same tiling), all twelve tracks identical,
and object IDs `AO_100b`..`AO_1015`, which are the IDs Dolby's Conversion Tool gave the same
programme (audit stimulus S11). The only profile finding is the one inherent to a
non-conformed LFE-only bed (`profile-bed-configuration`), which `--no-bed-conform` asks for.
The whole-film Pi records (`big/`) were not replayed (decision 13): the RF64 path was not
touched and the audit's C22 evidence stands.

### 5.3 Dolby's tools

Evidence: `adm-remediation-dolby.json`. The Conversion Tool is 2.1.2, the validators
`atmos_info` 5.7.2 (`--validate 1`) and 1.1, and `bwf_info`.

| Check | Result |
|---|---|
| Conversion Tool reads oadec's C02 ADM back to DAMF | exit 0; gains -6, +3, -inf and -12 dB (at 48000) on the four objects, 0 dB before the change |
| Conversion Tool's own ADM of the same DAMF | gain strings `0.0000000000`, `0.2511886358`, `0.5011872053`, `1.4125375748`: equal to oadec's; its read-back equals oadec's read-back |
| DEE 5.2.1 `convert_atmos_mezz` on the same DAMF | exit 0; gain strings `0.0000000000`, `0.2511886358`, `0.5011872053`, `1.4125375748`: equal to oadec's and to the Conversion Tool's |
| `atmos_info` 5.7.2 and 1.1, `bwf_info` on the tagged gain file | all exit 0, no finding |
| the same on the untagged file | `Content was not authored with Dolby tools`, as in the audit; `bwf_info` exit 0 |
| object IDs of the replayed `--no-bed-conform` ADM | equal to Dolby's S11 list |

### 5.4 EBU ADM Renderer

Evidence: `adm-remediation-render.json`. The renderer is `ear-render` 2.1.0; as in the audit,
the `audioPackFormatIDRef` the profile puts in `audioStreamFormat` is removed from every input
alike, because the renderer follows BS.2076 strictly.

| Comparison | Layouts | Result |
|---|---|---|
| oadec's C02 (gain on active objects) against the Conversion Tool's C02 | 0+5+0, 4+7+0 | sample-identical (maximum absolute difference 0) |
| pi-head50m in `--adm-interpolation real` against the audit's real-ramp variant of the default file (`mutate.set_interpolation` with the DAMF ramps) | 0+2+0, 4+7+0 | sample-identical (maximum absolute difference 0) |
| the toolkit on the real-mode file against the DAMF | -- | `loss` empty, no defect, 130 blocks (119 matched + the 11 ramp-only states now written), trajectory error at most 3e-8 room units (ten-decimal seconds round a 32-sample ramp by 2e-6 samples) |

### 5.5 Unit and integration tests

`cargo test --workspace --locked` passes at the final commit (208 tests; the 22 media-gated
tests of `tests/real.rs` stay ignored without `OADEC_MEDIA`, and the byte-identity gate among
them passed against the audited hash when run with the media), `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo fmt --all --check` are clean, and the CI job's own
command line (`run_remediation.py --stage harness --no-dolby --no-ear` with its default paths)
was run locally and passed. The audit toolkit's 136 self-tests pass unchanged.

## 6. What stays as it is, and why

| Item | Status |
|---|---|
| 250-sample interpolation length by default (D4) | INTENTIONAL-PROFILE; declared; real mode opt-in |
| importance on active objects; bed events and static bed gain (D11) | INTENTIONAL-PROFILE; declared |
| the inactive marker keeps the object's position (Dolby zeroes it) | INTENTIONAL; profile allows it |
| distance, divergence, warp mode, trim, screen reference, trim bypass (D12) | unrepresentable; declared; a BS.2076 non-profile mode DEFERRED |
| positioned ISF mapping | UNK / DEFERRED (TS 103 420 gives ring counts only; no Dolby reference); refuse or drop explicitly |
| `RF64` FourCC (D14) | DEFERRED / UNK until BS.2088 is available; every reader accepts RF64 |
| `bed_mask_bit` collision (D14) | unreachable; the refusal is pinned by a test |
| bed configurations beyond the 7.1.2 label set | explicit error already; 5.1 `Ls/Rs` labelling N/T |
| the JOC 256-sample decoder delay, `Timeline` semantics, all decoder behaviour | out of scope; preserved, proved by the replay |

## 7. Decisions applied

All fourteen decisions of the plan were applied as recommended: exit code 4 with the class
table (1); ISF default error, `--isf drop` explicit (2); gain on active objects by default in
Dolby's encoding (3); size reduced to the width with a diagnostic and `[f32; 3]` in the model
(4); non-48 kHz refused unless overridden (5); the late first event keeps the hold block and its
own block, not Dolby's default block (6); ADM sorts, DAMF writes as delivered and declares (7);
`--presentation` optional and refused unless 3, `--fps` limited to five rates (8);
`--adm-interpolation real` marked and refusing the origin tag (9); two small DEE-encoded
fixtures committed (10); harness updated in place, toolkit untouched, new material under
`adm-remediation` paths (11); version kept at 0.2.0 (12); no whole-film re-run (13);
`--loss-report` included (14).

One detail of decision 3 was corrected by measurement: the muted active gain is
`0.0000000000` in the Conversion Tool's output, not `0.0`; commit `feca038`.

## 8. Commits, files, evidence

```text
83115c7 docs(audit): ADM remediation plan (no code changes)
b23717e spatial+cli: a loss ledger for the object outputs, printed per run; exit 4 for declared losses
e31d9e3 adm: write the gain of active objects as Dolby's converters do
3a8c412 objects: refuse ISF elements unless --isf drop; derive the TrueHD ISF count from the major sync
7188002 program: carry all three size axes; write the width and say when the axes differ
c025c6c adm: refuse programmes that are not 48 kHz unless --adm-allow-non-profile-rate
a1a6d33 adm: number objects from AO_100b regardless of the bed; refuse more than 118 objects
a9ee661 adm: sort events, end the last block at the programme end, keep a late first event's own block
92f2114 cli: --presentation is refused with the object formats unless it is 3; --fps for the DAMF header
81e0b83 adm: opt-in real interpolation lengths, marked non-profile
4aa40b3 tests: end-to-end coverage of the object outputs in CI
feca038 adm: a muted active object's gain is written with ten decimals, as the Conversion Tool writes it
        docs: ADM remediation report, evidence and user documentation (this commit)
```

Production code touched: `crates/oadec-spatial/src/{loss.rs (new), program.rs, adm.rs, damf.rs, lib.rs}`,
`crates/oadec-cli/src/{main.rs, damf.rs, eac3_objects.rs, integrity.rs, author.rs, compare.rs, decode.rs, eac3.rs}`.
Tests: `crates/oadec-spatial/tests/adm_roundtrip.rs`, `crates/oadec-cli/tests/{cli_adm.rs, real.rs, fixtures/}`.
Documentation: `README.md`, `CHANGELOG.md`, `docs/exit-codes.md`, `docs/dolby-tools.md`, this report,
`docs/audit/adm-remediation/README.md`. CI: job `adm-toolkit` in `.github/workflows/ci.yml`.

Evidence (`docs/audit/evidence/adm-remediation/`): `00-provenance.json`,
`adm-remediation-harness.json`, `adm-remediation-byte-identity.json`,
`adm-remediation-dolby.json`, `adm-remediation-render.json`, `manifest.json`. Large
intermediate files (replayed outputs, renders, conversions) were deleted after the evidence was
written; the evidence keeps every hash, exit code, command line and measurement.

## 9. Still open

- A positioned ISF mapping (UNK; needs a reference) and a BS.2076 non-profile mode for
  `objectDivergence` and `absoluteDistance` (deferred).
- `BW64` per BS.2088 once the text is available.
- The version bump to 0.3.0 in a release commit after the merge: done the same day, with a
  different gate than planned (section 10).
- The whole-film replay, if wanted: `run_remediation.py --stage regression --with-full-film`.

## 10. Postscript, after the merge (2026-09-14)

PR #3 was merged as a normal merge commit (`7615c2e`) and 0.3.0 released the same day
(`release: 0.3.0`, `2ca2e63`, tag `v0.3.0`; the release workflow published the Windows and
Linux archives). Two things went differently from what section 9 planned:

- **The gate was not re-derived; it was made independent of the version.** The 0.2.0 and
  0.3.0 outputs of pi-head50m differ in exactly two bytes, both inside the `dbmd` payload: the
  version digit of the tool string and the segment's checksum, which moves with it. The gate
  now hashes everything up to the `dbmd` payload against the audited file's hash of that same
  prefix (`3809b77a5c05753ac45542a7766687c3bfed9c0868c7d659cf123b5a5434edc6`) and checks the
  payload's size, its pad byte and its two strings on their own (`41c3c88`). The first version
  of that gate, pushed with the release, expected two string replacements and failed when run
  with the media; the release chain did not stop because the test's output had been piped
  through a grep that returned success. The release artefacts do not depend on the test, and
  the tag stays where it is.
- **The harness crate has a lock file of its own.** `docs/audit/adm/harness/Cargo.lock`
  records the path dependencies' versions and a workspace bump does not touch it; the
  `adm-toolkit` job builds it with `--locked` and failed on `main` until `d24d6a5`. A release
  now touches `Cargo.toml`, `Cargo.lock`, that file and the changelog.
