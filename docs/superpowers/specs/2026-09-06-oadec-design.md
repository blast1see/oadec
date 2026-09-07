# oadec design specification

> Approved implementation plan (2026-09-06). This file is the design spec the
> per-milestone implementation plans in `docs/superpowers/plans/` argue from.


## Context

The author wants a from-scratch, English-language, GitHub-publishable **decoder engine** for
the two Dolby Atmos carriage formats found on consumer media:

1. **Dolby TrueHD with Dolby Atmos** (Blu-ray / UHD remuxes): a lossless MLP-family
   bitstream with up to four substreams. Substream 3 carries the 16-channel object
   presentation (restart sync `0x31EC`, delta-interpolated matrices); the access
   unit's *extra data* carries Evolution frames whose payload id 11 is the Object
   Audio Metadata (OAMD). Decoding the audio *together with* this metadata is the
   top priority ("çok önemli").
2. **E-AC-3 JOC** (Dolby Digital Plus with Dolby Atmos, streaming): lossy 5.1
   core + EMDF payloads (id 11 OAMD, id 14 JOC) in the audio-block skip field, from
   which up to 16 objects are reconstructed in the 64-band QMF domain.

Output must be something the Dolby tools on this machine re-encode: **DAMF**
(`.atmos` + `.atmos.metadata` + `.atmos.audio`) and **ADM BWF** (BW64 with `axml`,
`chna`, `dbmd`). Workflows: TrueHD Atmos → `oadec` → DAMF/ADM → DEE → E-AC-3 JOC;
E-AC-3 JOC → `oadec` → objects + OAMD.

Hard requirements: own code (prior art is studied and used as cross-check oracles,
never copied); every correctness claim backed by measurable tests on real files from
`E:\`; system language English; permission granted to install missing software and to
use `C:\dee`, Dolby Media Encoder, Dolby Reference Player.

## Decisions (confirmed by the author on 2026-09-06)

| Decision | Choice |
|---|---|
| Language | **Rust** (stable-msvc). The author asked to run `rustup update` if outdated — first step of M0 |
| Name / location | **`oadec`** — `%USERPROFILE%\Documents\oadec`, GitHub `blast1see/oadec`, **GPL-3.0**, work dir `E:\oadec-work` |
| E-AC-3 core | **Native decoder from ETSI TS 102 366**; FFmpeg only as a test oracle |
| Output priority | DAMF first (what DEE and truehdd ecosystems use), ADM BWF second, validated against DEE's own DAMF→ADM conversion |
| Vault | Turkish notes under `20-Projeler/oadec/` + `10-Notlar/`, linked from `MOC — Medya araclari` |

House rules inherited from kiyas/hibrit: brand-neutral naming, binaries resolved from
absolute paths, real-media tests opt-in via env var (`OADEC_MEDIA`), never touch
system-wide configs, nothing copyrighted or Dolby-proprietary committed to the repo.

## Research findings (evidence gathered 2026-09-06)

**Dolby tools on this machine (verified by running them):**
- DEE **5.2.1** at `C:\dee\dee.exe` (`--xml/-x job.xml`). Inputs include `adm`, `damf`,
  `atmos_mezz`, `ec3`; filters `encode_to_dthd` (TrueHD Atmos, `spatial_clusters`
  12|14|16), `encode_to_atmos_ddp` (JOC, 384–1024 kbps), `convert_atmos_mezz`
  (adm ⇄ damf ⇄ mxf_iab), `ddp_decode` (channel PCM only). Templates under
  `C:\dee\xml_templates\`. Validators `C:\dee\atmos_info.exe -i`, `C:\dee\bwf_info.exe -i`.
  No TrueHD decoder, no object-output DD+ decoder.
- DEE **5.7.2** CLIs inside DME 3.7.0: `C:\Program Files\Dolby Media Encoder\resources\dee\
  {dee_dthd_encoder.exe, dee_ddpjoc_encoder.exe, atmos_info.exe --validate 1}`;
  `--input-format atmos_mezz` auto-detects DAMF / ADM BWF / MXF IAB.
- DEE ADM validator rules (binary strings): exactly one `audioProgramme`; chunks
  `axml` + `chna` + **`dbmd`** required ("dbmd indicates not Dolby Atmos" is an error);
  objects start at 0 with correct duration; gapless `audioBlockFormat`s; Cartesian
  positions; "No objects are allowed in first 10 input channels" (7.1.2 bed first);
  48 kHz; "DAMF only supports 24-bit PCM".
- Dolby Reference Player 3.2.0 `drp.exe`: `--truehddec-presentation 16`,
  `--out-ch-config`, `--audio-out-file x.wav` (WAV export limited to 2.0/3.1/5.1/7.1),
  `--metadata-directory DIR` (CSV metadata dump), `--print-info`; accepts `.mlp .ec3
  .eac3 .ac3` (rename `.thd` → `.mlp`). Undocumented string `force-atmos-file-dump`.
- Toolchain: rustc/cargo 1.95 (msvc), Python 3.14 (numpy, lxml, PyYAML), git, MSVC 2022,
  cmake; ffmpeg git-2026-08-30 at `C:\ffmpeg\bin`; mkvextract v101 at `C:\temp`;
  mediainfo 26.05 at `%USERPROFILE%\Documents\tools\mediainfo.exe`; eac3to 3.65;
  **truehdd b54f209 (lib 0.7.1)** on PATH, source checkout at
  `%USERPROFILE%\.cargo\git\checkouts\truehdd-3567f30f26a39483\b54f209\` (Apache-2.0).

**Test material on `E:\` (MediaInfo 26.05, all 159 MKVs parsed):**
- TrueHD Atmos: 20 tracks / 17 files, all `16-ch`, 48 kHz, 11–15 objects. Smallest:
  `E:\Pi (1998).mkv` (26 GB, track ID 3, 11
  objects). Extremes: The Kings Man 7.6 Mbps, Knives Out 2.6 Mbps; Talk to Me 15
  objects; Avatar Fire and Ash EN+TR TrueHD and JOC. Raw 6.4 GB `.thd` (Braveheart,
  seamless-branching Blu-ray) under `%USERPROFILE%\Braveheart (1995)\`.
- E-AC-3 JOC: 27 tracks (640/768/1024 kbps). Raw ES already present:
  `E:\samples\Nightcrawler (2014)-English.ec3` (0.63 GB).
  Smallest container: The Day Of The Jackal S01E07 (4.9 GB, 640 kbps). Disclosure Day
  exists as streaming JOC and as two TrueHD Atmos remuxes (disc vs streaming mix).
- Controls: plain E-AC-3/AC-3 abundant; **no non-Atmos TrueHD** (generate with DEE).
  No ADM/DAMF sets anywhere. E:\ has 1.5 TB free; never stage on D:\ (109 GB) or H:\.
- MediaInfo pitfall: `--ParseSpeed=0` misses Atmos on some TrueHD tracks.

**Prior art:**
- **truehdd** (Rust, Apache-2.0): full TrueHD decode incl. substream 3, OAMD via
  Evolution frames, DAMF writer (no ADM, no E-AC-3). Known problems: sync drift /
  metadata halt at seamless-branching points (#19, #25), DEE rejecting output on wrong
  header channel counts (#29, fixed), `TRIM_LUT[14]` bug (fixed to −15.0), panics on
  multiple object-info blocks / multiple beds / ISF (fixed 0.6.0), ignores
  `block_offset_factor` and uses only block 0, no DRC.
- **Cavern** (C#, bespoke non-commercial licence — never read as a template, never
  copy): E-AC-3 JOC decoder; its author flags a possible defect in TS 103 420's
  sparse-mode dequantization ("revert 0 to gainStep when the standard is fixed").
- **harletty-bridge** (Rust; vendors `truehd`, adds an `eac3` JOC crate; lib GPL-3.0),
  **ac3forge** (C++23, GPL-3.0, clean-room AC-3/E-AC-3/JOC from the standards),
  **raress96/dolby-atmos-encoder** (JOC *encoder* PoC blocked by the keyed
  `emdf_protection` HMAC — confirms "decode ourselves, let DEE encode" is right).
- **FFmpeg**: `mlpdec.c` decodes substreams 0–2 only (`MAX_CHANNELS 8`), discards extra
  data; `ac3dec.c` skips the skip field. Bit-exact oracle for the 7.1 hierarchy only.
- **enable-atmos71**: binary patcher for Dolby's encoders (layout mode 19→21/28); only
  a reminder to support `joc_dmx_config_idx = 1` (7.X core).

**Specifications (public):**
- **ETSI TS 103 420 V1.2.1** — OAMD syntax (§5.5), JOC syntax (§6.2), band mapping
  (§6.5), JOC decoder (§6.6), QMF (§7), EMDF payload ids 11/14 (§8.2), `addbsi`
  complexity index (§8.3), Huffman tables (Annex A), ADM mapping (Annex B). Companion
  **`ts_103420v010201p0.zip`** ships the tables/QMF coefficients as symbolic source.
  etsi.org returns 403 to the fetch tool — download via Chrome tools or `curl -A`.
- **ETSI TS 102 366 V1.4.1** — AC-3/E-AC-3 syntax + Annex H EMDF (V1.2.1 lacks EMDF).
- **Dolby TrueHD high-level bitstream description** (developer.dolby.com, 2018),
  **Dolby Atmos Master ADM Profile v1.0/1.1**, ITU-R BS.2076-3, EBU Tech 3285 + s7
  (`chna`), AES "The MLP Lossless Compression System", US 9,794,712 (four-substream
  matrix hierarchy). DAMF has no public spec: truehdd `src/damf.rs` and Dolby's own
  `C:\dee\tools\dee_watch_folder\parse_damf.py` are the de-facto references.

## Architecture

Cargo workspace `%USERPROFILE%\Documents\oadec` (edition 2024, `#![forbid(unsafe_code)]`):

```
oadec/
  Cargo.toml                 workspace, resolver 3, shared lints; deny.toml (licence allowlist)
  crates/
    oadec-bits/              MSB-first BitReader (bits/sbits/peek/align16/variable_bits_max),
                             CRC-8/CRC-16 with the TrueHD polynomials, XOR parity, nibble
                             fold; BitWriter behind a test feature. No deps.
    oadec-emdf/              EMDF (TS 102 366 H) and Evolution containers (one parser, a
                             `Flavor` switch), OAMD payload per TS 103 420 §5.5 (+ Dolby
                             private elements 3/4 best-effort, unknown skipped by size),
                             JOC parameter syntax §6.2 (parse only). Object model.
    oadec-truehd/            Extractor (sync/AU framing), Parser (major sync incl.
                             extra_channel_meaning, directory, restart/block headers,
                             matrices, filters, Huffman, extra data), Decoder
                             (recorrelation, rematrix 31EA/EB/EC, remap, lossless check),
                             presentations 0–3, timing model (branches, duplicates),
                             conformance statistics for `verify`.
    oadec-eac3/              syncinfo/bsi/audfrm/audblk (AC-3 + E-AC-3, dependent
                             substreams, AHT, SPX, ECPL parse, TPNP), bit allocation,
                             exponents, mantissas, IMDCT 256/512 + KBD windows, core PCM
                             (f32, no DRC/dialnorm/downmix), skip-field capture → EMDF,
                             addbsi complexity index.
    oadec-joc/               64-band complex QMF analysis/synthesis, JOC dequantisation,
                             temporal interpolation, band mapping, object reconstruction.
    oadec-spatial/           `Program` model (10-slot bed, objects, event Timeline,
                             presentation meta); writers: DAMF (.atmos YAML,
                             .atmos.metadata YAML, .atmos.audio CAF 24-bit), ADM BWF
                             (BW64/RF64 + axml + chna + dbmd), WAV/CAF/W64; DAMF reader
                             for round trips.
    oadec-cli/               oadec info | verify [--json] | decode | oamd --dump | emdf --dump | compare
  tools/                     Python (numpy/lxml/PyYAML): corpus extraction, reference
                             generation (ffmpeg, truehdd, DRP), DEE job XML generator +
                             runner, adm_diff, pcm_compare, evidence report
  tests/                     integration tests; real-media suite gated by OADEC_MEDIA
  tests/assets/              tiny synthetic fixtures only (< 200 KB total)
  docs/                      format notes in our own words, dolby-tools.md (real CLI
                             flags), media-notes.md, evidence/<date>.md
  .github/workflows/ci.yml   windows + ubuntu: fmt, clippy -D warnings, test, deny; fuzz
                             build on Linux with a seed corpus; no real media
```

Dependencies kept minimal: `clap`, `serde` + `serde_json`, `thiserror`, `log` +
`env_logger`, `quick-xml` (axml), `rustfft`/`realfft` (IMDCT, QMF). YAML for DAMF is
written by hand (its layout is quirky). No FFI, no ffmpeg at runtime, no git deps.

Data flow: `bytes → Frame/AccessUnit structs (pure parse, no state) → decoder state
machine → DecodedUnit {pcm, sample_pos, timed OAMD} → Program → writer`. Parsers are
separate from decoders so `info`/`verify` never run DSP. Streaming writers only: a
2-hour 16-channel 24-bit film is ≈ 41 GB; free-space check before the first write.

Numeric rules: TrueHD is integer math with `i64` accumulators, bit-exact by
construction. E-AC-3 uses `f64` internally, `f32` output; agreement with FFmpeg's float
decoder is *measured* with a stated tolerance, never assumed.

## Milestones (dependency order; a gate must pass before the next milestone starts)

Sizes: S ≈ half a day, M ≈ 1–2 days, L ≈ 3–5 days of focused work.

**M0 — Bootstrap (S).** `rustup update stable`; create the workspace, LICENSE
(GPL-3.0), README skeleton, `.gitignore` (`/target`, media extensions, work dirs),
CI, `deny.toml`; `git init` + first commit. Create
`E:\oadec-work\{specs,thd,ec3,clips,ref-ffmpeg,ref-truehdd,ref-drp,dee,out}`.
Download specs to `specs/` (never committed): TS 103 420 PDF + `p0.zip`, TS 102 366
V1.4.1, Dolby TrueHD high-level bitstream description, Dolby Atmos Master ADM Profile,
EBU 3285 + s7, BS.2076-3; `pip install pypdf` to read them. Record the real CLI flags of
`dee.exe`, `dee_dthd_encoder.exe`, `dee_ddpjoc_encoder.exe`, `atmos_info.exe`, `drp.exe`
in `docs/dolby-tools.md`. Gate: `cargo build` green, specs on disk and readable.

**M1 — Test corpus and reference oracles (M, mostly machine time).**
- `mkvextract tracks` → `thd/pi.thd`, `talktome.thd`, `kingsman.thd`, `knivesout.thd`,
  the Disclosure Day TrueHD remux; copy `braveheart.thd`; JOC → `ec3/jackal-s01e07.ec3`,
  `disclosure-web.ec3`, `shaun-1024.ec3`; copy `nightcrawler.ec3`; one plain E-AC-3 5.1,
  one AC-3. `.mlp` copies for DRP.
- 60-second clips via `ffmpeg -c copy -t 60 -f truehd|eac3` → `clips/`.
- References: `ffmpeg -c:a pcm_s32le -f s32le` for presentation 2, and via `-downmix
  5.1` / `stereo` for presentations 1/0 (confirm the substream selection with `ffmpeg -h
  decoder=truehd`; record channel order); `ffmpeg -drc_scale 0 -c:a pcm_f32le -f f32le`
  for every E-AC-3 stream; `truehdd decode --presentation all` + `truehdd verify --json`
  (branch lists, AU counts); `drp.exe --print-info --metadata-directory` and
  `--truehddec-presentation 16 --out-ch-config 7.1 --audio-out-file` for every stream.
- Synthetic ground truth: `tools/make_synthetic_damf.py` → 7.1.2 bed + 6 dynamic
  objects, each a distinct signal (band-limited noise per object, sweeps), 30 s, moving
  positions and gain ramps; validate with `atmos_info`; encode with DEE to `synth.thd`
  (TrueHD Atmos, 16 elements), `synth.ec3` (JOC 768 kbps), convert to `synth.wav` (ADM
  via `convert_atmos_mezz`); also a non-Atmos TrueHD 7.1 control.
- Gate: `ref/manifest.json` lists every artefact with hashes and the exact command;
  branch counts per film noted in `docs/media-notes.md`.

**M2 — `oadec-bits` (S).** Bit reader, `variable_bits_max` (value accumulation
`(v+1) << n` per continuation group), CRC-8/16 (polynomials as used by FFmpeg/truehdd:
major sync CRC-16 poly 0x002D; restart header CRC-8 poly 0x1D; substream CRC-8 poly
0x63 — init/xor conventions pinned by real streams), bit-range CRC, parity, nibble fold.
Gate: unit tests with hand vectors and a crafted restart header that reproduces its
own CRC; BitWriter→BitReader property test.

**M3 — TrueHD extractor, major sync, `info`, parse-only `verify` (M).**
AU header `check_nibble:4, access_unit_length:12 (×2 bytes), input_timing:16` with
nibble parity over header + directory == 0xF; major sync (`0xF8726FBA`; `format_info`,
signature `0xB752`, flags, `variable_rate:1, peak_data_rate:15, substreams:4,
extended_substream_info:4, substream_info:8`, 64-bit `channel_meaning`,
`extra_channel_meaning` (`length:4` → `(len+1)·16` bits: `sixteench_dialogue_norm:5,
mix_level:6, channel_count:5, dyn_object_only:1 → lfe_present:1 | content_description:4
→ chan_distribute:1, lfe_only:1, sixteench_channel_assignment:10 / isf:3 /
dynamic_object_count:5`, align 16), CRC-16 over `format_sync..channel_meaning`);
`fs = 48000 << code` or `44100 << (code−8)`, `samples_per_au = 40·fs/44100`;
presentation masks `[1, (si>>2)&3, (si>>4)&7, ((si>>4)&8) | (7 ^ (7 >> (esi&3)))]`;
substream directory `extra_substream_word:1, restart_nonexistent:1, crc_present:1,
reserved:1, substream_end_ptr:12 (+ drc_gain_update s9, drc_time_update 3, reserved 4)`;
extra data header + Evolution/opaque/padding shapes + `extra_data_parity` (raw bytes
kept for M7). Extractor: find `F8 72 6F BA`, validate major-sync CRC before locking,
walk AUs by length, resync on failure; FBB (`0xF8726FBB`) refused with a clear message.
Gate: 100 % of AUs of all TrueHD files parse with zero CRC/parity failures; AU counts
equal truehdd `verify --json`; `oadec info` agrees with MediaInfo/truehdd/DRP on every
shared field.

**M4 — Substream syntax + full `verify` (L).** Restart header (fields and rules in the
appendix), block header with guard bits (`new_guards → 8`, `block_size:9 (8..=160)`,
matrixing, `output_shift s4 × (max_matrix_chan+1) ≤ max_shift`, `quantiser_step_size:4
× (max_chan+1)`, per-channel params: FIR/IIR `order:4 (A≤8, B≤4, A+B≤8), coeff_q:4,
coeff_bits:5, coeff_shift:3, coeffs, IIR states`, `huff_offset s15, huff_type:2,
huff_lsbs:5 (≤24; ≤31 for 31EC)`), matrix syntax for 31EA/31EB/31EC, block data
(`error_protect → block_data_bits:16 ≤ 16000`; per sample bypassed LSBs then Huffman /
plain codes; `block_header_crc:8`), segment end (`last_block_in_segment`, align16,
terminator `0x348D3` + `zero_samples_indicated:1 → zero_samples:13 | 0x1234`,
`substream_parity` = XOR ^ 0xA9, `substream_crc`), end pointer and sample-count
consistency. `verify` tallies every rule with `--json` lines and exit code 7.
Gate: zero parity/CRC/bit-count failures on every AU and substream of all films;
parse speed ≥ 200× realtime.

**M5 — Presentations 0–2 decode, bit-exact (L).** Recorrelation (FIR+IIR, `pred = acc
>> coeff_q`, quantiser mask, ±2²³ saturation), union rematrix buffer across the
presentation's substreams, 31EA noise channels from `dither_seed` (LCG
`seed = ((seed>>7) ^ ((seed>>7)<<5) ^ (seed<<16)) & 0x7FFFFF`), 31EB per-AU dither
table with index `((P−pmi)(2n+1)+n) & mask` and `<< (11+dither_scale)`, matrix
`in[matrix_ch] = ((acc>>18) & ~((1<<qss)−1)) + bypassed_lsb` with coefficients
`m << (18 − frac_bits)`, remap `out[ch_assign[ch]] = shift(in[ch], output_shift)`,
lossless check `^= (out & 0xFFFFFF) << (ch & 7)` folded and compared at the next
restart header, `max_bits` check, zero-sample trimming. `oadec decode --presentation
0|1|2 --out wav|pcm` and `oadec compare`.
Gate: **byte-identical** to FFmpeg (S32 `>> 8`) over whole films for presentations
0/1/2 on Pi, Talk to Me, The Kings Man, Knives Out, Braveheart, synth, the DEE 7.1
control; lossless-check mismatches = 0 (except the AU right after a valid branch).

**M6 — Presentation 3, `0x31EC` (L).** Exact syntax and formula in the appendix:
`cf_mask`, `cf_shift_code`, `lsb_bypass_bit_count`, `dither_scale`, delta coefficients
with `delta_precision`, per-sample linear ramp, `m_coeff += delta_cf` at AU end, 32-bit
range, dither table regenerated per AU, 16 output channels and labels from
`sixteench_channel_assignment` / `dyn_object_only`.
Gate: substream-3 lossless check passes on every AU of every film (the built-in
bit-exactness oracle — FFmpeg cannot help here); PCM equals truehdd's presentation-3
CAF sample-for-sample on Pi + ≥ 3 more films (mapping columns, ignoring its bed-conform
padding); every disagreement root-caused with the lossless check as arbiter; `synth.thd`
returns the element signals DEE was fed (measured); ≥ 60× realtime for all four
presentations.

**M7 — Extra data → Evolution → OAMD → timed metadata (L).** Evolution/EMDF container
(`evo_version:2 (+vbm(2,16) if 3), key_id:3 (+vbm(3,10) if 7)`, payloads until id 0
(`id:5`, EMDF escape `id 31 → +vbm(5)`), config `smploffst? / duration? vbm(11,2) /
groupid? vbm(2,16) / codecdata? 8 / discard_unknown → frame_aligned, create/remove
duplicate, priority:5, proc_allowed:2`, `size vbm(8,4)`, bytes, protection `2+2 bits →
{0,1,4,16} bytes`; `smploffst` is `variable_bits(11)` in Evolution vs `11 bits + 1
reserved` in EMDF — a `Flavor` switch, and `verify` counts a non-zero 12th bit). OAMD
per TS 103 420 §5.5: header, `program_assignment` (dyn-object-only + `b_lfe_present`;
beds standard 10-bit / non-standard 17-bit, `b_bed_chan_distribute`, multiple bed
instances `3 bits + 2`, `b_lfe_only`; ISF `3 bits → [4,8,10,14,15,30]`; dynamic count
`5 (+7 if 31) + 1`; reserved `(4 bits+1)·8`), elements (`id:4`, `size =
(variable_bits_max(4,4)+1)·8` bits, optional alternate id, `b_discard_unknown_element`,
pad to size): object (`sample_offset_code:2 → 0 | idx:2 → [8,16,18,24] | 5 bits`,
`num_obj_info_blocks:3 (+1)`, per block `block_offset_factor:6, ramp_duration_code:2 →
0|512|1536| idx:4 → LUT16 | 11 bits`, per object×block `object_info_block` with the
status-index rules: block 0 implies all, `0b01` reads no bits and implies all, `0b10`
reuse, `0b11` partial; gain codes; render info absolute `6+6+1+4` or differential
`3·s3`, distance, zones (7→0), elevation, size, screen ref, snap; additional table data
skipped), trim (`TRIM_LUT[14] = −15.0`), extended object (divergence, extended
precision `/310`, `/75`), private 3/4 best-effort. Timing: event position = first
sample of the AU (counted in *emitted* samples) + `smploffst` + `sample_offset` +
block offset; all blocks (≤ 8) supported. `Timeline` with the re-assert rule (a
payload restating current values with zero timing is not an event; `--all-events`
keeps them). `oadec oamd --dump csv|json`; consistency checks in `verify`.
Gate: 100 % of payloads parse; hand-built unit vectors for every status-index shape,
multiple blocks, multiple beds, ISF, differential positions; dump equals DRP's
`--metadata-directory` CSV (object count, bed assignment, per-event position / gain /
size / zone / snap and event times — this pins the `block_offset_factor` semantics) on
Pi, Talk to Me, Braveheart; truehdd's `.atmos.metadata` events compared, divergences
recorded with DRP as tie-breaker.

**M8 — Timing model, seamless branches, robustness (L).** `latency = (au_offset·spa +
output_timing − input_timing) & 0xFFFF`, `advance = (output_timing − spa −
input_timing) & 0xFFFF`, `fifo_duration = ceil(au_len·256 / peak_data_rate)`; input
timing jump flags; output timing expectation at restart headers; branch validity
c1–c4 (`advance ≤ prev_advance + 3·spa/4`, `≤ prev_advance + spa − prev_fifo`, `≤
samples_per_75ms − spa`, `prev_len·256 ≤ prev_peak·interval`); valid → re-anchor the
input clock, invalid → restart the timing model and report; duplicate AU = same
`output_timing` and same lossless word as the previous AU → dropped with its OAMD;
parser and decoder reset in lockstep and resume at the next major sync after an error;
`substream_info` change at a major sync = new segment (`_seg<N>` files + loud warning);
major sync every ≤ 128 AUs, restart gap 1 or ≥ 8. The sample clock is emitted samples,
never AU index × 40.
Gate: all films decode end-to-end; branch list identical to truehdd's `verify --json`;
duplicates dropped at the same AUs; presentation-2 length equals FFmpeg's with
`--keep-duplicates`; OAMD timeline monotonic across branches (Braveheart); byte-flipped
clip resyncs at the next major sync with skipped AUs reported and valid output files;
concatenated TrueHD files (`copy /b`) resync.

**M9 — `Program` model + DAMF writer + DEE TrueHD round trip (L).** Bed slots 0–9
(L R C LFE Lss Rss Lrs Rrs Lts Rts; missing bed channels written as silence; bed
channels outside 7.1.2 map to IDs 128+ and are refused for DEE targets unless
`--fold-extra-beds`), objects from 10; `.atmos` (version, presentations, fps, offset,
ffoa, scNumberOfElements/scBedConfiguration, warpMode, trimMode, bedInstances, objects),
`.atmos.metadata` (first payload full, later ones diffed; pure re-asserts dropped),
`.atmos.audio` CAF 24-bit big-endian; DAMF coordinates `x = (px−0.5)·2, y = (0.5−py)·2,
z = pz`. `oadec decode --out damf`.
Gate: `atmos_info -i` (5.2.1) and `atmos_info --validate 1` (5.7.2) pass for every
film; DEE `encode_to_dthd` (`spatial_clusters 16`) succeeds; decoding DEE's output
with `oadec` gives per-element PCM equality or a documented exact deviation and OAMD
equality within quantisation; same on `synth.thd`; truehdd's `--bed-conform` DAMF for
Pi is event-equivalent.

**M10 — ADM BWF writer (L).** Reference ADM via `convert_atmos_mezz` from `synth.atmos`
and `pi.atmos`; `tools/bwf_dump.py` dumps `axml`, `chna`, `dbmd`. Writer: one
`audioProgramme`; bed audioObject(s) with DirectSpeakers pack/channel formats; one
audioObject per dynamic object with Objects formats and gapless Cartesian
`audioBlockFormat`s from rtime 0 (`gain`, size, zone exclusion, `jumpPosition
interpolationLength` = ramp); `chna` with one UID per track; `dbmd` with the Dolby
Atmos segment (+ supplemental) laid out as the reference and the profile PDF agree;
WAVE_FORMAT_EXTENSIBLE 24-bit 48 kHz; BW64/RF64 `ds64` when > 4 GB; single-pass chunk
order (metadata after `data`) with a two-pass fallback if DEE/`bwf_info` reject it;
tracks 1–10 = bed, objects from 11.
Gate: `bwf_info` OK; `tools/adm_diff.py` shows structural equality with DEE's
conversion for `synth` (same element tree, same IDs modulo documented differences,
equal block times); `dee_dthd_encoder` and `dee_ddpjoc_encoder` accept our ADM; DME
GUI loads it; decode-back equality as in M9.

**M11 — E-AC-3 core decoder (L+).** `sync` (0x0B77, `frmsiz`, CRC-16 0x8005 coverage
confirmed on real frames), `bsi` (strmtyp, substreamid, fscod/fscod2, numblkscod →
1/2/3/6 blocks, acmod, lfeon, bsid 16, dialnorm, compr, chanmap, mixmdat, infomdat,
convsync, blkid, `addbsie → addbsil:6 → addbsi` with `flag_ec3_extension_type_a` and
`complexity_index_type_a` = object count ≤ 16), `audfrm` (expstre, ahte, snroffststr,
transproce, blkswe, dithflage, bamode, frmfgaincode, dbaflde, skipflde, spxattene,
coupling/exponent strategies incl. the 6-block `frmexpstr` table, `convexpstr`, AHT
flags, SNR offsets, transient pre-noise, block start info), `audblk` (blksw, dithflag,
dynrng, SPX, coupling incl. ECPL syntax, rematrixing, chbwcod, exponents D15/D25/D45,
bit-allocation params, delta BA, `skiple → skipl:9 → skipfld` captured, mantissas),
bit allocation, mantissas (grouped 3/5/11-level, dither for bap 0, AHT GAQ 0–3 + VQ
tables), SPX, coupling with phase flags (ECPL parse-only unless a stream needs it),
IMDCT 512 / 2×256 with KBD windows and overlap-add, substream merge per `chanmap`,
native layout out, no DRC/dialnorm/downmix (values reported by `info`), `emdf` scan of
every `skipfld` for `0x5838` + length + container (location recorded; JOC streams are
expected to carry it in the last substream).
Gate: vs FFmpeg (`-drc_scale 0`): max |diff| ≤ 1e-6 full scale on ≥ 99.99 % of
samples and none > 1e-4 on Nightcrawler (full), Jackal, Disclosure Day, Shaun, synth,
and 5 plain E-AC-3/AC-3 controls (if larger, mirror FFmpeg's IMDCT/window operation
order); zero CRC failures; `info` object count equals MediaInfo's complexity index;
EMDF found in 100 % of JOC frames and its OAMD equals DRP's CSV; feature-coverage table
(AHT/SPX/ECPL/TPNP per stream) — untested features marked as such.

**M12 — JOC objects (L+).** `joc()` from payload 14: `joc_dmx_config_idx:3` (0 = 5.X,
1 = 7.X, 2 = 5.X+2 heights, 3/4 = 90° phase-shift variants — apply the prescribed
phase network before analysis), `joc_num_objects:6 (+1)`, `joc_ext_config_idx`, per
object presence, `joc_num_bands_idx:3` (§6.5 table), sparse flag, quant idx, data
points (1–2, slope, time-slot offset), dense `joc_mtx[ch][band]` or sparse `(channel
idx, vector)` Huffman-coded differentially (Annex A / ZIP tables); dequantisation
(fine/coarse; sparse gain-step: spec reading *and* Cavern-reported reading behind a
flag); band → 64-bin mapping; QMF-64 analysis of the core channels (prototype from the
ZIP; 24 slots per 1536-sample frame; group delay measured by an impulse unit test and
compensated), per-slot interpolated matrix (step or ramp per `joc_slope_idx`, hold
after the last point, state carried across frames; cut streams start with zero history
and a warning), `obj[o][k][t] = Σ_ch W[o][ch][band(k)][t]·X[ch][k][t]`, synthesis per
object; LFE bypassed into bed slot 3 with delay compensation; elements assembled per
OAMD `program_assignment`. `oadec decode x.ec3 --out damf|adm`.
Gate: `synth.ec3`: the decoded-vs-source element correlation matrix is diagonal-
dominant (each element best matches its own source, normalised correlation ≥ 0.9 at
lag 0 after delay compensation), per-element SNR reported for both dequant readings
(expect 15–30 dB; JOC is parametric) and the winner documented; core LFE bit-parity;
OAMD equality. Real streams: DAMF → `atmos_info` OK → DEE `encode_to_atmos_ddp` →
decode again: OAMD equal, element correlations ≥ 0.95; DRP 7.1 render vs a static
test-only render of our objects ≥ 0.9 correlation per channel (sanity, not a claim).

**M13 — Release, evidence report, vault (M).** README (English: purpose, status per
milestone, quick start, oracles and how verification works, what it deliberately does
not do — no rendering, no DRC, no encoding — licence notes: truehdd read as Apache-2.0
reference and attributed, no Cavern code, no Dolby binaries/keys/templates, no
affiliation), CHANGELOG, CONTRIBUTING (licence-hygiene rules), `tools/evidence.py` →
`docs/evidence/<date>.md` (every gate as a table), release workflow like kiyas, tag
`v0.1.0`. Vault: `20-Projeler/oadec/oadec.md` (Turkish MOC: why/what/state/pitfalls),
permanent notes for lessons ("TrueHD 16 kanal sunumu dört alt akışın birleşimidir",
"DEE'nin ADM doğrulayıcısı dbmd ister", "MediaInfo ParseSpeed 0 Atmos'u kaçırır", …),
link in `MOC — Medya araclari.md`; memory file `oadec-projesi.md`.
Gate: fresh clone builds and tests on CI (Windows + Linux); evidence report committed.

## Evidence & verification strategy

Independent checks, none trusting a single decoder:
1. TrueHD built-in integrity: AU nibble parity, major-sync CRC-16, restart CRC-8,
   substream parity + CRC-8, block bit counts, `lossless_check` per presentation,
   `max_bits` — 100 % pass on every AU, reported by `oadec verify`.
2. FFmpeg bit-exactness for presentations 0–2 over whole films.
3. truehdd agreement for presentation 3, OAMD events, branch lists; disagreements
   root-caused (either side may be wrong; the lossless check and DRP arbitrate).
4. DRP metadata CSV agreement; DRP 7.1 renders as correlation-level sanity for JOC.
5. DEE round trips with synthetic known content (TrueHD and JOC) — quantitative tables.
6. DEE validators (`atmos_info --validate 1`, `bwf_info`) and DEE's DAMF→ADM conversion
   as the ADM oracle.
7. E-AC-3 core vs FFmpeg float with measured error; CRC per frame.

Test layers: unit tests with hand-built bitstreams (BitWriter) for every syntax
element; golden tests on 60-s clips using digests derived from the *oracles* (not from
our decoder), clips outside the repo, skipped without `OADEC_MEDIA`; the opt-in
real-media suite with a `media.toml` manifest (expected AU/frame counts, branch counts,
digests); `proptest` generators and `cargo fuzz` targets (`truehd_parse`, `emdf_parse`,
`oamd_parse`, `eac3_frame`, `joc_params`) — parsers never panic, typed errors only;
oracle round trips scripted (`tools/roundtrip.py`) as one command each.

## Risk register

| Risk | Mitigation |
|---|---|
| No public TrueHD spec; 31EC corners (`cf_shift_code`, `delta_precision`, integer `recip`, `lsb_bypass_bit_count`) | lossless check per AU is the arbiter; truehdd CAF differential test; FFmpeg for 0–2; Dolby high-level PDF, AES paper, US 9,794,712 |
| `block_offset_factor` / OAMD timing semantics | DRP CSV oracle on TrueHD and JOC; spec §5.5 semantics; never rely on truehdd here |
| truehdd wrong in places (community reports vs DRP) | oracle, not truth; DEE round trips and DRP checks as tie-breakers |
| Spec downloads blocked for the fetch tool | Chrome tools / `curl -A`; installs permitted |
| Dolby-private OAMD elements 3/4 | self-delimiting sizes → skip safely; best-effort parse flagged in `info` |
| TS 103 420 sparse dequantisation possibly defective | both readings behind a flag; synthetic round-trip SNR decides |
| `emdf_protection` keyed HMAC | not verifiable, documented; never sought or stored |
| ECPL / rare E-AC-3 features in the wild | parse fully so frames stay in sync; decode when a stream needs it; coverage table |
| E-AC-3 float mismatch vs FFmpeg | tolerance gate; mirror FFmpeg's IMDCT/window operation order if needed |
| JOC time-differential coding at clip starts | zero history + warning; JOC test clips start at stream start |
| `dbmd`/`axml` unknowns → DEE rejects | structural diff against `convert_atmos_mezz` output; validators before DEE; DAMF stays the primary hand-off |
| BW64 > 4 GB and chunk-after-data acceptance | 3-hour synthetic test; two-pass fallback |
| Seamless-branch drift (prior art's failure) | sample clock = emitted samples; Braveheart branch tests; timeline monotonicity assertion in writers |
| Multiple bed instances / ISF | model supports them; writers refuse with a message when the target cannot express them |
| Disk/perf (16-ch 24-bit 2 h ≈ 41 GB) | streaming writers, free-space check, `--workdir` required, intermediates deleted |
| Licence hygiene | GPL-3.0; truehdd (Apache-2.0) read for facts and cited in docs, code re-derived; Cavern never read as a template; `cargo deny`; no Dolby binaries, XML templates, outputs, logs or keys committed; DEE licensing is the author's responsibility and never referenced |
| DEE CLI flags uncertain | M0 records real `--help` output in `docs/dolby-tools.md` |
| Windows-only dev box | CI also on Ubuntu; no Windows-specific APIs |

## Appendix — format facts verified first-hand in the truehdd checkout (2026-09-06)

(`truehdd/` = `%USERPROFILE%\.cargo\git\checkouts\truehdd-3567f30f26a39483\b54f209\`;
facts to re-derive in our own code, cited in `docs/`.)

- **Restart header** (`truehd/src/structs/restart_header.rs:104-122, 350-361, 419-473,
  476-527`): `restart_sync_word:14 (0x31EA|EB|EC), output_timing:16, min_chan:4,
  max_chan:4, max_matrix_chan:4, dither_shift:4, dither_seed:23, max_shift:4,
  max_lsbs:5, max_bits:5, max_bits_repeat:5 (=), error_protect:1, lossless_check:8,
  hires_output_timing:1, reserved:2, heavy_drc_present:1 (only if flags & 0x2000;
  then heavy_drc_gain_update s9, heavy_drc_time_update:3, else 12 reserved bits),
  ch_assign:6 × (max_matrix_chan+1) (a permutation, each ≤ max_matrix_chan), crc:8`.
  31EC only in substream 3; 31EB never in substream 0; 31EA in substream 1 only if
  `substream_info & 8`. Lossless check compared at the next restart header for every
  decoded presentation (`restart_header.rs:607-639`).
- **31EA/31EB matrix syntax** (`structs/matrix.rs:174-206`): `primitive_matrices:4`;
  per matrix `matrix_ch:4, frac_bits:4, lsb_bypass_used:1`; coefficients for channels
  `0..=max_matrix_chan (+2 noise channels for 31EA)`: `m_flag:1 → s(frac_bits+2)`;
  31EB adds `dither_scale:4`. Scaling `m << (18 − frac_bits)` (`matrix.rs:334`).
- **31EC matrix syntax** (`matrix.rs:78-172`): `new_matrix:1 → new_matrix_config:1 →
  primitive_matrices:4 (+1)`, per matrix `matrix_ch:4, frac_bits:4, cf_shift_code =
  u3 − 1, lsb_bypass_bit_count:2, dither_scale:4, cf_mask:(max_matrix_chan+1) bits`;
  `m_coeff = s(frac_bits+2)` where `cf_mask` bit set, else 0; `interpolation_used:1 →
  new_delta:1 → new_delta_config:1 → {delta_bits:4, delta_precision:2}` per matrix;
  `delta_cf = s(delta_bits+1)` per masked channel (0 if `delta_bits == 0`);
  `interpolation_used && !new_delta` keeps previous deltas; `!interpolation_used`
  zeroes them. Config fields persist until the next `new_*_config`. Scaling:
  `M = m_coeff << (18 + cf_shift_code − frac_bits)`, `D = delta_cf << (18 +
  cf_shift_code − frac_bits − delta_precision)` (`matrix.rs:255-311`).
- **31EC per-sample application** (`process/decode.rs:663-717`), `n` = sample index in
  the AU, `spa` = samples per AU, `recip = 65536 / spa` (integer division: 1638 / 819 /
  409 for 40 / 80 / 160):
  `acc = Σ in[ch]·M[pmi][ch]`; `acc_delta = Σ in[ch]·D[pmi][ch]`;
  `if dither_scale ≠ 0: acc += dither_table[((P−pmi)(2n+1)+n) & mask] << (11+dither_scale)`;
  `acc += (acc_delta >> 18) · n · (recip << 2)`;
  `in[matrix_ch] = ((acc >> 18) & ~((1 << qss[matrix_ch]) − 1)) + bypassed_lsb[n][pmi]`;
  after the last block of the AU: `M += D`. Dither table regenerated per AU from the
  running seed (`utils/dither.rs:8-43`). Saturation ±2³¹ for 31EC (`decode.rs:490-494`).
- **Hierarchy** (`decode.rs:561-585, 811-831`): the rematrix input of presentation p is
  the union of recorrelated channels `min_chan..=max_chan` of every substream in
  `mask[p]`; only substream p's own matrices run; channel count = `max_matrix_chan+1`.
  Substreams decode fully, in order 0..=p, with per-substream `decoded_sample_len`.
- **Block data** (`structs/block.rs:189-539`): `block_header_exists:1 →
  [restart_header_exists:1 → restart_header] block_header`; `error_protect →
  block_data_bits:16`; per sample: bypassed LSBs (1 bit per matrix if `lsb_bypass_used`;
  `lsb_bypass_bit_count` bits for 31EC), then per channel: `lsbs_bits = huff_lsbs −
  qss`; Huffman: `v = lsbs + (code << lsbs_bits) − (shift < 0 ? 0 : 1 << shift)`,
  `shift = lsbs_bits + 2 − huff_type`; plain: `v = lsbs − (1 << (lsbs_bits−1))`;
  `v += huff_offset; v <<= qss`; deepest 9-bit Huffman code must end in 1.
- **Extra data / Evolution** (`structs/extra_data.rs:49-236`, `structs/evolution.rs`):
  layout in M3/M7 above; `extra_data_parity` = XOR of all block bytes after the header
  word (excluding itself) ^ 0xA9 ^ (reserved << 4) ^ reserved. OAMD = payload id 11
  (`process/decode.rs:347-359`).
- **Timing** (`block.rs:234-238`, `restart_header.rs:151-155, 231-253`,
  `access_unit.rs:445-455, 497-577`): formulas in M8.
- **DAMF** (`src/damf.rs`): `.atmos` fields lines 9–56 / 289–343 (`version 0.5.1`);
  events lines 376–500; coordinate conversion in `structs/oamd.rs:325-331`; bed ID
  mapping `damf.rs:236-243`; re-assert rule `damf.rs:640-654`. truehdd bails on multiple
  update blocks, multiple beds and ISF (`damf.rs:405-415`) — we must not.

## Verification (end-to-end, run before every gate claim and before release)

```powershell
rustup update stable; cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test --workspace
$env:OADEC_MEDIA = 'E:\oadec-work'; cargo test --release -p oadec-cli --test real -- --ignored
oadec verify --json E:\oadec-work\thd\pi.thd                       # 0 parity/CRC/lossless failures
oadec decode E:\oadec-work\thd\pi.thd --presentation 2 --out pcm  ; oadec compare --ref ref-ffmpeg\pi-p2.s32 --bits 24
oadec decode E:\oadec-work\thd\pi.thd --presentation 3 --out damf --output E:\oadec-work\out\pi
& "C:\Program Files\Dolby Media Encoder\resources\dee\atmos_info.exe" -i E:\oadec-work\out\pi.atmos --validate 1
& "C:\Program Files\Dolby Media Encoder\resources\dee\dee_ddpjoc_encoder.exe" --input-format atmos_mezz --input pi.atmos --output pi_joc.ec3 --data-rate 768
oadec decode pi_joc.ec3 --out damf                                    # objects + OAMD back out
python tools\evidence.py --out docs\evidence\<date>.md                # all oracle comparisons as tables
```

## Out of scope (v0.1)

AC-4, Dolby E, MLP/FBB (DVD-Audio), a production renderer (test panner only),
re-encoding to TrueHD/JOC (DEE does it; the HMAC key makes an own encoder pointless),
GUI, DRC application. Revisit after the evidence report exists.

## Execution notes

- After approval, follow the superpowers workflow: save the design spec into the repo
  (`docs/superpowers/specs/2026-09-06-oadec-design.md`, distilled from this plan), then
  `writing-plans` per milestone → TDD, subagents for independent modules (e.g. E-AC-3
  bit-allocation tables vs QMF), `verification-before-completion` before every gate claim.
- First actions: `rustup update`, create the repo, download the specs, start M1
  extraction jobs in the background while M2/M3 code is written.

## Status, 2026-09-07

M0–M10 are done and evidenced in `docs/evidence/2026-09-07.md`; M11 and
M12 are done and evidenced in `docs/evidence/2026-09-08.md`; M13 is done
except the release tag.

Deviations from the plan above:

- The Reference Player CSV (`--metadata-directory`) carries no per-object
  metadata, so it could not serve as the OAMD oracle of M7. The truehdd event
  lists and the encoder round trips (TrueHD and E-AC-3 JOC) took its place.
- No synthetic DAMF ground truth was built in M1. The round trips used real
  content (the first 1:45 of Pi, then the whole film) and measured the
  encoder deviations directly: LFE bit-exact, objects at −0.00565 dB and
  permuted, events one frame boundary earlier.
- The E-AC-3 corpus grew by three streaming JOC tracks supplied on
  2026-09-07 (Extraction, Red Notice, Glass Onion).
- The Dolby Atmos Conversion Tool 2.1.2, installed on 2026-09-07, became a
  second independent DAMF/ADM oracle. Its DAMF diff format omits unchanged
  `ID` and `samplePos` keys.
- ADM files pass the Dolby validators only with `--dolby-origin-tag`; the
  flag is off by default.
- The M11 gate was changed from a fixed 1e-6 tolerance to a three-decoder
  envelope, because AC-3 family decoders dither with their own sequences
  (clause 6.3.4) and never agree to float precision. The Dolby Encoding
  Engine's `ddp_decode` filter is the third decoder.
- The M12 gate used the encoder round trip of real content instead of a
  synthetic set; the QMF synthesis follows the matrix equation of clause 7.3
  rather than its pseudo-code, and the surround pair is rotated by -j for
  downmix configurations 3 and 4 (both measured, see the evidence).
- The corpus never used AHT, spectral extension or enhanced coupling, so
  those paths are untested (AHT, SPX) or unsupported (ECPL).
