//! Writes what the JOC object path holds, time slot by time slot, for the
//! measuring tools (`tools/joc_slot_error.py`, `tools/joc_qmf_reference.py`).
//!
//! ```text
//! cargo run --release -p oadec-engine --example joc_tap -- <stream.ec3> <dir>
//! ```
//!
//! `<dir>` gets one file of little-endian f64 per stage and an `index.json`
//! that says what each holds:
//!
//! - `downmix.f64`: `[slot][ch][sb][re, im]`, after the analysis bank and the
//!   quadrature;
//! - `matrix.f64`: `[slot][obj][ch][sb]`, the matrix each object meets;
//! - `objects.f64`: `[slot][obj][sb][re, im]`, before the synthesis bank;
//! - `slots.json` lines in `index.json`: frame, slot, sequence counter, reset,
//!   flush and the two clip gains of every slot.
//!
//! Round 5, stop 1: the command line is fixed and the decode is not wired; it
//! says so and exits 2.

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [stream, dir] = args.as_slice() else {
        eprintln!("usage: joc_tap <stream.ec3> <dir>");
        return ExitCode::from(2);
    };
    let (_stream, _dir) = (PathBuf::from(stream), PathBuf::from(dir));
    eprintln!("joc_tap: the tap is not wired yet (round 5, stop 2)");
    ExitCode::from(2)
}
