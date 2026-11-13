//! Parametric bit allocation (ETSI TS 102 366 clause 6.2.2 and, for AHT
//! channels, clause E.2.4.3).
//!
//! The routine is defined in fixed-point integer arithmetic and reproduced
//! step by step: psd mapping, banded log-addition, excitation with the
//! low-frequency compensation, masking curve, delta bit allocation and the
//! final table look-up into `baptab` or `hebaptab`.

use crate::tables::{
    BAND_SIZE, BAND_START, BAPTAB, DB_PER_BIT, FAST_DECAY, FAST_GAIN, FLOOR, HEBAPTAB, HTH, LATAB,
    MASKTAB, SLOW_DECAY, SLOW_GAIN,
};

/// Block-level allocation parameters (`sdcycod` .. `floorcod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocParams {
    pub sdecay: i32,
    pub fdecay: i32,
    pub sgain: i32,
    pub dbknee: i32,
    pub floor: i32,
    /// Index into the hearing threshold table (`fscod`, or `fscod2` for the
    /// reduced sample rates).
    pub hth_index: usize,
}

impl AllocParams {
    /// Looks the codes up in tables 6.6 to 6.10.
    #[must_use]
    pub fn from_codes(
        sdcycod: u8,
        fdcycod: u8,
        sgaincod: u8,
        dbpbcod: u8,
        floorcod: u8,
        hth_index: usize,
    ) -> Self {
        Self {
            sdecay: SLOW_DECAY[usize::from(sdcycod & 3)],
            fdecay: FAST_DECAY[usize::from(fdcycod & 3)],
            sgain: SLOW_GAIN[usize::from(sgaincod & 3)],
            dbknee: DB_PER_BIT[usize::from(dbpbcod & 3)],
            floor: FLOOR[usize::from(floorcod & 7)],
            hth_index: hth_index.min(2),
        }
    }
}

/// Delta bit allocation segments of one channel (clause 6.2.2.6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeltaBa {
    pub offsets: Vec<u8>,
    pub lengths: Vec<u8>,
    pub values: Vec<u8>,
}

/// The SNR offset of a channel: `((csnroffst - 15) << 4 + fsnroffst) << 2`.
#[must_use]
pub fn snr_offset(csnroffst: u8, fsnroffst: u8) -> i32 {
    (((i32::from(csnroffst) - 15) << 4) + i32::from(fsnroffst)) << 2
}

/// Fast gain from `fgaincod` (table 6.11).
#[must_use]
pub fn fast_gain(fgaincod: u8) -> i32 {
    FAST_GAIN[usize::from(fgaincod & 7)]
}

/// Initial leak values of the coupling channel: `(fastleak, slowleak)`.
#[must_use]
pub fn coupling_leak(cplfleak: u8, cplsleak: u8) -> (i32, i32) {
    (
        (i32::from(cplfleak) << 8) + 768,
        (i32::from(cplsleak) << 8) + 768,
    )
}

fn logadd(a: i32, b: i32) -> i32 {
    let c = a - b;
    let address = ((c.abs() >> 1).min(255)) as usize;
    if c >= 0 {
        a + LATAB[address]
    } else {
        b + LATAB[address]
    }
}

fn calc_lowcomp(a: i32, b0: i32, b1: i32, bin: usize) -> i32 {
    if bin < 7 {
        if b0 + 256 == b1 {
            384
        } else if b0 > b1 {
            (a - 64).max(0)
        } else {
            a
        }
    } else if bin < 20 {
        if b0 + 256 == b1 {
            320
        } else if b0 > b1 {
            (a - 64).max(0)
        } else {
            a
        }
    } else {
        (a - 128).max(0)
    }
}

/// Computes the bit allocation pointers of one channel over bins
/// `start..end` from its exponents.
///
/// `leak` gives the initial fast/slow leak of the coupling channel (for fbw
/// and LFE channels the excitation starts from the low-frequency
/// compensation instead). With `high_efficiency` the AHT pointer table is
/// used and the result ranges over `0..=19`.
#[allow(
    clippy::too_many_arguments,
    reason = "the routine has this many inputs"
)]
pub fn compute(
    params: &AllocParams,
    start: usize,
    end: usize,
    exps: &[u8],
    fgain: i32,
    snroffset: i32,
    leak: Option<(i32, i32)>,
    dba: Option<&DeltaBa>,
    high_efficiency: bool,
    bap: &mut [u8],
) {
    debug_assert!(end <= 256 && start <= end);
    if start >= end {
        return;
    }
    // 6.2.2.2 exponent mapping into psd
    let mut psd = [0i32; 256];
    for bin in start..end {
        psd[bin] = 3072 - (i32::from(exps[bin]) << 7);
    }

    // 6.2.2.3 psd integration
    let mut bndpsd = [0i32; 50];
    let bndstrt = usize::from(MASKTAB[start]);
    let bndend = usize::from(MASKTAB[end - 1]) + 1;
    {
        let mut j = start;
        let mut k = bndstrt;
        loop {
            let lastbin = (usize::from(BAND_START[k]) + usize::from(BAND_SIZE[k])).min(end);
            bndpsd[k] = psd[j];
            j += 1;
            while j < lastbin {
                bndpsd[k] = logadd(bndpsd[k], psd[j]);
                j += 1;
            }
            k += 1;
            if end <= lastbin {
                break;
            }
        }
    }

    // 6.2.2.4 excitation function
    //
    // Two things here are not in the pseudo-code of the clause. Both show only
    // where enhanced coupling begins at its lowest sub-band: the coupling
    // channel begins at band 13 then, and the channels coupled into it end
    // there. The streams of an encoder that does this decode only with both,
    // and then to what the Dolby decoder makes of them (`docs/eac3.md`).
    //
    // - The low-frequency compensation runs for the coupling channel as it
    //   does for any other, while its bands are below 22. The clause starts
    //   the coupling channel in the loop that has none, which is the same
    //   thing for one that begins at band 31 or later, as every coupling
    //   channel of standard coupling does.
    // - The compensation looks at the band after the one it is at, and the
    //   last band of a channel has none after it. The clause says so of the
    //   LFE channel, the one channel of AC-3 that ends below band 22.
    let mut excite = [0i32; 50];
    let (mut fastleak, mut slowleak) = leak.unwrap_or((0, 0));
    let mut lowcomp = 0;
    let ahead = |bin: usize| bin + 1 < bndend;
    let begin = if bndstrt == 0 {
        // fbw and lfe channels
        lowcomp = calc_lowcomp(lowcomp, bndpsd[0], bndpsd[1], 0);
        excite[0] = bndpsd[0] - fgain - lowcomp;
        lowcomp = calc_lowcomp(lowcomp, bndpsd[1], bndpsd[2], 1);
        excite[1] = bndpsd[1] - fgain - lowcomp;
        let mut b = 7;
        for bin in 2..7 {
            if ahead(bin) {
                lowcomp = calc_lowcomp(lowcomp, bndpsd[bin], bndpsd[bin + 1], bin);
            }
            fastleak = bndpsd[bin] - fgain;
            slowleak = bndpsd[bin] - params.sgain;
            excite[bin] = fastleak - lowcomp;
            if ahead(bin) && bndpsd[bin] <= bndpsd[bin + 1] {
                b = bin + 1;
                break;
            }
        }
        b
    } else {
        // coupling channel
        bndstrt
    };
    for bin in begin..bndend.min(22) {
        if ahead(bin) {
            lowcomp = calc_lowcomp(lowcomp, bndpsd[bin], bndpsd[bin + 1], bin);
        }
        fastleak -= params.fdecay;
        fastleak = fastleak.max(bndpsd[bin] - fgain);
        slowleak -= params.sdecay;
        slowleak = slowleak.max(bndpsd[bin] - params.sgain);
        excite[bin] = (fastleak - lowcomp).max(slowleak);
    }
    // from band 22 on the compensation has run out, whatever it was
    for bin in begin.max(22)..bndend {
        fastleak -= params.fdecay;
        fastleak = fastleak.max(bndpsd[bin] - fgain);
        slowleak -= params.sdecay;
        slowleak = slowleak.max(bndpsd[bin] - params.sgain);
        excite[bin] = fastleak.max(slowleak);
    }

    // 6.2.2.5 masking curve
    let mut mask = [0i32; 50];
    for bin in bndstrt..bndend {
        if bndpsd[bin] < params.dbknee {
            excite[bin] += (params.dbknee - bndpsd[bin]) >> 2;
        }
        mask[bin] = excite[bin].max(HTH[params.hth_index][bin]);
    }

    // 6.2.2.6 delta bit allocation
    if let Some(d) = dba {
        let mut band = 0usize;
        for seg in 0..d.offsets.len() {
            band += usize::from(d.offsets[seg]);
            let v = i32::from(d.values[seg]);
            let delta = if v >= 4 { (v - 3) << 7 } else { (v - 4) << 7 };
            for _ in 0..d.lengths[seg] {
                if band < 50 {
                    mask[band] += delta;
                }
                band += 1;
            }
        }
    }

    // 6.2.2.7 bit allocation pointers
    let table: &[u8; 64] = if high_efficiency { &HEBAPTAB } else { &BAPTAB };
    let mut i = start;
    let mut j = bndstrt;
    loop {
        let lastbin = (usize::from(BAND_START[j]) + usize::from(BAND_SIZE[j])).min(end);
        mask[j] -= snroffset;
        mask[j] -= params.floor;
        if mask[j] < 0 {
            mask[j] = 0;
        }
        mask[j] &= 0x1fe0;
        mask[j] += params.floor;
        while i < lastbin {
            let address = ((psd[i] - mask[j]) >> 5).clamp(0, 63) as usize;
            bap[i] = table[address];
            i += 1;
        }
        j += 1;
        if end <= lastbin {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snr_offset_formula() {
        assert_eq!(snr_offset(15, 0), 0);
        assert_eq!(snr_offset(16, 0), 64);
        assert_eq!(snr_offset(15, 1), 4);
        assert_eq!(snr_offset(0, 0), -960);
    }

    #[test]
    fn logadd_is_commutative_and_bounded() {
        assert_eq!(logadd(1000, 1000), 1000 + 0x40);
        assert_eq!(logadd(1000, 0), 1000);
        assert_eq!(logadd(0, 1000), 1000);
        assert_eq!(logadd(1000, 990), logadd(990, 1000));
    }

    #[test]
    fn silence_gets_no_bits_and_a_loud_flat_spectrum_gets_many() {
        let params = AllocParams::from_codes(2, 1, 1, 2, 7, 0);
        let mut bap = [0u8; 256];
        // exponents 24 everywhere: the lowest possible level
        compute(
            &params,
            0,
            253,
            &[24u8; 256],
            fast_gain(4),
            snr_offset(15, 0),
            None,
            None,
            false,
            &mut bap,
        );
        assert!(bap.iter().all(|&b| b == 0), "silence allocated bits");
        // exponents 0 everywhere with a generous SNR offset: every bin gets bits
        compute(
            &params,
            0,
            253,
            &[0u8; 256],
            fast_gain(4),
            snr_offset(40, 0),
            None,
            None,
            false,
            &mut bap,
        );
        assert!(
            bap[..253].iter().all(|&b| b > 0),
            "loud spectrum left bins unallocated"
        );
        assert!(bap[..253].iter().all(|&b| b <= 15));
        // the high-efficiency table ranges further
        compute(
            &params,
            0,
            253,
            &[0u8; 256],
            fast_gain(4),
            snr_offset(40, 0),
            None,
            None,
            true,
            &mut bap,
        );
        assert!(bap[..253].iter().all(|&b| b <= 19));
    }

    /// The allocation parameters of the two blocks below, as their stream
    /// codes them: `sdcycod` 2, `fdcycod` 1, `sgaincod` 1, `floorcod` 7.
    fn of_the_stream(dbpbcod: u8) -> AllocParams {
        AllocParams::from_codes(2, 1, 1, dbpbcod, 7, 0)
    }

    /// A coupling channel that begins at band 13, as enhanced coupling from
    /// its lowest sub-band makes it: the exponents and the parameters of one
    /// block of an encoder's stream, and the pointers its mantissas are
    /// packed with. Bands 16 and 17 are 12 dB apart, which sets the
    /// low-frequency compensation, and the bands after them take more bits
    /// for it. The clause runs no compensation for a coupling channel; read
    /// that way the block ends in a mantissa code that does not exist.
    #[test]
    fn a_coupling_channel_that_begins_low_is_compensated_at_low_frequencies() {
        #[rustfmt::skip]
        let coded: [u8; 72] = [
            14, 14, 14, 14, 12, 12, 12, 12, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
            13, 13, 13, 13, 13, 13, 12, 12, 12, 12, 13, 13, 13, 13, 14, 14, 14, 14,
            13, 13, 13, 13, 14, 14, 14, 14, 13, 13, 13, 13, 13, 13, 13, 13, 13, 13,
            13, 13, 13, 13, 13, 13, 14, 14, 14, 14, 14, 14, 14, 14, 13, 13, 13, 13,
        ];
        #[rustfmt::skip]
        let packed: [u8; 72] = [
            1, 1, 1, 1, 4, 4, 4, 5, 3, 3, 3, 3, 3, 3, 3, 2, 2, 2,
            2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 1, 1, 2, 2, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
            1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1,
        ];
        let mut exps = [0u8; 256];
        exps[13..85].copy_from_slice(&coded);
        let mut bap = [0u8; 256];
        compute(
            &of_the_stream(1),
            13,
            85,
            &exps,
            fast_gain(4),
            snr_offset(10, 9),
            Some(coupling_leak(0, 0)),
            None,
            false,
            &mut bap,
        );
        assert_eq!(bap[13..85], packed);
        // what the clause as printed gives for bands 17 to 21: 3, 3, 3, 3, 2
        assert_eq!(bap[17..22], [4, 4, 4, 5, 3]);
    }

    /// A channel that ends at band 13, where enhanced coupling takes over
    /// from its lowest sub-band: the exponents and the parameters of one
    /// block of an encoder's stream, and the pointers its mantissas are
    /// packed with. The compensation is still up at the last band, and it
    /// looks at the band after the one it is at; the last band has none, and
    /// reading a band of nothing there takes a step off the compensation and
    /// a bit off the last mantissa.
    #[test]
    fn the_last_band_of_a_channel_looks_at_no_band_after_it() {
        let coded: [u8; 13] = [5, 5, 5, 6, 6, 7, 5, 7, 6, 7, 7, 6, 8];
        let packed: [u8; 13] = [4, 4, 4, 4, 4, 7, 7, 5, 7, 6, 6, 6, 3];
        let mut exps = [0u8; 256];
        exps[..13].copy_from_slice(&coded);
        let mut bap = [0u8; 256];
        compute(
            &of_the_stream(3),
            0,
            13,
            &exps,
            fast_gain(4),
            snr_offset(12, 3),
            None,
            None,
            false,
            &mut bap,
        );
        assert_eq!(bap[..13], packed);
    }

    /// The LFE channel is the channel the clause names: it ends at band 7, and
    /// its last band is not looked past. Here the compensation is set at band
    /// 4, by the 12 dB the spectrum rises into band 5, and is still whole at
    /// band 6; were the band after that read as the nothing it is, a step
    /// would come off it and the last mantissa would lose bits.
    #[test]
    fn the_last_band_of_the_lfe_channel_is_the_one_the_clause_names() {
        for (coded, csnroffst, packed) in [
            ([6u8, 6, 6, 6, 6, 4, 4], 15u8, [7u8, 7, 7, 7, 10, 10, 10]),
            ([6, 6, 6, 6, 6, 4, 4], 20, [9, 9, 9, 9, 12, 12, 12]),
            ([8, 8, 8, 8, 6, 6, 6], 20, [9, 9, 9, 12, 12, 12, 12]),
            ([10, 10, 10, 8, 8, 8, 8], 22, [9, 9, 11, 13, 13, 13, 13]),
        ] {
            let mut exps = [24u8; 256];
            exps[..7].copy_from_slice(&coded);
            let mut bap = [0u8; 256];
            compute(
                &AllocParams::from_codes(2, 1, 1, 2, 7, 0),
                0,
                7,
                &exps,
                fast_gain(4),
                snr_offset(csnroffst, 0),
                None,
                None,
                false,
                &mut bap,
            );
            assert_eq!(bap[..7], packed, "{coded:?} at {csnroffst}");
        }
    }

    #[test]
    fn delta_allocation_raises_or_lowers_the_mask() {
        let params = AllocParams::from_codes(2, 1, 1, 2, 7, 0);
        let exps = [10u8; 256];
        let mut base = [0u8; 256];
        compute(
            &params,
            0,
            253,
            &exps,
            fast_gain(4),
            snr_offset(20, 0),
            None,
            None,
            false,
            &mut base,
        );
        // lower the mask by 3 x 6 dB in bands 10..=14: more bits
        let more = DeltaBa {
            offsets: vec![10],
            lengths: vec![5],
            values: vec![1],
        };
        let mut with = [0u8; 256];
        compute(
            &params,
            0,
            253,
            &exps,
            fast_gain(4),
            snr_offset(20, 0),
            None,
            Some(&more),
            false,
            &mut with,
        );
        assert!((10..15).all(|b| with[b] >= base[b]));
        assert!((10..15).any(|b| with[b] > base[b]));
        assert!((0..10).all(|b| with[b] == base[b]));
    }
}
