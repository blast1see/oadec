//! The catalog of the checks `verify` makes on a TrueHD stream.
//!
//! Every rule has a stable name, the severity a failure carries by default and
//! the source the rule rests on. The source matters as much as the check: a
//! rule taken from another implementation and confirmed by nothing else may
//! warn, and may never fail a stream by default, while a rule a public
//! specification states, or one measured on real streams, may. A unit test
//! below holds the catalog to that, and `docs/conformance-rules.md` is the
//! catalog rendered (a second test keeps the two equal).

pub use oadec_diag::{Provenance, RuleInfo};
use oadec_diag::{measured, secondary, spec};

/// Dolby's public high-level description of the FBA bitstream.
pub const DOLBY_HL: &str = "Dolby TrueHD (MLP) High-Level Bitstream Description (2018)";

/// The Blu-ray Disc Association's public white paper on the BD-ROM audio
/// visual format.
pub const BDA: &str =
    "BDA, White Paper BD-ROM Audio Visual Application Format Specifications, version 2.5 (2011)";

/// ETSI TS 102 366 V1.4.1 (AC-3 and E-AC-3, Annex H: EMDF).
pub const TS_102_366: &str = "ETSI TS 102 366 V1.4.1";

/// ETSI TS 103 420 V1.2.1 (object audio metadata).
pub const TS_103_420: &str = "ETSI TS 103 420 V1.2.1";

const MATRIX: &str = "docs/audit/conformance-matrix.md";
const CORRUPTION: &str = "docs/audit/evidence/verification-2026-09-14/03-corruption-replays.json";
const SMOKE: &str = "docs/audit/evidence/verification-2026-09-14/04-function-smoke-matrix.json";
const BRANCHES: &str = "docs/audit/evidence/verification-2026-09-14/27-branch-advance-corpus.json";
const RATES: &str = "docs/audit/evidence/remediation/sampling-rates.json";
const FOREIGN: &str =
    "docs/audit/evidence/verification-upgrade-2026-11/02-foreign-sync-old-new.json";
const CENSUS: &str =
    "docs/audit/evidence/verification-upgrade-2026-11/04-integrity-rule-census.json";
const PROTECTION: &str =
    "docs/audit/evidence/verification-upgrade-2026-11/05-protection-census.json";
const DECLARED: &str =
    "docs/audit/evidence/engine-round-2026-11/33-what-an-evolution-frame-declares.json";
const FIFO: &str = "docs/audit/evidence/verification-upgrade-2026-11/06-fifo-model.json";
const DISC: &str = "docs/audit/evidence/verification-upgrade-2026-11/07-disc-formats.json";

oadec_diag::catalog! {
    /// A check `verify` makes on a TrueHD stream.
    pub enum RuleId;
    /// The access unit could not be framed.
    AuFraming {
        id: "au.framing",
        severity: Fatal,
        legacy: Some("framing_errors"),
        summary: "the access unit cannot be framed: its length, its major sync, its directory or an end pointer",
        provenance: [spec(DOLBY_HL, "3.1, 4.1, 4.5"), measured(MATRIX)],
    },
    /// Header nibble parity.
    AuHeaderParity {
        id: "au.header_parity",
        severity: Error,
        legacy: Some("header_parity"),
        summary: "the check nibble does not fold the access-unit header and directory to 0xF",
        provenance: [spec(DOLBY_HL, "4.1"), measured(MATRIX)],
    },
    /// `restart_nonexistent` against the segment.
    AuRestartFlag {
        id: "au.restart_flag",
        severity: Error,
        legacy: Some("restart_flag"),
        summary: "a directory restart_nonexistent flag contradicts its segment",
        provenance: [spec(DOLBY_HL, "4.5")],
    },
    /// Major sync CRC.
    SyncCrc {
        id: "sync.crc",
        severity: Error,
        legacy: Some("major_sync_crc"),
        summary: "a major sync fails its CRC-16",
        provenance: [spec(DOLBY_HL, "2.8, 4.2"), measured(MATRIX)],
    },
    /// Major sync signature.
    SyncSignature {
        id: "sync.signature",
        severity: Error,
        legacy: Some("major_sync_signature"),
        summary: "a major sync's signature is not 0xB752",
        provenance: [spec(DOLBY_HL, "4.2")],
    },
    /// Sync lost and found.
    SyncResync {
        id: "sync.resync",
        severity: Error,
        legacy: Some("resyncs"),
        summary: "sync was lost and found again at a later major sync; the units between are gone",
        provenance: [measured(CORRUPTION)],
    },
    /// Bytes skipped while hunting for sync.
    SyncSkippedBytes {
        id: "sync.skipped_bytes",
        severity: Error,
        legacy: Some("skipped_bytes"),
        summary: "bytes that belong to no access unit were skipped (the magnitude is the byte count)",
        provenance: [measured(CORRUPTION), measured(SMOKE)],
    },
    /// Bytes after the last access unit.
    SyncTrailingBytes {
        id: "sync.trailing_bytes",
        severity: Error,
        legacy: Some("trailing_bytes"),
        summary: "bytes after the last access unit do not make one: a stream cut inside a unit (the magnitude is the byte count)",
        provenance: [measured(SMOKE)],
    },
    /// The DVD-Audio pattern in a TrueHD stream.
    SyncForeignPattern {
        id: "sync.foreign_pattern",
        severity: Info,
        legacy: None,
        summary: "the DVD-Audio (FBB) sync pattern met inside a TrueHD stream and passed over",
        provenance: [spec(DOLBY_HL, "3.1"), measured(FOREIGN)],
    },
    /// The configuration changed at a major sync.
    ConfigChanged {
        id: "config.changed",
        severity: Error,
        legacy: Some("config_changes"),
        summary: "a major sync changes the format, the substreams or the rate the first one declared",
        provenance: [measured(RATES)],
    },
    /// Extra-data length parity.
    ExtraHeaderParity {
        id: "extra.header_parity",
        severity: Error,
        legacy: Some("extra_header_parity"),
        summary: "the length check nibble of the extra data does not fold to 0xF",
        provenance: [spec(DOLBY_HL, "4.8")],
    },
    /// Extra data longer than the unit.
    ExtraTruncated {
        id: "extra.truncated",
        severity: Error,
        legacy: Some("extra_truncated"),
        summary: "the extra data declares more words than the access unit holds",
        provenance: [spec(DOLBY_HL, "4.8")],
    },
    /// Extra-data parity byte.
    ExtraParity {
        id: "extra.parity",
        severity: Error,
        legacy: Some("extra_evolution_parity"),
        summary: "the parity byte of an Evolution block does not match",
        provenance: [spec(DOLBY_HL, "4.8"), measured(MATRIX)],
    },
    /// Extra-data padding.
    ExtraPadding {
        id: "extra.padding",
        severity: Error,
        legacy: Some("extra_padding_nonzero"),
        summary: "padding in the extra data is not zero",
        provenance: [measured(MATRIX)],
    },
    /// The Evolution frame as an EMDF container.
    EvoContainer {
        id: "evo.container",
        severity: Error,
        legacy: Some("evolution_container_errors"),
        summary: "an Evolution frame does not open as an EMDF container",
        provenance: [spec(TS_102_366, "Annex H"), measured(MATRIX)],
    },
    /// Object Audio Metadata.
    EvoOamd {
        id: "evo.oamd",
        severity: Error,
        legacy: Some("oamd_errors"),
        summary: "an Object Audio Metadata payload does not parse",
        provenance: [spec(TS_103_420, "clause 5"), measured(MATRIX)],
    },
    /// A segment that does not parse.
    SegmentSyntax {
        id: "segment.syntax",
        severity: Fatal,
        legacy: Some("substream_errors"),
        summary: "a substream segment does not parse (restart header and its CRC, block header, filters, samples); the substream is read again from its next restart",
        provenance: [spec(DOLBY_HL, "2.8, 4.6"), measured(MATRIX)],
    },
    /// Segment parity.
    SegmentParity {
        id: "segment.parity",
        severity: Error,
        legacy: Some("segment_parity"),
        summary: "a segment's parity byte does not match",
        provenance: [spec(DOLBY_HL, "4.6"), measured(MATRIX)],
    },
    /// Segment CRC.
    SegmentCrc {
        id: "segment.crc",
        severity: Error,
        legacy: Some("segment_crc"),
        summary: "a segment's CRC-8 does not match",
        provenance: [spec(DOLBY_HL, "4.6"), measured(MATRIX)],
    },
    /// Segment end against its pointer.
    SegmentEndPointer {
        id: "segment.end_pointer",
        severity: Error,
        legacy: Some("segment_end"),
        summary: "a segment does not end where its end pointer says",
        provenance: [spec(DOLBY_HL, "4.5")],
    },
    /// Samples per segment.
    SegmentSampleCount {
        id: "segment.sample_count",
        severity: Error,
        legacy: Some("sample_count"),
        summary: "the blocks of a segment do not add up to one access unit of samples",
        provenance: [measured(MATRIX)],
    },
    /// Termination word tail.
    SegmentTerminator {
        id: "segment.terminator",
        severity: Error,
        legacy: Some("terminator_tail"),
        summary: "a termination word ends in neither 0x1234 nor a zero-sample count",
        provenance: [spec(DOLBY_HL, "4.6")],
    },
    /// Something other than a termination word where one fits.
    SegmentUnexpectedTail {
        id: "segment.unexpected_tail",
        severity: Error,
        legacy: Some("unexpected_tail"),
        summary: "a segment has room for a termination word and holds something else",
        provenance: [spec(DOLBY_HL, "4.6"), measured(MATRIX)],
    },
    /// `block_data_bits`.
    BlockDataBits {
        id: "block.data_bits",
        severity: Error,
        legacy: Some("block_data_bits"),
        summary: "a block's coded size differs from its block_data_bits",
        provenance: [measured(MATRIX)],
    },
    /// A timing jump that is not a valid seamless branch.
    TimingInvalidBranch {
        id: "timing.invalid_branch",
        severity: Error,
        legacy: Some("invalid_branches"),
        summary: "a timing jump that is not a valid seamless branch; the stream restarts there",
        provenance: [spec(DOLBY_HL, "2.7"), measured(BRANCHES)],
    },
    /// Major syncs too far apart.
    SyncMajorSyncInterval {
        id: "sync.major_sync_interval",
        severity: Warning,
        legacy: None,
        summary: "more than 128 access units since the last major sync",
        provenance: [spec(DOLBY_HL, "2.5"), measured(CENSUS)],
    },
    /// Major syncs too close together.
    SyncMajorSyncSpacing {
        id: "sync.major_sync_spacing",
        severity: Warning,
        legacy: None,
        summary: "fewer than eight access units since the last major sync, other than at the start of the stream or at a branch",
        provenance: [spec(DOLBY_HL, "2.5"), measured(CENSUS)],
    },
    /// A minor sync that could be taken for a major one.
    AuSyncEmulation {
        id: "au.sync_emulation",
        severity: Warning,
        legacy: None,
        summary: "the directory of a minor-sync unit begins with 0xF872, which an encoder must avoid",
        provenance: [spec(DOLBY_HL, "4.5"), measured(CENSUS)],
    },
    /// Blocks per segment.
    SegmentBlockCount {
        id: "segment.block_count",
        severity: Warning,
        legacy: None,
        summary: "more blocks in a segment than its restart sync word allows: 4 for 0x31EA, 2 for 0x31EB and 0x31EC",
        provenance: [spec(DOLBY_HL, "4.6"), measured(CENSUS)],
    },
    /// Added samples of a termination word.
    SegmentZeroSamples {
        id: "segment.zero_samples",
        severity: Warning,
        legacy: None,
        summary: "a termination word claims a whole access unit or more of added samples",
        provenance: [spec(DOLBY_HL, "4.6"), measured(CENSUS)],
    },
    /// Output timing across the substreams of a unit.
    TimingOutputTimingDisagree {
        id: "timing.output_timing_disagree",
        severity: Warning,
        legacy: None,
        summary: "the restart headers of one access unit disagree on output_timing",
        provenance: [spec(DOLBY_HL, "2.7"), measured(CENSUS)],
    },
    /// Bytes after the extra data.
    ExtraTrailingBytes {
        id: "extra.trailing_bytes",
        severity: Error,
        legacy: None,
        summary: "bytes after the extra-data block that its length does not cover, inside the access unit",
        provenance: [spec(DOLBY_HL, "4.8"), measured(CENSUS)],
    },
    /// The length of an Evolution frame against its syntax.
    EvoLengthMismatch {
        id: "evo.length_mismatch",
        severity: Error,
        legacy: None,
        summary: "an Evolution frame's declared length is not the length its container's syntax takes",
        provenance: [measured(CENSUS)],
    },
    /// The object programme of the metadata against the major sync.
    EvoProgramMismatch {
        id: "evo.program_mismatch",
        severity: Warning,
        legacy: None,
        summary: "the object metadata describes another bed or object count than the major sync declares",
        provenance: [spec(DOLBY_HL, "4.3"), spec(TS_103_420, "clause 5"), measured(CENSUS)],
    },
    /// A payload the object programme cannot take.
    EvoOamdUnusable {
        id: "evo.oamd_unusable",
        severity: Error,
        legacy: None,
        summary: "an Object Audio Metadata payload has no object element, or describes other beds or another number of objects than the first payload of the stream: an object output can make nothing of it and says so",
        provenance: [measured(MATRIX)],
    },
    /// The reserved primary protection length.
    EvoProtectionReserved {
        id: "evo.protection_reserved",
        severity: Warning,
        legacy: None,
        summary: "an Evolution container codes its primary protection length as 00, which ETSI TS 102 366 reserves",
        provenance: [spec(TS_102_366, "H.2.2.4.1, Table H.2.5"), measured(PROTECTION)],
    },
    /// An Evolution container of another syntax version.
    EvoSyntaxVersion {
        id: "evo.syntax_version",
        severity: Info,
        legacy: None,
        summary: "an Evolution container declares a syntax version other than 0, the version of the syntax it is read by",
        provenance: [spec(TS_102_366, "H.2.2.2.2"), measured(DECLARED)],
    },
    /// An Evolution container that declares another protection than the one
    /// before it.
    EvoProtectionChanged {
        id: "evo.protection_changed",
        severity: Info,
        legacy: None,
        summary: "an Evolution container declares another key identifier, or another length of a protection word, than the container before it: what is counted is what is declared, and no word is read",
        provenance: [spec(TS_102_366, "H.2.2.2.3, H.2.2.4"), measured(DECLARED)],
    },
    /// A substream's depth in the decoder buffer.
    BufferSubstreamDepth {
        id: "buffer.substream_depth",
        severity: Warning,
        legacy: None,
        summary: "a substream fills the decoder buffer beyond 15 000 bytes for each channel its restart header spans",
        provenance: [measured(FIFO)],
    },
    /// The whole stream's depth in the decoder buffer.
    BufferStreamDepth {
        id: "buffer.stream_depth",
        severity: Warning,
        legacy: None,
        summary: "the stream fills the decoder buffer beyond the 120 000 bytes every decoder has",
        provenance: [spec(DOLBY_HL, "2.7"), measured(FIFO)],
    },
    /// The delay through the buffer, upper bound.
    BufferLatencyOver75ms {
        id: "buffer.latency_over_75ms",
        severity: Warning,
        legacy: None,
        summary: "an access unit waits more than 75 ms in the decoder buffer before its output",
        provenance: [measured(FIFO), secondary("the bound Dolby's TrueHD encoder writes to (74.2 ms in its report) and states in its messages")],
    },
    /// The delay through the buffer, lower bound.
    BufferLatencyUnderFrame {
        id: "buffer.latency_under_frame",
        severity: Warning,
        legacy: None,
        summary: "an access unit arrives less than one frame before its output",
        provenance: [measured(FIFO), secondary("a bound Dolby's TrueHD encoder states in its messages")],
    },
    /// Delivery over the declared peak.
    RateOverDeclaredPeak {
        id: "rate.over_declared_peak",
        severity: Warning,
        legacy: None,
        summary: "an access unit is delivered faster than the peak data rate its major sync declares",
        provenance: [spec(DOLBY_HL, "2.7, 4.2"), measured(FIFO)],
    },
    /// Delivery over the format's ceiling.
    RateOverCeiling {
        id: "rate.over_ceiling",
        severity: Warning,
        legacy: None,
        summary: "an access unit is delivered faster than 18 Mbit/s",
        provenance: [spec(DOLBY_HL, "2.7"), measured(FIFO)],
    },
    /// `verify --require-disc`: a disc format the stream does not fit.
    DiscRequired {
        id: "disc.required",
        severity: Error,
        legacy: None,
        summary: "verify --require-disc: the stream does not fit a disc format the run requires, or its fit cannot be judged",
        provenance: [spec(BDA, "3.4.1, Figure 3-4"), spec(DOLBY_HL, "1, 2.7"), measured(DISC), secondary("the verifier messages of Dolby's TrueHD encoder, for HD DVD-Video at 176.4 and 192 kHz")],
    },
    /// `verify --decode`: the lossless check word.
    DecodeLosslessCheck {
        id: "decode.lossless_check",
        severity: Error,
        legacy: None,
        summary: "verify --decode: a restart header's lossless check word disagrees with the decoded samples",
        provenance: [measured(MATRIX)],
    },
    /// `verify --decode`: a presentation that stopped.
    DecodeStopped {
        id: "decode.stopped",
        severity: Fatal,
        legacy: None,
        summary: "verify --decode: a presentation's decode stopped before the end of the stream",
        provenance: [measured(MATRIX)],
    },
    /// `verify --decode`: `max_bits`.
    DecodeMaxBits {
        id: "decode.max_bits",
        severity: Error,
        legacy: None,
        summary: "verify --decode: decoded samples need more bits than their restart header's max_bits allows",
        provenance: [measured(MATRIX)],
    },
    /// `verify --decode`: samples beyond 24 bits.
    DecodeClipped {
        id: "decode.clipped",
        severity: Error,
        legacy: None,
        summary: "verify --decode: decoded samples beyond 24 bits, which every writer can only clamp",
        provenance: [measured(MATRIX)],
    },
}

/// The catalog as the Markdown of `docs/conformance-rules.md`.
#[must_use]
pub fn markdown() -> String {
    let mut out = String::from(
        "# Conformance rules\n\
         \n\
         Every check `oadec verify` makes on a TrueHD stream, with its name, the\n\
         severity a failure carries by default and the source the rule rests on.\n\
         This file is generated from `crates/oadec-truehd/src/rules.rs` (`oadec rules\n\
         --markdown`); a test keeps the two equal.\n\
         \n\
         A run fails when its worst finding is at or above `--fail-on` (default\n\
         `error`). The source decides how heavy a rule may be: a rule a public\n\
         specification states, or one measured on real streams, may fail a stream;\n\
         a rule only another implementation or tool states (`secondary`) warns at\n\
         most, and fails a stream only when asked to with `--fail-on warning`.\n\
         \n\
         | Rule | Default | What a failure means | Source | Counter |\n\
         |---|---|---|---|---|\n",
    );
    out.push_str(&oadec_diag::markdown_rows::<RuleId>());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::diag::Severity;

    /// What `oadec-diag` holds every catalog to, held here to this one.
    #[test]
    fn the_catalog_has_none_of_the_faults_of_a_catalog() {
        assert_eq!(oadec_diag::faults::<RuleId>(), Vec::<String>::new());
    }

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
    }

    #[test]
    fn names_are_unique_and_well_formed() {
        let mut seen = std::collections::BTreeSet::new();
        for rule in RuleId::ALL {
            let id = rule.info().id;
            assert!(seen.insert(id), "{id} twice");
            let parts: Vec<&str> = id.split('.').collect();
            assert!(parts.len() >= 2, "{id}: no domain");
            for part in parts {
                assert!(
                    !part.is_empty()
                        && part
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "{id}: not lowercase words"
                );
            }
            assert_eq!(RuleId::from_id(id), Some(*rule));
        }
        let mut counters = std::collections::BTreeSet::new();
        for rule in RuleId::ALL {
            if let Some(c) = rule.info().legacy_counter {
                assert!(counters.insert(c), "counter {c} twice");
                assert_eq!(RuleId::from_legacy_counter(c), Some(*rule));
            }
        }
    }

    /// The source decides how heavy a rule may be.
    #[test]
    fn a_rule_only_another_implementation_states_cannot_fail_a_stream_by_default() {
        for rule in RuleId::ALL {
            let info = rule.info();
            let independent = info
                .provenance
                .iter()
                .any(|p| matches!(p, Provenance::Spec { .. } | Provenance::Measured { .. }));
            if info.severity >= Severity::Error {
                assert!(
                    independent,
                    "{} is {} on a secondary source alone",
                    info.id,
                    info.severity.name()
                );
            }
            if !independent {
                assert!(info.severity <= Severity::Warning, "{}", info.id);
            }
        }
    }

    #[test]
    fn every_measured_source_is_in_the_repository() {
        for rule in RuleId::ALL {
            for p in rule.info().provenance {
                if let Provenance::Measured { evidence } = p {
                    assert!(
                        repo_root().join(evidence).is_file(),
                        "{}: {evidence} is not in the repository",
                        rule.info().id
                    );
                }
            }
        }
    }

    /// `docs/conformance-rules.md` opens with the catalog rendered; the
    /// catalog of the AC-3 family follows it, and a test of that crate holds
    /// the rest. `oadec rules --markdown` writes the whole document.
    #[test]
    fn the_rules_document_is_the_catalog() {
        let path = repo_root().join("docs").join("conformance-rules.md");
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            on_disk.starts_with(&markdown()),
            "docs/conformance-rules.md is not the catalog: write it again with `oadec rules --markdown`"
        );
    }
}
