//! Writes the seeds cut from a directory of streams as a libFuzzer corpus:
//! a directory per target, a file per seed.
//!
//! ```text
//! cargo run -p oadec-fuzz --example corpus -- crates/oadec-cli/tests/fixtures fuzz/corpus
//! ```

use std::path::Path;
use std::process::ExitCode;

use oadec_fuzz::TARGETS;
use oadec_fuzz::seeds::Seeds;

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [streams, corpus] = args.as_slice() else {
        eprintln!("usage: corpus <directory of streams> <corpus directory>");
        return ExitCode::from(2);
    };
    let seeds = match Seeds::from_dir(Path::new(streams)) {
        Ok(seeds) => seeds,
        Err(e) => {
            eprintln!("{}: {e}", Path::new(streams).display());
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = seeds.write_corpus(Path::new(corpus)) {
        eprintln!("{}: {e}", Path::new(corpus).display());
        return ExitCode::FAILURE;
    }
    for (name, _) in TARGETS {
        let set = seeds.of(name);
        let bytes: usize = set.iter().map(Vec::len).sum();
        println!("{name}: {} seeds, {bytes} bytes", set.len());
    }
    ExitCode::SUCCESS
}
