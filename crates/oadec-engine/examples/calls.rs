//! The calls at the root of the engine over the streams named on the command
//! line: for each a line of JSON with what `probe`, `verify` and `metadata`
//! say of it, or the kind of what a call could not do and its words.
//!
//! ```text
//! cargo run --release -p oadec-engine --example calls -- <stream>...
//! ```
//!
//! A caller needs to know nothing of a stream before it asks: the same three
//! calls answer for a TrueHD, an MLP, an AC-3 and an E-AC-3 stream, and for a
//! file that is none of them.

use std::path::Path;

use oadec_engine::Error;
use oadec_engine::api::{Source, VerifyOptions};
use serde_json::{Value, json};

/// What a call could not do: its kind, and the line it is written as.
fn failed(e: &Error) -> Value {
    let kind = match e {
        Error::Io(_) => "Io",
        Error::NoStream(_) => "NoStream",
        Error::Refused(_) => "Refused",
        Error::Stream(_) => "Stream",
    };
    json!({ "failed": kind, "words": e.to_string() })
}

fn main() {
    for name in std::env::args().skip(1) {
        let source = Source::file(Path::new(&name));
        let probe = match oadec_engine::probe(&source) {
            Ok(probe) => json!({ "format": probe.format.name() }),
            Err(e) => failed(&e),
        };
        let verify = match oadec_engine::verify(&source, &VerifyOptions::default()) {
            Ok(report) => json!({
                "format": report.format.name(),
                "clean": report.clean,
                "first_problem": report.first_problem,
                "fatal": report.tally.fatal,
                "error": report.tally.error,
                "warning": report.tally.warning,
                "info": report.tally.info,
                "rules": report.tally.rules.len(),
                "coverage": report.coverage.len(),
                "regions": report.regions.len(),
                "containers": report.protection["containers"],
            }),
            Err(e) => failed(&e),
        };
        let metadata = match oadec_engine::metadata(&source) {
            Ok(metadata) => json!({
                "objects_clean": metadata.objects.clean(),
                "containers_clean": metadata.containers.clean(),
            }),
            Err(e) => failed(&e),
        };
        println!(
            "{}",
            json!({ "stream": name, "probe": probe, "verify": verify, "metadata": metadata })
        );
    }
}
