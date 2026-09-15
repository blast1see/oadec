//! Error type of the TrueHD parser and decoder.

use oadec_bits::BitError;

/// Anything that stops an access unit from being parsed or decoded.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A field ran past the end of the data.
    #[error(transparent)]
    Bits(#[from] BitError),
    /// Meridian MLP (major sync `F8726FBB`) uses a different syntax.
    #[error("Meridian MLP (FBB) streams are not supported; only Dolby TrueHD (FBA) is")]
    UnsupportedFbb,
    /// The syntax was violated.
    #[error("{0}")]
    Malformed(String),
    /// A major sync changed what the decoder was set up for: the substream
    /// count, `substream_info`, the samples per access unit or the sampling
    /// frequency (`StreamConfig::incompatible_with`). The access unit is not
    /// decoded; everything before it was, and is sound.
    #[error("{what} changed at a major sync")]
    ConfigChanged {
        /// What changed, as a phrase ("the sampling frequency").
        what: &'static str,
    },
}

impl Error {
    pub(crate) fn malformed(what: impl Into<String>) -> Self {
        Self::Malformed(what.into())
    }
}

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;
