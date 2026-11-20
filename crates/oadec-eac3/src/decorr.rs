//! Experimental decorrelation of the coupled channels of an E-AC-3 stream.
//!
//! Above the coupling begin frequency the channels of a stream are the
//! coupling channel times a coordinate for each (clause 7.4), and the plain
//! decode makes nothing more of them. Dolby's decoders make more of them
//! when the stream is E-AC-3 (`docs/eac3.md`, "Decorrelation of the coupled
//! channels"): for a coupled coefficient `X` of the plain decode they write
//!
//! ```text
//! a X + b D(X),    a^2 + b^2 = 1
//! ```
//!
//! where `D` is a filter along the blocks on the same coefficient: a delay
//! of one block and the all-pass of three poles [`ALLPASS`]. Of a pair of
//! channels the second takes `b` with the other sign ([`Role`]).
//!
//! The filter and the signs were measured on Dolby's output and are taken
//! here as measured. The weight `a` was not: how Dolby's decoders choose it
//! is not known. This module computes `a` by a rule of its own, fitted to
//! the published description of such a decoder and to the measurements in
//! this repository: per block and per coupling sub-band, from how alike a
//! channel's content under the coupling begin frequency is to the sum of the
//! coupled channels there, falling with frequency, and with no randomness
//! in it. It is not Dolby's rule. The mode is experimental, it is off by
//! default, and a decode with it is not claimed to equal Dolby's.
//!
//! It runs only where the mapping of channels to filters was seen: an
//! E-AC-3 independent substream that is the whole programme
//! ([`ProgrammeScope::CoreOnly`]), in 2/0 or 3/2 with or without the LFE,
//! with standard coupling whose coupling channel is not coded with the
//! hybrid transform. Everywhere else the coefficients are left as they are
//! and [`NotApplied`] says why.
//!
//! The state of the filters, one per coupled bin of every channel, follows
//! three rules:
//!
//! - It is zero when a [`Decorrelator`] is made or reset, which the decoder
//!   does when it is made, reset, changes layout or is told another mode or
//!   scope, and in a frame it conceals.
//! - A bin that is not coupled in a block, because the block has no
//!   coupling, its channel is not in coupling, or the bin lies outside the
//!   coupling range, has its state set to zero in that block. When the
//!   coupling range changes, the bins that stay in it keep their state and
//!   those that come into it start from zero.
//! - In the first block of a run of blocks in which a bin is coupled, the
//!   bin has no past for `D` to give. It comes out as the plain decode has it, and the block only fills the
//!   filter. That is this decoder's choice, not something measured: `a X`
//!   alone would take the level of the bin down by `a^2` in that block.
//!   A decoder that starts in the middle of a stream therefore differs from
//!   one that started earlier by what is left in the filters, which falls by
//!   the largest pole radius, 0,7555, every block, and by what is left in
//!   the smoothing of the weight ([`SMOOTHING`]), which falls by half.
//!
//! # The weight
//!
//! Every constant of the rule below is an estimate of this decoder, fitted
//! or chosen as each says, and none is Dolby's. For a channel that takes
//! part, in one block:
//!
//! 1. Its share `rho` of what it has under the coupling begin frequency
//!    that the other coupled channels have too ([`share`]): over the bins
//!    from [`OWN_FIRST_BIN`] to the first coupled one, with `X` the channel
//!    and `S` the sum of the other coupled channels,
//!    `rho = sum(X^2 s) / sum(X^2)` with `s = clamp(X S / X^2, 0, 1)` for
//!    each bin. A channel with nothing there takes `rho = 1`. It goes bin
//!    by bin so that a channel whose only content is a tone the other
//!    channel has too takes 1 even when the other channel has noise of its
//!    own in other bins, as was measured.
//! 2. `rho` is smoothed along the blocks by [`SMOOTHING`].
//! 3. The angle of the pair, `a = cos t` and `b = sin t`, so that
//!    `a^2 + b^2 = 1` holds to the rounding, is
//!    `t = (1 - rho) t0(f) (1 + OTHERS (1 - rho_others))` ([`pair`]),
//!    where `t0` rises with the centre bin `f` of the coupling sub-band
//!    through [`ANGLE_FIRST`] and [`ANGLE_LAST`] ([`base_angle`]),
//!    `rho_others` is the mean smoothed `rho` of the other channels that
//!    take part, and `t` is held to [`ANGLE_MAX`].
//!
//! A property of this rule, which the mode does not hide: when a channel's
//! own region goes silent, its `rho` goes to 1 and `b` to 0 while `D` still
//! rings with a transient of the blocks before, so the tail of the
//! transient loses its filtered part and the channel keeps about `a^2` of
//! the transient's power. On the authored scene that is up to 3,1 dB in the
//! left channel (`docs/eac3.md`).

use crate::frame::{Coupling, N};

/// Whether the coupled channels are decorrelated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Decorrelation {
    /// The coupled region as the standard has it. The default.
    #[default]
    Off,
    /// The experimental decorrelation of this module, where it applies.
    Experimental,
}

/// The caller's statement of what programme a decoder's substream is part
/// of. A decoder sees only its own substream and cannot tell.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ProgrammeScope {
    /// Not said. Nothing is decorrelated. The default, so that the
    /// independent 5.1 core of a 7.1 programme cannot be decorrelated by
    /// accident.
    #[default]
    Unknown,
    /// The independent substream is the whole programme.
    CoreOnly,
    /// The programme has dependent substreams. How the channels of such a
    /// programme map to the filters was not worked out, so nothing is
    /// decorrelated.
    WithDependents,
}

/// Why the coefficients of a frame or block were left as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NotApplied {
    /// The mode is [`Decorrelation::Off`].
    Off,
    /// The scope is [`ProgrammeScope::Unknown`].
    ScopeUnknown,
    /// The scope is [`ProgrammeScope::WithDependents`].
    WithDependents,
    /// The stream is AC-3, which Dolby's decoders do not decorrelate.
    Ac3,
    /// The substream is a dependent one.
    DependentSubstream,
    /// The substream is an E-AC-3 substream converted from AC-3, which no
    /// stream at hand has.
    Converted,
    /// The channel layout is not one whose mapping was seen (2/0, 3/2, 3/2
    /// with the LFE).
    Layout,
    /// The frame is silence standing in for one that would not decode.
    Concealed,
    /// The decoder does not make samples ([`crate::Options::samples`]).
    NoSamples,
    /// No block of the frame uses coupling.
    NoCoupling,
    /// The coupling is enhanced coupling, whose bands are not sub-bands of
    /// twelve bins and which was not measured.
    EnhancedCoupling,
    /// The coupling channel is coded with the hybrid transform, whose
    /// coefficients without bits are not known for Dolby's decoders.
    Aht,
    /// No frame has been decoded since the mode was set.
    NothingDecoded,
}

impl NotApplied {
    /// A sentence for a person.
    #[must_use]
    pub fn reason(self) -> &'static str {
        match self {
            Self::Off => "decorrelation not applied: off",
            Self::ScopeUnknown => "decorrelation not applied: programme scope unknown",
            Self::WithDependents => {
                "decorrelation not applied: unproven mapping (programme has dependent substreams)"
            }
            Self::Ac3 => "decorrelation not applied: AC-3 stream",
            Self::DependentSubstream => {
                "decorrelation not applied: unproven mapping (dependent substream)"
            }
            Self::Converted => "decorrelation not applied: substream converted from AC-3",
            Self::Layout => "decorrelation not applied: unproven mapping (channel layout)",
            Self::Concealed => "decorrelation not applied: concealed frame",
            Self::NoSamples => "decorrelation not applied: no samples made",
            Self::NoCoupling => "decorrelation not applied: no coupling",
            Self::EnhancedCoupling => "decorrelation not applied: enhanced coupling",
            Self::Aht => "decorrelation not applied: coupling channel coded with AHT",
            Self::NothingDecoded => "decorrelation not applied: nothing decoded yet",
        }
    }
}

impl std::fmt::Display for NotApplied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

/// What was done to the coupled region of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// At least one block of the frame was decorrelated.
    Applied,
    /// Nothing was touched, for this reason.
    NotApplied(NotApplied),
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Applied => f.write_str("decorrelation applied (experimental)"),
            Self::NotApplied(why) => f.write_str(why.reason()),
        }
    }
}

/// The all-pass of `D`, `(c1, c2, c3)` of
/// `(c3 + c2 z^-1 + c1 z^-2 + z^-3) / (1 + c1 z^-1 + c2 z^-2 + c3 z^-3)`,
/// as fitted on Dolby's decodes (`docs/eac3.md`): a real pole at 0,7555 and
/// a pair at radius 0,7237 and 117,18 degrees either way.
pub const ALLPASS: [f64; 3] = [-0.0944, 0.0242, -0.3957];

/// Below this a value of the filter state is taken as zero, so that the
/// recursion never runs on subnormal numbers when the signal goes silent.
/// It is some 600 dB under full scale and makes no audible difference; it
/// is a fixed comparison, so the output stays deterministic.
pub const FLUSH: f64 = 1e-30;

/// The first bin of a channel's own region that the weight reads. Bins 0
/// and 1 (under 140 Hz) hold what a channel shares with the LFE more than
/// with the other channels, and are left out. A choice, not a fit.
pub const OWN_FIRST_BIN: usize = 2;

/// The share `rho` of a block is `SMOOTHING` times the smoothed share of
/// the block before plus the rest times its own, which keeps the weight
/// from jumping where the own region of one block says little. Dolby's
/// weight was seen to move by 0,02 to 0,10 from block to block
/// (`docs/eac3.md`). A choice, not a fit.
pub const SMOOTHING: f64 = 0.5;

/// The centre bin of the first coupling sub-band the angle was fitted on:
/// the stereo test signals of `tools/coupling_mix.py signals`, encoded by
/// FFmpeg, couple from bin 49 in eight sub-bands, whose centres are bins
/// 54,5 and 138,5 at the ends.
pub const FIT_FIRST_CENTRE: f64 = 54.5;
/// The centre bin of the last fitted sub-band; see [`FIT_FIRST_CENTRE`].
pub const FIT_LAST_CENTRE: f64 = 138.5;

/// The angle `t0`, in radians, of the first fitted sub-band: `acos 0,93`.
/// 0,93 is the mean direct weight Dolby's decoder gave the left channel of
/// the test signal `low-left` there, where only the left channel has noise
/// of its own under the coupling begin frequency and the right channel has
/// nothing but the shared tone (`docs/eac3.md`, evidence 52). The fit takes
/// the left channel's `rho` as 0 and the right channel's as 1. An estimate,
/// not Dolby's rule.
pub const ANGLE_FIRST: f64 = 0.376_383_482_317_728;
/// The same in the last fitted sub-band: `acos 0,81`, from the same
/// measurement. An estimate, not Dolby's rule.
pub const ANGLE_LAST: f64 = 0.626_644_211_640_741;

/// How much more a channel is decorrelated when the other channels have
/// content of their own too. Where both channels of the test signal
/// `low-both` have noise of their own the left channel's direct weight was
/// 0,72 to 0,61, where `low-left` gives 0,93 to 0,81: at the middle of the
/// eight sub-bands the angle is 1,677 times as large, so `OTHERS = 0,677`,
/// with every `rho` taken as 0. One figure for all sub-bands is a
/// simplification: the ratio is 2,0 in the first sub-band and 1,5 in the
/// last. An estimate, not Dolby's rule.
pub const OTHERS: f64 = 0.677;

/// The largest angle, 60 degrees, which holds `a` to 0,5 or more. Nothing
/// under 0,6 was measured. A choice, not a fit.
pub const ANGLE_MAX: f64 = std::f64::consts::FRAC_PI_3;

/// The angle `t0` of a coupling sub-band whose centre bin is `centre`:
/// along the line through the two fitted ends, never below zero.
#[must_use]
pub fn base_angle(centre: f64) -> f64 {
    let slope = (ANGLE_LAST - ANGLE_FIRST) / (FIT_LAST_CENTRE - FIT_FIRST_CENTRE);
    (ANGLE_FIRST + slope * (centre - FIT_FIRST_CENTRE)).max(0.0)
}

/// The pair `(a, b)` of a channel in a coupling sub-band, from its smoothed
/// share `rho`, the mean smoothed share of the other channels that take
/// part, and the centre bin of the sub-band. `a^2 + b^2 = 1` to the
/// rounding, and `b >= 0`: the sign is the channel's [`Role`].
#[must_use]
pub fn pair(rho: f64, rho_others: f64, centre: f64) -> (f64, f64) {
    let own = 1.0 - rho.clamp(0.0, 1.0);
    let others = 1.0 - rho_others.clamp(0.0, 1.0);
    let angle = (own * base_angle(centre) * (1.0 + OTHERS * others)).min(ANGLE_MAX);
    (angle.cos(), angle.sin())
}

/// The share `rho` of channel `ch`'s own region, the bins from
/// [`OWN_FIRST_BIN`] to the first coupled one, that the other coupled
/// full-bandwidth channels have too: step 1 of the rule of the module.
/// 1 for a channel with nothing there.
#[must_use]
#[allow(
    clippy::needless_range_loop,
    reason = "a bin addresses the same place in several channels"
)]
pub fn share(coeffs: &[[f64; N]], coupling: &Coupling, nfchans: usize, ch: usize) -> f64 {
    let mut energy = 0.0;
    let mut shared = 0.0;
    for bin in OWN_FIRST_BIN..coupling.start.min(N) {
        let x = coeffs[ch][bin];
        if x == 0.0 {
            continue;
        }
        let others: f64 = (0..nfchans)
            .filter(|&o| o != ch && coupling.is_coupled(o))
            .map(|o| coeffs[o][bin])
            .sum();
        let e = x * x;
        energy += e;
        shared += e * (x * others / e).clamp(0.0, 1.0);
    }
    if energy > 0.0 { shared / energy } else { 1.0 }
}

/// What a coded channel takes of the decorrelation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// `a X + b D(X)`.
    Plus,
    /// `a X - b D(X)`: the second channel of a pair.
    Minus,
    /// Left as it is: the centre and the LFE.
    Untouched,
}

/// The role of every coded channel (full-bandwidth channels in coded order,
/// then the LFE) for the layouts whose mapping was seen, `None` for every
/// other layout.
#[must_use]
pub fn roles(acmod: u8, lfeon: bool) -> Option<&'static [Role]> {
    use Role::{Minus, Plus, Untouched};
    match (acmod, lfeon) {
        (2, false) => Some(&[Plus, Minus]),
        (7, false) => Some(&[Plus, Untouched, Minus, Plus, Minus]),
        (7, true) => Some(&[Plus, Untouched, Minus, Plus, Minus, Untouched]),
        _ => None,
    }
}

/// `D` for one bin of one channel: the delay of one block and the all-pass
/// [`ALLPASS`], run along the blocks.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Delayed {
    /// The input of the block before, which the delay holds.
    held: f64,
    /// The all-pass state (transposed direct form II).
    state: [f64; 3],
    /// The filter has taken a block since it was last set to rest.
    primed: bool,
}

impl Delayed {
    /// A filter at rest.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: 0.0,
            state: [0.0; 3],
            primed: false,
        }
    }

    /// Takes the bin's coefficient of one block and gives `D` of it for this
    /// block, which depends only on the blocks before: the all-pass run on
    /// the coefficient of the block before, which the delay held.
    pub fn step(&mut self, x: f64) -> f64 {
        let [c1, c2, c3] = ALLPASS;
        let input = self.held;
        self.primed = true;
        self.held = if x.abs() < FLUSH { 0.0 } else { x };
        // Transposed direct form II of
        // (c3 + c2 z^-1 + c1 z^-2 + z^-3) / (1 + c1 z^-1 + c2 z^-2 + c3 z^-3).
        let y = c3 * input + self.state[0];
        self.state[0] = c2 * input - c1 * y + self.state[1];
        self.state[1] = c1 * input - c2 * y + self.state[2];
        self.state[2] = input - c3 * y;
        for v in &mut self.state {
            if v.abs() < FLUSH {
                *v = 0.0;
            }
        }
        if y.abs() < FLUSH { 0.0 } else { y }
    }

    /// Whether the filter is at rest: no past to give.
    #[must_use]
    pub fn is_at_rest(&self) -> bool {
        self.held == 0.0 && self.state == [0.0; 3]
    }

    /// Whether the filter has taken a block since it was made or set to
    /// rest, that is, whether the bin is past the first block of its run.
    #[must_use]
    pub fn is_primed(&self) -> bool {
        self.primed
    }
}

/// The decorrelation of one substream: the filter state of every bin of
/// every coded channel, carried from block to block.
#[derive(Debug, Clone)]
pub struct Decorrelator {
    filters: Vec<[Delayed; N]>,
    /// The smoothed share `rho` of every full-bandwidth channel, `None`
    /// where the channel took no part in the block before.
    shares: [Option<f64>; 5],
}

impl Default for Decorrelator {
    fn default() -> Self {
        Self::new()
    }
}

impl Decorrelator {
    /// A decorrelator whose filters are at rest.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
            shares: [None; 5],
        }
    }

    /// Sets every filter to rest and forgets the smoothed shares.
    pub fn reset(&mut self) {
        self.filters.clear();
        self.shares = [None; 5];
    }

    /// Whether one block of a stream whose layout is `(acmod, lfeon)` and
    /// whose coupling is `coupling` can be decorrelated, and if it can, the
    /// roles of its channels and the coupling.
    ///
    /// # Errors
    ///
    /// The reason it cannot.
    pub fn check(
        acmod: u8,
        lfeon: bool,
        coupling: Option<&Coupling>,
    ) -> Result<(&'static [Role], &Coupling), NotApplied> {
        let roles = roles(acmod, lfeon).ok_or(NotApplied::Layout)?;
        let coupling = coupling.ok_or(NotApplied::NoCoupling)?;
        if coupling.enhanced {
            return Err(NotApplied::EnhancedCoupling);
        }
        if coupling.aht {
            return Err(NotApplied::Aht);
        }
        Ok((roles, coupling))
    }

    /// Decorrelates the coupled region of one block in place, `coeffs` being
    /// the block's coefficients of every coded channel as the plain decode
    /// has them. Bins outside the coupled region, channels not in coupling
    /// and channels whose role is [`Role::Untouched`] are left as they are.
    /// Where the block cannot be decorrelated it is left as it is, every
    /// filter is set to rest, and the reason is returned.
    ///
    /// # Errors
    ///
    /// The reason the block was left as it is.
    pub fn block(
        &mut self,
        acmod: u8,
        lfeon: bool,
        coupling: Option<&Coupling>,
        coeffs: &mut [[f64; N]],
    ) -> Result<(), NotApplied> {
        let (roles, coupling) = match Self::check(acmod, lfeon, coupling) {
            Ok(checked) => checked,
            Err(why) => {
                self.reset();
                return Err(why);
            }
        };
        if coeffs.len() != roles.len() {
            self.reset();
            return Err(NotApplied::Layout);
        }
        if self.filters.len() != coeffs.len() {
            self.filters = vec![[Delayed::new(); N]; coeffs.len()];
        }
        let nfchans = roles.len() - usize::from(lfeon);
        let end = coupling.end.min(N);
        let start = coupling.start.min(end);
        let takes_part = |ch: usize| roles[ch] != Role::Untouched && coupling.is_coupled(ch);

        // The shares, of the plain coefficients, before any channel changes.
        let mut shares = [None; 5];
        for (ch, slot) in shares.iter_mut().enumerate().take(nfchans) {
            if takes_part(ch) {
                let raw = share(coeffs, coupling, nfchans, ch);
                *slot = Some(match self.shares[ch] {
                    Some(before) => SMOOTHING * before + (1.0 - SMOOTHING) * raw,
                    None => raw,
                });
            }
        }
        self.shares = shares;

        for (ch, role) in roles.iter().enumerate() {
            let sign = match role {
                Role::Plus => 1.0,
                Role::Minus => -1.0,
                Role::Untouched => continue,
            };
            let filters = &mut self.filters[ch];
            let Some(rho) = shares.get(ch).copied().flatten() else {
                *filters = [Delayed::new(); N];
                continue;
            };
            let (sum, count) = shares
                .iter()
                .enumerate()
                .filter_map(|(o, s)| s.filter(|_| o != ch))
                .fold((0.0, 0u32), |(sum, count), s| (sum + s, count + 1));
            let rho_others = if count == 0 {
                1.0
            } else {
                sum / f64::from(count)
            };
            for (bin, filter) in filters.iter_mut().enumerate() {
                if !(start..end).contains(&bin) {
                    *filter = Delayed::new();
                }
            }
            let mut first = start;
            while first < end {
                let last = (first + Coupling::SUB_BAND).min(end);
                let centre = 0.5 * (first + last - 1) as f64;
                let (a, b) = pair(rho, rho_others, centre);
                for bin in first..last {
                    let filter = &mut filters[bin];
                    let fresh = !filter.is_primed();
                    let x = coeffs[ch][bin];
                    let d = filter.step(x);
                    if !fresh {
                        coeffs[ch][bin] = a * x + sign * b * d;
                    }
                }
                first = last;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic spread of values in (-1, 1).
    fn lcg(seed: &mut u32) -> f64 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f64::from(*seed >> 8) / f64::from(1u32 << 23) - 1.0
    }

    fn stereo_coupling() -> Coupling {
        Coupling {
            start: 49,
            end: 49 + 8 * Coupling::SUB_BAND,
            channels: 0b11,
            band_structure: 0,
            enhanced: false,
            aht: false,
        }
    }

    /// A stereo block: the coupled region one shared waveform `x` times a
    /// coordinate for each channel, and under it what each channel has of
    /// its own.
    #[allow(
        clippy::needless_range_loop,
        reason = "a bin addresses the same place in both channels"
    )]
    fn stereo_block(seed: &mut u32, left_own: bool, right_own: bool) -> Vec<[f64; N]> {
        let c = stereo_coupling();
        let mut coeffs = vec![[0.0; N]; 2];
        let tone = 0.3;
        for ch in coeffs.iter_mut() {
            ch[9] = tone;
        }
        for bin in 12..c.start {
            let l = lcg(seed);
            let r = lcg(seed);
            if left_own {
                coeffs[0][bin] = 0.1 * l;
            }
            if right_own {
                coeffs[1][bin] = 0.1 * r;
            }
        }
        for bin in c.start..c.end {
            let x = 0.05 * lcg(seed);
            coeffs[0][bin] = x * 0.8;
            coeffs[1][bin] = x * 0.6;
        }
        coeffs
    }

    /// Acceptance 5: a channel with nothing of its own under the coupling
    /// begin frequency takes `a = 1` and comes out as the plain decode has
    /// it, block after block, while the other channel of the pair has noise
    /// of its own.
    #[test]
    fn a_channel_with_nothing_of_its_own_is_left_as_it_is() {
        let mut dec = Decorrelator::new();
        let c = stereo_coupling();
        let mut seed = 7;
        let mut left_moved = false;
        for _ in 0..24 {
            let plain = stereo_block(&mut seed, true, false);
            let mut out = plain.clone();
            assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
            for bin in 0..N {
                assert_eq!(
                    out[1][bin].to_bits(),
                    plain[1][bin].to_bits(),
                    "right channel, bin {bin}"
                );
            }
            left_moved |= out[0] != plain[0];
        }
        assert!(
            left_moved,
            "the left channel, which has noise of its own, did not move"
        );
    }

    #[test]
    fn a_pair_squares_to_one_and_falls_with_frequency() {
        for rho in [0.0, 0.3, 0.9, 1.0] {
            for others in [0.0, 0.5, 1.0] {
                let mut before = 1.0;
                for s in 0..8 {
                    let (a, b) = pair(rho, others, FIT_FIRST_CENTRE + 12.0 * f64::from(s));
                    assert!((a * a + b * b - 1.0).abs() < 1e-15);
                    assert!(b >= 0.0 && a >= ANGLE_MAX.cos() - 1e-12);
                    assert!(a <= before, "a rises with frequency");
                    before = a;
                }
            }
        }
        assert_eq!(pair(1.0, 0.0, 100.0), (1.0, 0.0));
        let (first, _) = pair(0.0, 1.0, FIT_FIRST_CENTRE);
        let (last, _) = pair(0.0, 1.0, FIT_LAST_CENTRE);
        assert!((first - 0.93).abs() < 1e-5 && (last - 0.81).abs() < 1e-5);
    }

    /// The term for the other channels: with `rho_others < 1` a channel is
    /// turned further than with `rho_others = 1`, by `1 + OTHERS (1 -
    /// rho_others)`, wherever the angle stays under [`ANGLE_MAX`].
    #[test]
    fn the_other_channels_turn_a_channel_further_by_the_documented_factor() {
        for rho in [0.0, 0.4] {
            for centre in [FIT_FIRST_CENTRE, 80.0, FIT_LAST_CENTRE] {
                let (alone, _) = pair(rho, 1.0, centre);
                for rho_others in [0.0, 0.25, 0.5, 0.75] {
                    let (a, _) = pair(rho, rho_others, centre);
                    let factor = 1.0 + OTHERS * (1.0 - rho_others);
                    let wanted = (alone.acos() * factor).min(ANGLE_MAX);
                    assert!(a < alone, "rho_others {rho_others} did not turn it further");
                    assert!((a.acos() - wanted).abs() < 1e-12, "{} {wanted}", a.acos());
                }
            }
        }
    }

    /// What the constants give where both channels of `low-both` have noise
    /// of their own (every `rho` 0), against what Dolby's decoder gave the
    /// left channel there: 0,72 in the first fitted sub-band and 0,61 in the
    /// last (`docs/eac3.md`). One factor for all sub-bands is fitted at the
    /// middle, where the two meet to 1e-3; at the ends it is 0,09 and 0,11
    /// off, which is the simplification [`OTHERS`] states, and the test
    /// holds them to 0,12.
    #[test]
    fn the_low_both_figures_come_out_of_the_constants() {
        let middle = 0.5 * (FIT_FIRST_CENTRE + FIT_LAST_CENTRE);
        let measured_middle = 0.5 * (0.72f64.acos() + 0.61f64.acos());
        let (a, _) = pair(0.0, 0.0, middle);
        assert!((a - measured_middle.cos()).abs() < 1e-3, "{a}");
        let (first, _) = pair(0.0, 0.0, FIT_FIRST_CENTRE);
        let (last, _) = pair(0.0, 0.0, FIT_LAST_CENTRE);
        assert!((first - 0.72).abs() < 0.12, "{first}");
        assert!((last - 0.61).abs() < 0.12, "{last}");
        // And `low-left`, where the right channel has nothing of its own.
        let (first, _) = pair(0.0, 1.0, FIT_FIRST_CENTRE);
        let (last, _) = pair(0.0, 1.0, FIT_LAST_CENTRE);
        assert!((first - 0.93).abs() < 1e-9 && (last - 0.81).abs() < 1e-9);
    }

    /// A stereo block whose left channel has noise of its own in bins 12 to
    /// 29, and whose right channel has, where `right_own`, noise of its own
    /// in bins 30 to 47, where the left channel has nothing: the left
    /// channel's share is the same either way, and only what the other
    /// channel has of its own changes. The coupled region is the same for
    /// the same seed.
    #[allow(
        clippy::needless_range_loop,
        reason = "a bin addresses the same place in both channels"
    )]
    fn apart_block(seed: &mut u32, right_own: bool) -> Vec<[f64; N]> {
        let c = stereo_coupling();
        let mut coeffs = vec![[0.0; N]; 2];
        for bin in 12..30 {
            coeffs[0][bin] = 0.1 * lcg(seed);
        }
        for bin in 30..48 {
            let r = 0.1 * lcg(seed);
            if right_own {
                coeffs[1][bin] = r;
            }
        }
        for bin in c.start..c.end {
            let x = 0.05 * lcg(seed);
            coeffs[0][bin] = x * 0.8;
            coeffs[1][bin] = x * 0.6;
        }
        coeffs
    }

    /// At the level of a block: the same left channel is decorrelated more
    /// when the right channel has content of its own under the coupling
    /// begin frequency than when it has none. What is compared is the power
    /// the decorrelation adds to the left channel's coupled region,
    /// `sum (Y - X)^2` over 24 blocks; with `rho_others` near 0 the angle is
    /// some 1,6 times as large, and the power of `Y - X` grows with the
    /// square of the angle.
    #[test]
    fn a_channel_is_decorrelated_more_when_the_other_has_content_of_its_own() {
        let c = stereo_coupling();
        let moved = |right_own: bool| {
            let mut dec = Decorrelator::new();
            let mut seed = 21;
            let mut sum = 0.0;
            for _ in 0..24 {
                let plain = apart_block(&mut seed, right_own);
                let mut out = plain.clone();
                assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
                for bin in c.start..c.end {
                    sum += (out[0][bin] - plain[0][bin]).powi(2);
                }
            }
            sum
        };
        let (alone, beside) = (moved(false), moved(true));
        assert!(alone > 0.0, "the left channel was not decorrelated at all");
        assert!(beside > 1.5 * alone, "{beside} against {alone}");
    }

    /// Bins on both sides of the coupled region, under its start and at and
    /// above its end, hold content of their own here and must come out to
    /// the bit as they went in, block after block, while the region itself
    /// moves. The fixtures cannot show the upper edge: in both, the coupled
    /// region ends where the channels' bandwidth does.
    #[test]
    fn bins_on_both_sides_of_the_coupled_region_are_left_as_they_are() {
        let c = Coupling {
            start: 49,
            end: 49 + 6 * Coupling::SUB_BAND,
            ..stereo_coupling()
        };
        let mut dec = Decorrelator::new();
        let mut seed = 31;
        let mut moved = false;
        for _ in 0..12 {
            let mut plain = vec![[0.0; N]; 2];
            for ch in plain.iter_mut() {
                for v in ch.iter_mut() {
                    *v = 0.1 * lcg(&mut seed);
                }
            }
            let mut out = plain.clone();
            assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
            for ch in 0..2 {
                for bin in (0..c.start).chain(c.end..N) {
                    assert_eq!(
                        out[ch][bin].to_bits(),
                        plain[ch][bin].to_bits(),
                        "channel {ch} bin {bin}"
                    );
                }
                moved |= out[ch][c.start..c.end] != plain[ch][c.start..c.end];
            }
        }
        assert!(moved, "the coupled region did not move");
    }

    /// Acceptance 10 at the level of a block: a coupling channel coded with
    /// the hybrid transform, and enhanced coupling, leave the block as it is
    /// and say why.
    #[test]
    fn aht_and_enhanced_coupling_are_left_as_they_are() {
        let mut seed = 3;
        for (coupling, why) in [
            (
                Coupling {
                    aht: true,
                    ..stereo_coupling()
                },
                NotApplied::Aht,
            ),
            (
                Coupling {
                    enhanced: true,
                    ..stereo_coupling()
                },
                NotApplied::EnhancedCoupling,
            ),
        ] {
            let mut dec = Decorrelator::new();
            let plain = stereo_block(&mut seed, true, true);
            let mut out = plain.clone();
            assert_eq!(dec.block(2, false, Some(&coupling), &mut out), Err(why));
            assert_eq!(out, plain);
            assert!(!why.reason().is_empty());
        }
    }

    #[test]
    fn layouts_whose_mapping_was_not_seen_are_left_as_they_are() {
        for acmod in 0..8u8 {
            for lfeon in [false, true] {
                let seen = matches!((acmod, lfeon), (2, false) | (7, _));
                assert_eq!(roles(acmod, lfeon).is_some(), seen, "{acmod} {lfeon}");
                if !seen {
                    assert_eq!(
                        Decorrelator::check(acmod, lfeon, Some(&stereo_coupling())),
                        Err(NotApplied::Layout)
                    );
                }
            }
        }
    }

    #[test]
    fn a_block_without_coupling_is_left_as_it_is() {
        let mut dec = Decorrelator::new();
        let mut seed = 5;
        let plain = stereo_block(&mut seed, true, true);
        let mut out = plain.clone();
        assert_eq!(
            dec.block(2, false, None, &mut out),
            Err(NotApplied::NoCoupling)
        );
        assert_eq!(out, plain);
    }

    #[test]
    fn a_filter_is_made_at_rest() {
        assert!(Delayed::new().is_at_rest());
        assert!(Delayed::default().is_at_rest());
    }

    /// The guard against subnormal numbers: an impulse followed by silence
    /// leaves the filter at rest, exactly, within a bounded number of
    /// blocks, instead of decaying through the subnormal range.
    #[test]
    fn silence_brings_the_filter_to_rest() {
        let mut d = Delayed::new();
        let mut moved = false;
        let first = d.step(1.0);
        moved |= first != 0.0;
        for _ in 0..400 {
            let y = d.step(0.0);
            moved |= y != 0.0;
            assert!(y == 0.0 || y.abs() >= FLUSH || y.is_normal(), "{y}");
        }
        assert!(moved, "the filter gave nothing at all");
        assert!(d.is_at_rest());
    }
}
