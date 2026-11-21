//! The enhanced coupling model (`oadec_eac3::ecpl::Model`): off, it is the
//! decode as it is without it, byte for byte; on, it changes the coupled
//! channels over the enhanced coupling region and nothing else.
//!
//! Round 5 (P2) has only the hook: `Model::Off` is the one model, and the
//! tests that need a model, and the stream with enhanced coupling they read,
//! are ignored until there is one (`tools/ecpl_promotion.py`, `docs/audit/evidence/round-5-2026-11/p2-ecpl.json`).
//! They fail when run with `--ignored`, and say why.

use std::path::Path;

use oadec_eac3::ecpl::Model;
use oadec_eac3::{Decoder, FrameHeader, Options, ProgramDecoder};

/// The stereo fixture with its standard coupling rewritten as enhanced
/// coupling, one amplitude code a frame:
/// `oadec eac3-ecpl-inject eac3-20-coupled.ec3 -o ecpl-20-amp-sweep.ec3
/// --plan P`, with P the plan `amp sweep-frames / angle 0 / chaos 0 /
/// interp 0 / trans never` of the measurement against Dolby's decoder.
const ECPL_FIXTURE: &str = "ecpl-20-amp-sweep.ec3";

/// FNV-1a (64 bits) of the fixture's bytes as written by the injector of
/// the round-5 base, and of its default decode there (each coded channel
/// whole, one after the other, little-endian `f32`: [`bytes`]). A
/// hand-written hash, so the test needs no crate.
const ECPL_FIXTURE_FNV: u64 = 0x777c_055d_311d_b85c;
const ECPL_DEFAULT_DECODE_FNV: u64 = 0x697b_7fda_2252_4d29;

fn fixture(name: &str) -> Option<Vec<u8>> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .ok()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The syncframes of a stream that is nothing but whole syncframes.
fn frames(stream: &[u8]) -> Vec<(Vec<u8>, FrameHeader)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a whole stream of frames");
        out.push((stream[at..at + header.frame_bytes].to_vec(), header.clone()));
        at += header.frame_bytes;
    }
    out
}

/// Every coded channel a decoder told `model` makes of `stream`, whole.
/// `None` leaves the decoder as made.
fn decode(stream: &[u8], model: Option<Model>) -> Vec<Vec<f32>> {
    let mut dec = Decoder::new(Options::default());
    if let Some(m) = model {
        dec.set_ecpl_model(m);
    }
    let mut out: Vec<Vec<f32>> = Vec::new();
    let mut keep = |pcm: &[Vec<f32>]| {
        out.resize_with(out.len().max(pcm.len()), Vec::new);
        for (all, ch) in out.iter_mut().zip(pcm) {
            all.extend_from_slice(ch);
        }
    };
    for (bytes, _) in frames(stream) {
        if let Some(d) = dec.decode(&bytes).expect("a clean stream") {
            keep(&d.pcm);
        }
    }
    while let Some(d) = dec.flush().expect("a clean stream") {
        keep(&d.pcm);
    }
    out
}

/// The channels one after the other, little-endian `f32`.
fn bytes(channels: &[Vec<f32>]) -> Vec<u8> {
    channels
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

fn decode_bytes(stream: &[u8], model: Option<Model>) -> Vec<u8> {
    bytes(&decode(stream, model))
}

/// The same through the programme decoder, every group's channels in turn.
fn programme_bytes(stream: &[u8], dec: ProgramDecoder) -> Vec<u8> {
    let mut dec = dec;
    let mut out = Vec::new();
    let take = |dec: &mut ProgramDecoder, out: &mut Vec<u8>| {
        while let Some(group) = dec.pop() {
            for ch in 0..group.channels() {
                for v in group.channel(ch) {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    };
    for (bytes, header) in frames(stream) {
        dec.push(&bytes, &header).expect("a clean stream");
        take(&mut dec, &mut out);
    }
    dec.finish().expect("a clean stream");
    take(&mut dec, &mut out);
    out
}

#[test]
fn the_models_are_named_and_off_is_the_default() {
    assert_eq!(Model::default(), Model::Off);
    assert_eq!(Model::ALL.first(), Some(&Model::Off));
    for m in Model::ALL {
        assert_eq!(Model::from_name(m.name()), Some(*m));
    }
    assert_eq!(Model::from_name("Off"), None, "names are exact");
    assert_eq!(Model::from_name(""), None);
    assert_eq!(Decoder::default().ecpl_model(), Model::Off);
    assert_eq!(
        ProgramDecoder::new(Options::default()).ecpl_model(),
        Model::Off
    );
}

/// With the model off, the decode is the decode without the call: for the
/// decoder and for the programme decoder, on a stream with standard
/// coupling, which no model may touch.
#[test]
fn with_the_model_off_the_decode_is_unchanged() {
    let stream = fixture("eac3-20-coupled.ec3").expect("the stereo fixture");
    let plain = decode_bytes(&stream, None);
    assert!(!plain.is_empty());
    assert_eq!(plain, decode_bytes(&stream, Some(Model::Off)));
    let plain = programme_bytes(&stream, ProgramDecoder::new(Options::default()));
    assert_eq!(
        plain,
        programme_bytes(
            &stream,
            ProgramDecoder::new(Options::default()).with_ecpl_model(Model::Off)
        )
    );
}

/// The model off on a stream with enhanced coupling: the bytes of the
/// round-5 base, and the same with every model on a stream without it.
#[test]
#[ignore = "r5 durak 2: the fixture ecpl-20-amp-sweep.ec3 is not in tests/fixtures (the root .gitignore takes *.ec3); oadec-cli ecpl_inject tests hold the same hashes on a stream made in the test"]
fn with_the_model_off_enhanced_coupling_decodes_as_at_the_base() {
    let stream = fixture(ECPL_FIXTURE)
        .unwrap_or_else(|| panic!("r5 durak 2: tests/fixtures/{ECPL_FIXTURE} is not there yet"));
    assert_eq!(
        fnv1a(&stream),
        ECPL_FIXTURE_FNV,
        "the fixture is not the one written at the base"
    );
    let plain = decode_bytes(&stream, None);
    assert_eq!(
        fnv1a(&plain),
        ECPL_DEFAULT_DECODE_FNV,
        "the default decode moved"
    );
    assert_eq!(plain, decode_bytes(&stream, Some(Model::Off)));

    let standard = fixture("eac3-20-coupled.ec3").expect("the stereo fixture");
    let plain = decode_bytes(&standard, None);
    for m in Model::ALL {
        assert_eq!(
            plain,
            decode_bytes(&standard, Some(*m)),
            "{} touched standard coupling",
            m.name()
        );
    }
}

/// A model on: the coupled channels change over the enhanced coupling
/// region, and below it they do not. Judged on the short-time spectrum of the
/// difference, with a guard of 20 bins of 31,25 Hz under the region for the
/// leakage of the transform's window.
#[test]
#[ignore = "r5 durak 2: no model besides off has passed the promotion criterion yet"]
fn a_model_changes_the_enhanced_coupling_region_and_nothing_below_it() {
    let model = Model::ALL
        .iter()
        .copied()
        .find(|m| *m != Model::Off)
        .expect("r5 durak 2: ecpl::Model has no model besides off yet");
    let stream = fixture(ECPL_FIXTURE)
        .unwrap_or_else(|| panic!("r5 durak 2: tests/fixtures/{ECPL_FIXTURE} is not there yet"));
    let plain = decode(&stream, None);
    let with = decode(&stream, Some(model));
    assert_eq!(plain.len(), with.len());
    assert_ne!(plain, with, "{} changed nothing", model.name());

    let first_bin = region_start(&stream);
    let (mut low, mut high) = (0.0, 0.0);
    for (a, b) in plain.iter().zip(&with) {
        let (l, h) = split_energy(a, b, first_bin);
        low += l;
        high += h;
    }
    assert!(high > 0.0);
    assert!(
        low <= high * 1e-6,
        "{}: {:.1} dB of the change is below the region",
        model.name(),
        10.0 * (low / high).log10()
    );
}

/// The first coupled bin of the stream's first block with enhanced coupling.
fn region_start(stream: &[u8]) -> usize {
    let mut noise = oadec_eac3::Noise::default();
    for (bytes, _) in frames(stream) {
        let frame = oadec_eac3::Frame::parse(
            &bytes,
            &mut noise,
            Options {
                samples: false,
                ..Options::default()
            },
        )
        .expect("a clean frame");
        if let Some(c) = frame
            .blocks
            .iter()
            .find_map(|b| b.coupling.filter(|c| c.enhanced))
        {
            return c.start;
        }
    }
    panic!("no enhanced coupling in the fixture");
}

/// The energy of `b - a`, one channel, below and above `first_bin` (of the
/// 256 bins a block has), with a guard of 20 DFT bins under it, over
/// 1536-sample Hann-windowed segments. A plain DFT: the test is not run
/// often and needs no crate.
fn split_energy(a: &[f32], b: &[f32], first_bin: usize) -> (f64, f64) {
    const SEG: usize = 1536;
    let d: Vec<f64> = b
        .iter()
        .zip(a)
        .map(|(y, x)| f64::from(*y) - f64::from(*x))
        .collect();
    // block bin k is 93,75 Hz wide, a DFT bin of 1536 points 31,25 Hz
    let edge = (first_bin * 3).saturating_sub(20);
    let (mut low, mut high) = (0.0, 0.0);
    for seg in d.as_chunks::<SEG>().0 {
        for k in 0..=SEG / 2 {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, v) in seg.iter().enumerate() {
                let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / SEG as f64).cos();
                let ph = 2.0 * std::f64::consts::PI * (k * n) as f64 / SEG as f64;
                re += v * w * ph.cos();
                im -= v * w * ph.sin();
            }
            let e = re * re + im * im;
            if k < edge {
                low += e;
            } else {
                high += e;
            }
        }
    }
    (low, high)
}
