//! Error type of the TrueHD parser and decoder.

use oadec_bits::BitError;

/// Anything that stops an access unit from being parsed or decoded.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A field ran past the end of the data.
    #[error(transparent)]
    Bits(#[from] BitError),
    /// A stream oadec recognises and does not decode, named.
    #[error("{0}")]
    Unsupported(String),
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
    /// A change of configuration that the decoder followed
    /// ([`crate::decoder::OnConfigChange::Continue`]) left the stream without
    /// the presentation it decodes. Nothing is decoded in its place, and no
    /// other presentation stands in for it; the decoder goes on following
    /// the stream, and decodes the presentation again from the access unit
    /// whose major sync brings it back.
    #[error("presentation {presentation} is not in the stream after a change of configuration")]
    PresentationGone {
        /// The presentation the decoder was set up for.
        presentation: usize,
    },
}

impl Error {
    pub(crate) fn malformed(what: impl Into<String>) -> Self {
        Self::Malformed(what.into())
    }
}

/// Result alias for this crate.
pub type Result<T> = core::result::Result<T, Error>;
