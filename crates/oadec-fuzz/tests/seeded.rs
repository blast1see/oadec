//! The seeded fuzz run: every target over the seeds cut from the committed
//! fixtures, and over seeded changes to them.
//!
//! A round is one changed input. `OADEC_FUZZ_ITERS` sets the rounds per
//! target, `OADEC_FUZZ_SEED` moves the whole run to other inputs, and a
//! failure names both along with the round, and leaves its input in a file:
//!
//! ```text
//! OADEC_FUZZ_ITERS=200000 cargo test --release -p oadec-fuzz --test seeded
//! cargo run -p oadec-fuzz --example replay -- <target> <file>
//! ```

use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::OnceLock;

use oadec_fuzz::TARGETS;
use oadec_fuzz::mutate::{Rng, mutate};
use oadec_fuzz::seeds::{Seeds, fnv1a};

/// Rounds per target when the environment sets none: enough to fail on a
/// parser that stops checking a length, short enough for every `cargo test`.
const DEFAULT_ROUNDS: u64 = 300;

fn seeds() -> &'static Seeds {
    static SEEDS: OnceLock<Seeds> = OnceLock::new();
    SEEDS.get_or_init(|| {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../oadec-cli/tests/fixtures");
        Seeds::from_dir(Path::new(dir)).expect("the fixtures are in the repository")
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

/// The input of `round`: a seed of the target, mutated. The same numbers
/// give the same bytes on every machine.
fn input(name: &str, seeds: &[Vec<u8>], base: u64, round: u64) -> (usize, Vec<u8>) {
    let mut rng = Rng::new(fnv1a(name.as_bytes()) ^ base.rotate_left(32) ^ round);
    let from = rng.below(seeds.len());
    let donor = &seeds[rng.below(seeds.len())];
    let mut bytes = seeds[from].clone();
    mutate(&mut bytes, donor, &mut rng);
    (from, bytes)
}

fn run(name: &str) {
    let (_, target) = TARGETS
        .iter()
        .find(|(known, _)| *known == name)
        .expect("a target by that name");
    let seeds = seeds().of(name);
    assert!(!seeds.is_empty(), "{name} has no seed");
    // what holds of any input holds of the streams the seeds were cut from
    for seed in seeds {
        target(seed);
    }
    let rounds = number("OADEC_FUZZ_ITERS", DEFAULT_ROUNDS);
    let base = number("OADEC_FUZZ_SEED", 0);
    for round in 0..rounds {
        let (from, bytes) = input(name, seeds, base, round);
        if let Err(cause) = panic::catch_unwind(AssertUnwindSafe(|| target(&bytes))) {
            let file = Path::new(env!("CARGO_TARGET_TMPDIR"))
                .join(format!("{name}-seed-{base}-round-{round}.bin"));
            let kept = std::fs::write(&file, &bytes).is_ok();
            eprintln!(
                "{name} failed on round {round} of OADEC_FUZZ_SEED={base}, a mutation of its \
                 seed {from}, {} bytes{}",
                bytes.len(),
                if kept {
                    format!("; the input is in {}", file.display())
                } else {
                    String::new()
                }
            );
            panic::resume_unwind(cause);
        }
    }
}

macro_rules! targets {
    ($($name:ident),* $(,)?) => {
        /// The targets this file runs.
        const RUN: &[&str] = &[$(stringify!($name)),*];
        $(
            #[test]
            fn $name() {
                run(stringify!($name));
            }
        )*
    };
}

targets!(
    truehd_extract,
    truehd_unit,
    truehd_unit_repaired,
    truehd_decode,
    truehd_decode_repaired,
    emdf_container,
    oamd_parse,
    joc_params,
    eac3_frame,
    eac3_program,
    verdict_parity,
);

/// A target added to the crate and not to the list above would never run.
#[test]
fn every_target_of_the_crate_is_run() {
    let known: Vec<&str> = TARGETS.iter().map(|(name, _)| *name).collect();
    assert_eq!(known, RUN);
}

/// Nor would libFuzzer run one that has no wrapper under `fuzz/`, no entry
/// in that manifest or no line in the workflow's matrix; and a wrapper for a
/// target that is gone would not build.
#[test]
fn every_target_has_its_libfuzzer_wrapper_and_its_place_in_the_workflow() {
    let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let read = |path: &str| {
        std::fs::read_to_string(root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
    };
    let known: Vec<&str> = TARGETS.iter().map(|(name, _)| *name).collect();

    let manifest = read("fuzz/Cargo.toml");
    let bins: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.strip_prefix("path = \"fuzz_targets/"))
        .filter_map(|rest| rest.strip_suffix(".rs\""))
        .collect();
    assert_eq!(bins, known, "the binaries of fuzz/Cargo.toml");

    let mut wrappers: Vec<String> = std::fs::read_dir(root.join("fuzz/fuzz_targets"))
        .expect("fuzz/fuzz_targets")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    wrappers.sort();
    let mut sorted = known.clone();
    sorted.sort_unstable();
    assert_eq!(wrappers, sorted, "the files of fuzz/fuzz_targets");
    for name in &known {
        let wrapper = read(&format!("fuzz/fuzz_targets/{name}.rs"));
        assert!(
            wrapper.contains(&format!("oadec_fuzz::targets::{name}(data)")),
            "fuzz/fuzz_targets/{name}.rs does not hand its input to {name}"
        );
    }

    let workflow = read(".github/workflows/fuzz.yml");
    let matrix: Vec<&str> = workflow
        .lines()
        .skip_while(|line| line.trim() != "target:")
        .skip(1)
        .map_while(|line| line.trim().strip_prefix("- "))
        .collect();
    assert_eq!(matrix, known, "the matrix of .github/workflows/fuzz.yml");
}

/// The run is only worth its name if the mutations get past the door: of the
/// rounds of each target, a good part must still be read as far as the seeds
/// are. Measured here on the framing, where it can be said without the
/// target's help: the mutated input still opens as what it was.
#[test]
fn most_mutations_still_open() {
    use oadec_eac3::FrameHeader;
    use oadec_truehd::AccessUnit;

    type Opens = fn(&[u8]) -> bool;
    let opens: [(&str, Opens); 4] = [
        ("truehd_unit", |b| AccessUnit::parse(b, None).is_ok()),
        ("truehd_decode", |b| {
            b.len() > 1 && AccessUnit::parse(&b[1..], None).is_ok()
        }),
        ("eac3_frame", |b| FrameHeader::parse(b).is_ok()),
        ("eac3_program", |b| FrameHeader::parse(b).is_ok()),
    ];
    for (name, open) in opens {
        let seeds = seeds().of(name);
        let rounds = 400;
        let opened = (0..rounds)
            .filter(|&round| open(&input(name, seeds, 0, round).1))
            .count();
        assert!(
            opened * 2 >= rounds as usize,
            "{name}: {opened} of {rounds} mutated inputs still open"
        );
    }
}

/// And a round that hands the target its seed back has tested nothing.
#[test]
fn nearly_every_round_is_a_seed_changed() {
    for (name, _) in TARGETS {
        let seeds = seeds().of(name);
        let rounds = 300;
        let changed = (0..rounds)
            .filter(|&round| {
                let (from, bytes) = input(name, seeds, 0, round);
                bytes != seeds[from]
            })
            .count();
        assert!(
            changed * 10 >= rounds as usize * 9,
            "{name}: {changed} of {rounds} rounds differ from their seed"
        );
    }
}

#[test]
fn a_round_is_the_same_bytes_every_time_and_another_seed_gives_others() {
    let seeds = seeds().of("truehd_unit");
    assert_eq!(
        input("truehd_unit", seeds, 0, 17),
        input("truehd_unit", seeds, 0, 17)
    );
    let differing = (0..50)
        .filter(|&round| {
            input("truehd_unit", seeds, 0, round) != input("truehd_unit", seeds, 1, round)
        })
        .count();
    assert!(
        differing >= 45,
        "{differing} of 50 rounds differ between two seeds"
    );
}
