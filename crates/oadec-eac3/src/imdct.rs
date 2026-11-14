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

/// Radix-2 complex inverse FFT (positive exponent, no scaling) of `N` points,
/// a power of two, over values whose real and imaginary parts are held apart.
///
/// Held apart because that is what was measured to be fast, and for no
/// better reason that is known. The same butterflies over an array of complex
/// values, with everything else alike, take 1.6 times as long, and what the
/// compiler makes of either takes one number at a time
/// (`docs/performance.md`). What is computed for each value is what it was,
/// operation for operation: no sample moves by a bit.
#[derive(Debug, Clone)]
struct Ifft<const N: usize> {
    /// Where the bit-reversal that opens the transform puts each value.
    rev: [usize; N],
    /// The twiddles of the stage of two points, then of the stage of four,
    /// and so on, in a row: `N - 1` of them.
    stage_re: [f64; N],
    stage_im: [f64; N],
}

impl<const N: usize> Ifft<N> {
    fn new() -> Self {
        assert!(N.is_power_of_two());
        let bits = N.trailing_zeros();
        let rev = core::array::from_fn(|i| {
            if bits == 0 {
                0
            } else {
                i.reverse_bits() >> (usize::BITS - bits)
            }
        });
        let mut stage_re = [0.0; N];
        let mut stage_im = [0.0; N];
        let mut at = 0;
        let mut len = 2;
        while len <= N {
            let step = N / len;
            for k in 0..len / 2 {
                let a = 2.0 * PI * (k * step) as f64 / N as f64;
                stage_re[at] = a.cos();
                stage_im[at] = a.sin();
                at += 1;
            }
            len *= 2;
        }
        Self {
            rev,
            stage_re,
            stage_im,
        }
    }

    /// The stages of the transform, over values that are already where the
    /// bit-reversal puts them: value `k` at `rev[k]`.
    fn stages(&self, re: &mut [f64; N], im: &mut [f64; N]) {
        let mut at = 0;
        let mut half = 1;
        while half < N {
            let twiddle_re = &self.stage_re[at..at + half];
            let twiddle_im = &self.stage_im[at..at + half];
            for (re, im) in re
                .chunks_exact_mut(2 * half)
                .zip(im.chunks_exact_mut(2 * half))
            {
                let (low_re, high_re) = re.split_at_mut(half);
                let (low_im, high_im) = im.split_at_mut(half);
                let low = low_re.iter_mut().zip(low_im);
                let high = high_re.iter_mut().zip(high_im);
                let twiddles = twiddle_re.iter().zip(twiddle_im);
                for (((ar, ai), (br, bi)), (wr, wi)) in low.zip(high).zip(twiddles) {
                    // b times the twiddle: (br + j bi)(wr + j wi)
                    let tr = *br * wr - *bi * wi;
                    let ti = *br * wi + *bi * wr;
                    let (xr, xi) = (*ar, *ai);
                    *ar = xr + tr;
                    *ai = xi + ti;
                    *br = xr - tr;
                    *bi = xi - ti;
                }
            }
            at += half;
            half *= 2;
        }
    }
}

/// The 512-point forward transform that enhanced coupling uses to turn the
/// reconstructed carrier into a spectrum (ATSC A/52:2018 clause E.3.5.5.1
/// step 5). The clause divides by N; that scaling is applied here.
#[derive(Debug, Clone)]
pub struct Dft512 {
    fft: Ifft<512>,
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
        Self { fft: Ifft::new() }
    }

    /// Transforms in place and scales by 1/512.
    ///
    /// The forward transform `X[k] = sum_n x[n] e^(-2 pi i k n / n)` is the
    /// inverse one, which has the positive exponent, between two
    /// conjugations: it turns around without a second table.
    pub fn forward(&self, data: &mut [Complex; 512]) {
        let mut re = [0.0f64; 512];
        let mut im = [0.0f64; 512];
        for (c, &to) in data.iter().zip(&self.fft.rev) {
            re[to] = c.re;
            im[to] = -c.im;
        }
        self.fft.stages(&mut re, &mut im);
        for ((c, re), im) in data.iter_mut().zip(&re).zip(&im) {
            c.re = re / 512.0;
            c.im = -im / 512.0;
        }
    }
}

/// The AC-3 inverse transform with its window and twiddle tables.
#[derive(Debug, Clone)]
pub struct Imdct {
    window: [f64; N2],
    /// The twiddles of the 512-sample transform, real and imaginary apart.
    x1_re: [f64; 128],
    x1_im: [f64; 128],
    /// The twiddles of the 256-sample transforms of a block-switched block.
    x2_re: [f64; 64],
    x2_im: [f64; 64],
    ifft128: Ifft<128>,
    ifft64: Ifft<64>,
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
        // -(cos + j sin) of 2 pi (8k + 1) / (8 N) and of 2 pi (8k + 1) / (4 N)
        let angle = |k: usize, of: f64| 2.0 * PI * (8.0 * k as f64 + 1.0) / (of * 512.0);
        Self {
            window,
            x1_re: core::array::from_fn(|k| -angle(k, 8.0).cos()),
            x1_im: core::array::from_fn(|k| -angle(k, 8.0).sin()),
            x2_re: core::array::from_fn(|k| -angle(k, 4.0).cos()),
            x2_im: core::array::from_fn(|k| -angle(k, 4.0).sin()),
            ifft128: Ifft::new(),
            ifft64: Ifft::new(),
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
        let (twiddle_re, twiddle_im) = (&self.x1_re, &self.x1_im);
        let mut re = [0.0f64; 128];
        let mut im = [0.0f64; 128];
        // step 2, each value put where the inverse FFT begins with it
        for k in 0..128 {
            let (ar, ai) = (xk[255 - 2 * k], xk[2 * k]);
            let to = self.ifft128.rev[k];
            re[to] = ar * twiddle_re[k] - ai * twiddle_im[k];
            im[to] = ar * twiddle_im[k] + ai * twiddle_re[k];
        }
        self.ifft128.stages(&mut re, &mut im);
        let mut y_re = [0.0f64; 128];
        let mut y_im = [0.0f64; 128];
        for n in 0..128 {
            y_re[n] = re[n] * twiddle_re[n] - im[n] * twiddle_im[n];
            y_im[n] = re[n] * twiddle_im[n] + im[n] * twiddle_re[n];
        }
        for n in 0..64 {
            x[2 * n] = -y_im[64 + n] * w[2 * n];
            x[2 * n + 1] = y_re[63 - n] * w[2 * n + 1];
            x[128 + 2 * n] = -y_re[n] * w[128 + 2 * n];
            x[128 + 2 * n + 1] = y_im[127 - n] * w[128 + 2 * n + 1];
            x[256 + 2 * n] = -y_re[64 + n] * w[255 - 2 * n];
            x[256 + 2 * n + 1] = y_im[63 - n] * w[254 - 2 * n];
            x[384 + 2 * n] = y_im[n] * w[127 - 2 * n];
            x[384 + 2 * n + 1] = -y_re[127 - n] * w[126 - 2 * n];
        }
    }

    fn short(&self, xk: &[f64; N2], x: &mut [f64; 512]) {
        let w = &self.window;
        let (twiddle_re, twiddle_im) = (&self.x2_re, &self.x2_im);
        // the even coefficients are one transform and the odd ones the other
        let mut re1 = [0.0f64; 64];
        let mut im1 = [0.0f64; 64];
        let mut re2 = [0.0f64; 64];
        let mut im2 = [0.0f64; 64];
        for k in 0..64 {
            let to = self.ifft64.rev[k];
            let (ar, ai) = (xk[254 - 4 * k], xk[4 * k]);
            re1[to] = ar * twiddle_re[k] - ai * twiddle_im[k];
            im1[to] = ar * twiddle_im[k] + ai * twiddle_re[k];
            let (ar, ai) = (xk[255 - 4 * k], xk[4 * k + 1]);
            re2[to] = ar * twiddle_re[k] - ai * twiddle_im[k];
            im2[to] = ar * twiddle_im[k] + ai * twiddle_re[k];
        }
        self.ifft64.stages(&mut re1, &mut im1);
        self.ifft64.stages(&mut re2, &mut im2);
        let mut y1_re = [0.0f64; 64];
        let mut y1_im = [0.0f64; 64];
        let mut y2_re = [0.0f64; 64];
        let mut y2_im = [0.0f64; 64];
        for n in 0..64 {
            y1_re[n] = re1[n] * twiddle_re[n] - im1[n] * twiddle_im[n];
            y1_im[n] = re1[n] * twiddle_im[n] + im1[n] * twiddle_re[n];
            y2_re[n] = re2[n] * twiddle_re[n] - im2[n] * twiddle_im[n];
            y2_im[n] = re2[n] * twiddle_im[n] + im2[n] * twiddle_re[n];
        }
        for n in 0..64 {
            x[2 * n] = -y1_im[n] * w[2 * n];
            x[2 * n + 1] = y1_re[63 - n] * w[2 * n + 1];
            x[128 + 2 * n] = -y1_re[n] * w[128 + 2 * n];
            x[128 + 2 * n + 1] = y1_im[63 - n] * w[128 + 2 * n + 1];
            x[256 + 2 * n] = -y2_re[n] * w[255 - 2 * n];
            x[256 + 2 * n + 1] = y2_im[63 - n] * w[254 - 2 * n];
            x[384 + 2 * n] = y2_im[n] * w[127 - 2 * n];
            x[384 + 2 * n + 1] = -y2_re[63 - n] * w[126 - 2 * n];
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

    /// The arithmetic of complex values, written out once: what the
    /// transform does to a pair of rows of numbers, it does to these.
    impl Complex {
        fn mul(self, o: Complex) -> Complex {
            Complex {
                re: self.re * o.re - self.im * o.im,
                im: self.re * o.im + self.im * o.re,
            }
        }
        fn add(self, o: Complex) -> Complex {
            Complex {
                re: self.re + o.re,
                im: self.im + o.im,
            }
        }
        fn sub(self, o: Complex) -> Complex {
            Complex {
                re: self.re - o.re,
                im: self.im - o.im,
            }
        }
    }

    /// The inverse FFT the plain way: one complex value at a time, the
    /// values swapped into bit-reversed order, one table of twiddles that
    /// every stage steps through.
    fn plain_ifft(data: &mut [Complex]) {
        let n = data.len();
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                data.swap(i, j);
            }
        }
        let twiddle = |k: usize| {
            let a = 2.0 * PI * k as f64 / n as f64;
            Complex {
                re: a.cos(),
                im: a.sin(),
            }
        };
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let w = twiddle(k * step);
                    let a = data[start + k];
                    let b = data[start + k + len / 2].mul(w);
                    data[start + k] = a.add(b);
                    data[start + k + len / 2] = a.sub(b);
                }
            }
            len *= 2;
        }
    }

    /// The twiddle of clause 6.9.4: `-(cos + j sin)` of `2 pi (8k + 1) / of`.
    fn plain_twiddle(k: usize, of: f64) -> Complex {
        let a = 2.0 * PI * (8.0 * k as f64 + 1.0) / of;
        Complex {
            re: -a.cos(),
            im: -a.sin(),
        }
    }

    /// Clause 6.9.4.1, steps 1 to 5, as the clause writes them.
    fn plain_long(w: &[f64; N2], xk: &[f64; N2]) -> [f64; 512] {
        let mut z = [Complex::default(); 128];
        for k in 0..128 {
            let a = Complex {
                re: xk[255 - 2 * k],
                im: xk[2 * k],
            };
            z[k] = a.mul(plain_twiddle(k, 8.0 * 512.0));
        }
        plain_ifft(&mut z);
        let mut y = [Complex::default(); 128];
        for n in 0..128 {
            y[n] = z[n].mul(plain_twiddle(n, 8.0 * 512.0));
        }
        let mut x = [0.0f64; 512];
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
        x
    }

    /// Clause 6.9.4.2, the pair of 256-sample transforms of a block-switched
    /// block, as the clause writes them.
    fn plain_short(w: &[f64; N2], xk: &[f64; N2]) -> [f64; 512] {
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
            .mul(plain_twiddle(k, 4.0 * 512.0));
            z2[k] = Complex {
                re: x2[127 - 2 * k],
                im: x2[2 * k],
            }
            .mul(plain_twiddle(k, 4.0 * 512.0));
        }
        plain_ifft(&mut z1);
        plain_ifft(&mut z2);
        let mut y1 = [Complex::default(); 64];
        let mut y2 = [Complex::default(); 64];
        for n in 0..64 {
            y1[n] = z1[n].mul(plain_twiddle(n, 4.0 * 512.0));
            y2[n] = z2[n].mul(plain_twiddle(n, 4.0 * 512.0));
        }
        let mut x = [0.0f64; 512];
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
        x
    }

    /// Blocks of coefficients a decoder meets: noise over the whole band,
    /// noise with a top that is zero, a single bin, nothing at all, and the
    /// zeros of each with their sign turned. And blocks no decoder meets,
    /// which the arithmetic has to come through all the same.
    fn blocks() -> Vec<[f64; N2]> {
        let mut seed = 0x0000_dd5e_ed50_fac3_u64;
        let mut blocks = Vec::new();
        for kind in 0..48 {
            let mut block = [0.0f64; N2];
            match kind % 4 {
                0 => block.iter_mut().for_each(|c| *c = lcg(&mut seed)),
                1 => block[..40 + 4 * kind]
                    .iter_mut()
                    .for_each(|c| *c = lcg(&mut seed)),
                2 => block[5 * kind] = lcg(&mut seed),
                _ => {}
            }
            if kind % 8 >= 4 {
                for c in &mut block {
                    if *c == 0.0 {
                        *c = -0.0;
                    }
                }
            }
            blocks.push(block);
        }
        // Numbers too small to come through a multiplication: the smallest
        // there is, here and there in a block of zeros of either sign. A
        // product of one is a zero with a sign, and the two ways have to
        // agree on the sign of every zero they make of them.
        let least = f64::from_bits(1);
        for kind in 0..32 {
            let zero = if kind % 2 == 0 { 0.0 } else { -0.0 };
            let mut block = [zero; N2];
            for bin in (kind..N2).step_by(3 + kind % 5) {
                block[bin] = if (bin / 7) % 2 == 0 { least } else { -least };
            }
            blocks.push(block);
        }
        // and the same among zeros whose sign goes by the bin: negative in
        // the even bins and positive in the odd ones, or the other way round
        for kind in 0..32 {
            let mut block: [f64; N2] =
                core::array::from_fn(|bin| if (bin + kind) % 2 == 0 { -0.0 } else { 0.0 });
            for bin in ((kind / 2)..N2).step_by(2 + kind % 7) {
                block[bin] = if (bin / 5) % 2 == 0 { least } else { -least };
            }
            blocks.push(block);
        }
        blocks
    }

    /// However the transform lays its numbers out, what it computes is the
    /// transform of the clause computed the plain way, to the last bit and
    /// to the sign of a zero: no sample of any decode may move.
    #[test]
    fn the_transform_is_the_plain_one_bit_for_bit() {
        let imdct = Imdct::new();
        let w = *imdct.window();
        for (n, block) in blocks().iter().enumerate() {
            for blksw in [false, true] {
                let plain = if blksw {
                    plain_short(&w, block)
                } else {
                    plain_long(&w, block)
                };
                let mut x = [f64::NAN; 512];
                imdct.windowed(block, blksw, &mut x);
                for (i, (got, want)) in x.iter().zip(&plain).enumerate() {
                    assert_eq!(
                        got.to_bits(),
                        want.to_bits(),
                        "block {n}, block switch {blksw}, sample {i}: {got:e} for {want:e}"
                    );
                }
            }
        }
    }

    /// And the forward transform of enhanced coupling is the plain one turned
    /// around, bit for bit.
    #[test]
    fn the_forward_transform_is_the_plain_one_bit_for_bit() {
        let mut seed = 0x5eed_f0f0_5eed_u64;
        for _ in 0..8 {
            let mut data = [Complex::default(); 512];
            for c in data.iter_mut() {
                c.re = lcg(&mut seed);
                c.im = lcg(&mut seed);
            }
            let mut plain = data;
            for c in plain.iter_mut() {
                c.im = -c.im;
            }
            plain_ifft(&mut plain);
            for c in plain.iter_mut() {
                c.im = -c.im;
                c.re /= 512.0;
                c.im /= 512.0;
            }
            Dft512::new().forward(&mut data);
            for (k, (got, want)) in data.iter().zip(&plain).enumerate() {
                assert_eq!(got.re.to_bits(), want.re.to_bits(), "k {k}");
                assert_eq!(got.im.to_bits(), want.im.to_bits(), "k {k}");
            }
        }
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
        plain_ifft(&mut data);
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
