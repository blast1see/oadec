//! Bit reader, CRCs and parity helpers for the oadec decoders.
//!
//! Part of the `oadec` object-audio decoder engine. This crate has no I/O and no
//! dependencies; everything here is pure and deterministic.

pub mod crc;
pub mod parity;
pub mod reader;
pub mod writer;

pub use crc::{CRC8_RESTART, CRC8_SUBSTREAM, CRC8_SUBSTREAM_INIT, CRC16_MAJOR_SYNC, Crc8, Crc16};
pub use parity::{fold_nibble, fold_u32, xor_bytes};
pub use reader::{BitError, BitReader};
pub use writer::BitWriter;
