//! Every bit of every short stream changed, one at a time, and each stream
//! put to every command.
//!
//! ```text
//! cargo run --release -p oadec-fuzz --example flips -- <fixtures> [tally.json] [directory]
//! ```
//!
//! The campaign changes streams at random and can only say what it happened
//! on. This goes through the whole of what is one bit away from the streams
//! the campaign starts from: for each stream and each of its bits, the bit is
//! changed, the checksums are put right, every command is asked and the
//! answers are held to the policy of the exit codes. It is exhaustive and
//! takes minutes, so it is a tool and not a test.
//!
//! The tally says how often each set of answers came and at which bit first.
//! With a directory, the first stream of every kind in which a metadata
//! report finds a fault and `verify` finds none is written there, which the
//! policy lets happen and a random change seldom makes.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;

use oadec_fuzz::parity::{Said, Survey, check, repair, survey};
use oadec_fuzz::seeds::Seeds;

/// How often a set of answers came, and the first bit that gave it.
type Kinds = BTreeMap<String, (u64, usize)>;

/// `stream` with one bit changed and its checksums put right.
fn changed(stream: &[u8], bit: usize) -> Vec<u8> {
    let mut out = stream.to_vec();
    out[bit / 8] ^= 0x80 >> (bit % 8);
    repair(&mut out);
    out
}

/// Whether a metadata report finds a fault in a stream `verify` calls clean.
fn stricter(said: &Survey) -> bool {
    said.verify == Said::Clean && (said.oamd == Said::Faulty || said.emdf == Said::Faulty)
}

/// Every bit of `stream`, changed one at a time on as many threads as the
/// machine has: the answers that came, and the clauses that broke.
fn every_bit(stream: &[u8], scratch: &Path) -> (Kinds, Vec<String>) {
    let bits = stream.len() * 8;
    let kinds: Mutex<Kinds> = Mutex::new(BTreeMap::new());
    let broken: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, std::num::NonZero::get);
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let (kinds, broken) = (&kinds, &broken);
            scope.spawn(move || {
                let dir = scratch.join(format!("w{worker}"));
                std::fs::create_dir_all(&dir).expect("a scratch directory");
                let mut local = Kinds::new();
                for bit in (worker..bits).step_by(workers) {
                    let said = survey(&changed(stream, bit), Some(&dir));
                    if let Err(why) = check(&said) {
                        broken.lock().unwrap().push(format!("bit {bit}: {why}"));
                    }
                    let entry = local.entry(said.line()).or_insert((0, bit));
                    entry.0 += 1;
                }
                let mut all = kinds.lock().unwrap();
                for (line, (count, first)) in local {
                    let entry = all.entry(line).or_insert((0, first));
                    entry.0 += count;
                    entry.1 = entry.1.min(first);
                }
            });
        }
    });
    let mut broken = broken.into_inner().unwrap();
    broken.sort();
    (kinds.into_inner().unwrap(), broken)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(fixtures) = args.first() else {
        eprintln!("usage: flips <fixtures> [tally.json] [directory]");
        return ExitCode::from(2);
    };
    let tally: Option<PathBuf> = args.get(1).map(PathBuf::from);
    let keep: Option<PathBuf> = args.get(2).map(PathBuf::from);
    let seeds = match Seeds::from_dir(Path::new(fixtures)) {
        Ok(seeds) => seeds,
        Err(e) => {
            eprintln!("{fixtures}: {e}");
            return ExitCode::FAILURE;
        }
    };
    // beside the program, in the directory of the build: every round writes
    // and removes files, which the temporary directory of a system can make
    // the slowest part of the round
    let scratch = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("flips-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("a scratch directory");

    let streams = seeds.of("verdict_parity");
    let (mut rounds, mut clauses_broken, mut alone) = (0u64, 0usize, 0u64);
    let mut json = String::from("{\n \"streams\": [\n");
    for (index, stream) in streams.iter().enumerate() {
        let (kinds, broken) = every_bit(stream, &scratch);
        let bits = stream.len() * 8;
        rounds += bits as u64;
        clauses_broken += broken.len();
        println!(
            "stream {index}: {} bytes, {bits} bits, {} kinds of answers, {} clauses broken",
            stream.len(),
            kinds.len(),
            broken.len()
        );
        for why in broken.iter().take(5) {
            println!("  BROKEN {why}");
        }
        let mut rows: Vec<_> = kinds.iter().collect();
        rows.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        let mut stream_alone = 0u64;
        let mut answers = String::new();
        for (line, (count, first)) in rows {
            // the first stream of the kind, made again to be looked at
            let again = changed(stream, *first);
            if stricter(&survey(&again, Some(&scratch))) {
                stream_alone += count;
                println!("  {count:>7}  {line}   (first at bit {first})");
                if let Some(dir) = &keep {
                    std::fs::create_dir_all(dir).expect("the directory");
                    let name = dir.join(format!("stream{index}-bit{first}.bin"));
                    std::fs::write(&name, &again).expect("a stream kept");
                }
            }
            let comma = if answers.is_empty() { "" } else { ",\n" };
            write!(
                answers,
                "{comma}    {{\"answers\": \"{line}\", \"rounds\": {count}, \"first_bit\": {first}}}"
            )
            .unwrap();
        }
        alone += stream_alone;
        let comma = if index == 0 { "" } else { ",\n" };
        write!(
            json,
            "{comma}  {{\"stream\": {index}, \"bytes\": {}, \"rounds\": {bits}, \"clauses_broken\": {}, \
             \"a_metadata_report_alone_finds_a_fault\": {stream_alone}, \"answers\": [\n{answers}\n   ]}}",
            stream.len(),
            broken.len()
        )
        .unwrap();
    }
    std::fs::remove_dir_all(&scratch).ok();
    write!(
        json,
        "\n ],\n \"streams_changed\": {},\n \"rounds\": {rounds},\n \"clauses_broken\": {clauses_broken},\n \
         \"a_metadata_report_alone_finds_a_fault\": {alone}\n}}\n",
        streams.len()
    )
    .unwrap();
    println!(
        "{} streams, {rounds} bits changed one at a time, {clauses_broken} clauses broken; \
         a metadata report alone found a fault in {alone}",
        streams.len()
    );
    if let Some(path) = &tally
        && let Err(e) = std::fs::write(path, json)
    {
        eprintln!("{}: {e}", path.display());
        return ExitCode::FAILURE;
    }
    if clauses_broken == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
