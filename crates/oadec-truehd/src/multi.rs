//! Several presentations of one stream in one pass.
//!
//! A [`Decoder`](crate::Decoder) decodes one presentation, and the
//! substreams under it again for every presentation it is asked for: the
//! four presentations of an object stream read substream 0 four times. A
//! [`MultiDecoder`] reads each substream of an access unit once -- the
//! Huffman decoding, the prediction (FIR and IIR), the recorrelation and the
//! state its blocks and restart headers carry -- whatever number of the
//! selected presentations read it, and gives every selected presentation
//! the samples, counters, faults and lossless check results a `Decoder` of
//! that presentation would have given.
//!
//! What belongs to a presentation is kept apart: the matrices of its own
//! substream applied to the recorrelated channels, the channel assignment,
//! the output, the lossless check, the dither and noise drawn for its
//! matrices, its damage and its concealment.

use crate::decoder::{ConfigChange, DecodeStats, Decoded, OnConfigChange};
use crate::error::{Error, Result};
use crate::presentation::MAX_PRESENTATIONS;
use crate::sync::MajorSync;

/// Substreams a stream can carry.
const SUBSTREAMS: usize = 4;

/// Decoder for several presentations of one TrueHD stream at once.
#[derive(Debug)]
pub struct MultiDecoder {
    /// The presentations asked for.
    selected: [bool; MAX_PRESENTATIONS],
    /// What a change of configuration does.
    on_config_change: OnConfigChange,
    /// Keeps access units flagged as duplicates instead of marking them.
    keep_duplicates: bool,
    /// The counters of each presentation, as its own decoder keeps them.
    stats: [DecodeStats; MAX_PRESENTATIONS],
    /// Access units given.
    units: u64,
    /// Per substream, the access units it was decoded in.
    substream_decodes: [u64; SUBSTREAMS],
    /// Per substream, the access units in which a selected presentation
    /// needed it, the unit carried it, and it could not be decoded: a fault
    /// in its segment, or a wait for its next restart header after one.
    substream_faults: [u64; SUBSTREAMS],
}

impl MultiDecoder {
    /// Creates a decoder for `presentations` (each 0..=3, in any order,
    /// repeats allowed) of the stream described by its first major sync.
    ///
    /// # Errors
    /// What [`Decoder::new`](crate::Decoder::new) gives for any one of the
    /// presentations; and for none at all.
    pub fn new(major_sync: &MajorSync, presentations: &[usize]) -> Result<Self> {
        let _ = (major_sync, presentations);
        Err(Error::Unsupported(
            "decoding several presentations in one pass is not built yet".to_string(),
        ))
    }

    /// What to do at a major sync that changes the configuration;
    /// [`OnConfigChange::Stop`] unless set. With
    /// [`OnConfigChange::Continue`], a selected presentation the new
    /// configuration does not carry is [`PresentationOut::Gone`] until a
    /// later change brings it back.
    pub fn on_config_change(&mut self, policy: OnConfigChange) {
        self.on_config_change = policy;
    }

    /// Keeps access units flagged as duplicates instead of marking them, for
    /// every presentation ([`Decoder::keep_duplicates`](crate::Decoder::keep_duplicates)).
    pub fn keep_duplicates(&mut self, keep: bool) {
        self.keep_duplicates = keep;
    }

    /// Decodes one access unit for every selected presentation, standing
    /// silence in for a presentation the unit's damage reaches, as
    /// [`Decoder::decode_concealing`](crate::Decoder::decode_concealing)
    /// does for each.
    ///
    /// # Errors
    /// A change of configuration under [`OnConfigChange::Stop`].
    pub fn decode_concealing(&mut self, unit: &[u8]) -> Result<MultiDecoded<'_>> {
        let _ = unit;
        Err(Error::Unsupported(
            "decoding several presentations in one pass is not built yet".to_string(),
        ))
    }

    /// The counters of presentation `p`, the same a
    /// [`Decoder`](crate::Decoder) of it alone would have kept; `None` when
    /// `p` was not selected.
    #[must_use]
    pub fn stats(&self, p: usize) -> Option<&DecodeStats> {
        if self.selected.get(p).copied().unwrap_or(false) {
            self.stats.get(p)
        } else {
            None
        }
    }

    /// Access units given so far.
    #[must_use]
    pub fn units(&self) -> u64 {
        self.units
    }

    /// Per substream, the access units it was decoded in: once each, however
    /// many of the selected presentations read it.
    #[must_use]
    pub fn substream_decodes(&self) -> [u64; SUBSTREAMS] {
        self.substream_decodes
    }

    /// Per substream, the access units in which a selected presentation
    /// needed it, the unit carried it, and it could not be decoded: a fault
    /// in its segment, or a wait for its next restart header after one.
    #[must_use]
    pub fn substream_faults(&self) -> [u64; SUBSTREAMS] {
        self.substream_faults
    }
}

/// One access unit decoded by [`MultiDecoder::decode_concealing`], lent out.
#[derive(Debug, Clone, Copy)]
pub struct MultiDecoded<'a> {
    /// What each presentation came to; `None` for one not selected.
    outputs: [Option<PresentationOut<'a>>; MAX_PRESENTATIONS],
    /// The change of configuration the unit carries.
    change: Option<&'a ConfigChange>,
}

impl<'a> MultiDecoded<'a> {
    /// What presentation `p` came to in this access unit; `None` when `p`
    /// was not selected or is not a presentation at all.
    #[must_use]
    pub fn presentation(&self, p: usize) -> Option<PresentationOut<'a>> {
        self.outputs.get(p).copied().flatten()
    }

    /// The change of configuration this access unit carries, if it carries
    /// one and the decoder followed it.
    #[must_use]
    pub fn config_change(&self) -> Option<&'a ConfigChange> {
        self.change
    }
}

/// What one selected presentation came to in one access unit.
#[derive(Debug, Clone, Copy)]
pub enum PresentationOut<'a> {
    /// The presentation is in the stream: its samples, and the fault that
    /// was concealed in it if any.
    Available {
        /// The samples, as a [`Decoder`](crate::Decoder) of the presentation
        /// alone gives them.
        decoded: Decoded<'a>,
        /// The fault concealed in this presentation.
        fault: Option<&'a Error>,
    },
    /// The configuration in force does not carry the presentation: it has no
    /// samples in this access unit.
    Gone,
}
