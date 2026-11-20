//! What a decode to standard output is held to.
//!
//! `decode -o -` writes its samples to standard output, and nothing else
//! goes there: what the run has to say, its summary, its warnings, its
//! verdict and its JSON report, goes to standard error or to the file
//! `--report` names. What would put anything else on standard output, or
//! samples on a terminal, is refused before the stream is read (exit 2).
//!
//! The receiver may close the pipe before the samples end, as `head -c`
//! does. That is not a failure of the decode, and it ends the run with one
//! line on standard error and [`EXIT_RECEIVER_CLOSED`]. Windows and the
//! shells that run on it do not report the closed pipe with one error kind,
//! so [`receiver_closed`] is the one place that knows them all.

use std::io;
use std::path::Path;

use oadec_engine::decode::Format;
use oadec_engine::stream::{LAYOUT_CHANGE_IS_SAMPLES, LayoutChange};

/// What the command line writes for standard output where it would name a
/// file.
pub(crate) const STDOUT: &str = "-";

/// The exit code of a decode whose receiver closed standard output before
/// the samples ended. It is the status a shell gives a process that a
/// `SIGPIPE` ended (128 + 13), so a script that already lets `head` close a
/// pipe early lets `oadec` do it too.
#[allow(
    dead_code,
    reason = "the decode to standard output ends with it once it is built"
)]
pub(crate) const EXIT_RECEIVER_CLOSED: u8 = 141;

/// What a decode says until it can write to standard output.
pub(crate) const NOT_YET: &str = "decoding to or from a pipe as the stream arrives \
     (-o -, --json, --report, --on-layout-change stop on an E-AC-3 stream) is not implemented yet";

/// What a decode is asked of where its output and its report go.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Asked<'a> {
    /// The output container.
    pub format: Format,
    /// The samples go to standard output (`-o -`).
    pub to_stdout: bool,
    /// The report is asked for as JSON (`--json`).
    pub json: bool,
    /// Where `--report` writes the JSON report; `-` is standard output.
    pub report: Option<&'a Path>,
    /// Standard output is a terminal.
    pub stdout_is_terminal: bool,
    /// What `--on-layout-change` asks.
    pub on_layout_change: Option<LayoutChange>,
}

/// Refuses what a decode cannot do with where its output and its report go,
/// before anything is read.
///
/// # Errors
/// An object output to standard output, which is gone back over when it is
/// closed; samples to a terminal; JSON to standard output beside samples;
/// a report or a policy for a change of layout with an object output.
pub(crate) fn check(asked: &Asked<'_>) -> anyhow::Result<()> {
    let objects = match asked.format {
        Format::Damf => Some("damf"),
        Format::Adm => Some("adm"),
        Format::Pcm | Format::Wav => None,
    };
    if let (true, Some(name)) = (asked.to_stdout, objects) {
        anyhow::bail!(
            "--format {name} cannot be written to standard output (-o -): its files are gone \
             back over when they are closed, and a pipe cannot be; name a base path, or ask for \
             --format pcm or wav. The stream itself can come in on standard input"
        );
    }
    if asked.to_stdout && asked.stdout_is_terminal {
        anyhow::bail!(
            "-o - writes binary samples to standard output, and standard output is a terminal \
             here: pipe it into a program, or name a file"
        );
    }
    if asked.to_stdout && asked.report.is_some_and(|r| r.as_os_str() == STDOUT) {
        anyhow::bail!(
            "--report - writes the JSON report to standard output, which -o - fills with \
             samples: name a file, or use --json, which writes it to standard error"
        );
    }
    if objects.is_some() && (asked.json || asked.report.is_some()) {
        anyhow::bail!(
            "--json and --report report a decode to samples: they apply to --format pcm or wav; \
             an object output writes its ledger with --loss-report"
        );
    }
    if objects.is_some() && asked.on_layout_change.is_some() {
        anyhow::bail!(LAYOUT_CHANGE_IS_SAMPLES);
    }
    Ok(())
}

/// Whether `err` says that the receiver of what was written closed the pipe.
///
/// Rust names it [`io::ErrorKind::BrokenPipe`] where it knows the code of
/// the system. On Windows the same condition comes back as a pipe that is
/// being closed or is not connected as well, depending on who held the other
/// end; each of those is the receiver gone, and none is a failure of the
/// decode.
#[allow(
    dead_code,
    reason = "the decode to standard output asks it once it is built"
)]
pub(crate) fn receiver_closed(err: &anyhow::Error) -> bool {
    err.chain()
        .filter_map(|cause| cause.downcast_ref::<io::Error>())
        .any(is_receiver_closed)
}

/// [`receiver_closed`] for one error of a write.
pub(crate) fn is_receiver_closed(err: &io::Error) -> bool {
    let _ = err;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(format: Format) -> Asked<'static> {
        Asked {
            format,
            to_stdout: true,
            json: false,
            report: None,
            stdout_is_terminal: false,
            on_layout_change: None,
        }
    }

    #[test]
    fn an_object_output_is_refused_on_standard_output_by_its_name() {
        for (format, name) in [(Format::Damf, "damf"), (Format::Adm, "adm")] {
            let refused = check(&asked(format)).expect_err("refused");
            assert!(
                refused
                    .to_string()
                    .starts_with(&format!("--format {name} ")),
                "{refused}"
            );
            // and to a file it is not
            let to_file = Asked {
                to_stdout: false,
                ..asked(format)
            };
            assert!(check(&to_file).is_ok());
        }
        assert!(check(&asked(Format::Pcm)).is_ok());
        assert!(check(&asked(Format::Wav)).is_ok());
    }

    /// No test can hand the binary a terminal through a pipe, so the
    /// decision is held here: samples to a terminal are refused, and to a
    /// pipe or a file they are not.
    #[test]
    fn samples_are_not_written_to_a_terminal() {
        let terminal = Asked {
            stdout_is_terminal: true,
            ..asked(Format::Pcm)
        };
        let refused = check(&terminal).expect_err("refused");
        assert!(refused.to_string().contains("terminal"), "{refused}");
        // a file is written whatever standard output is
        assert!(
            check(&Asked {
                to_stdout: false,
                ..terminal
            })
            .is_ok()
        );
    }

    #[test]
    fn json_goes_to_standard_output_only_when_samples_do_not() {
        let dash = Path::new("-");
        let file = Path::new("report.json");
        let refused = check(&Asked {
            report: Some(dash),
            ..asked(Format::Wav)
        })
        .expect_err("refused");
        assert!(refused.to_string().starts_with("--report -"), "{refused}");
        for ok in [
            Asked {
                report: Some(file),
                ..asked(Format::Wav)
            },
            Asked {
                json: true,
                ..asked(Format::Pcm)
            },
            Asked {
                report: Some(dash),
                to_stdout: false,
                ..asked(Format::Pcm)
            },
        ] {
            assert!(check(&ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn a_closed_pipe_is_the_receiver_gone_however_it_is_said() {
        let broken = io::Error::from(io::ErrorKind::BrokenPipe);
        assert!(is_receiver_closed(&broken));
        assert!(!is_receiver_closed(&io::Error::from(
            io::ErrorKind::NotFound
        )));
        assert!(!is_receiver_closed(&io::Error::other("full")));
        // and through the context a write is given on the way up
        let wrapped = anyhow::Error::new(broken).context("writing to standard output");
        assert!(receiver_closed(&wrapped));
        assert!(!receiver_closed(&anyhow::anyhow!(
            "a broken pipe, in words"
        )));
    }

    /// The codes Windows gives a write to a pipe whose reader has gone:
    /// `ERROR_BROKEN_PIPE`, `ERROR_NO_DATA` (the pipe is being closed) and
    /// `ERROR_PIPE_NOT_CONNECTED`.
    #[cfg(windows)]
    #[test]
    fn every_code_windows_gives_a_closed_pipe_is_the_receiver_gone() {
        for code in [109, 232, 233] {
            assert!(
                is_receiver_closed(&io::Error::from_raw_os_error(code)),
                "{code}"
            );
        }
        // a handle that is wrong is not a reader that went away
        assert!(!is_receiver_closed(&io::Error::from_raw_os_error(6)));
    }
}
