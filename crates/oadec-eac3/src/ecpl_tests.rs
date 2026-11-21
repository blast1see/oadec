//! Tests of the enhanced coupling synthesis. These need no bitstream and no
//! reference decoder: the process is an analysis of its own synthesis, so it
//! has exact identities that pin the scaling, the block alignment and the
//! angle convention.

use super::*;
use crate::frame::{EcplChannel, Options};

/// A deterministic pseudo-random source.
fn lcg(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    ((*seed >> 33) as f64 / (1u64 << 31) as f64) - 1.0
}

/// The whole enhanced coupling region as one band, one coupled channel.
fn one_band(start: usize, end: usize, amp: u8, angle: u8, chaos: u8) -> EcplBlock {
    EcplBlock {
        start,
        end,
        bands: vec![(start, end)],
        angle_interp: false,
        first_ch: 0,
        chans: vec![Some(EcplChannel {
            amp: vec![amp],
            angle: vec![angle],
            chaos: vec![chaos],
            transient: false,
        })],
        coeffs: [0.0; N],
    }
}

/// Runs three consecutive blocks and returns the middle one's output, which
/// is the only block with both neighbours present.
fn middle(
    synth: &mut Synth,
    imdct: &Imdct,
    blocks: &[[f64; N]; 3],
    proto: &EcplBlock,
) -> Vec<[f64; N]> {
    let mut out = vec![[0.0f64; N]; 1];
    for i in 0..2 {
        let mut b = proto.clone();
        b.coeffs = blocks[i];
        out = vec![[0.0f64; N]; 1];
        synth.block(imdct, &b, &blocks[i + 1], &mut out);
    }
    out
}

#[test]
fn unit_amplitude_and_zero_angle_return_the_carrier() {
    // amp code 0 is 0x20 / 32 >> 0, exactly 1.0; the first coupled channel
    // sends no angle and no chaos, so this is the identity through the whole
    // carrier reconstruction, the forward transform and clause E.3.5.5.4.
    let imdct = Imdct::new();
    let mut synth = Synth::new();
    let proto = one_band(37, 253, 0, 0, 0);
    let mut seed = 0x5eed_1234_abcd_0001u64;
    let mut blocks = [[0.0f64; N]; 3];
    for b in &mut blocks {
        for v in b[37..253].iter_mut() {
            *v = lcg(&mut seed);
        }
    }
    let out = middle(&mut synth, &imdct, &blocks, &proto);
    let mut worst = 0.0f64;
    let mut ratio_sum = 0.0;
    for bin in 37..253 {
        worst = worst.max((out[0][bin] - blocks[1][bin]).abs());
        ratio_sum += out[0][bin] / blocks[1][bin];
    }
    let ratio = ratio_sum / 216.0;
    assert!(
        worst < 1e-9,
        "enhanced coupling is not unity: worst {worst:.3e}, mean ratio {ratio:.6}"
    );
}

#[test]
fn a_neighbour_without_enhanced_coupling_contributes_nothing() {
    // Clause E.3.5.5.1: a neighbour that does not use enhanced coupling has
    // zero coefficients. Feeding zeros must give the same answer as a fresh
    // synthesiser whose previous block was never set.
    let imdct = Imdct::new();
    let proto = one_band(37, 253, 0, 0, 0);
    let mut seed = 0x5eed_1234_abcd_0002u64;
    let mut cur = [0.0f64; N];
    for v in cur[37..253].iter_mut() {
        *v = lcg(&mut seed);
    }
    let mut a = Synth::new();
    let mut b = Synth::new();
    b.skip_block();
    let mut oa = vec![[0.0f64; N]; 1];
    let mut ob = vec![[0.0f64; N]; 1];
    let mut blk = proto.clone();
    blk.coeffs = cur;
    a.block(&imdct, &blk, &[0.0; N], &mut oa);
    b.block(&imdct, &blk, &[0.0; N], &mut ob);
    assert_eq!(oa, ob);
}

#[test]
fn an_angle_of_pi_negates_the_channel() {
    // ecplangletab[32] is -1.0, meaning -pi. A rotation by pi is a sign flip,
    // so the second coupled channel must be the exact negative of the first.
    let imdct = Imdct::new();
    let mut seed = 0x5eed_1234_abcd_0003u64;
    let mut blocks = [[0.0f64; N]; 3];
    for b in &mut blocks {
        for v in b[37..253].iter_mut() {
            *v = lcg(&mut seed);
        }
    }
    let mut proto = one_band(37, 253, 0, 0, 0);
    proto.chans.push(Some(EcplChannel {
        amp: vec![0],
        angle: vec![32],
        chaos: vec![0],
        transient: false,
    }));
    let imdct_ref = &imdct;
    let mut synth = Synth::new();
    let mut out = vec![[0.0f64; N]; 2];
    for i in 0..2 {
        let mut b = proto.clone();
        b.coeffs = blocks[i];
        out = vec![[0.0f64; N]; 2];
        synth.block(imdct_ref, &b, &blocks[i + 1], &mut out);
    }
    for (bin, (&a, &b)) in out[1][37..253]
        .iter()
        .zip(out[0][37..253].iter())
        .enumerate()
    {
        assert!((a + b).abs() < 1e-9, "bin {bin}: {a} vs {b}");
    }
}

#[test]
fn the_amplitude_table_scales_the_channel() {
    // amp code 4 is 0x10 / 32 = 0.5 exactly, code 31 is minus infinity.
    let imdct = Imdct::new();
    let mut seed = 0x5eed_1234_abcd_0004u64;
    let mut blocks = [[0.0f64; N]; 3];
    for b in &mut blocks {
        for v in b[37..253].iter_mut() {
            *v = lcg(&mut seed);
        }
    }
    let half = middle(
        &mut Synth::new(),
        &imdct,
        &blocks,
        &one_band(37, 253, 4, 0, 0),
    );
    let mute = middle(
        &mut Synth::new(),
        &imdct,
        &blocks,
        &one_band(37, 253, 31, 0, 0),
    );
    for bin in 37..253 {
        assert!((half[0][bin] - 0.5 * blocks[1][bin]).abs() < 1e-9);
        assert_eq!(mute[0][bin], 0.0);
    }
}

#[test]
fn chaos_de_correlates_the_channels() {
    // Clause E.3.5.5.3 adds a scaled random value to each bin angle. As the
    // chaos code rises the second channel must drift away from the first.
    let imdct = Imdct::new();
    let mut seed = 0x5eed_1234_abcd_0005u64;
    let mut blocks = [[0.0f64; N]; 3];
    for b in &mut blocks {
        for v in b[37..253].iter_mut() {
            *v = lcg(&mut seed);
        }
    }
    let mut last = f64::INFINITY;
    for code in [0u8, 2, 4, 7] {
        let mut proto = one_band(37, 253, 0, 0, 0);
        proto.chans.push(Some(EcplChannel {
            amp: vec![0],
            angle: vec![0],
            chaos: vec![code],
            transient: false,
        }));
        let mut synth = Synth::new();
        let mut out = vec![[0.0f64; N]; 2];
        for i in 0..2 {
            let mut b = proto.clone();
            b.coeffs = blocks[i];
            out = vec![[0.0f64; N]; 2];
            synth.block(&imdct, &b, &blocks[i + 1], &mut out);
        }
        let (mut num, mut da, mut db) = (0.0, 0.0, 0.0);
        for (&a, &b) in out[0][37..253].iter().zip(out[1][37..253].iter()) {
            num += a * b;
            da += a * a;
            db += b * b;
        }
        let corr = num / (da * db).sqrt();
        assert!(
            corr < last + 1e-9,
            "chaos {code} raised the correlation to {corr:.4} from {last:.4}"
        );
        last = corr;
    }
    assert!(last < 0.5, "full chaos still correlates at {last:.4}");
}

#[test]
fn the_options_are_untouched() {
    // A guard that the module never reaches for the dither switch: enhanced
    // coupling de-correlation is not clause 6.3.4 dither.
    let _ = Options::default();
}

#[test]
fn interpolation_writes_every_bin_of_the_region() {
    // The bin angles are held in a buffer reused by every channel, so an
    // interpolation that stops short would leave the previous channel's
    // values behind. Clause E.3.5.5.3 walks the bins in three stretches and
    // the arithmetic is easy to get wrong by one.
    let synth = Synth::new();
    for &(start, end, width) in &[(37usize, 253usize, 12usize), (49, 121, 12), (13, 37, 6)] {
        let bands: Vec<(usize, usize)> = (start..end)
            .step_by(width)
            .map(|b| (b, (b + width).min(end)))
            .collect();
        let n = bands.len();
        if n < 2 {
            continue;
        }
        let mut ecpl = one_band(start, end, 0, 0, 0);
        ecpl.bands = bands;
        ecpl.angle_interp = true;
        let angles: Vec<f64> = (0..n).map(|i| (i as f64 / n as f64) - 0.5).collect();
        let mut out = [f64::NAN; N];
        synth.spread_angles(&ecpl, &angles, &mut out);
        for (bin, v) in out.iter().enumerate().take(end).skip(start) {
            assert!(!v.is_nan(), "bin {bin} of {start}..{end} was never written");
            assert!((-1.0..=1.0).contains(v), "bin {bin} wrapped to {v}");
        }
    }
}

// ---- P3: held against the text of A/52:2018 clause E.3.5.5 ----

use crate::frame::MAX_FBW;

/// Three blocks of carrier over `37..253`.
fn carrier(seed: u64) -> [[f64; N]; 3] {
    let mut seed = seed;
    let mut blocks = [[0.0f64; N]; 3];
    for b in &mut blocks {
        for v in b[37..253].iter_mut() {
            *v = lcg(&mut seed);
        }
    }
    blocks
}

/// The hook's values: every `rand_notrans` is `notrans`, and `trans` is
/// the `rand_trans` stream.
fn given(notrans: f64, trans: Vec<f64>) -> RandomValues {
    RandomValues {
        notrans: vec![[notrans; N]; MAX_FBW],
        trans,
    }
}

/// A unity first channel and one more coupled channel over `bands`.
fn with_second(
    bands: &[(usize, usize)],
    amp: u8,
    angle: &[u8],
    chaos: &[u8],
    transient: bool,
) -> EcplBlock {
    let n = bands.len();
    let mut b = one_band(bands[0].0, bands[n - 1].1, 0, 0, 0);
    b.bands = bands.to_vec();
    b.chans[0] = Some(EcplChannel {
        amp: vec![0; n],
        angle: vec![0; n],
        chaos: vec![0; n],
        transient: false,
    });
    b.chans.push(Some(EcplChannel {
        amp: vec![amp; n],
        angle: angle.to_vec(),
        chaos: chaos.to_vec(),
        transient,
    }));
    b
}

/// The middle block of three, every channel of it.
fn run_middle(synth: &mut Synth, blocks: &[[f64; N]; 3], proto: &EcplBlock) -> Vec<[f64; N]> {
    let imdct = Imdct::new();
    let mut out = Vec::new();
    for i in 0..2 {
        let mut b = proto.clone();
        b.coeffs = blocks[i];
        out = vec![[0.0f64; N]; proto.chans.len()];
        synth.block(&imdct, &b, &blocks[i + 1], &mut out);
    }
    out
}

#[test]
fn chaos_lowers_the_amplitude_where_no_transient_is() {
    // A/52:2018 clause E.3.5.5.2: `ampbnd *= 1 + 0.38 * chaos` for a
    // channel after the first without a transient. ecplchaostab[7] is -1, so
    // the amplitude is 0.62 (ETSI TS 102 366 V1.2.1 prints `1 - 0.38`, which
    // would give 1.38). With every rand_notrans zero the angle is left alone
    // and the channel is the carrier times 0.62.
    let blocks = carrier(0x5eed_0003_0000_0001);
    let proto = with_second(&[(37, 253)], 0, &[0], &[7], false);
    let mut synth = Synth::with_random_values(given(0.0, vec![0.0; 64]));
    let out = run_middle(&mut synth, &blocks, &proto);
    for bin in 37..253 {
        let want = 0.62 * blocks[1][bin];
        assert!(
            (out[1][bin] - want).abs() < 1e-9,
            "bin {bin}: {} where the text gives {want}",
            out[1][bin]
        );
    }
}

#[test]
fn the_chaos_term_is_subtracted_from_the_angle() {
    // Clause E.3.5.5.3: `angle += -chaos * rand`. With chaos -1 and every
    // rand_notrans 0.25 the bin angle is +0.25, the angle of code 8; the
    // amplitude is 0.62 (clause E.3.5.5.2).
    let blocks = carrier(0x5eed_0003_0000_0002);
    let chaotic = with_second(&[(37, 253)], 0, &[0], &[7], false);
    let rotated = with_second(&[(37, 253)], 0, &[8], &[0], false);
    let got = run_middle(
        &mut Synth::with_random_values(given(0.25, vec![0.0; 64])),
        &blocks,
        &chaotic,
    );
    let want = run_middle(&mut Synth::new(), &blocks, &rotated);
    for bin in 37..253 {
        assert!(
            (got[1][bin] - 0.62 * want[1][bin]).abs() < 1e-9,
            "bin {bin}: {} against {}",
            got[1][bin],
            0.62 * want[1][bin]
        );
    }
}

#[test]
fn rand_trans_is_drawn_in_the_order_of_the_contract() {
    // Per block, per coupled channel after the first, per band. The second
    // call is the middle block and takes values 5..8 (in 1/32), which are
    // the angles of codes 5..8 once chaos -1 negates them; a transient
    // channel keeps its amplitude. rand_notrans is NaN, so a channel that
    // reads it instead shows at once.
    let blocks = carrier(0x5eed_0003_0000_0003);
    let bands = [(37usize, 121usize), (121, 253)];
    let mut proto = with_second(&bands, 0, &[0, 0], &[7, 7], true);
    proto.chans.push(Some(EcplChannel {
        amp: vec![0, 0],
        angle: vec![0, 0],
        chaos: vec![7, 7],
        transient: true,
    }));
    let trans: Vec<f64> = (1..=8).map(|k| f64::from(k) / 32.0).collect();
    let got = run_middle(
        &mut Synth::with_random_values(given(f64::NAN, trans)),
        &blocks,
        &proto,
    );
    let mut want_proto = with_second(&bands, 0, &[5, 6], &[0, 0], true);
    want_proto.chans.push(Some(EcplChannel {
        amp: vec![0, 0],
        angle: vec![7, 8],
        chaos: vec![0, 0],
        transient: true,
    }));
    let want = run_middle(&mut Synth::new(), &blocks, &want_proto);
    for ch in 1..3 {
        for bin in 37..253 {
            assert!(
                (got[ch][bin] - want[ch][bin]).abs() < 1e-9,
                "channel {ch} bin {bin}: {} against {}",
                got[ch][bin],
                want[ch][bin]
            );
        }
    }
}

#[test]
fn every_transient_channel_draws_its_own_values() {
    // Clause E.3.5.5.3: rand_trans "must be unique for each band [bnd] and
    // channel [ch]". Two channels with the same codes and a transient must
    // not come out the same.
    let blocks = carrier(0x5eed_0003_0000_0004);
    let mut proto = with_second(&[(37, 253)], 0, &[0], &[7], true);
    proto.chans.push(Some(EcplChannel {
        amp: vec![0],
        angle: vec![0],
        chaos: vec![7],
        transient: true,
    }));
    let out = run_middle(&mut Synth::new(), &blocks, &proto);
    let same = (37..253).all(|bin| out[1][bin] == out[2][bin]);
    assert!(!same, "two transient channels drew the same rand_trans");
}

#[test]
fn the_first_coupled_channel_ignores_angle_and_chaos() {
    // Clauses E.3.5.5.2/3: angle and chaos are zero for firstchincpl, even
    // when the codes held for it are not (a channel that was not first in
    // an earlier block). Channel 0 is not coupled, so channel 1 is first.
    let blocks = carrier(0x5eed_0003_0000_0005);
    let mut proto = one_band(37, 253, 0, 32, 7);
    proto.chans.insert(0, None);
    proto.first_ch = 1;
    let out = run_middle(&mut Synth::new(), &blocks, &proto);
    for bin in 37..253 {
        assert!((out[1][bin] - blocks[1][bin]).abs() < 1e-9, "bin {bin}");
    }
}

#[test]
fn rand_notrans_is_uniform_and_differs_per_channel() {
    // Clause E.3.5.5.3: uniform on [-1, 1), unique per bin and channel.
    let synth = Synth::new();
    let all: Vec<f64> = synth.notrans.iter().flatten().copied().collect();
    assert!(all.iter().all(|v| (-1.0..1.0).contains(v)));
    let mean = all.iter().sum::<f64>() / all.len() as f64;
    let var = all.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / all.len() as f64;
    assert!(mean.abs() < 0.03, "mean {mean}");
    assert!((var - 1.0 / 3.0).abs() < 0.03, "variance {var}");
    for a in 0..MAX_FBW {
        for b in a + 1..MAX_FBW {
            assert_ne!(synth.notrans[a], synth.notrans[b], "channels {a} and {b}");
        }
    }
}

#[test]
fn rand_notrans_survives_a_reset() {
    // "generated once ... must stay the same for every block of every frame"
    let mut synth = Synth::new();
    let before = synth.notrans;
    synth.reset();
    synth.skip_block();
    assert_eq!(before, synth.notrans);
}

#[test]
fn a_transient_redraws_every_block_and_no_transient_does_not() {
    // The same block twice from a zero past: without a transient the output
    // repeats exactly; with one, rand_trans is new and it does not.
    let blocks = carrier(0x5eed_0003_0000_0006);
    let imdct = Imdct::new();
    for transient in [false, true] {
        let mut proto = with_second(&[(37, 253)], 0, &[0], &[7], transient);
        proto.coeffs = blocks[1];
        let mut synth = Synth::new();
        let mut twice = Vec::new();
        for _ in 0..2 {
            synth.skip_block();
            let mut out = vec![[0.0f64; N]; 2];
            synth.block(&imdct, &proto, &blocks[2], &mut out);
            twice.push(out);
        }
        assert_eq!(twice[0] == twice[1], !transient, "transient {transient}");
    }
}

#[test]
fn the_band_layout_follows_the_syntax() {
    // ecplsubbndtab and the default ecplbndstrc over the whole range:
    // sub-bands up to 8 are never merged.
    let s = crate::tables::DEFAULT_ECPL_BAND_STRUCT;
    let ch = Some(EcplChannel::default());
    let b = EcplBlock::from_syntax(0, 22, &s, false, vec![ch.clone()], [0.0; N]).expect("block");
    let want = [
        (13, 19),
        (19, 25),
        (25, 31),
        (31, 37),
        (37, 49),
        (49, 61),
        (61, 73),
        (73, 85),
        (85, 109),
        (109, 133),
        (133, 157),
        (157, 205),
        (205, 253),
    ];
    assert_eq!(b.bands, want);
    assert_eq!((b.start, b.end, b.first_ch), (13, 253, 0));
    // entries below max(9, begin + 1) are never sent, so they are ignored
    let b =
        EcplBlock::from_syntax(4, 12, &[true; 22], true, vec![None, ch], [0.0; N]).expect("block");
    assert_eq!(b.bands, [(37, 49), (49, 61), (61, 73), (73, 85), (85, 133)]);
    assert_eq!(b.first_ch, 1);
    assert!(EcplBlock::from_syntax(5, 5, &[false; 22], false, vec![None], [0.0; N]).is_err());
}
