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
//! See `docs/exit-codes.md` for the policy this implements.

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

    /// Whether the decode is trustworthy.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    /// Prints the verdict to stderr and returns whether it was clean.
    ///
    /// Nothing is printed on a clean decode: a clean run should be quiet.
    pub fn report(&self) -> bool {
        if self.is_clean() {
            return true;
        }
        eprintln!("integrity: {}", self.entries.join(", "));
        if let Some(first) = &self.first {
            eprintln!("first problem: {first}");
        }
        eprintln!(
            "the output was written and is not trustworthy; `oadec verify` reports the same faults"
        );
        false
    }
}
