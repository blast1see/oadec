//! The 90-degree phase shift of `joc_dmx_config_idx` 3 and 4.
//!
//! Table 47 of ETSI TS 103 420 names two downmix configurations "5.X with
//! 90 degree phase shift" and then the standard never says what the shift is
//! or how a decoder takes it back out. In the quadrature mirror filter
//! domain the obvious reading is to multiply the surround pair by -j, and
//! for every subband but the lowest that is right: against the object output
//! of the Dolby decoder the difference above the lowest subband sits on the
//! dither floor.
//!
//! Subband 0 is different. It spans 0 to 375 Hz, so it straddles direct
//! current and its complex signal is not analytic there: the image of the
//! negative frequencies of a real signal falls inside the passband, only a
//! few decibels down near direct current. A single rotation by -j turns
//! that image the wrong way and the two halves cancel instead of adding, so
//! the reconstructed objects lose their bottom octaves.
//!
//! What the Dolby decoder does there was measured (`docs/joc.md`). The
//! lowest subband is a signal of one value a time slot, and the decoder puts
//! it through a filter across time slots: 21 complex taps, the same either
//! side of the middle one, [`OPERATOR`]. A decode is linear in those taps, so
//! they were solved for by least squares between what the Dolby decoder
//! writes for a stream and what this one writes with one tap set at a time
//! (`tools/gen_joc_quadrature.py`). Fourteen streams of both configurations
//! give the same taps, they leave the lowest band of the objects 96 to 130 dB
//! from Dolby's, and nothing shorter or simpler comes near: 62 dB at the
//! median with 19 taps, 13 dB with real ones.
//!
//! From 169 Hz up the operator is the turn by -j of the other subbands, to
//! four parts in a thousand, on the image of the negative frequencies as on
//! the signal. Below, it is no turn: 1,0022 - 2,4512j at direct current.
//! Signal and image meet again in the synthesis, and what that makes of a
//! real tone is a level that stays, within a third of a per cent, and a phase
//! that goes from nothing at direct current through 45 degrees at 25 Hz to
//! the quarter turn at 80 Hz.
//!
//! The operator reaches ten slots either side of its middle, so it delays by
//! ten slots, and every subband of every channel is held back by as much to
//! stay in step with it.

use crate::qmf::{BANDS, Complex};

/// Time slots the operator on the lowest subband reaches either side of its
/// middle, which is what it delays by. Every downmix channel is held back by
/// this much so that the subbands stay aligned, at a cost of
/// `LOW_DELAY * BANDS` samples of decoder delay.
pub const LOW_DELAY: usize = 10;

/// Taps of the operator on the lowest subband.
pub const LOW_TAPS: usize = 2 * LOW_DELAY + 1;

/// Time slots between the side information of a frame and the subband
/// samples the analysis bank hands over for it.
///
/// The standard says nothing about this: clause 6.6.6 pairs `x[ch][ts][sb]`
/// with `joc_mix_mtx_interp[obj][ch][ts][sb]` and leaves it to the reader to
/// notice that the analysis bank the decoder runs is not the one the encoder
/// ran. Measured against the object output of the Dolby decoder on three
/// titles from three encoders, the matrix of slot `ts` belongs to the
/// subband samples the analysis produces `MATRIX_ALIGN` slots earlier; the
/// optimum is sharp and symmetric: sweeping the offset against Dolby's objects
/// gave 37,2 dB at 0, 47,2 at 9, 48,2 at 10, 47,2 at 11 and 38,5 at 18, so it
/// is about 1 dB per slot on the mean over objects and 11 dB better than
/// reading the clause literally.
///
/// That was measured before the operator was, and the operator accounts for
/// it: ten slots is [`LOW_DELAY`], what the filter on the lowest subband
/// delays by, and it is what a decoder comes to that applies the matrix of a
/// slot to what the filter hands over in that slot. So the samples are held
/// back by the filter and the matrices are not held back at all.
pub const MATRIX_ALIGN: usize = 10;

const fn tap(re: f64, im: f64) -> Complex {
    Complex { re, im }
}

/// The operator on the lowest subband of a channel that carries the phase
/// shift, from its middle tap outwards: `OPERATOR[k]` meets the slots `k`
/// either side of the one that comes out.
///
/// Measured, not derived: the taps fourteen streams give together, which a
/// stream gives alone to a few parts in ten million wherever the shifted
/// channels carry enough of its lowest band to say. No closed form was found
/// for them.
pub const OPERATOR: [Complex; LOW_DELAY + 1] = [
    tap(0.094_400_348_7, -1.195_874_689_9),
    tap(0.092_151_259_1, -0.183_449_414_4),
    tap(0.085_668_131_7, -0.151_365_486_3),
    tap(0.075_699_902_1, -0.111_551_384_9),
    tap(0.063_358_090_7, -0.075_144_639_4),
    tap(0.049_940_435_7, -0.047_519_703_3),
    tap(0.036_734_939_3, -0.028_654_400_5),
    tap(0.024_838_790_9, -0.016_336_535_8),
    tap(0.015_025_257_7, -0.008_531_010_4),
    tap(0.007_678_160_7, -0.003_846_888_7),
    tap(0.002_799_361_7, -0.001_248_450_3),
];

/// The whole operator, a tap a time slot, as [`Quadrature::with_operator`]
/// takes one.
#[must_use]
pub fn operator() -> [Complex; LOW_TAPS] {
    std::array::from_fn(|i| OPERATOR[i.abs_diff(LOW_DELAY)])
}

/// What one downmix channel needs before the mixing matrix is applied.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Carry {
    /// The channel carries no phase shift, so it is only held back.
    Plain,
    /// The channel carries the 90-degree phase shift of table 47. It is
    /// rotated back by -j, or by +j when `plus`, and its lowest subband is
    /// put through the operator the Dolby decoder puts it through.
    Shifted { plus: bool },
    /// The same rotation applied flat to every subband, the lowest one
    /// included. This is the plain reading of table 47 and what a decoder
    /// does when it takes subband 0 for an analytic signal; kept so that the
    /// difference can be measured.
    Flat { plus: bool },
    /// The rotation applied to every subband but the lowest, which is left
    /// alone. With [`Carry::Flat`], one of the two readings the lowest
    /// subband was first bracketed between, and on its own no closer to
    /// Dolby; kept for the same reason.
    FlatAbove { plus: bool },
}

/// Holds one downmix channel back by the reach of the operator and, when the
/// channel carries the 90-degree phase shift, takes the shift back out.
#[derive(Debug)]
pub struct Quadrature {
    carry: Carry,
    /// The operator on the lowest subband: `taps[i]` meets the slot `i` back
    /// from the newest. Already turned the other way for a channel whose
    /// shift is taken out by +j.
    taps: Box<[Complex]>,
    /// Subband 0 of the last `taps.len()` slots, oldest first.
    low: Box<[Complex]>,
    /// Whole slots of the last `taps.len() / 2 + 1` time slots, a ring.
    hold: Box<[[Complex; BANDS]]>,
    at: usize,
}

impl Quadrature {
    /// With the operator the Dolby decoder applies, [`OPERATOR`].
    #[must_use]
    pub fn new(carry: Carry) -> Self {
        Self::with_operator(carry, &operator())
    }

    /// With an operator from outside, for measuring: `taps[i]` meets the slot
    /// `i` back from the newest, and the slot that comes out is the one its
    /// middle tap meets, so the channel is held back by half its length.
    /// Every channel of a stream is given the same one whether or not it
    /// carries the shift, because that hold is what keeps them in step. For
    /// a channel whose shift is taken out by +j the taps are conjugated: no
    /// stream seen asks for that direction, and the mirror image is what the
    /// symmetry of the two directions gives.
    ///
    /// # Panics
    ///
    /// When `taps` has no middle: an even number of them, or none.
    #[must_use]
    pub fn with_operator(carry: Carry, taps: &[Complex]) -> Self {
        assert!(
            taps.len() % 2 == 1,
            "an operator of {} taps has no middle",
            taps.len()
        );
        let mirrored = matches!(carry, Carry::Shifted { plus: true });
        Self {
            carry,
            taps: taps
                .iter()
                .map(|t| if mirrored { tap(t.re, -t.im) } else { *t })
                .collect(),
            low: vec![Complex::default(); taps.len()].into_boxed_slice(),
            hold: vec![[Complex::default(); BANDS]; taps.len() / 2 + 1].into_boxed_slice(),
            at: 0,
        }
    }

    /// Time slots a slot comes out late by: the reach of the operator.
    #[must_use]
    pub fn delay(&self) -> usize {
        self.hold.len() - 1
    }

    /// Forgets the history, as at a splice.
    pub fn reset(&mut self) {
        self.low.fill(Complex::default());
        for slot in &mut self.hold {
            *slot = [Complex::default(); BANDS];
        }
        self.at = 0;
    }

    /// Takes the subband samples of one time slot and writes the samples of
    /// the slot [`Quadrature::delay`] slots back, ready for the mixing matrix.
    pub fn step(&mut self, slot: &[Complex; BANDS], out: &mut [Complex; BANDS]) {
        self.low.rotate_left(1);
        let newest = self.low.len() - 1;
        self.low[newest] = slot[0];
        self.hold[self.at] = *slot;
        self.at = (self.at + 1) % self.hold.len();
        // the entry the ring is about to overwrite is as old as the hold is long
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
                for (t, x) in self.taps.iter().zip(self.low.iter().rev()) {
                    acc = acc.plus(t.times(*x));
                }
                out[0] = acc;
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

    /// What comes out of the lowest subband, slot by slot, of a channel that
    /// was given one sample in its first slot.
    fn impulse(carry: Carry, slots: usize) -> Vec<Complex> {
        let mut q = Quadrature::new(carry);
        let mut out = [Complex::default(); BANDS];
        (0..slots)
            .map(|t| {
                let mut slot = [Complex::default(); BANDS];
                if t == 0 {
                    slot[0] = Complex { re: 1.0, im: 0.0 };
                }
                q.step(&slot, &mut out);
                out[0]
            })
            .collect()
    }

    /// A slot whose every subband says which slot and which subband it is.
    fn marked(t: usize) -> [Complex; BANDS] {
        std::array::from_fn(|i| Complex {
            re: (t * 100 + i) as f64,
            im: (t * 7 + i) as f64 + 0.5,
        })
    }

    #[test]
    fn the_operator_is_twenty_one_slots_long_and_the_same_either_side_of_its_middle() {
        assert_eq!(LOW_TAPS, 21);
        assert_eq!(LOW_DELAY, 10);
        let seen = impulse(Carry::Shifted { plus: false }, 40);
        for t in 0..LOW_TAPS {
            assert_eq!(seen[t], seen[LOW_TAPS - 1 - t], "slot {t}");
            assert!(seen[t].re != 0.0 && seen[t].im != 0.0, "slot {t} is empty");
        }
        for (t, v) in seen.iter().enumerate().skip(LOW_TAPS) {
            assert_eq!((v.re, v.im), (0.0, 0.0), "slot {t}, past the operator");
        }
    }

    /// The two ends of the response, which are sums over the whole table: all
    /// of it at direct current, every other tap against the rest at the top of
    /// the subband. Both are what the measurement gave, so a tap that moves
    /// moves one of them.
    #[test]
    fn the_response_at_both_ends_of_the_subband_is_the_measured_one() {
        let (g, p) = response(0.0, Carry::Shifted { plus: false });
        let (re, im) = (g * p.to_radians().cos(), g * p.to_radians().sin());
        assert!((re - 1.002_189_0).abs() < 2e-7, "real part at 0 Hz {re:.9}");
        assert!(
            (im + 2.451_170_5).abs() < 2e-7,
            "imaginary part at 0 Hz {im:.9}"
        );
        let (g, p) = response(375.0, Carry::Shifted { plus: false });
        let (re, im) = (g * p.to_radians().cos(), g * p.to_radians().sin());
        assert!(
            (re - 0.000_954_8).abs() < 2e-7,
            "real part at 375 Hz {re:.9}"
        );
        assert!(
            (im + 1.000_354_8).abs() < 2e-7,
            "imaginary part at 375 Hz {im:.9}"
        );
    }

    /// From 0,45 of the way up the subband, 169 Hz, the operator is the turn
    /// by -j that the other subbands get, to four parts in a thousand, and it
    /// is that on the image of the negative frequencies as well.
    #[test]
    fn away_from_direct_current_the_operator_is_the_quarter_turn_of_the_other_subbands() {
        for hz in [
            170.0, 200.0, 280.0, 350.0, 375.0, -170.0, -200.0, -280.0, -350.0,
        ] {
            let (g, p) = response(hz, Carry::Shifted { plus: false });
            assert!((g - 1.0).abs() < 4e-3, "gain at {hz} Hz {g:.5}");
            assert!((p + 90.0).abs() < 0.2, "phase at {hz} Hz {p:.3} degrees");
        }
    }

    #[test]
    fn the_other_direction_is_the_mirror_image() {
        for hz in [0.0, 40.0, 200.0, -40.0, -200.0] {
            let (g, p) = response(hz, Carry::Shifted { plus: false });
            let (mirror_g, mirror_p) = response(hz, Carry::Shifted { plus: true });
            assert!((g - mirror_g).abs() < 1e-12, "gain at {hz} Hz");
            assert!((p + mirror_p).abs() < 1e-9, "phase at {hz} Hz");
            assert!(p < -60.0, "{hz} Hz is turned back by {p:.1} degrees");
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
        assert_eq!(q.delay(), LOW_DELAY);
        let mut out = [Complex::default(); BANDS];
        for t in 0..64 {
            q.step(&marked(t), &mut out);
            if t >= LOW_DELAY {
                assert_eq!(out, marked(t - LOW_DELAY), "slot {t}");
            } else {
                assert_eq!(out, [Complex::default(); BANDS], "slot {t}");
            }
        }
    }

    #[test]
    fn the_subbands_above_the_lowest_are_held_back_with_it_and_turned() {
        for plus in [false, true] {
            let mut q = Quadrature::new(Carry::Shifted { plus });
            let mut out = [Complex::default(); BANDS];
            for t in 0..40 {
                q.step(&marked(t), &mut out);
                if t < LOW_DELAY {
                    continue;
                }
                let was = marked(t - LOW_DELAY);
                for sb in 1..BANDS {
                    // by -j, or by +j
                    let want = if plus {
                        Complex {
                            re: -was[sb].im,
                            im: was[sb].re,
                        }
                    } else {
                        Complex {
                            re: was[sb].im,
                            im: -was[sb].re,
                        }
                    };
                    assert_eq!(out[sb], want, "slot {t}, subband {sb}, plus {plus}");
                }
            }
        }
    }

    #[test]
    fn an_operator_from_outside_is_applied_tap_by_tap_as_it_is_written() {
        // three slots back, times 2 - 3j, and nothing else
        let mut taps = [Complex::default(); LOW_TAPS];
        taps[3] = Complex { re: 2.0, im: -3.0 };
        for plus in [false, true] {
            let mut q = Quadrature::with_operator(Carry::Shifted { plus }, &taps);
            let mut out = [Complex::default(); BANDS];
            for t in 0..40 {
                q.step(&marked(t), &mut out);
                let want = if t < 3 {
                    Complex::default()
                } else {
                    // the mirror image for the other direction
                    marked(t - 3)[0].times(Complex {
                        re: 2.0,
                        im: if plus { 3.0 } else { -3.0 },
                    })
                };
                assert_eq!(out[0], want, "slot {t}, plus {plus}");
            }
        }
        // a channel that carries no shift is not put through it
        let mut q = Quadrature::with_operator(Carry::Plain, &taps);
        let mut out = [Complex::default(); BANDS];
        for t in 0..40 {
            q.step(&marked(t), &mut out);
        }
        assert_eq!(out, marked(39 - LOW_DELAY));
    }

    #[test]
    fn a_longer_operator_holds_every_subband_back_by_its_own_reach() {
        // the operator there is, with an empty tap either side: the same
        // filter a slot later
        let mut longer = vec![Complex::default()];
        longer.extend(operator());
        longer.push(Complex::default());
        for carry in [Carry::Plain, Carry::Shifted { plus: false }] {
            let mut usual = Quadrature::new(carry);
            let mut held = Quadrature::with_operator(carry, &longer);
            assert_eq!(held.delay(), LOW_DELAY + 1);
            let mut was = [Complex::default(); BANDS];
            let mut out = [Complex::default(); BANDS];
            let mut late = [Complex::default(); BANDS];
            for t in 0..60 {
                usual.step(&marked(t), &mut out);
                held.step(&marked(t), &mut late);
                assert_eq!(late, was, "slot {t}, {carry:?}");
                was = out;
            }
        }
    }

    #[test]
    #[should_panic(expected = "an operator of 20 taps has no middle")]
    fn an_operator_without_a_middle_is_refused() {
        let _ = Quadrature::with_operator(Carry::Plain, &[Complex::default(); 20]);
    }

    #[test]
    fn a_channel_that_is_reset_is_a_new_one() {
        for carry in [Carry::Plain, Carry::Shifted { plus: false }] {
            let mut used = Quadrature::new(carry);
            let mut out = [Complex::default(); BANDS];
            // not a whole number of turns of the ring
            for t in 0..37 {
                used.step(&marked(t), &mut out);
            }
            used.reset();
            let mut new = Quadrature::new(carry);
            let mut want = [Complex::default(); BANDS];
            for t in 0..40 {
                used.step(&marked(t + 5), &mut out);
                new.step(&marked(t + 5), &mut want);
                assert_eq!(out, want, "slot {t}, {carry:?}");
            }
        }
    }

    /// A real tone through the analysis, the operator and the synthesis, as a
    /// gain and a phase against the tone that went in.
    fn real_tone(hz: usize, carry: Carry) -> (f64, f64) {
        let mut ana = Analysis::new();
        let mut quad = Quadrature::new(carry);
        let mut syn = Synthesis::new();
        let mut slot = [Complex::default(); BANDS];
        let mut back = [Complex::default(); BANDS];
        let mut pcm = [0.0f64; BANDS];
        let tone = |n: usize| (2.0 * PI * (hz * n) as f64 / 48_000.0).sin();
        let mut out = Vec::new();
        for t in 0..1_500 {
            let chunk: [f64; BANDS] = std::array::from_fn(|i| tone(t * BANDS + i));
            ana.step(&chunk, &mut slot);
            quad.step(&slot, &mut back);
            syn.step(&back, &mut pcm);
            out.extend(pcm);
        }
        // the two banks delay by 577 samples and the operator by its reach;
        // a second is a whole number of periods of any whole number of hertz
        let delay = 577 + LOW_DELAY * BANDS;
        let (mut top, mut bottom) = (Complex::default(), Complex::default());
        for n in 16_000..64_000 {
            let w = 2.0 * PI * (hz * n) as f64 / 48_000.0;
            let turn = Complex {
                re: w.cos(),
                im: -w.sin(),
            };
            top = top.plus(turn.scale(out[n + delay]));
            bottom = bottom.plus(turn.scale(tone(n)));
        }
        let norm = bottom.re * bottom.re + bottom.im * bottom.im;
        let ratio = top.times(Complex {
            re: bottom.re / norm,
            im: -bottom.im / norm,
        });
        (
            ratio.re.hypot(ratio.im),
            ratio.im.atan2(ratio.re).to_degrees(),
        )
    }

    /// What the operator makes of a real tone, which is where signal and
    /// image meet again: a level that stays and a phase that goes from
    /// nothing to the quarter turn. The figures are those of a model of the
    /// two banks written apart from this crate, with the measured taps.
    #[test]
    fn a_real_tone_keeps_its_level_and_is_turned_back_as_the_measured_operator_turns_it() {
        for (hz, gain, phase) in [
            (3, 1.002_23, -5.602),
            (25, 0.997_60, -44.980),
            (60, 1.000_87, -85.239),
            (80, 0.997_09, -90.426),
            (141, 0.997_96, -90.165),
            (300, 1.000_12, -89.961),
        ] {
            let (g, p) = real_tone(hz, Carry::Shifted { plus: false });
            assert!((g - gain).abs() < 2e-5, "gain at {hz} Hz {g:.6}");
            assert!((p - phase).abs() < 2e-3, "phase at {hz} Hz {p:.4} degrees");
        }
    }

    /// The point of the whole file: a real low tone that has been through a
    /// 90-degree shift comes back at its own level, where the plain reading
    /// loses most of it.
    #[test]
    fn a_low_tone_survives_the_round_trip_that_the_plain_reading_loses() {
        for hz in [25, 40] {
            let (kept, _) = real_tone(hz, Carry::Shifted { plus: false });
            let (plain, _) = real_tone(hz, Carry::Flat { plus: false });
            assert!(
                (kept - 1.0).abs() < 0.005,
                "{hz} Hz comes back at {kept:.4}"
            );
            assert!(
                plain < 0.6 * kept,
                "{hz} Hz the plain reading kept {plain:.3} of {kept:.3}"
            );
        }
    }
}
