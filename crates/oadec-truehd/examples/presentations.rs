//! Every presentation of a TrueHD stream, in one pass or one decoder each.
//!
//! ```text
//! cargo run --release -p oadec-truehd --example presentations -- one-pass <stream> [repeats]
//! cargo run --release -p oadec-truehd --example presentations -- separate <stream> [repeats]
//! ```
//!
//! `one-pass` decodes every presentation the stream starts with through one
//! `MultiDecoder`; `separate` through one `Decoder` each, one after the
//! other, as a caller without the one-pass decoder does. The stream is read
//! into memory and decoded `repeats` times over (1 unless given), each time
//! from its first unit with new decoders. What is printed: the wall-clock
//! time of the decoding, the samples made (so that nothing is optimised
//! away), and how often each substream was decoded against how many access
//! units it was needed in. CPU time and peak memory are the process's, for
//! whatever runs this to read from the operating system.

use std::process::ExitCode;
use std::time::Instant;

use oadec_truehd::{AccessUnit, Decoder, Extractor, MultiDecoder, PresentationOut, StreamConfig};

fn units(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut extractor = Extractor::new();
    extractor.push(bytes);
    let mut out = Vec::new();
    while let Some(unit) = extractor.next_unit().map_err(|e| e.to_string())? {
        out.push(unit.bytes.to_vec());
    }
    let (rest, _) = extractor.finish().map_err(|e| e.to_string())?;
    out.extend(rest.into_iter().map(|unit| unit.bytes.to_vec()));
    Ok(out)
}

/// The substreams presentation `p` reads under `config`, as a mask.
fn mask(config: &StreamConfig, p: usize) -> u8 {
    let map = &config.presentations;
    let top = match map.kind(p) {
        oadec_truehd::PresentationKind::CopyOf(i) => i,
        _ => p,
    };
    map.mask(top) & ((2u8 << top) - 1)
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, path) = match (args.first(), args.get(1)) {
        (Some(mode), Some(path)) => (mode.as_str(), path),
        _ => return Err("usage: presentations one-pass|separate <stream> [repeats]".into()),
    };
    let repeats: u64 = match args.get(2) {
        Some(n) => n.parse().map_err(|e| format!("repeats: {e}"))?,
        None => 1,
    };
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let units = units(&bytes)?;
    let (first, config) = AccessUnit::parse(&units[0], None).map_err(|e| e.to_string())?;
    let sync = first
        .major_sync
        .ok_or("the first unit carries no major sync")?;
    let presentations: Vec<usize> = (0..4).filter(|&p| config.has_presentation(p)).collect();

    // the access units each substream is needed in, by any presentation and
    // by each in turn
    let mut needed = [0u64; 4];
    let mut needed_each = [0u64; 4];
    for s in 0..4 {
        let union = presentations.iter().fold(0u8, |m, &p| m | mask(&config, p));
        needed[s] = u64::from((union >> s) & 1) * units.len() as u64 * repeats;
        needed_each[s] = presentations
            .iter()
            .map(|&p| u64::from((mask(&config, p) >> s) & 1))
            .sum::<u64>()
            * units.len() as u64
            * repeats;
    }

    let start = Instant::now();
    let mut samples = 0u64;
    let decodes = match mode {
        "one-pass" => {
            let mut decodes = [0u64; 4];
            for _ in 0..repeats {
                let mut decoder =
                    MultiDecoder::new(&sync, &presentations).map_err(|e| e.to_string())?;
                for unit in &units {
                    let decoded = decoder.decode_concealing(unit).map_err(|e| e.to_string())?;
                    for &p in &presentations {
                        if let Some(PresentationOut::Available { decoded, .. }) =
                            decoded.presentation(p)
                        {
                            samples += (decoded.pcm.len() * decoded.channels) as u64;
                        }
                    }
                }
                for (total, n) in decodes.iter_mut().zip(decoder.substream_decodes()) {
                    *total += n;
                }
            }
            decodes
        }
        "separate" => {
            for _ in 0..repeats {
                for &p in &presentations {
                    let mut decoder = Decoder::new(&sync, p).map_err(|e| e.to_string())?;
                    for unit in &units {
                        let (decoded, _) =
                            decoder.decode_concealing(unit).map_err(|e| e.to_string())?;
                        samples += (decoded.pcm.len() * decoded.channels) as u64;
                    }
                }
            }
            needed_each
        }
        other => return Err(format!("no mode {other}")),
    };
    let wall = start.elapsed();
    println!("mode {mode}");
    println!("presentations {presentations:?}");
    println!("units {}", units.len() as u64 * repeats);
    println!("wall_seconds {:.6}", wall.as_secs_f64());
    println!("samples {samples}");
    for s in 0..4 {
        println!(
            "substream {s}: decoded {} needed {} more than once {}",
            decodes[s],
            needed[s],
            decodes[s].saturating_sub(needed[s])
        );
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("presentations: {e}");
            ExitCode::FAILURE
        }
    }
}
