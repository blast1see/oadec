//! What a DAMF or ADM output could not carry of the programme it was given.
//!
//! The writers approximate or drop several things. The Dolby Atmos master ADM
//! profile fixes the interpolation length and forbids gain and importance on
//! active objects; DAMF has one size and no distance; neither has a field for
//! divergence, warp mode or trim configurations. None of that used to be
//! said: the run printed a frame count and exited 0. Every such mapping is
//! now counted where it happens, the command prints the ledger, and a loss the
//! user asked for or one that leaves the profile sets a distinct exit code
//! (see `docs/exit-codes.md`).

use std::collections::BTreeMap;

/// Why an output does not carry something the programme had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossClass {
    /// The Dolby Atmos master ADM profile cannot carry it, and Dolby's own
    /// converters drop it the same way.
    ProfileReduction,
    /// Neither DAMF nor the profile has a field for it.
    Unrepresentable,
    /// oadec's own choice where the formats leave room; Dolby chooses
    /// differently or has no equivalent.
    Approximation,
    /// The output was written with something missing or outside the profile,
    /// at the user's request or because the input was anomalous.
    DeclaredLoss,
}

impl LossClass {
    /// Every class, in reporting order.
    pub const ALL: [Self; 4] = [
        Self::ProfileReduction,
        Self::Unrepresentable,
        Self::Approximation,
        Self::DeclaredLoss,
    ];

    /// Title of the line the class is reported under.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::ProfileReduction => "profile reductions",
            Self::Unrepresentable => "not representable in DAMF or the ADM profile",
            Self::Approximation => "approximations",
            Self::DeclaredLoss => "written with loss",
        }
    }

    /// Stable identifier for machine-readable reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ProfileReduction => "profile-reduction",
            Self::Unrepresentable => "unrepresentable",
            Self::Approximation => "approximation",
            Self::DeclaredLoss => "declared-loss",
        }
    }
}

/// One kind of information the output does not carry as the programme had it.
///
/// The variants are declared in reporting order, class by class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LossKind {
    /// `interpolationLength` written as 250 samples (profile table 11) where the
    /// source ramp was something else.
    RampReplaced,
    /// An active object's importance is not 1.0; the profile allows importance
    /// only as the inactive marker.
    ImportanceOmitted,
    /// A bed channel changed state after its first event; DirectSpeakers blocks
    /// have no time.
    BedEventDropped,
    /// A bed channel's first state has a gain other than 0 dB or is inactive;
    /// the bed block carries neither.
    BedGainDropped,
    /// A screen-referenced block; `screenRef` shall not be used.
    ScreenReferenceDropped,
    /// A trim bypass flag; ADM has no field.
    TrimBypassDropped,
    /// An object update with a specified distance.
    DistanceDropped,
    /// An object update with divergence.
    DivergenceDropped,
    /// A warp mode other than the default.
    WarpModeDropped,
    /// Explicit trim configurations.
    TrimConfigDropped,
    /// Width, depth and height differ; the width is written.
    SizeAxesCollapsed,
    /// An object's first event arrives after sample 0; its state is held from 0.
    LateFirstEventHeld,
    /// Two events at one sample; the last wins.
    SamePositionSuperseded,
    /// An event at or after the programme end.
    EventBeyondEndDropped,
    /// ISF elements written out at the user's request.
    IsfDropped,
    /// DAMF: an event earlier than the previous one of its element, written as delivered.
    OutOfOrderWrittenAsIs,
    /// ADM: a programme that is not at 48 kHz, written at the user's request.
    NonProfileSampleRate,
}

impl LossKind {
    /// The class the kind is reported under.
    #[must_use]
    pub const fn class(self) -> LossClass {
        match self {
            Self::RampReplaced
            | Self::ImportanceOmitted
            | Self::BedEventDropped
            | Self::BedGainDropped
            | Self::ScreenReferenceDropped
            | Self::TrimBypassDropped => LossClass::ProfileReduction,
            Self::DistanceDropped
            | Self::DivergenceDropped
            | Self::WarpModeDropped
            | Self::TrimConfigDropped => LossClass::Unrepresentable,
            Self::SizeAxesCollapsed
            | Self::LateFirstEventHeld
            | Self::SamePositionSuperseded
            | Self::EventBeyondEndDropped => LossClass::Approximation,
            Self::IsfDropped | Self::OutOfOrderWrittenAsIs | Self::NonProfileSampleRate => {
                LossClass::DeclaredLoss
            }
        }
    }

    /// Stable identifier for machine-readable reports.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RampReplaced => "ramp-replaced",
            Self::ImportanceOmitted => "importance-omitted",
            Self::BedEventDropped => "bed-event-dropped",
            Self::BedGainDropped => "bed-gain-dropped",
            Self::ScreenReferenceDropped => "screen-reference-dropped",
            Self::TrimBypassDropped => "trim-bypass-dropped",
            Self::DistanceDropped => "distance-dropped",
            Self::DivergenceDropped => "divergence-dropped",
            Self::WarpModeDropped => "warp-mode-dropped",
            Self::TrimConfigDropped => "trim-config-dropped",
            Self::SizeAxesCollapsed => "size-axes-collapsed",
            Self::LateFirstEventHeld => "late-first-event-held",
            Self::SamePositionSuperseded => "same-position-superseded",
            Self::EventBeyondEndDropped => "event-beyond-end-dropped",
            Self::IsfDropped => "isf-dropped",
            Self::OutOfOrderWrittenAsIs => "out-of-order-written-as-is",
            Self::NonProfileSampleRate => "non-profile-sample-rate",
        }
    }

    /// Wording after a count of one and after a larger count.
    const fn wording(self) -> (&'static str, &'static str) {
        match self {
            Self::RampReplaced => (
                "interpolation length replaced by 250 samples",
                "interpolation lengths replaced by 250 samples",
            ),
            Self::ImportanceOmitted => (
                "importance value of an active object omitted",
                "importance values of active objects omitted",
            ),
            Self::BedEventDropped => (
                "bed event after the first not carried",
                "bed events after the first not carried",
            ),
            Self::BedGainDropped => (
                "bed channel with a gain other than 0 dB or inactive, not carried",
                "bed channels with a gain other than 0 dB or inactive, not carried",
            ),
            Self::ScreenReferenceDropped => (
                "screen-referenced block not carried as such",
                "screen-referenced blocks not carried as such",
            ),
            Self::TrimBypassDropped => (
                "trim bypass flag not carried",
                "trim bypass flags not carried",
            ),
            Self::DistanceDropped => (
                "update with a specified distance",
                "updates with a specified distance",
            ),
            Self::DivergenceDropped => ("update with divergence", "updates with divergence"),
            Self::WarpModeDropped => ("warp mode setting", "warp mode settings"),
            Self::TrimConfigDropped => ("trim configuration set", "trim configuration sets"),
            Self::SizeAxesCollapsed => (
                "event with differing size axes written with the width",
                "events with differing size axes written with the width",
            ),
            Self::LateFirstEventHeld => (
                "first event held from sample 0",
                "first events held from sample 0",
            ),
            Self::SamePositionSuperseded => (
                "event superseded by a later one at the same sample",
                "events superseded by later ones at the same sample",
            ),
            Self::EventBeyondEndDropped => (
                "event at or after the programme end dropped",
                "events at or after the programme end dropped",
            ),
            Self::IsfDropped => ("ISF element dropped", "ISF elements dropped"),
            Self::OutOfOrderWrittenAsIs => (
                "out-of-order event written as delivered",
                "out-of-order events written as delivered",
            ),
            Self::NonProfileSampleRate => (
                "programme not at 48 kHz written outside the profile",
                "programmes not at 48 kHz written outside the profile",
            ),
        }
    }

    /// Whether the report names the elements and samples; payload-level kinds
    /// have no place worth naming.
    const fn located(self) -> bool {
        !matches!(
            self,
            Self::RampReplaced
                | Self::ImportanceOmitted
                | Self::WarpModeDropped
                | Self::TrimConfigDropped
                | Self::IsfDropped
                | Self::NonProfileSampleRate
        )
    }
}

/// Examples kept per kind.
const EXAMPLES: usize = 3;

/// Counts per kind, the source ramps that were replaced, and up to three
/// (element id, sample position) examples per kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LossLedger {
    counts: BTreeMap<LossKind, u64>,
    ramp_sources: BTreeMap<u32, u64>,
    examples: BTreeMap<LossKind, Vec<(u32, u64)>>,
}

impl LossLedger {
    /// Records one loss of `kind` for `element` at `sample`.
    pub fn note(&mut self, kind: LossKind, element: u32, sample: u64) {
        *self.counts.entry(kind).or_insert(0) += 1;
        let examples = self.examples.entry(kind).or_default();
        if examples.len() < EXAMPLES {
            examples.push((element, sample));
        }
    }

    /// Records a replaced ramp and remembers the source length.
    pub fn note_ramp(&mut self, element: u32, sample: u64, source: u32) {
        self.note(LossKind::RampReplaced, element, sample);
        *self.ramp_sources.entry(source).or_insert(0) += 1;
    }

    /// Losses of `kind`.
    #[must_use]
    pub fn count(&self, kind: LossKind) -> u64 {
        self.counts.get(&kind).copied().unwrap_or(0)
    }

    /// The first examples of `kind`, as (element id, sample position).
    #[must_use]
    pub fn examples(&self, kind: LossKind) -> &[(u32, u64)] {
        self.examples.get(&kind).map_or(&[], Vec::as_slice)
    }

    /// Source ramp length → replaced events.
    #[must_use]
    pub const fn ramp_sources(&self) -> &BTreeMap<u32, u64> {
        &self.ramp_sources
    }

    /// Adds another ledger's counts, histogram and examples.
    pub fn merge(&mut self, other: &Self) {
        for (&kind, &n) in &other.counts {
            *self.counts.entry(kind).or_insert(0) += n;
        }
        for (&ramp, &n) in &other.ramp_sources {
            *self.ramp_sources.entry(ramp).or_insert(0) += n;
        }
        for (&kind, examples) in &other.examples {
            let mine = self.examples.entry(kind).or_default();
            for &example in examples {
                if mine.len() < EXAMPLES {
                    mine.push(example);
                }
            }
        }
    }

    /// Whether nothing was lost.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.values().all(|&n| n == 0)
    }

    /// Whether a loss of the declared class was recorded (the exit-code case).
    #[must_use]
    pub fn declared_loss(&self) -> bool {
        self.counts
            .iter()
            .any(|(kind, &n)| n > 0 && kind.class() == LossClass::DeclaredLoss)
    }

    /// Every kind with a non-zero count, in reporting order.
    pub fn iter(&self) -> impl Iterator<Item = (LossKind, u64)> + '_ {
        self.counts
            .iter()
            .filter(|(_, n)| **n > 0)
            .map(|(&kind, &n)| (kind, n))
    }

    /// One line per class that has losses, prefixed with `target`
    /// (`adm` or `damf`), kinds joined with semicolons.
    #[must_use]
    pub fn lines(&self, target: &str) -> Vec<String> {
        let mut out = Vec::new();
        for class in LossClass::ALL {
            let parts: Vec<String> = self
                .iter()
                .filter(|(kind, _)| kind.class() == class)
                .map(|(kind, n)| self.phrase(kind, n))
                .collect();
            if !parts.is_empty() {
                out.push(format!("{target}: {}: {}", class.title(), parts.join("; ")));
            }
        }
        out
    }

    fn phrase(&self, kind: LossKind, n: u64) -> String {
        let (one, many) = kind.wording();
        let mut s = format!("{n} {}", if n == 1 { one } else { many });
        if kind == LossKind::RampReplaced && !self.ramp_sources.is_empty() {
            let mut hist: Vec<(u32, u64)> =
                self.ramp_sources.iter().map(|(&r, &c)| (r, c)).collect();
            hist.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let h: Vec<String> = hist.iter().map(|(r, c)| format!("{r} x{c}")).collect();
            s.push_str(&format!(" ({})", h.join(", ")));
        } else if kind.located() {
            let ex: Vec<String> = self
                .examples(kind)
                .iter()
                .map(|(e, p)| format!("element {e} at {p}"))
                .collect();
            if !ex.is_empty() {
                s.push_str(&format!(" ({})", ex.join(", ")));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_counts_and_keeps_three_examples() {
        let mut l = LossLedger::default();
        assert!(l.is_empty());
        for sample in [0u64, 1536, 3072, 4608] {
            l.note(LossKind::ImportanceOmitted, 10, sample);
        }
        assert_eq!(l.count(LossKind::ImportanceOmitted), 4);
        assert_eq!(
            l.examples(LossKind::ImportanceOmitted),
            &[(10, 0), (10, 1536), (10, 3072)]
        );
        assert_eq!(l.count(LossKind::IsfDropped), 0);
        assert!(!l.is_empty());
    }

    #[test]
    fn declared_loss_is_only_the_declared_kinds() {
        let mut l = LossLedger::default();
        l.note_ramp(10, 1536, 1536);
        l.note(LossKind::DistanceDropped, 10, 0);
        l.note(LossKind::LateFirstEventHeld, 11, 96_000);
        assert!(!l.declared_loss());
        l.note(LossKind::IsfDropped, 0, 0);
        assert!(l.declared_loss());
        assert_eq!(LossKind::RampReplaced.class(), LossClass::ProfileReduction);
        assert_eq!(
            LossKind::DistanceDropped.class(),
            LossClass::Unrepresentable
        );
        assert_eq!(
            LossKind::LateFirstEventHeld.class(),
            LossClass::Approximation
        );
        assert_eq!(
            LossKind::OutOfOrderWrittenAsIs.class(),
            LossClass::DeclaredLoss
        );
    }

    #[test]
    fn merge_adds_counts_and_ramp_histograms() {
        let mut a = LossLedger::default();
        a.note_ramp(10, 1536, 1536);
        a.note_ramp(10, 3072, 32);
        let mut b = LossLedger::default();
        b.note_ramp(11, 1536, 1536);
        b.note(LossKind::BedEventDropped, 3, 48_000);
        a.merge(&b);
        assert_eq!(a.count(LossKind::RampReplaced), 3);
        assert_eq!(a.ramp_sources().get(&1536), Some(&2));
        assert_eq!(a.ramp_sources().get(&32), Some(&1));
        assert_eq!(a.count(LossKind::BedEventDropped), 1);
    }

    #[test]
    fn lines_are_grouped_by_class_in_a_fixed_order() {
        let mut l = LossLedger::default();
        l.note(LossKind::IsfDropped, 0, 0);
        l.note(LossKind::LateFirstEventHeld, 10, 96_000);
        l.note_ramp(10, 1536, 1536);
        l.note_ramp(10, 3072, 1536);
        l.note_ramp(11, 1536, 32);
        l.note(LossKind::DistanceDropped, 12, 0);
        let lines = l.lines("adm");
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert_eq!(
            lines[0],
            "adm: profile reductions: 3 interpolation lengths replaced by 250 samples (1536 x2, 32 x1)"
        );
        assert_eq!(
            lines[1],
            "adm: not representable in DAMF or the ADM profile: 1 update with a specified distance (element 12 at 0)"
        );
        assert_eq!(
            lines[2],
            "adm: approximations: 1 first event held from sample 0 (element 10 at 96000)"
        );
        assert_eq!(lines[3], "adm: written with loss: 1 ISF element dropped");
        assert!(LossLedger::default().lines("damf").is_empty());
    }
}
