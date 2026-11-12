//! The campaign: seeded changes to the streams of the repository, each put
//! to every command that reads a stream, the object outputs among them.
//!
//! A round is one changed stream. Three rounds in four have their checksums
//! put right after the change, so that the parsers behind them see it; the
//! fourth is left as a medium would leave it. What the commands say of a
//! round has to hang together as `oadec_fuzz::parity::check` has it, and the
//! campaign counts what it saw beside: which answers came how often, and how
//! often a change went through every command without a word.
//!
//! ```text
//! OADEC_CAMPAIGN_ITERS=200000 cargo test --release -p oadec-fuzz --test campaign -- --nocapture
//! ```
//!
//! `OADEC_CAMPAIGN_SEED` moves the run to other streams. A round that breaks
//! a clause leaves its stream in a file and names it.

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use oadec_fuzz::parity::{self, Said, Survey, check, survey};
use oadec_fuzz::seeds::Seeds;

/// Rounds when the environment sets none: enough to see every kind of
/// answer, short enough for every `cargo test`.
const DEFAULT_ROUNDS: u64 = 3000;

/// The longest a round may take. A round is a few kilobytes through seven
/// commands and takes milliseconds; the bound is far above what a loaded
/// machine makes of that, and far below a loop that does not end.
const LONGEST: Duration = Duration::from_secs(60);

fn streams() -> &'static [Vec<u8>] {
    static STREAMS: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    STREAMS.get_or_init(|| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../oadec-cli/tests/fixtures");
        let seeds = Seeds::from_dir(Path::new(dir)).expect("the fixtures are in the repository");
        seeds.of("verdict_parity").to_vec()
    })
}

fn number(name: &str, default: u64) -> u64 {
    match std::env::var(name) {
        Ok(text) => text
            .parse()
            .unwrap_or_else(|_| panic!("{name} is not a number: {text}")),
        Err(_) => default,
    }
}

/// The stream of `round`.
fn input(base: u64, round: u64) -> (usize, Vec<u8>) {
    parity::round(streams(), base, round)
}

/// What a run saw.
#[derive(Debug, Default)]
struct Tally {
    rounds: u64,
    /// How often each set of answers came.
    answers: BTreeMap<String, u64>,
    /// Rounds every command called clean.
    clean: u64,
    /// Of those, the ones whose stream is not the seed's any more.
    clean_and_changed: u64,
    /// Of those, the ones that decode to other samples than the seed: a
    /// change no command noticed, in TrueHD or MLP and in the AC-3 family.
    other_samples_truehd: u64,
    other_samples_ac3: u64,
    /// Rounds a metadata report found a fault in what it read where `verify`
    /// found none, which the policy allows them.
    metadata_stricter: u64,
    /// Rounds an object output was refused though every report was clean.
    objects_refused: u64,
    /// The longest a round took, and which round it was.
    slowest: Duration,
    slowest_round: u64,
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.rounds += other.rounds;
        for (line, count) in other.answers {
            *self.answers.entry(line).or_default() += count;
        }
        self.clean += other.clean;
        self.clean_and_changed += other.clean_and_changed;
        self.other_samples_truehd += other.other_samples_truehd;
        self.other_samples_ac3 += other.other_samples_ac3;
        self.metadata_stricter += other.metadata_stricter;
        self.objects_refused += other.objects_refused;
        if other.slowest > self.slowest {
            self.slowest = other.slowest;
            self.slowest_round = other.slowest_round;
        }
    }

    fn count(&self, of: impl Fn(&str) -> bool) -> u64 {
        self.answers
            .iter()
            .filter(|(line, _)| of(line))
            .map(|(_, count)| count)
            .sum()
    }

    fn json(&self, base: u64) -> String {
        let mut rows: Vec<(&String, &u64)> = self.answers.iter().collect();
        rows.sort_by_key(|(line, count)| (std::cmp::Reverse(**count), line.as_str()));
        let answers: Vec<String> = rows
            .iter()
            .map(|(line, count)| format!("  {{\"answers\": \"{line}\", \"rounds\": {count}}}"))
            .collect();
        format!(
            "{{\n \"seed\": {base},\n \"rounds\": {},\n \"streams_changed\": {},\n \
             \"every_command_clean\": {},\n \"clean_and_not_the_seed\": {},\n \
             \"clean_with_other_samples_truehd\": {},\n \"clean_with_other_samples_ac3\": {},\n \
             \"a_metadata_report_stricter_than_verify\": {},\n \
             \"an_object_output_refused_where_every_report_is_clean\": {},\n \
             \"slowest_round_ms\": {},\n \"slowest_round\": {},\n \"kinds_of_answers\": {},\n \
             \"answers\": [\n{}\n ]\n}}\n",
            self.rounds,
            streams().len(),
            self.clean,
            self.clean_and_changed,
            self.other_samples_truehd,
            self.other_samples_ac3,
            self.metadata_stricter,
            self.objects_refused,
            self.slowest.as_millis(),
            self.slowest_round,
            self.answers.len(),
            answers.join(",\n"),
        )
    }
}

/// What every command says of each seed as it is: the samples a round is
/// held against.
fn of_the_seeds() -> &'static [Survey] {
    static SURVEYS: OnceLock<Vec<Survey>> = OnceLock::new();
    SURVEYS.get_or_init(|| streams().iter().map(|s| survey(s, None)).collect())
}

fn keep(base: u64, round: u64, bytes: &[u8]) -> String {
    let file = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("campaign-seed-{base}-round-{round}.bin"));
    match std::fs::write(&file, bytes) {
        Ok(()) => format!("; the stream is in {}", file.display()),
        Err(_) => String::new(),
    }
}

/// The rounds of one worker: every `workers`-th one from `first`.
fn work(base: u64, rounds: u64, first: u64, workers: u64, scratch: &Path) -> Tally {
    std::fs::create_dir_all(scratch).expect("a directory to write the object outputs to");
    let mut tally = Tally::default();
    for round in (first..rounds).step_by(workers as usize) {
        let (from, bytes) = input(base, round);
        let started = Instant::now();
        let said = match panic::catch_unwind(AssertUnwindSafe(|| survey(&bytes, Some(scratch)))) {
            Ok(said) => said,
            Err(cause) => {
                eprintln!(
                    "a command panicked on round {round} of OADEC_CAMPAIGN_SEED={base}{}",
                    keep(base, round, &bytes)
                );
                panic::resume_unwind(cause);
            }
        };
        let took = started.elapsed();
        if took > tally.slowest {
            tally.slowest = took;
            tally.slowest_round = round;
        }
        if let Err(why) = check(&said) {
            panic!(
                "round {round} of OADEC_CAMPAIGN_SEED={base}, a change to stream {from}: {why}{}",
                keep(base, round, &bytes)
            );
        }
        tally.rounds += 1;
        *tally.answers.entry(said.line()).or_default() += 1;
        let seed = &of_the_seeds()[from];
        if said.all_clean() {
            tally.clean += 1;
            if bytes != streams()[from] {
                tally.clean_and_changed += 1;
                if said.samples != seed.samples {
                    if said.ac3 {
                        tally.other_samples_ac3 += 1;
                    } else {
                        tally.other_samples_truehd += 1;
                    }
                }
            }
        }
        if said.verify == Said::Clean && (said.oamd == Said::Faulty || said.emdf == Said::Faulty) {
            tally.metadata_stricter += 1;
        }
        if said.verify_decode == Said::Clean
            && !said.objects.is_empty()
            && said.objects.iter().all(|o| *o == Said::Refused)
            && seed.verify == Said::Clean
            && of_the_objects(from)
        {
            tally.objects_refused += 1;
        }
    }
    std::fs::remove_dir_all(scratch).ok();
    tally
}

/// Whether the seed itself decodes to an object output: a refusal counts
/// only where there was a programme to refuse.
fn of_the_objects(from: usize) -> bool {
    static CARRY: OnceLock<Vec<bool>> = OnceLock::new();
    CARRY.get_or_init(|| {
        let scratch = scratch_of("seeds");
        std::fs::create_dir_all(&scratch).expect("a directory");
        let carry = streams()
            .iter()
            .map(|s| survey(s, Some(&scratch)).objects.contains(&Said::Clean))
            .collect();
        std::fs::remove_dir_all(&scratch).ok();
        carry
    })[from]
}

fn scratch_of(tag: &str) -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("campaign-{}-{tag}", std::process::id()))
}

fn run(base: u64, rounds: u64) -> Tally {
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get() as u64)
        .min(rounds.max(1));
    let mut tally = Tally::default();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let scratch = scratch_of(&format!("worker-{worker}"));
                scope.spawn(move || work(base, rounds, worker, workers, &scratch))
            })
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(part) => tally.add(part),
                Err(cause) => panic::resume_unwind(cause),
            }
        }
    });
    tally
}

/// Every seed, as it is, is a stream every command agrees on: a campaign
/// that began from streams the commands already disagree on would say
/// nothing of a change.
#[test]
fn the_commands_agree_on_every_seed() {
    assert!(streams().len() >= 60, "{} streams", streams().len());
    for (index, said) in of_the_seeds().iter().enumerate() {
        assert_eq!(check(said), Ok(()), "stream {index}");
    }
}

#[test]
fn the_commands_agree_on_every_changed_stream() {
    let rounds = number("OADEC_CAMPAIGN_ITERS", DEFAULT_ROUNDS);
    let base = number("OADEC_CAMPAIGN_SEED", 0);
    let tally = run(base, rounds);
    assert_eq!(tally.rounds, rounds);
    assert!(
        tally.slowest < LONGEST,
        "round {} took {:?}",
        tally.slowest_round,
        tally.slowest
    );

    let report = tally.json(base);
    let file = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("campaign-seed-{base}.json"));
    std::fs::write(&file, &report).expect("the tally written");
    println!("{report}");
    println!("the tally is in {}", file.display());

    // A campaign that only ever saw one kind of answer has tested one clause.
    // These are the kinds every clause needs, and a run of the default
    // length has to have met each of them.
    let seen = [
        ("every command clean", tally.clean),
        (
            "no stream to any command",
            tally.count(|line| line.contains("info - verify -")),
        ),
        (
            "a fault verify finds",
            tally.count(|line| line.contains("verify F")),
        ),
        (
            "a fault only a decode shows",
            tally.count(|line| line.contains("verify c decode F")),
        ),
        (
            "a stream of the AC-3 family",
            tally.count(|line| line.starts_with("ac3")),
        ),
        (
            "an object output written",
            tally.count(|line| line.ends_with("objects cc") || line.ends_with("objects FF")),
        ),
    ];
    if base == 0 && rounds >= DEFAULT_ROUNDS {
        for (kind, count) in seen {
            assert!(count > 0, "no round with {kind} in {rounds} rounds");
        }
    }
}

/// The streams earlier campaigns turned up, and the one that changing every
/// bit in turn did, are kept beside the fixtures. What the commands say of
/// each hangs together, and the ones that show what the policy lets a command
/// say that another does not still show it: without the first of them, the
/// exception in `check` would be held by nothing.
#[test]
fn the_streams_a_campaign_found_still_hold() {
    let dir = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../oadec-cli/tests/found"
    ));
    let scratch = scratch_of("found");
    std::fs::create_dir_all(&scratch).expect("a directory");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).expect("the streams that were found") {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|e| e != "bin") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let said = survey(&std::fs::read(&path).expect("read"), Some(&scratch));
        assert_eq!(check(&said), Ok(()), "{name}");
        seen += 1;
        match name.as_str() {
            "eac3-complexity-index-mismatch.bin" => {
                assert_eq!(said.verify, Said::Faulty, "{name}");
                assert!(!said.delivery_fault, "{name}");
                assert!(said.deliveries().all(|d| d == Said::Clean), "{name}");
            }
            "eac3-payload-without-object-element.bin"
            | "truehd-payload-without-object-element.bin" => {
                assert_eq!(said.verify, Said::Faulty, "{name}");
                assert!(said.delivery_fault, "{name}");
                assert!(said.deliveries().all(|d| d == Said::Faulty), "{name}");
            }
            "truehd-presentation-over-missing-substream.bin" => {
                assert_eq!(said.verify, Said::Clean, "{name}");
                assert_eq!(said.verify_decode, Said::Faulty, "{name}");
                assert_eq!(
                    said.pcm,
                    [Said::Clean, Said::Refused, Said::Refused, Said::Refused],
                    "{name}"
                );
            }
            "truehd-major-sync-unreadable.bin" => {
                assert_eq!(said.verify, Said::Faulty, "{name}");
                assert!(said.deliveries().all(|d| d == Said::Refused), "{name}");
            }
            // what a metadata report alone counts: nothing else finds a fault
            "truehd-payload-long-padding.bin" => {
                assert_eq!(said.oamd, Said::Faulty, "{name}");
                assert_eq!(said.verify, Said::Clean, "{name}");
                assert_eq!(said.verify_decode, Said::Clean, "{name}");
                assert_eq!(said.emdf, Said::Clean, "{name}");
                assert!(said.deliveries().all(|d| d == Said::Clean), "{name}");
            }
            // a warning: counted, and a fault to no command
            "eac3-element-of-a-stale-size.bin" => {
                assert_eq!(said.verify, Said::Clean, "{name}");
                assert_eq!(said.oamd, Said::Clean, "{name}");
                assert!(said.deliveries().all(|d| d == Said::Clean), "{name}");
            }
            other => panic!("{other} is a found stream this test says nothing of"),
        }
    }
    std::fs::remove_dir_all(&scratch).ok();
    assert_eq!(seen, 7);
}

/// A round is the same stream every time, and another seed gives others.
#[test]
fn a_round_is_the_same_stream_every_time() {
    assert_eq!(input(0, 17), input(0, 17));
    let differing = (0..50)
        .filter(|&round| input(0, round) != input(1, round))
        .count();
    assert!(
        differing >= 45,
        "{differing} of 50 rounds differ between two seeds"
    );
}
