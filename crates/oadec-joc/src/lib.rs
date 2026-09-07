//! Joint Object Coding object reconstruction (ETSI TS 103 420 clause 6.6) in
//! the 64-band QMF domain of clause 7.
//!
//! Part of the `oadec` object-audio decoder engine. The E-AC-3 core decoder
//! delivers the downmix channels; this crate turns them into the coded
//! objects with the side information of EMDF payload 14.

pub mod qmf;
pub mod qmf_window;

use oadec_emdf::joc::{Joc, JocObject, MAX_BANDS, MAX_CHANNELS, MAX_OBJECTS, Slope};

pub use qmf::{Analysis, BANDS, Complex, DELAY, Synthesis};

/// Parameter band of every QMF subband for a given band count (table 54).
#[must_use]
pub fn band_map(num_bands: usize) -> [u8; BANDS] {
    // rows of table 54: (first subband of the row, band index per column)
    // columns: 23, 15, 12, 9, 7, 5, 3, 1 bands
    const ROWS: [(usize, [u8; 8]); 23] = [
        (0, [0, 0, 0, 0, 0, 0, 0, 0]),
        (1, [1, 1, 1, 1, 1, 1, 0, 0]),
        (2, [2, 2, 2, 2, 2, 1, 0, 0]),
        (3, [3, 3, 3, 3, 2, 2, 1, 0]),
        (4, [4, 4, 4, 3, 3, 2, 1, 0]),
        (5, [5, 5, 4, 4, 3, 2, 1, 0]),
        (6, [6, 6, 5, 4, 3, 2, 1, 0]),
        (7, [7, 7, 5, 5, 3, 2, 1, 0]),
        (8, [8, 8, 6, 5, 4, 2, 1, 0]),
        (9, [9, 9, 6, 6, 4, 3, 1, 0]),
        (10, [10, 9, 6, 6, 4, 3, 1, 0]),
        (11, [11, 10, 7, 6, 4, 3, 1, 0]),
        (12, [12, 10, 7, 6, 4, 3, 1, 0]),
        (14, [13, 11, 8, 7, 5, 3, 2, 0]),
        (16, [14, 11, 8, 7, 5, 3, 2, 0]),
        (18, [15, 12, 9, 7, 5, 3, 2, 0]),
        (20, [16, 12, 9, 7, 5, 3, 2, 0]),
        (23, [17, 13, 10, 8, 6, 4, 2, 0]),
        (26, [18, 13, 10, 8, 6, 4, 2, 0]),
        (30, [19, 13, 10, 8, 6, 4, 2, 0]),
        (35, [20, 14, 11, 8, 6, 4, 2, 0]),
        (41, [21, 14, 11, 8, 6, 4, 2, 0]),
        (48, [22, 14, 11, 8, 6, 4, 2, 0]),
    ];
    let column = match num_bands {
        23 => 0,
        15 => 1,
        12 => 2,
        9 => 3,
        7 => 4,
        5 => 5,
        3 => 6,
        _ => 7,
    };
    let mut map = [0u8; BANDS];
    for (i, (start, cols)) in ROWS.iter().enumerate() {
        let end = ROWS.get(i + 1).map_or(BANDS, |r| r.0);
        map[*start..end].fill(cols[column]);
    }
    map
}

/// Dequantized matrix coefficient (clause 6.6.4).
#[must_use]
pub fn dequantize(q: u8, quant_idx: u8) -> f64 {
    let nquant = if quant_idx == 0 { 96.0 } else { 192.0 };
    (f64::from(q) - nquant / 2.0) * 820.0 / (4096.0 * (1.0 + f64::from(quant_idx)))
}

/// Object reconstruction state: the previous frame's matrices.
#[derive(Debug, Clone)]
pub struct JocDecoder {
    num_channels: usize,
    num_objects: usize,
    /// `joc_mix_mtx_prev[obj][ch][sb]`.
    prev: Vec<[[f64; BANDS]; MAX_CHANNELS]>,
    /// Interpolated matrix of the current frame: `[obj][ts][ch][sb]`.
    interp: Vec<Vec<[[f64; BANDS]; MAX_CHANNELS]>>,
}

impl JocDecoder {
    /// A decoder for `num_channels` downmix channels and `num_objects`
    /// objects, with zero history (clause 6.6.5, last paragraph).
    #[must_use]
    pub fn new(num_channels: usize, num_objects: usize) -> Self {
        let num_channels = num_channels.min(MAX_CHANNELS);
        let num_objects = num_objects.min(MAX_OBJECTS);
        Self {
            num_channels,
            num_objects,
            prev: vec![[[0.0; BANDS]; MAX_CHANNELS]; num_objects],
            interp: Vec::new(),
        }
    }

    /// Forgets the history (after a splice).
    pub fn reset(&mut self) {
        self.prev.fill([[0.0; BANDS]; MAX_CHANNELS]);
    }

    /// Number of objects.
    #[must_use]
    pub fn num_objects(&self) -> usize {
        self.num_objects
    }

    /// Keeps the previous matrices for a frame without side information.
    pub fn hold(&mut self, num_ts: usize) {
        if self.interp.len() != self.num_objects
            || self.interp.first().is_some_and(|v| v.len() != num_ts)
        {
            self.interp = vec![vec![[[0.0; BANDS]; MAX_CHANNELS]; num_ts]; self.num_objects];
        }
        for obj in 0..self.num_objects {
            for ts in 0..num_ts {
                self.interp[obj][ts] = self.prev[obj];
            }
        }
    }

    /// Computes the interpolated matrices of a frame with `num_ts` time slots
    /// from its side information (clauses 6.6.4 and 6.6.5). Objects absent
    /// from the payload keep the previous matrix.
    pub fn update(&mut self, joc: &Joc, num_ts: usize) {
        if self.interp.len() != self.num_objects
            || self.interp.first().is_some_and(|v| v.len() != num_ts)
        {
            self.interp = vec![vec![[[0.0; BANDS]; MAX_CHANNELS]; num_ts]; self.num_objects];
        }
        let nch = self.num_channels.min(joc.num_channels);
        for obj in 0..self.num_objects {
            let Some(Some(info)) = joc.objects.get(obj) else {
                // absent: hold the previous matrix
                for ts in 0..num_ts {
                    self.interp[obj][ts] = self.prev[obj];
                }
                continue;
            };
            self.update_object(obj, info, nch, num_ts);
        }
    }

    #[allow(
        clippy::needless_range_loop,
        reason = "channel and subband indices address several parallel arrays"
    )]
    fn update_object(&mut self, obj: usize, info: &JocObject, nch: usize, num_ts: usize) {
        let map = band_map(info.num_bands);
        // dequantized data points: [dp][ch][sb]
        let mut dq: Vec<[[f64; BANDS]; MAX_CHANNELS]> = Vec::with_capacity(info.num_dpoints);
        for dp in 0..info.num_dpoints {
            let mut m = [[0.0; BANDS]; MAX_CHANNELS];
            for ch in 0..nch {
                let row: &[u8; MAX_BANDS] = &info.mtx_q[dp][ch];
                for sb in 0..BANDS {
                    m[ch][sb] = dequantize(row[usize::from(map[sb])], info.quant_idx);
                }
            }
            dq.push(m);
        }
        let prev = self.prev[obj];
        let interp = &mut self.interp[obj];
        let n = num_ts as f64;
        match info.slope {
            Slope::Smooth => {
                if info.num_dpoints == 1 {
                    for ts in 0..num_ts {
                        let f = (ts as f64 + 1.0) / n;
                        for ch in 0..nch {
                            for sb in 0..BANDS {
                                let delta = dq[0][ch][sb] - prev[ch][sb];
                                interp[ts][ch][sb] = prev[ch][sb] + f * delta;
                            }
                        }
                    }
                } else {
                    let ts_2 = num_ts / 2;
                    for ts in 0..num_ts {
                        for ch in 0..nch {
                            for sb in 0..BANDS {
                                interp[ts][ch][sb] = if ts < ts_2 {
                                    let delta = dq[0][ch][sb] - prev[ch][sb];
                                    prev[ch][sb] + (ts as f64 + 1.0) * delta / ts_2 as f64
                                } else {
                                    let delta = dq[1][ch][sb] - dq[0][ch][sb];
                                    dq[0][ch][sb]
                                        + (ts - ts_2 + 1) as f64 * delta / (num_ts - ts_2) as f64
                                };
                            }
                        }
                    }
                }
            }
            Slope::Steep => {
                let o0 = usize::from(info.offset_ts[0]);
                let o1 = usize::from(info.offset_ts[1]);
                for ts in 0..num_ts {
                    let src: &[[f64; BANDS]; MAX_CHANNELS] = if info.num_dpoints == 1 {
                        if ts < o0 { &prev } else { &dq[0] }
                    } else if ts < o0 {
                        &prev
                    } else if ts < o1 {
                        &dq[0]
                    } else {
                        &dq[1]
                    };
                    interp[ts] = *src;
                }
            }
        }
        self.prev[obj] = dq[info.num_dpoints - 1];
    }

    /// Reconstructs the objects of one time slot (clause 6.6.6):
    /// `out[obj] = sum_ch input[ch] * m[obj][ts][ch]`.
    #[allow(
        clippy::needless_range_loop,
        reason = "channel and subband indices address several parallel arrays"
    )]
    pub fn reconstruct(&self, ts: usize, input: &[[Complex; BANDS]], out: &mut [[Complex; BANDS]]) {
        let nch = self.num_channels.min(input.len());
        for (obj, o) in out.iter_mut().enumerate().take(self.num_objects) {
            let m = &self.interp[obj][ts];
            for sb in 0..BANDS {
                let mut acc = Complex::default();
                for ch in 0..nch {
                    acc = acc.plus(input[ch][sb].scale(m[ch][sb]));
                }
                o[sb] = acc;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_map_matches_the_examples_of_table_54() {
        let m15 = band_map(15);
        assert_eq!(
            m15[24], 13,
            "clause 6.6.5 example: sb_to_pb(24) = 13 for 15 bands"
        );
        assert_eq!(m15[0], 0);
        assert_eq!(m15[63], 14);
        let m23 = band_map(23);
        assert_eq!(m23[11], 11);
        assert_eq!(m23[12], 12);
        assert_eq!(m23[13], 12);
        assert_eq!(m23[48], 22);
        let m12 = band_map(12);
        assert_eq!(m12[5], 4);
        assert_eq!(m12[63], 11);
        let m1 = band_map(1);
        assert!(m1.iter().all(|&b| b == 0));
        // every map is non-decreasing and covers 0..num_bands
        for nb in [1usize, 3, 5, 7, 9, 12, 15, 23] {
            let m = band_map(nb);
            assert!(m.windows(2).all(|w| w[0] <= w[1]));
            assert_eq!(usize::from(*m.last().unwrap()), nb - 1);
        }
    }

    #[test]
    fn dequantization_ranges_match_the_note_of_clause_6_6_4() {
        assert!((dequantize(0, 0) + 9.609_375).abs() < 1e-9);
        assert!((dequantize(95, 0) - 9.409_179_687_5).abs() < 1e-9);
        assert!((dequantize(0, 1) + 9.609_375).abs() < 1e-9);
        assert!((dequantize(191, 1) - 9.509_277_343_75).abs() < 1e-9);
        assert_eq!(dequantize(48, 0), 0.0);
        assert_eq!(dequantize(96, 1), 0.0);
    }
}
