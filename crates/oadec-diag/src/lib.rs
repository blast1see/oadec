//! Findings and the rules that make them, for every verifier of the engine.
//!
//! A check that fails says three things: which rule it is, where in the
//! stream, and how much it covers. What the rule is worth is said once, in a
//! catalog: its stable name, the severity a failure carries by default and
//! the source the rule rests on. This crate holds what is the same for every
//! format that is verified:
//!
//! - [`Severity`], how much a finding weighs, and the threshold a run fails at;
//! - [`Location`], where a finding is;
//! - [`Provenance`] and [`RuleInfo`], what a catalog says about a rule;
//! - [`Rule`], what a catalog is to the code that tallies its findings, and
//!   [`catalog!`], which writes one from a list;
//! - [`Finding`], [`RuleTally`] and [`Tally`], the findings of a pass by rule,
//!   at a bounded cost however often a rule fires;
//! - [`faults`], what every catalog has to keep whatever its rules are.
//!
//! It depends on nothing, and knows no format: the rules of a format are an
//! enumeration in the crate that reads the format.

mod location;
mod rule;
mod severity;
mod tally;

pub use location::Location;
pub use rule::{Provenance, Rule, RuleInfo, faults, markdown_rows, measured, secondary, spec};
pub use severity::Severity;
pub use tally::{Finding, RuleTally, Tally};

#[cfg(test)]
pub(crate) mod probe {
    //! A catalog of three rules for the tests of this crate.

    crate::catalog! {
        /// A rule of the tests.
        pub enum Probe;
        /// A thing worth knowing.
        Light {
            id: "probe.light",
            severity: Info,
            legacy: None,
            summary: "a thing worth knowing",
            provenance: [crate::spec("A document", "1.2")],
        },
        /// A thing that fails a stream.
        Heavy {
            id: "probe.heavy",
            severity: Error,
            legacy: Some("heavy_things"),
            summary: "a thing that fails a stream",
            provenance: [crate::spec("A document", "3"), crate::measured("measured/heavy.json")],
        },
        /// A thing only another tool says.
        Hearsay {
            id: "probe.hearsay_2",
            severity: Warning,
            legacy: None,
            summary: "a thing only another tool says",
            provenance: [crate::secondary("another tool")],
        },
    }
}
