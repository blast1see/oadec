//! The experimental decorrelation of the coupled channels of an E-AC-3
//! stream (`oadec_eac3::decorr`, `docs/eac3.md`).
//!
//! What is held here is what the mode does and where it does nothing; not
//! that it equals Dolby's decode, which it is not claimed to.
//! `tools/coupling_mix.py measure` sets a decode against a reference decode.

use oadec_eac3::decorr::{ALLPASS, ANGLE_MAX, Decorrelator, Delayed, Role, roles};
use oadec_eac3::frame::N;
use oadec_eac3::{
    Coupling, Decoder, Decorrelation, DecorrelationStatus, Frame, FrameHeader, Noise, NotApplied,
    Options, ProgrammeScope, StreamType, Syntax,
};

const SCENE: &str = "authored-scene.ec3";
const STEREO: &str = "eac3-20-coupled.ec3";
const AC3: &str = "ac3-20-coupled.ac3";
const SEVEN_ONE: &str = "eac3-71-tones.ec3";

/// A fixture: the authored scene of the CLI's tests, or one of this crate's.
fn fixture(name: &str) -> Vec<u8> {
    let folder = if name == SCENE {
        "../oadec-cli/tests/fixtures"
    } else {
        "tests/fixtures"
    };
    let path = format!("{}/{folder}/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(path).expect("the fixture")
}

/// The syncframes of a stream with their headers.
fn frames(stream: &[u8]) -> Vec<(FrameHeader, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a syncframe");
        out.push((header.clone(), &stream[at..at + header.frame_bytes]));
        at += header.frame_bytes;
    }
    out
}

/// The syncframes of one substream.
fn substream<'a>(
    all: &[(FrameHeader, &'a [u8])],
    kind: StreamType,
    id: u8,
) -> Vec<(FrameHeader, &'a [u8])> {
    all.iter()
        .filter(|(h, _)| h.stream_type == kind && h.substream_id == id)
        .cloned()
        .collect()
}

/// The substreams a stream has, as `(type, id)`.
fn substreams(all: &[(FrameHeader, &[u8])]) -> Vec<(StreamType, u8)> {
    let mut out: Vec<(StreamType, u8)> = Vec::new();
    for (h, _) in all {
        if !out.contains(&(h.stream_type, h.substream_id)) {
            out.push((h.stream_type, h.substream_id));
        }
    }
    out
}

/// What a decode gives: the samples of every frame, by channel, as bits,
/// and the decorrelation status after every call that took a frame in.
struct Decode {
    pcm: Vec<Vec<Vec<u32>>>,
    status: Vec<DecorrelationStatus>,
}

/// Decodes `frames` from `first` on with a fresh decoder; `setup` is called
/// on it before the first frame, and `between(n, decoder)` before frame `n`.
/// A frame in `concealed` is concealed instead of decoded.
fn decode_with(
    frames: &[(FrameHeader, &[u8])],
    first: usize,
    setup: impl FnOnce(&mut Decoder),
    mut between: impl FnMut(usize, &mut Decoder),
    concealed: &[usize],
) -> Decode {
    let mut dec = Decoder::new(Options::default());
    setup(&mut dec);
    let mut out = Decode {
        pcm: Vec::new(),
        status: Vec::new(),
    };
    let keep = |d: oadec_eac3::Decoded, pcm: &mut Vec<Vec<Vec<u32>>>| {
        pcm.push(
            d.pcm
                .iter()
                .map(|ch| ch.iter().map(|v| v.to_bits()).collect())
                .collect(),
        );
    };
    for (n, (header, bytes)) in frames.iter().enumerate().skip(first) {
        between(n, &mut dec);
        let got = if concealed.contains(&n) {
            dec.conceal(bytes, header).expect("a concealed frame")
        } else {
            dec.decode(bytes).expect("a frame")
        };
        out.status.push(dec.decorrelation_status());
        if let Some(d) = got {
            keep(d, &mut out.pcm);
        }
    }
    while let Some(d) = dec.flush().expect("the end") {
        keep(d, &mut out.pcm);
    }
    out
}

fn decode(frames: &[(FrameHeader, &[u8])], setup: impl FnOnce(&mut Decoder)) -> Decode {
    decode_with(frames, 0, setup, |_, _| {}, &[])
}

fn plain(frames: &[(FrameHeader, &[u8])]) -> Decode {
    decode(frames, |_| {})
}

fn experimental(scope: ProgrammeScope) -> impl FnOnce(&mut Decoder) {
    move |d: &mut Decoder| d.set_decorrelation(Decorrelation::Experimental, scope)
}

/// The independent substream 0 of a fixture.
fn core(stream: &[u8]) -> Vec<(FrameHeader, &[u8])> {
    substream(&frames(stream), StreamType::Independent, 0)
}

/// The coefficients of every block of `frames` as the plain decode has them,
/// with each block's coupling.
fn blocks(frames: &[(FrameHeader, &[u8])]) -> Vec<(Vec<[f64; N]>, Option<Coupling>)> {
    let mut noise = Noise::default();
    let mut out = Vec::new();
    for (_, bytes) in frames {
        let frame = Frame::parse(bytes, &mut noise, Options::default()).expect("a frame");
        for block in frame.blocks {
            assert!(block.ecpl.is_none(), "the fixtures use standard coupling");
            out.push((block.coeffs, block.coupling));
        }
    }
    out
}

/// A block of every coded channel, after the decorrelator, and what it said.
type Said = (Vec<[f64; N]>, Result<(), NotApplied>);

/// The blocks of `frames` run through one decorrelator, as the decoder runs
/// them, with what it said of each.
fn decorrelated(frames: &[(FrameHeader, &[u8])]) -> Vec<Said> {
    let (acmod, lfeon) = (frames[0].0.acmod, frames[0].0.lfeon);
    let mut dec = Decorrelator::new();
    blocks(frames)
        .into_iter()
        .map(|(mut coeffs, coupling)| {
            let said = dec.block(acmod, lfeon, coupling.as_ref(), &mut coeffs);
            (coeffs, said)
        })
        .collect()
}

// ---------------------------------------------------------------------
// 1. Off is the plain decode, on every fixture of AC-3 and E-AC-3.

#[test]
fn off_is_the_plain_decode_of_every_fixture() {
    for name in [SCENE, STEREO, AC3, SEVEN_ONE] {
        let stream = fixture(name);
        let all = frames(&stream);
        for (kind, id) in substreams(&all) {
            let frames = substream(&all, kind, id);
            let reference = plain(&frames);
            assert!(!reference.pcm.is_empty());
            for scope in [
                ProgrammeScope::Unknown,
                ProgrammeScope::CoreOnly,
                ProgrammeScope::WithDependents,
            ] {
                let off = decode(&frames, |d| d.set_decorrelation(Decorrelation::Off, scope));
                assert!(
                    off.pcm == reference.pcm,
                    "{name} {kind:?} {id} {scope:?}: Off is not the plain decode"
                );
                assert!(
                    off.status
                        .iter()
                        .all(|s| *s == DecorrelationStatus::NotApplied(NotApplied::Off))
                );
            }
        }
    }
}

#[test]
fn a_new_decoder_is_off() {
    let dec = Decoder::new(Options::default());
    assert_eq!(
        dec.decorrelation_status(),
        DecorrelationStatus::NotApplied(NotApplied::Off)
    );
    assert_eq!(Decorrelation::default(), Decorrelation::Off);
    assert_eq!(ProgrammeScope::default(), ProgrammeScope::Unknown);
}

// ---------------------------------------------------------------------
// 2. AC-3 is never decorrelated.

#[test]
fn ac3_is_left_as_it_is() {
    let stream = fixture(AC3);
    let frames = core(&stream);
    assert_eq!(frames[0].0.syntax, Syntax::Ac3);
    let on = decode(&frames, experimental(ProgrammeScope::CoreOnly));
    assert!(on.pcm == plain(&frames).pcm);
    assert!(
        on.status
            .iter()
            .all(|s| *s == DecorrelationStatus::NotApplied(NotApplied::Ac3))
    );
    assert!(NotApplied::Ac3.to_string().contains("not applied"));
}

// ---------------------------------------------------------------------
// 3. A scope that is not known, or a programme with dependents, is left as
//    it is, and the decoder says why.

#[test]
fn unknown_or_dependent_scope_is_left_as_it_is() {
    for name in [STEREO, SCENE] {
        let stream = fixture(name);
        let frames = core(&stream);
        let reference = plain(&frames);
        for (scope, why) in [
            (ProgrammeScope::Unknown, NotApplied::ScopeUnknown),
            (ProgrammeScope::WithDependents, NotApplied::WithDependents),
        ] {
            let on = decode(&frames, experimental(scope));
            assert!(on.pcm == reference.pcm, "{name} {scope:?}");
            for status in &on.status {
                assert_eq!(*status, DecorrelationStatus::NotApplied(why), "{name}");
            }
            let text = on.status[0].to_string();
            assert!(text.contains("not applied"), "{text}");
        }
    }
    let mut dec = Decoder::new(Options::default());
    dec.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::WithDependents);
    assert!(
        dec.decorrelation_status()
            .to_string()
            .contains("unproven mapping")
    );
}

// ---------------------------------------------------------------------
// 4. CoreOnly changes the coupled region and nothing else.

/// The coupled bins of a channel in a block.
fn coupled(coupling: Option<&Coupling>, ch: usize) -> std::ops::Range<usize> {
    match coupling {
        Some(c) if c.is_coupled(ch) => c.start..c.end,
        _ => 0..0,
    }
}

fn changes_only_the_coupled_region(name: &str) {
    let stream = fixture(name);
    let frames = core(&stream);
    let (acmod, lfeon) = (frames[0].0.acmod, frames[0].0.lfeon);
    let roles = roles(acmod, lfeon).expect("a layout whose mapping was seen");

    // The fixture has what the test needs.
    let plain_blocks = blocks(&frames);
    assert!(
        plain_blocks
            .iter()
            .any(|(_, c)| c.is_some_and(|c| !c.aht && !c.enhanced)),
        "{name}: no block with standard coupling"
    );
    // What the fixture cannot show: in both fixtures the coupled region ends
    // where the channels' bandwidth does, so every coupled channel is zero
    // at and above the end of the coupled region, and the comparison below
    // holds only the lower edge. The upper edge is held at the level of a
    // block (`bins_on_both_sides_of_the_coupled_region_are_left_as_they_are`
    // in `decorr.rs`). Should a fixture come to hold something there, this
    // says so, and the comparison below then holds that edge as well.
    let above = plain_blocks.iter().any(|(coeffs, c)| {
        c.is_some_and(|c| {
            (0..coeffs.len())
                .filter(|&ch| c.is_coupled(ch))
                .any(|ch| coeffs[ch][c.end..].iter().any(|v| *v != 0.0))
        })
    });
    assert!(
        !above,
        "{name} now holds content above the coupled region: update the note above"
    );

    // In the coefficient domain: only coupled bins of channels that take a
    // part move, and some of them do.
    let done = decorrelated(&frames);
    let mut moved = 0usize;
    for ((plain, coupling), (out, said)) in plain_blocks.iter().zip(&done) {
        if coupling.is_some_and(|c| !c.aht && !c.enhanced) {
            assert_eq!(
                *said,
                Ok(()),
                "{name}: a coupled block was not decorrelated"
            );
        }
        for (ch, role) in roles.iter().enumerate() {
            let region = if *role == Role::Untouched {
                0..0
            } else {
                coupled(coupling.as_ref(), ch)
            };
            for bin in 0..N {
                let same = out[ch][bin].to_bits() == plain[ch][bin].to_bits();
                if region.contains(&bin) {
                    moved += usize::from(!same);
                } else {
                    assert!(
                        same,
                        "{name}: channel {ch} bin {bin} outside the coupled region moved"
                    );
                }
            }
        }
    }
    assert!(moved > 0, "{name}: nothing in the coupled region moved");

    // In the samples: the decode differs, the channels that take no part
    // do not, and the decoder says it decorrelated.
    let reference = plain(&frames);
    let on = decode(&frames, experimental(ProgrammeScope::CoreOnly));
    assert!(on.pcm != reference.pcm, "{name}: the decode did not change");
    for (ch, role) in roles.iter().enumerate() {
        if *role == Role::Untouched {
            for (a, b) in on.pcm.iter().zip(&reference.pcm) {
                assert!(a[ch] == b[ch], "{name}: channel {ch} changed");
            }
        }
    }
    assert!(on.status.contains(&DecorrelationStatus::Applied));
}

#[test]
fn core_only_changes_only_the_coupled_region_of_stereo() {
    changes_only_the_coupled_region(STEREO);
}

#[test]
fn core_only_changes_only_the_coupled_region_of_five_one() {
    changes_only_the_coupled_region(SCENE);
}

// ---------------------------------------------------------------------
// 6. The all-pass and the delay.

/// The real root of `z^3 + c1 z^2 + c2 z + c3` and the pair of the rest.
fn poles(c: [f64; 3]) -> (f64, f64, f64) {
    let p = |z: f64| ((z + c[0]) * z + c[1]) * z + c[2];
    let (mut lo, mut hi) = (-1.0, 1.0);
    assert!(p(lo) < 0.0 && p(hi) > 0.0);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if p(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let r = 0.5 * (lo + hi);
    // z^3 + c1 z^2 + c2 z + c3 = (z - r)(z^2 + q1 z + q0)
    let q1 = c[0] + r;
    let q0 = c[1] + r * q1;
    let radius = q0.sqrt();
    let angle = (-q1 / (2.0 * radius)).acos().to_degrees();
    (r, radius, angle)
}

#[test]
fn the_allpass_is_the_one_measured() {
    let (real, radius, angle) = poles(ALLPASS);
    assert!((real - 0.7555).abs() < 1e-4, "{real}");
    assert!((radius - 0.7237).abs() < 1e-4, "{radius}");
    assert!((angle - 117.18).abs() < 1e-2, "{angle}");
    assert_eq!(ALLPASS, [-0.0944, 0.0242, -0.3957]);
}

/// The response of `D` to an impulse in the first block.
fn impulse_response(len: usize) -> Vec<f64> {
    let mut d = Delayed::new();
    (0..len)
        .map(|n| d.step(if n == 0 { 1.0 } else { 0.0 }))
        .collect()
}

#[test]
fn d_is_a_delay_of_one_block_then_the_allpass() {
    let h = impulse_response(8);
    let [c1, c2, c3] = ALLPASS;
    // Nothing in the block of the impulse: the delay.
    assert_eq!(h[0], 0.0);
    // Then the all-pass from its first tap: c3, c2 - c1 c3, ...
    assert!((h[1] - c3).abs() < 1e-12, "{h:?}");
    assert!((h[2] - (c2 - c1 * c3)).abs() < 1e-12, "{h:?}");
}

#[test]
fn d_has_unit_magnitude() {
    let h = impulse_response(400);
    let energy: f64 = h.iter().map(|v| v * v).sum();
    assert!((energy - 1.0).abs() < 1e-6, "{energy}");
    for k in 0..16 {
        let w = std::f64::consts::PI * f64::from(k) / 16.0;
        let (re, im) = h.iter().enumerate().fold((0.0, 0.0), |(re, im), (n, v)| {
            let a = w * n as f64;
            (re + v * a.cos(), im - v * a.sin())
        });
        let mag = (re * re + im * im).sqrt();
        assert!((mag - 1.0).abs() < 1e-6, "|D| at {w}: {mag}");
    }
}

// ---------------------------------------------------------------------
// 7. The state at the start, the channel table, and the signs.

#[test]
fn the_channel_table() {
    use Role::{Minus, Plus, Untouched};
    assert_eq!(roles(2, false), Some(&[Plus, Minus][..]));
    assert_eq!(
        roles(7, false),
        Some(&[Plus, Untouched, Minus, Plus, Minus][..])
    );
    assert_eq!(
        roles(7, true),
        Some(&[Plus, Untouched, Minus, Plus, Minus, Untouched][..])
    );
    for (acmod, lfeon) in [(0, false), (1, false), (2, true), (3, false), (6, true)] {
        assert_eq!(roles(acmod, lfeon), None, "{acmod} {lfeon}");
    }
}

/// A coupling from bin 49 in eight sub-bands, of the channels in `mask`.
fn coupling(mask: u8, start: usize, end: usize) -> Coupling {
    Coupling {
        start,
        end,
        channels: mask,
        band_structure: 0,
        enhanced: false,
        aht: false,
    }
}

/// A deterministic spread of values in (-1, 1).
fn lcg(seed: &mut u32) -> f64 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    f64::from(*seed >> 8) / f64::from(1u32 << 23) - 1.0
}

/// A stereo block in which both channels have content of their own under
/// the coupling, which is one waveform times a coordinate above it.
#[allow(
    clippy::needless_range_loop,
    reason = "a bin addresses the same place in both channels"
)]
fn stereo_block(seed: &mut u32, c: &Coupling) -> Vec<[f64; N]> {
    let mut coeffs = vec![[0.0; N]; 2];
    for bin in 4..c.start {
        coeffs[0][bin] = 0.1 * lcg(seed);
        coeffs[1][bin] = 0.1 * lcg(seed);
    }
    for bin in c.start..c.end {
        let x = 0.05 * lcg(seed);
        coeffs[0][bin] = x * 0.8;
        coeffs[1][bin] = x * 0.6;
    }
    coeffs
}

#[test]
fn the_first_block_of_a_fresh_decorrelator_is_the_plain_decode() {
    let c = coupling(0b11, 49, 145);
    let mut seed = 11;
    let mut dec = Decorrelator::new();
    let first = stereo_block(&mut seed, &c);
    let mut out = first.clone();
    assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
    assert_eq!(out, first, "the first block has no past to give");
    let second = stereo_block(&mut seed, &c);
    let mut out = second.clone();
    assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
    assert_ne!(out[0], second[0], "the second block is decorrelated");
}

#[test]
fn a_bin_out_of_coupling_starts_again_from_rest() {
    let full = coupling(0b11, 49, 145);
    let narrow = coupling(0b11, 73, 145);
    let mut seed = 13;
    let input: Vec<_> = (0..6).map(|_| stereo_block(&mut seed, &full)).collect();

    // Coupled, coupled, no coupling, coupled: the last block is what a
    // fresh decorrelator makes of it, which is the plain decode.
    let mut dec = Decorrelator::new();
    for (n, block) in input.iter().take(4).enumerate() {
        let mut out = block.clone();
        let c = (n != 2).then_some(&full);
        let said = dec.block(2, false, c, &mut out);
        if n == 2 {
            assert_eq!(said, Err(NotApplied::NoCoupling));
            assert_eq!(out, *block);
        } else {
            assert_eq!(said, Ok(()));
        }
        if n == 3 {
            assert_eq!(out, *block, "the state was not set to rest");
        }
    }

    // The range narrows and widens again: the bins that left it start from
    // rest, the bins that stayed carry on.
    let mut dec = Decorrelator::new();
    for (n, c) in [&full, &full, &narrow, &full].into_iter().enumerate() {
        let mut out = input[n].clone();
        assert_eq!(dec.block(2, false, Some(c), &mut out), Ok(()));
        if n == 3 {
            for bin in 49..73 {
                assert_eq!(
                    out[0][bin], input[n][0][bin],
                    "bin {bin} came back with a past"
                );
            }
            assert!(
                (73..145).any(|bin| out[0][bin] != input[n][0][bin]),
                "the bins that stayed lost their past"
            );
        }
    }
}

/// Fits `y = a x + b d` on the bins of one sub-band.
fn fit(x: &[f64], d: &[f64], y: &[f64]) -> Option<(f64, f64, f64)> {
    let dot = |p: &[f64], q: &[f64]| p.iter().zip(q).map(|(a, b)| a * b).sum::<f64>();
    let (xx, xd, dd) = (dot(x, x), dot(x, d), dot(d, d));
    let det = xx * dd - xd * xd;
    if dd < 1e-20 || det.abs() < 1e-12 * xx * dd {
        return None;
    }
    let (xy, dy) = (dot(x, y), dot(d, y));
    let a = (xy * dd - dy * xd) / det;
    let b = (dy * xx - xy * xd) / det;
    let residual: f64 = (0..x.len())
        .map(|i| (y[i] - a * x[i] - b * d[i]).powi(2))
        .sum::<f64>();
    Some((a, b, residual / dot(y, y).max(1e-300)))
}

/// The pairs `(a, b)` every channel of `frames` was decorrelated with, by
/// channel, read back from the output with `D` run beside it.
fn pairs(frames: &[(FrameHeader, &[u8])]) -> Vec<Vec<(f64, f64)>> {
    let plain_blocks = blocks(frames);
    let done = decorrelated(frames);
    let nch = plain_blocks[0].0.len();
    let mut filters = vec![[Delayed::new(); N]; nch];
    let mut out = vec![Vec::new(); nch];
    for ((plain, coupling), (decorrelated, _)) in plain_blocks.iter().zip(&done) {
        for ch in 0..nch {
            let region = coupled(coupling.as_ref(), ch);
            let mut d = [0.0; N];
            for bin in 0..N {
                if region.contains(&bin) {
                    d[bin] = filters[ch][bin].step(plain[ch][bin]);
                } else {
                    filters[ch][bin] = Delayed::new();
                }
            }
            let Some(c) = coupling else { continue };
            if !c.is_coupled(ch) {
                continue;
            }
            for s in 0..c.sub_bands() {
                let r = c.sub_band(s);
                if let Some((a, b, residual)) =
                    fit(&plain[ch][r.clone()], &d[r.clone()], &decorrelated[ch][r])
                {
                    assert!(residual < 1e-12, "channel {ch}: not a X + b D(X)");
                    out[ch].push((a, b));
                }
            }
        }
    }
    out
}

fn signs_of_a_pair(name: &str, first: usize, second: usize) {
    let stream = fixture(name);
    let pairs = pairs(&core(&stream));
    for ch in [first, second] {
        let moved: Vec<_> = pairs[ch].iter().filter(|(_, b)| b.abs() > 0.05).collect();
        assert!(
            !moved.is_empty(),
            "{name}: channel {ch} was not decorrelated"
        );
        for (a, b) in &moved {
            assert!(
                (a * a + b * b - 1.0).abs() < 1e-9,
                "{name}: a^2 + b^2 = {}",
                a * a + b * b
            );
            let positive = ch == first;
            assert_eq!(*b > 0.0, positive, "{name}: channel {ch} takes b = {b}");
        }
    }
}

#[test]
fn left_and_right_take_opposite_signs() {
    signs_of_a_pair(STEREO, 0, 1);
    signs_of_a_pair(SCENE, 0, 2);
}

#[test]
fn the_surround_pair_takes_opposite_signs() {
    signs_of_a_pair(SCENE, 3, 4);
}

#[test]
fn the_centre_and_the_lfe_are_untouched() {
    let stream = fixture(SCENE);
    let frames = core(&stream);
    assert_eq!((frames[0].0.acmod, frames[0].0.lfeon), (7, true));
    let mut left_moved = false;
    for ((plain, _), (out, _)) in blocks(&frames).iter().zip(&decorrelated(&frames)) {
        assert_eq!(out[1], plain[1], "the centre");
        assert_eq!(out[5], plain[5], "the LFE");
        left_moved |= out[0] != plain[0];
    }
    assert!(left_moved, "nothing was decorrelated, so nothing was shown");
}

// ---------------------------------------------------------------------
// 8. Determinism, a late start, a concealed frame, a change of scope.

#[test]
fn two_fresh_decoders_agree_to_the_bit() {
    for name in [STEREO, SCENE] {
        let stream = fixture(name);
        let frames = core(&stream);
        let one = decode(&frames, experimental(ProgrammeScope::CoreOnly));
        let two = decode(&frames, experimental(ProgrammeScope::CoreOnly));
        assert!(one.pcm == two.pcm, "{name}");
        assert_eq!(one.status, two.status);
        assert!(one.status.contains(&DecorrelationStatus::Applied), "{name}");
        assert!(
            one.pcm != plain(&frames).pcm,
            "{name}: nothing was decorrelated"
        );
    }
}

/// A decoder that starts ten frames in stands, ten frames later, within
/// what is left in the filters of the one that started at the beginning:
/// sixty blocks at a pole radius of 0,7555 is a factor of 5e-8.
#[test]
fn a_late_start_converges() {
    const LATE: usize = 10;
    let stream = fixture(STEREO);
    let frames = core(&stream);
    let early = decode(&frames, experimental(ProgrammeScope::CoreOnly));
    let late = decode_with(
        &frames,
        LATE,
        experimental(ProgrammeScope::CoreOnly),
        |_, _| {},
        &[],
    );
    let reference = plain(&frames);
    assert!(
        early.pcm[2 * LATE..] != reference.pcm[2 * LATE..],
        "nothing was decorrelated where the two are compared"
    );
    // Where the late decoder starts, the two stand apart: it has no past.
    assert!(early.pcm[LATE + 1] != late.pcm[1]);
    let mut worst = 0.0f32;
    for (a, b) in early.pcm[2 * LATE..].iter().zip(&late.pcm[LATE..]) {
        for (ca, cb) in a.iter().zip(b) {
            for (x, y) in ca.iter().zip(cb) {
                worst = worst.max((f32::from_bits(*x) - f32::from_bits(*y)).abs());
            }
        }
    }
    assert!(worst < 1e-6, "{worst}");
}

/// Decodes from frame 0 with `between` acting before frame `at` and
/// `concealed` frames, and holds what comes out from frame `at + 1` on to a
/// fresh decoder started at `at`. A decoder without `between`, and without
/// the concealed frame just before `at`, keeps its past at `at` and must
/// stand apart from the fresh one: else the test shows nothing.
fn starts_from_rest_at(at: usize, concealed: &[usize], between: impl FnMut(usize, &mut Decoder)) {
    let stream = fixture(STEREO);
    let frames = core(&stream);
    let through = decode_with(
        &frames,
        0,
        experimental(ProgrammeScope::CoreOnly),
        between,
        concealed,
    );
    let fresh = decode_with(
        &frames,
        at,
        experimental(ProgrammeScope::CoreOnly),
        |_, _| {},
        &[],
    );
    assert!(through.pcm[at + 1..] == fresh.pcm[1..]);
    let others: Vec<usize> = concealed.iter().copied().filter(|&n| n + 1 != at).collect();
    let control = decode_with(
        &frames,
        0,
        experimental(ProgrammeScope::CoreOnly),
        |_, _| {},
        &others,
    );
    assert!(
        control.pcm[at + 1..] != fresh.pcm[1..],
        "a decoder whose filters were not set to rest stands with the fresh one"
    );
}

#[test]
fn a_concealed_frame_sets_the_filters_to_rest() {
    const LOST: usize = 20;
    starts_from_rest_at(LOST + 1, &[LOST], |_, _| {});
    let stream = fixture(STEREO);
    let frames = core(&stream);
    let on = decode_with(
        &frames,
        0,
        experimental(ProgrammeScope::CoreOnly),
        |_, _| {},
        &[LOST],
    );
    assert_eq!(
        on.status[LOST],
        DecorrelationStatus::NotApplied(NotApplied::Concealed)
    );
    assert_eq!(on.status[LOST + 1], DecorrelationStatus::Applied);
}

#[test]
fn a_change_of_scope_sets_the_filters_to_rest() {
    const AT: usize = 15;
    starts_from_rest_at(AT, &[], |n, d| {
        if n == AT {
            d.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::WithDependents);
            d.set_decorrelation(Decorrelation::Experimental, ProgrammeScope::CoreOnly);
        }
    });
}

// ---------------------------------------------------------------------
// 9. Power, measured.

/// The power of every channel in every coupling sub-band, summed over the
/// blocks from the second frame to the last, of the decorrelated
/// coefficients over the plain ones, in dB, as `(channel, sub-band, dB)`.
///
/// It is measured on the coefficients the decorrelation hands to the
/// transform. The windowed overlap-add of the transform is orthogonal, so
/// over the same blocks this is the power of the samples in those bins. The
/// first frame is left out: its first block is the plain decode by rule.
/// A sub-band whose plain power is more than 100 dB under the loudest is
/// left out as silence.
///
/// `a^2 + b^2 = 1` keeps the power only where `X` and `D(X)` are
/// uncorrelated over the window: the power of `a X + b D(X)` is that of `X`
/// plus `2 a b` times their correlation. Along the blocks the coefficients
/// of noise are uncorrelated and those of a steady tone are not.
///
/// `tail` blocks are run through the decorrelator after the last one: the
/// block whose coupled region is loudest, with its coupled region silent,
/// so that what `D` still holds of the last blocks comes out and is
/// counted, under the weight that block's own region makes. With `tail` zero the window ends
/// where the stream does, and what `D` holds then is lost, as it is in a
/// decode.
fn power_figures(name: &str, tail: usize) -> Vec<(usize, usize, f64)> {
    let stream = fixture(name);
    let frames = core(&stream);
    let (acmod, lfeon) = (frames[0].0.acmod, frames[0].0.lfeon);
    let blocks_per_frame = usize::from(frames[0].0.blocks);
    let mut plain_blocks = blocks(&frames);
    let coupled_energy = |(coeffs, coupling): &(Vec<[f64; N]>, Option<Coupling>)| {
        coupling.map_or(0.0, |c| {
            coeffs
                .iter()
                .map(|ch| ch[c.start..c.end].iter().map(|v| v * v).sum::<f64>())
                .sum::<f64>()
        })
    };
    let loudest_block = plain_blocks
        .iter()
        .max_by(|p, q| coupled_energy(p).total_cmp(&coupled_energy(q)))
        .cloned()
        .expect("a block");
    let (mut silent, held_coupling) = loudest_block;
    if let Some(c) = held_coupling {
        for channel in &mut silent {
            channel[c.start..c.end].fill(0.0);
        }
    }
    plain_blocks.extend(std::iter::repeat_n((silent, held_coupling), tail));
    let mut dec = Decorrelator::new();
    let done: Vec<_> = plain_blocks
        .iter()
        .map(|(coeffs, coupling)| {
            let mut out = coeffs.clone();
            let _ = dec.block(acmod, lfeon, coupling.as_ref(), &mut out);
            (out, ())
        })
        .collect();
    let nch = plain_blocks[0].0.len();
    // (channel, sub-band) -> (plain, decorrelated)
    let mut sums = vec![[(0.0f64, 0.0f64); 18]; nch];
    for ((plain, coupling), (out, _)) in plain_blocks.iter().zip(&done).skip(blocks_per_frame) {
        let Some(c) = coupling else { continue };
        for (ch, sums) in sums.iter_mut().enumerate() {
            if !c.is_coupled(ch) {
                continue;
            }
            for (s, sum) in sums.iter_mut().enumerate().take(c.sub_bands()) {
                for bin in c.sub_band(s) {
                    sum.0 += plain[ch][bin] * plain[ch][bin];
                    sum.1 += out[ch][bin] * out[ch][bin];
                }
            }
        }
    }
    let loudest = sums.iter().flatten().map(|s| s.0).fold(0.0f64, f64::max);
    assert!(loudest > 0.0, "{name}: no power in the coupled region");
    let mut figures = Vec::new();
    for (ch, row) in sums.iter().enumerate() {
        for (s, (p, q)) in row.iter().enumerate() {
            if *p >= loudest * 1e-10 {
                figures.push((ch, s, 10.0 * (q / p).log10()));
            }
        }
    }
    assert!(!figures.is_empty(), "{name}: nothing measured");
    assert!(
        figures.iter().any(|f| f.2 != 0.0),
        "{name}: nothing was decorrelated, so nothing was measured"
    );
    for (ch, s, db) in &figures {
        println!("{name}, tail {tail}: channel {ch} sub-band {s}: {db:+.3} dB");
    }
    figures
}

/// The stereo fixture is noise in the coupled region, and keeps its power
/// to 0,1 dB in every channel and sub-band.
#[test]
fn power_stays_in_stereo() {
    for (ch, s, db) in power_figures(STEREO, 0) {
        assert!(db.abs() <= 0.1, "channel {ch} sub-band {s}: {db:+.3} dB");
    }
}

/// The authored scene is sines under 6 kHz, and its coupled region, bins
/// 133 to 205, is close to silence, some 110 dB down, but for one transient
/// in block 376 of 378, where the stream ends.
///
/// With the window ending where the stream does, two blocks after the
/// transient, what `D` makes of it is mostly cut off, and the blocks after
/// it have an own region that is silent, which takes `b` to 0: a channel
/// keeps about `a^2` of its power there. Those figures fall with the
/// sub-band as `a` does, down to -3,1 dB, and they are what a decode of this
/// stream gives. They are asserted only against `10 log a^2` at the largest
/// angle, -6 dB, and 0,1 dB above: a sub-band can lose no more than the
/// filtered part, and gain nothing it did not have.
///
/// With a tail of 32 blocks that keeps the own region of the transient's
/// block, which a pole radius of 0,7555 leaves 1e-4 of the filter's past,
/// the figures stand within 0,47 dB: what is left is the weight moving with
/// the smoothing while `D` rings out, and the transient reaching into the
/// blocks around it. The bound asserted is 0,6 dB. A pair whose squares do
/// not sum to 1 breaks it: `b = 1 - a` stands at -1,7 to -2,7 dB in the left
/// channel and -0,64 in the right surround under the same measurement.
#[test]
fn power_of_the_scene_is_measured() {
    let floor = 20.0 * ANGLE_MAX.cos().log10();
    for (ch, s, db) in power_figures(SCENE, 0) {
        assert!(
            (floor - 0.1..=0.1).contains(&db),
            "channel {ch} sub-band {s}: {db:+.3} dB"
        );
    }
    for (ch, s, db) in power_figures(SCENE, 32) {
        assert!(
            db.abs() <= 0.6,
            "with a tail: channel {ch} sub-band {s}: {db:+.3} dB"
        );
    }
}

// ---------------------------------------------------------------------
// 10. A dependent substream is left as it is, and the 5.1 core of a 7.1
//     programme is left as it is unless the caller says it is the whole
//     programme, which it is not.

#[test]
fn a_dependent_substream_is_left_as_it_is() {
    let stream = fixture(SEVEN_ONE);
    let all = frames(&stream);
    let dependent = substream(&all, StreamType::Dependent, 0);
    assert!(!dependent.is_empty());
    let on = decode(&dependent, experimental(ProgrammeScope::CoreOnly));
    assert!(on.pcm == plain(&dependent).pcm);
    assert!(
        on.status
            .iter()
            .all(|s| *s == DecorrelationStatus::NotApplied(NotApplied::DependentSubstream))
    );
    assert!(on.status[0].to_string().contains("unproven mapping"));
}

#[test]
fn the_core_of_a_seven_one_programme_is_left_as_it_is() {
    let stream = fixture(SEVEN_ONE);
    let frames = core(&stream);
    let reference = plain(&frames);
    for scope in [ProgrammeScope::Unknown, ProgrammeScope::WithDependents] {
        let on = decode(&frames, experimental(scope));
        assert!(on.pcm == reference.pcm, "{scope:?}");
        assert!(
            on.status
                .iter()
                .all(|s| matches!(s, DecorrelationStatus::NotApplied(_)))
        );
    }
}
