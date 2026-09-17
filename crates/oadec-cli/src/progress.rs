//! A percentage on the terminal, and nothing at all anywhere else.
//!
//! A decode of a feature film reads several gigabytes and takes minutes. This
//! draws how far it has got, in place, on stderr.
//!
//! It renders only when stderr is a terminal. Four tools under `tools/` parse
//! oadec's stderr, a media test reads it as text, and most of the CLI tests
//! match on it; a pipeline, a log file and a test are all not terminals, and for
//! them the output stays exactly what it was. That is the whole reason the check
//! is here rather than behind a flag someone has to remember.
//!
//! The work is measured in bytes of the input, not in access units or frames:
//! the file size is known before the walk begins, every walk reports the offset
//! it has reached, and a byte is the one unit every format has in common.

use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// Redraw at most this often. Ten a second is smooth to read and costs nothing;
/// a write per access unit would be 9.5 million writes on a two-hour film.
const EVERY: Duration = Duration::from_millis(100);

/// Where a drawn line goes. Stderr in a real run; a buffer under test, so the
/// drawing can be asserted rather than only its absence.
enum Sink {
    Stderr,
    #[cfg(test)]
    Buffer(std::rc::Rc<std::cell::RefCell<String>>),
}

impl Sink {
    fn put(&self, s: &str) {
        match self {
            Sink::Stderr => {
                let mut err = std::io::stderr().lock();
                let _ = write!(err, "{s}");
                let _ = err.flush();
            }
            #[cfg(test)]
            Sink::Buffer(b) => b.borrow_mut().push_str(s),
        }
    }
}

/// Draws how far a walk has got, on a terminal only.
///
/// The line is cleared when the reporter is dropped, which is the only way to
/// cover every exit: a walk can leave through any `?` in it, and an error
/// printed onto a half-drawn bar is worse than no bar at all.
pub struct Progress {
    /// Total bytes of the input, zero when it could not be measured.
    total: u64,
    /// When the last line was drawn.
    last: Instant,
    /// Whether anything has been drawn, so the line is only cleared if used.
    drawn: bool,
    /// Whether to draw at all: false when stderr is not a terminal.
    live: bool,
    /// What the work is called, shown beside the percentage.
    what: &'static str,
    /// Where the line goes.
    sink: Sink,
}

impl Progress {
    /// A reporter for a walk over `total` bytes.
    ///
    /// Silent when stderr is not a terminal, and when the total is unknown --
    /// a percentage of an unknown quantity is a guess, and this prints none.
    pub fn new(what: &'static str, total: u64) -> Self {
        Self {
            total,
            last: Instant::now() - EVERY,
            drawn: false,
            live: total > 0 && std::io::stderr().is_terminal(),
            what,
            sink: Sink::Stderr,
        }
    }

    /// A reporter that draws into a buffer, for testing the half a piped run
    /// can never reach.
    #[cfg(test)]
    fn for_test(what: &'static str, total: u64) -> Self {
        Self {
            total,
            last: Instant::now() - EVERY,
            drawn: false,
            live: true,
            what,
            sink: Sink::Buffer(std::rc::Rc::new(std::cell::RefCell::new(String::new()))),
        }
    }

    /// What has been drawn so far, under test.
    #[cfg(test)]
    fn drawn_text(&self) -> String {
        match &self.sink {
            Sink::Buffer(b) => b.borrow().clone(),
            Sink::Stderr => String::new(),
        }
    }

    /// A handle on the buffer that outlives the reporter, so a test can drop
    /// the reporter and still read what it wrote on the way out.
    #[cfg(test)]
    fn buffer(&self) -> std::rc::Rc<std::cell::RefCell<String>> {
        match &self.sink {
            Sink::Buffer(b) => std::rc::Rc::clone(b),
            Sink::Stderr => unreachable!("only a test reporter has a buffer"),
        }
    }

    /// Reports that the walk has reached `done` bytes.
    ///
    /// Cheap to call for every access unit: it returns immediately unless the
    /// redraw interval has passed.
    pub fn at(&mut self, done: u64) {
        if !self.live || self.last.elapsed() < EVERY {
            return;
        }
        self.last = Instant::now();
        self.drawn = true;
        let line = Self::render(self.what, done, self.total);
        // a carriage return without a newline redraws the same line
        self.sink.put(&format!("\r{line}"));
    }

    /// Clears the line, so whatever is printed next starts clean.
    ///
    /// Does nothing if nothing was drawn, which keeps a piped run untouched.
    pub fn clear(&mut self) {
        if !self.live || !self.drawn {
            return;
        }
        self.sink.put(&format!("\r{:width$}\r", "", width = 48));
        self.drawn = false;
    }

    /// The line itself: a pure function of how far the walk has got.
    ///
    /// Clamped at 100, because the last access unit of a file can end past the
    /// size taken before the walk began if the file is being written to, and a
    /// percentage above a hundred reads as a fault where there is none.
    fn render(what: &str, done: u64, total: u64) -> String {
        let percent = if total == 0 {
            0
        } else {
            ((done.min(total) as f64 / total as f64) * 100.0).round() as u64
        };
        let filled = (percent as usize * 24).div_ceil(100);
        let bar: String = "#".repeat(filled) + &"-".repeat(24 - filled);
        format!("{what} [{bar}] {percent:>3}%")
    }
}

impl Drop for Progress {
    /// Clears on every exit, including the `?` that leaves a walk early.
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::Progress;

    #[test]
    fn the_percentage_is_the_share_of_the_bytes_read() {
        assert!(Progress::render("decode", 0, 100).ends_with("  0%"));
        assert!(Progress::render("decode", 50, 100).ends_with(" 50%"));
        assert!(Progress::render("decode", 100, 100).ends_with("100%"));
    }

    #[test]
    fn a_walk_past_the_size_taken_before_it_began_is_not_more_than_finished() {
        // the file can grow while it is read; 101% would read as a fault
        assert!(Progress::render("decode", 200, 100).ends_with("100%"));
    }

    #[test]
    fn an_unknown_total_gives_no_percentage_rather_than_a_guess() {
        assert!(Progress::render("decode", 42, 0).ends_with("  0%"));
    }

    #[test]
    fn the_bar_fills_with_the_percentage() {
        assert!(Progress::render("decode", 0, 100).contains("[------------------------]"));
        assert!(Progress::render("decode", 100, 100).contains("[########################]"));
        let half = Progress::render("decode", 50, 100);
        assert_eq!(half.matches('#').count(), 12, "{half}");
    }

    #[test]
    fn a_live_reporter_draws_the_line_and_redraws_it_in_place() {
        let mut p = Progress::for_test("reading", 1000);
        p.at(250);
        std::thread::sleep(super::EVERY);
        p.at(1000);
        let drawn = p.drawn_text();
        assert_eq!(
            drawn.matches('\r').count(),
            2,
            "each draw redraws in place: {drawn:?}"
        );
        assert!(drawn.contains(" 25%"), "{drawn:?}");
        assert!(drawn.contains("100%"), "{drawn:?}");
        assert!(
            !drawn.contains('\n'),
            "a redrawn line carries no newline: {drawn:?}"
        );
    }

    #[test]
    fn a_reporter_dropped_mid_walk_clears_the_line_it_drew() {
        // The case no piped test can reach, and the reason Drop exists: a walk
        // leaves early through `?` with a bar on the screen and an error about
        // to be printed over it. The buffer is held outside the reporter so the
        // drop itself is observable -- delete the Drop impl and this fails,
        // which the earlier version of this test did not.
        let seen = {
            let mut p = Progress::for_test("reading", 1000);
            p.at(500);
            assert!(p.drawn, "precondition: something was drawn");
            let handle = p.buffer();
            drop(p);
            handle
        };
        let drawn = seen.borrow().clone();
        assert!(
            drawn.ends_with('\r'),
            "dropping must wipe the line and return the cursor: {drawn:?}"
        );
        assert!(
            drawn.contains(" 50%"),
            "and what it wiped was the line it drew: {drawn:?}"
        );
    }

    #[test]
    fn a_reporter_that_drew_nothing_clears_nothing() {
        let mut p = Progress::for_test("reading", 1000);
        p.clear();
        assert_eq!(p.drawn_text(), "", "nothing drawn, nothing to wipe");
    }

    #[test]
    fn nothing_is_drawn_when_stderr_is_not_a_terminal() {
        // the tests run piped, which is exactly the case that must stay silent
        let mut p = Progress::new("decode", 1000);
        assert!(!p.live, "a piped run must not be live");
        p.at(500);
        p.clear();
    }
}
