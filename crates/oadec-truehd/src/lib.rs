//! Dolby TrueHD (MLP) bitstream parser and decoder with the 16-channel object presentation.
//!
//! Part of the `oadec` object-audio decoder engine.
//!
//! The crate is layered: [`extract`] frames access units out of a byte stream,
//! [`au`] reads the framing of one access unit (header, [`sync`], directory,
//! [`extra`] data), and the substream layer parses and decodes the segments.

pub mod au;
pub mod channel;
pub mod error;
pub mod extra;
pub mod extract;
pub mod presentation;
pub mod sync;

#[cfg(test)]
pub(crate) mod testutil;

pub use au::{AccessUnit, AuHeader, DirectoryEntry, StreamConfig};
pub use channel::{ChannelLabel, ChannelMeaning, ExtraChannelMeaning};
pub use error::{Error, Result};
pub use extra::{ExtraData, ExtraKind};
pub use extract::{ExtractStats, Extractor, Unit};
pub use presentation::{MAX_PRESENTATIONS, PresentationKind, PresentationMap};
pub use sync::{FormatInfo, MajorSync};
