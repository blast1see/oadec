# Conformance rules

Every check `oadec verify` makes on a TrueHD stream, with its name, the
severity a failure carries by default and the source the rule rests on.
This file is generated from `crates/oadec-truehd/src/rules.rs` (`oadec rules
--markdown`); a test keeps the two equal.

A run fails when its worst finding is at or above `--fail-on` (default
`error`). The source decides how heavy a rule may be: a rule a public
specification states, or one measured on real streams, may fail a stream;
a rule only another implementation or tool states (`secondary`) warns at
most, and fails a stream only when asked to with `--fail-on warning`.

| Rule | Default | What a failure means | Source | Counter |
|---|---|---|---|---|
| `au.framing` | fatal | the access unit cannot be framed: its length, its major sync, its directory or an end pointer | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 3.1, 4.1, 4.5; measured: `docs/audit/conformance-matrix.md` | `framing_errors` |
| `au.header_parity` | error | the check nibble does not fold the access-unit header and directory to 0xF | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.1; measured: `docs/audit/conformance-matrix.md` | `header_parity` |
| `au.restart_flag` | error | a directory restart_nonexistent flag contradicts its segment | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.5 | `restart_flag` |
| `sync.crc` | error | a major sync fails its CRC-16 | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 2.8, 4.2; measured: `docs/audit/conformance-matrix.md` | `major_sync_crc` |
| `sync.signature` | error | a major sync's signature is not 0xB752 | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.2 | `major_sync_signature` |
| `sync.resync` | error | sync was lost and found again at a later major sync; the units between are gone | measured: `docs/audit/evidence/verification-2026-09-14/03-corruption-replays.json` | `resyncs` |
| `sync.skipped_bytes` | error | bytes that belong to no access unit were skipped (the magnitude is the byte count) | measured: `docs/audit/evidence/verification-2026-09-14/03-corruption-replays.json`; measured: `docs/audit/evidence/verification-2026-09-14/04-function-smoke-matrix.json` | `skipped_bytes` |
| `sync.trailing_bytes` | error | bytes after the last access unit do not make one: a stream cut inside a unit (the magnitude is the byte count) | measured: `docs/audit/evidence/verification-2026-09-14/04-function-smoke-matrix.json` | `trailing_bytes` |
| `sync.foreign_pattern` | info | the DVD-Audio (FBB) sync pattern met inside a TrueHD stream and passed over | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 3.1; measured: `docs/audit/evidence/verification-upgrade-2026-11/02-foreign-sync-old-new.json` | -- |
| `config.changed` | error | a major sync changes the format, the substreams or the rate the first one declared | measured: `docs/audit/evidence/remediation/sampling-rates.json` | `config_changes` |
| `extra.header_parity` | error | the length check nibble of the extra data does not fold to 0xF | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.8 | `extra_header_parity` |
| `extra.truncated` | error | the extra data declares more words than the access unit holds | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.8 | `extra_truncated` |
| `extra.parity` | error | the parity byte of an Evolution block does not match | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.8; measured: `docs/audit/conformance-matrix.md` | `extra_evolution_parity` |
| `extra.padding` | error | padding in the extra data is not zero | measured: `docs/audit/conformance-matrix.md` | `extra_padding_nonzero` |
| `evo.container` | error | an Evolution frame does not open as an EMDF container | ETSI TS 102 366 V1.4.1, Annex H; measured: `docs/audit/conformance-matrix.md` | `evolution_container_errors` |
| `evo.oamd` | error | an Object Audio Metadata payload does not parse | ETSI TS 103 420 V1.2.1, clause 5; measured: `docs/audit/conformance-matrix.md` | `oamd_errors` |
| `segment.syntax` | fatal | a substream segment does not parse (restart header and its CRC, block header, filters, samples); the substream is read again from its next restart | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 2.8, 4.6; measured: `docs/audit/conformance-matrix.md` | `substream_errors` |
| `segment.parity` | error | a segment's parity byte does not match | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.6; measured: `docs/audit/conformance-matrix.md` | `segment_parity` |
| `segment.crc` | error | a segment's CRC-8 does not match | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.6; measured: `docs/audit/conformance-matrix.md` | `segment_crc` |
| `segment.end_pointer` | error | a segment does not end where its end pointer says | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.5 | `segment_end` |
| `segment.sample_count` | error | the blocks of a segment do not add up to one access unit of samples | measured: `docs/audit/conformance-matrix.md` | `sample_count` |
| `segment.terminator` | error | a termination word ends in neither 0x1234 nor a zero-sample count | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.6 | `terminator_tail` |
| `segment.unexpected_tail` | error | a segment has room for a termination word and holds something else | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 4.6; measured: `docs/audit/conformance-matrix.md` | `unexpected_tail` |
| `block.data_bits` | error | a block's coded size differs from its block_data_bits | measured: `docs/audit/conformance-matrix.md` | `block_data_bits` |
| `timing.invalid_branch` | error | a timing jump that is not a valid seamless branch; the stream restarts there | Dolby TrueHD (MLP) High-Level Bitstream Description (2018), 2.7; measured: `docs/audit/evidence/verification-2026-09-14/27-branch-advance-corpus.json` | `invalid_branches` |
| `decode.lossless_check` | error | verify --decode: a restart header's lossless check word disagrees with the decoded samples | measured: `docs/audit/conformance-matrix.md` | -- |
| `decode.stopped` | fatal | verify --decode: a presentation's decode stopped before the end of the stream | measured: `docs/audit/conformance-matrix.md` | -- |
