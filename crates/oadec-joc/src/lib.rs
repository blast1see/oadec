//! Joint Object Coding object reconstruction (ETSI TS 103 420 clause 6.6) in
//! the 64-band QMF domain of clause 7.
//!
//! Part of the `oadec` object-audio decoder engine. The E-AC-3 core decoder
//! delivers the downmix channels; this crate turns them into the coded
//! objects with the side information of EMDF payload 14.

pub mod qmf;
pub mod qmf_window;
pub mod quadrature;

use oadec_emdf::joc::{Joc, JocObject, MAX_BANDS, MAX_CHANNELS, MAX_OBJECTS, Slope};

pub use qmf::{Analysis, BANDS, Complex, DELAY, Synthesis};
pub use quadrature::{Carry, LOW_DELAY, MATRIX_ALIGN, Quadrature};

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

/// Where the steep interpolation of clause 6.6.5 puts its switch.
///
/// `joc_offset_ts` is one-based: clause 6.3.4.4 defines it as
/// `joc_offset_ts_bits + 1`, so the smallest transmittable offset names the
/// first time slot. The `ts` of clause 6.6.5 counts from zero, and its
/// pseudo-code compares the two directly, which places the switch one slot
/// after the offset names. Dolby's decoder switches at the slot the offset
/// names. See `docs/joc.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SteepReading {
    /// `ts < joc_offset_ts`, exactly as clause 6.6.5 prints it.
    AsPrinted,
    /// `ts < joc_offset_ts - 1`: the one-based offset against the zero-based
    /// slot index, which is what Dolby's decoder does.
    #[default]
    Measured,
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
    /// Time slots the subband samples run behind the side information,
    /// which is what the low-band quadrature filter of
    /// [`quadrature`] costs when the downmix carries the 90-degree phase
    /// shift. Zero without it.
    lag: usize,
    /// The last `lag` matrices of the previous frame, so that a slot held
    /// back across the frame boundary still meets the matrix it was coded
    /// with: `[obj][ts]`.
    carried: Vec<Vec<[[f64; BANDS]; MAX_CHANNELS]>>,
    /// Which reading of the steep switch point to apply.
    steep: SteepReading,
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
            lag: 0,
            carried: Vec::new(),
            steep: SteepReading::default(),
        }
    }

    /// Chooses between the printed and the measured reading of the steep
    /// switch point (clause 6.6.5). The default is [`SteepReading::Measured`].
    pub fn set_steep_reading(&mut self, steep: SteepReading) {
        self.steep = steep;
    }

    /// Tells the decoder that the subband samples it will be given run
    /// `lag` time slots behind the side information, so that it holds the
    /// matrices back by the same amount.
    pub fn set_lag(&mut self, lag: usize) {
        self.lag = lag;
        self.carried = vec![vec![[[0.0; BANDS]; MAX_CHANNELS]; lag]; self.num_objects];
    }

    /// Forgets the history (after a splice).
    pub fn reset(&mut self) {
        self.prev.fill([[0.0; BANDS]; MAX_CHANNELS]);
        for obj in &mut self.carried {
            obj.fill([[0.0; BANDS]; MAX_CHANNELS]);
        }
    }

    /// Keeps the tail of the frame that is about to be replaced, so that the
    /// slots still in the filter meet their own matrices.
    fn carry(&mut self) {
        if self.lag == 0 {
            return;
        }
        for obj in 0..self.num_objects {
            let Some(frame) = self.interp.get(obj) else {
                continue;
            };
            let n = frame.len();
            for k in 0..self.lag {
                // slot n - lag + k of the frame going out
                self.carried[obj][k] = frame[(n + k).saturating_sub(self.lag).min(n - 1)];
            }
        }
    }

    /// Number of objects.
    #[must_use]
    pub fn num_objects(&self) -> usize {
        self.num_objects
    }

    /// Keeps the previous matrices for a frame without side information.
    pub fn hold(&mut self, num_ts: usize) {
        self.carry();
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
        self.carry();
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
        let steep = self.steep;
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
                // `joc_offset_ts` is one-based and `ts` is not; see
                // `SteepReading`.
                let back = usize::from(steep == SteepReading::Measured);
                let o0 = usize::from(info.offset_ts[0]).saturating_sub(back);
                let o1 = usize::from(info.offset_ts[1]).saturating_sub(back);
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
            let m = if ts < self.lag {
                &self.carried[obj][ts]
            } else {
                &self.interp[obj][ts - self.lag]
            };
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
    use oadec_emdf::joc::{Joc, JocObject};

    /// One frame of side information for a single object, with the branch of
    /// clause 6.6.5 the test wants.
    fn one_object(
        slope: Slope,
        num_dpoints: usize,
        offset_ts: [u8; 2],
        dp: &[[u8; 5]],
        nch: usize,
    ) -> Joc {
        let mut mtx_q = Vec::new();
        for (d, row) in dp.iter().enumerate().take(num_dpoints) {
            let mut m = [[0u8; oadec_emdf::joc::MAX_BANDS]; oadec_emdf::joc::MAX_CHANNELS];
            for (ch, slot) in m.iter_mut().enumerate().take(nch) {
                // a distinct value per channel, per band and per data point, so
                // that a branch reading the wrong one of the three shows up
                for (pb, v) in slot.iter_mut().enumerate().take(4) {
                    *v = row[ch] + pb as u8 + d as u8;
                }
            }
            mtx_q.push(m);
        }
        Joc {
            dmx_config: 3,
            num_channels: nch,
            num_objects: 1,
            ext_config: 0,
            clipgain: 1.0,
            seq_count: 1,
            objects: vec![Some(JocObject {
                num_bands: 3,
                sparse: false,
                quant_idx: 1,
                slope,
                num_dpoints,
                offset_ts,
                mtx_q,
                sparse_channel: None,
            })],
            bits_used: 0,
            padding_zero: true,
        }
    }

    /// The matrix coefficient the decoder would apply to channel `ch`,
    /// subband `sb`, time slot `ts`, read out through the public interface: a
    /// unit impulse in one channel comes out of `reconstruct` as that
    /// coefficient.
    fn coefficient(d: &JocDecoder, ts: usize, ch: usize, sb: usize, nch: usize) -> f64 {
        let mut input = vec![[Complex::default(); BANDS]; nch];
        input[ch][sb] = Complex { re: 1.0, im: 0.0 };
        let mut out = vec![[Complex::default(); BANDS]; 1];
        d.reconstruct(ts, &input, &mut out);
        out[0][sb].re
    }

    /// Clause 6.6.5, pseudo-code 6, restated here rather than shared with the
    /// implementation, so that the test can disagree with it.
    fn expected(
        branch: (Slope, usize, [u8; 2]),
        points: (f64, f64, f64),
        ts: usize,
        num_ts: usize,
    ) -> f64 {
        let (slope, num_dpoints, offset_ts) = branch;
        let (prev, d0, d1) = points;
        match slope {
            Slope::Smooth if num_dpoints == 1 => {
                prev + (ts as f64 + 1.0) * (d0 - prev) / num_ts as f64
            }
            Slope::Smooth => {
                let ts_2 = num_ts / 2;
                if ts < ts_2 {
                    prev + (ts as f64 + 1.0) * (d0 - prev) / ts_2 as f64
                } else {
                    d0 + (ts - ts_2 + 1) as f64 * (d1 - d0) / (num_ts - ts_2) as f64
                }
            }
            Slope::Steep if num_dpoints == 1 => {
                if ts < usize::from(offset_ts[0]) {
                    prev
                } else {
                    d0
                }
            }
            Slope::Steep => {
                if ts < usize::from(offset_ts[0]) {
                    prev
                } else if ts < usize::from(offset_ts[1]) {
                    d0
                } else {
                    d1
                }
            }
        }
    }

    /// All four branches of clause 6.6.5, pseudo-code 6, read exactly as the
    /// clause prints them.
    ///
    /// Two of them -- smooth and steep with two data points -- have never
    /// occurred in any real stream measured: 0 of 32 493 245 object updates in
    /// 31 JOC streams. No encoder on hand emits them either; DEE 5.2.1 writes
    /// one data point at every data rate it offers, even for objects moving
    /// four times per frame. So this is the only thing holding them: it proves
    /// the implementation matches the printed pseudo-code, and not that Dolby
    /// agrees with the printed pseudo-code -- on the steep switch point it
    /// does not, which is why the reading is selectable and why the default is
    /// the other one. See `steep_switches_one_slot_before_the_printed_reading`.
    #[test]
    fn every_interpolation_branch_matches_pseudocode_6() {
        const NCH: usize = 5;
        const NUM_TS: usize = 24;
        let cases: [(Slope, usize, [u8; 2]); 4] = [
            (Slope::Smooth, 1, [0, 0]),
            (Slope::Smooth, 2, [0, 0]),
            (Slope::Steep, 1, [7, 0]),
            (Slope::Steep, 2, [7, 17]),
        ];
        for (slope, num_dpoints, offset_ts) in cases {
            let mut d = JocDecoder::new(NCH, 1);
            d.set_steep_reading(SteepReading::AsPrinted);
            // the history is zero before the first frame (clause 6.6.5)
            let first = one_object(
                slope,
                num_dpoints,
                offset_ts,
                &[[120, 90, 150, 60, 100]; 2],
                NCH,
            );
            d.update(&first, NUM_TS);
            // a second frame, so `prev` is not zero for the branch under test
            let second = one_object(
                slope,
                num_dpoints,
                offset_ts,
                &[[80, 130, 40, 170, 96]; 2],
                NCH,
            );
            let prev_q = [120u8, 90, 150, 60, 100];
            let dp_q = [80u8, 130, 40, 170, 96];
            d.update(&second, NUM_TS);

            let map = band_map(3);
            for ts in 0..NUM_TS {
                for ch in 0..NCH {
                    for sb in [0usize, 17, 63] {
                        let pb = usize::from(map[sb]);
                        // the first frame's last data point is what `prev` holds
                        let prev = dequantize(prev_q[ch] + (num_dpoints - 1) as u8 + pb as u8, 1);
                        let d0 = dequantize(dp_q[ch] + pb as u8, 1);
                        let d1 = dequantize(dp_q[ch] + 1 + pb as u8, 1);
                        let want =
                            expected((slope, num_dpoints, offset_ts), (prev, d0, d1), ts, NUM_TS);
                        let got = coefficient(&d, ts, ch, sb, NCH);
                        assert!(
                            (got - want).abs() < 1e-12,
                            "{slope:?} with {num_dpoints} data point(s), ts {ts}, ch {ch}, sb {sb}: {got} != {want}"
                        );
                    }
                }
            }
        }
    }

    /// The default reading switches one slot before the printed one, at the
    /// slot `joc_offset_ts` names rather than the slot after it.
    ///
    /// `joc_offset_ts` is `joc_offset_ts_bits + 1` (clause 6.3.4.4) and `ts`
    /// counts from zero, so comparing the two directly, as pseudo-code 6
    /// prints it, is one-based against zero-based. Dolby's decoder switches
    /// at the named slot. Measured on four titles: Glass Onion's worst object
    /// moves from 25,24 dB to 49,93 dB against Dolby's object decoder and its
    /// median from 47,76 to 65,44; Shaun of the Dead's frame 581 from 23,51 dB
    /// to 58,90; the two titles whose steep objects fall outside the window do
    /// not move at all. A sweep of the switch position over +/-3 slots has its
    /// optimum exactly here and nowhere near it: the neighbouring positions
    /// give 27,09 and 25,24 dB where this one gives 49,93. See `docs/joc.md`.
    #[test]
    fn steep_switches_one_slot_before_the_printed_reading() {
        const NCH: usize = 5;
        const NUM_TS: usize = 24;
        const OFFSET: u8 = 7;
        let mut printed = JocDecoder::new(NCH, 1);
        printed.set_steep_reading(SteepReading::AsPrinted);
        let mut measured = JocDecoder::new(NCH, 1);
        for d in [&mut printed, &mut measured] {
            d.update(
                &one_object(
                    Slope::Steep,
                    1,
                    [OFFSET, 0],
                    &[[96, 96, 96, 96, 96]; 2],
                    NCH,
                ),
                NUM_TS,
            );
            d.update(
                &one_object(
                    Slope::Steep,
                    1,
                    [OFFSET, 0],
                    &[[40, 40, 40, 40, 40]; 2],
                    NCH,
                ),
                NUM_TS,
            );
        }
        let before = dequantize(96, 1);
        let after = dequantize(40, 1);
        assert!((before - after).abs() > 1e-9, "the test needs two values");
        for ts in 0..NUM_TS {
            let want_printed = if ts < usize::from(OFFSET) {
                before
            } else {
                after
            };
            let want_measured = if ts + 1 < usize::from(OFFSET) {
                before
            } else {
                after
            };
            assert!(
                (coefficient(&printed, ts, 0, 0, NCH) - want_printed).abs() < 1e-12,
                "printed reading, ts {ts}"
            );
            assert!(
                (coefficient(&measured, ts, 0, 0, NCH) - want_measured).abs() < 1e-12,
                "measured reading, ts {ts}"
            );
        }
        // the one slot the two readings disagree about
        let ts = usize::from(OFFSET) - 1;
        assert!((coefficient(&printed, ts, 0, 0, NCH) - before).abs() < 1e-12);
        assert!((coefficient(&measured, ts, 0, 0, NCH) - after).abs() < 1e-12);

        // Both offsets of the two-data-point branch take the same correction.
        // No stream measured carries that branch, so this pins the code and
        // not agreement with Dolby.
        const O0: u8 = 7;
        const O1: u8 = 17;
        let mut two = JocDecoder::new(NCH, 1);
        two.update(
            &one_object(Slope::Steep, 2, [O0, O1], &[[96, 96, 96, 96, 96]; 2], NCH),
            NUM_TS,
        );
        two.update(
            &one_object(
                Slope::Steep,
                2,
                [O0, O1],
                &[[40, 40, 40, 40, 40], [170, 170, 170, 170, 170]],
                NCH,
            ),
            NUM_TS,
        );
        // `one_object` adds the data-point index to every value, so the first
        // frame ends at 97 and the second frame's points are 40 and 171
        let (prev, dp0, dp1) = (dequantize(97, 1), dequantize(40, 1), dequantize(171, 1));
        for ts in 0..NUM_TS {
            let want = if ts + 1 < usize::from(O0) {
                prev
            } else if ts + 1 < usize::from(O1) {
                dp0
            } else {
                dp1
            };
            assert!(
                (coefficient(&two, ts, 0, 0, NCH) - want).abs() < 1e-12,
                "two data points, ts {ts}"
            );
        }
    }

    /// The two branches with two data points reach the second matrix and the
    /// two with one do not, which is the whole difference between them and the
    /// reason a test that only checked the endpoints would pass either way.
    #[test]
    fn the_second_data_point_is_reached_only_when_there_is_one() {
        const NCH: usize = 5;
        const NUM_TS: usize = 24;
        for (slope, offset_ts) in [(Slope::Smooth, [0u8, 0]), (Slope::Steep, [7, 17])] {
            let mut d = JocDecoder::new(NCH, 1);
            d.set_steep_reading(SteepReading::AsPrinted);
            d.update(
                &one_object(slope, 2, offset_ts, &[[96, 96, 96, 96, 96]; 2], NCH),
                NUM_TS,
            );
            // first data point 40, second 170, so the two are far apart
            let joc = one_object(
                slope,
                2,
                offset_ts,
                &[[40, 40, 40, 40, 40], [170, 170, 170, 170, 170]],
                NCH,
            );
            d.update(&joc, NUM_TS);
            let last = coefficient(&d, NUM_TS - 1, 0, 0, NCH);
            let second = dequantize(joc.objects[0].as_ref().unwrap().mtx_q[1][0][0], 1);
            assert!(
                (last - second).abs() < 1e-12,
                "{slope:?}: the last slot should hold the second data point, {last} != {second}"
            );
        }
    }

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
