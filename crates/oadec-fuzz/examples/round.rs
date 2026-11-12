//! One round of the campaign, looked at: what every command says of its
//! stream, and the stream in a file if one is named.
//!
//! ```text
//! cargo run -p oadec-fuzz --example round -- <fixtures> <seed> <round> [file]
//! ```
//!
//! The campaign names the seed and the round of a stream that breaks a
//! clause, and its tally names the slowest. This makes the same stream
//! again, from the same numbers.

use std::path::Path;
use std::process::ExitCode;

use oadec_fuzz::parity::{check, round, survey};
use oadec_fuzz::seeds::Seeds;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let numbers: Option<(u64, u64)> = match (args.get(1), args.get(2)) {
        (Some(seed), Some(round)) => seed.parse().ok().zip(round.parse().ok()),
        _ => None,
    };
    let (Some(fixtures), Some((seed, number))) = (args.first(), numbers) else {
        eprintln!("usage: round <fixtures> <seed> <round> [file]");
        return ExitCode::from(2);
    };
    let seeds = match Seeds::from_dir(Path::new(fixtures)) {
        Ok(seeds) => seeds,
        Err(e) => {
            eprintln!("{fixtures}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (from, bytes) = round(seeds.of("verdict_parity"), seed, number);
    if let Some(file) = args.get(3)
        && let Err(e) = std::fs::write(file, &bytes)
    {
        eprintln!("{file}: {e}");
        return ExitCode::FAILURE;
    }
    let scratch = std::env::temp_dir().join(format!("oadec-round-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&scratch) {
        eprintln!("{}: {e}", scratch.display());
        return ExitCode::FAILURE;
    }
    let started = std::time::Instant::now();
    let said = survey(&bytes, Some(&scratch));
    let took = started.elapsed();
    std::fs::remove_dir_all(&scratch).ok();
    println!(
        "round {number} of seed {seed}: stream {from} changed, {} bytes, {took:.1?}",
        bytes.len()
    );
    println!("{}", said.line());
    match check(&said) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            println!("{why}");
            ExitCode::FAILURE
        }
    }
}
