//! Enhanced coupling synthesis (ATSC A/52:2018 clause E.3.5.5).
//!
//! Standard coupling sends one magnitude per band per channel, so every
//! coupled channel comes out of the shared coupling channel in phase. Enhanced
//! coupling adds a phase angle and a chaos measure, and rebuilds each channel
//! as a complex rotation of a carrier. Rotating a phase requires a signal
//! without time-domain aliasing, so the carrier is reconstructed from the
//! enhanced coupling coefficients of the previous, current and next blocks
//! before it is transformed back to the frequency domain (clause E.3.5.5.1).
//! A block therefore cannot be finished until the following one is parsed, and
//! the following block may sit in the next syncframe.
//!
//! ETSI TS 102 366 V1.4.1 does not describe any of this: its clause E.2.5.5
//! reduces enhanced coupling to a real-valued gain. The full process is in
//! ETSI TS 102 366 V1.2.1 clause E.2.5.5 and in ATSC A/52:2012 and A/52:2018
//! clause E.3.5.5, which agree word for word. See `docs/eac3.md`.

use std::f64::consts::PI;

use crate::frame::{EcplBlock, MAX_FBW, N, Noise};
use crate::imdct::{Complex, Dft512, Imdct};
use crate::tables::{ECPL_AMP_EXP, ECPL_AMP_MANT, ECPL_ANGLE_TAB, ECPL_CHAOS_TAB};

/// Transform size of the carrier reconstruction (clause E.3.5.5.1).
const NN: usize = 2 * N;

/// The overlap-add of clause E.3.5.5.1 step 3 is printed without the factor
/// two that the main body's overlap-add carries (clause 6.9.4), which leaves
/// the whole chain 6,02 dB down. The identity test
/// `unit_amplitude_and_zero_angle_return_the_carrier` measures it.
const OLA_GAIN: f64 = 2.0;

/// Deferred enhanced coupling synthesis; one per decoder.
#[derive(Debug)]
pub struct Synth {
    dft: Dft512,
    /// The previous block's windowed inverse transform; all zero when the
    /// previous block carried no enhanced coupling.
    prev: [f64; NN],
    /// `rand_notrans[ch][bin]`: uniform on [-1, 1), unique per bin and
    /// channel, drawn once and never redrawn (clause E.3.5.5.3).
    notrans: [[f64; N]; MAX_FBW],
    /// The source of `rand_trans[ch][bnd]`, redrawn every block. Kept apart
    /// from the dither of clause 6.3.4 so the two do not interleave.
    trans: Noise,
    /// `y[k] = cos(2 pi (N/4 + 1/2) (k + 1/2) / N)` of clause E.3.5.5.4.
    y: [f64; N],
}

impl Default for Synth {
    fn default() -> Self {
        Self::new()
    }
}

impl Synth {
    /// Builds the tables and draws the fixed de-correlation values.
    #[must_use]
    pub fn new() -> Self {
        let mut fixed = Noise::new(0x9E37_79B9);
        let mut notrans = [[0.0; N]; MAX_FBW];
        for ch in &mut notrans {
            for v in ch.iter_mut() {
                *v = fixed.uniform();
            }
        }
        let mut y = [0.0; N];
        for (k, v) in y.iter_mut().enumerate() {
            *v = (2.0 * PI * (NN as f64 / 4.0 + 0.5) * (k as f64 + 0.5) / NN as f64).cos();
        }
        Self {
            dft: Dft512::new(),
            prev: [0.0; NN],
            notrans,
            trans: Noise::new(0x85EB_CA6B),
            y,
        }
    }

    /// Forgets the previous block (after a splice or an error).
    pub fn reset(&mut self) {
        self.prev = [0.0; NN];
    }

    /// Records that the block just decoded carried no enhanced coupling, so
    /// the next block's carrier sees a zero predecessor (clause E.3.5.5.1).
    pub fn skip_block(&mut self) {
        self.prev = [0.0; NN];
    }

    /// Fills the coupled channels of one block from its enhanced coupling
    /// data. `next` is the following block's enhanced coupling coefficients,
    /// all zero when that block uses none or the stream ends.
    pub fn block(
        &mut self,
        imdct: &Imdct,
        ecpl: &EcplBlock,
        next: &[f64; N],
        coeffs: &mut [[f64; N]],
    ) {
        // 1-2: the windowed inverse transforms of the three blocks.
        let mut cur = [0.0f64; NN];
        let mut nxt = [0.0f64; NN];
        imdct.windowed(&ecpl.coeffs, false, &mut cur);
        imdct.windowed(next, false, &mut nxt);

        // 3: overlap and add into one aliasing-free block of NN samples.
        let mut pcm = [0.0f64; NN];
        for n in 0..N {
            pcm[n] = OLA_GAIN * (self.prev[N + n] + cur[n]);
            pcm[N + n] = OLA_GAIN * (cur[N + n] + nxt[n]);
        }
        self.prev = cur;

        // 4: window again and rotate onto the oddly stacked filterbank.
        let w = imdct.window();
        let mut seg = [Complex::default(); NN];
        for n in 0..N {
            let a = PI * n as f64 / NN as f64;
            let b = PI * (N + n) as f64 / NN as f64;
            let lo = pcm[n] * w[n];
            let hi = pcm[N + n] * w[N - n - 1];
            seg[n] = Complex {
                re: lo * a.cos(),
                im: -lo * a.sin(),
            };
            seg[N + n] = Complex {
                re: hi * b.cos(),
                im: -hi * b.sin(),
            };
        }

        // 5: the complex carrier.
        self.dft.forward(&mut seg);

        // E.3.5.5.2-4: one rotation of the carrier per coupled channel.
        let nbnd = ecpl.bands.len();
        let mut rand_trans = [0.0f64; 22];
        for v in rand_trans.iter_mut().take(nbnd) {
            *v = self.trans.uniform();
        }
        let mut amp = [0.0f64; 22];
        let mut chaos = [0.0f64; 22];
        let mut angle_bnd = [0.0f64; 22];
        let mut angle_bin = [0.0f64; N];
        for (ch, slot) in ecpl.chans.iter().enumerate() {
            let Some(c) = slot else { continue };
            let first = ch == ecpl.first_ch;
            for bnd in 0..nbnd {
                let code = usize::from(c.amp[bnd]);
                let mut a = if code == 31 {
                    0.0
                } else {
                    f64::from(ECPL_AMP_MANT[code]) / 32.0 / f64::from(1u32 << ECPL_AMP_EXP[code])
                };
                chaos[bnd] = if first {
                    0.0
                } else {
                    ECPL_CHAOS_TAB[usize::from(c.chaos[bnd])]
                };
                if !c.transient && !first {
                    a *= 1.0 + 0.38 * chaos[bnd];
                }
                amp[bnd] = a;
                angle_bnd[bnd] = if first {
                    0.0
                } else {
                    ECPL_ANGLE_TAB[usize::from(c.angle[bnd])]
                };
            }
            self.spread_angles(ecpl, &angle_bnd[..nbnd], &mut angle_bin);
            for (bnd, &(lo, hi)) in ecpl.bands.iter().enumerate() {
                for bin in lo..hi {
                    let r = if c.transient {
                        rand_trans[bnd]
                    } else {
                        self.notrans[ch][bin]
                    };
                    let mut ang = angle_bin[bin] + chaos[bnd] * r;
                    if ang < -1.0 {
                        ang += 2.0;
                    } else if ang >= 1.0 {
                        ang -= 2.0;
                    }
                    let (s, co) = (PI * ang).sin_cos();
                    let z = seg[bin];
                    let zr = amp[bnd] * (z.re * co - z.im * s);
                    let zi = amp[bnd] * (z.im * co + z.re * s);
                    coeffs[ch][bin] = -2.0 * (self.y[bin] * zr + self.y[N - 1 - bin] * zi);
                }
            }
        }
    }

    /// Bin angles from band angles (clause E.3.5.5.3). Without interpolation
    /// the band value covers its bins; with it the values are interpolated
    /// linearly between band centres, wrapping at +/-1. A region of one band
    /// has no pair to interpolate between, so it stays flat.
    fn spread_angles(&self, ecpl: &EcplBlock, angle_bnd: &[f64], out: &mut [f64; N]) {
        let bands = &ecpl.bands;
        if !ecpl.angle_interp || bands.len() < 2 {
            for (bnd, &(lo, hi)) in bands.iter().enumerate() {
                out[lo..hi].fill(angle_bnd[bnd]);
            }
            return;
        }
        let base = ecpl.start;
        let width = |b: usize| bands[b].1 - bands[b].0;
        let wrap = |mut v: f64| {
            while v > 1.0 {
                v -= 2.0;
            }
            while v < -1.0 {
                v += 2.0;
            }
            v
        };
        let mut bin = 0usize;
        let mut y = 0.0f64;
        let mut slope = 0.0f64;
        let mut nbins_curr = width(0);
        for bnd in 1..bands.len() {
            let nbins_prev = width(bnd - 1);
            nbins_curr = width(bnd);
            let angle_prev = angle_bnd[bnd - 1];
            let mut angle_curr = angle_bnd[bnd];
            while angle_curr - angle_prev > 1.0 {
                angle_curr -= 2.0;
            }
            while angle_prev - angle_curr > 1.0 {
                angle_curr += 2.0;
            }
            slope = (angle_curr - angle_prev) / ((nbins_curr + nbins_prev) as f64 / 2.0);
            if bnd == 1 && nbins_prev > 1 {
                // the lower half of the first band, walking downwards
                let (mut yv, first_bin) = if nbins_prev.is_multiple_of(2) {
                    (angle_prev - slope / 2.0, nbins_prev / 2 - 1)
                } else {
                    (angle_prev - slope, (nbins_prev - 3) / 2)
                };
                for b in (0..=first_bin).rev() {
                    out[base + b] = wrap(yv);
                    yv -= slope;
                }
                bin = first_bin + 1;
            }
            let count = if nbins_prev.is_multiple_of(2) {
                y = angle_prev + slope / 2.0;
                nbins_curr / 2 + nbins_prev / 2
            } else {
                y = angle_prev;
                nbins_curr / 2 + nbins_prev.div_ceil(2)
            };
            for _ in 0..count {
                out[base + bin] = wrap(y);
                y += slope;
                bin += 1;
            }
        }
        let count = if nbins_curr.is_multiple_of(2) {
            nbins_curr / 2
        } else {
            nbins_curr / 2 + 1
        };
        for _ in 0..count {
            out[base + bin] = wrap(y);
            y += slope;
            bin += 1;
        }
    }
}

#[cfg(test)]
#[path = "ecpl_tests.rs"]
mod tests;
