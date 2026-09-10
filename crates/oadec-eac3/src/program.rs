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

use crate::bsi::Bsi;
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

    /// The JOC downmix input this location feeds (TS 103 420 table 53), if
    /// any. Table 53 names inputs 5 and 6 `Lb` and `Rb`; they are the rear
    /// surround pair that table E.1.4 calls `Lrs` and `Rrs`.
    #[must_use]
    pub const fn joc_input(self) -> Option<usize> {
        use ChannelLoc::{C, L, Lrs, Ls, R, Rrs, Rs};
        Some(match self {
            L => 0,
            R => 1,
            C => 2,
            Ls => 3,
            Rs => 4,
            Lrs => 5,
            Rrs => 6,
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
) -> Result<DependentLocations, LocationError> {
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

#[cfg(test)]
mod tests {
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

    /// The JOC downmix inputs of table 53, including the two the rear pair
    /// feeds that only a dependent substream can supply.
    #[test]
    fn the_joc_downmix_inputs_cover_seven_channels() {
        use ChannelLoc::{C, L, Lfe, Lrs, Ls, R, Rrs, Rs};
        let inputs: Vec<_> = [L, R, C, Ls, Rs, Lrs, Rrs]
            .iter()
            .map(|c| c.joc_input())
            .collect();
        assert_eq!(
            inputs,
            (0..7).map(Some).collect::<Vec<_>>(),
            "table 53 inputs 0 to 6"
        );
        assert_eq!(ChannelLoc::from_name("Lb"), Some(Lrs));
        assert_eq!(ChannelLoc::from_name("Rb"), Some(Rrs));
        assert_eq!(Lfe.joc_input(), None, "the LFE bypasses JOC");
    }
}
