//! Measurements on decoded samples.

use std::f64::consts::PI;

/// Signal to noise ratio of `test` against `reference` over the samples both
/// have, in dB: the power of the reference over the power of the difference.
/// Infinite when they are identical.
#[must_use]
pub fn snr_db(reference: &[f64], test: &[f64]) -> f64 {
    let n = reference.len().min(test.len());
    let signal: f64 = reference[..n].iter().map(|v| v * v).sum();
    let noise: f64 = reference[..n]
        .iter()
        .zip(&test[..n])
        .map(|(a, b)| (a - b) * (a - b))
        .sum();
    if noise == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (signal / noise).log10()
    }
}

/// The largest absolute difference over the samples both have.
#[must_use]
pub fn peak_error(reference: &[f64], test: &[f64]) -> f64 {
    reference
        .iter()
        .zip(test)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max)
}

/// The amplitude of the tone at `freq` in `x`, by a Hann-windowed single-bin
/// DFT, scaled so that a full-scale sine reads 1.
#[must_use]
pub fn tone_level(x: &[f64], freq: f64, rate: f64) -> f64 {
    let n = x.len();
    if n < 2 {
        return 0.0;
    }
    let (mut re, mut im, mut wsum) = (0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / (n - 1) as f64).cos();
        let ph = 2.0 * PI * freq * i as f64 / rate;
        re += w * v * ph.cos();
        im -= w * v * ph.sin();
        wsum += w;
    }
    2.0 * (re * re + im * im).sqrt() / wsum
}

/// Which of `candidates` (frequencies) is loudest in `x`.
#[must_use]
pub fn dominant(x: &[f64], rate: f64, candidates: &[f64]) -> usize {
    candidates
        .iter()
        .enumerate()
        .map(|(i, &f)| (i, tone_level(x, f, rate)))
        .fold((0, f64::MIN), |best, c| if c.1 > best.1 { c } else { best })
        .0
}

/// The lag in `-max..=max` at which `b` best matches `a` (`b[i + lag]`
/// against `a[i]`), by the plain cross-correlation.
#[must_use]
pub fn best_lag(a: &[f64], b: &[f64], max: isize) -> isize {
    let mut best = (0isize, f64::MIN);
    for lag in -max..=max {
        let mut acc = 0.0;
        for (i, &v) in a.iter().enumerate() {
            let j = i as isize + lag;
            if j >= 0 && (j as usize) < b.len() {
                acc += v * b[j as usize];
            }
        }
        if acc > best.1 {
            best = (lag, acc);
        }
    }
    best.0
}

/// The normalised correlation of two signals over the samples both have: 1
/// for the same shape, 0 for a quarter turn of a tone.
#[must_use]
pub fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    let ab: f64 = a[..n].iter().zip(&b[..n]).map(|(x, y)| x * y).sum();
    let aa: f64 = a[..n].iter().map(|x| x * x).sum();
    let bb: f64 = b[..n].iter().map(|x| x * x).sum();
    ab / (aa * bb).sqrt()
}
