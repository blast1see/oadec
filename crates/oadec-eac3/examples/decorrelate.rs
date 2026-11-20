//! Decodes the independent substream 0 of an E-AC-3 or AC-3 stream to a
//! 32-bit float WAVE file, with the experimental decorrelation of the coupled
//! channels set as the arguments say (`oadec_eac3::decorr`,
//! `docs/eac3.md`), with the scope given by hand. The command line has the
//! mode as `oadec decode --decorrelate`, which takes the scope from the
//! programme; of a stream whose programme is its independent substream
//! alone it writes what this writes with `experimental core-only`. The
//! decorrelation is this decoder's estimate and a decode with it is not
//! claimed to equal Dolby's.
//!
//! ```text
//! cargo run --release -p oadec-eac3 --example decorrelate -- \
//!     <stream> <out.wav> [off|experimental] [unknown|core-only|with-dependents]
//! ```
//!
//! The mode is `off` and the scope `unknown` when they are not given. The
//! channels are written in WAVE order (L R C LFE Ls Rs), the dither is on,
//! and the first block's half window is kept, as `oadec decode` does. The
//! decorrelation status after the last frame is printed to standard error.

use std::io::Write;
use std::process::ExitCode;

use oadec_eac3::{
    Decoded, Decoder, Decorrelation, FrameHeader, Options, ProgrammeScope, StreamType, find_sync,
};

/// The place of a channel in WAVE order.
fn wave_rank(name: &str) -> usize {
    ["L", "R", "C", "LFE", "Ls", "Rs", "S", "Ch1", "Ch2"]
        .iter()
        .position(|n| *n == name)
        .unwrap_or(usize::MAX)
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: decorrelate <stream> <out.wav> [off|experimental] [unknown|core-only|with-dependents]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || args.len() > 4 {
        return usage();
    }
    let mode = match args.get(2).map(String::as_str) {
        None | Some("off") => Decorrelation::Off,
        Some("experimental") => Decorrelation::Experimental,
        Some(_) => return usage(),
    };
    let scope = match args.get(3).map(String::as_str) {
        None | Some("unknown") => ProgrammeScope::Unknown,
        Some("core-only") => ProgrammeScope::CoreOnly,
        Some("with-dependents") => ProgrammeScope::WithDependents,
        Some(_) => return usage(),
    };
    match run(&args[0], &args[1], mode, scope) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("decorrelate: {why}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    input: &str,
    output: &str,
    mode: Decorrelation,
    scope: ProgrammeScope,
) -> Result<(), String> {
    let stream = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let mut dec = Decoder::new(Options::default());
    dec.set_decorrelation(mode, scope);
    let mut order: Option<Vec<usize>> = None;
    let mut interleaved: Vec<f32> = Vec::new();
    let mut keep = |d: Decoded, order: &mut Option<Vec<usize>>| -> Result<(), String> {
        let names = Decoder::channel_names(&d.header);
        let want = order.get_or_insert_with(|| {
            let mut o: Vec<usize> = (0..names.len()).collect();
            o.sort_by_key(|&i| wave_rank(names[i]));
            o
        });
        if want.len() != d.pcm.len() {
            return Err("the channels of the stream change partway".into());
        }
        let len = d.pcm.first().map_or(0, Vec::len);
        for n in 0..len {
            for &ch in want.iter() {
                interleaved.push(d.pcm[ch][n]);
            }
        }
        Ok(())
    };
    let mut at = 0;
    while let Some(from) = find_sync(&stream, at) {
        let Ok(header) = FrameHeader::parse(&stream[from..]) else {
            at = from + 1;
            continue;
        };
        let end = from + header.frame_bytes;
        if header.frame_bytes == 0 || end > stream.len() {
            break;
        }
        at = end;
        if header.stream_type == StreamType::Dependent || header.substream_id != 0 {
            continue;
        }
        if let Some(d) = dec.decode(&stream[from..end]).map_err(|e| e.to_string())? {
            keep(d, &mut order)?;
        }
    }
    while let Some(d) = dec.flush().map_err(|e| e.to_string())? {
        keep(d, &mut order)?;
    }
    let channels = order.map_or(0, |o| o.len());
    if channels == 0 {
        return Err("no frame of the independent substream 0".into());
    }
    write_wave(output, channels, &interleaved)?;
    eprintln!("decorrelation: {}", dec.decorrelation_status());
    Ok(())
}

/// A WAVE file of 32-bit float samples at 48 kHz.
fn write_wave(path: &str, channels: usize, samples: &[f32]) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("{path}: {e}");
    let channels = u16::try_from(channels).map_err(|_| "too many channels".to_string())?;
    let body = u32::try_from(samples.len() * 4).map_err(|_| "too long for WAVE".to_string())?;
    let rate = 48_000u32;
    let align = channels * 4;
    let mut out = Vec::with_capacity(samples.len() * 4 + 44);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + body).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    out.extend_from_slice(&align.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&body.to_le_bytes());
    for v in samples {
        out.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::File::create(path)
        .and_then(|mut f| f.write_all(&out))
        .map_err(fail)
}
