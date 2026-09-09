# Changelog

All notable changes to `oadec` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
Semantic Versioning.

## [Unreleased]

## [0.2.0] - 2026-09-10

### Fixed

- **The JOC mixing matrix was ten time slots out of step with the subband
  samples.** Clause 6.6.6 pairs slot `ts` of the samples with slot `ts` of
  the matrix and says nothing about the analysis bank in between. Measured
  against the object output of the Dolby decoder on three titles from three
  encoders, the matrix belongs with the samples the bank produces ten slots
  earlier, sharply: a slot either way costs more than 20 dB. Correcting it
  takes the residual against Dolby from about -15 dB to below -70 dB in the
  bands where the core decode is itself exact.
- **The 90-degree phase shift of downmix configurations 3 and 4 is not a
  rotation at the bottom of the band.** Rotating every subband by -j is right
  above 141 Hz and wrong below it, because subband 0 straddles direct current
  and the image of a real signal's negative frequencies falls inside its
  passband; the objects lost up to 14 dB under 50 Hz. The operator Dolby uses
  was measured (the identity at direct current, -j by 141 Hz, the same on
  every title) and is applied as a 37-tap filter across time slots.
  `--flat-quadrature` restores the plain reading for measurement.
- **The dither was 3 dB louder than Dolby's.** ATSC A/52 clause 7.3.4 offers
  0,707, 0,75 and 0,5 as scalings and leaves the sequence to the
  implementation. Which one Dolby uses is measurable without their sequence:
  the power ratio between their dither and ours is 2,00 on fifteen channels
  of three streams, so theirs is 0,5. Matching it is worth 1,76 dB on every
  channel of every stream.
- Together the three above take the per-object distance to the Dolby
  decoder from 11-15 dB to 40-56 dB, which is the floor the unshared
  dither sets. `docs/evidence/2026-09-09-c.md` has the measurements.
- **The two metadata commands reported nothing instead of refusing.** `oadec
  oamd` walks TrueHD access units and an E-AC-3 file has none, so it printed
  nought payloads and called the result clean; `oadec emdf` walks E-AC-3
  frames and answered a TrueHD file with nothing but sync errors. Each now
  says what the file is and points at the other. `emdf` also counts the frames
  it could not parse rather than passing over them.
- **`oadec emdf` missed half the EMDF containers and invented errors.** It
  hunted the sync word in the raw frame bytes, so it found only the containers
  that happened to land on a byte boundary: 480 of 976 on one stream, 208 of
  1250 on another, with twenty-odd "malformed container" reports that were
  false syncs in audio data. It now parses the frame and reads the skip
  fields, where the containers are, and agrees with `verify` to the container.
  A new opt-in test holds the two to the same count.
### Added

- **Enhanced coupling** (ATSC A/52:2018 clause E.3.5.5) decodes end to end.
  ETSI TS 102 366 V1.4.1 reduces the tool to a real-valued gain and marks the
  angle and chaos fields reserved; V1.2.1 of the same document and both ATSC
  editions carry the full complex process and count the coordinate field nine
  bits longer. Both Dolby decoders on hand return bit-identical audio whether
  the angle and chaos carry zeros or a full spread, so the amplitude-only
  reading is the default and `--ecpl-spec` selects the ATSC one.
- **Transient pre-noise processing** (clause E.3.7) is applied. The correction
  reads across the previous frame and can be aimed at a transient in a later
  one, so decoded samples are released only once no future frame can rewrite
  them; frames still come out whole, in order and the same length.
  `--no-tpnp` keeps the old behaviour.
- **JOC clip gain** (ETSI TS 103 420 clause 6.3.3.2) is applied to the object
  program. Encoding one Atmos master at two levels shows the encoder divides
  the whole downmix, LFE included, by it. `--no-clip-gain` keeps the old
  behaviour.
- `tools/gen_joc_quadrature.py`, which measures the low-band quadrature
  operator against the Dolby object decoder and prints the table.
- `tools/sweep.py` records each JOC track's downmix configuration, object
  count and clip gains, which is how the one configuration 0 stream in the
  library was found.
- `oadec eac3-ecpl-inject`, which rewrites a stream's standard coupling as
  enhanced coupling so a tool nothing emits can be tested, and `oadec-bits`
  gained the `BitWriter` it needs.
- `verify --json` reports `ecpl_frames`, `tpnp_frames`, the transient
  parameters and the frames that carry a clip gain.
- `tools/two_way.py` compares a decode with the Dolby one where FFmpeg cannot
  follow, and `tools/tpnp_window.py` measures what the transient correction
  changes.
- `oadec thd-demux` splits a Blu-ray audio dump that interleaves TrueHD access
  units with the AC-3 core frames of the same track. Such a file is refused
  outright by FFmpeg and MediaInfo; the split is exact because both streams
  carry their own length.
- The stream sniffer looks for a TrueHD major sync with an access-unit chain
  behind it, so a dump that opens with an AC-3 core frame is no longer routed
  to the E-AC-3 decoder.


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
- `oadec-eac3`: AC-3 and Enhanced AC-3 core decoder from ETSI TS 102 366
  (both syntaxes, coupling, rematrixing, spectral extension, the adaptive
  hybrid transform with vector and gain-adaptive quantization, delta bit
  allocation, block switching, dither), with the skip fields captured for
  EMDF. Enhanced coupling is parsed but not decoded; transient pre-noise
  processing is parsed but not applied.
- `oadec-joc`: the JOC side information (ETSI TS 103 420 clause 6), the
  64-band complex QMF bank (clause 7) and the object reconstruction, with
  the -j rotation of the surround pair for the phase-shifted downmix
  configurations.
- `oadec` command line: `info`, `verify [--json]`, `decode` to `pcm`,
  `wav`, `damf` or `adm` (TrueHD presentations and E-AC-3 JOC objects),
  `compare` against a raw PCM reference (24-bit integer for TrueHD, 32-bit
  float for E-AC-3), `oamd` and `emdf` payload dumps.
- `tools/three_way.py`: oadec against FFmpeg and a Dolby decode of the same
  stream; `tools/gen_joc_tables.py`, `tools/gen_vq_tables.py`: format
  tables from the specification files.
- `tools/adm_diff.py`: structural comparison of two ADM BWF files.
- Documentation: format notes (`docs/truehd.md`, `docs/oamd.md`,
  `docs/eac3.md`, `docs/joc.md`), the behaviour of the Dolby command-line
  tools (`docs/dolby-tools.md`) and the evidence reports under
  `docs/evidence/`.

### Changed

- `Decoder::decode` returns `Option<Decoded>` and `Decoder::flush` drains what
  the two lookaheads hold, so no frame is lost at the end of a stream or after
  an error. Streams that use neither tool are never held and decode
  byte-identically to before.
