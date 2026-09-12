# ADM BWF semantic-fidelity audit: tools, harness and reproduction

Material for the audit reported in `docs/audit/2026-09-11-adm-semantic-fidelity-audit.md`
(matrix: `docs/audit/adm-conformance-matrix.md`, raw evidence: `docs/audit/evidence/adm/`).
Everything under `docs/audit/adm/` was written for the audit and imports nothing from
oadec's Rust code or from `tools/`. The production crates are untouched on the `adm-audit`
branch (`git diff main -- crates/ tools/ Cargo.toml Cargo.lock` is empty).

## Layout

```
docs/audit/adm/
  README.md                this file
  tools/admaudit/          the independent toolkit (Python 3.12, numpy + stdlib)
    riff.py                RIFF / RF64 / BW64 chunk walker, ds64, 24-bit track reader
    timecode.py            BS.2076 timecodes -> exact sample positions (Fractions)
    axml.py                ADM XML parser, reference graph, chna chain
    damf.py                .atmos / .atmos.metadata reader, delta -> full-state reconstruction
    caf.py                 CAF 24-bit big-endian reader
    normalise.py           common scene schema for DAMF and ADM (effective / strict values)
    compare_events.py      the reconciliation ledger (every state and block gets one class)
    trajectory.py          what a parameter does over time under DAMF and ADM rules
    compare_pcm.py         per-track and interleaved one-pass PCM comparison
    mutate.py              defect injection for the negative controls (ADM and DAMF)
    profile.py             Dolby Atmos Master ADM profile v1.0 rule checker (tables 9-23)
    dolby.py               wrappers for the Conversion Tool, DEE, atmos_info, bwf_info
    oamd_dump.py           parser for `oadec oamd --dump`
    provenance.py          run records (argv, exit, hashes, OADEC_* scrubbed), file records
    evidence.py            evidence envelopes and the manifest
  tools/selftest/          134 unit tests for the toolkit
  tools/run_*.py           drivers (one per audit stage, see below)
  tools/build_evidence.py  assembles docs/audit/evidence/adm/*.json from the work directory
  tools/matrix_rows.py     the matrix judgements (rows, scores, verdicts)
  tools/build_matrix.py    renders the matrix from the evidence + matrix_rows.py
  harness/                 Rust crate driving AdmWriter / DamfWriter with controlled events
```

## Environment

| Item | Value |
|---|---|
| oadec binary | `target/release/oadec.exe`, SHA-256 `292f5f91b7b26052957dc6dd83f778829400c8dd545de780e4f359e54147506b`, built from the production sources as merged at `796e022` (unchanged on this branch) |
| Python | 3.12.10 in the isolated venv `E:\oadec-work\audit\adm\venv` (system Python untouched) |
| Packages | numpy 1.26.4, lxml 4.9.4, scipy 1.17.1, ear 2.1.0, setuptools 79.0.1; full `pip freeze` in `evidence/adm/00-provenance.json` |
| Why setuptools < 80 | `ear-render` 2.1.0 imports `pkg_resources`, removed in setuptools 80 |
| Rust harness | `cargo build --release` in `docs/audit/adm/harness` with `CARGO_TARGET_DIR=E:/oadec-work/audit/adm/target` |
| Dolby tools (references only) | Dolby Atmos Conversion Tool 2.1.2, Dolby Encoding Engine 5.2.1 (`convert_atmos_mezz`), `atmos_info` 5.7.2 (`--validate 1`) and 1.1, `bwf_info`; hashes in `00-provenance.json` |
| Other | ffprobe (frame counts), EBU ADM Renderer 2.1.0 (`ear-render`) |
| Specifications | Dolby Atmos Master ADM Profile v1.0 (22 Jul 2019), ITU-R BS.2076-3, ETSI TS 103 420 V1.2.1, EBU Tech 3285 / 3306; BS.2088 and profile v1.1 were not obtainable (see `01-spec-extracts.json`, `spec_gaps`) |

Every decode was run with the `OADEC_*` environment variables cleared and recorded.

## Reproduction

All commands run from `docs/audit/adm/tools` with the venv's `python.exe`;
`W = E:/oadec-work/audit/adm` is the work directory (outside the repository).

```
# 0. toolkit self-tests (134 tests)
python -m unittest discover -s selftest -t .

# 1. decode one stream to DAMF and to ADM (default and --no-bed-conform)
python run_decode.py --oadec <oadec.exe> --input <stream> --out <base> --format damf
python run_decode.py --oadec <oadec.exe> --input <stream> --out <base> --format adm [--no-bed-conform]

# 2. the differential: DAMF (the writer's input) against the ADM (the writer's output)
python run_compare.py --damf <base> --adm <base>.wav --out compare.json --pcm full

# 3. batches over the TrueHD and JOC clips (inputs and hashes are in adm-batch-summary.json)
python run_batch_decode.py --oadec <oadec.exe> --out-root W/work/thd --inputs <clips...> --pcm full
python run_batch_decode.py --oadec <oadec.exe> --out-root W/work/joc --inputs <clips...> --pcm full

# 4. negative controls (22 injected defects + positive controls) on pi-head50m
python run_negative_controls.py --adm <pi-head50m.wav> --damf <pi-head50m> --work W/work/negative --out W/work/negative/negative-controls.json

# 5. Dolby reference leg (stimuli S0-S13 -> Conversion Tool / DEE -> read-back -> validators)
python run_dolby_reference.py --work W/work/f5 --stage stimuli
python run_dolby_reference.py --work W/work/f5 --stage ct
python run_dolby_reference.py --work W/work/f5 --stage dee
python run_dolby_reference.py --work W/work/f5 --stage reverse
python run_dolby_reference.py --work W/work/f5 --stage validate
python run_dolby_reference.py --work W/work/f5 --stage report

# 6. writer-level harness (cases C01-C22b; --ct also converts each DAMF with the Conversion Tool)
CARGO_TARGET_DIR=W/target cargo build --release --manifest-path ../harness/Cargo.toml
python run_harness.py --exe W/target/release/adm_harness.exe --work W/work/harness --ct

# 7. container: RF64 / ds64 / chunk layout, third-party frame counts
python run_container.py --adm <file.wav> --damf <base> --dolby --ffprobe --out container.json

# 8. raw OAMD timing and coordinates (TrueHD only)
python run_oamd_timing.py --oadec <oadec.exe> --stream <clip.thd> --damf <base> --adm <base>.wav --out oamd-timing.json

# 9. renderer leg (EBU ADM Renderer), with a real-ramp variant built from the DAMF
python run_render.py --work W/work/render/<scene> --out render-report.json --layouts 0+2+0,0+5+0,4+5+0,4+7+0 \
    --adm oadec=<oadec.wav> dolby-ct=<ct.wav> --realramps "oadec-realramps=<oadec.wav>|<damf base>"

# 10. whole-film Pi (>4 GiB RF64): decode, full PCM compare, container
python run_decode.py --oadec <oadec.exe> --input <pi.thd> --out W/big/pi/pi --format damf
python run_decode.py --oadec <oadec.exe> --input <pi.thd> --out W/big/pi/pi --format adm
python run_compare.py --damf W/big/pi/pi --adm W/big/pi/pi.wav --out W/big/pi/compare.json --pcm full --label pi-full-film
python run_container.py --adm W/big/pi/pi.wav --damf W/big/pi/pi --dolby --ffprobe --out W/big/pi/container.json

# 11. evidence envelopes and the matrix
python build_evidence.py --work W --repo <repo>
python build_matrix.py --repo <repo>
```

## Evidence files (`docs/audit/evidence/adm/`)

| File | Content |
|---|---|
| `00-provenance.json` | git state, oadec binary hash, tool versions and hashes, Python packages, specification availability |
| `01-spec-extracts.json` | the profile, BS.2076-3 and TS 103 420 clauses relied on, quoted |
| `adm-batch-summary.json` | every decoded input with hashes, exit codes, run records, ledger summary |
| `adm-object-pcm-compare.json` | per-track PCM comparison DAMF vs ADM, full pairing matrix |
| `adm-track-mapping.json` | chna -> audioTrackUID -> track -> stream -> channel -> object chain per file |
| `adm-reference-graph.json` | reference-graph and profile findings per file |
| `adm-event-timing.json` | ledger per input, timecode round trips, tiling, raw-OAMD cross-check |
| `adm-coordinate-diff.json` | position comparison per input |
| `adm-interpolation-diff.json` | ramp loss and trajectory deviation per input, evaluator self-check |
| `adm-gain-diff.json` | gain / importance presence and values per input |
| `adm-harness.json` | writer-level cases C01-C22b with the Conversion Tool cross-reference |
| `adm-dolby-reference.json` | stimuli S0-S13 through Dolby's converters, read-back, validators |
| `adm-negative-controls.json` | the 22 injected defects and what fired |
| `adm-container-validation.json` | RF64 / ds64 / chunk measurements, ffprobe, bwf_info, atmos_info |
| `adm-render-comparison.json` | EAR speaker-output comparisons |
| `adm-long-duration.json` | whole-film Pi: hashes, sizes, ledger, first/last events, drift |
| `adm-conformance-matrix.json` | the rendered matrix rows, scores, verdicts and the numbers used |
| `manifest.json` | id, title, class, results and hash of every evidence file |

## Large temporary files

The whole-film ADM (`pi.wav`, 15 278 169 042 bytes, SHA-256
`8ff5bbd3c8e1264a2e63a472a4695d7b7c9c00169a3dfc6009b74fef18d20e5e`), its DAMF, the two
RF64 threshold files of harness cases C22 / C22b and the rendered speaker WAVs were generated
on `E:` and deleted after the measurements were captured; the evidence files keep the hashes,
sizes, sample counts, chunk measurements, drift figures, first/last event positions and the
run records needed to reproduce every conclusion. No media is stored in the repository.
