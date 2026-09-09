//! AC-3 and Enhanced AC-3 (E-AC-3) core decoder written from ETSI TS 102 366
//! V1.4.1, with the skip fields captured for the EMDF metadata that JOC
//! streams carry (ETSI TS 103 420 clause 8). Enhanced coupling and transient
//! pre-noise processing follow ATSC A/52:2018 annex E, which carries clauses
//! that TS 102 366 V1.4.1 dropped; see `docs/eac3.md`.
//!
//! Part of the `oadec` object-audio decoder engine. The decoder produces the
//! coded channels as floating point samples without dynamic range control,
//! dialogue normalisation or downmixing; those are reported, not applied.

pub mod bitalloc;
pub mod bsi;
pub mod decoder;
pub mod ecpl;
pub mod error;
pub mod frame;
pub mod header;
pub mod imdct;
pub mod tables;
pub mod tpnp;
pub mod vq;

pub use bsi::Bsi;
pub use decoder::{Decoded, Decoder, find_sync};
pub use error::{Eac3Error, Result};
pub use frame::{
    Block, BlockInfo, Coverage, EcplBlock, EcplChannel, Frame, Noise, Options, Partial, Transient,
};
pub use header::{BLOCK_SAMPLES, FrameHeader, SYNC_WORD, StreamType, Syntax};
