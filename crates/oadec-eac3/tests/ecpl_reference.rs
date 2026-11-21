//! The enhanced coupling synthesis of the full reading (`--ecpl-spec`)
//! held against `tools/ecpl_reference.py`, a reference written from the text
//! of ATSC A/52:2018 clause E.3.5.5, and not from this crate. ETSI TS 102 366
//! V1.2.1 clause E.2.5.5 prints the same process but for two signs; the
//! reference reads either (`text`), the decoder and these cases A/52:2018.
//!
//! Each case is a `.case` file under `tests/ecpl_reference/` (the format is
//! in the reference's own documentation, `make_cases.py` writes them) and
//! the reference's answer to it, `.ref32.bin`. The test builds every block
//! with `EcplBlock::from_syntax`, feeds the case's random values through
//! `Synth::with_random_values`, and wants every case within 100 dB of the
//! reference: the error energy over all the coefficients of the case at
//! least 100 dB below their energy.
//!
//! With `OADEC_ECPL_REF_F64` naming a directory of `<case>.ref64.bin` files
//! (the reference run with `--f64`), the same comparison runs against those,
//! which takes the f32 rounding of the files kept here out of the figure.
//!
//! The comparison says the synthesis does what the text says. What Dolby's
//! decoders do is measured apart (`docs/eac3.md`), and neither measurement
//! stands in for the other.

use std::path::{Path, PathBuf};

use oadec_eac3::ecpl::{RandomValues, Synth};
use oadec_eac3::frame::{EcplBlock, EcplChannel, MAX_FBW, N};
use oadec_eac3::imdct::Imdct;

/// The test matrix: every case the reference answers.
const CASES: &[&str] = &[
    // angle: codes 0..63 over the bands of four channels, flat and
    // interpolated, and neighbours across +/-pi (31/32, 63/0, 0/63)
    "angle-all-flat",
    "angle-all-interp",
    "angle-wrap-interp",
    // amplitude: codes 0..31, 31 adding nothing
    "amp-all",
    // chaos: codes 0..7 without and with a transient, and a channel whose
    // transient comes and goes from block to block
    "chaos-all-steady",
    "chaos-all-transient",
    "chaos-transient-switch",
    // bands: the most (22), merged by ecplbndstrc, the fewest (1) flat and
    // interpolated, the six-bin sub-bands below bin 37
    "bands-max",
    "bands-merged",
    "bands-min-flat",
    "bands-min-interp",
    "bands-narrow",
    // channels: the first coupled channel is not channel 0, and its codes
    // are not zero
    "first-channel-later",
    // time: enhanced coupling switches off and on (a block without it, as a
    // concealed frame gives), and the stream ends (no successor)
    "timeline-on-off",
    "timeline-eof",
];

/// What the test wants of every case.
const FLOOR_DB: f64 = 100.0;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/ecpl_reference")
}

/// The generator of the case format.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64 / 9_007_199_254_740_992.0) * 2.0 - 1.0
    }
}

/// One block of a case: `None` for a block without enhanced coupling.
type CaseBlock = Option<(usize, EcplBlock)>;

struct Case {
    gain: f64,
    window: String,
    random_seed: u64,
    blocks: Vec<CaseBlock>,
}

fn codes(v: &str) -> Vec<u8> {
    v.split(',').map(|c| c.parse().expect("a code")).collect()
}

fn parse(path: &Path) -> Case {
    let text = std::fs::read_to_string(path).expect("case file");
    let mut case = Case {
        gain: 2.0,
        window: "kbd5".into(),
        random_seed: 0,
        blocks: Vec::new(),
    };
    let mut mant: Option<(Lcg, f64)> = None;
    // the block being read: (nfchans, begin, end, bndstrc, interp, chans)
    type Open = (
        usize,
        usize,
        usize,
        [bool; 22],
        bool,
        Vec<Option<EcplChannel>>,
    );
    let mut open: Option<Open> = None;
    let close = |open: &mut Option<Open>, case: &mut Case, mant: &mut Option<(Lcg, f64)>| {
        if let Some((nf, begin, end, strc, interp, chans)) = open.take() {
            assert_eq!(chans.len(), nf, "{}: channel lines", path.display());
            let mut b = EcplBlock::from_syntax(begin, end, &strc, interp, chans, [0.0; N])
                .expect("a valid block");
            let (draw, scale) = mant.as_mut().expect("mant before the first block");
            for v in &mut b.coeffs[b.start..b.end] {
                *v = *scale * draw.next();
            }
            case.blocks.push(Some((nf, b)));
        }
    };
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some(&head) = words.first() else { continue };
        match head {
            "ecplref" => assert_eq!(words[1], "1"),
            "window" => case.window = words[1].into(),
            "ola-gain" => case.gain = words[1].parse().expect("gain"),
            "text" => assert_eq!(words[1], "a52-2018", "the decoder follows A/52:2018"),
            "random" => {
                assert_eq!(words[1], "lcg", "only lcg random values are kept here");
                case.random_seed = words[2].parse().expect("seed");
            }
            "mant" => {
                mant = Some((
                    Lcg(words[2].parse().expect("seed")),
                    words[3].parse().expect("scale"),
                ));
            }
            "keep" => panic!(
                "{}: the cases kept here compare every block",
                path.display()
            ),
            "block" => {
                close(&mut open, &mut case, &mut mant);
                if words[1] == "none" {
                    case.blocks.push(None);
                    continue;
                }
                let kv = |k: &str| {
                    words[1..]
                        .iter()
                        .find_map(|w| w.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
                        .unwrap_or_else(|| panic!("{k} missing"))
                };
                let mut strc = [false; 22];
                for (s, c) in strc.iter_mut().zip(kv("bndstrc").chars()) {
                    *s = c == '1';
                }
                open = Some((
                    kv("nfchans").parse().expect("nfchans"),
                    kv("begin").parse().expect("begin"),
                    kv("end").parse().expect("end"),
                    strc,
                    kv("interp") == "1",
                    Vec::new(),
                ));
            }
            "ch" => {
                let chans = &mut open.as_mut().expect("ch inside a block").5;
                assert_eq!(chans.len(), words[1].parse::<usize>().expect("ch"));
                if words[2] == "off" {
                    chans.push(None);
                    continue;
                }
                let kv = |k: &str| {
                    words[2..]
                        .iter()
                        .find_map(|w| w.strip_prefix(k).and_then(|r| r.strip_prefix('=')))
                        .unwrap_or_else(|| panic!("{k} missing"))
                };
                chans.push(Some(EcplChannel {
                    amp: codes(kv("amp")),
                    angle: codes(kv("angle")),
                    chaos: codes(kv("chaos")),
                    transient: kv("trans") == "1",
                }));
            }
            "end" => close(&mut open, &mut case, &mut mant),
            other => panic!("{}: unknown directive {other}", path.display()),
        }
    }
    assert!(open.is_none(), "{}: no `end`", path.display());
    case
}

/// The decoder's answer to a case, in the order the reference writes it.
fn decode(case: &Case) -> Vec<f64> {
    assert_eq!(case.window, "kbd5", "the decoder's window is the KBD one");
    assert_eq!(case.gain, 2.0, "the decoder applies the factor two");
    let mut draw = Lcg(case.random_seed);
    let mut notrans = vec![[0.0f64; N]; MAX_FBW];
    for row in &mut notrans {
        for v in row.iter_mut() {
            *v = draw.next();
        }
    }
    let draws: usize = case
        .blocks
        .iter()
        .flatten()
        .map(|(_, b)| {
            let coupled = b.chans.iter().filter(|c| c.is_some()).count();
            (coupled - 1) * b.bands.len()
        })
        .sum();
    let trans = (0..draws).map(|_| draw.next()).collect();
    let mut synth = Synth::with_random_values(RandomValues { notrans, trans });
    let imdct = Imdct::new();
    let mut out = Vec::new();
    for (i, blk) in case.blocks.iter().enumerate() {
        let Some((nf, b)) = blk else {
            synth.skip_block();
            continue;
        };
        let next = match case.blocks.get(i + 1) {
            Some(Some((_, n))) => n.coeffs,
            _ => [0.0; N],
        };
        let mut coeffs = vec![[0.0f64; N]; *nf];
        synth.block(&imdct, b, &next, &mut coeffs);
        for (ch, c) in b.chans.iter().enumerate() {
            if c.is_some() {
                out.extend_from_slice(&coeffs[ch][b.start..b.end]);
            }
        }
    }
    out
}

fn reference(path: &Path, f64s: bool) -> Vec<f64> {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    if f64s {
        let (words, _) = bytes.as_chunks::<8>();
        words.iter().map(|&w| f64::from_le_bytes(w)).collect()
    } else {
        let (words, _) = bytes.as_chunks::<4>();
        words
            .iter()
            .map(|&w| f64::from(f32::from_le_bytes(w)))
            .collect()
    }
}

/// How far below the reference's energy the difference lies, in dB.
fn below(got: &[f64], want: &[f64]) -> f64 {
    let signal: f64 = want.iter().map(|v| v * v).sum();
    let noise: f64 = got.iter().zip(want).map(|(a, b)| (a - b) * (a - b)).sum();
    if noise == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (signal / noise).log10()
    }
}

#[test]
fn every_case_of_the_matrix_has_its_reference_answer() {
    let mut missing = Vec::new();
    for name in CASES {
        for ext in ["case", "ref32.bin"] {
            let path = dir().join(format!("{name}.{ext}"));
            if !path.exists() {
                missing.push(path.display().to_string());
            }
        }
    }
    assert!(missing.is_empty(), "missing: {missing:#?}");
}

#[test]
fn the_full_reading_is_within_100_db_of_the_reference() {
    let full = std::env::var_os("OADEC_ECPL_REF_F64").map(PathBuf::from);
    let mut failed = Vec::new();
    for name in CASES {
        let case = parse(&dir().join(format!("{name}.case")));
        let got = decode(&case);
        let want = match &full {
            Some(d) => reference(&d.join(format!("{name}.ref64.bin")), true),
            None => reference(&dir().join(format!("{name}.ref32.bin")), false),
        };
        assert_eq!(got.len(), want.len(), "{name}: coefficient count");
        let signal: f64 = want.iter().map(|v| v * v).sum();
        assert!(signal > 0.0, "{name}: the reference is silent");
        let db = below(&got, &want);
        println!("{name:24} {db:8.1} dB  ({} coefficients)", want.len());
        if db < FLOOR_DB {
            failed.push(format!("{name}: {db:.1} dB"));
        }
    }
    assert!(failed.is_empty(), "below {FLOOR_DB} dB: {failed:#?}");
}
