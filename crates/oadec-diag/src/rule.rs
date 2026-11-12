//! What a catalog says about a rule, and what a catalog is to the code
//! that tallies its findings.
//!
//! Every rule has a stable name, the severity a failure carries by default and
//! the source the rule rests on. The source matters as much as the check: a
//! rule taken from another implementation and confirmed by nothing else may
//! warn, and may never fail a stream by default, while a rule a public
//! specification states, or one measured on real streams, may. [`faults`]
//! holds a catalog to that.

use crate::severity::Severity;

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

    /// Whether the source stands without another implementation's word: a
    /// public specification, or a measurement.
    #[must_use]
    pub const fn is_independent(&self) -> bool {
        matches!(self, Self::Spec { .. } | Self::Measured { .. })
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

/// A clause of a public specification.
#[must_use]
pub const fn spec(doc: &'static str, clause: &'static str) -> Provenance {
    Provenance::Spec { doc, clause }
}

/// A measurement on real streams, by the path of its evidence in the
/// repository.
#[must_use]
pub const fn measured(evidence: &'static str) -> Provenance {
    Provenance::Measured { evidence }
}

/// The word of another implementation or tool.
#[must_use]
pub const fn secondary(source: &'static str) -> Provenance {
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

/// A rule of a catalog: one value of the enumeration a format's checks are
/// named by.
///
/// The order of the values is the order of the catalog, which is the order
/// a tally reports its rules in.
pub trait Rule: Copy + Ord + core::fmt::Debug + 'static {
    /// Every rule, in catalog order.
    const ALL: &'static [Self];

    /// What the catalog says about this rule.
    fn info(self) -> &'static RuleInfo;

    /// The rule named `id`.
    #[must_use]
    fn named(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|r| r.info().id == id)
    }

    /// The rule that counts under the legacy counter `name`.
    #[must_use]
    fn counted_as(name: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|r| r.info().legacy_counter == Some(name))
    }
}

/// Writes a catalog: the enumeration of its rules, what it says about each,
/// and the [`Rule`] that lets a [`crate::Tally`] count them.
///
/// ```
/// oadec_diag::catalog! {
///     /// A check on a stream of some format.
///     pub enum Check;
///     /// The frame is not the length it says.
///     FrameLength {
///         id: "frame.length",
///         severity: Error,
///         legacy: Some("length_errors"),
///         summary: "a frame is not the length its header declares",
///         provenance: [oadec_diag::spec("A specification", "4.2")],
///     },
/// }
/// assert_eq!(Check::from_id("frame.length"), Some(Check::FrameLength));
/// assert_eq!(Check::FrameLength.info().severity, oadec_diag::Severity::Error);
/// ```
#[macro_export]
macro_rules! catalog {
    (
        $(#[doc = $enum_doc:literal])*
        $vis:vis enum $name:ident;
        $(
            $(#[doc = $doc:literal])*
            $variant:ident {
                id: $id:literal,
                severity: $sev:ident,
                legacy: $legacy:expr,
                summary: $summary:literal,
                provenance: [$($prov:expr),+ $(,)?] $(,)?
            }
        ),+ $(,)?
    ) => {
        $(#[doc = $enum_doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis enum $name {
            $( $(#[doc = $doc])* $variant, )+
        }

        // a catalog need not use every lookup it is given
        #[allow(dead_code)]
        impl $name {
            /// Every rule, in catalog order.
            pub const ALL: &'static [$name] = <$name as $crate::Rule>::ALL;

            /// What the catalog says about this rule.
            #[must_use]
            pub fn info(self) -> &'static $crate::RuleInfo {
                <$name as $crate::Rule>::info(self)
            }

            /// The rule named `id`.
            #[must_use]
            pub fn from_id(id: &str) -> Option<Self> {
                <$name as $crate::Rule>::named(id)
            }

            /// The rule that counts under the legacy counter `name`.
            #[must_use]
            pub fn from_legacy_counter(name: &str) -> Option<Self> {
                <$name as $crate::Rule>::counted_as(name)
            }
        }

        impl $crate::Rule for $name {
            const ALL: &'static [$name] = &[ $( $name::$variant, )+ ];

            fn info(self) -> &'static $crate::RuleInfo {
                match self {
                    $( $name::$variant => {
                        static INFO: $crate::RuleInfo = $crate::RuleInfo {
                            id: $id,
                            severity: $crate::Severity::$sev,
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

/// What is wrong with the catalog of `R`, whatever its rules are; nothing
/// when it is sound.
///
/// - A name is used once, and is lowercase words and digits under a domain:
///   `domain.rule`.
/// - A legacy counter belongs to one rule.
/// - A rule has a source, and the source decides how heavy the rule may be:
///   one that only another implementation or tool states warns at most.
#[must_use]
pub fn faults<R: Rule>() -> Vec<String> {
    let mut out = Vec::new();
    let mut names = std::collections::BTreeSet::new();
    let mut counters = std::collections::BTreeSet::new();
    for rule in R::ALL {
        let info = rule.info();
        let id = info.id;
        if !names.insert(id) {
            out.push(format!("{id} names two rules"));
        }
        let parts: Vec<&str> = id.split('.').collect();
        if parts.len() < 2 {
            out.push(format!("{id} has no domain"));
        }
        if parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }) {
            out.push(format!("{id} is not lowercase words"));
        }
        if let Some(counter) = info.legacy_counter
            && !counters.insert(counter)
        {
            out.push(format!("the counter {counter} belongs to two rules"));
        }
        if info.provenance.is_empty() {
            out.push(format!("{id} has no source"));
        }
        let independent = info.provenance.iter().any(Provenance::is_independent);
        if !independent && info.severity > Severity::Warning {
            out.push(format!(
                "{id} is {} on a secondary source alone",
                info.severity.name()
            ));
        }
    }
    out
}

/// The rows of the table a catalog is documented in, one for each rule:
/// its name, its default severity, what a failure means, its sources and its
/// legacy counter.
#[must_use]
pub fn markdown_rows<R: Rule>() -> String {
    let mut out = String::new();
    for rule in R::ALL {
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
    use crate::probe::Probe;

    #[test]
    fn a_catalog_names_its_rules_both_ways() {
        assert_eq!(Probe::ALL, [Probe::Light, Probe::Heavy, Probe::Hearsay]);
        for rule in Probe::ALL {
            assert_eq!(Probe::from_id(rule.info().id), Some(*rule));
        }
        assert_eq!(Probe::from_id("probe.none"), None);
        assert_eq!(
            Probe::from_legacy_counter("heavy_things"),
            Some(Probe::Heavy)
        );
        assert_eq!(Probe::from_legacy_counter("light_things"), None);
        assert_eq!(Probe::Heavy.info().severity, Severity::Error);
        assert_eq!(Probe::Heavy.info().provenance.len(), 2);
        // the order of the rules is the order they are written in
        assert!(Probe::Light < Probe::Heavy && Probe::Heavy < Probe::Hearsay);
    }

    #[test]
    fn a_source_is_named_by_its_kind_and_written_whole() {
        let p = spec("A document", "1.2");
        assert_eq!(
            (p.kind(), p.to_string().as_str()),
            ("spec", "A document, 1.2")
        );
        let p = measured("measured/x.json");
        assert_eq!(
            (p.kind(), p.to_string().as_str()),
            ("measured", "measured: measured/x.json")
        );
        let p = secondary("another tool");
        assert_eq!(
            (p.kind(), p.to_string().as_str()),
            ("secondary", "secondary: another tool")
        );
        assert!(spec("d", "c").is_independent() && measured("e").is_independent());
        assert!(!secondary("s").is_independent());
    }

    #[test]
    fn a_sound_catalog_has_no_fault() {
        assert_eq!(faults::<Probe>(), Vec::<String>::new());
    }

    crate::catalog! {
        /// A catalog with everything wrong that a catalog can have wrong.
        enum Unsound;
        /// The first of two rules of one name, and of one counter.
        First {
            id: "bad.twice",
            severity: Info,
            legacy: Some("shared"),
            summary: "the first of two",
            provenance: [crate::spec("A document", "1")],
        },
        /// The second.
        Second {
            id: "bad.twice",
            severity: Info,
            legacy: Some("shared"),
            summary: "the second of two",
            provenance: [crate::spec("A document", "1")],
        },
        /// A name without a domain.
        Bare {
            id: "bare",
            severity: Info,
            legacy: None,
            summary: "a name without a domain",
            provenance: [crate::spec("A document", "1")],
        },
        /// A name that is not lowercase words.
        Shouted {
            id: "bad.Shouted-Name",
            severity: Info,
            legacy: None,
            summary: "a name that is not lowercase words",
            provenance: [crate::spec("A document", "1")],
        },
        /// An error on the word of another tool.
        Borrowed {
            id: "bad.borrowed",
            severity: Error,
            legacy: None,
            summary: "an error on the word of another tool",
            provenance: [crate::secondary("another tool")],
        },
    }

    #[test]
    fn an_unsound_catalog_is_told_each_thing_that_is_wrong_with_it() {
        let found = faults::<Unsound>();
        for expected in [
            "bad.twice names two rules",
            "the counter shared belongs to two rules",
            "bare has no domain",
            "bad.Shouted-Name is not lowercase words",
            "bad.borrowed is error on a secondary source alone",
        ] {
            assert!(
                found.iter().any(|f| f == expected),
                "{expected:?} is not among {found:?}"
            );
        }
        // a warning on the word of another tool is what such a rule may be
        assert!(!found.iter().any(|f| f.contains("probe.")));
        assert!(!found.iter().any(|f| f.starts_with("bad.twice is")));
    }

    #[test]
    fn the_rows_of_a_catalog_are_its_rules_in_order() {
        let rows = markdown_rows::<Probe>();
        assert_eq!(
            rows,
            "| `probe.light` | info | a thing worth knowing | A document, 1.2 | -- |\n\
             | `probe.heavy` | error | a thing that fails a stream | A document, 3; measured: `measured/heavy.json` | `heavy_things` |\n\
             | `probe.hearsay_2` | warning | a thing only another tool says | secondary: another tool | -- |\n"
        );
    }
}
