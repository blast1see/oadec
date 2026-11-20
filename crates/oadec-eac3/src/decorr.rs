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
//!   bin has no past for `D` to give, and it comes out as the plain decode
//!   has it; the block only fills the filter. A decoder that starts in the
//!   middle of a stream therefore differs from one that started earlier by
//!   what is left in the filters, which falls by the largest pole radius,
//!   0,7555, every block.
//!
//! Checkpoint 1 of the work: the interface, the gates and the channel
//! table are here; the filter and the processing are not built yet, and
//! [`Decorrelator::block`] says so ([`NotApplied::NotBuilt`]).

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
    /// Checkpoint 1: the processing is not built yet.
    NotBuilt,
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
            Self::NotBuilt => "decorrelation not applied: not built yet",
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
}

impl Delayed {
    /// A filter at rest.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            held: 0.0,
            state: [0.0; 3],
        }
    }

    /// Takes the bin's coefficient of one block and gives `D` of it for this
    /// block, which depends only on the blocks before.
    ///
    /// Checkpoint 1: not built; gives zero and keeps no state.
    pub fn step(&mut self, x: f64) -> f64 {
        let _ = x;
        0.0
    }

    /// Whether the filter is at rest: no past to give.
    #[must_use]
    pub fn is_at_rest(&self) -> bool {
        self.held == 0.0 && self.state == [0.0; 3]
    }
}

/// The decorrelation of one substream: the filter state of every bin of
/// every coded channel, carried from block to block.
#[derive(Debug, Clone)]
pub struct Decorrelator {
    filters: Vec<[Delayed; N]>,
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
        }
    }

    /// Sets every filter to rest.
    pub fn reset(&mut self) {
        self.filters.clear();
    }

    /// Whether one block of a stream whose layout is `(acmod, lfeon)` and
    /// whose coupling is `coupling` can be decorrelated, and if it can, the
    /// roles of its channels.
    ///
    /// # Errors
    ///
    /// The reason it cannot.
    pub fn check(
        acmod: u8,
        lfeon: bool,
        coupling: Option<&Coupling>,
    ) -> Result<&'static [Role], NotApplied> {
        let roles = roles(acmod, lfeon).ok_or(NotApplied::Layout)?;
        let coupling = coupling.ok_or(NotApplied::NoCoupling)?;
        if coupling.enhanced {
            return Err(NotApplied::EnhancedCoupling);
        }
        if coupling.aht {
            return Err(NotApplied::Aht);
        }
        Ok(roles)
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
        if let Err(why) = Self::check(acmod, lfeon, coupling) {
            self.reset();
            return Err(why);
        }
        let _ = coeffs;
        Err(NotApplied::NotBuilt)
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
        for _ in 0..24 {
            let plain = stereo_block(&mut seed, true, false);
            let mut out = plain.clone();
            let _ = dec.block(2, false, Some(&c), &mut out);
            for bin in 0..N {
                assert_eq!(
                    out[1][bin].to_bits(),
                    plain[1][bin].to_bits(),
                    "right channel, bin {bin}"
                );
            }
        }
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
