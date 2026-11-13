//! The decoder's kernels over the committed fixtures.
//!
//! ```text
//! cargo bench -p oadec-bench
//! cargo bench -p oadec-bench -- --filter truehd --json kernels.json
//! ```
//!
//! Every kernel here is reached through the public interface of a decoder
//! crate, with the stream already in memory, so the figures are of decoding
//! alone: no file is read and nothing is written.

use std::hint::black_box;
use std::path::Path;

use oadec_bench::{Sample, json, line, measure};
use oadec_bits::{CRC8_SUBSTREAM, CRC8_SUBSTREAM_INIT, xor_bytes};
use oadec_eac3::{FrameHeader, Options};
use oadec_joc::{Analysis, BANDS, Complex, Synthesis};
use oadec_truehd::{AccessUnit, Decoder, Extractor, Unit};

fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oadec-cli/tests/fixtures")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The access units of a TrueHD or MLP stream.
fn units(bytes: &[u8]) -> Vec<Unit> {
    let mut extractor = Extractor::new();
    extractor.push(bytes);
    let mut out = Vec::new();
    while let Some(unit) = extractor.next_unit().expect("the fixture frames") {
        out.push(unit);
    }
    out.extend(extractor.finish().expect("the fixture ends on a unit").0);
    out
}

/// Decodes `units` at `presentation` and returns the samples it made.
fn truehd(units: &[Unit], presentation: usize) -> usize {
    let (first, _) = AccessUnit::parse(&units[0].bytes, None).expect("the first unit");
    let sync = first.major_sync.as_ref().expect("a major sync");
    let mut decoder = Decoder::new(sync, presentation).expect("the presentation");
    let mut samples = 0;
    for unit in units {
        let decoded = decoder.decode(&unit.bytes).expect("the unit decodes");
        samples += black_box(decoded.pcm).len();
    }
    samples
}

/// How long the audio of `units` is, and how many channels `presentation` has.
fn truehd_facts(units: &[Unit], presentation: usize) -> (f64, usize) {
    let (first, _) = AccessUnit::parse(&units[0].bytes, None).expect("the first unit");
    let sync = first.major_sync.as_ref().expect("a major sync");
    let mut decoder = Decoder::new(sync, presentation).expect("the presentation");
    // the channel count is the restart header's, so one unit has to be read
    let channels = decoder
        .decode(&units[0].bytes)
        .expect("the first unit decodes")
        .channels;
    let config = decoder.config();
    let seconds = units.len() as f64 * f64::from(config.samples_per_au)
        / f64::from(config.sampling_frequency);
    (seconds, channels)
}

/// The syncframes of an AC-3 or E-AC-3 stream.
fn frames(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let header = FrameHeader::parse(&bytes[at..]).expect("a syncframe");
        out.push(&bytes[at..at + header.frame_bytes]);
        at += header.frame_bytes;
    }
    out
}

/// Decodes `frames` as one substream and returns the samples it made.
fn eac3(frames: &[&[u8]]) -> usize {
    let mut decoder = oadec_eac3::Decoder::new(Options::default());
    let mut samples = 0;
    for frame in frames {
        if let Some(decoded) = decoder.decode(frame).expect("the frame decodes") {
            samples += black_box(&decoded.pcm)[0].len();
        }
    }
    while let Some(decoded) = decoder.flush().expect("the decoder drains") {
        samples += black_box(&decoded.pcm)[0].len();
    }
    samples
}

/// `len` bytes that are no stream, the same on every run.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u8
        })
        .collect()
}

/// One frame of time slots for a filter bank: a tone and a little noise, the
/// same on every run.
fn slots() -> Vec<[f64; BANDS]> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..24)
        .map(|slot| {
            core::array::from_fn(|i| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let noise = (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5;
                let t = (slot * BANDS + i) as f64 / 48_000.0;
                0.25 * (2.0 * core::f64::consts::PI * 440.0 * t).sin() + 0.01 * noise
            })
        })
        .collect()
}

fn main() {
    let mut filter: Option<String> = None;
    let mut out: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--filter" => filter = args.next(),
            "--json" => out = args.next(),
            // cargo passes `--bench`; anything else is not this program's
            _ => {}
        }
    }
    let wanted = |name: &str| filter.as_deref().is_none_or(|f| name.contains(f));
    let mut samples: Vec<Sample> = Vec::new();
    let mut run = |name: String, seconds: Option<f64>, kernel: &mut dyn FnMut() -> usize| {
        if wanted(&name) {
            let sample = measure(&name, seconds, kernel);
            println!("{}", line(&sample, 52));
            samples.push(sample);
        }
    };

    let scene = fixture("authored-scene.mlp");
    run(
        "truehd: frame the access units".to_string(),
        None,
        &mut || units(&scene).len(),
    );
    let scene_units = units(&scene);
    for presentation in 0..4 {
        let (seconds, channels) = truehd_facts(&scene_units, presentation);
        run(
            format!("truehd: decode presentation {presentation}, {channels} channels"),
            Some(seconds),
            &mut || truehd(&scene_units, presentation),
        );
    }
    let high_rate = units(&fixture("truehd-51-88k.thd"));
    let (seconds, channels) = truehd_facts(&high_rate, 1);
    run(
        format!("truehd: decode 88.2 kHz, {channels} channels"),
        Some(seconds),
        &mut || truehd(&high_rate, 1),
    );
    let fbb = units(&fixture("mlp-51-96k.mlp"));
    let (seconds, channels) = truehd_facts(&fbb, 0);
    run(
        format!("mlp: decode 96 kHz, {channels} channels"),
        Some(seconds),
        &mut || truehd(&fbb, 0),
    );

    let joc = fixture("authored-scene.ec3");
    let joc_frames = frames(&joc);
    let seconds = joc_frames.len() as f64 * 1536.0 / 48_000.0;
    run(
        "eac3: decode the 5.1 core of the object stream".to_string(),
        Some(seconds),
        &mut || eac3(&joc_frames),
    );

    let input = slots();
    let seconds = input.len() as f64 * BANDS as f64 / 48_000.0;
    let mut analysis = Analysis::new();
    let mut bands = [Complex::default(); BANDS];
    run(
        "joc: analysis bank, one channel, one frame".to_string(),
        Some(seconds),
        &mut || {
            for slot in &input {
                analysis.step(slot, &mut bands);
            }
            black_box(&bands).len()
        },
    );
    let mut analysis = Analysis::new();
    let subbands: Vec<[Complex; BANDS]> = input
        .iter()
        .map(|slot| {
            let mut out = [Complex::default(); BANDS];
            analysis.step(slot, &mut out);
            out
        })
        .collect();
    let mut synthesis = Synthesis::new();
    let mut pcm = [0.0f64; BANDS];
    run(
        "joc: synthesis bank, one object, one frame".to_string(),
        Some(seconds),
        &mut || {
            for slot in &subbands {
                synthesis.step(slot, &mut pcm);
            }
            black_box(&pcm).len()
        },
    );

    // What a byte costs the checks every frame and every segment makes of
    // itself: the CRC-16 of the AC-3 family, the CRC-8 and the parity of a
    // TrueHD substream segment, each over a megabyte.
    let megabyte = noise(1 << 20);
    run(
        "checksum: CRC-16 of a megabyte".to_string(),
        None,
        &mut || usize::from(oadec_eac3::frame::CRC16.update_bytes(0, black_box(&megabyte))),
    );
    run(
        "checksum: CRC-8 of a megabyte".to_string(),
        None,
        &mut || usize::from(CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, black_box(&megabyte))),
    );
    run(
        "checksum: parity of a megabyte".to_string(),
        None,
        &mut || usize::from(xor_bytes(black_box(&megabyte))),
    );

    if let Some(path) = out {
        std::fs::write(&path, json(&samples)).unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}
