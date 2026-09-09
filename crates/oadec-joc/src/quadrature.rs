//! The 90-degree phase shift of `joc_dmx_config_idx` 3 and 4.
//!
//! Table 47 of ETSI TS 103 420 names two downmix configurations "5.X with
//! 90 degree phase shift" and then the standard never says what the shift is
//! or how a decoder takes it back out. In the quadrature mirror filter
//! domain the obvious reading is to multiply the surround pair by -j, and
//! for every subband but the lowest that is exactly right: against the
//! object output of the Dolby decoder the difference above 141 Hz sits on
//! the dither floor.
//!
//! Subband 0 is different. It spans 0 to 375 Hz, so it straddles direct
//! current and its complex signal is not analytic there: the image of the
//! negative frequencies of a real signal falls inside the passband, only a
//! few decibels down near direct current. A single rotation by -j turns
//! that image the wrong way and the two halves cancel instead of adding, so
//! the reconstructed objects lose their bottom octaves. Measured against
//! Dolby the loss reaches 14 dB below 50 Hz.
//!
//! What Dolby does instead was measured on three titles from three encoders
//! (`docs/evidence`): the operator is the identity at direct current and
//! reaches -j by about 141 Hz. Since the lowest subband is a signal sampled
//! at one value per time slot, a delay of one slot is a delay of [`BANDS`]
//! samples exactly, and the operator is therefore a short filter across time
//! slots. It is written here as the plain rotation plus a real correction,
//! so that switching the correction off leaves the plain reading behind for
//! comparison. `tools/gen_joc_quadrature.py` regenerates [`CORRECTION`] from
//! the measurement.

use crate::qmf::{BANDS, Complex};

/// Taps of the subband 0 correction.
pub const LOW_TAPS: usize = 37;

/// Group delay of the correction, in time slots. Every downmix channel is
/// held back by this much so that the subbands stay aligned, at a cost of
/// `LOW_DELAY * BANDS` samples of decoder delay.
pub const LOW_DELAY: usize = LOW_TAPS / 2;

/// Time slots between the side information of a frame and the subband
/// samples the analysis bank hands over for it.
///
/// The standard says nothing about this: clause 6.6.6 pairs `x[ch][ts][sb]`
/// with `joc_mix_mtx_interp[obj][ch][ts][sb]` and leaves it to the reader to
/// notice that the analysis bank the decoder runs is not the one the encoder
/// ran. Measured against the object output of the Dolby decoder on three
/// titles from three encoders, the matrix of slot `ts` belongs to the
/// subband samples the analysis produces `MATRIX_ALIGN` slots earlier; the
/// optimum is sharp, 15 dB per slot either side. Holding the samples back by
/// this much and reading the matrix as it comes takes the object residual
/// from -15 dB to below -70 dB in the bands where the core decode itself is
/// exact.
pub const MATRIX_ALIGN: usize = 10;

/// Correction of subband 0, oldest tap first. Its response is 1 at direct
/// current and 0 from about 141 Hz; the subband is rotated by `-j` where the
/// response is 0 and left alone where it is 1.
const CORRECTION: [f64; LOW_TAPS] = [
    -5.91681646831882e-4,
    -3.96577998889008e-4,
    -4.89987417639055e-4,
    -3.80195704118650e-4,
    -3.11752619393662e-4,
    -1.57833513265854e-4,
    -3.07416248646661e-5,
    -7.35799134449175e-4,
    -1.14759224551262e-3,
    -3.92909913944947e-4,
    1.15058489925437e-3,
    3.70595902647225e-3,
    6.57877469152787e-3,
    9.11478110568980e-3,
    1.05739758211185e-2,
    1.21739014906547e-2,
    1.79624386477984e-2,
    3.23441699408751e-2,
    5.64597311650803e-2,
    8.52338032116079e-2,
    1.09632870344164e-1,
    1.22185158891375e-1,
    1.20607936365208e-1,
    1.08168072865291e-1,
    8.99364946059255e-2,
    7.02700589774902e-2,
    5.18414250657970e-2,
    3.60740289659201e-2,
    2.36196998000810e-2,
    1.39805851360216e-2,
    8.19101518636338e-3,
    5.04000981537348e-3,
    3.04461626559270e-3,
    1.81051655891756e-3,
    1.06972855753346e-3,
    5.07240399870356e-4,
    2.75697524427658e-4,
];

/// What one downmix channel needs before the mixing matrix is applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Carry {
    /// The channel carries no phase shift, so it is only held back.
    Plain,
    /// The channel carries the 90-degree phase shift of table 47. It is
    /// rotated back by -j, or by +j when `plus`, with the lowest subband
    /// corrected as the Dolby decoder corrects it.
    Shifted { plus: bool },
    /// The same rotation applied flat to every subband, the lowest one
    /// included. This is the plain reading of table 47 and what a decoder
    /// does when it takes subband 0 for an analytic signal; kept so that the
    /// difference can be measured.
    Flat { plus: bool },
    /// The rotation applied to every subband but the lowest, which is left
    /// alone. The other end of the measurement the correction was fitted
    /// between, and on its own no closer to Dolby than [`Carry::Flat`].
    FlatAbove { plus: bool },
}

/// Holds one downmix channel back by [`LOW_DELAY`] time slots and, when the
/// channel carries the 90-degree phase shift, takes the shift back out.
#[derive(Debug)]
pub struct Quadrature {
    carry: Carry,
    /// Subband 0 of the last `2 * LOW_DELAY + 1` slots, oldest first.
    low: [Complex; LOW_TAPS],
    /// Whole slots of the last `LOW_DELAY + 1` time slots, a ring.
    hold: Box<[[Complex; BANDS]]>,
    at: usize,
}

impl Quadrature {
    #[must_use]
    pub fn new(carry: Carry) -> Self {
        Self {
            carry,
            low: [Complex::default(); LOW_TAPS],
            hold: vec![[Complex::default(); BANDS]; LOW_DELAY + 1].into_boxed_slice(),
            at: 0,
        }
    }

    /// Forgets the history, as at a splice.
    pub fn reset(&mut self) {
        self.low = [Complex::default(); LOW_TAPS];
        for slot in &mut self.hold {
            *slot = [Complex::default(); BANDS];
        }
        self.at = 0;
    }

    /// Takes the subband samples of one time slot and writes the samples of
    /// the slot [`LOW_DELAY`] slots back, ready for the mixing matrix.
    pub fn step(&mut self, slot: &[Complex; BANDS], out: &mut [Complex; BANDS]) {
        self.low.rotate_left(1);
        self.low[LOW_TAPS - 1] = slot[0];
        self.hold[self.at] = *slot;
        self.at = (self.at + 1) % self.hold.len();
        // the entry the ring is about to overwrite is LOW_DELAY slots old
        *out = self.hold[self.at];
        match self.carry {
            Carry::Plain => {}
            Carry::Flat { plus } => {
                for s in out.iter_mut() {
                    *s = rotate(*s, plus);
                }
            }
            Carry::FlatAbove { plus } => {
                for s in &mut out[1..] {
                    *s = rotate(*s, plus);
                }
            }
            Carry::Shifted { plus } => {
                // a convolution, so the taps run against the shift register
                let mut acc = Complex::default();
                for (c, x) in CORRECTION.iter().rev().zip(&self.low) {
                    acc = acc.plus(x.scale(*c));
                }
                // the correction restores what the rotation takes away:
                // (1 + j) for -j, its conjugate for +j
                let back = if plus {
                    Complex {
                        re: acc.re + acc.im,
                        im: acc.im - acc.re,
                    }
                } else {
                    Complex {
                        re: acc.re - acc.im,
                        im: acc.re + acc.im,
                    }
                };
                out[0] = rotate(out[0], plus).plus(back);
                for s in &mut out[1..] {
                    *s = rotate(*s, plus);
                }
            }
        }
    }
}

fn rotate(c: Complex, plus: bool) -> Complex {
    if plus {
        Complex {
            re: -c.im,
            im: c.re,
        }
    } else {
        Complex {
            re: c.im,
            im: -c.re,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qmf::{Analysis, Synthesis};
    use std::f64::consts::PI;

    /// Gain and phase the filter gives an audio frequency. One slot of
    /// delay is [`BANDS`] samples of delay, so a filter across slots has a
    /// response in audio frequency of `sum h[k] exp(-2 pi i f k / 750)`,
    /// which is what a slot-rate exponential at `hz / 750` measures.
    fn response(hz: f64, carry: Carry) -> (f64, f64) {
        let mut q = Quadrature::new(carry);
        let mut out = [Complex::default(); BANDS];
        let (mut re, mut im, mut n) = (0.0, 0.0, 0.0);
        for t in 0..4096 {
            let mut slot = [Complex::default(); BANDS];
            let w = 2.0 * PI * hz * t as f64 / 750.0;
            slot[0] = Complex {
                re: w.cos(),
                im: w.sin(),
            };
            q.step(&slot, &mut out);
            if t < LOW_TAPS + 8 {
                continue;
            }
            let d = 2.0 * PI * hz * (t - LOW_DELAY) as f64 / 750.0;
            let r = out[0].times(Complex {
                re: d.cos(),
                im: -d.sin(),
            });
            re += r.re;
            im += r.im;
            n += 1.0;
        }
        let (re, im) = (re / n, im / n);
        (re.hypot(im), im.atan2(re).to_degrees())
    }

    #[test]
    fn the_correction_leaves_direct_current_alone() {
        let (g, p) = response(0.0, Carry::Shifted { plus: false });
        assert!((g - 1.0).abs() < 0.05, "gain at 0 Hz {g:.3}");
        assert!(p.abs() < 5.0, "phase at 0 Hz {p:.1} degrees");
    }

    #[test]
    fn the_correction_is_gone_above_the_transition() {
        for hz in [200.0, 280.0, 350.0] {
            let (g, p) = response(hz, Carry::Shifted { plus: false });
            assert!((g - 1.0).abs() < 0.02, "gain at {hz} Hz {g:.3}");
            assert!((p + 90.0).abs() < 2.0, "phase at {hz} Hz {p:.1} degrees");
        }
    }

    #[test]
    fn the_plain_reading_rotates_every_subband_by_the_same_amount() {
        for hz in [0.0, 60.0, 300.0] {
            let (g, p) = response(hz, Carry::Flat { plus: false });
            assert!((g - 1.0).abs() < 1e-12, "gain at {hz} Hz {g}");
            assert!((p + 90.0).abs() < 1e-9, "phase at {hz} Hz {p} degrees");
        }
        let (_, p) = response(120.0, Carry::Flat { plus: true });
        assert!((p - 90.0).abs() < 1e-9, "+j phase {p} degrees");
    }

    #[test]
    fn a_plain_channel_comes_out_held_back_and_untouched() {
        let mut q = Quadrature::new(Carry::Plain);
        let mut out = [Complex::default(); BANDS];
        for t in 0..64 {
            let mut slot = [Complex::default(); BANDS];
            for (i, s) in slot.iter_mut().enumerate() {
                *s = Complex {
                    re: (t * 100 + i) as f64,
                    im: (t * 7 + i) as f64,
                };
            }
            q.step(&slot, &mut out);
            if t >= LOW_DELAY {
                let want = t - LOW_DELAY;
                assert_eq!(out[3].re, (want * 100 + 3) as f64, "slot {t}");
                assert_eq!(out[0].im, (want * 7) as f64, "slot {t}");
            }
        }
    }

    /// The point of the whole file: a real low tone that has been through a
    /// 90-degree shift comes back at its own level, where the plain reading
    /// loses most of it.
    #[test]
    fn a_low_tone_survives_the_round_trip_that_the_plain_reading_loses() {
        for hz in [25.0, 40.0] {
            let mut level = Vec::new();
            for carry in [Carry::Shifted { plus: false }, Carry::Flat { plus: false }] {
                // shift the tone by +90 degrees, the way an encoder would,
                // then ask the decoder to take the shift back out
                let mut ana = Analysis::new();
                let mut quad = Quadrature::new(carry);
                let mut syn = Synthesis::new();
                let mut slot = [Complex::default(); BANDS];
                let mut back = [Complex::default(); BANDS];
                let mut pcm = [0.0f64; BANDS];
                let (mut energy, mut count) = (0.0f64, 0usize);
                for t in 0..600 {
                    let mut chunk = [0.0f64; BANDS];
                    for (i, c) in chunk.iter_mut().enumerate() {
                        let n = (t * BANDS + i) as f64;
                        // the shifted tone: cos becomes sin
                        *c = (2.0 * PI * hz * n / 48_000.0).sin();
                    }
                    ana.step(&chunk, &mut slot);
                    quad.step(&slot, &mut back);
                    syn.step(&back, &mut pcm);
                    if t > 40 {
                        for v in pcm {
                            energy += v * v;
                            count += 1;
                        }
                    }
                }
                level.push((2.0 * energy / count as f64).sqrt());
            }
            assert!(
                (level[0] - 1.0).abs() < 0.15,
                "{hz} Hz corrected amplitude {:.3}",
                level[0]
            );
            assert!(
                level[1] < 0.6 * level[0],
                "{hz} Hz the plain reading kept {:.3} of {:.3}",
                level[1],
                level[0]
            );
        }
    }
}
