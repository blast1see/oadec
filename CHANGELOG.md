# Changelog

All notable changes to `oadec` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
Semantic Versioning.

## [Unreleased]

### Added

- `oadec-bits`: MSB-first bit reader with windowed peeking, the TrueHD
  CRC-8/CRC-16 polynomials and the parity helpers.
- `oadec-truehd`: access-unit framing and resynchronisation, major sync with
  the extra channel meaning, substream directory, extra data with Evolution
  containers, the complete substream syntax (restart header, block header,
  `0x31EA`/`0x31EB`/`0x31EC` matrices, FIR/IIR filters, Huffman and plain
  block data, segment terminator, parity and CRC), the sample decoder for
  presentations 0–3 with lossless checks, and the timing model with
  seamless-branch judgement and duplicate detection.
- `oadec-emdf`: EMDF/Evolution container parser and the Object Audio
  Metadata payload of ETSI TS 103 420 §5.5 (program assignment, object,
  trim and extended object elements; unknown elements skipped by size).
- `oadec-spatial`: program model and event timeline, DAMF writer
  (`.atmos`, `.atmos.metadata`, `.atmos.audio` as 24-bit CAF), ADM BWF
  writer (RIFF or RF64 with `axml`, `chna` and `dbmd` chunks).
- `oadec` command line: `info`, `verify [--json]`, `decode` to `pcm`,
  `wav`, `damf` or `adm`, `compare` against a raw PCM reference, `oamd`
  and `emdf` payload dumps.
- `tools/adm_diff.py`: structural comparison of two ADM BWF files.
- Documentation: format notes (`docs/truehd.md`, `docs/oamd.md`), the
  behaviour of the Dolby command-line tools (`docs/dolby-tools.md`) and
  the evidence report `docs/evidence/2026-09-07.md`.
