//! Decodes the independent substream 0 of an E-AC-3 stream to raw 32-bit
//! float PCM with an enhanced coupling model laid over the amplitude-only
//! reading (`oadec_eac3::ecpl::Model`): the decode the measurements of
//! `tools/ecpl_amp_sweep.py` and `tools/ecpl_promotion.py` set against
//! Dolby's, before any model has a command-line flag.
//!
//! ```text
//! cargo run --release -p oadec-eac3 --example ecpl_model_decode -- \
//!     <stream> <out.f32> [model]
//! ```
//!
//! The model is `off` when it is not given, and a name the crate does not
//! know is refused with the names it does (exit 2). The channels are
//! interleaved in WAVE order (L R C LFE Ls Rs for 5.1, L R for 2/0), the
//! dither is on, and nothing is trimmed, as `oadec decode --format pcm`
//! writes them; with the model `off` the two are expected to give the same
//! bytes, and the measurement checks it every time rather than trust it.

use std::io::Write;
use std::process::ExitCode;

use oadec_eac3::ecpl::Model;
use oadec_eac3::{Decoded, Decoder, FrameHeader, Options, StreamType, find_sync};

/// The place of a channel in WAVE order.
fn wave_rank(name: &str) -> usize {
    ["L", "R", "C", "LFE", "Ls", "Rs", "S", "Ch1", "Ch2"]
        .iter()
        .position(|n| *n == name)
        .unwrap_or(usize::MAX)
}

fn names() -> String {
    Model::ALL
        .iter()
        .map(|m| m.name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || args.len() > 3 {
        eprintln!(
            "usage: ecpl_model_decode <stream> <out.f32> [model]; models: {}",
            names()
        );
        return ExitCode::from(2);
    }
    let model = match args.get(2) {
        None => Model::Off,
        Some(name) => match Model::from_name(name) {
            Some(m) => m,
            None => {
                eprintln!(
                    "ecpl_model_decode: no model '{name}'; the models are: {}",
                    names()
                );
                return ExitCode::from(2);
            }
        },
    };
    match run(&args[0], &args[1], model) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("ecpl_model_decode: {why}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: &str, output: &str, model: Model) -> Result<(), String> {
    let stream = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let mut dec = Decoder::new(Options::default());
    dec.set_ecpl_model(model);
    let mut order: Option<Vec<usize>> = None;
    let mut out: Vec<u8> = Vec::new();
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
                out.extend_from_slice(&d.pcm[ch][n].to_le_bytes());
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
    if order.is_none() {
        return Err("no frame of the independent substream 0".into());
    }
    std::fs::File::create(output)
        .and_then(|mut f| f.write_all(&out))
        .map_err(|e| format!("{output}: {e}"))
}
