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

use crate::decoder::{
    ConfigChange, DecodeStats, Decoded, Engine, Lent, OnConfigChange, no_outcome,
};
use crate::error::{Error, Result};
use crate::presentation::MAX_PRESENTATIONS;
use crate::sync::MajorSync;

/// Substreams a stream can carry.
const SUBSTREAMS: usize = 4;

/// Decoder for several presentations of one TrueHD stream at once.
#[derive(Debug, Clone)]
pub struct MultiDecoder {
    engine: Engine,
    /// The lane of each presentation asked for.
    lane: [Option<usize>; MAX_PRESENTATIONS],
}

impl MultiDecoder {
    /// Creates a decoder for `presentations` (each 0..=3, in any order,
    /// repeats allowed) of the stream described by its first major sync.
    ///
    /// # Errors
    /// What [`Decoder::new`](crate::Decoder::new) gives for any one of the
    /// presentations; and for none at all.
    pub fn new(major_sync: &MajorSync, presentations: &[usize]) -> Result<Self> {
        let mut asked: Vec<usize> = presentations.to_vec();
        asked.sort_unstable();
        asked.dedup();
        if asked.is_empty() {
            return Err(Error::malformed("no presentation asked for"));
        }
        if let Some(&p) = asked.iter().find(|&&p| p >= MAX_PRESENTATIONS) {
            return Err(Error::malformed(format!(
                "presentation {p} is not available"
            )));
        }
        let engine = Engine::new(major_sync, &asked)?;
        let mut lane = [None; MAX_PRESENTATIONS];
        for k in 0..engine.lanes() {
            lane[engine.presentation_of(k)] = Some(k);
        }
        Ok(Self { engine, lane })
    }

    /// What to do at a major sync that changes the configuration;
    /// [`OnConfigChange::Stop`] unless set. With
    /// [`OnConfigChange::Continue`], a selected presentation the new
    /// configuration does not carry is [`PresentationOut::Gone`] until a
    /// later change brings it back.
    pub fn on_config_change(&mut self, policy: OnConfigChange) {
        self.engine.on_config_change(policy);
    }

    /// Keeps access units flagged as duplicates instead of marking them, for
    /// every presentation ([`Decoder::keep_duplicates`](crate::Decoder::keep_duplicates)).
    pub fn keep_duplicates(&mut self, keep: bool) {
        self.engine.keep_duplicates(keep);
    }

    /// Decodes one access unit for every selected presentation, standing
    /// silence in for a presentation the unit's damage reaches, as
    /// [`Decoder::decode_concealing`](crate::Decoder::decode_concealing)
    /// does for each.
    ///
    /// # Errors
    /// A change of configuration under [`OnConfigChange::Stop`].
    pub fn decode_concealing(&mut self, unit: &[u8]) -> Result<MultiDecoded<'_>> {
        self.engine.decode_concealing(unit)?;
        let mut outputs = [None; MAX_PRESENTATIONS];
        for (p, lane) in self.lane.iter().enumerate() {
            let Some(k) = *lane else { continue };
            outputs[p] = Some(match self.engine.lent(k) {
                Some(Lent::Samples(decoded, fault)) => {
                    PresentationOut::Available { decoded, fault }
                }
                Some(Lent::Gone) => PresentationOut::Gone,
                None => return Err(no_outcome(p)),
            });
        }
        Ok(MultiDecoded {
            outputs,
            change: self.engine.config_change(),
        })
    }

    /// The counters of presentation `p`, the same a
    /// [`Decoder`](crate::Decoder) of it alone would have kept; `None` when
    /// `p` was not selected.
    #[must_use]
    pub fn stats(&self, p: usize) -> Option<&DecodeStats> {
        let k = (*self.lane.get(p)?)?;
        Some(self.engine.stats(k))
    }

    /// Access units given so far.
    #[must_use]
    pub fn units(&self) -> u64 {
        self.engine.units()
    }

    /// Per substream, the access units it was decoded in: once each, however
    /// many of the selected presentations read it.
    #[must_use]
    pub fn substream_decodes(&self) -> [u64; SUBSTREAMS] {
        self.engine.substream_decodes()
    }

    /// Per substream, the access units in which it was needed and present
    /// and was not decoded. Needed: a selected presentation that the
    /// configuration in force carries reads it. Present: the configuration
    /// in force has it. Not decoded: a fault in its segment (one that does
    /// not parse, or fails its parity, CRC, end-pointer or sample-count
    /// check), a wait for its next restart header after a fault, or the
    /// unit's framing failing -- a unit whose framing fails counts as
    /// carrying every substream of the configuration in force. For every
    /// substream `substream_decodes()[s] + substream_faults()[s]` is the
    /// number of units in which it was needed and present.
    #[must_use]
    pub fn substream_faults(&self) -> [u64; SUBSTREAMS] {
        self.engine.substream_faults()
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
