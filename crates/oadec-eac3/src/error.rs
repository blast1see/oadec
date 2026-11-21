//! Error type of the E-AC-3 decoder.

use oadec_bits::BitError;

/// Anything that stops a syncframe from being parsed or decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Eac3Error {
    /// The bytes do not start with the sync word `0x0B77`.
    #[error("no sync word at the start of the frame")]
    NoSync,
    /// The frame is shorter than its header says.
    #[error("frame needs {needed} bytes, {available} available")]
    Truncated { needed: usize, available: usize },
    /// A field has a value the specification reserves.
    #[error("reserved value {value} in {field}")]
    Reserved { field: &'static str, value: u32 },
    /// The bit stream identification is one this decoder must not decode.
    #[error("bit stream identification {0} is not decodable")]
    Bsid(u8),
    /// Ran out of bits inside the frame.
    #[error("bit stream ends early: {0}")]
    Bits(#[from] BitError),
    /// A syntactic constraint of the specification is violated.
    #[error("{0}")]
    Syntax(&'static str),
    /// A feature the decoder does not implement yet.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
    /// A CRC did not check.
    #[error("{0} check failed")]
    Crc(&'static str),
    /// A programme was asked for by name and no frame of it came: `wanted`
    /// is the substream id of its independent substream, and `seen` the
    /// substream ids of the independent substreams that did come, ascending,
    /// each once (clause E.2.8.3).
    #[error("programme {wanted} is not in the stream; the programmes in it are {seen:?}")]
    ProgrammeNotFound { wanted: u8, seen: Vec<u8> },
}

/// Result alias of this crate.
pub type Result<T> = std::result::Result<T, Eac3Error>;
