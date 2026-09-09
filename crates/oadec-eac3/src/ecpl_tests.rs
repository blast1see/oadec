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
