//! Dolby TrueHD (MLP) bitstream parser and decoder with the 16-channel object presentation.
//!
//! Part of the `oadec` object-audio decoder engine.
//!
//! The crate is layered: [`extract`] frames access units out of a byte stream,
//! [`au`] reads the framing of one access unit (header, [`sync`], directory,
//! [`extra`] data), and the substream layer ([`segment`], [`block`], [`restart`],
//! [`matrix`], [`filter`], [`huffman`]) parses the segments against a persistent
//! [`state`].

pub mod au;
pub mod block;
pub mod channel;
pub mod decoder;
pub mod dither;
pub mod error;
pub mod extra;
pub mod extract;
pub mod filter;
pub mod huffman;
pub mod matrix;
pub mod presentation;
pub mod restart;
pub mod segment;
pub mod state;
pub mod sync;
pub mod timing;

#[cfg(test)]
pub(crate) mod testutil;

pub use au::{AccessUnit, AuHeader, DirectoryEntry, StreamConfig};
pub use block::{Block, BlockHeader, ChannelParams, SampleBuffer};
pub use channel::{ChannelLabel, ChannelMeaning, ExtraChannelMeaning};
pub use decoder::{DecodeStats, Decoded, Decoder};
pub use error::{Error, Result};
pub use extra::{ExtraData, ExtraKind};
pub use extract::{ExtractStats, Extractor, Unit};
pub use filter::{FilterCoeffs, FilterKind};
pub use matrix::Matrixing;
pub use presentation::{MAX_PRESENTATIONS, PresentationKind, PresentationMap};
pub use restart::RestartHeader;
pub use segment::{Segment, Terminator};
pub use state::{ParserState, SubstreamState};
pub use sync::{FormatInfo, MajorSync};
pub use timing::{Branch, BranchConditions, RestartTiming, StreamTiming, TimingModel};
