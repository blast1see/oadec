//! The channels of one programme, and where they come from (clause E.2.8).
//!
//! An Enhanced AC-3 bit stream carries a programme of more than 5.1 channels as
//! an independent substream holding a 5.1-compatible downmix followed
//! immediately by the dependent substreams that replace and extend it
//! (clause E.1.3.1.2). Which channels a dependent substream carries is said
//! either by its own `acmod` and `lfeon` or, when it sets `chanmape`, by the
//! custom channel map of table E.1.4.
//!
//! This module names those channels. Everything derived from a location — its
//! WAVE mask bit, its place in interchange order, the JOC downmix input it
//! feeds — hangs off one enum, so the three can never disagree with each other
//! the way three separate `match` ladders eventually do.

use std::collections::{BTreeMap, VecDeque};

use crate::bsi::Bsi;
use crate::decoder::{Decoded, Decoder};
use crate::error::Result;
use crate::frame::Options;
use crate::header::{FrameHeader, StreamType};
use crate::tables::CHANNEL_ORDER;

/// A channel location in a programme.
///
/// Table E.1.4 for the custom channel map, plus the names table 4.3 gives to
/// coded channels the custom map has no bit for: the dual-mono pair and the
/// mono surround.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChannelLoc {
    L,
    C,
    R,
    Ls,
    Rs,
    Lc,
    Rc,
    Lrs,
    Rrs,
    Cs,
    Ts,
    Lsd,
    Rsd,
    Lw,
    Rw,
    Vhl,
    Vhr,
    Vhc,
    Lts,
    Rts,
    Lfe2,
    Lfe,
    /// `acmod 0`, the first channel of a dual-mono pair.
    Ch1,
    /// `acmod 0`, the second channel of a dual-mono pair.
    Ch2,
    /// `acmod 4` and `acmod 5`, the mono surround. Table E.1.4 has no bit for
    /// it, so it can only reach a programme from an independent substream.
    S,
}

impl ChannelLoc {
    /// The name the tables of TS 102 366 use.
    #[must_use]
    pub const fn name(self) -> &'static str {
        use ChannelLoc::{
            C, Ch1, Ch2, Cs, L, Lc, Lfe, Lfe2, Lrs, Ls, Lsd, Lts, Lw, R, Rc, Rrs, Rs, Rsd, Rts, Rw,
            S, Ts, Vhc, Vhl, Vhr,
        };
        match self {
            L => "L",
            C => "C",
            R => "R",
            Ls => "Ls",
            Rs => "Rs",
            Lc => "Lc",
            Rc => "Rc",
            Lrs => "Lrs",
            Rrs => "Rrs",
            Cs => "Cs",
            Ts => "Ts",
            Lsd => "Lsd",
            Rsd => "Rsd",
            Lw => "Lw",
            Rw => "Rw",
            Vhl => "Vhl",
            Vhr => "Vhr",
            Vhc => "Vhc",
            Lts => "Lts",
            Rts => "Rts",
            Lfe2 => "LFE2",
            Lfe => "LFE",
            Ch1 => "Ch1",
            Ch2 => "Ch2",
            S => "S",
        }
    }

    /// The location of a name from table 4.3 or table E.1.4.
    ///
    /// `Lb` and `Rb` are accepted as the names TS 103 420 table 53 gives the
    /// rear surround pair that table E.1.4 calls `Lrs` and `Rrs`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        use ChannelLoc::{
            C, Ch1, Ch2, Cs, L, Lc, Lfe, Lfe2, Lrs, Ls, Lsd, Lts, Lw, R, Rc, Rrs, Rs, Rsd, Rts, Rw,
            S, Ts, Vhc, Vhl, Vhr,
        };
        Some(match name {
            "L" => L,
            "C" => C,
            "R" => R,
            "Ls" => Ls,
            "Rs" => Rs,
            "Lc" => Lc,
            "Rc" => Rc,
            "Lrs" | "Lb" => Lrs,
            "Rrs" | "Rb" => Rrs,
            "Cs" => Cs,
            "Ts" => Ts,
            "Lsd" => Lsd,
            "Rsd" => Rsd,
            "Lw" => Lw,
            "Rw" => Rw,
            "Vhl" => Vhl,
            "Vhr" => Vhr,
            "Vhc" => Vhc,
            "Lts" => Lts,
            "Rts" => Rts,
            "LFE2" => Lfe2,
            "LFE" => Lfe,
            "Ch1" => Ch1,
            "Ch2" => Ch2,
            "S" => S,
            _ => return None,
        })
    }

    /// The `dwChannelMask` bit of `WAVEFORMATEXTENSIBLE`, or zero for a
    /// location WAVE cannot name (the surround-direct and wide pairs and the
    /// second LFE).
    #[must_use]
    pub const fn wave_mask(self) -> u32 {
        use ChannelLoc::{
            C, Ch1, Ch2, Cs, L, Lc, Lfe, Lfe2, Lrs, Ls, Lsd, Lts, Lw, R, Rc, Rrs, Rs, Rsd, Rts, Rw,
            S, Ts, Vhc, Vhl, Vhr,
        };
        match self {
            L | Ch1 => 0x1,
            R | Ch2 => 0x2,
            C => 0x4,
            Lfe => 0x8,
            Lrs => 0x10,
            Rrs => 0x20,
            Lc => 0x40,
            Rc => 0x80,
            S | Cs => 0x100,
            Ls => 0x200,
            Rs => 0x400,
            Ts => 0x800,
            Vhl => 0x1000,
            Vhc => 0x2000,
            Vhr => 0x4000,
            Lts => 0x8000,
            Rts => 0x2_0000,
            Lsd | Rsd | Lw | Rw | Lfe2 => 0,
        }
    }

    /// Position in WAVE interchange order, which is the order of the mask
    /// bits. Locations WAVE cannot name sort after every one it can, keeping
    /// their coded order among themselves.
    #[must_use]
    pub const fn interchange_rank(self) -> u8 {
        match self.wave_mask() {
            0 => 32,
            mask => mask.trailing_zeros() as u8,
        }
    }

    /// The JOC downmix input this location feeds under a given
    /// `joc_dmx_config_idx`, if any.
    ///
    /// Table 47 of TS 103 420 gives the downmix channels per configuration and
    /// the last two are not the same pair in all of them: configuration 1 ends
    /// `Lb, Rb`, the rear surround pair that table E.1.4 of TS 102 366 calls
    /// `Lrs` and `Rrs`, while configurations 2 and 4 end `Tfl, Tfr`, the top
    /// front pair that table E.1.4 calls `Vhl` and `Vhr`. Configurations 0 and
    /// 3 have five channels and no such pair. Reading the seven-channel
    /// configurations as if they all ended in the rear pair leaves a real
    /// stream's two height channels unmapped: Green Book's Dolby Digital Plus
    /// track is configuration 4 over `L C R Ls Rs LFE Vhl Vhr`.
    #[must_use]
    pub const fn joc_input(self, dmx_config: u8) -> Option<usize> {
        use ChannelLoc::{C, L, Lrs, Ls, R, Rrs, Rs, Vhl, Vhr};
        Some(match self {
            L => 0,
            R => 1,
            C => 2,
            Ls => 3,
            Rs => 4,
            Lrs if dmx_config == 1 => 5,
            Rrs if dmx_config == 1 => 6,
            Vhl if dmx_config == 2 || dmx_config == 4 => 5,
            Vhr if dmx_config == 2 || dmx_config == 4 => 6,
            _ => return None,
        })
    }
}

/// Channel locations of the custom channel map by `chanmap` bit, in the same
/// order as [`CHANMAP_LOCATIONS`], which the unit tests pin this against.
const CHANMAP: [&[ChannelLoc]; 16] = {
    use ChannelLoc::{
        C, Cs, L, Lc, Lfe, Lfe2, Lrs, Ls, Lsd, Lts, Lw, R, Rc, Rrs, Rs, Rsd, Rts, Rw, Ts, Vhc, Vhl,
        Vhr,
    };
    [
        &[L],
        &[C],
        &[R],
        &[Ls],
        &[Rs],
        &[Lc, Rc],
        &[Lrs, Rrs],
        &[Cs],
        &[Ts],
        &[Lsd, Rsd],
        &[Lw, Rw],
        &[Vhl, Vhr],
        &[Vhc],
        &[Lts, Rts],
        &[Lfe2],
        &[Lfe],
    ]
};

/// The most channels one programme may render (clause E.2.8.2).
pub const MAX_PROGRAM_CHANNELS: usize = 16;

/// Why a substream's channels could not be named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
    /// The channel map names a different number of channels than the substream
    /// codes. Clause E.1.3.1.8 requires the two to be equal.
    Count { locations: usize, coded: usize },
    /// A custom channel map on a substream that is not dependent. Only
    /// dependent substreams may carry one (clause E.1.3.1.7).
    NotDependent,
}

impl core::fmt::Display for LocationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Count { locations, coded } => write!(
                f,
                "the channel map names {locations} channel locations but the substream codes {coded} channels"
            ),
            Self::NotDependent => {
                write!(
                    f,
                    "a custom channel map on a substream that is not dependent"
                )
            }
        }
    }
}

impl core::error::Error for LocationError {}

/// The channels a dependent substream carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependentLocations {
    pub locations: Vec<ChannelLoc>,
    /// The custom map named as many locations as the substream has
    /// full-bandwidth channels, and the substream also carries an LFE, so the
    /// LFE was appended. Clause E.1.3.1.8 says the two counts shall be equal
    /// and no stream to hand settles whether the map counts the LFE, so this is
    /// accepted and counted rather than refused.
    pub lfe_implied: bool,
}

/// The locations of the coded channels of an independent substream, in coded
/// order (table 4.3, with the LFE last).
#[must_use]
pub fn independent_locations(header: &FrameHeader) -> Vec<ChannelLoc> {
    let mut out: Vec<ChannelLoc> = CHANNEL_ORDER[usize::from(header.acmod)]
        .iter()
        .map(|n| ChannelLoc::from_name(n).expect("table 4.3 names are locations"))
        .collect();
    if header.lfeon {
        out.push(ChannelLoc::Lfe);
    }
    out
}

/// The locations named by a custom channel map, in coded order, pair bits
/// expanded to their two adjacent channels (table E.1.4).
#[must_use]
pub fn chanmap_locations(chanmap: u16) -> Vec<ChannelLoc> {
    (0..16)
        .filter(|bit| chanmap & (1 << (15 - bit)) != 0)
        .flat_map(|bit| CHANMAP[bit].iter().copied())
        .collect()
}

/// The locations of the coded channels of a dependent substream, in coded
/// order (clause E.2.8.2).
///
/// With `chanmape` clear the substream's own `acmod` and `lfeon` name them;
/// with it set the custom channel map does.
///
/// # Errors
///
/// [`LocationError::Count`] when the map names a different number of channels
/// than the substream codes, and [`LocationError::NotDependent`] when a
/// substream that is not dependent carries a map.
pub fn dependent_locations(
    header: &FrameHeader,
    bsi: &Bsi,
) -> core::result::Result<DependentLocations, LocationError> {
    if header.stream_type != StreamType::Dependent {
        if bsi.chanmap.is_some() {
            return Err(LocationError::NotDependent);
        }
        return Ok(DependentLocations {
            locations: independent_locations(header),
            lfe_implied: false,
        });
    }
    let Some(chanmap) = bsi.chanmap else {
        return Ok(DependentLocations {
            locations: independent_locations(header),
            lfe_implied: false,
        });
    };
    let mut locations = chanmap_locations(chanmap);
    let coded = header.nchans();
    let mut lfe_implied = false;
    if locations.len() != coded {
        if locations.len() == header.nfchans() && header.lfeon {
            locations.push(ChannelLoc::Lfe);
            lfe_implied = true;
        } else {
            return Err(LocationError::Count {
                locations: locations.len(),
                coded,
            });
        }
    }
    Ok(DependentLocations {
        locations,
        lfe_implied,
    })
}

/// What merging one substream's channels into a programme did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Channels that took the place of one the programme already had.
    pub replaced: usize,
    /// Channels the programme did not have before.
    pub added: usize,
    /// Channels dropped because the programme was already at its limit.
    pub over_capacity: usize,
}

/// The channels of a programme and the substream channel each is taken from.
///
/// Built by merging substreams in bitstream order per clause E.2.8.2: a
/// channel whose location the programme already carries replaces it, and one
/// whose location is new is appended. The independent substream is merged
/// first, so its 5.1-compatible downmix is what a dependent substream
/// overwrites.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProgramLayout {
    /// Programme channels, in the order they were first seen.
    pub channels: Vec<ChannelLoc>,
    /// For each programme channel, the substream it comes from and the coded
    /// channel within it.
    pub sources: Vec<(usize, usize)>,
}

impl ProgramLayout {
    /// Merges the channels of substream `part`, given in coded order.
    pub fn merge(&mut self, part: usize, locations: &[ChannelLoc]) -> MergeOutcome {
        let mut out = MergeOutcome::default();
        for (coded, &loc) in locations.iter().enumerate() {
            if let Some(i) = self.channels.iter().position(|c| *c == loc) {
                self.sources[i] = (part, coded);
                out.replaced += 1;
            } else if self.channels.len() < MAX_PROGRAM_CHANNELS {
                self.channels.push(loc);
                self.sources.push((part, coded));
                out.added += 1;
            } else {
                out.over_capacity += 1;
            }
        }
        out
    }

    /// Number of programme channels.
    #[must_use]
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether no substream has been merged yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Programme channel indices in WAVE interchange order.
    #[must_use]
    pub fn interchange_order(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.channels.len()).collect();
        idx.sort_by_key(|&i| self.channels[i].interchange_rank());
        idx
    }

    /// The channel names in programme order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.channels.iter().map(|c| c.name()).collect()
    }
}

/// The substream one part of a programme came from: `(strmtyp, substreamid)`.
pub type SubstreamKey = (u8, u8);

/// One decoded substream of a frame group.
#[derive(Debug)]
pub struct Part {
    pub key: SubstreamKey,
    /// The channels this substream carries, in its own coded order.
    pub locations: Vec<ChannelLoc>,
    /// The channel map named the full-bandwidth channels of a substream that
    /// also carries an LFE, so the LFE was appended (see [`DependentLocations`]).
    pub lfe_implied: bool,
    pub decoded: Decoded,
}

/// One frame group: an independent substream and the dependent substreams that
/// immediately followed it, merged into one programme frame.
#[derive(Debug)]
pub struct ProgramFrame {
    /// Group index, counted from 0.
    pub index: u64,
    /// The programme's channels and where each is taken from.
    pub layout: ProgramLayout,
    /// The substreams of this group in bitstream order; `parts[0]` is the
    /// independent one.
    pub parts: Vec<Part>,
    /// This group carried a different set of channels from the programme's, so
    /// the ones it did not carry are silent.
    pub layout_changed: bool,
    /// Set only when `layout_changed`: the programme's channels materialised,
    /// with silence where this group had nothing.
    filled: Vec<Vec<f32>>,
}

impl ProgramFrame {
    /// The independent substream of the group.
    #[must_use]
    pub fn core(&self) -> &Decoded {
        &self.parts[0].decoded
    }

    /// Samples of programme channel `i`.
    ///
    /// Nothing is copied in the usual case: the merge is a permutation, so a
    /// programme channel is a borrow of one substream's decoded channel.
    #[must_use]
    pub fn channel(&self, i: usize) -> &[f32] {
        if self.layout_changed {
            return &self.filled[i];
        }
        let (part, coded) = self.layout.sources[i];
        &self.parts[part].decoded.pcm[coded]
    }

    /// Number of programme channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.layout.len()
    }

    /// Samples per channel.
    #[must_use]
    pub fn samples(&self) -> usize {
        self.parts[0].decoded.header.samples()
    }

    /// The substream carrying the EMDF metadata (TS 103 420 clause 8.2): the
    /// last dependent substream when the programme has one, else the
    /// independent substream.
    #[must_use]
    pub fn metadata_part(&self) -> &Part {
        self.parts
            .iter()
            .rev()
            .find(|p| p.key.0 == StreamType::Dependent as u8)
            .unwrap_or(&self.parts[0])
    }
}

/// What a pass over the substreams of a programme found, beyond what the caller
/// can see for itself in each [`Part`].
#[derive(Debug, Default, Clone)]
pub struct ProgramStats {
    /// Frame groups emitted.
    pub groups: u64,
    /// Frames of a programme other than the selected one. Legal, and skipped
    /// (clause E.2.8.3).
    pub other_program_frames: u64,
    /// Dependent frames with no independent substream before them.
    pub orphan_dependents: u64,
    /// Groups whose channel set differed from the programme's.
    pub layout_changes: u64,
    /// Dependent frames dropped because their block count or sample rate did
    /// not match the independent substream (clause E.1.3.1.2).
    pub misaligned: u64,
    /// Channels dropped because the programme was already at sixteen.
    pub over_capacity: u64,
    /// Dependent frames whose channel map could not be read.
    pub location_errors: u64,
    /// Frames whose channel map left the LFE implied.
    pub lfe_implied: u64,
    /// Dependent frames whose channels did not reach the programme, for any
    /// reason above.
    pub dependent_dropped: u64,
    /// Decode errors per substream.
    pub decode_errors: BTreeMap<SubstreamKey, u64>,
    pub first_error: Option<String>,
}

impl ProgramStats {
    /// Whether every substream of the programme reached the output intact.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.orphan_dependents == 0
            && self.layout_changes == 0
            && self.misaligned == 0
            && self.over_capacity == 0
            && self.location_errors == 0
            && self.dependent_dropped == 0
            && self.decode_errors.values().all(|&n| n == 0)
    }

    fn note(&mut self, message: String) {
        if self.first_error.is_none() {
            self.first_error = Some(message);
        }
    }
}

/// One substream's decoder and the group each frame in flight belongs to.
#[derive(Debug)]
struct Sub {
    key: SubstreamKey,
    dec: Decoder,
    /// Groups fed but not yet released, oldest first.
    inflight: VecDeque<u64>,
    /// Released frames with the group they belong to.
    ready: VecDeque<(u64, Decoded)>,
}

/// A group being assembled or waiting for its substreams.
#[derive(Debug)]
struct GroupSlot {
    index: u64,
    /// Substream slots fed this group, in bitstream order.
    members: Vec<usize>,
    /// The slot of the independent substream, once it has been fed.
    core: Option<usize>,
}

/// Decodes one programme of an Enhanced AC-3 stream: an independent substream
/// and the dependent substreams that immediately follow it (clause E.2.8.2).
///
/// Frames go in in bitstream order and merged groups come out. Each substream
/// keeps its own [`Decoder`], because a decoder holds per-substream state --
/// the transform delay buffers, the noise generator, the enhanced-coupling
/// synthesiser, a held frame, the transient pre-noise look-ahead -- none of
/// which may be shared.
///
/// That last point is why a released frame carries its group index rather than
/// being paired by arrival: enhanced coupling makes a decoder hold a frame
/// back, and transient pre-noise processing holds samples back, so a substream
/// using either releases a group later than one that does not. Pairing whatever
/// came out of one decoder with whatever came out of another would misalign the
/// programme the moment an encoder switched a tool on in the core and not in
/// the dependent substream.
#[derive(Debug)]
pub struct ProgramDecoder {
    opts: Options,
    subs: Vec<Sub>,
    slots: BTreeMap<SubstreamKey, usize>,
    open: Option<GroupSlot>,
    pending: VecDeque<GroupSlot>,
    next_group: u64,
    /// Substream id of the programme the frames now arriving belong to.
    program: Option<u8>,
    layout: Option<ProgramLayout>,
    stats: ProgramStats,
}

impl ProgramDecoder {
    #[must_use]
    pub fn new(opts: Options) -> Self {
        Self {
            opts,
            subs: Vec::new(),
            slots: BTreeMap::new(),
            open: None,
            pending: VecDeque::new(),
            next_group: 0,
            program: None,
            layout: None,
            stats: ProgramStats::default(),
        }
    }

    /// What the pass has found so far.
    #[must_use]
    pub fn stats(&self) -> &ProgramStats {
        &self.stats
    }

    /// The programme's channel layout, once a group has been emitted.
    #[must_use]
    pub fn layout(&self) -> Option<&ProgramLayout> {
        self.layout.as_ref()
    }

    fn slot_for(&mut self, key: SubstreamKey) -> usize {
        if let Some(&s) = self.slots.get(&key) {
            return s;
        }
        let s = self.subs.len();
        self.subs.push(Sub {
            key,
            dec: Decoder::new(self.opts),
            inflight: VecDeque::new(),
            ready: VecDeque::new(),
        });
        self.slots.insert(key, s);
        s
    }

    /// Feeds one syncframe, in bitstream order.
    ///
    /// A frame that will not decode is recorded and its substream reset, so a
    /// broken dependent substream never costs the programme its core.
    ///
    /// # Errors
    ///
    /// Only from flushing a decoder after a failed frame.
    pub fn push(&mut self, bytes: &[u8], header: &FrameHeader) -> Result<()> {
        let key = (header.stream_type as u8, header.substream_id);
        let dependent = header.stream_type == StreamType::Dependent;
        // Dependent substreams immediately follow the independent substream
        // they belong to (clause E.1.3.1.2), so bitstream order is what says
        // which programme a dependent frame is part of.
        if !dependent {
            self.program = Some(header.substream_id);
        }
        if dependent && self.program.is_none() {
            self.stats.orphan_dependents += 1;
            self.stats
                .note("a dependent substream before any independent one".to_string());
            return Ok(());
        }
        if self.program != Some(0) {
            self.stats.other_program_frames += 1;
            return Ok(());
        }
        let slot = self.slot_for(key);
        if !dependent {
            if let Some(g) = self.open.take() {
                self.pending.push_back(g);
            }
            self.open = Some(GroupSlot {
                index: self.next_group,
                members: Vec::new(),
                core: Some(slot),
            });
            self.next_group += 1;
        }
        let Some(open) = self.open.as_mut() else {
            return Ok(());
        };
        let group = open.index;
        if !open.members.contains(&slot) {
            open.members.push(slot);
        }
        let sub = &mut self.subs[slot];
        sub.inflight.push_back(group);
        match sub.dec.decode(bytes) {
            Ok(Some(d)) => {
                let at = sub.inflight.pop_front().unwrap_or(group);
                sub.ready.push_back((at, d));
            }
            Ok(None) => {}
            Err(e) => {
                sub.inflight.pop_back();
                *self.stats.decode_errors.entry(key).or_default() += 1;
                let (t, id) = key;
                let kind = if t == 1 { "dependent" } else { "independent" };
                self.stats
                    .note(format!("group {group}, {kind} substream {id}: {e}"));
                while let Some(d) = sub.dec.flush()? {
                    let at = sub.inflight.pop_front().unwrap_or(group);
                    sub.ready.push_back((at, d));
                }
                sub.dec.reset();
                // the failing frame is always the one just fed, and its group is
                // still open, so dropping the slot from it stops `pop` waiting
                // for a frame that will never arrive
                if let Some(open) = self.open.as_mut() {
                    open.members.retain(|&s| s != slot);
                    if open.core == Some(slot) {
                        open.core = None;
                    }
                }
            }
        }
        Ok(())
    }

    /// Closes the stream: drains every substream's look-ahead and closes the
    /// open group. Call [`Self::pop`] in a loop afterwards.
    ///
    /// # Errors
    ///
    /// Only from flushing a decoder.
    pub fn finish(&mut self) -> Result<()> {
        for sub in &mut self.subs {
            while let Some(d) = sub.dec.flush()? {
                let at = sub.inflight.pop_front().unwrap_or(0);
                sub.ready.push_back((at, d));
            }
        }
        if let Some(g) = self.open.take() {
            self.pending.push_back(g);
        }
        Ok(())
    }

    /// The next group whose every substream has arrived.
    pub fn pop(&mut self) -> Option<ProgramFrame> {
        loop {
            let g = self.pending.front()?;
            let ready = g.members.iter().all(|&s| {
                self.subs[s]
                    .ready
                    .front()
                    .is_some_and(|(gi, _)| *gi == g.index)
            });
            if !ready {
                return None;
            }
            let g = self.pending.pop_front()?;
            // a group whose independent substream failed produced no audio
            let Some(core) = g.core else {
                for &s in &g.members {
                    self.subs[s].ready.pop_front();
                }
                continue;
            };
            if let Some(frame) = self.assemble(&g, core) {
                self.stats.groups += 1;
                return Some(frame);
            }
        }
    }

    fn assemble(&mut self, g: &GroupSlot, core: usize) -> Option<ProgramFrame> {
        let mut parts: Vec<Part> = Vec::with_capacity(g.members.len());
        for &s in &g.members {
            let key = self.subs[s].key;
            let (_, decoded) = self.subs[s].ready.pop_front()?;
            if s != core {
                let ch = &parts[0].decoded.header;
                if decoded.header.blocks != ch.blocks
                    || decoded.header.sample_rate != ch.sample_rate
                {
                    self.stats.misaligned += 1;
                    self.stats.dependent_dropped += 1;
                    let (_, id) = key;
                    self.stats.note(format!(
                        "group {}, dependent substream {id}: {} blocks at {} Hz against the independent substream's {} at {}",
                        g.index, decoded.header.blocks, decoded.header.sample_rate, ch.blocks, ch.sample_rate
                    ));
                    continue;
                }
            }
            match dependent_locations(&decoded.header, &decoded.bsi) {
                Ok(l) => {
                    if l.lfe_implied {
                        self.stats.lfe_implied += 1;
                    }
                    parts.push(Part {
                        key,
                        locations: l.locations,
                        lfe_implied: l.lfe_implied,
                        decoded,
                    });
                }
                Err(e) => {
                    self.stats.location_errors += 1;
                    self.stats.dependent_dropped += 1;
                    let (_, id) = key;
                    self.stats
                        .note(format!("group {}, substream {id}: {e}", g.index));
                }
            }
        }
        if parts.is_empty() {
            return None;
        }
        let mut layout = ProgramLayout::default();
        for (i, part) in parts.iter().enumerate() {
            let out = layout.merge(i, &part.locations);
            if out.over_capacity > 0 {
                self.stats.over_capacity += u64::try_from(out.over_capacity).unwrap_or(u64::MAX);
                self.stats.dependent_dropped += 1;
                self.stats.note(format!(
                    "group {}: {} channels past the sixteen a programme may carry",
                    g.index, out.over_capacity
                ));
            }
        }
        let programme = self.layout.get_or_insert_with(|| layout.clone());
        let mut layout_changed = false;
        let mut filled = Vec::new();
        if programme.channels != layout.channels {
            // The output width is fixed by the first group -- `decode` writes
            // the WAVE header from it and only patches the length afterwards --
            // so a group that carries a different set of channels is reported
            // and silence-filled, never acted on.
            self.stats.layout_changes += 1;
            self.stats.note(format!(
                "group {}: the programme changed from {:?} to {:?}",
                g.index,
                programme
                    .channels
                    .iter()
                    .map(|c| c.name())
                    .collect::<Vec<_>>(),
                layout.channels.iter().map(|c| c.name()).collect::<Vec<_>>()
            ));
            layout_changed = true;
            let samples = parts[0].decoded.header.samples();
            filled = programme
                .channels
                .iter()
                .map(|loc| {
                    layout.channels.iter().position(|c| c == loc).map_or_else(
                        || vec![0.0; samples],
                        |i| {
                            let (part, coded) = layout.sources[i];
                            parts[part].decoded.pcm[coded].clone()
                        },
                    )
                })
                .collect();
        }
        let layout = if layout_changed {
            programme.clone()
        } else {
            layout
        };
        Some(ProgramFrame {
            index: g.index,
            layout,
            parts,
            layout_changed,
            filled,
        })
    }
}

#[cfg(test)]
mod tests {

    /// Each counter the verdict reads, one at a time.
    ///
    /// A mutation pass short-circuited the verdict to `true` and no test
    /// noticed, which is the worst place for that to be possible: the whole
    /// point of the first defect was that a truncated programme used to be
    /// called clean. The list is what makes the difference, so the list is what
    /// is pinned -- and `other_program_frames` and `lfe_implied` are in it as
    /// the two that must *not* count, since a second programme is legal and an
    /// implied LFE is a documented reading.
    #[test]
    fn every_counter_in_the_verdict_can_make_a_stream_unclean() {
        assert!(ProgramStats::default().is_clean(), "an empty pass is clean");

        let makes_it_unclean: [(&str, fn(&mut ProgramStats)); 6] = [
            ("orphan_dependents", |s| s.orphan_dependents = 1),
            ("layout_changes", |s| s.layout_changes = 1),
            ("misaligned", |s| s.misaligned = 1),
            ("over_capacity", |s| s.over_capacity = 1),
            ("location_errors", |s| s.location_errors = 1),
            ("dependent_dropped", |s| s.dependent_dropped = 1),
        ];
        for (name, set) in makes_it_unclean {
            let mut stats = ProgramStats::default();
            set(&mut stats);
            assert!(!stats.is_clean(), "{name} left the programme looking clean");
        }

        let mut with_errors = ProgramStats::default();
        with_errors.decode_errors.insert((1, 0), 1);
        assert!(
            !with_errors.is_clean(),
            "a decode error left the programme looking clean"
        );

        let leaves_it_clean: [(&str, fn(&mut ProgramStats)); 3] = [
            ("other_program_frames", |s| s.other_program_frames = 1),
            ("lfe_implied", |s| s.lfe_implied = 1),
            ("groups", |s| s.groups = 1),
        ];
        for (name, set) in leaves_it_clean {
            let mut stats = ProgramStats::default();
            set(&mut stats);
            assert!(
                stats.is_clean(),
                "{name} should not make a programme unclean"
            );
        }
    }
    use super::*;
    use crate::header::Syntax;
    use crate::tables::CHANMAP_LOCATIONS;

    fn header(acmod: u8, lfeon: bool, stream_type: StreamType) -> FrameHeader {
        FrameHeader {
            syntax: Syntax::Eac3,
            stream_type,
            substream_id: 0,
            frame_bytes: 2048,
            fscod: 0,
            fscod2: None,
            sample_rate: 48_000,
            blocks: 6,
            acmod,
            lfeon,
            bsid: 16,
            frmsizecod: None,
        }
    }

    /// The two spellings of table E.1.4 must stay in step: `tables.rs` holds
    /// the names, this module holds the locations.
    #[test]
    fn the_channel_map_table_agrees_with_its_names() {
        for (bit, names) in CHANMAP_LOCATIONS.iter().enumerate() {
            let locs = CHANMAP[bit];
            assert_eq!(names.len(), locs.len(), "bit {bit}");
            for (name, loc) in names.iter().zip(locs) {
                assert_eq!(ChannelLoc::from_name(name), Some(*loc), "bit {bit}");
                assert_eq!(loc.name(), *name, "bit {bit}");
            }
        }
    }

    /// Every name table 4.3 prints is a location, so an independent
    /// substream's channels can always be named.
    #[test]
    fn table_4_3_names_are_all_locations() {
        for names in CHANNEL_ORDER {
            for n in names {
                assert!(ChannelLoc::from_name(n).is_some(), "{n}");
            }
        }
    }

    /// The measured dependent substream of a Blu-ray 7.1 track: `acmod 5`,
    /// four coded channels, `chanmap` 0x1a00 naming Ls, Rs and the Lrs/Rrs
    /// pair.
    #[test]
    fn the_measured_seven_one_channel_map_names_the_surrounds() {
        assert_eq!(
            chanmap_locations(0x1a00),
            vec![
                ChannelLoc::Ls,
                ChannelLoc::Rs,
                ChannelLoc::Lrs,
                ChannelLoc::Rrs
            ]
        );
        let h = header(5, false, StreamType::Dependent);
        assert_eq!(h.nchans(), 4);
        let bsi = Bsi {
            chanmap: Some(0x1a00),
            ..Bsi::default()
        };
        let d = dependent_locations(&h, &bsi).expect("four locations for four channels");
        assert_eq!(d.locations.len(), h.nchans());
        assert!(!d.lfe_implied);
    }

    #[test]
    fn a_channel_map_that_does_not_match_the_coded_count_is_an_error() {
        let h = header(7, false, StreamType::Dependent); // five coded channels
        let bsi = Bsi {
            chanmap: Some(0x1a00), // four locations
            ..Bsi::default()
        };
        assert_eq!(
            dependent_locations(&h, &bsi),
            Err(LocationError::Count {
                locations: 4,
                coded: 5
            })
        );
    }

    #[test]
    fn a_map_that_leaves_out_the_lfe_gets_it_appended_and_flagged() {
        let h = header(5, true, StreamType::Dependent); // four fbw plus LFE
        let bsi = Bsi {
            chanmap: Some(0x1a00), // four locations, no LFE bit
            ..Bsi::default()
        };
        let d = dependent_locations(&h, &bsi).expect("the LFE is implied");
        assert!(d.lfe_implied);
        assert_eq!(*d.locations.last().unwrap(), ChannelLoc::Lfe);
        assert_eq!(d.locations.len(), h.nchans());
    }

    #[test]
    fn a_dependent_substream_without_a_map_is_named_by_its_own_acmod() {
        let h = header(1, true, StreamType::Dependent); // centre plus LFE
        let d = dependent_locations(&h, &Bsi::default()).expect("acmod names them");
        assert_eq!(d.locations, vec![ChannelLoc::C, ChannelLoc::Lfe]);
    }

    /// Interchange order is the order of the WAVE mask bits, so a programme's
    /// order and its mask can never disagree.
    #[test]
    fn interchange_rank_follows_the_mask_bit() {
        for loc in [
            ChannelLoc::L,
            ChannelLoc::R,
            ChannelLoc::C,
            ChannelLoc::Lfe,
            ChannelLoc::Lrs,
            ChannelLoc::Rrs,
            ChannelLoc::Ls,
            ChannelLoc::Rs,
        ] {
            assert_eq!(
                u32::from(loc.interchange_rank()),
                loc.wave_mask().trailing_zeros()
            );
        }
        assert_eq!(ChannelLoc::Lw.interchange_rank(), 32);
    }

    /// The eight channels of a Blu-ray 7.1 programme come out in FFmpeg's
    /// order with the canonical 7.1 mask.
    #[test]
    fn a_seven_one_programme_sorts_into_wave_order() {
        use ChannelLoc::{C, L, Lfe, Lrs, Ls, R, Rrs, Rs};
        let mut chans = vec![L, C, R, Ls, Rs, Lfe, Lrs, Rrs];
        chans.sort_by_key(|c| c.interchange_rank());
        assert_eq!(chans, vec![L, R, C, Lfe, Lrs, Rrs, Ls, Rs]);
        let mask = chans.iter().fold(0, |a, c| a | c.wave_mask());
        assert_eq!(mask, 0x63F);
        assert_eq!(mask.count_ones() as usize, chans.len());
    }

    /// The measured Blu-ray 7.1 group: an AC-3 5.1 core followed by a
    /// dependent substream naming Ls, Rs and the rear pair. The surrounds are
    /// replaced, the rear pair is added, and nothing else moves.
    #[test]
    fn a_dependent_substream_replaces_the_surrounds_and_adds_the_rear_pair() {
        use ChannelLoc::{C, L, Lfe, Lrs, Ls, R, Rrs, Rs};
        let mut layout = ProgramLayout::default();
        let core = layout.merge(
            0,
            &independent_locations(&{
                let mut h = header(7, true, StreamType::Independent);
                h.syntax = Syntax::Ac3;
                h
            }),
        );
        assert_eq!(
            (core.replaced, core.added, core.over_capacity),
            (0, 6, 0),
            "the core brings six channels and replaces nothing"
        );
        let dep = layout.merge(1, &chanmap_locations(0x1a00));
        assert_eq!((dep.replaced, dep.added, dep.over_capacity), (2, 2, 0));
        assert_eq!(layout.channels, vec![L, C, R, Ls, Rs, Lfe, Lrs, Rrs]);
        assert_eq!(
            layout.sources,
            vec![
                (0, 0),
                (0, 1),
                (0, 2),
                (1, 0),
                (1, 1),
                (0, 5),
                (1, 2),
                (1, 3)
            ]
        );
        let order = layout.interchange_order();
        let ordered: Vec<ChannelLoc> = order.iter().map(|&i| layout.channels[i]).collect();
        assert_eq!(ordered, vec![L, R, C, Lfe, Lrs, Rrs, Ls, Rs]);
        assert_eq!(
            ordered.iter().fold(0, |a, c| a | c.wave_mask()),
            0x63F,
            "canonical WAVE 7.1"
        );
    }

    /// A programme cannot grow past sixteen channels (clause E.2.8.2).
    #[test]
    fn a_programme_stops_at_sixteen_channels() {
        use ChannelLoc::{
            C, Cs, L, Lc, Lfe, Lfe2, Lrs, Ls, Lsd, Lts, Lw, R, Rc, Rrs, Rs, Rsd, Rts, Rw, Ts, Vhc,
            Vhl, Vhr,
        };
        let all = [
            L, C, R, Ls, Rs, Lc, Rc, Lrs, Rrs, Cs, Ts, Lsd, Rsd, Lw, Rw, Vhl, Vhr, Vhc, Lts, Rts,
            Lfe2, Lfe,
        ];
        let mut layout = ProgramLayout::default();
        let out = layout.merge(0, &all);
        assert_eq!(layout.len(), MAX_PROGRAM_CHANNELS);
        assert_eq!(out.added, MAX_PROGRAM_CHANNELS);
        assert_eq!(out.over_capacity, all.len() - MAX_PROGRAM_CHANNELS);
    }

    /// Merging the same locations twice replaces rather than duplicating, so a
    /// second dependent substream carrying a channel a first one already
    /// supplied takes it over (clause E.2.8.2).
    #[test]
    fn a_later_substream_takes_over_a_channel_an_earlier_one_supplied() {
        use ChannelLoc::{Ls, Rs};
        let mut layout = ProgramLayout::default();
        layout.merge(0, &[Ls, Rs]);
        let second = layout.merge(1, &[Ls]);
        assert_eq!((second.replaced, second.added), (1, 0));
        assert_eq!(layout.channels, vec![Ls, Rs]);
        assert_eq!(layout.sources, vec![(1, 0), (0, 1)]);
    }

    /// The JOC downmix inputs of table 47, including the two that only a
    /// dependent substream can supply -- and they are not the same two in
    /// every configuration.
    #[test]
    fn the_joc_downmix_inputs_cover_seven_channels() {
        use ChannelLoc::{C, L, Lfe, Lrs, Ls, R, Rrs, Rs, Vhl, Vhr};
        // configuration 2 and 4 end in the top front pair, not the rear one
        for cfg in [2u8, 4] {
            let seven: Vec<_> = [L, R, C, Ls, Rs, Vhl, Vhr]
                .iter()
                .map(|c| c.joc_input(cfg))
                .collect();
            assert_eq!(
                seven,
                (0..7).map(Some).collect::<Vec<_>>(),
                "configuration {cfg} takes the top front pair"
            );
            assert_eq!(Lrs.joc_input(cfg), None, "and not the rear pair");
            assert_eq!(Rrs.joc_input(cfg), None);
        }
        // configurations 0 and 3 have five channels and neither pair
        for cfg in [0u8, 3] {
            assert_eq!(Lrs.joc_input(cfg), None);
            assert_eq!(Vhl.joc_input(cfg), None);
            assert_eq!(L.joc_input(cfg), Some(0));
        }
        let inputs: Vec<_> = [L, R, C, Ls, Rs, Lrs, Rrs]
            .iter()
            .map(|c| c.joc_input(1))
            .collect();
        assert_eq!(
            inputs,
            (0..7).map(Some).collect::<Vec<_>>(),
            "configuration 1 takes the rear pair, table 47 inputs 0 to 6"
        );
        assert_eq!(ChannelLoc::from_name("Lb"), Some(Lrs));
        assert_eq!(ChannelLoc::from_name("Rb"), Some(Rrs));
        for cfg in 0..5u8 {
            assert_eq!(Lfe.joc_input(cfg), None, "the LFE bypasses JOC");
        }
    }
}
