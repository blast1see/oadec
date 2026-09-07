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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    #[inline]
    fn mul(self, o: C) -> C {
        C {
            re: self.re * o.re - self.im * o.im,
            im: self.re * o.im + self.im * o.re,
        }
    }
    #[inline]
    fn add(self, o: C) -> C {
        C {
            re: self.re + o.re,
            im: self.im + o.im,
        }
    }
    #[inline]
    fn sub(self, o: C) -> C {
        C {
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
    twiddles: Vec<C>,
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
                C {
                    re: a.cos(),
                    im: a.sin(),
                }
            })
            .collect();
        Self { n, twiddles, rev }
    }

    fn run(&self, data: &mut [C]) {
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
}

/// The AC-3 inverse transform with its window and twiddle tables.
#[derive(Debug, Clone)]
pub struct Imdct {
    window: [f64; N2],
    x1: [C; 128],
    x2: [C; 64],
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
        let mut x1 = [C::default(); 128];
        for (k, x) in x1.iter_mut().enumerate() {
            let a = 2.0 * PI * (8.0 * k as f64 + 1.0) / (8.0 * 512.0);
            *x = C {
                re: -a.cos(),
                im: -a.sin(),
            };
        }
        let mut x2 = [C::default(); 64];
        for (k, x) in x2.iter_mut().enumerate() {
            let a = 2.0 * PI * (8.0 * k as f64 + 1.0) / (4.0 * 512.0);
            *x = C {
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
        if blksw {
            self.short(coeffs, &mut x);
        } else {
            self.long(coeffs, &mut x);
        }
        for n in 0..N2 {
            out[n] = 2.0 * (x[n] + delay[n]);
            delay[n] = x[N2 + n];
        }
    }

    fn long(&self, xk: &[f64; N2], x: &mut [f64; 512]) {
        let w = &self.window;
        let mut z = [C::default(); 128];
        for k in 0..128 {
            let a = C {
                re: xk[255 - 2 * k],
                im: xk[2 * k],
            };
            z[k] = a.mul(self.x1[k]);
        }
        self.ifft128.run(&mut z);
        let mut y = [C::default(); 128];
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
        let mut z1 = [C::default(); 64];
        let mut z2 = [C::default(); 64];
        for k in 0..64 {
            z1[k] = C {
                re: x1[127 - 2 * k],
                im: x1[2 * k],
            }
            .mul(self.x2[k]);
            z2[k] = C {
                re: x2[127 - 2 * k],
                im: x2[2 * k],
            }
            .mul(self.x2[k]);
        }
        self.ifft64.run(&mut z1);
        self.ifft64.run(&mut z2);
        let mut y1 = [C::default(); 64];
        let mut y2 = [C::default(); 64];
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
