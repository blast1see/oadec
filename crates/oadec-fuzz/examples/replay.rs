//! Runs one target over the inputs in files: the input the seeded fuzz run left
//! behind, or a crash libFuzzer found on a platform it links on, replayed
//! where it does not.
//!
//! ```text
//! cargo run -p oadec-fuzz --example replay -- <target> <file>...
//! ```
//!
//! A target that finds something panics, and the process ends with the
//! panic's exit code; otherwise each file is named with its length.

use std::process::ExitCode;

use oadec_fuzz::TARGETS;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let name = args.next().and_then(|name| name.into_string().ok());
    let files: Vec<_> = args.collect();
    let target = name
        .as_deref()
        .and_then(|name| TARGETS.iter().find(|(known, _)| *known == name));
    let (Some((name, target)), false) = (target, files.is_empty()) else {
        eprintln!("usage: replay <target> <file>...");
        let names: Vec<&str> = TARGETS.iter().map(|(name, _)| *name).collect();
        eprintln!("targets: {}", names.join(", "));
        return ExitCode::from(2);
    };
    for file in files {
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("{}: {e}", file.to_string_lossy());
                return ExitCode::FAILURE;
            }
        };
        target(&bytes);
        println!(
            "{name}: {} bytes of {} hold",
            bytes.len(),
            file.to_string_lossy()
        );
    }
    ExitCode::SUCCESS
}
