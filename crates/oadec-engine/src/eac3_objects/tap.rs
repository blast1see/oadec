//! A read-only tap on the JOC object path, for measuring where the objects
//! part from those of the Dolby decoder.
//!
//! The Dolby decoder writes only PCM and refuses a rewritten JOC payload, so
//! nothing between its core decode and its objects can be read off it. What
//! can be read is every stage of this decoder: the downmix as the analysis
//! bank and the quadrature hand it to the matrix, the matrix each time slot
//! meets, and the objects the matrix makes before the synthesis bank. A tool
//! that fits Dolby's objects onto the first (`tools/joc_slot_error.py`) then
//! says, slot by slot and band by band, whether what is left is the matrix or
//! the core.
//!
//! The tap only reads. It is handed borrowed slices of the pipeline's own
//! buffers, one time slot at a time, on the thread that decodes; it changes
//! nothing the pipeline computes, and without a tap the object path is the
//! one it has always been. It is not part of [`super::JocOverrides`], which is
//! compared and defaulted as a value.

use oadec_emdf::joc::MAX_CHANNELS;
use oadec_joc::{BANDS, Carry, Complex, JocDecoder};

/// Measurement switches of a tapped decode, kept out of
/// [`super::JocOverrides`] until the command line carries them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TapOptions {
    /// At a splice of the JOC sequence counter, clear the analysis,
    /// quadrature and synthesis banks as well as the matrix history
    /// (`--joc-reset-banks` once the command line has it). Off by default:
    /// the decode is then the one without a tap.
    pub reset_banks: bool,
}

/// What a tap is told once, before the first time slot.
#[derive(Debug, Clone, PartialEq)]
pub struct TapStart {
    /// Sample rate.
    pub rate: u32,
    /// `joc_dmx_config_idx` of the first payload.
    pub dmx_config: u8,
    /// JOC downmix channels, in the order of table 47.
    pub channels: usize,
    /// JOC objects the payload declares.
    pub objects: usize,
    /// What each downmix channel carries into the quadrature.
    pub carry: Vec<Carry>,
    /// Time slots the matrices are held back by (`--joc-lag` or its default).
    pub lag: usize,
    /// Samples an object runs late by against the core it is made of: the
    /// decoder delay, the two banks and the hold of the low-band operator.
    pub object_delay: usize,
}

/// One time slot of the object path, as the pipeline has it.
///
/// Every slice is the pipeline's own buffer, borrowed for the call: a tap that
/// keeps anything copies it.
#[derive(Debug)]
pub struct TapSlot<'a> {
    /// Frames given to the pipeline before this one (0 for the first). The
    /// slots of the flush after the last frame carry the count of frames.
    pub frame: u64,
    /// Time slot within the frame.
    pub ts: usize,
    /// `joc_seq_count` of the frame's payload, `None` for a frame without one
    /// and for the flush.
    pub seq_count: Option<u16>,
    /// This frame's counter said the stream was spliced, and the matrix
    /// history was forgotten before its first slot (clause 6.3.3.3).
    pub reset: bool,
    /// The slot belongs to the zeros run through the banks after the last
    /// frame.
    pub flush: bool,
    /// The downmix after the analysis bank and the quadrature, `[ch][sb]`:
    /// what the matrix of this slot is applied to.
    pub downmix: &'a [[Complex; BANDS]],
    /// The objects the matrix makes of it, `[obj][sb]`, before synthesis.
    pub objects: &'a [[Complex; BANDS]],
    /// The decoder, for the matrix each object meets in this slot
    /// ([`TapSlot::matrix`]).
    pub(super) decoder: &'a JocDecoder,
    /// The clip gain the frame starts from: that of the frame before.
    pub clip_gain_before: f64,
    /// The clip gain of the frame, reached over its first block.
    pub clip_gain: f64,
}

impl TapSlot<'_> {
    /// The interpolated matrix object `obj` meets in this slot, `[ch][sb]`,
    /// after the lag has moved it against the subband samples: the one
    /// `objects[obj]` was made with. `None` outside the objects.
    #[must_use]
    pub fn matrix(&self, obj: usize) -> Option<&[[f64; BANDS]; MAX_CHANNELS]> {
        self.decoder.applied(obj, self.ts)
    }
}

/// What reads the object path a time slot at a time.
pub trait JocTap {
    /// Called once, before the first slot.
    fn start(&mut self, start: &TapStart) {
        let _ = start;
    }

    /// Called for every time slot the pipeline runs, the flush included, in
    /// order, after the matrix and before the synthesis bank.
    fn slot(&mut self, slot: &TapSlot<'_>);
}
