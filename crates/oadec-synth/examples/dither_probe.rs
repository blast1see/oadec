//! Writes the stream a decoder's dither is read off: an AC-3 stream of two
//! channels in which no mantissa has a bit, every exponent is 8 and every
//! block asks for dither. What a decoder writes for it is its dither and
//! nothing else, and `tools/dither_probe.py` takes it from there.
//!
//!     cargo run --release -p oadec-synth --example dither_probe -- out.ac3 [frames [left [right [flags]]]]
//!
//! `frames` (40) of 768 bytes at 48 kHz; `left` and `right` (253): the
//! coefficients of each channel, 73 and three more for each step up to 253;
//! `flags` (11): which of the two channels ask for dither, left then right.

use std::process::ExitCode;

use oadec_synth::ac3::{Ac3Layout, write_without_a_bit};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let number = |at: usize, otherwise: usize| -> Option<usize> {
        args.get(at)
            .map_or(Some(otherwise), |text| text.parse().ok())
    };
    let flags = args.get(4).map_or("11", String::as_str);
    let (Some(out), Some(frames), Some(left), Some(right), true) = (
        args.first(),
        number(1, 40),
        number(2, 253),
        number(3, 253),
        flags.len() == 2 && flags.bytes().all(|b| b == b'0' || b == b'1'),
    ) else {
        eprintln!("usage: dither_probe <out.ac3> [frames [left [right [flags]]]]");
        return ExitCode::from(2);
    };
    let layout = Ac3Layout {
        dither: [flags.as_bytes()[0] == b'1', flags.as_bytes()[1] == b'1'],
        ..Ac3Layout::new(48_000, 20)
    };
    match write_without_a_bit(&layout, frames, [left, right], 8)
        .and_then(|stream| std::fs::write(out, &stream.bytes).map_err(|e| e.to_string()))
    {
        Ok(()) => {
            println!("{frames} frames, {left} and {right} coefficients, dither flags {flags}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}
