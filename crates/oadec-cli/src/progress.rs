//! A progress line on the terminal, and nothing at all anywhere else.
//!
//! A decode of a feature film reads several gigabytes and takes minutes. This
//! draws how far it has got, in place, on stderr:
//!
//! ```text
//! reading  ██████████░░░░░░░░░░  46%  1.7/3.7 GiB  118 MiB/s  eta 0:17
//! ```
//!
//! Three rules decide whether anything is drawn at all, and each exists because
//! of something that went wrong without it:
//!
//! 1. **Only on a terminal.** Four tools under `tools/` parse oadec's stderr, a
//!    media test reads it as text, and most of the CLI tests match on it. A
//!    pipeline, a log file and a test are none of them terminals, and for all of
//!    them the output stays byte for byte what it was.
//! 2. **Only from the main thread.** `decode` and the DAMF writer read the file
//!    a second time on another thread, for the checks `verify` performs beside
//!    the decode. Both walks report, so both drew on the same line and the
//!    percentage jumped between them -- the fast scan racing ahead to 54 % while
//!    the slow decode was at 23 %. The walk the command is actually doing is on
//!    the main thread; the one beside it stays quiet.
//! 3. **One at a time.** A claim, released when the reporter is dropped, so that
//!    two walks in one command cannot both draw even if a future one runs on the
//!    main thread too.
//!
//! The work is measured in bytes of the input: the size is known before the walk
//! begins, every walk reports the offset it has reached, and a byte is the one
//! unit every format has in common.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Redraw at most this often. Ten a second is smooth to read and costs nothing;
/// a write per access unit would be 9.5 million writes on a two-hour film.
const EVERY: Duration = Duration::from_millis(100);

/// Cells in the bar.
const CELLS: usize = 20;

/// Whether a reporter is currently drawing. One line, one writer.
static DRAWING: AtomicBool = AtomicBool::new(false);

/// Takes the right to draw, or reports that someone else holds it.
fn claim() -> bool {
    DRAWING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

/// Gives the right to draw back.
fn release() {
    DRAWING.store(false, Ordering::Release);
}

/// The walk a command is doing runs on the main thread; the checks that run
/// beside it do not, and must not draw over it.
fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

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
    /// When the walk began, for the rate and the estimate.
    started: Instant,
    /// When the last line was drawn.
    last: Instant,
    /// Width of the last line drawn, so exactly that much is wiped.
    last_len: usize,
    /// Whether to draw at all.
    live: bool,
    /// What the work is called, shown beside the bar.
    what: &'static str,
    /// Where the line goes.
    sink: Sink,
}

impl Progress {
    /// A reporter for a walk over `total` bytes.
    ///
    /// Draws only on a terminal, only from the main thread, and only if no other
    /// reporter is drawing. Silent when the total is unknown, because a
    /// percentage of an unknown quantity is a guess and this prints none.
    pub fn new(what: &'static str, total: u64) -> Self {
        let live = total > 0 && on_main_thread() && std::io::stderr().is_terminal() && claim();
        Self {
            total,
            started: Instant::now(),
            last: Instant::now() - EVERY,
            last_len: 0,
            live,
            what,
            sink: Sink::Stderr,
        }
    }

    /// A reporter that draws into a buffer, for testing the half a piped run
    /// can never reach. Does not take the claim, so tests do not contend.
    #[cfg(test)]
    fn for_test(what: &'static str, total: u64) -> Self {
        Self {
            total,
            started: Instant::now(),
            last: Instant::now() - EVERY,
            last_len: 0,
            live: true,
            what,
            sink: Sink::Buffer(std::rc::Rc::new(std::cell::RefCell::new(String::new()))),
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

    /// What has been drawn so far, under test.
    #[cfg(test)]
    fn drawn_text(&self) -> String {
        match &self.sink {
            Sink::Buffer(b) => b.borrow().clone(),
            Sink::Stderr => String::new(),
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
        let line = Self::render(
            self.what,
            done,
            self.total,
            self.started.elapsed().as_secs_f64(),
        );
        // a carriage return without a newline redraws the same line
        self.sink.put(&format!("\r{line}"));
        self.last_len = line.chars().count();
    }

    /// Clears the line, so whatever is printed next starts clean.
    ///
    /// Does nothing if nothing was drawn, which keeps a piped run untouched.
    pub fn clear(&mut self) {
        if !self.live || self.last_len == 0 {
            return;
        }
        self.sink
            .put(&format!("\r{:width$}\r", "", width = self.last_len));
        self.last_len = 0;
    }

    /// The line itself: a pure function of how far the walk has got and how long
    /// it has taken, so what it shows can be asserted without a terminal.
    fn render(what: &str, done: u64, total: u64, elapsed: f64) -> String {
        let done = done.min(total);
        let fraction = if total == 0 {
            0.0
        } else {
            done as f64 / total as f64
        };
        let percent = (fraction * 100.0).round() as u64;
        let filled = (fraction * CELLS as f64).round() as usize;
        let bar: String = "\u{2588}".repeat(filled) + &"\u{2591}".repeat(CELLS - filled);

        let mut line = format!(
            "{what}  {bar} {percent:>3}%  {}/{}",
            bytes(done),
            bytes(total)
        );
        // The rate needs a moment of work behind it before it means anything.
        if elapsed > 0.5 && done > 0 {
            let rate = done as f64 / elapsed;
            line.push_str(&format!("  {}/s", bytes(rate as u64)));
            let left = total.saturating_sub(done);
            line.push_str(&format!("  eta {}", eta(left as f64 / rate)));
        }
        line
    }
}

impl Drop for Progress {
    /// Clears on every exit, including the `?` that leaves a walk early, and
    /// gives the right to draw back.
    fn drop(&mut self) {
        self.clear();
        if self.live {
            release();
        }
    }
}

/// A byte count, at the scale a person reads.
fn bytes(n: u64) -> String {
    const K: f64 = 1024.0;
    let n = n as f64;
    if n >= K * K * K {
        format!("{:.1} GiB", n / (K * K * K))
    } else if n >= K * K {
        format!("{:.0} MiB", n / (K * K))
    } else if n >= K {
        format!("{:.0} KiB", n / K)
    } else {
        format!("{n:.0} B")
    }
}

/// Seconds as a clock, the way a player shows what is left.
fn eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "--".into();
    }
    let s = seconds.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::{CELLS, Progress, bytes, claim, eta, release};

    #[test]
    fn the_percentage_is_the_share_of_the_bytes_read() {
        assert!(Progress::render("reading", 0, 100, 0.0).contains("  0%"));
        assert!(Progress::render("reading", 50, 100, 0.0).contains(" 50%"));
        assert!(Progress::render("reading", 100, 100, 0.0).contains("100%"));
    }

    #[test]
    fn a_walk_past_the_size_taken_before_it_began_is_not_more_than_finished() {
        // the file can grow while it is read; 101% would read as a fault
        assert!(Progress::render("reading", 200, 100, 0.0).contains("100%"));
    }

    #[test]
    fn the_bar_fills_with_the_percentage() {
        let empty = Progress::render("reading", 0, 100, 0.0);
        let full = Progress::render("reading", 100, 100, 0.0);
        let half = Progress::render("reading", 50, 100, 0.0);
        assert_eq!(empty.matches('\u{2588}').count(), 0, "{empty}");
        assert_eq!(full.matches('\u{2588}').count(), CELLS, "{full}");
        assert_eq!(half.matches('\u{2588}').count(), CELLS / 2, "{half}");
        assert_eq!(half.matches('\u{2591}').count(), CELLS / 2, "{half}");
    }

    #[test]
    fn the_line_carries_the_bytes_and_what_is_left() {
        let line = Progress::render("reading", 1 << 30, 4 << 30, 10.0);
        assert!(line.contains("1.0 GiB/4.0 GiB"), "{line}");
        assert!(
            line.contains("/s"),
            "a rate once there is work behind it: {line}"
        );
        assert!(line.contains("eta "), "{line}");
    }

    #[test]
    fn no_rate_is_shown_before_there_is_work_behind_it() {
        // a rate from a tenth of a second is noise, and an eta from it is worse
        let line = Progress::render("reading", 1000, 100_000, 0.1);
        assert!(!line.contains("/s"), "{line}");
        assert!(!line.contains("eta"), "{line}");
    }

    #[test]
    fn byte_counts_read_at_human_scale() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(4096), "4 KiB");
        assert_eq!(bytes(5 << 20), "5 MiB");
        assert_eq!(bytes(3 << 30), "3.0 GiB");
    }

    #[test]
    fn the_estimate_reads_as_a_clock() {
        assert_eq!(eta(17.0), "0:17");
        assert_eq!(eta(72.0), "1:12");
        assert_eq!(eta(3725.0), "1:02:05");
        assert_eq!(eta(f64::INFINITY), "--");
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
        // to be printed over it.
        let seen = {
            let mut p = Progress::for_test("reading", 1000);
            p.at(500);
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
    fn the_wipe_covers_the_whole_line_that_was_drawn() {
        // a fixed-width wipe leaves the tail of a longer line on the screen
        let mut p = Progress::for_test("reading", 4 << 30);
        p.at(1 << 30);
        let drawn_len = p.drawn_text().trim_start_matches('\r').chars().count();
        p.clear();
        let wipe = p.drawn_text();
        let spaces = wipe.chars().rev().skip(1).take_while(|c| *c == ' ').count();
        assert!(
            spaces >= drawn_len,
            "wiped {spaces} columns of a {drawn_len}-column line: {wipe:?}"
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
        let mut p = Progress::new("reading", 1000);
        assert!(!p.live, "a piped run must not be live");
        p.at(500);
        p.clear();
    }

    #[test]
    fn only_the_thread_doing_the_work_may_draw() {
        // This is the rule that fixes the reported defect. `decode` and the DAMF
        // writer spawn a second walk of the same file for the checks `verify`
        // performs beside the decode, and that walk reported too: two bars on
        // one line, the percentage jumping between a fast scan at 54 % and the
        // slow decode at 23 %. The walk the command is doing is on the main
        // thread; the one beside it must stay quiet.
        //
        // The harness runs each test on its own worker thread, named after the
        // test, so this cannot assert that it is itself the main thread. What it
        // asserts is the discrimination the rule rests on. Measured separately:
        // a real binary's initial thread is Some("main") and a spawned one is
        // None, which is what makes the rule work outside the harness.
        let beside =
            std::thread::spawn(|| (super::on_main_thread(), Progress::new("reading", 1000).live))
                .join()
                .expect("the spawned walk");
        assert!(!beside.0, "a spawned walk is not the main thread");
        assert!(!beside.1, "and so it must not be live");
    }

    #[test]
    fn only_one_reporter_may_draw_at_a_time() {
        // The defect this guards: `decode` and the DAMF writer read the file a
        // second time on another thread for the checks of `verify`, and both
        // walks reported. Two reporters drew on one line and the percentage
        // jumped between them -- 54 % from the fast scan, 23 % from the slow
        // decode, on the same screen a second apart.
        assert!(claim(), "the first reporter takes the line");
        assert!(!claim(), "the second must not draw over it");
        release();
        assert!(claim(), "and the line is free again once the first is done");
        release();
    }
}
