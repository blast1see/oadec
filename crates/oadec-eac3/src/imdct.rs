//! Inverse transform, windowing and overlap-add (ETSI TS 102 366 clause 6.9).
//!
//! The 512-sample transform is computed with the N/4-point complex IFFT and
//! the pre/post twiddles of clause 6.9.4.1; the block-switched pair of
//! 256-sample transforms follows clause 6.9.4.2. The window is the
//! Kaiser-Bessel-derived window (alpha 5) of which table 6.33 prints the
//! rounded values.

use std::f64::consts::PI;

/// Transform coefficients per block.
pub const N2: usize = 256;

/// A complex value of the transforms.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    #[inline]
    pub(crate) fn mul(self, o: Complex) -> Complex {
        Complex {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }
    #[inline]
    pub(crate) fn add(self, o: Complex) -> Complex {
        Complex {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
    #[inline]
    pub(crate) fn sub(self, o: Complex) -> Complex {
        Complex {
            re: self.re - o.re,
            im: self.im - o.im,
        }
    }
}

/// Radix-2 complex inverse FFT (positive exponent, no scaling) of a fixed
/// power-of-two size.
#[derive(Debug, Clone)]
struct Ifft {
    n: usize,
    twiddles: Vec<Complex>,
    rev: Vec<usize>,
}

impl Ifft {
    fn new(n: usize) -> Self {
        assert!(n.is_power_of_two());
        let bits = n.trailing_zeros();
        let rev = (0..n)
            .map(|i| {
                if bits == 0 {
                    0
                } else {
                    i.reverse_bits() >> (usize::BITS - bits)
                }
            })
            .collect();
        let twiddles = (0..n / 2)
            .map(|k| {
                let a = 2.0 * PI * k as f64 / n as f64;
                Complex {
                    re: a.cos(),
                    im: a.sin(),
                }
            })
            .collect();
        Self { n, twiddles, rev }
    }

    fn run(&self, data: &mut [Complex]) {
        let n = self.n;
        for i in 0..n {
            let j = self.rev[i];
            if j > i {
                data.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let w = self.twiddles[k * step];
                    let a = data[start + k];
                    let b = data[start + k + len / 2].mul(w);
                    data[start + k] = a.add(b);
                    data[start + k + len / 2] = a.sub(b);
                }
            }
            len *= 2;
        }
    }

    /// The forward transform `X[k] = sum_n x[n] e^(-2 pi i k n / n)`, no
    /// scaling. `run` uses the positive exponent, so conjugating on the way
    /// in and out turns it around without a second table.
    fn run_forward(&self, data: &mut [Complex]) {
        for c in data.iter_mut() {
            c.im = -c.im;
        }
        self.run(data);
        for c in data.iter_mut() {
            c.im = -c.im;
        }
    }
}

/// The 512-point forward transform that enhanced coupling uses to turn the
/// reconstructed carrier into a spectrum (ATSC A/52:2018 clause E.3.5.5.1
/// step 5). The clause divides by N; that scaling is applied here.
#[derive(Debug, Clone)]
pub struct Dft512 {
    fft: Ifft,
}

impl Default for Dft512 {
    fn default() -> Self {
        Self::new()
    }
}

impl Dft512 {
    /// Builds the tables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            fft: Ifft::new(512),
        }
    }

    /// Transforms in place and scales by 1/512.
    pub fn forward(&self, data: &mut [Complex; 512]) {
        self.fft.run_forward(data);
        for c in data.iter_mut() {
            c.re /= 512.0;
            c.im /= 512.0;
        }
    }
}

/// The AC-3 inverse transform with its window and twiddle tables.
#[derive(Debug, Clone)]
pub struct Imdct {
    window: [f64; N2],
    x1: [Complex; 128],
    x2: [Complex; 64],
    ifft128: Ifft,
    ifft64: Ifft,
}

/// Modified Bessel function of the first kind, order zero (power series).
fn bessel_i0(x: f64) -> f64 {
    let q = x * x / 4.0;
    let mut term = 1.0;
    let mut sum = 1.0;
    for k in 1..200 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Kaiser-Bessel-derived window of length `2 * half` with parameter `alpha`
/// (the first half; the second half is the mirror image).
fn kbd_window(half: usize, alpha: f64) -> Vec<f64> {
    let kaiser: Vec<f64> = (0..=half)
        .map(|j| {
            let t = 2.0 * j as f64 / half as f64 - 1.0;
            bessel_i0(PI * alpha * (1.0 - t * t).max(0.0).sqrt())
        })
        .collect();
    let total: f64 = kaiser.iter().sum();
    let mut acc = 0.0;
    kaiser[..half]
        .iter()
        .map(|k| {
            acc += k;
            (acc / total).sqrt()
        })
        .collect()
}

impl Default for Imdct {
    fn default() -> Self {
        Self::new()
    }
}

impl Imdct {
    /// Builds the tables.
    #[must_use]
    pub fn new() -> Self {
        let w = kbd_window(N2, 5.0);
        let mut window = [0.0; N2];
        window.copy_from_slice(&w);
        let mut x1 = [Complex::default(); 128];
        for (k, x) in x1.iter_mut().enumerate() {
            let a = 2.0 * PI * (8.0 * k as f64 + 1.0) / (8.0 * 512.0);
            *x = Complex {
                re: -a.cos(),
                im: -a.sin(),
            };
        }
        let mut x2 = [Complex::default(); 64];
        for (k, x) in x2.iter_mut().enumerate() {
            let a = 2.0 * PI * (8.0 * k as f64 + 1.0) / (4.0 * 512.0);
            *x = Complex {
                re: -a.cos(),
                im: -a.sin(),
            };
        }
        Self {
            window,
            x1,
            x2,
            ifft128: Ifft::new(128),
            ifft64: Ifft::new(64),
        }
    }

    /// The window (first half of the 512-point symmetric window).
    #[must_use]
    pub fn window(&self) -> &[f64; N2] {
        &self.window
    }

    /// The windowed inverse transform of one block (clause 6.9.4.1 steps 1 to
    /// 5): the 512 samples before the overlap-add. Enhanced coupling needs
    /// them for the block before and the block after (ATSC A/52:2018 clause
    /// E.3.5.5.1).
    pub fn windowed(&self, coeffs: &[f64; N2], blksw: bool, x: &mut [f64; 512]) {
        if blksw {
            self.short(coeffs, x);
        } else {
            self.long(coeffs, x);
        }
    }

    /// Transforms one block of 256 coefficients and overlap-adds it with the
    /// previous block held in `delay`, producing 256 output samples.
    pub fn process(
        &self,
        coeffs: &[f64; N2],
        blksw: bool,
        delay: &mut [f64; N2],
        out: &mut [f64; N2],
    ) {
        let mut x = [0.0f64; 512];
        self.windowed(coeffs, blksw, &mut x);
        for n in 0..N2 {
            out[n] = 2.0 * (x[n] + delay[n]);
            delay[n] = x[N2 + n];
        }
    }

    fn long(&self, xk: &[f64; N2], x: &mut [f64; 512]) {
        let w = &self.window;
        let mut z = [Complex::default(); 128];
        for k in 0..128 {
            let a = Complex {
                re: xk[255 - 2 * k],
                im: xk[2 * k],
            };
            z[k] = a.mul(self.x1[k]);
        }
        self.ifft128.run(&mut z);
        let mut y = [Complex::default(); 128];
        for n in 0..128 {
            y[n] = z[n].mul(self.x1[n]);
        }
        for n in 0..64 {
            x[2 * n] = -y[64 + n].im * w[2 * n];
            x[2 * n + 1] = y[63 - n].re * w[2 * n + 1];
            x[128 + 2 * n] = -y[n].re * w[128 + 2 * n];
            x[128 + 2 * n + 1] = y[127 - n].im * w[128 + 2 * n + 1];
            x[256 + 2 * n] = -y[64 + n].re * w[255 - 2 * n];
            x[256 + 2 * n + 1] = y[63 - n].im * w[254 - 2 * n];
            x[384 + 2 * n] = y[n].im * w[127 - 2 * n];
            x[384 + 2 * n + 1] = -y[127 - n].re * w[126 - 2 * n];
        }
    }

    fn short(&self, xk: &[f64; N2], x: &mut [f64; 512]) {
        let w = &self.window;
        let mut x1 = [0.0f64; 128];
        let mut x2 = [0.0f64; 128];
        for k in 0..128 {
            x1[k] = xk[2 * k];
            x2[k] = xk[2 * k + 1];
        }
        let mut z1 = [Complex::default(); 64];
        let mut z2 = [Complex::default(); 64];
        for k in 0..64 {
            z1[k] = Complex {
                re: x1[127 - 2 * k],
                im: x1[2 * k],
            }
            .mul(self.x2[k]);
            z2[k] = Complex {
                re: x2[127 - 2 * k],
                im: x2[2 * k],
            }
            .mul(self.x2[k]);
        }
        self.ifft64.run(&mut z1);
        self.ifft64.run(&mut z2);
        let mut y1 = [Complex::default(); 64];
        let mut y2 = [Complex::default(); 64];
        for n in 0..64 {
            y1[n] = z1[n].mul(self.x2[n]);
            y2[n] = z2[n].mul(self.x2[n]);
        }
        for n in 0..64 {
            x[2 * n] = -y1[n].im * w[2 * n];
            x[2 * n + 1] = y1[63 - n].re * w[2 * n + 1];
            x[128 + 2 * n] = -y1[n].re * w[128 + 2 * n];
            x[128 + 2 * n + 1] = y1[63 - n].im * w[128 + 2 * n + 1];
            x[256 + 2 * n] = -y2[n].re * w[255 - 2 * n];
            x[256 + 2 * n + 1] = y2[63 - n].im * w[254 - 2 * n];
            x[384 + 2 * n] = y2[n].im * w[127 - 2 * n];
            x[384 + 2 * n + 1] = -y2[63 - n].re * w[126 - 2 * n];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::WINDOW_TABLE;

    /// A deterministic pseudo-random source for the transform tests.
    fn lcg(seed: &mut u64) -> f64 {
        *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((*seed >> 33) as f64 / (1u64 << 31) as f64) - 1.0
    }

    #[test]
    fn the_forward_transform_matches_the_direct_sum() {
        // The only forward transform in the crate, so it is checked against
        // the definition rather than against another implementation.
        let mut seed = 0x1234_5678_9abc_def0u64;
        let mut data = [Complex::default(); 512];
        for c in data.iter_mut() {
            c.re = lcg(&mut seed);
            c.im = lcg(&mut seed);
        }
        let src = data;
        Dft512::new().forward(&mut data);
        for k in [0usize, 1, 7, 64, 255, 256, 511] {
            let (mut re, mut im) = (0.0, 0.0);
            for (n, c) in src.iter().enumerate() {
                let a = -2.0 * PI * (k * n % 512) as f64 / 512.0;
                re += c.re * a.cos() - c.im * a.sin();
                im += c.re * a.sin() + c.im * a.cos();
            }
            assert!((data[k].re - re / 512.0).abs() < 1e-9, "k {k} real");
            assert!((data[k].im - im / 512.0).abs() < 1e-9, "k {k} imag");
        }
    }

    #[test]
    fn the_forward_transform_inverts_the_inverse_one() {
        let mut seed = 0x0fed_cba9_8765_4321u64;
        let mut data = [Complex::default(); 512];
        for c in data.iter_mut() {
            c.re = lcg(&mut seed);
            c.im = lcg(&mut seed);
        }
        let src = data;
        Dft512::new().forward(&mut data);
        Ifft::new(512).run(&mut data);
        for (got, want) in data.iter().zip(src.iter()) {
            assert!((got.re - want.re).abs() < 1e-9 && (got.im - want.im).abs() < 1e-9);
        }
    }

    #[test]
    fn the_windowed_transform_and_an_overlap_add_reproduce_process() {
        let mut seed = 0xdead_beef_cafe_1234u64;
        for blksw in [false, true] {
            let mut coeffs = [0.0f64; N2];
            for c in coeffs.iter_mut() {
                *c = lcg(&mut seed);
            }
            let mut delay = [0.0f64; N2];
            for d in delay.iter_mut() {
                *d = lcg(&mut seed);
            }
            let imdct = Imdct::new();
            let before = delay;
            let mut out = [0.0f64; N2];
            imdct.process(&coeffs, blksw, &mut delay, &mut out);
            let mut x = [0.0f64; 512];
            imdct.windowed(&coeffs, blksw, &mut x);
            for n in 0..N2 {
                assert!((out[n] - 2.0 * (x[n] + before[n])).abs() < 1e-12);
                assert!((delay[n] - x[N2 + n]).abs() < 1e-12);
            }
        }
    }

    /// The forward transform of clause 7.2.3.2 on a windowed block.
    fn forward(x: &[f64], alpha: f64, n: usize) -> Vec<f64> {
        let nf = n as f64;
        (0..n / 2)
            .map(|k| {
                let kf = k as f64;
                let mut acc = 0.0;
                for (i, &xi) in x.iter().enumerate() {
                    let nn = i as f64;
                    let phase = 2.0 * PI / (4.0 * nf) * (2.0 * nn + 1.0) * (2.0 * kf + 1.0)
                        + PI / 4.0 * (2.0 * kf + 1.0) * (1.0 + alpha);
                    acc += xi * phase.cos();
                }
                -2.0 / nf * acc
            })
            .collect()
    }

    #[test]
    fn window_matches_table_6_33() {
        let im = Imdct::new();
        let mut worst = 0.0f64;
        for (n, &printed) in WINDOW_TABLE.iter().enumerate() {
            worst = worst.max((im.window[n] - printed).abs());
        }
        assert!(
            worst < 6e-6,
            "largest deviation from the printed window {worst}"
        );
    }

    #[test]
    fn long_blocks_reconstruct_the_signal() {
        // three overlapping windowed blocks of a test signal; the middle
        // 256 samples come back exactly (time-domain aliasing cancellation)
        let im = Imdct::new();
        let sig: Vec<f64> = (0..1024)
            .map(|i| {
                let t = i as f64;
                0.3 * (t * 0.05).sin() + 0.2 * (t * 0.31 + 1.0).cos() + 0.1 * (t * 1.7).sin()
            })
            .collect();
        let full_window: Vec<f64> = (0..512)
            .map(|n| {
                if n < 256 {
                    im.window[n]
                } else {
                    im.window[511 - n]
                }
            })
            .collect();
        let mut delay = [0.0; 256];
        let mut out = [0.0; 256];
        let mut worst = 0.0f64;
        for b in 0..3 {
            let start = b * 256;
            let windowed: Vec<f64> = (0..512).map(|n| sig[start + n] * full_window[n]).collect();
            let coeffs_v = forward(&windowed, 0.0, 512);
            let mut coeffs = [0.0; 256];
            coeffs.copy_from_slice(&coeffs_v);
            im.process(&coeffs, false, &mut delay, &mut out);
            if b > 0 {
                for n in 0..256 {
                    worst = worst.max((out[n] - sig[start + n]).abs());
                }
            }
        }
        assert!(worst < 1e-9, "reconstruction error {worst}");
    }

    #[test]
    fn short_blocks_reconstruct_the_signal() {
        let im = Imdct::new();
        let sig: Vec<f64> = (0..1024)
            .map(|i| {
                let t = i as f64;
                0.4 * (t * 0.11).sin() + 0.25 * (t * 0.7 + 0.5).cos()
            })
            .collect();
        let full_window: Vec<f64> = (0..512)
            .map(|n| {
                if n < 256 {
                    im.window[n]
                } else {
                    im.window[511 - n]
                }
            })
            .collect();
        let mut delay = [0.0; 256];
        let mut out = [0.0; 256];
        let mut worst = 0.0f64;
        for b in 0..3 {
            let start = b * 256;
            let windowed: Vec<f64> = (0..512).map(|n| sig[start + n] * full_window[n]).collect();
            // two 256-sample transforms, interleaved bin by bin
            let first = forward(&windowed[..256], -1.0, 256);
            let second = forward(&windowed[256..], 1.0, 256);
            let mut coeffs = [0.0; 256];
            for k in 0..128 {
                coeffs[2 * k] = first[k];
                coeffs[2 * k + 1] = second[k];
            }
            im.process(&coeffs, true, &mut delay, &mut out);
            if b > 0 {
                for n in 0..256 {
                    worst = worst.max((out[n] - sig[start + n]).abs());
                }
            }
        }
        assert!(worst < 1e-9, "reconstruction error {worst}");
    }
}
