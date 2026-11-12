//! Findings: what a check found, where, and how the findings of a pass are
//! tallied. The checks are named in [`crate::rules`].
//!
//! What a finding is, where it is and how a pass tallies its findings is the
//! same for every format that is verified, and lives in `oadec-diag`. These
//! are its types for the rules of a TrueHD stream.

pub use oadec_diag::{Location, Severity};

use crate::rules::RuleId;

/// One finding of a rule of the TrueHD catalog.
pub type Finding = oadec_diag::Finding<RuleId>;

/// The findings of one rule of the TrueHD catalog over a pass.
pub type RuleTally = oadec_diag::RuleTally<RuleId>;

/// The findings of a pass over a TrueHD stream, by rule.
///
/// Every finding is counted. At most `max_per_rule` of them are kept in full
/// per rule (0 keeps all), so a limit changes what is listed and never a count,
/// a severity or a verdict.
pub type Tally = oadec_diag::Tally<RuleId>;

#[cfg(test)]
mod tests {
    use super::*;

    /// The tally of the crate counts the rules of the TrueHD catalog, by the
    /// severities the catalog gives them.
    #[test]
    fn a_tally_weighs_a_finding_by_the_catalog() {
        let mut t = Tally::default();
        assert_eq!(t.max_per_rule(), Tally::DEFAULT_MAX_PER_RULE);
        assert!(t.passes(Severity::Info), "nothing fired");
        let rule = RuleId::SegmentParity;
        for unit in 0..3u64 {
            t.add(rule, Location::unit(unit, unit * 100), 1, || {
                format!("at {unit}")
            });
        }
        let r: &RuleTally = t.get(rule).unwrap();
        let first: &Finding = &r.first;
        assert_eq!((r.count, first.rule, first.at.unit), (3, rule, Some(0)));
        let s = rule.info().severity;
        assert_eq!(t.worst(), Some(s));
        assert!(!t.passes(s));
        assert!(t.passes(Severity::Fatal) || s == Severity::Fatal);
    }
}
