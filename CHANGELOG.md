# Changelog

All notable changes to `oadec` are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
Semantic Versioning.

## [Unreleased]

### Fixed

- **E-AC-3 dependent substreams are decoded.** A Dolby Digital Plus 7.1
  programme used to come out as its 5.1 core with `verify` calling the file
  clean; it now decodes to all eight channels, and each one pairs with FFmpeg's
  by at least 53 dB. Across the 40 eight-channel tracks in one library, 40 give
  eight channels and 320 channel comparisons give no mismatch. Two custom
  channel maps occur in the wild and the file name does not say which: 0x1a00
  is 7.1 and 0xa010 is 5.1.2.
- **Corruption reaches the exit code.** `verify` caught every one of 220
  injected bit errors and exited 7; `decode --format damf` produced Atmos
  objects and metadata with exit 0 and no diagnostic on 99 of them. Every
  delivery path now decides with the list `verify` uses and exits 7, and
  `docs/exit-codes.md` writes the policy down. Replaying both campaigns: 0
  silent, 0 panics.
- **Sparse JOC matrices.** Clause 6.6.2's pseudo-code is wrong in three places,
  and Dolby's decoder disagrees with all three: an unselected channel takes the
  code that dequantises to zero gain, not the printed 50 or 100; the channel
  index accumulates from the resolved previous index; and the coefficient chain
  runs unbroken across the bands whatever channel each one selects, instead of
  restarting at the offset every time the channel changes. Measured on every
  sparse frame there was, and then on fifty-five times as much: one clip of
  Extraction 2 holds 825 sparse objects and no steep object at all, and with all
  three corrections its sparse frames sit at 46,78 dB against Dolby where the
  rest of the clip is 46,69. The printed reading puts them at -3,92. The seed of the chain is the printed 50/100 and
  stays there; reading it as 48/96 costs 50 dB. `--sparse-as-printed` restores
  the printed reading. The first parameter band's channel index is no longer
  taken modulo the channel count, which clause 6.6.2 does not ask for: a band
  whose index names no channel now selects none of them instead of wrapping
  onto a real one. No conforming stream can tell the difference.
- **The steep interpolation switched one time slot too late.** `joc_offset_ts`
  is one-based (clause 6.3.4.4 defines it as the transmitted bits plus one) and
  the `ts` of clause 6.6.5 counts from zero, but the printed pseudo-code
  compares them directly. Dolby's decoder switches at the slot the offset
  names. Steep is 737 503 of 32 493 245 object updates, so this reaches most
  streams: Glass Onion's worst object goes from 25,24 dB against Dolby's
  objects to 49,93 and its median from 47,76 to 65,44; Shaun of the Dead's
  frame 581 from 23,51 dB to 58,90. Titles with no steep object are
  bit-identical either way. `--steep-as-printed` restores the printed
  reading.
- **The JOC downmix input mapping was written for configuration 1 alone.**
  Table 47 of TS 103 420 ends configuration 1 in the rear surround pair and
  configurations 2 and 4 in the top front pair; reading all three the same way
  leaves a configuration 4 stream's height channels unmapped. It refused the
  decode rather than producing wrong output, and it had never fired because
  configuration 4 needs a seven-channel downmix, which needs a dependent
  substream, which this release is the first to decode. Three library titles
  carry configuration 4 and their objects now come out at 50,13 dB worst
  against Dolby's. Reading every Dolby track of every file rather than the first
  finds five such tracks in the library and none at all at configurations 1
  and 2: `tools/joc_config_sweep.py`. Swapping the top front pair costs 29 dB, so the order is
  measured.
- **Dolby's object path reads the TrueHD major sync far more strictly than
  ordinary decoding does.** Of nine bits edited with the defined CRC-16
  repaired, eight are refused -- reserved bits, an undefined flag, a lower peak
  data rate, a cleared variable-rate flag, a DRC start-up gain, a mix level --
  while a legal change to `extended_substream_info` is accepted, and every one
  of the edited streams still decodes at presentation 2 and at presentation 16
  with the default channel configuration. That weakens the reading that
  `2ch_control_enabled` is necessary for the object presentation: clearing it
  refuses, but so does changing a dynamic-range gain, which cannot be causal.
  The correlation across six unmodified titles is untouched.
- **`--no-dither`, `--no-tpnp` and `--ecpl-spec` reach the object path.**
  `decode --format damf` on an E-AC-3 stream built the core decoder with its
  defaults and ignored all three, so three measurement flags read as applied and
  were not. `--core-only` is now refused with an object output rather than
  quietly ignored: the object programme is the whole programme. Two media tests
  hold both.
- The EMDF container is looked for where TS 103 420 clause 8.2 puts it, the
  last dependent substream, and `auxdata` is read where clause 4.4.4 puts it.
  `eac3-joc-config` no longer skips dependent substreams, which would have made
  it a silent no-op on exactly the streams it exists to interrogate.
- The media suite fails when it cannot reach the media. It used to report
  "10 passed" in 0.00 s with `OADEC_MEDIA` unset, and CI now has a job that
  fails if that comes back.

### Added

- Authored ground truth for the dependent-substream merge. A Dolby Digital Plus
  7.1 stream made by Dolby's own encoder from eight tones, one per channel,
  decodes so that every channel carries its own tone at -20,0 dBFS with the
  loudest tone belonging to another channel 108 to 191 dB below, and FFmpeg
  gives the identical assignment. `--core-only` on the same clip shows what the
  defect delivered: a left surround holding the back-left tone at -20,0, the
  side-left at -21,2 and the side-right at -26,2. Clause E.2.8.2's
  replace-and-add, measured. Kept as a media test that fails by 197 dB if the
  side and back pairs are swapped.
- The clean experiment the presentation-16 question needed, by authoring the
  stimulus instead of editing a finished stream. DEE writes
  `2ch_control_enabled` clear when `presentation_2ch/drc_default_on` is false,
  and Dolby's object path opens the result -- in the same session where it
  refuses all three library titles that carry the flag clear. Both encodes of
  the controlled scene are the same size, `oadec info` differs in that one line,
  and the object audio is byte-identical. So the field that correlates perfectly
  across six titles is **not sufficient**, and the earlier bit patch that seemed
  to show necessity was measuring the edit rather than the field. Nor is the
  content: each refused title's own objects, decoded here and re-encoded by DEE,
  are opened by the same object path that refuses the originals. The same holds
  for the other refusal, the configuration 0 stream Dolby gives six channels:
  its own objects come back as sixteen that Dolby opens, on the head clip and on
  a mid-file cut, with the accepted title at sixteen either way.
- Material where the steep branch of clause 6.6.5 is the rule instead of the
  exception, and a gate on it. Three authored scenes say what provokes it: a
  sweep every half frame gives 15 steep objects of 4 695, teleporting between
  opposite corners gives 45 and all of them in the first three frames, and
  objects arriving out of silence mid-file give **1 410**, from frame 63 to the
  end. This encoder answers movement with smooth interpolation and the arrival
  of level with the steep branch, which is why film soundtracks carry steep
  objects at all. On the third scene, against Dolby's decode of the stream its
  own encoder wrote, the reading in use is 51,39 dB median where the printed one
  is 45,93, better on five elements of five -- kept as a media test that fails
  when the two readings are swapped. Neither sparse matrices nor two data points
  could be authored at any data rate or from any scene tried.
- A library-wide answer to what Dolby's own object decoder opens: 226 Dolby
  Digital Plus tracks across 210 files, 223 opened as sixteen objects and **two**
  refused. The one stream known to be refused is not a singleton -- The King
  (2019), a streaming release, gets the same six channels. With nine
  configuration-0 streams instead of two, exactly one field splits the refused
  from the opened: `dialnorm`, 31 in both refused and 23 to 27 in every accepted
  one. Neither half is the answer alone, since Dolby opens seven configuration-0
  streams and 90 streams carrying `dialnorm` 31. `tools/ec3_patch_dialnorm.py`
  can move the field and repair the frame CRC exactly -- it round-trips byte for
  byte -- and the answer is still no: Dolby refuses a patched stream for a legal
  value that is not 31, so it is reacting to the edit. A fourth instrument with
  a limit on it, and a false positive caught by its control.
- A third decoder's opinion, which is neither ours nor Dolby's. `truehdd` 0.6.1
  opens the object presentation on all six TrueHD Atmos titles, including the
  three Dolby's object path refuses, with the element counts oadec reports and
  `.atmos.audio` files that carry the same MD5 -- 212 527 200 element-samples,
  zero differing. That does not say why Dolby refuses, but it separates *Dolby
  refuses these three* from *these three are not object programmes*. The
  presentation-3 baseline now covers six titles rather than three.
- A unit test for the OAMD event-timing equation of TS 103 420 clause 5.3.2,
  `start_sample = sample_offset + 32 x block_offset_factor`, over seven
  combinations. It is the one field whose value the two decoders disagree
  about: `truehdd` drops the second term, which puts one event in five 32
  samples early. Modulo the 1 536-sample codec frame the clause names, oadec's
  event positions take four distinct residues over seven streams and
  `truehdd`'s take seven, so the streams say the same thing the clause does.
- `oadec atmos-author`, which writes a Dolby Atmos master from a scene
  description so that a decode can be checked against authored metadata rather
  than against another decoder. A second scene settles object gain and object
  size: Dolby's encoders carry neither. An object authored at -24 dB comes back
  at the same level as one authored at 0 dB with gain 0 in its metadata, and a
  sized object is spread over seven to eleven encoded objects with size 0 and
  its energy within a decibel of what went in. `tools/ground_truth.py` reads
  both fields back and measures the essence level. Dolby's `atmos_info` accepts the master; DEE
  encodes it both ways; seven static object positions come back exactly, at
  sample offset zero, through both TrueHD Atmos and E-AC-3 JOC.
- `--core-only` on `decode` and `compare`, which writes the independent
  substream's channels alone -- the 5.1-compatible decode clause E.2.8.2
  allows, and what a reference decoder limited to 5.1 produces.
- `verify --json` reports the programme's substreams, the JOC syntax each
  stream uses branch by branch, and the frames where the rare branches occur,
  so a clip that exercises one can be cut.

- `oadec eac3-joc-offset`, which rewrites `joc_offset_ts_bits` (clause 6.3.4.4)
  in every JOC payload and changes nothing else, beside the existing
  `eac3-joc-config`. Both are instruments against oadec itself and **neither
  works against the Dolby decoder**: it discards a payload that has been
  rewritten and holds the previous matrix, whatever the new value says. That
  withdraws the support for one earlier conclusion -- relabelling a working
  stream from configuration 3 to 0 makes Dolby drop to six channels, but so
  does discarding the payload, and the experiment cannot tell them apart. The
  claim does not survive either: a second stream carrying configuration 0
  unmodified, Dredd, is decoded to sixteen objects by Dolby and agrees with ours
  to 52,44 dB at worst. Whatever makes it refuse the other one belongs to that
  stream.
- The parsers now report where they found things: `Frame::skip_bits` gives the
  bit offset of each skip field, and `container::Payload::data_bit` the bit
  offset of a payload's first byte. Between them a tool can reach a field
  inside an EMDF container and rewrite it in place.
- `docs/evidence/2026-09-10.md`, which records that the TrueHD object
  presentation is bit-exact against the Dolby decoder's own object output.
  That decoder refuses a raw elementary stream but takes the same audio in an
  MP4; with that, Pi's twelve objects and Talk to Me's sixteen come out
  identical, all 60 909 600 and 30 720 000 samples, worst difference zero.

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
