//! What a command does when what it writes to is gone.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};

/// A report written to a pipe whose reader has gone is a write that fails,
/// and the command ends with the error and exit code 2. The modules that
/// report write to what they are handed and say when they cannot, where a
/// line that was printed ended the process in a panic, with exit code 101
/// and a backtrace hint for whoever had piped a listing into `head`.
#[test]
fn a_report_into_a_pipe_that_is_closed_ends_with_an_error_and_not_a_panic() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/authored-scene.ec3"
    );
    // every payload listed in full: far more than a pipe holds
    let mut run = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["oamd", "--dump", "100000", fixture])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run oadec");
    // one line of it is read, and then nobody reads
    let mut out = BufReader::new(run.stdout.take().expect("its stdout"));
    let mut line = String::new();
    out.read_line(&mut line).expect("a line");
    assert!(!line.is_empty(), "the listing began");
    drop(out);
    let mut said = String::new();
    run.stderr
        .take()
        .expect("its stderr")
        .read_to_string(&mut said)
        .expect("what it said");
    let status = run.wait().expect("its end");
    assert_eq!(status.code(), Some(2), "{said}");
    assert!(said.starts_with("error: "), "{said}");
    assert!(!said.contains("panicked"), "{said}");
}

/// The same of a stream that comes in on standard input: the receiver of the
/// report goes, the command ends with exit code 2 and one line, and the
/// writer of the stream is not waited for. Exit code 141 is of `decode -o -`
/// alone (`docs/exit-codes.md`), and a report command keeps its 2 when its
/// stream is read as it comes.
#[test]
fn a_report_of_a_stream_piped_in_into_a_pipe_that_is_closed_ends_with_an_error() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/authored-scene.ec3"
    );
    let mut run = Command::new(env!("CARGO_BIN_EXE_oadec"))
        .args(["oamd", "--dump", "100000", "-"])
        .stdin(Stdio::from(
            std::fs::File::open(fixture).expect("the fixture"),
        ))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run oadec");
    let mut out = BufReader::new(run.stdout.take().expect("its stdout"));
    let mut line = String::new();
    out.read_line(&mut line).expect("a line");
    assert!(!line.is_empty(), "the listing began");
    drop(out);
    let mut said = String::new();
    run.stderr
        .take()
        .expect("its stderr")
        .read_to_string(&mut said)
        .expect("what it said");
    let status = run.wait().expect("its end");
    assert_eq!(status.code(), Some(2), "{said}");
    assert!(said.starts_with("error: "), "{said}");
    assert_eq!(said.lines().count(), 1, "{said}");
    assert!(!said.contains("panicked"), "{said}");
}
