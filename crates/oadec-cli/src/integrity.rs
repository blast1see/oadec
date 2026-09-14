//! One verdict on a decode, shared by every command that delivers audio.
//!
//! Integrity was detected everywhere and reached almost nothing. `verify` read
//! every counter and exited 7; `decode` re-derived a narrower rule of its own
//! and exited 0 while printing the same CRC failure; the object path read no
//! integrity flag at all and produced Atmos objects and metadata from corrupted
//! frames without saying a word. The fix is not more checks -- the checks were
//! there -- but one place that turns them into a verdict, and one exit code
//! that every delivery path uses.
//!
//! The object outputs add a second question: did the file carry the whole
//! programme? A DAMF set or ADM file written with a declared loss (an ISF
//! element dropped, a programme outside the profile) is sound but incomplete,
//! and says so with its own exit code.
//!
//! See `docs/exit-codes.md` for the policy this implements.

use oadec_spatial::LossLedger;

/// The outcome of a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing wrong, nothing missing.
    Clean,
    /// Sound, but written with a declared loss.
    Lossy,
    /// An integrity fault affects what was delivered.
    Faulty,
}

impl Verdict {
    /// The verdict of a delivery that has no loss ledger.
    #[must_use]
    pub const fn from_clean(clean: bool) -> Self {
        if clean { Self::Clean } else { Self::Faulty }
    }

    /// The process exit code.
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Clean => 0,
            Self::Lossy => 4,
            Self::Faulty => 7,
        }
    }
}

/// What a decode found wrong.
///
/// A finding is anything that makes the delivered output untrustworthy: a
/// failed check word, a frame that would not decode, a byte of the file that
/// was skipped, a metadata payload that would not parse. Recoverable output is
/// still written -- throwing away audio that decodes is the wrong trade, and
/// the project already argued that once over a frame that ends inside its own
/// tail -- but the run says what happened and does not exit clean.
#[derive(Debug, Default, Clone)]
pub struct Findings {
    entries: Vec<String>,
    first: Option<String>,
    lossy: bool,
}

impl Findings {
    /// Records `what` when `count` is not zero.
    pub fn note(&mut self, count: u64, what: &str) {
        if count > 0 {
            self.entries.push(format!("{count} {what}"));
        }
    }

    /// Records the first problem the pass described, if any.
    pub fn first_problem(&mut self, problem: Option<&str>) {
        if self.first.is_none() {
            self.first = problem.map(ToString::to_string);
        }
    }

    /// Records whether the output was written with a declared loss.
    pub fn note_losses(&mut self, losses: &LossLedger) {
        self.lossy = losses.declared_loss();
    }

    /// Whether the decode is trustworthy.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    /// Prints the verdict and returns whether it was clean, for the delivery
    /// paths that have no loss ledger (PCM, WAV, CAF, `compare`).
    pub fn report_clean(&self) -> bool {
        self.report() == Verdict::Clean
    }

    /// Prints the verdict to stderr and returns it.
    ///
    /// Nothing is printed on a clean decode: a clean run should be quiet.
    pub fn report(&self) -> Verdict {
        if self.lossy {
            eprintln!(
                "the output was written and does not carry the whole programme; see the lines above"
            );
        }
        if self.is_clean() {
            return if self.lossy {
                Verdict::Lossy
            } else {
                Verdict::Clean
            };
        }
        eprintln!("integrity: {}", self.entries.join(", "));
        if let Some(first) = &self.first {
            eprintln!("first problem: {first}");
        }
        eprintln!(
            "the output was written and is not trustworthy; `oadec verify` reports the same faults"
        );
        Verdict::Faulty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oadec_spatial::LossKind;

    #[test]
    fn a_declared_loss_is_lossy_and_a_fault_wins() {
        let mut f = Findings::default();
        assert_eq!(f.report(), Verdict::Clean);
        let mut l = LossLedger::default();
        l.note(LossKind::RampReplaced, 10, 0);
        f.note_losses(&l);
        assert_eq!(
            f.report(),
            Verdict::Clean,
            "a profile reduction is not a declared loss"
        );
        l.note(LossKind::IsfDropped, 0, 0);
        f.note_losses(&l);
        assert_eq!(f.report(), Verdict::Lossy);
        f.note(1, "CRC failures");
        assert_eq!(f.report(), Verdict::Faulty);
        assert_eq!(Verdict::Clean.exit_code(), 0);
        assert_eq!(Verdict::Lossy.exit_code(), 4);
        assert_eq!(Verdict::Faulty.exit_code(), 7);
        assert_eq!(Verdict::from_clean(true), Verdict::Clean);
        assert_eq!(Verdict::from_clean(false), Verdict::Faulty);
    }
}
