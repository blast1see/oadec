//! The catalog of the checks `verify` makes on a TrueHD stream.
//!
//! Every rule has a stable name, the severity a failure carries by default and
//! the source the rule rests on. The source matters as much as the check: a
//! rule taken from another implementation and confirmed by nothing else may
//! warn, and may never fail a stream by default, while a rule a public
//! specification states, or one measured on real streams, may. A unit test
//! below holds the catalog to that, and `docs/conformance-rules.md` is the
//! catalog rendered (a second test keeps the two equal).

use crate::diag::Severity;

/// Dolby's public high-level description of the FBA bitstream.
pub const DOLBY_HL: &str = "Dolby TrueHD (MLP) High-Level Bitstream Description (2018)";

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

/// Where a rule comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// A public specification: the document and the clause.
    Spec {
        /// The document.
        doc: &'static str,
        /// The clause or clauses.
        clause: &'static str,
    },
    /// Measured on real streams; the evidence, a path in the repository.
    Measured {
        /// The evidence file.
        evidence: &'static str,
    },
    /// Stated by another implementation or tool, confirmed by neither of the
    /// above.
    Secondary {
        /// Who states it.
        source: &'static str,
    },
}

impl Provenance {
    /// A short name for the kind of source.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Spec { .. } => "spec",
            Self::Measured { .. } => "measured",
            Self::Secondary { .. } => "secondary",
        }
    }
}

impl core::fmt::Display for Provenance {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Spec { doc, clause } => write!(f, "{doc}, {clause}"),
            Self::Measured { evidence } => write!(f, "measured: {evidence}"),
            Self::Secondary { source } => write!(f, "secondary: {source}"),
        }
    }
}

const fn spec(doc: &'static str, clause: &'static str) -> Provenance {
    Provenance::Spec { doc, clause }
}

const fn measured(evidence: &'static str) -> Provenance {
    Provenance::Measured { evidence }
}

#[allow(dead_code)]
const fn secondary(source: &'static str) -> Provenance {
    Provenance::Secondary { source }
}

/// What the catalog says about one rule.
#[derive(Debug, PartialEq, Eq)]
pub struct RuleInfo {
    /// The stable name, `domain.rule`.
    pub id: &'static str,
    /// The severity of a failure by default.
    pub severity: Severity,
    /// The counter of `verify --json` that counted this rule before it had a
    /// name, if it had one.
    pub legacy_counter: Option<&'static str>,
    /// One line on what a failure means.
    pub summary: &'static str,
    /// What the rule rests on.
    pub provenance: &'static [Provenance],
}

macro_rules! catalog {
    ($(
        $(#[doc = $doc:literal])*
        $variant:ident {
            id: $id:literal,
            severity: $sev:ident,
            legacy: $legacy:expr,
            summary: $summary:literal,
            provenance: [$($prov:expr),+ $(,)?] $(,)?
        }
    ),+ $(,)?) => {
        /// A check `verify` makes on a TrueHD stream.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum RuleId {
            $( $(#[doc = $doc])* $variant, )+
        }

        impl RuleId {
            /// Every rule, in catalog order.
            pub const ALL: &'static [RuleId] = &[ $( RuleId::$variant, )+ ];

            /// What the catalog says about this rule.
            #[must_use]
            pub fn info(self) -> &'static RuleInfo {
                match self {
                    $( RuleId::$variant => {
                        static INFO: RuleInfo = RuleInfo {
                            id: $id,
                            severity: Severity::$sev,
                            legacy_counter: $legacy,
                            summary: $summary,
                            provenance: &[$($prov),+],
                        };
                        &INFO
                    } )+
                }
            }
        }
    };
}

catalog! {
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
}

impl RuleId {
    /// The rule named `id`.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.info().id == id)
    }

    /// The rule that counts under the legacy counter `name`.
    #[must_use]
    pub fn from_legacy_counter(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.info().legacy_counter == Some(name))
    }
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
    for rule in RuleId::ALL {
        let info = rule.info();
        let sources = info
            .provenance
            .iter()
            .map(|p| match p {
                Provenance::Measured { evidence } => format!("measured: `{evidence}`"),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join("; ");
        let counter = info
            .legacy_counter
            .map_or_else(|| "--".to_string(), |c| format!("`{c}`"));
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} |\n",
            info.id,
            info.severity.name(),
            info.summary,
            sources,
            counter
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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

    /// `docs/conformance-rules.md` is the catalog rendered. Run the test with
    /// `OADEC_BLESS=1` to write it.
    #[test]
    fn the_rules_document_is_the_catalog() {
        let path = repo_root().join("docs").join("conformance-rules.md");
        let rendered = markdown();
        if std::env::var_os("OADEC_BLESS").is_some() {
            std::fs::write(&path, &rendered).unwrap();
        }
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert_eq!(
            on_disk, rendered,
            "docs/conformance-rules.md is not the catalog; run the test with OADEC_BLESS=1"
        );
    }
}
