//! Findings: what a check found, where, and how the findings of a pass are
//! tallied.
//!
//! A pass used to keep one counter per check and the first message any check
//! wrote. That says *how often* and *what first*; it does not say where the
//! damage is, whether it is one burst or spread over the film, or which of two
//! checks that fired together came first. A [`Tally`] keeps, per rule, the
//! count, the first and last place it fired, and the first few findings in
//! full, at a bounded cost however often a rule fires.

use std::collections::BTreeMap;

use crate::location::Location;
use crate::rule::Rule;
use crate::severity::Severity;

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding<R> {
    /// The rule that fired.
    pub rule: R,
    /// Where.
    pub at: Location,
    /// How much it covers: 1 for one event, a byte count for skipped bytes.
    pub magnitude: u64,
    /// What the check saw, without the location (that is in `at`).
    pub message: String,
}

/// The findings of one rule over a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleTally<R> {
    /// How often the rule fired.
    pub count: u64,
    /// The sum of the magnitudes.
    pub magnitude: u64,
    /// The first finding, in full.
    pub first: Finding<R>,
    /// Where it last fired.
    pub last: Location,
    /// The first findings in full, at most the pass's limit.
    pub listed: Vec<Finding<R>>,
}

impl<R> RuleTally<R> {
    /// Findings counted but not listed.
    #[must_use]
    pub fn suppressed(&self) -> u64 {
        self.count - self.listed.len() as u64
    }
}

/// The findings of a pass, by rule.
///
/// Every finding is counted. At most `max_per_rule` of them are kept in full
/// per rule (0 keeps all), so a limit changes what is listed and never a count,
/// a severity or a verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tally<R: Rule> {
    max_per_rule: usize,
    rules: BTreeMap<R, RuleTally<R>>,
}

impl<R: Rule> Default for Tally<R> {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_PER_RULE)
    }
}

impl<R: Rule> Tally<R> {
    /// Findings listed per rule unless asked otherwise.
    pub const DEFAULT_MAX_PER_RULE: usize = 20;

    /// An empty tally that keeps at most `max_per_rule` findings per rule in
    /// full (0 keeps every one).
    #[must_use]
    pub const fn new(max_per_rule: usize) -> Self {
        Self {
            max_per_rule,
            rules: BTreeMap::new(),
        }
    }

    /// The listing limit per rule (0 means none).
    #[must_use]
    pub const fn max_per_rule(&self) -> usize {
        self.max_per_rule
    }

    /// Records one finding. The message is made only when it is kept.
    pub fn add(&mut self, rule: R, at: Location, magnitude: u64, message: impl FnOnce() -> String) {
        let limit = self.max_per_rule;
        match self.rules.get_mut(&rule) {
            Some(t) => {
                t.count += 1;
                t.magnitude += magnitude;
                t.last = at;
                if limit == 0 || t.listed.len() < limit {
                    t.listed.push(Finding {
                        rule,
                        at,
                        magnitude,
                        message: message(),
                    });
                }
            }
            None => {
                let first = Finding {
                    rule,
                    at,
                    magnitude,
                    message: message(),
                };
                // a limit of 0 lists every finding, and any other limit has
                // room for the first one
                self.rules.insert(
                    rule,
                    RuleTally {
                        count: 1,
                        magnitude,
                        listed: vec![first.clone()],
                        first,
                        last: at,
                    },
                );
            }
        }
    }

    /// The rules that fired, in catalog order, with their tallies.
    pub fn iter(&self) -> impl Iterator<Item = (R, &RuleTally<R>)> {
        self.rules.iter().map(|(r, t)| (*r, t))
    }

    /// The tally of one rule, if it fired.
    #[must_use]
    pub fn get(&self, rule: R) -> Option<&RuleTally<R>> {
        self.rules.get(&rule)
    }

    /// Whether nothing fired.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The heaviest severity that fired, by the catalog's default severities.
    #[must_use]
    pub fn worst(&self) -> Option<Severity> {
        self.rules.keys().map(|r| r.info().severity).max()
    }

    /// How many findings fired at `severity`.
    #[must_use]
    pub fn count_at(&self, severity: Severity) -> u64 {
        self.rules
            .iter()
            .filter(|(r, _)| r.info().severity == severity)
            .map(|(_, t)| t.count)
            .sum()
    }

    /// Whether a run that fails at `fail_on` passes: nothing fired at or above it.
    #[must_use]
    pub fn passes(&self, fail_on: Severity) -> bool {
        self.worst().is_none_or(|w| w < fail_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::Probe;

    #[test]
    fn a_limit_changes_the_listing_and_never_a_count() {
        let rule = Probe::Heavy;
        for limit in [0usize, 1, 3] {
            let mut t = Tally::new(limit);
            for unit in 0..5u64 {
                t.add(rule, Location::unit(unit, unit * 100), 1, || {
                    format!("at {unit}")
                });
            }
            let r = t.get(rule).unwrap();
            assert_eq!(r.count, 5, "limit {limit}");
            assert_eq!(r.magnitude, 5);
            assert_eq!(r.first.at.unit, Some(0));
            assert_eq!(r.first.message, "at 0");
            assert_eq!(r.last.unit, Some(4));
            let listed = if limit == 0 { 5 } else { limit };
            assert_eq!(r.listed.len(), listed, "limit {limit}");
            assert_eq!(r.suppressed(), 5 - listed as u64);
            assert_eq!(t.worst(), Some(Severity::Error));
            assert_eq!(t.max_per_rule(), limit);
        }
    }

    #[test]
    fn a_message_is_made_only_when_it_is_kept() {
        let mut t = Tally::new(1);
        let mut made = 0;
        for unit in 0..4u64 {
            t.add(Probe::Heavy, Location::unit(unit, 0), 1, || {
                made += 1;
                String::new()
            });
        }
        assert_eq!(made, 1);
    }

    #[test]
    fn the_threshold_decides_on_the_worst_finding() {
        let mut t = Tally::default();
        assert!(t.is_empty() && t.worst().is_none());
        assert!(t.passes(Severity::Info), "nothing fired");
        t.add(Probe::Light, Location::default(), 1, String::new);
        assert!(!t.passes(Severity::Info));
        assert!(t.passes(Severity::Warning));
        t.add(Probe::Hearsay, Location::default(), 1, String::new);
        assert!(!t.passes(Severity::Warning));
        assert!(
            t.passes(Severity::Error),
            "a warning fails no stream by default"
        );
        t.add(Probe::Heavy, Location::default(), 3, String::new);
        t.add(Probe::Heavy, Location::default(), 4, String::new);
        assert!(!t.passes(Severity::Error));
        assert!(t.passes(Severity::Fatal));
        assert_eq!(t.worst(), Some(Severity::Error));
        assert_eq!(t.count_at(Severity::Error), 2);
        assert_eq!(t.count_at(Severity::Info), 1);
        assert_eq!(t.count_at(Severity::Fatal), 0);
        assert_eq!(t.get(Probe::Heavy).unwrap().magnitude, 7);
    }

    #[test]
    fn the_rules_that_fired_come_in_the_order_of_the_catalog() {
        let mut t = Tally::default();
        for rule in [Probe::Hearsay, Probe::Light, Probe::Heavy, Probe::Light] {
            t.add(rule, Location::default(), 1, String::new);
        }
        let order: Vec<Probe> = t.iter().map(|(r, _)| r).collect();
        assert_eq!(order, [Probe::Light, Probe::Heavy, Probe::Hearsay]);
        assert_eq!(t.get(Probe::Light).unwrap().count, 2);
    }
}
