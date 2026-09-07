//! The 64-band complex quadrature mirror filter bank of ETSI TS 103 420
//! clause 7: analysis (7.2) and synthesis (7.3) with the 640-tap prototype
//! window of clause 7.4.
//!
//! One analysis step turns 64 time-domain samples into 64 complex subband
//! samples (one time slot); one synthesis step turns a time slot back into
//! 64 samples. The pair delays the signal by [`DELAY`] samples, which the
//! unit test measures. The modulations are computed with a 128-point FFT;
//! a test checks them against the direct sums of the specification.
//!
//! The synthesis uses the phase term of the matrix equation of clause 7.3,
//! `(j - 2n + 1/2)`; the pseudo-code of the same clause prints
//! `(2j - 2n - 1)`, which does not invert the analysis (0.2 dB instead of
//! 78 dB reconstruction with the QWIN prototype).

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
    pub fn plus(self, o: Self) -> Self {
        Self {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }

    /// Product.
    #[inline]
    #[must_use]
    pub fn times(self, o: Self) -> Self {
        Self {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }

    #[inline]
    fn minus(self, o: Self) -> Self {
        Self {
            re: self.re - o.re,
            im: self.im - o.im,
        }
    }
}

/// Radix-2 complex inverse DFT (positive exponent, no scaling) of 128 points.
#[derive(Debug, Clone)]
struct Idft128 {
    twiddles: [Complex; 64],
    rev: [u8; 128],
}

impl Idft128 {
    fn new() -> Self {
        let mut twiddles = [Complex::default(); 64];
        for (k, t) in twiddles.iter_mut().enumerate() {
            let a = 2.0 * PI * k as f64 / 128.0;
            *t = Complex {
                re: a.cos(),
                im: a.sin(),
            };
        }
        let mut rev = [0u8; 128];
        for (i, r) in rev.iter_mut().enumerate() {
            *r = (i as u8).reverse_bits() >> 1;
        }
        Self { twiddles, rev }
    }

    fn run(&self, data: &mut [Complex; 128]) {
        for i in 0..128 {
            let j = usize::from(self.rev[i]);
            if j > i {
                data.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= 128 {
            let step = 128 / len;
            for start in (0..128).step_by(len) {
                for k in 0..len / 2 {
                    let w = self.twiddles[k * step];
                    let a = data[start + k];
                    let b = data[start + k + len / 2].times(w);
                    data[start + k] = a.plus(b);
                    data[start + k + len / 2] = a.minus(b);
                }
            }
            len *= 2;
        }
    }
}

/// One channel's analysis filter state.
#[derive(Debug, Clone)]
pub struct Analysis {
    buf: [f64; LENGTH],
    idft: Idft128,
    /// `exp(i pi (j - 1/2) / 128)` for `j` in `0..128`.
    pre: [Complex; 128],
    /// `exp(-i pi sb / 128)` for `sb` in `0..64`.
    post: [Complex; BANDS],
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
        let mut pre = [Complex::default(); 128];
        for (j, p) in pre.iter_mut().enumerate() {
            let a = PI * (j as f64 - 0.5) / 128.0;
            *p = Complex {
                re: a.cos(),
                im: a.sin(),
            };
        }
        let mut post = [Complex::default(); BANDS];
        for (sb, p) in post.iter_mut().enumerate() {
            let a = -PI * sb as f64 / 128.0;
            *p = Complex {
                re: a.cos(),
                im: a.sin(),
            };
        }
        Self {
            buf: [0.0; LENGTH],
            idft: Idft128::new(),
            pre,
            post,
        }
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.buf = [0.0; LENGTH];
    }

    /// Consumes 64 new samples and produces one time slot of 64 subbands:
    /// `Q[sb] = sum_j u[j] exp(i pi (sb + 1/2)(j - 1/2) / 64)`.
    pub fn step(&mut self, pcm: &[f64; BANDS], out: &mut [Complex; BANDS]) {
        // 1. shift out 64 old samples
        self.buf.copy_within(0..LENGTH - BANDS, BANDS);
        // 2. shift in the new samples, most recent first
        for j in 0..BANDS {
            self.buf[j] = pcm[BANDS - 1 - j];
        }
        // 3./4. window and fold to 128 values, with the half-band pre-twist
        let mut v = [Complex::default(); 128];
        for (j, vj) in v.iter_mut().enumerate() {
            let mut acc = 0.0;
            let mut k = j;
            while k < LENGTH {
                acc += self.buf[k] * QWIN[k];
                k += 2 * BANDS;
            }
            *vj = self.pre[j].scale(acc);
        }
        // 5. modulate: exp(i pi sb (j - 1/2) / 64) = exp(2 pi i sb j / 128) exp(-i pi sb / 128)
        self.idft.run(&mut v);
        for (sb, o) in out.iter_mut().enumerate() {
            *o = v[sb].times(self.post[sb]);
        }
    }
}

/// One channel's synthesis filter state.
#[derive(Debug, Clone)]
pub struct Synthesis {
    buf: [f64; 2 * LENGTH],
    idft: Idft128,
    /// `exp(-i pi 255 sb / 128)` for `sb` in `0..64`.
    pre: [Complex; BANDS],
    /// `exp(i pi (2j - 255) / 256) / 64` for `j` in `0..128`.
    post: [Complex; 128],
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
        let mut pre = [Complex::default(); BANDS];
        for (sb, p) in pre.iter_mut().enumerate() {
            let a = -PI * 255.0 * sb as f64 / 128.0;
            *p = Complex {
                re: a.cos(),
                im: a.sin(),
            };
        }
        let mut post = [Complex::default(); 128];
        for (j, p) in post.iter_mut().enumerate() {
            let a = PI * (2.0 * j as f64 - 255.0) / 256.0;
            *p = Complex {
                re: a.cos() / BANDS as f64,
                im: a.sin() / BANDS as f64,
            };
        }
        Self {
            buf: [0.0; 2 * LENGTH],
            idft: Idft128::new(),
            pre,
            post,
        }
    }

    /// Clears the history.
    pub fn reset(&mut self) {
        self.buf = [0.0; 2 * LENGTH];
    }

    /// Consumes one time slot and produces 64 output samples:
    /// `buf[j] = Re(sum_sb Q[sb]/n exp(i pi/(4n) (2 sb + 1)(2 j - 4n + 1)))`.
    pub fn step(&mut self, q: &[Complex; BANDS], pcm: &mut [f64; BANDS]) {
        // 1. shift by 2n
        self.buf.copy_within(0..2 * LENGTH - 2 * BANDS, 2 * BANDS);
        // 2. 128 new values through the inverse DFT
        let mut v = [Complex::default(); 128];
        for (sb, (vv, qq)) in v.iter_mut().zip(q.iter()).enumerate() {
            *vv = qq.times(self.pre[sb]);
        }
        self.idft.run(&mut v);
        for (j, b) in self.buf[..2 * BANDS].iter_mut().enumerate() {
            let t = v[j].times(self.post[j]);
            *b = t.re;
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
        for (ts, p) in pcm.iter_mut().enumerate() {
            let mut acc = 0.0;
            for j in 0..LENGTH / BANDS {
                acc += w[BANDS * j + ts];
            }
            *p = acc;
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

    /// The analysis modulation as the direct double sum of clause 7.2.
    fn direct_analysis(buf: &[f64; LENGTH]) -> [Complex; BANDS] {
        let mut u = [0.0f64; 2 * BANDS];
        for (j, uj) in u.iter_mut().enumerate() {
            let mut k = j;
            while k < LENGTH {
                *uj += buf[k] * QWIN[k];
                k += 2 * BANDS;
            }
        }
        let mut out = [Complex::default(); BANDS];
        for (sb, o) in out.iter_mut().enumerate() {
            for (j, uj) in u.iter().enumerate() {
                let a = PI * (sb as f64 + 0.5) * (j as f64 - 0.5) / BANDS as f64;
                o.re += uj * a.cos();
                o.im += uj * a.sin();
            }
        }
        out
    }

    #[test]
    #[allow(clippy::needless_range_loop, reason = "index used in the formula")]
    fn fft_modulation_equals_the_direct_sum() {
        let x = test_signal(64 * 12);
        let mut ana = Analysis::new();
        let mut slot = [Complex::default(); BANDS];
        for chunk in x.chunks(BANDS) {
            let mut inp = [0.0f64; BANDS];
            inp.copy_from_slice(chunk);
            ana.step(&inp, &mut slot);
        }
        let direct = direct_analysis(&ana.buf);
        let mut worst = 0.0f64;
        for sb in 0..BANDS {
            worst = worst.max((slot[sb].re - direct[sb].re).abs());
            worst = worst.max((slot[sb].im - direct[sb].im).abs());
        }
        assert!(worst < 1e-10, "FFT versus direct modulation: {worst:e}");
        // synthesis: direct real sum of clause 7.3 with the matrix-equation phase
        let syn = Synthesis::new();
        let mut v = [Complex::default(); 128];
        for (sb, (vv, qq)) in v.iter_mut().zip(slot.iter()).enumerate() {
            *vv = qq.times(syn.pre[sb]);
        }
        syn.idft.run(&mut v);
        for j in 0..128 {
            let fast = v[j].times(syn.post[j]).re;
            let mut direct = 0.0;
            for (sb, qq) in slot.iter().enumerate() {
                let a = PI / (4.0 * BANDS as f64)
                    * (2.0 * sb as f64 + 1.0)
                    * (2.0 * j as f64 - 4.0 * BANDS as f64 + 1.0);
                direct += (qq.re * a.cos() - qq.im * a.sin()) / BANDS as f64;
            }
            assert!(
                (fast - direct).abs() < 1e-10,
                "synthesis j {j}: {fast} vs {direct}"
            );
        }
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
