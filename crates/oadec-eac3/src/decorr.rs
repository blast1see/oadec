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
//! here as measured. How Dolby's decoders choose the weight `a` is not
//! known. This module computes `a` by a rule of its own, fitted to the pairs
//! Dolby's decoder was measured to use on test signals (`docs/eac3.md`), per
//! block and per coupling sub-band, with no randomness in it. It is this
//! decoder's estimate and not Dolby's rule. The mode is experimental, it is
//! off by default, and a decode with it is not claimed to equal Dolby's.
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
//!   bin has no past for `D` to give. It comes out as the plain decode has
//!   it, and the block only fills the filter. That is this decoder's choice,
//!   not something measured: `a X` alone would take the level of the bin
//!   down by `a^2` in that block. A decoder that starts in the middle of a
//!   stream therefore differs from one that started earlier by what is left
//!   in the filters, which falls by the largest pole radius, 0,7555, every
//!   block, and by what is left in the smoothed powers of the weight
//!   ([`SMOOTHING`]), which falls by 7/8 every block.
//!
//! # The weight
//!
//! Every constant of the rule below is an estimate of this decoder, fitted
//! as each says on Dolby's decodes of stereo test signals encoded by FFmpeg,
//! and none is Dolby's. Of a channel that takes part, in one block:
//!
//! 1. The powers, smoothed along the blocks by [`SMOOTHING`]: of the whole
//!    channel (bins 1 and up), and of each band of twelve bins under the
//!    coupling begin frequency, counted down from the first coupled bin and
//!    not below [`OWN_FIRST_BIN`]; of a pair of channels ([`pairs`]), also
//!    the product of the two in each such band.
//! 2. How active the channel is under the coupling begin frequency
//!    ([`activity`]): the power of those bands over the power of the whole
//!    channel, from 0 at [`ACTIVE_FROM_DB`] to 1 at [`ACTIVE_TO_DB`].
//! 3. The coherence of the pair: in the highest band in which both channels
//!    have more than [`ACTIVE_FROM_DB`] of their power, the magnitude of
//!    the smoothed product over the root of the smoothed powers; 0 where
//!    there is no such band, for two channels with content in no band in
//!    common have nothing in common there. (Where one of the two has no
//!    content its activity is 0, and the coherence does not count.)
//! 4. The angle ([`angle`]): `t = w ANGLE (1 + w' (1 - coherence)^APART)`,
//!    `w` the channel's activity and `w'` its partner's.
//! 5. The pair of each coupling sub-band ([`pair`]): `a = cos t
//!    h(f)^(w/2)` and `b = sqrt(1 - a^2)`, with `h` falling with the centre
//!    bin `f` of the sub-band ([`falloff`]). So `a^2 + b^2 = 1` to the
//!    rounding, and a channel that is not active takes `a = 1`.
//!
//! What it does not reproduce: Dolby's decoder turns the second channel of
//! a stereo pair by about 0,1 radian more than the first on most of the test
//! signals, the two channels the same, and the other way round on two
//! signals. The rule turns both alike, which leaves the coherence of the
//! pair, a function of the sum of the two angles, as measured.

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

/// The first bin the weight reads under the coupling begin frequency, some
/// 1,2 kHz. On Dolby's decodes a channel whose content under the coupling
/// begin frequency is a tone of 440 Hz (bin 4,7) is not decorrelated, and
/// one with a tone of 2 kHz or noise from 1 kHz up is, whether that content
/// is noise or a tone. The bands of twelve bins are counted down from the
/// first coupled bin, `37 + 12 cplbegf`, so that from bin 13 they fall on
/// the grid of the coupling sub-bands. Fitted only as far as that: the
/// limit lies somewhere between bins 5 and 13.
pub const OWN_FIRST_BIN: usize = 13;

/// The width of a band the weight reads, that of a coupling sub-band.
pub const BAND: usize = Coupling::SUB_BAND;

/// The powers of a block enter the smoothed powers by this much:
/// `p = p + SMOOTHING (x - p)`, the first block of a channel taken as it
/// is. The weight read off Dolby's decodes moves smoothly, its correlation
/// from one block to the next 0,6 to 0,9 and gone after 12 to 24 blocks.
/// Fitted with 1/4, 1/8 and 1/16, the rule misses it per block by 0,0569,
/// 0,0563 and 0,0560 radian (root mean square), so the figure matters
/// little; the eighth is a choice between the two.
pub const SMOOTHING: f64 = 0.125;

/// The activity of a channel is 0 where the bands under the coupling begin
/// frequency hold this much of its power or less, in dB. Fitted: on Dolby's
/// decodes a channel with that region 26,8 dB or more under its whole power
/// was never decorrelated, and one with it 17,8 dB or less under always
/// was; the ramp between the two is the line that puts the channel at
/// 17,8 dB at the angle Dolby gave it, 0,92 of the full one.
pub const ACTIVE_FROM_DB: f64 = -27.0;
/// The activity is 1 from here up; see [`ACTIVE_FROM_DB`].
pub const ACTIVE_TO_DB: f64 = -17.0;

/// The angle, in radians, of an active channel whose partner has the same
/// content as it, or none. Fitted by least squares, per block, with
/// [`APART`], on 24 test signals (`docs/eac3.md`); Dolby's decoder gave 0,37
/// to the first channel and 0,48 to the second there.
pub const ANGLE: f64 = 0.42;

/// How the angle grows as the two channels of a pair have other content:
/// by the factor `1 + (1 - coherence)^APART`, so that two channels with
/// nothing in common take twice [`ANGLE`]. Fitted with [`ANGLE`].
pub const APART: f64 = 0.68;

/// How fast `a^2` falls with the centre bin of the coupling sub-band, per
/// bin: `h = exp(-FALLOFF (f - FALLOFF_FROM))`. Fitted on the active
/// channels of the fit signals, where `a^2` of the last of eight sub-bands
/// is 0,754 of that of the first, and the same on every signal. All of them
/// couple from bin 49; how `h` goes for another coupling range was not
/// measured.
pub const FALLOFF: f64 = 0.003_36;
/// The centre bin from which `h` falls, that of the first coupling sub-band
/// of the fit signals; `h` is 1 under it.
pub const FALLOFF_FROM: f64 = 54.5;

/// The activity of a channel whose bands under the coupling begin frequency
/// hold `share` of its power: step 2 of the rule of the module.
#[must_use]
pub fn activity(share: f64) -> f64 {
    if share.is_nan() || share <= 0.0 {
        return 0.0;
    }
    let db = 10.0 * share.log10();
    ((db - ACTIVE_FROM_DB) / (ACTIVE_TO_DB - ACTIVE_FROM_DB)).clamp(0.0, 1.0)
}

/// The angle of a channel from its activity, its partner's and the
/// coherence of the pair: step 4 of the rule of the module.
#[must_use]
pub fn angle(active: f64, partner: f64, coherence: f64) -> f64 {
    let apart = (1.0 - coherence.clamp(0.0, 1.0)).powf(APART);
    active.clamp(0.0, 1.0) * ANGLE * (1.0 + partner.clamp(0.0, 1.0) * apart)
}

/// `h` of a coupling sub-band whose centre bin is `centre`, at most 1.
#[must_use]
pub fn falloff(centre: f64) -> f64 {
    (-FALLOFF * (centre - FALLOFF_FROM).max(0.0)).exp()
}

/// The pair `(a, b)` of a channel in a coupling sub-band, from its angle,
/// its activity and the centre bin of the sub-band: step 5 of the rule of
/// the module. `a^2 + b^2 = 1` to the rounding, and `b >= 0`: the sign is
/// the channel's [`Role`].
#[must_use]
pub fn pair(angle: f64, active: f64, centre: f64) -> (f64, f64) {
    let a = (angle.cos() * falloff(centre).powf(0.5 * active.clamp(0.0, 1.0))).clamp(0.0, 1.0);
    (a, (1.0 - a * a).max(0.0).sqrt())
}

/// The bands the weight reads under a coupling that starts at bin `start`,
/// lowest first.
#[must_use]
pub fn bands(start: usize) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut high = start.min(N);
    while high >= OWN_FIRST_BIN + BAND {
        out.push(high - BAND..high);
        high -= BAND;
    }
    out.reverse();
    out
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

/// The pairs of channels that take `b` with opposite signs, as indices of
/// coded channels: each [`Role::Plus`] with the [`Role::Minus`] after it.
#[must_use]
pub fn pairs(roles: &[Role]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut open = None;
    for (ch, role) in roles.iter().enumerate() {
        match role {
            Role::Plus => open = Some(ch),
            Role::Minus => {
                if let Some(first) = open.take() {
                    out.push((first, ch));
                }
            }
            Role::Untouched => {}
        }
    }
    out
}

/// The smoothed powers of one channel: step 1 of the rule of the module.
#[derive(Debug, Clone, PartialEq)]
struct Powers {
    /// Of the whole channel, bins 1 and up.
    total: f64,
    /// Of each band of [`bands`].
    bands: Vec<f64>,
}

/// `old + SMOOTHING (new - old)`, or `new` where there is nothing before.
fn smoothed(old: Option<f64>, new: f64) -> f64 {
    old.map_or(new, |old| old + SMOOTHING * (new - old))
}

/// The decorrelation of one substream: the filter state of every bin of
/// every coded channel and the smoothed powers of the weight, carried from
/// block to block.
#[derive(Debug, Clone)]
pub struct Decorrelator {
    filters: Vec<[Delayed; N]>,
    /// The smoothed powers of every full-bandwidth channel, `None` where the
    /// channel took no part in the block before.
    powers: Vec<Option<Powers>>,
    /// The smoothed product of the two channels of every pair of
    /// [`pairs`] in each band, `None` where the pair took no part.
    products: Vec<Option<Vec<f64>>>,
    /// The first coupled bin the bands were counted from.
    start: usize,
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
            powers: Vec::new(),
            products: Vec::new(),
            start: 0,
        }
    }

    /// Sets every filter to rest and forgets the smoothed powers.
    pub fn reset(&mut self) {
        self.filters.clear();
        self.powers.clear();
        self.products.clear();
        self.start = 0;
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

    /// Steps 1 to 4 of the rule of the module for the channels of one block:
    /// the smoothed powers are brought up to the block, and the angle and the
    /// activity of every coded channel come back, `(0, 0)` for a channel
    /// that takes no part.
    fn angles(
        &mut self,
        roles: &[Role],
        nfchans: usize,
        coupling: &Coupling,
        coeffs: &[[f64; N]],
    ) -> Vec<(f64, f64)> {
        let start = coupling.start.min(N);
        let bands = bands(start);
        let couples = pairs(roles);
        if self.start != start
            || self.powers.len() != nfchans
            || self.products.len() != couples.len()
        {
            self.powers = vec![None; nfchans];
            self.products = vec![None; couples.len()];
            self.start = start;
        }
        let takes_part =
            |ch: usize| ch < nfchans && roles[ch] != Role::Untouched && coupling.is_coupled(ch);

        for (ch, slot) in self.powers.iter_mut().enumerate() {
            if !takes_part(ch) {
                *slot = None;
                continue;
            }
            let power = |range: std::ops::Range<usize>| -> f64 {
                coeffs[ch][range].iter().map(|v| v * v).sum()
            };
            let old = slot.take();
            let total = smoothed(old.as_ref().map(|p| p.total), power(1..N));
            let in_bands = bands
                .iter()
                .enumerate()
                .map(|(j, b)| smoothed(old.as_ref().map(|p| p.bands[j]), power(b.clone())))
                .collect();
            *slot = Some(Powers {
                total,
                bands: in_bands,
            });
        }
        for (k, &(p, q)) in couples.iter().enumerate() {
            if !(takes_part(p) && takes_part(q)) {
                self.products[k] = None;
                continue;
            }
            let old = self.products[k].take();
            let now = bands
                .iter()
                .enumerate()
                .map(|(j, b)| {
                    let raw: f64 = b.clone().map(|bin| coeffs[p][bin] * coeffs[q][bin]).sum();
                    smoothed(old.as_ref().map(|o| o[j]), raw)
                })
                .collect();
            self.products[k] = Some(now);
        }

        let active: Vec<f64> = self
            .powers
            .iter()
            .map(|p| {
                p.as_ref().map_or(0.0, |p| {
                    if p.total > 0.0 {
                        activity(p.bands.iter().sum::<f64>() / p.total)
                    } else {
                        0.0
                    }
                })
            })
            .collect();
        let floor = 10f64.powf(ACTIVE_FROM_DB / 10.0);
        let mut out = vec![(0.0, 0.0); roles.len()];
        for (ch, slot) in out.iter_mut().enumerate().take(nfchans) {
            if !takes_part(ch) {
                continue;
            }
            let mut partner = 0.0;
            let mut coherence = 1.0;
            if let Some((k, &(p, q))) = couples
                .iter()
                .enumerate()
                .find(|&(_, &(p, q))| p == ch || q == ch)
            {
                let other = if p == ch { q } else { p };
                if let (Some(pp), Some(qq), Some(product)) =
                    (&self.powers[p], &self.powers[q], &self.products[k])
                {
                    partner = active[other];
                    let heard = |x: &Powers, j: usize| x.bands[j] > floor * x.total;
                    let common = (0..bands.len())
                        .rev()
                        .find(|&j| heard(pp, j) && heard(qq, j));
                    // Two channels with content in no band in common have
                    // nothing in common. Where one of them has no content,
                    // its activity is 0 and the coherence does not count.
                    coherence = match common {
                        Some(j) => {
                            let norm = (pp.bands[j] * qq.bands[j]).sqrt();
                            if norm > 0.0 {
                                (product[j].abs() / norm).min(1.0)
                            } else {
                                0.0
                            }
                        }
                        None => 0.0,
                    };
                }
            }
            *slot = (angle(active[ch], partner, coherence), active[ch]);
        }
        out
    }

    /// Decorrelates the coupled region of one block in place, `coeffs` being
    /// the block's coefficients of every coded channel as the plain decode
    /// has them. Bins outside the coupled region, channels not in coupling
    /// and channels whose role is [`Role::Untouched`] are left as they are,
    /// and so is a channel whose activity is 0. Where the block cannot be
    /// decorrelated it is left as it is, every filter is set to rest, and the
    /// reason is returned.
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
        // The angles, of the plain coefficients, before any channel changes.
        let angles = self.angles(roles, nfchans, coupling, coeffs);

        for (ch, role) in roles.iter().enumerate() {
            let sign = match role {
                Role::Plus => 1.0,
                Role::Minus => -1.0,
                Role::Untouched => continue,
            };
            let filters = &mut self.filters[ch];
            if ch >= nfchans || !coupling.is_coupled(ch) {
                *filters = [Delayed::new(); N];
                continue;
            }
            let (t, active) = angles[ch];
            for (bin, filter) in filters.iter_mut().enumerate() {
                if !(start..end).contains(&bin) {
                    *filter = Delayed::new();
                }
            }
            let mut first = start;
            while first < end {
                let last = (first + Coupling::SUB_BAND).min(end);
                let centre = 0.5 * (first + last - 1) as f64;
                let (a, b) = pair(t, active, centre);
                for bin in first..last {
                    let filter = &mut filters[bin];
                    let fresh = !filter.is_primed();
                    let x = coeffs[ch][bin];
                    let d = filter.step(x);
                    if !fresh && b != 0.0 {
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
        for t in [0.0, 0.2, ANGLE, 2.0 * ANGLE] {
            for active in [0.0, 0.5, 1.0] {
                let mut before = 1.0;
                for s in 0..8 {
                    let (a, b) = pair(t, active, FALLOFF_FROM + 12.0 * f64::from(s));
                    assert!((a * a + b * b - 1.0).abs() < 1e-15);
                    assert!(b >= 0.0 && a <= 1.0);
                    assert!(a <= before, "a rises with frequency");
                    before = a;
                }
            }
        }
        assert_eq!(pair(0.0, 0.0, 200.0), (1.0, 0.0));
        assert_eq!(falloff(FALLOFF_FROM - 20.0), 1.0);
    }

    /// The constants against what they were fitted on (`docs/eac3.md`): the
    /// angle of an active channel beside a partner with the same content or
    /// none, twice that beside one with nothing in common, and `h` of the
    /// last of eight sub-bands, which Dolby's decodes put at 0,754 of the
    /// first. And the direct weight of the first sub-band against Dolby's:
    /// 0,931 for the left channel of `low-left`, a mean of 0,68 for the two
    /// channels of `low-both`, whose noise has a coherence of some 0,05.
    #[test]
    fn the_fitted_figures_come_out_of_the_constants() {
        assert_eq!(angle(1.0, 0.0, 0.3), ANGLE);
        assert_eq!(angle(1.0, 1.0, 1.0), ANGLE);
        assert!((angle(1.0, 1.0, 0.0) - 2.0 * ANGLE).abs() < 1e-15);
        assert!((falloff(FALLOFF_FROM + 84.0) - 0.754).abs() < 0.002);
        let (alone, _) = pair(angle(1.0, 0.0, 1.0), 1.0, FALLOFF_FROM);
        assert!((alone - 0.931).abs() < 0.02, "{alone}");
        let (apart, _) = pair(angle(1.0, 1.0, 0.05), 1.0, FALLOFF_FROM);
        assert!((apart - 0.68).abs() < 0.01, "{apart}");
        // `both-phi30`, whose noise has a coherence of 0,87: Dolby turned
        // the two channels by 0,509 radian in the mean.
        let t = angle(1.0, 1.0, 0.87);
        assert!((t - 0.509).abs() < 0.03, "{t}");
    }

    /// The term for the partner: a channel is turned further by `1 + w' (1 -
    /// coherence)^APART` than it is alone.
    #[test]
    fn the_partner_turns_a_channel_further_by_the_documented_factor() {
        for active in [0.5, 1.0] {
            let alone = angle(active, 0.0, 0.0);
            for partner in [0.25, 1.0] {
                for coherence in [0.0f64, 0.3, 0.9] {
                    let factor = 1.0 + partner * (1.0 - coherence).powf(APART);
                    let t = angle(active, partner, coherence);
                    assert!(t > alone);
                    assert!((t - alone * factor).abs() < 1e-15, "{t} {alone} {factor}");
                }
            }
        }
    }

    #[test]
    fn activity_ramps_between_its_two_levels() {
        let share = |db: f64| 10f64.powf(db / 10.0);
        assert_eq!(activity(0.0), 0.0);
        assert_eq!(activity(share(ACTIVE_FROM_DB - 1.0)), 0.0);
        assert!((activity(share(0.5 * (ACTIVE_FROM_DB + ACTIVE_TO_DB))) - 0.5).abs() < 1e-12);
        assert_eq!(activity(share(ACTIVE_TO_DB + 0.1)), 1.0);
        assert_eq!(activity(1.0), 1.0);
    }

    #[test]
    fn the_bands_lie_on_the_grid_of_the_coupling_sub_bands() {
        assert_eq!(bands(49), vec![13..25, 25..37, 37..49]);
        assert_eq!(bands(37), vec![13..25, 25..37]);
        assert!(bands(24).is_empty());
        assert_eq!(pairs(roles(2, false).unwrap()), vec![(0, 1)]);
        assert_eq!(pairs(roles(7, true).unwrap()), vec![(0, 2), (3, 4)]);
    }

    /// Stereo blocks whose left channel has a tone in bin `left_bin` and the
    /// right channel nothing under the coupling begin frequency: the left
    /// channel is decorrelated only when its tone lies in the bands the
    /// weight reads.
    #[test]
    #[allow(
        clippy::needless_range_loop,
        reason = "a bin addresses the same place in both channels"
    )]
    fn a_tone_counts_only_from_the_first_band_up() {
        let c = stereo_coupling();
        for (left_bin, wanted) in [(5, false), (OWN_FIRST_BIN - 1, false), (20, true)] {
            let mut dec = Decorrelator::new();
            let mut seed = 11;
            let mut moved = false;
            for n in 0..24 {
                let mut plain = vec![[0.0; N]; 2];
                plain[0][left_bin] = if n % 2 == 0 { 0.3 } else { -0.25 };
                for bin in c.start..c.end {
                    let x = 0.05 * lcg(&mut seed);
                    plain[0][bin] = 0.8 * x;
                    plain[1][bin] = 0.6 * x;
                }
                let mut out = plain.clone();
                assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
                assert_eq!(out[1], plain[1], "the right channel has nothing of its own");
                moved |= out[0] != plain[0];
            }
            assert_eq!(moved, wanted, "a tone in bin {left_bin}");
        }
    }

    /// A stereo block whose left channel has noise of its own in bins 13 to
    /// 29, and whose right channel has, where `right_own`, other noise of its
    /// own, in bins 30 to 47 or, where `same`, the left channel's noise
    /// again. The coupled region is the same for the same seed.
    #[allow(
        clippy::needless_range_loop,
        reason = "a bin addresses the same place in both channels"
    )]
    fn apart_block(seed: &mut u32, right_own: bool, same: bool) -> Vec<[f64; N]> {
        let c = stereo_coupling();
        let mut coeffs = vec![[0.0; N]; 2];
        for bin in 13..30 {
            coeffs[0][bin] = 0.1 * lcg(seed);
            if same {
                coeffs[1][bin] = coeffs[0][bin];
            }
        }
        for bin in 30..48 {
            let r = 0.1 * lcg(seed);
            if right_own && !same {
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

    /// The power the decorrelation adds to the left channel's coupled region,
    /// `sum (Y - X)^2` over 24 blocks of [`apart_block`].
    fn left_moved(right_own: bool, same: bool) -> f64 {
        let c = stereo_coupling();
        let mut dec = Decorrelator::new();
        let mut seed = 21;
        let mut sum = 0.0;
        for _ in 0..24 {
            let plain = apart_block(&mut seed, right_own, same);
            let mut out = plain.clone();
            assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
            for bin in c.start..c.end {
                sum += (out[0][bin] - plain[0][bin]).powi(2);
            }
        }
        sum
    }

    /// At the level of a block: the same left channel is decorrelated more
    /// when the right channel has other content of its own under the
    /// coupling begin frequency than when it has none or the same. With
    /// nothing in common the angle is some twice as large, and the power of
    /// `Y - X` grows with the square of the angle.
    #[test]
    fn a_channel_is_decorrelated_more_when_the_other_has_other_content() {
        let alone = left_moved(false, false);
        let same = left_moved(true, true);
        let beside = left_moved(true, false);
        assert!(alone > 0.0, "the left channel was not decorrelated at all");
        assert!(
            (same - alone).abs() < 0.05 * alone,
            "{same} against {alone}"
        );
        assert!(beside > 2.5 * alone, "{beside} against {alone}");
    }

    /// The power the decorrelation adds to the left channel's coupled region
    /// over 24 stereo blocks whose own regions `own` writes, with the same
    /// coupled region for the same seed.
    #[allow(
        clippy::needless_range_loop,
        reason = "a bin addresses the same place in both channels"
    )]
    fn moved_by(own: impl Fn(&mut u32, &mut [[f64; N]])) -> f64 {
        let c = stereo_coupling();
        let mut dec = Decorrelator::new();
        let mut seed = 41;
        let mut sum = 0.0;
        for _ in 0..24 {
            let mut plain = vec![[0.0; N]; 2];
            own(&mut seed, &mut plain);
            for bin in c.start..c.end {
                let x = 0.05 * lcg(&mut seed);
                plain[0][bin] = 0.8 * x;
                plain[1][bin] = 0.6 * x;
            }
            let mut out = plain.clone();
            assert_eq!(dec.block(2, false, Some(&c), &mut out), Ok(()));
            for bin in c.start..c.end {
                sum += (out[0][bin] - plain[0][bin]).powi(2);
            }
        }
        sum
    }

    /// The coherence of a pair is read in the highest band both channels
    /// have content in: a tone the two share in bins 13 to 24 does not make
    /// them alike where their noise in bins 37 to 48 is not. And two
    /// channels with content in no band in common are as apart as two with
    /// other noise in the same band.
    #[test]
    fn the_highest_band_in_common_decides() {
        let same_noise = moved_by(|seed, own| {
            for bin in 37..49 {
                own[0][bin] = 0.1 * lcg(seed);
                own[1][bin] = own[0][bin];
            }
        });
        let other_noise = moved_by(|seed, own| {
            for bin in 37..49 {
                own[0][bin] = 0.1 * lcg(seed);
                own[1][bin] = 0.1 * lcg(seed);
            }
        });
        let shared_tone_other_noise = moved_by(|seed, own| {
            own[0][15] = 0.3;
            own[1][15] = 0.3;
            for bin in 37..49 {
                own[0][bin] = 0.1 * lcg(seed);
                own[1][bin] = 0.1 * lcg(seed);
            }
        });
        let apart_bands = moved_by(|seed, own| {
            for bin in 13..25 {
                own[0][bin] = 0.1 * lcg(seed);
            }
            for bin in 37..49 {
                own[1][bin] = 0.1 * lcg(seed);
            }
        });
        assert!(same_noise > 0.0);
        assert!(other_noise > 2.5 * same_noise, "{other_noise} {same_noise}");
        assert!(
            shared_tone_other_noise > 2.5 * same_noise,
            "{shared_tone_other_noise} {same_noise}"
        );
        assert!(apart_bands > 2.5 * same_noise, "{apart_bands} {same_noise}");
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
