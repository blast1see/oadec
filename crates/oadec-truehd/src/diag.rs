//! Findings: what a check found, where, and how the findings of a pass are
//! tallied. The checks are named in [`crate::rules`].
//!
//! A pass used to keep one counter per check and the first message any check
//! wrote. That says *how often* and *what first*; it does not say where the
//! damage is, whether it is one burst or spread over the film, or which of two
//! checks that fired together came first. A [`Tally`] keeps, per rule, the
//! count, the first and last place it fired, and the first few findings in
//! full, at a bounded cost however often a rule fires.

use std::collections::BTreeMap;

use crate::rules::RuleId;

/// How much a finding weighs.
///
/// A run fails when the worst finding is at or above the threshold it was
/// given (`verify --fail-on`); the default threshold is [`Severity::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Worth knowing; says nothing against the stream.
    Info,
    /// Outside what the rule's source asks for, or a rule not confirmed
    /// independently; the audio is not known to be affected.
    Warning,
    /// The stream breaks a rule it is known to have to keep; what was
    /// delivered from it is not trustworthy.
    Error,
    /// The stream could not be read past this point as it stands.
    Fatal,
}

impl Severity {
    /// Every severity, lightest first.
    pub const ALL: [Self; 4] = [Self::Info, Self::Warning, Self::Error, Self::Fatal];

    /// The name used on the command line and in reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Fatal => "fatal",
        }
    }

    /// The severity of a name, as [`Severity::name`] writes it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// Where a finding is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Location {
    /// Index of the access unit, counted from the first one framed. For
    /// bytes the extractor passed over, the unit it found sync again at.
    pub unit: Option<u64>,
    /// Byte offset in the input.
    pub offset: Option<u64>,
    /// Bit within that byte, 0 being the most significant, when the check
    /// knows the field it stopped at.
    pub bit: Option<u8>,
    /// The substream the finding belongs to, when it belongs to one.
    pub substream: Option<u8>,
}

impl Location {
    /// The start of access unit `unit`, which begins at byte `offset`.
    #[must_use]
    pub const fn unit(unit: u64, offset: u64) -> Self {
        Self {
            unit: Some(unit),
            offset: Some(offset),
            bit: None,
            substream: None,
        }
    }

    /// A place in the input that belongs to no access unit.
    #[must_use]
    pub const fn byte(offset: u64) -> Self {
        Self {
            unit: None,
            offset: Some(offset),
            bit: None,
            substream: None,
        }
    }

    /// The same place, in substream `substream`.
    #[must_use]
    pub const fn in_substream(mut self, substream: usize) -> Self {
        self.substream = Some(substream as u8);
        self
    }

    /// The same access unit, at bit `bit_position` counted from byte `base`.
    #[must_use]
    pub const fn at_bit(mut self, base: u64, bit_position: usize) -> Self {
        self.offset = Some(base + (bit_position / 8) as u64);
        self.bit = Some((bit_position % 8) as u8);
        self
    }
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The rule that fired.
    pub rule: RuleId,
    /// Where.
    pub at: Location,
    /// How much it covers: 1 for one event, a byte count for skipped bytes.
    pub magnitude: u64,
    /// What the check saw, without the location (that is in `at`).
    pub message: String,
}

/// The findings of one rule over a pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleTally {
    /// How often the rule fired.
    pub count: u64,
    /// The sum of the magnitudes.
    pub magnitude: u64,
    /// The first finding, in full.
    pub first: Finding,
    /// Where it last fired.
    pub last: Location,
    /// The first findings in full, at most the pass's limit.
    pub listed: Vec<Finding>,
}

impl RuleTally {
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
pub struct Tally {
    max_per_rule: usize,
    rules: BTreeMap<RuleId, RuleTally>,
}

impl Default for Tally {
    fn default() -> Self {
        Self::new(Self::DEFAULT_MAX_PER_RULE)
    }
}

impl Tally {
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
    pub fn add(
        &mut self,
        rule: RuleId,
        at: Location,
        magnitude: u64,
        message: impl FnOnce() -> String,
    ) {
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
    pub fn iter(&self) -> impl Iterator<Item = (RuleId, &RuleTally)> {
        self.rules.iter().map(|(r, t)| (*r, t))
    }

    /// The tally of one rule, if it fired.
    #[must_use]
    pub fn get(&self, rule: RuleId) -> Option<&RuleTally> {
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

    #[test]
    fn severities_are_ordered_and_named_both_ways() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Fatal);
        for s in Severity::ALL {
            assert_eq!(Severity::from_name(s.name()), Some(s));
        }
        assert_eq!(Severity::from_name("bad"), None);
    }

    #[test]
    fn a_limit_changes_the_listing_and_never_a_count() {
        let rule = RuleId::SegmentParity;
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
            assert_eq!(t.worst(), Some(rule.info().severity));
        }
    }

    #[test]
    fn a_message_is_made_only_when_it_is_kept() {
        let mut t = Tally::new(1);
        let mut made = 0;
        for unit in 0..4u64 {
            t.add(RuleId::SegmentCrc, Location::unit(unit, 0), 1, || {
                made += 1;
                String::new()
            });
        }
        assert_eq!(made, 1);
    }

    #[test]
    fn the_threshold_decides_on_the_worst_finding() {
        let mut t = Tally::default();
        assert!(t.passes(Severity::Info), "nothing fired");
        t.add(RuleId::SegmentParity, Location::default(), 1, String::new);
        let s = RuleId::SegmentParity.info().severity;
        assert!(!t.passes(s));
        assert!(t.passes(Severity::Fatal) || s == Severity::Fatal);
    }

    #[test]
    fn a_bit_position_becomes_a_byte_and_a_bit() {
        let at = Location::unit(7, 1000).at_bit(1010, 19);
        assert_eq!(at.offset, Some(1012));
        assert_eq!(at.bit, Some(3));
        assert_eq!(at.unit, Some(7));
    }
}
