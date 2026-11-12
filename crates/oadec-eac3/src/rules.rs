//! The catalog of the checks `verify` makes on a stream of the AC-3 family.
//!
//! Every check of the pass over an AC-3 or E-AC-3 stream was a counter, and
//! what a counter meant was said where it was printed. Each is a rule here,
//! as the checks of a TrueHD stream are: a stable name, the severity a
//! failure carries by default, the counter of `verify --json` it is counted
//! under, and the source it rests on. `oadec-diag` holds the catalog to what
//! it holds every catalog to.
//!
//! One thing a rule of this family says that a TrueHD rule does not:
//! whether a failure is a fault of what a decode delivers
//! ([`RuleId::faults_a_delivery`]). `verify` is for conformance, and fails a
//! stream for things that change neither the audio nor the metadata a decode
//! hands over; a decode of the same stream keeps its verdict
//! (`docs/exit-codes.md`).

pub use oadec_diag::{Provenance, RuleInfo};
use oadec_diag::{measured, spec};

/// ETSI TS 102 366 V1.4.1 (AC-3 and E-AC-3).
pub const TS_102_366: &str = "ETSI TS 102 366 V1.4.1";

/// ETSI TS 103 420 V1.2.1 (object audio metadata and joint object coding in
/// E-AC-3).
pub const TS_103_420: &str = "ETSI TS 103 420 V1.2.1";

const MATRIX: &str = "docs/audit/conformance-matrix.md";
const PARITY: &str = "docs/audit/evidence/engine-round-2026-11/22-verdict-parity.json";

oadec_diag::catalog! {
    /// A check `verify` makes on a stream of the AC-3 family.
    pub enum RuleId;
    /// Bytes that are no syncframe where one has to start.
    FrameSync {
        id: "frame.sync",
        severity: Error,
        legacy: Some("sync_errors"),
        summary: "a place where a syncframe has to start and none does: no sync word, a header that cannot be read, or a frame that the next sync word does not follow",
        provenance: [spec(TS_102_366, "4.3.1, E.1.2.2"), measured(MATRIX)],
    },
    /// Bytes of the stream that belong to no syncframe.
    FrameSkipped {
        id: "frame.skipped",
        severity: Error,
        legacy: Some("skipped_bytes"),
        summary: "bytes of the stream that belong to no syncframe, passed over to find the next or left at the end",
        provenance: [spec(TS_102_366, "4.3.1, E.1.2.2"), measured(MATRIX)],
    },
    /// A frame of the programme's independent substream would not decode.
    FrameDecode {
        id: "frame.decode",
        severity: Error,
        legacy: Some("decode_errors"),
        summary: "a syncframe of the independent substream whose syntax cannot be decoded",
        provenance: [spec(TS_102_366, "4.3, E.1.2"), measured(MATRIX)],
    },
    /// The frame CRC.
    FrameCrc {
        id: "frame.crc",
        severity: Error,
        legacy: Some("crc_failures"),
        summary: "the CRC-16 over a syncframe of the independent substream does not hold",
        provenance: [spec(TS_102_366, "6.10.1"), measured(MATRIX)],
    },
    /// The audio blocks reach into the bits that close the frame.
    FrameTailOverrun {
        id: "frame.tail_overrun",
        severity: Error,
        legacy: Some("tail_overruns"),
        summary: "the audio blocks of a syncframe end inside the eighteen bits that close it; the frame decodes, and a decode keeps its verdict",
        provenance: [spec(TS_102_366, "4.4.4"), measured(MATRIX)],
    },
    /// A frame of a dependent substream would not decode.
    SubstreamDecode {
        id: "substream.decode",
        severity: Error,
        legacy: Some("substream_decode_errors"),
        summary: "a syncframe of a dependent substream whose syntax cannot be decoded",
        provenance: [spec(TS_102_366, "E.1.2, E.1.3.1.2"), measured(MATRIX)],
    },
    /// The frame CRC of a dependent substream.
    SubstreamCrc {
        id: "substream.crc",
        severity: Error,
        legacy: Some("substream_crc_failures"),
        summary: "the CRC-16 over a syncframe of a dependent substream does not hold",
        provenance: [spec(TS_102_366, "6.10.1"), measured(MATRIX)],
    },
    /// The audio blocks of a dependent frame reach into its closing bits.
    SubstreamTailOverrun {
        id: "substream.tail_overrun",
        severity: Error,
        legacy: Some("substream_tail_overruns"),
        summary: "the audio blocks of a syncframe of a dependent substream end inside the eighteen bits that close it; a decode keeps its verdict",
        provenance: [spec(TS_102_366, "4.4.4"), measured(MATRIX)],
    },
    /// A dependent frame whose channels did not reach the programme.
    ProgramDependentDropped {
        id: "program.dependent_dropped",
        severity: Error,
        legacy: Some("dependent_dropped"),
        summary: "a frame of a dependent substream whose channels did not reach the programme, for whatever reason",
        provenance: [spec(TS_102_366, "E.2.8.2"), measured(MATRIX)],
    },
    /// A dependent frame with no independent substream in front of it.
    ProgramOrphanDependent {
        id: "program.orphan_dependent",
        severity: Error,
        legacy: Some("orphan_dependents"),
        summary: "a frame of a dependent substream with no independent substream before it to belong to",
        provenance: [spec(TS_102_366, "E.2.8.2"), measured(MATRIX)],
    },
    /// A dependent frame that does not line up with its independent one.
    ProgramMisaligned {
        id: "program.misaligned",
        severity: Error,
        legacy: Some("misaligned_substreams"),
        summary: "a frame of a dependent substream whose block count or sample rate is not that of its independent substream",
        provenance: [spec(TS_102_366, "E.1.3.1.2"), measured(MATRIX)],
    },
    /// A channel map that cannot be read.
    ProgramLocation {
        id: "program.location",
        severity: Error,
        legacy: Some("location_errors"),
        summary: "a frame of a dependent substream whose custom channel map cannot be read",
        provenance: [spec(TS_102_366, "E.1.3.1.7, E.1.3.1.8, Table E.1.4"), measured(MATRIX)],
    },
    /// More channels than a programme may carry.
    ProgramOverCapacity {
        id: "program.over_capacity",
        severity: Error,
        legacy: Some("channels_over_capacity"),
        summary: "a channel past the sixteen a programme may carry, which was dropped",
        provenance: [spec(TS_102_366, "E.2.8.2"), measured(MATRIX)],
    },
    /// The channels of the programme changed.
    ProgramLayoutChange {
        id: "program.layout_change",
        severity: Error,
        legacy: Some("layout_changes"),
        summary: "a frame group whose channels are not the channels the programme began with",
        provenance: [measured(MATRIX)],
    },
    /// Two frames of one substream in one group.
    ProgramDuplicateFrame {
        id: "program.duplicate_frame",
        severity: Error,
        legacy: Some("duplicate_substream_frames"),
        summary: "a frame of a substream that already had a frame in its group; only the first reaches the decoder",
        provenance: [spec(TS_102_366, "E.1.3.1.2"), measured(MATRIX)],
    },
    /// A substream that stopped supplying frames.
    ProgramStalledSubstream {
        id: "program.stalled_substream",
        severity: Error,
        legacy: Some("stalled_substreams"),
        summary: "a substream that stopped supplying frames and was flushed so that the groups after it could be delivered",
        provenance: [spec(TS_102_366, "E.1.3.1.2"), measured(MATRIX)],
    },
    /// A group delivered without one of its frames.
    ProgramMissingFrame {
        id: "program.missing_frame",
        severity: Error,
        legacy: Some("missing_substream_frames"),
        summary: "a frame group delivered without the frame of one of its substreams, which never came out of its decoder",
        provenance: [spec(TS_102_366, "E.1.3.1.2"), measured(MATRIX)],
    },
    /// A frame without its EMDF container.
    EmdfMissingContainer {
        id: "emdf.missing_container",
        severity: Error,
        legacy: Some("missing_containers"),
        summary: "in a substream that carries EMDF, a syncframe whose skip fields hold no container that opens: erased, broken, or of a length its syntax disagrees with",
        provenance: [spec(TS_103_420, "8.2, 8.3.1"), spec(TS_102_366, "H.1"), measured(MATRIX)],
    },
    /// A payload configuration outside the table.
    EmdfPayloadConfig {
        id: "emdf.payload_config",
        severity: Error,
        legacy: Some("payload_config_violations"),
        summary: "a field of the configuration of an Object Audio Metadata or JOC payload outside what the table fixes; a decode keeps its verdict",
        provenance: [spec(TS_103_420, "Table 56"), measured(MATRIX)],
    },
    /// The complexity index and the object total disagree.
    BsiComplexityIndex {
        id: "bsi.complexity_index",
        severity: Error,
        legacy: Some("complexity_mismatches"),
        summary: "complexity_index_type_a is not the total of beds and objects the Object Audio Metadata declares, or is more than 16; a decode keeps its verdict",
        provenance: [spec(TS_103_420, "8.3.2.2"), measured(MATRIX)],
    },
    /// An Object Audio Metadata payload that will not parse.
    OamdParse {
        id: "oamd.parse",
        severity: Error,
        legacy: Some("oamd_errors"),
        summary: "an Object Audio Metadata payload that cannot be parsed",
        provenance: [spec(TS_103_420, "clause 5"), measured(MATRIX)],
    },
    /// A payload an object output can make nothing of.
    OamdUnusable {
        id: "oamd.unusable",
        severity: Error,
        legacy: Some("oamd_unusable"),
        summary: "an Object Audio Metadata payload that parses and has no object element, or describes other beds or another number of objects than the first payload of the stream: an object output can make nothing of it and says so",
        provenance: [measured(PARITY), measured(MATRIX)],
    },
    /// An element whose declared size is not its size.
    OamdStaleSize {
        id: "oamd.stale_size",
        severity: Warning,
        legacy: Some("oamd_stale_sizes"),
        summary: "an element of an Object Audio Metadata payload whose declared size is not the size its syntax takes; counted, and no verdict rests on it",
        provenance: [spec(TS_103_420, "clause 5"), measured(MATRIX)],
    },
    /// A JOC payload that will not parse.
    JocParse {
        id: "joc.parse",
        severity: Error,
        legacy: Some("joc_errors"),
        summary: "a JOC payload that cannot be parsed",
        provenance: [spec(TS_103_420, "clause 6"), measured(MATRIX)],
    },
    /// A reserved extension configuration.
    JocReservedExtConfig {
        id: "joc.reserved_ext_config",
        severity: Error,
        legacy: Some("joc_reserved_ext_config"),
        summary: "a JOC payload whose joc_ext_config_idx is reserved: what follows cannot be read, and the matrices of the frame before are held in its place",
        provenance: [spec(TS_103_420, "Table 49"), measured(MATRIX)],
    },
    /// A JOC payload of another size than it declares.
    JocSizeMismatch {
        id: "joc.size_mismatch",
        severity: Error,
        legacy: Some("joc_size_mismatches"),
        summary: "a JOC payload whose declared size is not the size its syntax takes",
        provenance: [spec(TS_103_420, "clause 6"), measured(MATRIX)],
    },
}

impl RuleId {
    /// Whether a failure of this rule is a fault of what a decode delivers.
    ///
    /// A frame that ends inside its own tail decodes to audio three decoders
    /// agree on; a payload configuration outside the table and a complexity
    /// index that disagrees with the object total change neither the audio
    /// nor the metadata. `verify` counts them and exits 7, and a decode of
    /// the same stream keeps its verdict (`docs/exit-codes.md`). A rule that
    /// fails no stream by default is no fault of a delivery either.
    #[must_use]
    pub fn faults_a_delivery(self) -> bool {
        !matches!(
            self,
            Self::FrameTailOverrun
                | Self::SubstreamTailOverrun
                | Self::EmdfPayloadConfig
                | Self::BsiComplexityIndex
        ) && self.info().severity >= oadec_diag::Severity::Error
    }
}

/// The catalog as the part of `docs/conformance-rules.md` that is about the
/// AC-3 family.
#[must_use]
pub fn markdown() -> String {
    let mut out = String::from(
        "\n## AC-3 and E-AC-3\n\
         \n\
         Every check `oadec verify` makes on an AC-3 or E-AC-3 stream. This part is\n\
         generated from `crates/oadec-eac3/src/rules.rs`. The last column says\n\
         whether a failure is a fault of what a decode delivers: `verify` fails a\n\
         stream for the others too, and a decode of the same stream keeps its\n\
         verdict (`docs/exit-codes.md`).\n\
         \n\
         | Rule | Default | What a failure means | Source | Counter | A fault of a delivery |\n\
         |---|---|---|---|---|---|\n",
    );
    let rows = oadec_diag::markdown_rows::<RuleId>();
    for (rule, row) in RuleId::ALL.iter().zip(rows.lines()) {
        let delivery = if rule.faults_a_delivery() {
            "yes"
        } else {
            "no"
        };
        out.push_str(row);
        out.push_str(&format!(" {delivery} |\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use oadec_diag::Severity;
    use std::path::Path;

    fn repo_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
    }

    /// What `oadec-diag` holds every catalog to: a name used once and well
    /// formed, a counter for one rule, no rule heavier than its source
    /// carries.
    #[test]
    fn the_catalog_has_none_of_the_faults_of_a_catalog() {
        assert_eq!(oadec_diag::faults::<RuleId>(), Vec::<String>::new());
        for rule in RuleId::ALL {
            assert_eq!(RuleId::from_id(rule.info().id), Some(*rule));
            // every check of the pass was a counter before it had a name
            let counter = rule.info().legacy_counter.expect("a counter");
            assert_eq!(RuleId::from_legacy_counter(counter), Some(*rule));
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

    /// The three things `verify` counts and a delivery does not, the fourth
    /// being the same thing in a dependent substream; and a rule that fails
    /// no stream.
    #[test]
    fn what_is_no_fault_of_a_delivery_is_these_and_no_other() {
        let not: Vec<&str> = RuleId::ALL
            .iter()
            .filter(|r| !r.faults_a_delivery())
            .map(|r| r.info().id)
            .collect();
        assert_eq!(
            not,
            [
                "frame.tail_overrun",
                "substream.tail_overrun",
                "emdf.payload_config",
                "bsi.complexity_index",
                "oamd.stale_size",
            ]
        );
        for rule in RuleId::ALL {
            if rule.info().severity < Severity::Error {
                assert!(!rule.faults_a_delivery(), "{}", rule.info().id);
            }
        }
    }

    /// The part of `docs/conformance-rules.md` about this family is the
    /// catalog rendered: the document ends with it. `oadec rules --markdown`
    /// writes the whole document.
    #[test]
    fn the_rules_document_ends_with_the_catalog() {
        let path = repo_root().join("docs").join("conformance-rules.md");
        let on_disk = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .replace("\r\n", "\n");
        assert!(
            on_disk.ends_with(&markdown()),
            "docs/conformance-rules.md is not the catalog: write it again with `oadec rules --markdown`"
        );
        assert_eq!(
            markdown().lines().filter(|l| l.starts_with("| `")).count(),
            RuleId::ALL.len()
        );
    }
}
