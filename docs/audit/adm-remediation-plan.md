# ADM Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task once it is approved. Steps use checkbox (`- [ ]`) syntax for tracking. **Nothing in this plan is to be implemented before the decisions in section 7 are approved.**

**Goal:** Close the writer-level defects of the ADM BWF export that the 2026-09-11 audit confirmed (D1–D14), so that every semantic the Dolby Atmos Master ADM profile can carry is carried, every semantic it cannot carry is declared, and the default 250-sample profile behaviour stays exactly as Dolby writes it.

**Architecture:** A `LossLedger` in `oadec-spatial` records every lossy mapping at the place it happens (programme model, DAMF writer, ADM writer); both writers return it in their summaries; the CLI prints it and maps declared losses to a new exit code. Each defect is then fixed in the writer or the programme model with the ledger reporting what remains. The ADM writer gains Dolby's gain encoding, bed-independent object numbering, sorted and end-clamped block tiling, an explicit ISF policy, a sample-rate guard, and an opt-in non-profile real-ramp mode. CI gets a true `--format adm` end-to-end path.

**Tech Stack:** Rust 1.98 / edition 2024 (workspace lints: `unsafe_code = forbid`, clippy `-D warnings`), clap 4, no new runtime dependencies in `oadec-spatial`; validation with the audit toolkit (`docs/audit/adm/tools`, Python 3.12 venv at `E:\oadec-work\audit\adm\venv`), the Rust harness (`docs/audit/adm/harness`), Dolby Conversion Tool 2.1.2, DEE 5.2.1, `atmos_info` 5.7.2/1.1, EBU ADM Renderer 2.1.0.

**Spec:** `docs/audit/2026-09-11-adm-semantic-fidelity-audit.md` (section 19 defects D1–D14, section 23 backlog, Appendix A answers 15/16/27), `docs/audit/adm-conformance-matrix.md`, and the remediation brief of 2026-09-13 (priorities 1–9). Evidence the defects rest on: `docs/audit/evidence/adm/adm-harness.json` (C01–C22b), `adm-dolby-reference.json` (S0–S13), `adm-gain-diff.json`, `adm-event-timing.json`, `adm-work-inventory.json` (output hashes of every audited decode).

## Global Constraints

- **Plan first.** This document is committed alone; no file under `crates/` changes until section 7 is approved.
- **Branch:** `adm-remediation`, created from `main` at `7b07189f15652a6fcb7dc7e9e0b78a1666eb05a4`. The audit branch `adm-audit` is not touched.
- **Decoders untouched.** `oadec-truehd`, `oadec-eac3`, `oadec-joc` and the parsing code of `oadec-emdf` are not modified; the remediation only reads fields they already parse (`RenderInfo.size`, `.distance`, `ExtendedObjectElement.divergence`, `TrimElement`, `ExtraChannelMeaning.isf_index`/`has_isf()`, `ISF_OBJECTS`).
- **250 samples stay.** `INTERPOLATION_SAMPLES = 250` and the `0 / 250` pattern remain the default (profile table 11; Dolby's converters write the same, `adm-dolby-reference.json` S1: `{0: 8, 250: 3278}` for both tools). A real-ramp mode is opt-in and marked non-profile.
- **Audit history immutable.** `docs/audit/2026-09-11-*.md`, `docs/audit/adm-conformance-matrix.md`, `docs/audit/evidence/adm/**` and `docs/audit/adm/tools/admaudit/**` are not edited. New evidence goes to `docs/audit/evidence/adm-remediation/`; the closing report is `docs/audit/adm-remediation-report.md`. The harness crate under `docs/audit/adm/harness` may be updated only so that it compiles against the changed API and can drive the new options (decision 7.11). **Exception, 2026-09-17, for publication:** an absolute path that names the machine a run was made on may be made repository-relative in these files, because it identifies a person rather than a measurement. Nothing else changes -- not a hash, not a finding, not an exit code, not the `files[].path` keys the byte-identity gate looks records up by. The Dolby installation path and the work directory stay as recorded: they say which tool ran and where, which is part of the measurement.
- **Byte-identity gate.** For every audited input whose decode triggers none of the changed paths (all 25 default-conform outputs: gain 0 dB, importance 1.0, size 0, no ISF, 48 kHz, in-order, first events at 0), the DAMF set and the ADM file written by the remediated binary must be byte-identical to the audited artefacts (hashes in `adm-work-inventory.json`; pi-head50m default ADM `5abe2e85abf13113c5075bf56ed614e2137ddf33f0ba8951562c6d16c7cce15c`). The only permitted difference is the `dbmd` tool string if the crate version changes (decision 7.12).
- **Exit codes** stay documented in `docs/exit-codes.md`; the new code is added there in the same commit that introduces it.
- **No new runtime dependencies** in `oadec-spatial` (serialisation of the ledger is done in the CLI with `serde_json`, which it already has).
- **TDD** per task: failing test first, minimal implementation, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, commit.

---

## 0. Baseline facts the plan relies on

| Fact | Where established |
|---|---|
| `AdmWriter::axml()` writes `<gain>`/`<importance>` only for inactive objects (`adm.rs:488-492`); `adm_equal` ignores gain (`adm.rs:589-596`) | audit section 8, harness C02 (3 gain losses, the −12 dB change popped) |
| Dolby CT/DEE write `<gain>` linear with ten decimals on active objects when the gain is not 0 dB, omit it at 0 dB, write `<gain>0.0</gain>` alone for −∞, and emit a block for a gain-only change | `adm-harness.json` C02 `ct.adm_blocks`: 0.5011872053 / 1.4125375748 / 0.0 / 0.2511886358; S2: 4 of 5 objects |
| `10f32.powf(db/20)` printed with `{:.10}` reproduces Dolby's strings exactly (−6 → 0.5011872053, +3 → 1.4125375748, −12 → 0.2511886358); an f64 computation does not (0.5011872336) | computed 2026-09-13 with numpy float32; recorded in this plan |
| Object IDs are `0x1001 + bed_tracks + k - 1` (`adm.rs:374`), so a bed of fewer than ten channels yields `AO_1002…` | harness C11/C12/C21b `profile-id` findings; Dolby S11 (LFE-only bed) numbers objects `AO_100b…` |
| Events are written in arrival order; `next <= pos` skips a state; the last block's `next` is not clamped to `frames` | harness C09 (overlap 1, state lost; Dolby sorts: blocks 0/24000/48000), C07 (block ends at 96040 > 96000; Dolby ends at 96000) |
| A first event after 0 is absorbed into a synthetic block holding the first state; Dolby writes an active default block at (0,0,0) then the real event | harness C06 `oadec` vs `ct` blocks |
| ISF elements get `Slot::None` in both writers and `element_id() == None` in the timeline; the TrueHD driver hardcodes `isf_objects: 0` although the major sync carries `isf_index` and `has_isf()` | `adm.rs:178-180`, `damf.rs:121-123`, `program.rs:136-143`, `cli/damf.rs:86`, `oadec-truehd/src/channel.rs:72-90`; harness C13 |
| `ObjectState.size` is one scalar taken from `r.size[0]` (`program.rs:242`); OAMD carries `[width, depth, height]` (`oamd.rs:621-635`) | harness C04 |
| `audioTrackUID sampleRate` follows the stream; nothing checks 48 000 | harness C15 |
| `oadec-spatial` has no logging; `Findings` (`cli/integrity.rs`) knows only integrity faults; exit codes 0/2/7 | `docs/exit-codes.md`, audit D3 |
| `--presentation` defaults to 2 and is ignored by the object formats (the session is created with presentation 3 at `cli/damf.rs:205`); DAMF `fps` is the constant "24" (`damf.rs:42`) | audit D13; Dolby CT accepted an fps of 23.976 (S10) |
| Distance, divergence, warp mode and trim configurations are parsed and never reach `ObjectState` | `program.rs:229-250`, harness C19 |

---

## 1. Design shared by every fix

### 1.1 The loss ledger (`crates/oadec-spatial/src/loss.rs`, new)

```rust
//! What a DAMF or ADM output could not carry of the programme it was given,
//! recorded where the loss happens and reported by the CLI.
use std::collections::BTreeMap;

/// One kind of information the output does not carry as the programme had it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossKind {
    // ProfileReduction: the Dolby Atmos Master ADM profile cannot carry it and
    // Dolby's converters drop it identically (ADM only).
    RampReplaced,           // interpolationLength fixed at 250 samples (table 11)
    ImportanceOmitted,      // active object whose importance is not 1.0
    BedEventDropped,        // a bed state change after the first event
    BedGainDropped,         // first bed state with a gain other than 0 dB, or inactive
    ScreenReferenceDropped, // screen_factor != 0 (screenRef is forbidden)
    TrimBypassDropped,      // trimBypass true (no ADM field)
    // Unrepresentable: neither DAMF nor the profile has a field.
    DistanceDropped,
    DivergenceDropped,
    WarpModeDropped,
    TrimConfigDropped,
    // Approximation: oadec's own choice where the formats leave room.
    SizeAxesCollapsed,      // width/depth/height differ; the width is written
    LateFirstEventHeld,     // the first state is held from sample 0 to its arrival
    SamePositionSuperseded, // two events at one sample: the last wins
    EventBeyondEndDropped,  // an event at or after the programme end
    // DeclaredLoss: the output is written with something missing or outside the
    // profile, at the user's explicit request or because the input is anomalous.
    IsfDropped,             // --isf drop
    OutOfOrderWrittenAsIs,  // DAMF: events written in arrival order (ADM sorts them)
    NonProfileSampleRate,   // --adm-allow-non-profile-rate
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LossClass { ProfileReduction, Unrepresentable, Approximation, DeclaredLoss }

impl LossKind {
    pub const fn class(self) -> LossClass { /* table above */ }
    /// One line of English for the report, e.g. "interpolation length replaced by 250 samples".
    pub const fn describe(self) -> &'static str { /* … */ }
}

/// Counts per kind, the source ramps that were replaced, and up to three
/// (element id, sample position) examples per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LossLedger {
    counts: BTreeMap<LossKind, u64>,
    pub ramp_sources: BTreeMap<u32, u64>,
    examples: BTreeMap<LossKind, Vec<(u32, u64)>>,
}

impl LossLedger {
    pub fn note(&mut self, kind: LossKind, element: u32, sample: u64);
    pub fn note_ramp(&mut self, element: u32, sample: u64, source_ramp: u32); // RampReplaced + histogram
    pub fn count(&self, kind: LossKind) -> u64;
    pub fn merge(&mut self, other: &LossLedger);
    pub fn is_empty(&self) -> bool;
    pub fn declared_loss(&self) -> bool; // any DeclaredLoss kind counted
    pub fn iter(&self) -> impl Iterator<Item = (LossKind, u64, &[(u32, u64)])>;
    /// Human-readable lines grouped by class, in a fixed order, e.g.
    /// "profile reductions: 117 ramps replaced by 250 samples (1536 x106, 32 x11); 2 bed events not carried"
    pub fn lines(&self, target: &str) -> Vec<String>; // target = "adm" | "damf"
}
```

Where each kind is recorded:

| Kind | Recorded by | Condition |
|---|---|---|
| RampReplaced | `AdmWriter::axml` | profile mode, block index > 0, `state.ramp != INTERPOLATION_SAMPLES` (histogram of `state.ramp`) |
| ImportanceOmitted | `AdmWriter::axml` | `state.active && state.importance != 1.0` |
| BedEventDropped / BedGainDropped | `AdmWriter::push_event` | bed event after the first for that id / first bed state with `gain != Db(0)` or `!active` |
| ScreenReferenceDropped, TrimBypassDropped | `AdmWriter::axml` | `screen_factor != 0.0` / `trim_bypass` |
| DistanceDropped, DivergenceDropped | `Timeline::push` (per dynamic-object update) | `render.distance != Distance::Unspecified` / `divergence[index][blk] > 0.0` |
| WarpModeDropped, TrimConfigDropped | `Timeline::push` (per payload) | `trim.warp_mode != 0` / `trim.global_trim_mode == 2 \|\| !trim.configs.is_empty()` |
| SizeAxesCollapsed | both writers | `size[0] != size[1] \|\| size[0] != size[2]` |
| LateFirstEventHeld, SamePositionSuperseded, EventBeyondEndDropped | `AdmWriter::axml` | section 2, R7 |
| IsfDropped | both writers at `create` | `program.isf_objects > 0 && options.isf == IsfPolicy::Drop` |
| OutOfOrderWrittenAsIs | `DamfWriter::push_event` | `sample_pos < last_pos[id]` |
| NonProfileSampleRate | `AdmWriter::create` | `sample_rate != 48_000 && options.allow_non_profile_rate` |

`Timeline` gains `pub losses: LossLedger`; `AdmSummary` and `DamfSummary` gain `pub losses: LossLedger` (both summaries lose `Copy`, keep `Clone`). The CLI merges `timeline.losses` with the writer's ledger.

### 1.2 Exit status policy (extends `docs/exit-codes.md`)

| Ledger class | Printed on stderr | Exit code |
|---|---|---|
| ProfileReduction | one line per output, e.g. `adm: profile reductions: 117 ramps replaced by 250 samples (1536 x106, 32 x11)` | 0 |
| Unrepresentable | `adm: not representable in DAMF or the ADM profile: distance specified on 12 updates (objects 10, 13); …` | 0 |
| Approximation | `adm: approximations: 1 first event held from sample 0 (object 10 at 96000); …` | 0 |
| DeclaredLoss | `adm: written with loss: 4 ISF elements dropped (--isf drop)` and the closing line `the output was written and does not carry the whole programme; see the lines above` | **4** |
| integrity fault (existing) | unchanged | 7 (takes precedence over 4; the loss lines are still printed) |

`Findings` gets `pub fn note_losses(&mut self, ledger: &LossLedger)` and `report()` returns `Verdict { Clean, Lossy, Faulty }`; `main.rs` maps `Lossy` to `ExitCode::from(4)`. A clean decode of a stream whose only losses are profile reductions still prints the one-line summary (the audit's D3 asks that nothing be silent) but exits 0. Optional `--loss-report <file.json>` writes the merged ledger as JSON (decision 7.14).

### 1.3 CLI surface (all in `crates/oadec-cli/src/main.rs`, `Decode`)

| Flag | Values / default | Applies to | Purpose |
|---|---|---|---|
| `--isf <error\|drop>` | `error` | damf, adm | ISF elements: refuse the decode with a message naming the count and the ISF type (exit 2), or drop them explicitly (ledger `IsfDropped`, exit 4) |
| `--adm-interpolation <profile\|real>` | `profile` | adm | `real` writes `interpolationLength = rampLength / rate` (non-profile); refused together with `--dolby-origin-tag` |
| `--adm-allow-non-profile-rate` | off | adm | write a programme whose rate is not 48 000 Hz (ledger `NonProfileSampleRate`, exit 4); without it the decode is refused (exit 2) |
| `--fps <23.976\|24\|25\|29.97\|30>` | `24` | damf | the `fps` field of the `.atmos` header (header only; timing stays sample-based) |
| `--presentation` | becomes `Option<usize>`; wav/pcm default 2 | damf, adm | given with an object format and not equal to 3 → error (exit 2) instead of being ignored |
| `--loss-report <path>` | none | damf, adm | optional JSON dump of the merged ledger (decision 7.14) |

### 1.4 API changes in `oadec-spatial` (all pre-1.0, `publish = false`)

- `program::ObjectState.size: f32` → `size: [f32; 3]` (width, depth, height as OAMD codes them) plus `pub fn uniform_size(&self) -> f32` returning `size[0]`. Construction sites to update: `program.rs` (`object_state`), `adm.rs` (default state and tests), `damf.rs` (tests), `crates/oadec-cli/src/author.rs` (`size: [s; 3]`), `docs/audit/adm/harness/src/main.rs`.
- `program::Timeline.losses: LossLedger`.
- `adm::AdmOptions` gains `isf: IsfPolicy`, `interpolation: Interpolation`, `allow_non_profile_rate: bool` (defaults `Error`, `Profile`, `false`).
- `adm::AdmError` gains `IsfNotRepresentable { count: usize, isf_type: &'static str }`, `NonProfileSampleRate(u32)`, `TooManyObjects { objects: usize, max: usize }`.
- `damf::DamfOptions` gains `isf: IsfPolicy`; `damf::DamfError` (new, `thiserror`) with `Io` and `IsfNotRepresentable`; `DamfWriter::create` returns `Result<Self, DamfError>`.
- `adm::AdmSummary` / `damf::DamfSummary` gain `losses: LossLedger` and drop `Copy`.
- New `pub mod loss;` re-exported from `lib.rs` (`LossKind`, `LossClass`, `LossLedger`, `IsfPolicy`, `Interpolation`).

### 1.5 File map

| File | Change |
|---|---|
| `crates/oadec-spatial/src/loss.rs` | **create**: ledger types (1.1), unit tests |
| `crates/oadec-spatial/src/lib.rs` | export the new module and types |
| `crates/oadec-spatial/src/program.rs` | `size: [f32;3]`, `uniform_size()`, `Timeline.losses`, counting of unrepresentable semantics |
| `crates/oadec-spatial/src/adm.rs` | gain encoding, `adm_equal` with gain (and ramp in real mode), ID formula, sort/clamp/late-first, ISF policy, sample-rate guard, object limit, interpolation mode, ledger, tests |
| `crates/oadec-spatial/src/damf.rs` | `uniform_size()`, ISF policy, out-of-order counting, `fps` validation, ledger, tests |
| `crates/oadec-spatial/tests/adm_roundtrip.rs` | **create**: writer-level end-to-end test with an independent RIFF/chna/axml walker written in the test (no new dependencies) |
| `crates/oadec-cli/src/damf.rs` | ISF count from the major sync; `Options` gains `isf`, `interpolation`, `allow_non_profile_rate`, `fps`, `loss_report`; loss reporting; `Sink` forwards ledgers |
| `crates/oadec-cli/src/eac3_objects.rs` | loss reporting (same helper) |
| `crates/oadec-cli/src/integrity.rs` | `note_losses`, `Verdict` |
| `crates/oadec-cli/src/main.rs` | new flags, `Option<usize>` presentation, exit code 4 |
| `crates/oadec-cli/src/author.rs` | `size: [s; 3]` |
| `crates/oadec-cli/tests/cli_adm.rs` | **create**: CLI end-to-end (`decode --format adm/damf`) on committed synthetic fixtures (decision 7.10) |
| `crates/oadec-cli/tests/fixtures/` | **create**: `authored-scene.ec3`, `authored-scene.mlp` (≤ 1 MB together) + `README.md` with provenance (decision 7.10) |
| `crates/oadec-cli/tests/real.rs` | media-gated ADM regression test (`pi_head_adm_is_byte_identical_to_the_audited_file`) |
| `.github/workflows/ci.yml` | run the new tests (already covered by `cargo test --workspace`); add an `adm-toolkit` job that runs the audit toolkit self-tests and the harness subset on ubuntu (Python 3.12 + numpy) |
| `docs/exit-codes.md`, `README.md`, `CHANGELOG.md`, `docs/dolby-tools.md` | code 4, new flags, gain/ID/ISF behaviour, real-ramp mode |
| `docs/audit/adm/harness/src/main.rs` | compile against the new API; new case knobs (`isf`, `interpolation`, `allow_non_profile_rate`) |
| `docs/audit/adm-remediation/tools/run_remediation.py` | **create**: drives harness cases C01–C22b plus R-cases with the audit toolkit's `admaudit` package (imported, not modified), writes `docs/audit/evidence/adm-remediation/*.json` |
| `docs/audit/adm-remediation-report.md`, `docs/audit/evidence/adm-remediation/` | **create** at the end |

---

## 2. Fix specifications

Each entry carries the fields the brief asks for. "Classification" uses: **profile-conformant** (required or allowed by the Dolby Atmos Master ADM profile v1.0), **interoperability-driven** (matches Dolby's observed output where the profile text is silent or contradicted by Dolby's own tools), **non-profile extension** (explicitly outside the profile, opt-in).

### R1. Loss ledger, diagnostics, exit status (D3; also covers D11 reporting and D12)

- **Affected:** `oadec-spatial/src/loss.rs` (new), `program.rs::Timeline::push`, `adm.rs::{push_event, axml, finish}`, `damf.rs::{push_event, finish}`, `oadec-cli/src/{integrity.rs, damf.rs, eac3_objects.rs, main.rs}`, `docs/exit-codes.md`.
- **Defect / backlog item:** D3 (P1) "no diagnostic for any lossy path; exit 0"; D11 (importance, bed events) and D12 (distance, divergence, warp, trim) are reported, not changed; backlog item 2.
- **Semantic defect:** the writers approximate or drop ramp length, importance, bed events, screen reference, trim bypass, distance, divergence, warp mode, trim configurations, ISF elements and (until R4) the second and third size axes without a word, and the run exits 0.
- **Current behaviour:** `oadec-spatial` has no reporting path; `Findings` knows only integrity faults; `docs/exit-codes.md` defines 0/2/7.
- **Intended behaviour:** every lossy mapping is counted where it happens (table 1.1), printed once per run grouped by class, and declared losses set exit 4 (table 1.2). Profile reductions and unrepresentable semantics print but keep exit 0.
- **Classification:** product diagnostics; profile-conformant output unchanged.
- **Regression tests:** `loss::tests::{a_ledger_counts_and_keeps_three_examples, declared_loss_is_only_the_declared_kinds, merge_adds_counts_and_ramp_histograms, lines_are_grouped_by_class_in_a_fixed_order}`; `program::tests::unrepresentable_semantics_are_counted_by_the_timeline` (an `Oamd` with `Distance::Factor(2.0)`, divergence 1.0 and `warp_mode 1` yields counts 1/1/1); `adm::tests::profile_reductions_are_counted` (ramp 1536, importance 0.5, screen_factor 0.5 → `RampReplaced 1` with `ramp_sources {1536: 1}`, `ImportanceOmitted 1`, `ScreenReferenceDropped 1`); `adm::tests::bed_events_after_the_first_are_counted`; `integrity::tests::a_declared_loss_is_lossy_and_a_fault_wins` (`Verdict` ordering); CLI: `cli_adm::the_loss_summary_is_printed_and_the_exit_code_is_zero_for_profile_reductions` on the fixture.
- **Existing evidence:** `adm-harness.json` every case (no diagnostic anywhere), `adm-batch-summary.json` (stderr of the 25 decodes), audit section 19 D3.
- **Independent validation:** re-run `run_remediation.py` on pi-head50m default: stderr must carry `adm: profile reductions: 117 ramps replaced by 250 samples (1536 x106, 32 x11); 2 bed events not carried` (numbers from `adm-event-timing.json` `pi-head50m/default`: loss.ramp 117, ramp_hist per object, bed_event 2) and exit 0; the ADM file byte-identical to `5abe2e85…`. Harness C13 with `--isf drop` exits 4.
- **Risks:** scripts that parse stderr see new lines (documented in CHANGELOG); the new exit code 4 is only produced by paths that today produce no output difference, so existing pipelines see it only with `--isf drop` or the rate override.
- **Commit:** C1 `spatial+cli: a loss ledger for the object outputs, printed per run; exit 4 for declared losses`.

### R2. Gain of active objects (D1)

- **Affected:** `adm.rs::axml` (block body), `adm.rs::adm_equal`, new `fn gain_text(g: Gain) -> Option<String>`.
- **Defect:** D1 (P1); backlog item 1.
- **Semantic defect:** an active object's gain (−49…+15 dB or −∞) never reaches the ADM; a muted or attenuated object plays at full level in an ADM consumer that applies gain.
- **Current behaviour:** `<gain>0.0</gain><importance>0</importance>` only when `!s.active`; `adm_equal` ignores gain, so a gain-only change is popped (trailing) or duplicated (interior).
- **Intended behaviour (Dolby's observed encoding, `adm-harness.json` C02 `ct.adm_blocks`):**
  - `Gain::Db(0)` → no `<gain>` element;
  - `Gain::Db(db)` → `<gain>{}</gain>` with `format!("{:.10}", f64::from(10f32.powf(f32::from(db) / 20.0)))` (float32 arithmetic reproduces Dolby's ten decimals: 0.5011872053, 1.4125375748, 0.2511886358);
  - `Gain::MinusInfinity` on an active object → `<gain>0.0</gain>` and **no** `<importance>` (Dolby's C02 object 3); the inactive marker (`gain 0.0` + `importance 0`, position kept) stays as it is;
  - `adm_equal` compares `gain` too, so a gain change produces a new block (Dolby emits a block for the −12 dB change at 48000).
- **Classification:** interoperability-driven. Profile table 11 says gain appears only for inactive objects; Dolby's Conversion Tool and DEE write it on active objects, `atmos_info` 5.7.2 `--validate` and 1.1 accept such files (`adm-dolby-reference.json` `validate.ct-S2`: exit 0/0/0). The audit's profile checker flags these blocks (`profile-gain-on-active` / `profile-inactive-encoding`) on Dolby's output and will flag ours identically; that is expected and recorded.
- **Regression tests:** `adm::tests::active_gain_is_written_as_dolby_prints_it` (states −6 dB, +3 dB, −∞, 0 dB → exact strings above; 0 dB → no `<gain>`; −∞ active → `<gain>0.0</gain>` and no `<importance>`); `adm::tests::a_gain_only_change_gets_its_own_block` (0 dB then −12 dB at 48000 → two blocks, the second with `<gain>0.2511886358</gain>`); `adm::tests::the_inactive_marker_is_unchanged`; `adm_roundtrip.rs::gain_survives_the_file` (write, walk, parse the gain back).
- **Existing evidence:** `adm-harness.json` C02 (`loss.gain 3`, `trailing_popped 1` vs Dolby's 4 gain blocks), `adm-gain-diff.json`, `adm-dolby-reference.json` S2 (`gain_present 4` of 5 objects).
- **Independent validation:** harness C02 → toolkit ledger `loss.gain == 0`, classes `matched 5` (the change now a block); Conversion Tool `-f atmos` read-back of the new C02 ADM returns `gain: -6 / 3 / -inf / -12` per element (compare with the read-back of Dolby's own C02 ADM); `atmos_info 5.7.2 --validate 1` accepts the tagged file; EAR renders of oadec's and Dolby's C02 ADM at 0+5+0 are sample-identical (EAR applies block gain); S2 stimulus decoded through the harness path compared with `ct` blocks: identical gain strings.
- **Risks:** none for the corpus (every gain is 0 dB: `adm-gain-diff.json` `real_material`), byte-identity gate proves it. A consumer that implements the profile text literally could read a gain-only element as an inactive marker; Dolby's own files already carry the pattern.
- **Commit:** C2 `adm: write the gain of active objects as Dolby's converters do`.

### R3. ISF elements (D2)

- **Affected:** `adm.rs::create` and `damf.rs::create` (ISF policy), `program.rs` (unchanged mapping, ledger), `cli/damf.rs::program_from_major_sync` (ISF count from the major sync), `cli/main.rs` (`--isf`), `oadec-emdf` read-only (`ISF_OBJECTS`, `ProgramAssignment::isf_objects`).
- **Defect:** D2 (P1); backlog item 3.
- **Semantic defect:** ISF objects (TS 103 420 table 11b: SR3.1.0.0 = 4 objects … SR15.9.5.1 = 30) lose audio and metadata silently in both formats; the TrueHD driver additionally hardcodes the count to 0, so a TrueHD stream with ISF would mis-assign the 16-channel presentation (bed, ISF, dynamic order).
- **Current behaviour:** `Slot::None` for ISF elements, `element_id() == None`, no message, exit 0.
- **Intended behaviour:** `IsfPolicy::Error` (default): `AdmWriter::create` / `DamfWriter::create` return `IsfNotRepresentable { count, isf_type }`; the CLI prints `the programme carries 4 intermediate-spatial-format objects (SR3.1.0.0), which DAMF and the Dolby Atmos Master ADM profile cannot represent; pass --isf drop to write the beds and dynamic objects without them` and exits 2. `IsfPolicy::Drop`: today's layout, `IsfDropped` counted with the count, exit 4. TrueHD: `isf_objects = if extra.has_isf() { ISF_OBJECTS[usize::from(extra.isf_index & 7)] } else { Some(0) }`, a reserved index (6, 7) is an error; the OAMD/major-sync mismatch warning already present stays.
- **Why no positioned mapping:** TS 103 420 v1.2.1 gives only the ring composition (M/U/L/Z counts) of each ISF type, not azimuths or elevations; no Dolby reference output for ISF exists in the corpus; positioning ISF objects would be an invention. Recorded as UNK/DEFERRED (section 3).
- **Classification:** explicit failure / declared loss; profile-conformant output either way.
- **Regression tests:** `adm::tests::isf_elements_are_refused_by_default_and_dropped_on_request` (program `isf_objects: 4`: `create` → `Err(IsfNotRepresentable{count:4,..})`; with `Drop` → 12 channels, ledger `IsfDropped 4`); same for `damf::tests`; `cli/damf.rs::tests::the_isf_count_comes_from_the_major_sync` (an `ExtraChannelMeaning` with `content_description` bit 1 set, `isf_index 0` → `isf_objects 4`; `isf_index 6` → error); harness C13 through `run_remediation.py`: default exit 2 with the message, `--isf drop` exit 4 and the ISF tones absent as before.
- **Existing evidence:** `adm-harness.json` C13 (`adm_track_tones` without 300–360 Hz, no diagnostic), audit section 18.
- **Independent validation:** harness C13 both policies; TrueHD path: a unit test only (no ISF stream exists; N/T on real material stays).
- **Risks:** any stream carrying ISF that decoded silently before now stops with exit 2 unless `--isf drop` is given; no corpus stream is affected (0 ISF streams in 67 JOC and all TrueHD titles scanned by the previous audits).
- **Commit:** C3 `objects: refuse ISF elements unless --isf drop; derive the TrueHD ISF count from the major sync`.

### R4. Three-axis size (D5)

- **Affected:** `program.rs::{ObjectState, object_state}`, `damf.rs::object_fields`, `adm.rs::axml`, `cli/author.rs`, harness.
- **Defect:** D5 (P2); backlog item 4.
- **Semantic defect:** OAMD `object_size_idx == 2` carries width, depth and height; the model keeps the first axis and both writers re-emit it, silently.
- **Current behaviour:** `size: r.size[0]`.
- **Intended behaviour:** `ObjectState.size: [f32; 3]`; both writers write `uniform_size() == size[0]` (the width, as today, so corpus output is unchanged) and count `SizeAxesCollapsed` when the axes differ, with the example carrying the three values; `adm_equal` and the DAMF delta comparison use `uniform_size()` so a depth- or height-only change does not create a block or a DAMF line (it is counted instead).
- **Why width:** DAMF has one `size`; profile table 11 requires identical width/depth/height; the renderer's single "size" control is width-like; no Dolby reference carries a 3-D size to compare against. The axis choice remains an implementation choice and is documented (decision 7.4).
- **Classification:** approximation, profile-conformant output.
- **Regression tests:** `program::tests::three_size_axes_reach_the_state` (`RenderInfo.size = [0.2, 0.5, 0.8]` → `state.size == [0.2, 0.5, 0.8]`, `uniform_size() == 0.2`); `adm::tests::size_axes_that_differ_are_counted_and_the_width_is_written` (`<width>0.2000000030</width>` three times, `SizeAxesCollapsed 1`); `damf::tests::size_is_written_from_the_width_and_axes_that_differ_are_counted`; harness C04.
- **Existing evidence:** `adm-harness.json` C04 (`damf_states` size 0.2 for [0.2,0.5,0.8]).
- **Independent validation:** harness C04: ADM/DAMF unchanged (byte-identical to the audited C04 output except the stderr line), ledger line `1 event with differing size axes (object 10 at 0: 0.2/0.5/0.8, written 0.2)`.
- **Risks:** API break for `ObjectState` literals (five sites, all in-repo plus the harness); no output change.
- **Commit:** C4 `program: carry all three size axes; write the width and say when the axes differ`.

### R5. Sample rate (D6)

- **Affected:** `adm.rs::create` (guard), `cli/main.rs` (`--adm-allow-non-profile-rate`).
- **Defect:** D6 (P2); backlog item 5.
- **Semantic defect:** profile table 23 requires `sampleRate 48000`; a 96 kHz or 44.1 kHz programme is written with the stream rate and no warning; the 250-sample constant is silently scaled (0.002604 s at 96 kHz).
- **Intended behaviour:** `sample_rate != 48_000` → `AdmError::NonProfileSampleRate(rate)` (CLI exit 2, message `the Dolby Atmos Master ADM profile requires 48 000 Hz; this programme is 96 000 Hz; pass --adm-allow-non-profile-rate to write it anyway`) unless `allow_non_profile_rate`, in which case the file is written as today and `NonProfileSampleRate` is counted (exit 4). DAMF is not affected (DAMF carries any `sampleRate`).
- **Classification:** profile-conformant guard; the override is a declared non-profile output.
- **Regression tests:** `adm::tests::a_non_profile_sample_rate_is_refused_unless_allowed`; harness C15 default (exit 2) and with the override (file identical to the audited C15 output, exit 4).
- **Existing evidence:** `adm-harness.json` C15 (`profile-sample-rate` findings, no warning).
- **Independent validation:** harness C15 both ways; the audit's profile checker still reports `profile-sample-rate` on the override output (expected, recorded).
- **Risks:** the 44.1 kHz test encode `hires/pi-441.thd` now needs the flag for `--format adm`; no 96 kHz Atmos exists in the library.
- **Commit:** C5 `adm: refuse programmes that are not 48 kHz unless --adm-allow-non-profile-rate`.

### R6. Object IDs with `--no-bed-conform` (D7)

- **Affected:** `adm.rs::axml` (`object_id` closure), `adm.rs::create` (object limit).
- **Defect:** D7 (P2); backlog item 6.
- **Semantic defect:** profile table 17 numbers `Objects` audioObjects `AO_100b…AO_1080`; with fewer than ten bed channels oadec starts at `AO_1002`; Dolby starts at `AO_100b` regardless (S11).
- **Intended behaviour:** `AO_{0x100b + k - 1}` for object k (1-based) independent of the bed; bed stays `AO_1001`; `AdmError::TooManyObjects { objects, max: 118 }` when `dynamic_objects > 118` or `bed.len() + dynamic_objects > 128` (profile limits, table 17 / general limits). Channel, pack, stream, track and UID IDs are already bed-independent and stay.
- **Classification:** profile-conformant, Dolby-identical.
- **Regression tests:** `adm::tests::objects_are_numbered_from_ao_100b_whatever_the_bed` (LFE-only bed, `bed_conform: false` → `AO_100b`, `AO_100c`; conformed bed → unchanged `AO_100b`); `adm::tests::more_than_118_objects_is_an_error`; the existing `writes_a_profile_shaped_file` keeps asserting `AO_100b`.
- **Existing evidence:** `adm-harness.json` C11/C12/C21b (`profile-id` findings), `adm-dolby-reference.json` S11 (`AO_100b…AO_1015`).
- **Independent validation:** harness C11, C12, C21b → profile checker `profile-id` count 0; `audioContent` references and `chna` chain still resolve (toolkit `check_references`); S11 stimulus: AO list equal to Dolby's.
- **Risks:** default-conform output unchanged (bed 10 → `AO_100b` already); `--no-bed-conform` files change IDs (intended); anyone diffing old nbc files sees the renumbering.
- **Commit:** C6 `adm: number objects from AO_100b regardless of the bed; refuse more than 118 objects`.

### R7. Block tiling: sort, clamp, keep the first event's time (D8, D9, D10)

- **Affected:** `adm.rs::axml` (event preparation loop), `damf.rs::push_event` (out-of-order counting only).
- **Defects:** D8 (P3) overrun past the end, D9 (P3) out-of-order overlap and lost state, D10 (P3) late first event; backlog items 7 and 8.
- **Current behaviour:** events in arrival order; `next <= pos → continue` drops a state; the last block's `next` is `frames` only when it is the last event; a first event after 0 is copied to 0 and the original absorbed.
- **Intended behaviour (per object, in `axml`):**
  1. stable-sort the object's events by `sample_pos` (Dolby's C09 output: blocks 0/24000/48000);
  2. drop events with `sample_pos >= frames` (count `EventBeyondEndDropped`); the last block therefore ends at `frames` (Dolby's C07 output);
  3. among events at the same position keep the last (count `SamePositionSuperseded`; unchanged semantics);
  4. if the first event is after 0, insert the synthetic hold block `[0, t1)` with the first state **and keep the real first event as its own block** (`LateFirstEventHeld` counted); the trailing-pop rule never removes the first real event. Dolby's active default block at (0,0,0) is deliberately not adopted (decision 7.6).
  DAMF: events stay in arrival order (the file is streamed and `--all-events` can produce millions); an out-of-order event is counted (`OutOfOrderWrittenAsIs`, declared loss, exit 4) so it can no longer pass silently.
- **Classification:** 1–3 profile-conformant and Dolby-identical; 4 approximation (differs from Dolby, keeps the arrival time without inventing a position).
- **Regression tests:** `adm::tests::events_are_sorted_and_the_last_block_ends_at_the_programme_end` (events 0, 48000, 24000, `frames 96000` → rtime/duration 0/24000, 24000/24000, 48000/48000; an event at 96000 and 96040 dropped, `EventBeyondEndDropped 2`); `adm::tests::a_late_first_event_keeps_its_own_block` (first event 96000 → blocks (0,96000) and (96000, frames−96000), `LateFirstEventHeld 1`); `adm::tests::two_events_at_one_sample_keep_the_last`; `damf::tests::out_of_order_events_are_written_as_delivered_and_counted`.
- **Existing evidence:** `adm-harness.json` C07 (`overrun_objects 1`, `last_end 96040`), C09 (`overlaps 1`, `unexplained_missing 1`), C06 (`absorbed_by_synthetic 2`), with Dolby's block lists for each.
- **Independent validation:** harness C07/C09 → toolkit tiling `overrun_objects 0`, `overlaps 0`, `unexplained_missing 0`, block lists equal to Dolby's `ct.adm_blocks`; C06 → classes `synthetic_block0 2`, `absorbed_by_synthetic 0`, `matched 2`; pi whole-film and the 24 clips: `tile_dec`, gaps, overlaps unchanged (byte-identity gate).
- **Risks:** none on the corpus (0 out-of-order events, every object starts at 0 and ends at `frames`).
- **Commit:** C7 `adm: sort events, end the last block at the programme end, keep a late first event's own block`.

### R8. `--presentation` with the object formats; DAMF `fps` (D13)

- **Affected:** `cli/main.rs` (`presentation: Option<usize>`, `--fps`), `cli/decode.rs` (default 2 when `None`), `cli/damf.rs`, `damf.rs::DamfOptions.fps` validation.
- **Defect:** D13 (P3); backlog item 10.
- **Semantic defect:** `-p 0/1/2` is accepted and ignored with `--format damf|adm`; the `.atmos` header always says `fps: 24`.
- **Intended behaviour:** `--presentation` given with an object format and ≠ 3 → `error: --presentation 2 does not apply to --format adm: the object formats always decode the object presentation (3)` (exit 2); `--presentation 3` accepted; absent → 3. wav/pcm keep default 2. `--fps` accepted values `23.976, 24, 25, 29.97, 30` (Dolby's Conversion Tool accepted 23.976 in S10; the others are the DAMF frame rates the Dolby tools list), default `24`, written verbatim into the header; anything else is a usage error. Event timing stays sample-based (the audit measured 0 drift; `fps` is header data only).
- **Classification:** CLI correctness; DAMF header option.
- **Regression tests:** `cli_adm::presentation_is_refused_with_the_object_formats` (exit 2 and the message on the fixture); `cli_adm::the_default_presentation_still_decodes_wav`; `damf::tests::fps_is_written_verbatim_and_validated`.
- **Existing evidence:** audit section 19 D13 (INFERRED from the previous audit, `cli/damf.rs:205`).
- **Independent validation:** Conversion Tool `-f wav` on a DAMF written with `--fps 23.976` (S10 showed acceptance); `atmos_info` on the same.
- **Risks:** users who passed `-p 3` explicitly keep working; users who passed `-p 0/1/2` with an object format now get an error (they were being ignored).
- **Commit:** C8 `cli: --presentation is refused with the object formats unless it is 3; --fps for the DAMF header`.

### R9. Unrepresentable semantics (D12) — reporting only

Covered by R1 (`DistanceDropped`, `DivergenceDropped`, `WarpModeDropped`, `TrimConfigDropped`, `ScreenReferenceDropped`, `TrimBypassDropped`). No representation is added: DAMF has no field for distance, divergence, warp or trim configurations; the profile forbids `absoluteDistance`, `objectDivergence`, `screenRef`. Classification: **INTENTIONAL-PROFILE / N/T** (no corpus stream specifies any of them). A non-profile BS.2076 mode carrying `objectDivergence` and `absoluteDistance` is DEFERRED (section 3).

### R10. Opt-in real interpolation length (priority 9)

- **Affected:** `adm.rs` (`Interpolation`, `axml`, `adm_equal`), `cli/main.rs` (`--adm-interpolation`), `cli/damf.rs` (refuse with `--dolby-origin-tag`, marker).
- **Backlog item:** 9.
- **Semantic defect addressed:** with `profile`, the OAMD trajectory is approximated by 250-sample ramps (measured: up to 1.67 room units inside a block; 6.9–63 dB SDR rendered against the real ramps). This is **not** a defect of the default output (INTENTIONAL-PROFILE); the mode exists for consumers that honour BS.2076 interpolation.
- **Intended behaviour (`Interpolation::Real`):** block index 0 keeps `interpolationLength="0.000000"`; later blocks write `format!("{:.10}", f64::from(state.ramp) / f64::from(rate))` (ten decimals so 32 samples round-trip exactly: 0.0006666667 × 48000 = 32.00000016); `adm_equal` also compares `ramp`, so ramp-only changes become blocks and trailing ramp-only events are kept; `RampReplaced` is not counted. The output is marked non-profile: the `dbmd` tool string becomes `oadec <version> (non-profile: real interpolation lengths)`, stderr prints `ADM BWF written outside the Dolby Atmos Master ADM profile: interpolation lengths are the source ramps`, and `--dolby-origin-tag` is refused with it (exit 2). The XML is otherwise unchanged (no BS.2076-2 elements are added, so Dolby's readers still parse it; they return `rampLength 0` for any value, as the audit showed).
- **Classification:** explicit non-profile extension, opt-in, never default.
- **Regression tests:** `adm::tests::real_interpolation_writes_the_ramp_and_keeps_ramp_only_changes` (ramps 1536 and 32 → `interpolationLength="0.0320000000"` and `"0.0006666667"`, a trailing ramp-only event kept, ledger without `RampReplaced`); `adm::tests::profile_interpolation_is_the_default_and_unchanged` (the existing `"0.005208"` assertion); `cli_adm::real_interpolation_refuses_the_dolby_origin_tag`.
- **Existing evidence:** `adm-interpolation-diff.json`, `adm-render-comparison.json` (`oadec-realramps` variants), `adm-negative-controls.json` M-A6/M-T1, `adm-dolby-reference.json` reverse (`ramp_back_hist {0: …}` for every input).
- **Independent validation:** pi-head50m `--adm-interpolation real` → toolkit ledger `loss.ramp == 0` and trajectory evaluator `max_e == 0` on every object (the M-T1 criterion); EAR render at 0+2+0 / 0+5+0 / 4+7+0 compared with the audit's `oadec-realramps` render: sample-identical when the audit variant is regenerated at ten decimals, otherwise SDR ≥ 60 dB; Conversion Tool read-back returns `rampLength 0` (known limitation, recorded); `atmos_info` not run (the file is untagged by rule).
- **Risks:** none for default output; the mode must never be combined with the Dolby tag (enforced).
- **Commit:** C9 `adm: opt-in real interpolation lengths, marked non-profile`.

### R11. CI end-to-end coverage of `--format adm` (priority 8)

- **Affected:** `crates/oadec-spatial/tests/adm_roundtrip.rs` (new), `crates/oadec-cli/tests/cli_adm.rs` (new), `crates/oadec-cli/tests/fixtures/` (new, decision 7.10), `crates/oadec-cli/tests/real.rs`, `.github/workflows/ci.yml`.
- **Backlog item:** 12.
- **Gap:** no test runs the ADM writer end to end; CI never executes `--format adm`.
- **Intended coverage:**
  1. **Writer-level, no media (always in CI):** `adm_roundtrip.rs` builds a `Program` (LFE bed + 3 objects) with events covering gain, size, snap, zones, a late first event, an event beyond the end and out-of-order events, writes ADM and DAMF with `AdmWriter`/`DamfWriter`, then re-reads the ADM with a ~120-line independent walker written in the test (RIFF chunks, `fmt`, `data`, `chna` entries, `axml` text) and asserts: chunk order and sizes, `chna` numTracks == channels, track UID ↔ pack references, block rtimes/durations tile to `frames`, gain strings, `AO_100b`, and that the `data` samples equal the DAMF CAF samples track for track.
  2. **CLI-level, no media (always in CI, needs the fixture):** `cli_adm.rs` runs `env!("CARGO_BIN_EXE_oadec") decode --format adm` and `--format damf` on `tests/fixtures/authored-scene.ec3` (JOC) and `.mlp` (TrueHD), both 2 s encodes of one `atmos-author` scene (7 static positions + one moving object + one −6 dB object), asserts exit codes, the loss summary line, byte-equality of PCM between the two outputs, block count, `AO_100b`, and the guards (`--presentation 2` refused, `--adm-interpolation real --dolby-origin-tag` refused).
  3. **Media-gated (local, `OADEC_MEDIA`):** `real.rs::pi_head_adm_is_byte_identical_to_the_audited_file` decodes `clips/pi-head50m.thd` to ADM and asserts SHA-256 `5abe2e85…` (or, if the crate version differs from 0.2.0, equality of every chunk except `dbmd` plus the audited `data`/`axml` sizes 319 775 400 / 81 695).
  4. **Toolkit job (CI, ubuntu):** `adm-toolkit`: set up Python 3.12 + numpy, run `python -m unittest discover -s selftest -t .` in `docs/audit/adm/tools` (134 tests), build the harness with `CARGO_TARGET_DIR=target/harness`, run `docs/audit/adm-remediation/tools/run_remediation.py --no-dolby` on cases C01–C13, C15–C21b and the R-cases, and assert the expectations table (ledger classes, tiling, profile findings). This exercises oadec's writers through files and an independent parser without any Dolby tool.
- **Classification:** test infrastructure.
- **Independent validation:** the CI run itself; the fixture's provenance (scene JSON, DEE job XML, DEE version, hashes) committed next to it.
- **Risks:** fixture size (target ≤ 1 MB total); Python in CI adds ~1 minute; decision 7.10 governs whether encoded fixtures may live in the repository.
- **Commit:** C10 `tests: end-to-end coverage of the object outputs in CI`.

### R12. Documentation, evidence and report

- README (flags, exit 4, gain/ID/ISF behaviour, real-ramp mode), `docs/exit-codes.md` (code 4 table row and policy), `docs/dolby-tools.md` (gain on active objects is Dolby practice; validators accept), CHANGELOG `[Unreleased]` (Fixed: D1, D2, D7, D8–D10, D13; Added: loss ledger/exit 4, `--isf`, `--adm-interpolation`, `--adm-allow-non-profile-rate`, `--fps`, `--loss-report`; Changed: `ObjectState.size`), `docs/audit/adm-remediation-report.md` with per-defect before/after and the validation results, evidence under `docs/audit/evidence/adm-remediation/` (harness outputs summarised, Dolby/EAR runs, byte-identity table for the 25 inputs, CI run id).
- **Commit:** C11 `docs: ADM remediation report, evidence and user documentation`.

---

## 3. Items that remain as they are

| Item | Status | Reason |
|---|---|---|
| 250-sample interpolation length by default (D4) | **INTENTIONAL-PROFILE** | profile table 11; Dolby CT/DEE identical; Dolby's reader returns 0 for any value. Reported by R1, optional real mode R10. |
| Importance omitted on active objects; bed events and static bed gain not carried (D11) | **INTENTIONAL-PROFILE** | table 11 / tables 9–10; Dolby identical (S3, S8). Counted by R1. |
| Inactive marker keeps the object's position (Dolby zeroes it, S5/C16) | **INTENTIONAL** | profile allows it; keeps more of the DAMF. |
| Distance, divergence, warp mode, trim configurations, screen reference, trim bypass (D12) | **INTENTIONAL-PROFILE / N/T** | no target field in DAMF or the profile; no corpus stream specifies them. Counted by R1. A BS.2076 non-profile mode for `objectDivergence`/`absoluteDistance` is DEFERRED. |
| Positioned ISF mapping | **UNK / DEFERRED** | TS 103 420 gives ring counts only; no Dolby reference. R3 refuses or drops explicitly. |
| `RF64` FourCC instead of BS.2088 `BW64` (D14) | **DEFERRED / UNK** | BS.2088 not obtainable; ffprobe, bwf_info, EAR's reader and Dolby's tools accept RF64. Revisit when the text is available. |
| `bed_mask_bit` collision for Lw/Rw/LFE2 (D14) | **DEFERRED** | unreachable: `bed_profile` refuses those channels first; the dbmd bed-mask semantics are proprietary (UNK). A unit test pins the refusal (R6 commit). |
| Bed configurations beyond the 7.1.2 label set (C14) | **INTENTIONAL** | explicit `UnsupportedBedChannel` error already; 5.1 `Ls/Rs` labelling stays N/T (no material). |
| JOC 256-sample decoder delay, `Timeline` event semantics, all decoder behaviour | **out of scope** | preserved; the byte-identity gate proves it. |

---

## 4. Ordered implementation plan

Tasks are executed in this order after approval. Every task ends with `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace --locked` green and one commit. Test names are the ones listed in section 2.

### Task 1 (C1): loss ledger and exit status

**Files:** create `crates/oadec-spatial/src/loss.rs`; modify `lib.rs`, `program.rs`, `adm.rs`, `damf.rs`, `crates/oadec-cli/src/{integrity.rs,damf.rs,eac3_objects.rs,main.rs}`, `docs/exit-codes.md`.

- [ ] Write `loss::tests` (four tests of 1.1) and watch them fail to compile; implement `loss.rs`; green.
- [ ] Write `program::tests::unrepresentable_semantics_are_counted_by_the_timeline` using the existing `one_update` builder extended with `Distance::Factor(2.0)`, an `ExtendedObjectElement { divergence: Some(vec![vec![1.0]]), .. }` element and a `TrimElement { warp_mode: 1, .. }`; fail; add `pub losses: LossLedger` to `Timeline` and the counting in `push`; green.
- [ ] Write `adm::tests::profile_reductions_are_counted` and `bed_events_after_the_first_are_counted`; fail; add the ledger to `AdmWriter`, count in `push_event`/`axml`, return it in `AdmSummary` (drop `Copy`); green. Same for `damf::tests::isf_drop_is_counted` placeholder is **not** written here (R3); DAMF gets the ledger field only.
- [ ] `integrity.rs`: write `tests::a_declared_loss_is_lossy_and_a_fault_wins`; implement `note_losses`, `Verdict`, `report() -> Verdict`; update both drivers to merge `timeline.losses` with `summary.losses`, print `ledger.lines("adm"|"damf")`, and return the verdict; `main.rs` maps `Verdict::Lossy` to `ExitCode::from(4)`.
- [ ] `docs/exit-codes.md`: add row `4   the output was written but declares a loss (see the loss lines)`, the class table of 1.2 and the precedence rule 7 > 4 > 0.
- [ ] Commit: `spatial+cli: a loss ledger for the object outputs, printed per run; exit 4 for declared losses`.

### Task 2 (C2): gain

- [ ] Write `adm::tests::active_gain_is_written_as_dolby_prints_it`, `a_gain_only_change_gets_its_own_block`, `the_inactive_marker_is_unchanged`; fail.
- [ ] Implement:

```rust
/// The `<gain>` text of an active object, as the Dolby converters print it:
/// nothing at 0 dB, ten decimals of the float32 linear factor otherwise, `0.0` for −∞.
fn active_gain_text(g: Gain) -> Option<String> {
    match g {
        Gain::Db(0) => None,
        Gain::Db(db) => Some(format!("{:.10}", f64::from(10f32.powf(f32::from(db) / 20.0)))),
        Gain::MinusInfinity => Some("0.0".to_string()),
    }
}
```

  In `axml`, after `<cartesian>1</cartesian>`: if `!s.active` keep the inactive marker; else if `let Some(g) = active_gain_text(s.gain)` write `<gain>{g}</gain>`. Add `&& a.gain == b.gain` to `adm_equal`.
- [ ] Green; commit `adm: write the gain of active objects as Dolby's converters do`.

### Task 3 (C3): ISF policy and the TrueHD count

- [ ] Add `IsfPolicy { Error, Drop }` to `loss.rs` (re-exported); tests in `adm.rs`, `damf.rs`, `cli/damf.rs` (section 2 R3); fail.
- [ ] `AdmWriter::create` / `DamfWriter::create`: `if program.isf_objects > 0 { match options.isf { Error => return Err(IsfNotRepresentable { count, isf_type: isf_type_name(program.isf_index) }), Drop => ledger.note(IsfDropped, …) x count } }`. `Program` gains `pub isf_index: Option<u8>` (from `ProgramAssignment.isf_index`; the major sync's `isf_index` for TrueHD) so the message can name `SR3.1.0.0` etc. (table 11b names as a `const ISF_TYPES: [&str; 6]` in `program.rs`).
- [ ] `cli/damf.rs::program_from_major_sync`: derive `isf_objects` and `isf_index` from `extra` (reserved index → `bail!`); `main.rs`: `--isf` flag threaded through `Options` to both writers' options.
- [ ] Green; commit `objects: refuse ISF elements unless --isf drop; derive the TrueHD ISF count from the major sync`.

### Task 4 (C4): three-axis size

- [ ] Tests of R4; fail; change `ObjectState.size` to `[f32; 3]`, add `uniform_size()`, update `object_state`, both writers (`uniform_size()` for output and equality, `SizeAxesCollapsed` when axes differ), `author.rs`, the harness; green.
- [ ] Commit `program: carry all three size axes; write the width and say when the axes differ`.

### Task 5 (C5): sample-rate guard

- [ ] Test `a_non_profile_sample_rate_is_refused_unless_allowed`; implement the guard in `create` and `allow_non_profile_rate` in `AdmOptions`; `--adm-allow-non-profile-rate` in the CLI; green; commit.

### Task 6 (C6): object numbering and limit

- [ ] Tests `objects_are_numbered_from_ao_100b_whatever_the_bed`, `more_than_118_objects_is_an_error`, `unsupported_wide_and_second_lfe_channels_are_refused` (pins the `bed_profile` guard for Lw/Rw/LFE2); implement `let object_id = |k: usize| 0x100b + k - 1;` and the limit in `create`; green; commit.

### Task 7 (C7): tiling

- [ ] Tests of R7; implement the event preparation in `axml`:

```rust
let mut events: Vec<(u64, ObjectState)> = self.events.get(&element_id).cloned().unwrap_or_default();
events.sort_by_key(|(pos, _)| *pos);                       // stable: same-position order kept
let before = events.len();
events.retain(|(pos, _)| *pos < self.frames);              // nothing can start at or after the end
for _ in events.len()..before { ledger.note(LossKind::EventBeyondEndDropped, element_id, self.frames); }
// same position: keep the last
let mut deduped: Vec<(u64, ObjectState)> = Vec::with_capacity(events.len());
for e in events { if deduped.last().is_some_and(|(p, _)| *p == e.0) { ledger.note(SamePositionSuperseded, element_id, e.0); deduped.pop(); } deduped.push(e); }
let synthetic = deduped.first().is_none_or(|(pos, _)| *pos > 0);
if synthetic { /* insert (0, first state or default); if a real first event exists note LateFirstEventHeld */ }
// trailing pop never removes index 1 when index 0 is synthetic
```

  `next` becomes `deduped.get(n + 1).map_or(self.frames, |(p, _)| *p)`; the `next <= pos` guard stays as a debug assertion. `DamfWriter::push_event` tracks `last_pos: BTreeMap<u32, u64>` and notes `OutOfOrderWrittenAsIs`.
- [ ] Green; commit `adm: sort events, end the last block at the programme end, keep a late first event's own block`.

### Task 8 (C8): presentation and fps

- [ ] Tests of R8; `presentation: Option<usize>` in `Command::Decode` (`#[arg(short, long)]`), `decode::run` receives `presentation.unwrap_or(2)`, the object paths bail on `Some(p) if p != 3`; `--fps` validated against the five values and threaded to `DamfOptions.fps`; green; commit.

### Task 9 (C9): real interpolation

- [ ] Tests of R10; `Interpolation { Profile, Real }` in `AdmOptions`; `axml` writes the ramp in real mode; `adm_equal` takes the mode (compare `ramp` in real mode); `RampReplaced` only in profile mode; CLI flag, tag refusal, marker in the `dbmd` tool string and stderr; green; commit.

### Task 10 (C10): end-to-end tests and CI

- [ ] Author the fixture scene (`atmos-author` JSON committed under `crates/oadec-cli/tests/fixtures/authored-scene.json`), encode with DEE to `.ec3` (JOC, 448 kbps, 2 s) and `.mlp` (TrueHD), record provenance (`fixtures/README.md`: DEE version, job XML hashes, SHA-256 of both files, sizes); confirm total ≤ 1 MB.
- [ ] Write `crates/oadec-spatial/tests/adm_roundtrip.rs` and `crates/oadec-cli/tests/cli_adm.rs` (assertions of R11); run locally; add `real.rs::pi_head_adm_is_byte_identical_to_the_audited_file`.
- [ ] `.github/workflows/ci.yml`: add the `adm-toolkit` job; run locally with `act` or by pushing the branch (CI runs on pull requests).
- [ ] Commit `tests: end-to-end coverage of the object outputs in CI`.

### Task 11 (C11): validation runs, evidence, docs, report

- [ ] Update the harness for the new options and add R-cases (R-C02b gain change coalescing, R-C13 both ISF policies, R-C15 override, R-C23 119 objects, R-C24 real ramps); write `docs/audit/adm-remediation/tools/run_remediation.py` (imports `admaudit`, defines expectations); run with Dolby tools and EAR where section 2 names them.
- [ ] Re-decode the 25 audited inputs (default conform, and `--no-bed-conform` for pi-head50m) and compare hashes with `adm-work-inventory.json`; whole-film Pi optional (decision 7.13).
- [ ] Write `docs/audit/evidence/adm-remediation/*.json` (envelopes as the audit's `evidence.write`), `docs/audit/adm-remediation-report.md`, README/CHANGELOG/exit-codes/dolby-tools updates.
- [ ] Commit `docs: ADM remediation report, evidence and user documentation`; open a PR `adm-remediation → main` (normal merge, not automatic).

---

## 5. Test and validation matrix

| Fix | Unit / integration tests (CI) | Harness case(s) | Toolkit check | Dolby / EAR check | Regression gate |
|---|---|---|---|---|---|
| R1 ledger, exit 4 | loss (4), program (1), adm (2), integrity (1), cli_adm (1) | all | stderr lines present; exit 0 unless declared loss | — | byte-identity of all 25 outputs; `cargo test --workspace` |
| R2 gain | adm (3), adm_roundtrip (1) | C02, S2 replay | `loss.gain 0`, classes `matched 5` | CT read-back gains −6/+3/−inf/−12; `atmos_info` accepts tagged; EAR oadec = CT sample-identical at 0+5+0 | corpus (all 0 dB) unchanged |
| R3 ISF | adm (1), damf (1), cli/damf (1) | C13 error / drop | tones absent as before; exit codes 2 / 4 | — (no reference) | none affected |
| R4 size | program (1), adm (1), damf (1) | C04 | ledger line with 0.2/0.5/0.8; ADM/DAMF unchanged | CT unchanged (input identical) | corpus (size 0) unchanged |
| R5 rate | adm (1) | C15 default / override | exit 2 / exit 4; `profile-sample-rate` still flagged on override | — | 48 kHz corpus unchanged |
| R6 IDs | adm (3) | C11, C12, C21b, S11 | `profile-id 0`; references resolve | AO list equals Dolby's S11 | default conform unchanged |
| R7 tiling | adm (3), damf (1) | C06, C07, C09 | tiling clean; C07/C09 blocks equal Dolby's; C06 `absorbed 0` | CT block lists (recorded) | corpus tiling unchanged |
| R8 presentation/fps | cli_adm (2), damf (1) | — | — | CT/`atmos_info` accept `fps 23.976` DAMF | wav/pcm default unchanged |
| R10 real ramps | adm (2), cli_adm (1) | pi-head50m real | `loss.ramp 0`, trajectory `max_e 0` | EAR = audit `oadec-realramps` render (identical / ≥ 60 dB); CT read-back 0 (known) | default output unchanged |
| R11 CI | adm_roundtrip, cli_adm, real (gated), toolkit job | subset in CI | 134 self-tests + expectations | — | CI green on windows/ubuntu |

Acceptance for the whole branch: every row green, the byte-identity table complete (25/25), CI green, report written.

---

## 6. Expected commit sequence

| # | Commit | Scope | Tests added |
|---|---|---|---|
| C0 | `docs(audit): ADM remediation plan` | this file | — |
| C1 | `spatial+cli: a loss ledger for the object outputs, printed per run; exit 4 for declared losses` | R1 | 9 |
| C2 | `adm: write the gain of active objects as Dolby's converters do` | R2 | 4 |
| C3 | `objects: refuse ISF elements unless --isf drop; derive the TrueHD ISF count from the major sync` | R3 | 3 |
| C4 | `program: carry all three size axes; write the width and say when the axes differ` | R4 | 3 (+ harness update) |
| C5 | `adm: refuse programmes that are not 48 kHz unless --adm-allow-non-profile-rate` | R5 | 1 |
| C6 | `adm: number objects from AO_100b regardless of the bed; refuse more than 118 objects` | R6 | 3 |
| C7 | `adm: sort events, end the last block at the programme end, keep a late first event's own block` | R7 | 4 |
| C8 | `cli: --presentation is refused with the object formats unless it is 3; --fps for the DAMF header` | R8 | 3 |
| C9 | `adm: opt-in real interpolation lengths, marked non-profile` | R10 | 3 |
| C10 | `tests: end-to-end coverage of the object outputs in CI` | R11 | 2 files + 1 media test + CI job |
| C11 | `docs: ADM remediation report, evidence and user documentation` | R12 | — |

Each commit ends with the attribution lines required for this repository. C1–C9 are independently revertible; C4 is the only API-breaking one.

---

## 7. Decisions that need approval before implementation

1. **Exit code 4** for "written with declared loss", with the class table of 1.2 (profile reductions, unrepresentable semantics and approximations print but exit 0; declared losses exit 4; integrity 7 wins). Alternative: report everything, exit 0 always; or exit 4 also for approximations (size axes, late first event).
2. **ISF default = error**, `--isf drop` explicit; no positioned ISF mapping (ring geometry not in TS 103 420, no Dolby reference) → UNK/DEFERRED.
3. **Gain on active objects written by default**, Dolby's encoding (linear, ten decimals, float32 arithmetic, `0.0` alone for −∞); this follows Dolby's tools against the letter of profile table 11. Alternative: behind a flag (not recommended: the corpus is unaffected and Dolby's validators accept it).
4. **3-D size reduced to the width** (first axis) with a diagnostic; `ObjectState.size` becomes `[f32; 3]` (API break; harness and `atmos-author` updated). Alternative: maximum of the three axes.
5. **Non-48 kHz ADM refused by default**; `--adm-allow-non-profile-rate` writes it with exit 4. Alternative: warn and write (exit 4) without a flag.
6. **Late first event:** keep the hold-from-zero block but preserve the real first event's own block (arrival time visible), **not** Dolby's active default block at (0,0,0).
7. **Out-of-order events:** ADM sorts (Dolby-identical); DAMF writes as delivered and declares the loss (exit 4) instead of buffering the whole event stream.
8. **`--presentation` becomes optional** and is refused with the object formats unless 3; **`--fps`** limited to 23.976/24/25/29.97/30, default 24.
9. **Real-ramp mode** as `--adm-interpolation real`, marked by the `dbmd` tool string and stderr, refusing `--dolby-origin-tag`, ten-decimal seconds, first block 0.
10. **CI fixtures:** commit two ≤ 0.5 MB DEE-encoded clips of a synthetic `atmos-author` scene (`.ec3` JOC and `.mlp` TrueHD) under `crates/oadec-cli/tests/fixtures/` with provenance. Without them the CLI end-to-end test cannot run in CI and coverage stops at the library level plus the media-gated test.
11. **Audit tooling:** the harness crate (`docs/audit/adm/harness`) is updated in place to compile against the new API and gain the new knobs; `admaudit/*` stays untouched; new drivers and evidence live under `docs/audit/adm-remediation/` and `docs/audit/evidence/adm-remediation/`.
12. **Version:** keep `0.2.0` during the remediation so the byte-identity gate (which includes the `dbmd` tool string) holds; bump to `0.3.0` in a separate release commit after the report.
13. **Whole-film re-run:** re-decode Pi (2 × 3 min, 30 GB temporary) for the byte-identity table, or rely on the 24 clips plus pi-head50m.
14. **`--loss-report <json>`:** include the optional JSON dump of the ledger (recommended for CI and for the audit toolkit), or stderr only.

---

## 8. Self-review against the brief

- Priorities 1–9 map to R2, R3, R1, R4, R5, R6, R7+R8+R9, R11, R10. D1–D14 each appear in section 2 or section 3 (D4, D11, D12, D14 intentionally unchanged, with reasons).
- No task modifies decoder crates; every change is in `oadec-spatial`, `oadec-cli`, tests, CI and docs.
- Every fix names files and functions, the defect, current and intended behaviour, classification, tests, existing evidence, independent validation, risks and its commit.
- The default 250-sample behaviour is preserved and the real-ramp mode is opt-in and marked.
- The audit report, matrix and evidence are not edited; new evidence has its own directory.
- Type consistency: `LossLedger`, `LossKind`, `IsfPolicy`, `Interpolation`, `Verdict`, `ObjectState.size: [f32; 3]`, `uniform_size()`, `active_gain_text` are used with the same names throughout.
