//! A measuring loop for the decoder's kernels, with no dependency.
//!
//! `cargo bench -p oadec-bench` runs `benches/kernels.rs` over the streams
//! committed under `crates/oadec-cli/tests/fixtures`, so it needs no media and
//! gives the same work on every machine. It is for comparing two builds on one
//! machine: the same command before and after a change. What a whole decode of
//! real material costs, in time, processor time and memory, is measured from
//! outside the process by `tools/bench.py`.
//!
//! A kernel is called in batches long enough for the clock to resolve, and the
//! figure reported is the median over the batches of the time one call took.
//! The median absolute deviation is reported beside it: a difference between
//! two builds smaller than a few of those is not a difference.

use std::hint::black_box;
use std::time::{Duration, Instant};

/// Timed batches per kernel.
pub const BATCHES: usize = 15;

/// The least time one batch is made to take.
const BATCH_TIME: Duration = Duration::from_millis(40);

/// How one kernel measured.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// What was measured.
    pub name: String,
    /// Calls of the kernel in one batch.
    pub calls: u64,
    /// Median over the batches of the time of one call, in nanoseconds.
    pub median_ns: f64,
    /// Median absolute deviation over the batches, as a share of the median.
    pub mad_share: f64,
    /// Seconds of audio one call decodes, for a kernel that decodes audio.
    pub audio_seconds: Option<f64>,
}

impl Sample {
    /// How many times faster than real time the kernel decodes.
    #[must_use]
    pub fn realtime(&self) -> Option<f64> {
        self.audio_seconds
            .filter(|_| self.median_ns > 0.0)
            .map(|s| s / (self.median_ns * 1e-9))
    }
}

/// The median of `values`, which it sorts. Zero for none.
pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        f64::midpoint(values[mid - 1], values[mid])
    }
}

/// Median and median absolute deviation, the second as a share of the first.
#[must_use]
pub fn spread(values: &[f64]) -> (f64, f64) {
    let mut sorted = values.to_vec();
    let centre = median(&mut sorted);
    let mut deviations: Vec<f64> = values.iter().map(|v| (v - centre).abs()).collect();
    let mad = median(&mut deviations);
    (centre, if centre > 0.0 { mad / centre } else { 0.0 })
}

/// Measures `kernel`: finds how many calls fill a batch, then times
/// [`BATCHES`] of them.
pub fn measure<R>(name: &str, audio_seconds: Option<f64>, mut kernel: impl FnMut() -> R) -> Sample {
    let mut calls = 1u64;
    loop {
        let start = Instant::now();
        for _ in 0..calls {
            black_box(kernel());
        }
        let took = start.elapsed();
        if took >= BATCH_TIME || calls >= 1 << 24 {
            break;
        }
        let short = BATCH_TIME.as_secs_f64() / took.as_secs_f64().max(1e-9);
        calls = (calls as f64 * short.clamp(2.0, 100.0)).ceil() as u64;
    }
    let per_call: Vec<f64> = (0..BATCHES)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..calls {
                black_box(kernel());
            }
            start.elapsed().as_secs_f64() * 1e9 / calls as f64
        })
        .collect();
    let (median_ns, mad_share) = spread(&per_call);
    Sample {
        name: name.to_string(),
        calls,
        median_ns,
        mad_share,
        audio_seconds,
    }
}

/// A time in the unit that reads best.
#[must_use]
pub fn readable(ns: f64) -> String {
    if ns >= 1e9 {
        format!("{:.3} s", ns / 1e9)
    } else if ns >= 1e6 {
        format!("{:.3} ms", ns / 1e6)
    } else if ns >= 1e3 {
        format!("{:.3} us", ns / 1e3)
    } else {
        format!("{ns:.1} ns")
    }
}

/// One kernel as a line of a table whose first column is `width` wide.
#[must_use]
pub fn line(sample: &Sample, width: usize) -> String {
    let speed = sample
        .realtime()
        .map_or_else(String::new, |x| format!("  {x:.0}x real time"));
    format!(
        "{:<width$}  {:>12}  +/- {:>4.1} %{speed}",
        sample.name,
        readable(sample.median_ns),
        sample.mad_share * 100.0
    )
}

/// The samples as a table, one kernel to a line.
#[must_use]
pub fn table(samples: &[Sample]) -> String {
    let width = samples.iter().map(|s| s.name.len()).max().unwrap_or(0);
    samples.iter().map(|s| line(s, width) + "\n").collect()
}

/// The samples as JSON. Written by hand: the names are this crate's own and
/// the rest are numbers, so the crate needs no serialiser.
#[must_use]
pub fn json(samples: &[Sample]) -> String {
    let rows: Vec<String> = samples
        .iter()
        .map(|s| {
            let name = s.name.replace('\\', "\\\\").replace('"', "\\\"");
            let audio = s
                .audio_seconds
                .map_or_else(|| "null".to_string(), |v| format!("{v}"));
            let speed = s
                .realtime()
                .map_or_else(|| "null".to_string(), |v| format!("{v:.1}"));
            format!(
                " {{\"kernel\": \"{name}\", \"calls_per_batch\": {}, \"median_ns\": {:.1}, \
                 \"mad_share\": {:.4}, \"audio_seconds\": {audio}, \"realtime\": {speed}}}",
                s.calls, s.median_ns, s.mad_share
            )
        })
        .collect();
    format!(
        "{{\"schema\": 1, \"batches\": {BATCHES}, \"kernels\": [\n{}\n]}}\n",
        rows.join(",\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_median_is_the_middle_value_or_the_mean_of_the_two() {
        assert_eq!(median(&mut []), 0.0);
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }

    #[test]
    fn the_spread_is_a_share_of_the_median() {
        let (centre, share) = spread(&[10.0, 10.0, 12.0, 8.0, 10.0]);
        assert_eq!(centre, 10.0);
        assert_eq!(share, 0.0);
        let (centre, share) = spread(&[8.0, 10.0, 12.0]);
        assert_eq!(centre, 10.0);
        assert!((share - 0.2).abs() < 1e-12);
    }

    #[test]
    fn a_kernel_is_called_and_timed() {
        let mut calls = 0u64;
        let sample = measure("counts", Some(1.0), || {
            calls += 1;
            calls
        });
        assert!(calls >= sample.calls * BATCHES as u64);
        assert!(sample.median_ns > 0.0);
        assert!(sample.realtime().is_some());
    }

    #[test]
    fn the_report_names_every_kernel() {
        let samples = [Sample {
            name: "a \"quoted\" kernel".to_string(),
            calls: 3,
            median_ns: 1500.0,
            mad_share: 0.01,
            audio_seconds: None,
        }];
        assert!(table(&samples).contains("1.500 us"));
        let text = json(&samples);
        assert!(text.contains("\"kernel\": \"a \\\"quoted\\\" kernel\""));
        assert!(text.contains("\"realtime\": null"));
    }
}
