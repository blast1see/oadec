//! The 64-band complex quadrature mirror filter bank of ETSI TS 103 420
//! clause 7: analysis (7.2) and synthesis (7.3) with the 640-tap prototype
//! window of clause 7.4.
//!
//! One analysis step turns 64 time-domain samples into 64 complex subband
//! samples (one time slot); one synthesis step turns a time slot back into
//! 64 samples. The pair delays the signal by [`DELAY`] samples, which the
//! unit test measures.

use std::f64::consts::PI;

use crate::qmf_window::QWIN;

/// Subbands and samples per time slot.
pub const BANDS: usize = 64;
/// Prototype length.
pub const LENGTH: usize = 640;
/// Delay of analysis followed by synthesis, in samples (measured by the
/// reconstruction test).
pub const DELAY: usize = LENGTH - BANDS + 1;

/// A complex subband sample.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    /// `self * s` for a real scalar.
    #[inline]
    #[must_use]
    pub fn scale(self, s: f64) -> Self {
        Self {
            re: self.re * s,
            im: self.im * s,
        }
    }

    /// Sum.
    #[inline]
    #[must_use]
    pub fn add(self, o: Self) -> Self {
        Self {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
}

/// One channel's analysis filter state.
#[derive(Debug, Clone)]
pub struct Analysis {
    buf: [f64; LENGTH],
    /// `exp(i pi (sb + 1/2)(j - 1/2) / 64)` for `sb` in `0..64`, `j` in `0..128`.
    twiddle: Vec<Complex>,
}

impl Default for Analysis {
    fn default() -> Self {
        Self::new()
    }
}

impl Analysis {
    /// A filter with empty history.
    #[must_use]
    pub fn new() -> Self {
        let mut twiddle = Vec::with_capacity(BANDS * 2 * BANDS);
        for sb in 0..BANDS {
            for j in 0..2 * BANDS {
                let a = PI * (sb as f64 + 0.5) * (j as f64 - 0.5) / BANDS as f64;
                twiddle.push(Complex {
                    re: a.cos(),
                    im: a.sin(),
                });
            }
        }
        Self {
            buf: [0.0; LENGTH],
            twiddle,
        }
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.buf = [0.0; LENGTH];
    }

    /// Consumes 64 new samples and produces one time slot of 64 subbands.
    pub fn step(&mut self, pcm: &[f64; BANDS], out: &mut [Complex; BANDS]) {
        // 1. shift out 64 old samples
        self.buf.copy_within(0..LENGTH - BANDS, BANDS);
        // 2. shift in the new samples, most recent first
        for j in 0..BANDS {
            self.buf[j] = pcm[BANDS - 1 - j];
        }
        // 3./4. window and fold to 128 values
        let mut u = [0.0f64; 2 * BANDS];
        for (j, uj) in u.iter_mut().enumerate() {
            let mut acc = 0.0;
            let mut k = j;
            while k < LENGTH {
                acc += self.buf[k] * QWIN[k];
                k += 2 * BANDS;
            }
            *uj = acc;
        }
        // 5. modulate
        for sb in 0..BANDS {
            let tw = &self.twiddle[sb * 2 * BANDS..(sb + 1) * 2 * BANDS];
            let mut re = 0.0;
            let mut im = 0.0;
            for j in 0..2 * BANDS {
                re += u[j] * tw[j].re;
                im += u[j] * tw[j].im;
            }
            out[sb] = Complex { re, im };
        }
    }
}

/// One channel's synthesis filter state.
#[derive(Debug, Clone)]
pub struct Synthesis {
    buf: [f64; 2 * LENGTH],
    /// `exp(i pi/(4n) (2 sb + 1)(2 j - 4n + 1)) / n` for `sb` in `0..64`, `j` in `0..128`,
    /// stored as `(cos, -sin)` pairs.
    cos: Vec<f64>,
}

impl Default for Synthesis {
    fn default() -> Self {
        Self::new()
    }
}

impl Synthesis {
    /// A filter with empty history.
    #[must_use]
    pub fn new() -> Self {
        let n = BANDS as f64;
        let mut cos = Vec::with_capacity(BANDS * 2 * BANDS);
        let mut sin = Vec::with_capacity(BANDS * 2 * BANDS);
        for sb in 0..BANDS {
            for j in 0..2 * BANDS {
                // exp(i pi/(4n) (2 sb + 1)(2 j - 4n + 1)): the phase reference that
                // inverts the analysis of clause 7.2 with this prototype (the text of
                // clause 7.3 prints the term differently; see the reconstruction test)
                let a = PI / (4.0 * n) * (2.0 * sb as f64 + 1.0) * (2.0 * j as f64 - 4.0 * n + 1.0);
                cos.push(a.cos() / n);
                sin.push(a.sin() / n);
            }
        }
        // real(Q e^{ia}) = Q.re cos a - Q.im sin a ; store both interleaved
        let mut table = Vec::with_capacity(cos.len() * 2);
        for (c, s) in cos.iter().zip(&sin) {
            table.push(*c);
            table.push(-*s);
        }
        Self {
            buf: [0.0; 2 * LENGTH],
            cos: table,
        }
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.buf = [0.0; 2 * LENGTH];
    }

    /// Consumes one time slot and produces 64 output samples.
    pub fn step(&mut self, q: &[Complex; BANDS], pcm: &mut [f64; BANDS]) {
        // 1. shift by 2n
        self.buf.copy_within(0..2 * LENGTH - 2 * BANDS, 2 * BANDS);
        // 2. 128 new values
        for j in 0..2 * BANDS {
            let mut acc = 0.0;
            for sb in 0..BANDS {
                let t = (sb * 2 * BANDS + j) * 2;
                acc += q[sb].re * self.cos[t] + q[sb].im * self.cos[t + 1];
            }
            self.buf[j] = acc;
        }
        // 3./4. gather and window
        let mut w = [0.0f64; LENGTH];
        for j in 0..LENGTH / (2 * BANDS) {
            for sb in 0..BANDS {
                w[2 * BANDS * j + sb] = self.buf[4 * BANDS * j + sb] * QWIN[2 * BANDS * j + sb];
                w[2 * BANDS * j + BANDS + sb] =
                    self.buf[4 * BANDS * j + 3 * BANDS + sb] * QWIN[2 * BANDS * j + BANDS + sb];
            }
        }
        // 5. sum
        for ts in 0..BANDS {
            let mut acc = 0.0;
            for j in 0..LENGTH / BANDS {
                acc += w[BANDS * j + ts];
            }
            pcm[ts] = acc;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_signal(n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| {
                let t = i as f64;
                0.5 * (t * 0.0137).sin()
                    + 0.3 * (t * 0.211 + 0.4).cos()
                    + 0.2 * (t * 1.9).sin()
                    + 0.1 * (t * 2.9 + 1.0).cos()
            })
            .collect()
    }

    #[test]
    fn analysis_then_synthesis_reconstructs_with_a_fixed_delay() {
        let n = 64 * 400;
        let x = test_signal(n);
        let mut ana = Analysis::new();
        let mut syn = Synthesis::new();
        let mut y = Vec::with_capacity(n);
        let mut slot = [Complex::default(); BANDS];
        let mut out = [0.0f64; BANDS];
        for chunk in x.chunks(BANDS) {
            let mut inp = [0.0f64; BANDS];
            inp.copy_from_slice(chunk);
            ana.step(&inp, &mut slot);
            syn.step(&slot, &mut out);
            y.extend_from_slice(&out);
        }
        // find the delay by cross-correlation over a steady-state stretch
        let start = 64 * 40;
        let span = 64 * 200;
        let mut best = (0usize, f64::MIN);
        for d in 0..2 * LENGTH {
            let mut c = 0.0;
            for i in start..start + span {
                c += x[i] * y[i + d];
            }
            if c > best.1 {
                best = (d, c);
            }
        }
        assert_eq!(best.0, DELAY, "measured delay");
        let mut worst = 0.0f64;
        let mut energy = 0.0f64;
        let mut err = 0.0f64;
        for i in start..start + span {
            let e = (y[i + DELAY] - x[i]).abs();
            worst = worst.max(e);
            energy += x[i] * x[i];
            err += e * e;
        }
        let snr = 10.0 * (energy / err).log10();
        // the prototype gives near-perfect reconstruction, about 78 dB
        assert!(
            snr > 75.0,
            "reconstruction SNR {snr:.1} dB, worst {worst:.2e}"
        );
    }

    #[test]
    fn a_tone_lands_in_its_subband() {
        // 64 subbands over 0..24 kHz at 48 kHz: subband k spans k*375 Hz .. (k+1)*375 Hz
        let f = 2_000.0; // subband 5 (1875..2250 Hz)
        let n = 64 * 60;
        let x: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * f * i as f64 / 48_000.0).sin())
            .collect();
        let mut ana = Analysis::new();
        let mut slot = [Complex::default(); BANDS];
        let mut power = [0.0f64; BANDS];
        for chunk in x.chunks(BANDS) {
            let mut inp = [0.0f64; BANDS];
            inp.copy_from_slice(chunk);
            ana.step(&inp, &mut slot);
            for (p, s) in power.iter_mut().zip(&slot) {
                *p += s.re * s.re + s.im * s.im;
            }
        }
        let peak = power.iter().cloned().fold(0.0, f64::max);
        let k = power.iter().position(|&p| p == peak).unwrap();
        assert_eq!(k, 5);
        // two bands away the leakage is far below the peak
        assert!(
            power[7] < peak * 1e-2 && power[3] < peak * 1e-2,
            "{:?}",
            &power[..10]
        );
        assert!(power[20] < peak * 1e-6 && power[60] < peak * 1e-6);
    }
}
