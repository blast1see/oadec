//! The dither of the mantissas that got no bits.
//!
//! Clause 7.3.4 of ATSC A/52 puts a random value in the place of a mantissa
//! without bits when the `dithflag` of its channel is set, and leaves the
//! sequence to the decoder. Dolby's decoder draws the same values every time
//! it decodes a stream, so what it draws can be read off what it writes, and
//! was: `docs/eac3.md` says how, and
//! `docs/audit/evidence/engine-round-2026-11/51-the-dither-dolbys-decoder-draws.json`
//! holds the measurement. This module is that sequence, so that a decode
//! here and a decode there are the same samples where a stream has no bits
//! as well as where it has.
//!
//! What was measured:
//!
//! - **The generator.** A state of 16 bits, `x -> x * 47989 + 3`. A value
//!   takes two draws `a` and `b`, read as signed numbers, and is
//!   `(((a >> 1) + (b >> 1)) * 23170) >> 15` in units of `2^-15`: the mean
//!   of two draws times `1/sqrt(2)`, between -0,7071 and 0,7071, with the
//!   triangular spread of a sum of two.
//! - **The states.** Not one but sixty: for each of the six blocks of a
//!   frame and each of the five full-bandwidth channels, one for the
//!   channel's own coefficients and one for those it takes from the coupling
//!   channel. A state moves only by what is drawn from it.
//! - **AC-3, and a dependent substream.** The states are seeded once and go
//!   on from frame to frame: block 0 of a channel takes up where block 0 of
//!   the frame before left off. The seeds are the run of a second generator,
//!   `g -> g * 47989 + 1` from 1: block by block and channel by channel, the
//!   next value for the own state and the one after for the coupled.
//! - **An independent E-AC-3 substream.** Every frame seeds all sixty anew,
//!   from the 16 bits that end 18 bits before the end of the frame (what
//!   stands before `auxdatae`, the bit after it and the CRC; the encoders at
//!   hand write a different word there in every frame). The second generator
//!   runs from that word: through the thirty own states, block by block and
//!   channel by channel, then through the thirty coupled ones.
//! - **The adaptive hybrid transform.** A coefficient without bits is not
//!   dithered before the transform across the blocks. In each block it takes
//!   the next draw of that block's own state, `2 * ((sum * 28378) >> 15)`
//!   where `sum` is `(a >> 1) + (b >> 1)`: the ordinary value times
//!   `sqrt(6)`, which is the size the transform would have given six of
//!   them.
//!
//! The LFE channel is not dithered, and a channel whose `dithflag` is clear
//! draws nothing.

use crate::frame::{MAX_BLOCKS, MAX_FBW};

/// The multiplier of both generators.
const MULTIPLIER: u16 = 47989;
/// What a draw adds.
const DRAW: u16 = 3;
/// What a step of the seed generator adds.
const SEED: u16 = 1;
/// `1/sqrt(2)` in units of `2^-15`.
const SCALE: i32 = 23170;
/// `sqrt(3)/2` in units of `2^-15`.
const HYBRID_SCALE: i32 = 28378;
/// A state for the channel's own coefficients, one for those it is coupled
/// into.
const OWN: usize = 0;
const COUPLED: usize = 1;

/// The states the dither of one substream is drawn from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dither {
    /// By block of the frame, by full-bandwidth channel, own and coupled.
    states: [[[u16; 2]; MAX_FBW]; MAX_BLOCKS],
}

impl Default for Dither {
    fn default() -> Self {
        Self::new()
    }
}

const fn seed_step(g: u16) -> u16 {
    g.wrapping_mul(MULTIPLIER).wrapping_add(SEED)
}

impl Dither {
    /// The states a decode starts from, which an AC-3 stream and a dependent
    /// substream keep to the end.
    #[must_use]
    pub const fn new() -> Self {
        let mut states = [[[0u16; 2]; MAX_FBW]; MAX_BLOCKS];
        let mut g: u16 = 1;
        let mut blk = 0;
        while blk < MAX_BLOCKS {
            let mut ch = 0;
            while ch < MAX_FBW {
                g = seed_step(g);
                states[blk][ch][OWN] = g;
                g = seed_step(g);
                states[blk][ch][COUPLED] = g;
                ch += 1;
            }
            blk += 1;
        }
        Self { states }
    }

    /// Seeds every state anew for a frame of an independent E-AC-3
    /// substream, from the 16 bits that end 18 bits before its end.
    pub fn seed_from_frame(&mut self, word: u16) {
        let mut g = word;
        for part in [OWN, COUPLED] {
            for block in &mut self.states {
                for channel in block.iter_mut() {
                    g = seed_step(g);
                    channel[part] = g;
                }
            }
        }
    }

    /// Two draws of a state, as the sum of their halves: between -32768 and
    /// 32767.
    fn sum(&mut self, blk: usize, ch: usize, part: usize) -> i32 {
        let state = &mut self.states[blk][ch][part];
        let a = state.wrapping_mul(MULTIPLIER).wrapping_add(DRAW);
        let b = a.wrapping_mul(MULTIPLIER).wrapping_add(DRAW);
        *state = b;
        (i32::from(a as i16) >> 1) + (i32::from(b as i16) >> 1)
    }

    /// The value for a coefficient without bits of channel `ch` in block
    /// `blk` of its frame, to be scaled by the exponent.
    pub fn own(&mut self, blk: usize, ch: usize) -> f64 {
        f64::from((self.sum(blk, ch, OWN) * SCALE) >> 15) / 32768.0
    }

    /// The value for a coefficient without bits of the coupling channel, as
    /// channel `ch` takes it in block `blk`.
    pub fn coupled(&mut self, blk: usize, ch: usize) -> f64 {
        f64::from((self.sum(blk, ch, COUPLED) * SCALE) >> 15) / 32768.0
    }

    /// The value for a coefficient without bits of a channel coded with the
    /// adaptive hybrid transform, in block `blk`.
    pub fn hybrid(&mut self, blk: usize, ch: usize) -> f64 {
        f64::from(2 * ((self.sum(blk, ch, OWN) * HYBRID_SCALE) >> 15)) / 32768.0
    }
}

#[cfg(test)]
mod tests {
    use super::Dither;

    /// A value as the whole number Dolby's decoder wrote: its units are
    /// `2^-15`.
    fn whole(v: f64) -> i32 {
        let scaled = v * 32768.0;
        assert!(
            (scaled - scaled.round()).abs() < 1e-9,
            "{v} is not a multiple of 2^-15"
        );
        scaled.round() as i32
    }

    fn own(d: &mut Dither, blk: usize, ch: usize, count: usize) -> Vec<i32> {
        (0..count).map(|_| whole(d.own(blk, ch))).collect()
    }

    fn coupled(d: &mut Dither, blk: usize, ch: usize, count: usize) -> Vec<i32> {
        (0..count).map(|_| whole(d.coupled(blk, ch))).collect()
    }

    fn hybrid(d: &mut Dither, blk: usize, ch: usize, count: usize) -> Vec<i32> {
        (0..count).map(|_| whole(d.hybrid(blk, ch))).collect()
    }

    // A stream of two channels in which no mantissa has a bit and every
    // exponent is 8 was written and put to Dolby's decoder; what it wrote,
    // taken back through the transform and times 2^8, is these numbers. The
    // decode leaves out the first block, so the first it shows whole is
    // block 1 of the first frame.

    #[test]
    fn a_block_draws_what_dolbys_decoder_wrote_for_it() {
        let mut d = Dither::new();
        assert_eq!(
            own(&mut d, 3, 0, 12),
            [
                -13519, 8304, 3437, -1418, 3785, 6646, 12867, -5881, -1900, 3722, -15173, -2922
            ]
        );
        assert_eq!(
            own(&mut d, 3, 1, 12),
            [
                -19881, -5990, -17610, -2700, 10411, -90, 14221, -19156, 1784, 199, 15821, 13806
            ]
        );
        assert_eq!(
            own(&mut d, 5, 0, 12),
            [
                1135, -9196, 16960, -5205, -3373, 14399, 233, -346, -16389, 4867, 6857, 3246
            ]
        );
        assert_eq!(
            own(&mut d, 5, 1, 12),
            [
                -14210, 7533, 1773, 13311, 18163, -12724, -3414, -10475, 2930, -941, -1180, 6467
            ]
        );
    }

    #[test]
    fn a_block_goes_on_in_the_next_frame_from_where_it_stopped() {
        // 253 coefficients a channel: what block 0 and block 3 of the second
        // frame start with
        let mut d = Dither::new();
        for (blk, ch) in [(0, 0), (0, 1), (3, 0), (3, 1)] {
            let _ = own(&mut d, blk, ch, 253);
        }
        assert_eq!(
            own(&mut d, 0, 0, 12),
            [
                -3042, 7376, 12828, -17913, 6297, -8039, 2708, -1378, -7353, -1550, 1458, -592
            ]
        );
        assert_eq!(
            own(&mut d, 0, 1, 12),
            [
                -7210, 21163, 854, 7075, 11497, 15478, 21099, 2204, 14455, -18989, 11115, 411
            ]
        );
        assert_eq!(
            own(&mut d, 3, 0, 12),
            [
                6884, -7622, -8720, -848, -10884, -1269, -14813, -18299, -956, -3424, -7691, 10771
            ]
        );
        assert_eq!(
            own(&mut d, 3, 1, 12),
            [
                963, -385, -5251, 13064, -4903, -2043, 4178, 8601, -10588, -4967, 22478, -11611
            ]
        );
    }

    #[test]
    fn a_state_moves_only_by_what_is_drawn_from_it() {
        // the same stream with 73 coefficients a channel: block 0 of the
        // second frame is 73 values on, whatever the other blocks drew
        let mut d = Dither::new();
        let _ = own(&mut d, 0, 0, 73);
        let _ = own(&mut d, 1, 0, 200);
        let _ = coupled(&mut d, 0, 0, 40);
        assert_eq!(
            own(&mut d, 0, 0, 8),
            [-10051, -13797, 3918, 8974, -3065, -12739, -5659, -1463]
        );
    }

    #[test]
    fn the_states_a_decode_starts_from_are_the_ones_measured() {
        // the first value of a block says which state it started from; these
        // are the states of the first frame of the stream above, and the
        // coupled ones of a stream of two coupled channels
        let d = Dither::new();
        let own: Vec<[u16; 2]> = (0..6)
            .map(|blk| [d.states[blk][0][0], d.states[blk][1][0]])
            .collect();
        assert_eq!(
            own,
            [
                [47990, 30780],
                [12916, 28746],
                [4546, 42536],
                [3296, 29014],
                [52046, 32084],
                [51340, 43426]
            ]
        );
        let coupled: Vec<[u16; 2]> = (1..6)
            .map(|blk| [d.states[blk][0][1], d.states[blk][1][1]])
            .collect();
        assert_eq!(
            coupled,
            [
                [51973, 24531],
                [54187, 10313],
                [33377, 40527],
                [58535, 41829],
                [60413, 56587]
            ]
        );
    }

    #[test]
    fn the_seeds_run_on_through_the_other_channels_and_blocks() {
        // one generator seeds them all: `g -> g * 47989 + 1` from 1, a step
        // for the own state of a channel and a step for its coupled one
        let d = Dither::new();
        let mut g: u16 = 1;
        for blk in 0..6 {
            for ch in 0..5 {
                for part in 0..2 {
                    g = g.wrapping_mul(47989).wrapping_add(1);
                    assert_eq!(d.states[blk][ch][part], g, "block {blk} channel {ch}");
                }
            }
        }
    }

    #[test]
    fn a_coupled_coefficient_draws_from_the_channels_second_state() {
        // block 1 of the first frame of an AC-3 stream of two coupled
        // channels: Dolby's decode less a decode without dither, over the
        // exponent and the coupling coordinate
        let mut d = Dither::new();
        assert_eq!(coupled(&mut d, 1, 0, 5), [6429, -12132, 18781, 8931, -862]);
        assert_eq!(
            coupled(&mut d, 1, 1, 5),
            [6023, -13816, -7262, -7348, -2943]
        );
        // and leaves the own state alone
        assert_eq!(d.states[1][0][0], 12916);
    }

    #[test]
    fn a_frame_of_an_independent_substream_seeds_every_state_from_its_tail_word() {
        // the states the blocks of two frames of a 5.1 E-AC-3 stream started
        // from, read off Dolby's decode; the words are the 16 bits that end
        // 18 bits before the end of those frames
        let mut d = Dither::new();
        d.seed_from_frame(0x17F4);
        assert_eq!(
            d.states[1].map(|ch| ch[0]),
            [3558, 23583, 48940, 33565, 6978]
        );
        assert_eq!(
            d.states[2].map(|ch| ch[0]),
            [43819, 41896, 33737, 3550, 32887]
        );
        d.seed_from_frame(0x150A);
        assert_eq!(
            d.states[0].map(|ch| ch[0]),
            [60307, 2864, 11505, 38182, 60511]
        );
        assert_eq!(
            d.states[1].map(|ch| ch[0]),
            [27756, 29021, 48770, 1899, 36072]
        );
        assert_eq!(
            d.states[2].map(|ch| ch[0]),
            [56841, 3358, 59575, 2212, 48885]
        );
    }

    #[test]
    fn the_coupled_states_of_a_frame_come_after_all_its_own() {
        // a stream of two coupled channels whose first frame ends in the
        // word 0x31EF: the coupled runs of blocks 1 and 4 were found 36, 37
        // and 51, 52 steps of the seed generator after the word
        let mut d = Dither::new();
        d.seed_from_frame(0x31EF);
        assert_eq!([d.states[1][0][1], d.states[1][1][1]], [35163, 16280]);
        assert_eq!([d.states[4][0][1], d.states[4][1][1]], [24738, 32779]);
    }

    #[test]
    fn a_frame_seeded_anew_forgets_what_was_drawn() {
        let mut d = Dither::new();
        d.seed_from_frame(0x150A);
        let first = own(&mut d, 2, 3, 6);
        let _ = own(&mut d, 2, 3, 100);
        d.seed_from_frame(0x150A);
        assert_eq!(own(&mut d, 2, 3, 6), first);
    }

    #[test]
    fn a_coefficient_of_the_hybrid_transform_takes_the_blocks_draw_sqrt_6_as_large() {
        // the third channel of a frame of a 5.1 E-AC-3 stream whose tail
        // word is 0x150A: what Dolby's decode has, less a decode without
        // dither, in the coefficients without bits of each block
        let mut d = Dither::new();
        d.seed_from_frame(0x150A);
        assert_eq!(
            hybrid(&mut d, 0, 2, 10),
            [
                -1676, 31064, -7890, 11602, 1524, -17946, 10608, 54156, -24982, -29268
            ]
        );
        assert_eq!(
            hybrid(&mut d, 1, 2, 10),
            [
                2494, -22730, 1738, -13008, 16170, -10272, -48214, 19180, 26742, -32844
            ]
        );
        assert_eq!(
            hybrid(&mut d, 3, 2, 10),
            [
                -35220, -35862, -9648, 52064, 3618, -15090, 9902, 9206, -13854, 2576
            ]
        );
        assert_eq!(
            hybrid(&mut d, 5, 2, 10),
            [
                16716, 22256, 37134, -45288, -17716, 2566, 13564, 868, -19782, -16680
            ]
        );
    }

    #[test]
    fn the_hybrid_draw_is_the_ordinary_one_of_the_same_state() {
        // one state, two sizes: a value is the same two draws either way
        let mut plain = Dither::new();
        let mut wide = Dither::new();
        for _ in 0..2000 {
            let a = plain.own(4, 2);
            let b = wide.hybrid(4, 2);
            assert!(
                (b - a * 6f64.sqrt()).abs() < 4.0 / 32768.0,
                "{b} is not {a} times sqrt(6)"
            );
        }
        assert_eq!(plain.states, wide.states);
    }

    #[test]
    fn the_values_are_spread_as_a_sum_of_two_draws() {
        // within 1/sqrt(2), a mean of nought, and the power of an even
        // spread over a half: 1/12
        let mut d = Dither::new();
        let vals: Vec<f64> = (0..20_000).map(|_| d.own(0, 0)).collect();
        let most = vals.iter().fold(0f64, |m, v| m.max(v.abs()));
        assert!(most <= 23170.0 / 32768.0, "largest {most}");
        assert!(most > 0.6, "largest {most}");
        let mean = vals.iter().sum::<f64>() / vals.len() as f64;
        assert!(mean.abs() < 0.01, "mean {mean}");
        let power = vals.iter().map(|v| v * v).sum::<f64>() / vals.len() as f64;
        assert!((power - 1.0 / 12.0).abs() < 0.003, "power {power}");
        // a triangle: half the values within 0,2071 of nought
        let near = vals.iter().filter(|v| v.abs() < 0.2071).count() as f64 / vals.len() as f64;
        assert!(
            (near - 0.5).abs() < 0.02,
            "{near} of the values near nought"
        );
    }
}
