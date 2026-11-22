//! Writes what the JOC object path holds, time slot by time slot, for the
//! measuring tools (`tools/joc_slot_error.py`, `tools/joc_qmf_reference.py`).
//!
//! ```text
//! cargo run --release -p oadec-engine --example joc_tap -- <stream.ec3> <dir> [--reset-banks] [--no-dither]
//! ```
//!
//! `<dir>` must not hold files. It gets little-endian f64 planes and an
//! `index.json` that says what each holds and how it is shaped:
//!
//! - `downmix.f64`: `[slot][ch][sb][re, im]`, after the analysis bank and the
//!   quadrature: what the matrix of the slot is applied to;
//! - `matrix.f64`: `[slot][obj][ch][sb]`, the matrix each object meets;
//! - `objects.f64`: `[slot][obj][sb][re, im]`, before the synthesis bank;
//! - `pcm_objects.f64`: `[sample][obj]`, the objects as `joc_pcm` gives them,
//!   aligned with the core;
//! - `pcm_core.f64`: `[sample][ch]`, the core decode of every programme
//!   channel without the clip gain, the decoder delay dropped;
//! - `slots` in `index.json`: frame, slot, sequence counter, reset, flush and
//!   the two clip gains of every slot.
//!
//! Slots run from the first slot of the first frame; an object sample `i` is
//! synthesis output sample `i + object_delay`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use oadec_engine::damf::Options;
use oadec_engine::eac3_objects::{TapOptions, tap_to_dir};
use oadec_engine::source::Source;

fn run(stream: &Path, dir: &Path, measure: TapOptions, dither: bool) -> anyhow::Result<()> {
    let mut opts = Options::default();
    opts.core.dither = dither;
    tap_to_dir(&Source::file(stream), dir, &opts, measure)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| {
        let had = args.iter().any(|a| a == name);
        args.retain(|a| a != name);
        had
    };
    let measure = TapOptions {
        reset_banks: flag("--reset-banks"),
    };
    let dither = !flag("--no-dither");
    let [stream, dir] = args.as_slice() else {
        eprintln!("usage: joc_tap <stream.ec3> <dir> [--reset-banks] [--no-dither]");
        return ExitCode::from(2);
    };
    match run(&PathBuf::from(stream), &PathBuf::from(dir), measure, dither) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("joc_tap: {e:#}");
            ExitCode::FAILURE
        }
    }
}
