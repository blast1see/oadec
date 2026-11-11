//! What must hold of the parsers and decoders whatever bytes they are given.
//!
//! Each function of [`targets`] takes arbitrary bytes and asserts what is
//! true of every input: no panic, no samples or offsets beyond what the
//! format allows, the same answer by two routes that are meant to agree. A
//! function that returns has found nothing; one that panics has.
//!
//! The same functions run three ways:
//!
//! - `tests/seeded.rs`, the seeded fuzz run, runs each over [`seeds`] cut
//!   from the committed fixtures and over seeded changes to them ([`mutate`]).
//!   It needs neither media nor a nightly toolchain, so it runs with every
//!   `cargo test`, on every platform. `OADEC_FUZZ_ITERS` lengthens it.
//! - `fuzz/` wraps each in a libFuzzer target (`cargo +nightly fuzz run`),
//!   where the coverage feedback finds what a blind change does not. That
//!   needs a platform libFuzzer links on.
//! - A function is plain Rust, so a failing input is a unit test once it is
//!   written down.

pub mod mutate;
pub mod seeds;
pub mod targets;

/// A target by name.
pub type Target = (&'static str, fn(&[u8]));

/// Every target, by the name its libFuzzer wrapper and its seeds go by.
pub const TARGETS: [Target; 8] = [
    ("truehd_extract", targets::truehd_extract),
    ("truehd_unit", targets::truehd_unit),
    ("truehd_decode", targets::truehd_decode),
    ("emdf_container", targets::emdf_container),
    ("oamd_parse", targets::oamd_parse),
    ("joc_params", targets::joc_params),
    ("eac3_frame", targets::eac3_frame),
    ("eac3_program", targets::eac3_program),
];
