# TrueHD parsing layer (M2–M4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> Execution note: this plan is executed inline by the planning session (bit-exact
> decoder work needs continuity); subagents are used for review at each gate.

**Goal:** A bit reader with the TrueHD integrity primitives, a TrueHD access-unit
extractor and parser that reads every header field of every substream, and an
`oadec info` / `oadec verify` that pass on 100 % of the access units of the real
corpus with zero parity/CRC failures.

**Architecture:** `oadec-bits` (pure, no I/O) → `oadec-truehd::{extract, sync, au,
substream}` (pure parsers producing structs) → `oadec-cli` (`info`, `verify`).
Parsing never runs DSP; the decoder (M5+) consumes the parsed structs.

**Tech Stack:** Rust 1.98 stable, no external crates in `oadec-bits`; `thiserror`
for typed errors; `clap` for the CLI (added in M3).

**Spec:** `docs/superpowers/specs/2026-09-06-oadec-design.md` (appendix = format facts).

## Global Constraints

- `#![forbid(unsafe_code)]` everywhere (workspace lint); parsers return typed errors, never panic on malformed input.
- Real media lives only under `OADEC_MEDIA` (default `E:\oadec-work`); tests that need it are `#[ignore]` and skip cleanly when the variable is absent.
- Nothing from truehdd or FFmpeg is copied; syntax is re-derived from the spec appendix and cited in `docs/`.
- All arithmetic on samples is integer (`i64` accumulators); no floating point in `oadec-truehd`.
- Commit after every green task; messages in English, conventional-commit style.

---

### Task 1: `oadec-bits` — BitReader

**Files:**
- Create: `crates/oadec-bits/src/reader.rs`
- Modify: `crates/oadec-bits/src/lib.rs`

**Interfaces:**
- Produces: `BitReader<'a>::{new, data, position, len_bits, remaining, is_aligned(bits), seek(pos), skip(n), align(bits), peek(n<=32)->u32, read(n<=32)->u32, read_u64(n<=64), read_bool, read_signed(n<=32)->i32, read_variable_bits_max(n, max_groups)->u32}` and `BitError { position, requested, available }`.
- MSB-first; `read(0)` returns 0; `read_signed` sign-extends two's complement.
- `read_variable_bits_max` follows the decoder convention: `value = 0; groups = 0; loop { value += read(n); more = read_bool(); groups += 1; if !more || groups == max_groups { break } value = (value + 1) << n }`.

- [x] **Step 1: Write the failing tests** (`reader.rs` `#[cfg(test)]`): sequential reads `0b1010_0101, 0xFF` → `read(3)=5, read(5)=5, read(8)=255`; `peek` does not advance; `read_signed(4)` of `0b1111` = −1 and of `0b0111` = 7; reading past the end returns `BitError { position, requested, available }`; `align(16)` from bit 3 lands on 16; 32-bit read at bit offset 7 spanning five bytes; `read_u64(40)`; `variable_bits_max(4, 4)` on bits `0101 0` = 5 and on `1111 1 0010 0` = 258; group cap: `1111 1 1111 1` with max 2 = `((15+1) << 4) + 15` and the position is 10.
- [x] **Step 2: Run** `cargo test -p oadec-bits` → compile error (types missing).
- [x] **Step 3: Implement** `BitReader` with a zero-padded big-endian 64-bit window (`load64`) so any `peek(n<=32)` at any bit offset is one load + shift.
- [x] **Step 4: Run** `cargo test -p oadec-bits` → all pass.
- [x] **Step 5: Commit** `feat(bits): bit reader, TrueHD CRC-8/16 and parity helpers`.

### Task 2: `oadec-bits` — CRC-8 / CRC-16 (shift-then-XOR convention) and parity

**Files:**
- Create: `crates/oadec-bits/src/crc.rs`, `crates/oadec-bits/src/parity.rs`
- Modify: `crates/oadec-bits/src/lib.rs`

**Interfaces:**
- Produces: `Crc8::{new(poly) const, poly, step_bit(crc, bit), update_byte(crc, byte) = table[crc] ^ byte, update_bytes(crc, &[u8]), update_bits(crc, data, start_bit, len_bits)}`; `Crc16` with the same shape (`update_byte = table[crc>>8] ^ (crc<<8) ^ byte`); constants `CRC8_RESTART = Crc8::new(0x1D)`, `CRC8_SUBSTREAM = Crc8::new(0x63)` with `CRC8_SUBSTREAM_INIT = 0xA2`, `CRC16_MAJOR_SYNC = Crc16::new(0x002D)`; `parity::{xor_bytes, fold_nibble, fold_u32}`.
- Convention: for every data bit (MSB first) `crc = (crc << 1) ^ (poly if the old MSB was set); crc ^= bit`. Processing eight bits this way equals `shift8(crc) ^ byte`, which is what the tables implement.

- [x] **Step 1: Write the failing tests**: (a) `update_bits` over a byte-aligned range equals `update_bytes`; (b) the textbook formulation used by the other public decoder (xor-then-shift CRC with init 0x3C over the first n−1 bytes, then XOR the last byte) equals ours with init 0xA2 over all n bytes, for pseudo-random buffers; (c) a port of that decoder's restart checksum (init = top two bits of byte 0, bitwise tail) equals `CRC8_RESTART.update_bits(0, buf, 2, bit_len)`; (d) CRC-16/CRC-8 table update equals eight `step_bit` calls; (e) `fold_nibble(0xF0) == 0xF`, `xor_bytes(&[1,2,4]) == 7`, `fold_u32(0x0100_0100) == 0`.
- [x] **Step 2: Run** → fails to compile.
- [x] **Step 3: Implement** tables as `const fn` (loops in const context), bitwise edges in `update_bits`.
- [x] **Step 4: Run** → pass.
- [x] **Step 5: Commit** (same commit as Task 1).

### Task 3: `oadec-truehd` — access-unit extractor

**Files:**
- Create: `crates/oadec-truehd/src/{extract.rs, error.rs}`; modify `lib.rs`.

**Interfaces:**
- Produces: `Extractor::new() -> Self`, `Extractor::push(&mut self, bytes: &[u8])`, `Extractor::next_unit(&mut self) -> Option<Unit>` where `Unit { offset: u64, bytes: Vec<u8>, has_major_sync: bool, resynced: bool }`; `Extractor::finish(&mut self) -> Vec<Unit>`; statistics `skipped_bytes`.
- Rules: an AU begins with `check_nibble:4, access_unit_length:12 (16-bit words), input_timing:16`; a major sync starts at byte 4 with `F8 72 6F BA` (`F8 72 6F BB` = MLP/FBB → `Error::UnsupportedFbb`); lock only after a major sync whose CRC-16 verifies; walk by length; if the next header nibble parity fails or the length is 0, drop bytes until the next verified major sync (count them).

- [ ] **Step 1: Failing tests**: two hand-built AUs (major sync + minor) round-trip; garbage before the first sync is skipped and reported; a truncated tail is kept pending until `finish`.
- [ ] **Step 2–4:** implement; tests pass.
- [ ] **Step 5: Commit** `feat(truehd): access-unit extractor with resync`.

### Task 4: `oadec-truehd` — major sync + channel meaning + presentation map

**Files:** create `sync.rs`, `channel.rs`, `presentation.rs`; tests inline.

**Interfaces:**
- Produces: `MajorSync::parse(&[u8]) -> Result<MajorSync>` with every field of the spec appendix (`format_info` decoded: `sampling_frequency: u32`, `samples_per_au: u16`, multichannel types, channel modifiers, `ch_assign_6/8: u16`), `signature`, `flags`, `variable_rate`, `peak_data_rate`, `substreams`, `extended_substream_info`, `substream_info`, `channel_meaning: ChannelMeaning`, `extra: Option<ExtraChannelMeaning>`, `crc_ok: bool`, `len_bytes`.
- `PresentationMap::from(substream_info, extended_substream_info)` → `mask(p) -> u8`, `kind(p) -> {Independent, CopyOf(q), Invalid}`.
- `ChannelLabel` enum + `labels_for_presentation(p) -> Vec<ChannelLabel>`.

- [ ] Tests: a synthetic major sync assembled with a `BitWriter` test helper (crc computed with `CRC16_MAJOR_SYNC`) parses back field by field; `PresentationMap` for `substream_info = 0xF8, esi = 3` (four presentations) and for a 2-substream stream; `fs` code 0 → 48000 / 40, code 2 → 192000 / 160, code 8 → 44100 / 40.
- [ ] Commit `feat(truehd): major sync, channel meaning, presentation map`.

### Task 5: `oadec-truehd` — AU header, substream directory, extra data (parse only)

**Files:** create `au.rs`, `extra.rs`.

**Interfaces:**
- Produces: `AccessUnit::parse(bytes, &StreamConfig) -> Result<AccessUnit>` with `header: AuHeader { check_nibble, length_words, input_timing }`, `major_sync: Option<MajorSync>`, `directory: Vec<DirectoryEntry { extra_word, restart_nonexistent, crc_present, end_ptr_words, drc: Option<(i16, u8)> }>`, `segments: Vec<Range<usize>>` (byte ranges, 16-bit aligned), `extra: Option<ExtraData { kind: Padding|Opaque(Vec<u8>)|Evolution{reserved, frame_bytes: Vec<u8>, parity_ok} }>`, `parity_ok: bool`.
- Header parity: XOR of the 4 header bytes and all directory bytes, folded to a nibble, must be `0xF`.

- [ ] Tests: hand-built AU with 2 substreams and Evolution extra data; parity failures reported not panicked; a directory whose end pointer exceeds the AU → `Error::Malformed`.
- [ ] Commit `feat(truehd): access-unit header, directory and extra-data parsing`.

### Task 6: `oadec-cli info` and parse-only `verify`

**Files:** `crates/oadec-cli/src/{main.rs, info.rs, verify.rs, input.rs}`; `tests/real_media.rs` (ignored).

- `oadec info <file.thd>`: format (TrueHD/FBA), sampling rate, samples/AU, substreams, presentations with channel labels, 16-channel info (bed assignment, dynamic object count, dyn-object-only, LFE), peak data rate, AU count, duration, Evolution/OAMD payload presence (ids seen), major sync interval.
- `oadec verify --parse-only <file.thd> [--json]`: counters (AUs, major syncs, CRC-16 failures, header parity failures, extra-data parity failures, resyncs/skipped bytes, AU length consistency), exit 7 on any failure.
- [ ] Real-media test (`OADEC_MEDIA`): `pi.thd` → 0 failures; AU count equals `truehdd verify --json` (stored in `ref/manifest.json`).
- [ ] Commit `feat(cli): info and parse-only verify`.

### Task 7: Substream syntax — restart header, block header, matrices, filters, block data, segment end (M4)

**Files:** `crates/oadec-truehd/src/{restart.rs, block.rs, matrix.rs, filter.rs, huffman.rs, segment.rs, state.rs}`.

**Interfaces:**
- `ParserState` (per stream) with per-substream `SubstreamParserState` (restart config, guards, per-channel params, matrix config incl. 31EC fields).
- `Segment::parse(reader, &mut ParserState, substream_index) -> Result<Segment { blocks: Vec<Block>, terminator: Option<Terminator>, parity_ok: Option<bool>, crc_ok: Option<bool>, end_bit }>`.
- `Block { restart: Option<RestartHeader>, header: Option<BlockHeader>, samples: Vec<[i32;16]> (block_size rows of raw decoded codes: (huff + lsbs + offset) << qss), bypassed_lsb: Vec<[i32;16]> }`.
- Huffman: 9-bit lookup tables built from the three code books at first use; the deepest 9-bit code (`0x001` for −7 and `0x081` for the maximum) must end in 1.

- [ ] Tests: crafted restart header with matching CRC parses and rejects a corrupted CRC; block header guard bits; matrix syntax for 31EA (with two noise columns) / 31EB (dither_scale) / 31EC (cf_mask, cf_shift_code, lsb_bypass_bit_count, delta config); Huffman decode of every code of the three books (built from the code list verified against the FFmpeg tables: book 1 = −7..10, book 2 = −7..8, book 3 = −7..7); segment terminator and parity/CRC on a crafted segment.
- [ ] `oadec verify` (full) on the corpus: zero substream parity/CRC failures, zero block bit-count mismatches, restart CRC = 0 failures, ≥ 200× realtime.
- [ ] Commit `feat(truehd): full substream syntax parsing and verify`.

## Self-review

- Spec coverage: M2 (Tasks 1–2), M3 (Tasks 3–6), M4 (Task 7). Decoding (M5+) is a separate plan.
- Type names used across tasks: `BitReader`, `BitError`, `Crc8/Crc16`, `MajorSync`, `PresentationMap`, `AccessUnit`, `Segment`, `Block`, `ParserState` — consistent.
