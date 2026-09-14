# ADM remediation: validation driver and reproduction

Material for the remediation reported in `docs/audit/adm-remediation-report.md`
(plan: `docs/audit/adm-remediation-plan.md`; raw evidence:
`docs/audit/evidence/adm-remediation/`). The remediation closes the findings of the
2026-09-11 ADM semantic-fidelity audit (`docs/audit/2026-09-11-adm-semantic-fidelity-audit.md`).
The audit's reports, evidence and toolkit are unchanged; this directory adds to them.

## Layout

```
docs/audit/adm-remediation/
  README.md                 this file
  tools/run_remediation.py  the validation driver (five stages, expectations per case)
docs/audit/adm/tools/       the audit toolkit, imported read-only (admaudit, run_harness, run_compare, run_render)
docs/audit/adm/harness/     the writer harness crate, updated for the new writer API and options
docs/audit/evidence/adm-remediation/
  00-provenance.json                 git state, binary and tool hashes, Python environment
  adm-remediation-harness.json       27 writer-level cases with their expectations and results
  adm-remediation-byte-identity.json every audited decode replayed against adm-work-inventory.json
  adm-remediation-dolby.json         Conversion Tool read-back, validators, object numbering
  adm-remediation-render.json        EBU ADM Renderer comparisons
  manifest.json                      hashes of the files above
```

## What the measure is

The audit toolkit is the judge, not the writers' own tests: the same normaliser,
reconciliation ledger, tiling check, profile checker and PCM comparison that found the
defects now decide whether they are gone. `run_remediation.py` adds one thing on top, an
expectation per case: what the fix must have changed and what must have stayed. Two
readings of the ledger are deliberate and documented in the script:

- the ledger classifies a metadata state from the DAMF side, so a gain-only or ramp-only
  change is filed under `inexpressible_change` even when the block now carries the value;
  the value itself is checked by the ledger's `loss` counters (the toolkit compares a gain
  wherever the ADM carries one) and by the script block by block;
- the normaliser derives "inactive" from a bare zero gain, so Dolby's encoding of an
  active muted object (a zero `<gain>` with no `<importance>`, audit stimulus S2) draws
  one `value_mismatch` on `active`; exactly that one item is accepted, with its object,
  time and field pinned. The profile checker likewise flags every gain-bearing block
  (`profile-inactive-encoding`), as it did on Dolby's own S2 files.

## Stages

| Stage | What runs | What must hold |
|---|---|---|
| `harness` | the audit's cases C01-C21b (the two RF64 cases are the audit's: that code was not touched) plus `C13b-isf-drop`, `C15b-96k-allowed`, `C20b-real-ramps`, `R01-119-objects`, `R02-gain-tagged` through `adm_harness`, then `run_compare.compare` | every expectation in `expectations()`; the C07 and C09 block lists equal the Conversion Tool's from `adm-harness.json` |
| `regression` | the 51 decode run records embedded in `adm-work-inventory.json` (25 inputs; default and `--no-bed-conform`; `big/` only with `--with-full-film`) replayed with the new binary | every output hash and exit code as audited, except the one difference `EXPECTED_DIFFERENCES` names (R6 object numbering), which is verified against the unchanged DAMF and Dolby's S11 numbering |
| `dolby` | Conversion Tool 2.1.2: oadec's C02 ADM to DAMF (read-back), the C02 DAMF to ADM (Dolby's own gain strings) and that file back to DAMF; DEE 5.2.1 `convert_atmos_mezz` on the same DAMF (the second converter's strings); `atmos_info` 5.7.2 `--validate 1` and 1.1 and `bwf_info` on the tagged and untagged gain files; object IDs of the replayed `--no-bed-conform` ADM | read-back gains -6, +3, -inf and -12 dB; the Conversion Tool's gain strings equal oadec's; the validators accept the tagged file; the IDs equal Dolby's |
| `render` | EBU ADM Renderer 2.1.0 (`ear-render`): oadec's C02 against the Conversion Tool's C02 at 0+5+0 and 4+7+0; pi-head50m in `--adm-interpolation real` against the audit's real-ramp variant of the default file at 0+2+0 and 4+7+0; the toolkit's ledger and trajectory evaluator on the real-mode file | C02 renders identical; real mode: `loss.ramp` 0, no defect, trajectory error within 1e-6 room units (ten-decimal seconds round a 32-sample ramp by 2e-6 samples), renders identical or at least 60 dB SDR |
| `evidence` | envelopes for the four stages and a provenance record | written under `docs/audit/evidence/adm-remediation/` with a manifest |

## Environment

| Item | Value |
|---|---|
| oadec binary | `target/release/oadec.exe`, SHA-256 `0689ba53b797ae3109b660da2329c3d5a126b5aec66bc5260222a871dd4e961e`, built from the production sources of the `adm-remediation` branch (crate version 0.2.0 kept so the `dbmd` tool string and the byte-identity gate hold) |
| harness | `adm_harness.exe`, SHA-256 `d790600cbff043d74140c940fd6510c00361d17f7c167099567d63e965baeb5a`, `CARGO_TARGET_DIR=E:/oadec-work/audit/adm/target` |
| Python | the audit's venv `E:\oadec-work\audit\adm\venv` (3.12, numpy 1.26.4, ear 2.1.0); 136 toolkit self-tests pass |
| Dolby tools | Conversion Tool 2.1.2, `atmos_info` 5.7.2 and 1.1, `bwf_info`; hashes in `00-provenance.json` |
| Work directory | `E:\oadec-work\audit\adm\remed\val` (outside the repository; large outputs deleted after the evidence was written) |

Every decode runs with the `OADEC_*` environment variables cleared.

## Reproduction

```
# with the venv's python
(cd docs/audit/adm/tools && python -m unittest discover -s selftest -t .)   # 136 toolkit self-tests
CARGO_TARGET_DIR=<target> cargo build --release --manifest-path docs/audit/adm/harness/Cargo.toml
cargo build --release

# from the repository root

python docs/audit/adm-remediation/tools/run_remediation.py --repo . --work <W> --stage harness    --harness <adm_harness.exe> --oadec target/release/oadec.exe
python docs/audit/adm-remediation/tools/run_remediation.py --repo . --work <W> --stage regression --oadec target/release/oadec.exe [--with-full-film]
python docs/audit/adm-remediation/tools/run_remediation.py --repo . --work <W> --stage dolby      --oadec target/release/oadec.exe
python docs/audit/adm-remediation/tools/run_remediation.py --repo . --work <W> --stage render     --oadec target/release/oadec.exe --media E:/oadec-work
python docs/audit/adm-remediation/tools/run_remediation.py --repo . --work <W> --stage evidence
```

`--stage all` runs the five in order; `--no-dolby` and `--no-ear` skip the stages that need
the Dolby tools or the renderer, which is how CI runs the harness stage
(`.github/workflows/ci.yml`, job `adm-toolkit`). The driver exits 1 when any expectation
is not met.
