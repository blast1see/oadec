//! Parser state that persists across blocks and access units.
//!
//! Most block-header fields are only sent when they change; the values in force
//! live here, one set per substream, and a restart header resets them.

use crate::au::StreamConfig;
use crate::filter::FilterCoeffs;
use crate::presentation::MAX_PRESENTATIONS;

/// Maximum matrix channels of a substream (the 16-channel presentation).
pub const MAX_CHANNELS: usize = 16;

/// Maximum primitive matrices of a substream.
pub const MAX_MATRICES: usize = 16;

/// Largest access unit in samples (160 at 192 kHz / 176.4 kHz).
pub const MAX_SAMPLES_PER_AU: usize = 160;

/// Guard bit: `new_guards` may follow.
pub const GUARD_PRESENCE: u8 = 1 << 0;
/// Guard bit: `new_huff_offset` may follow in channel parameters.
pub const GUARD_HUFF_OFFSET: u8 = 1 << 1;
/// Guard bit: `new_coeffs_b` (IIR) may follow in channel parameters.
pub const GUARD_IIR: u8 = 1 << 2;
/// Guard bit: `new_coeffs_a` (FIR) may follow in channel parameters.
pub const GUARD_FIR: u8 = 1 << 3;
/// Guard bit: `new_quantiser_step_size` may follow.
pub const GUARD_QUANT_STEP: u8 = 1 << 4;
/// Guard bit: `new_output_shift` may follow.
pub const GUARD_OUTPUT_SHIFT: u8 = 1 << 5;
/// Guard bit: `new_matrixing` may follow.
pub const GUARD_MATRIX: u8 = 1 << 6;
/// Guard bit: `new_block_size` may follow.
pub const GUARD_BLOCK_SIZE: u8 = 1 << 7;

/// Restart sync word of substreams 0 and 1 (MLP-style noise channels).
pub const SYNC_A: u16 = 0x31EA;
/// Restart sync word of substreams 1 and 2 (dither table).
pub const SYNC_B: u16 = 0x31EB;
/// Restart sync word of substream 3 (object presentation, delta matrices).
pub const SYNC_C: u16 = 0x31EC;

/// Per-substream parser state.
#[derive(Debug, Clone)]
pub struct SubstreamState {
    /// A restart header has been parsed since the state was created or invalidated.
    pub restart_seen: bool,
    /// Restart sync word of the current restart section.
    pub sync_word: u16,
    /// First channel coded by this substream.
    pub min_chan: usize,
    /// Last channel coded by this substream.
    pub max_chan: usize,
    /// Last channel the matrices of this substream may write.
    pub max_matrix_chan: usize,
    /// `dither_shift`.
    pub dither_shift: u8,
    /// `dither_seed` at the restart header.
    pub dither_seed: u32,
    /// `max_shift`.
    pub max_shift: u8,
    /// `max_lsbs`.
    pub max_lsbs: u32,
    /// `max_bits`.
    pub max_bits: u8,
    /// `error_protect`.
    pub error_protect: bool,
    /// `lossless_check` of the current restart header.
    pub lossless_check: u8,
    /// `ch_assign` of the current restart header (output channel of each matrix channel).
    pub ch_assign: [u8; MAX_CHANNELS],
    /// Guard bits currently in force.
    pub guards: u8,
    /// Block size in samples.
    pub block_size: usize,
    /// Number of primitive matrices.
    pub primitive_matrices: usize,
    /// Output channel of each matrix.
    pub matrix_ch: [u8; MAX_MATRICES],
    /// Fractional bits of each matrix.
    pub frac_bits: [u8; MAX_MATRICES],
    /// One LSB bypass bit per sample for each matrix (sync words A and B).
    pub lsb_bypass_used: [bool; MAX_MATRICES],
    /// Coefficient shift code of each matrix (sync word C).
    pub cf_shift_code: [i8; MAX_MATRICES],
    /// LSB bypass bits per sample of each matrix (sync word C).
    pub lsb_bypass_bit_count: [u8; MAX_MATRICES],
    /// Coefficient presence mask of each matrix (sync word C).
    pub cf_mask: [u16; MAX_MATRICES],
    /// Delta coefficient bits of each matrix (sync word C).
    pub delta_bits: [u8; MAX_MATRICES],
    /// Delta precision of each matrix (sync word C).
    pub delta_precision: [u8; MAX_MATRICES],
    /// Dither scale of each matrix (sync words B and C).
    pub dither_scale: [u8; MAX_MATRICES],
    /// Raw matrix coefficients (`frac_bits + 2` bit two's complement), indexed by
    /// `[matrix][channel]`; the two extra columns are the noise channels of sync word A.
    pub matrix_coeff: [[i32; MAX_CHANNELS + 2]; MAX_MATRICES],
    /// Raw delta coefficients (`delta_bits + 1` bits) of sync word C, `[matrix][channel]`.
    pub delta_cf: [[i32; MAX_CHANNELS]; MAX_MATRICES],
    /// `interpolation_used` of the last matrixing block (sync word C).
    pub interpolation_used: bool,
    /// Output shift of each matrix channel.
    pub output_shift: [i8; MAX_CHANNELS],
    /// Quantiser step size of each channel.
    pub quant_step_size: [u32; MAX_CHANNELS],
    /// Huffman offset of each channel.
    pub huff_offset: [i32; MAX_CHANNELS],
    /// Huffman code book of each channel (0 = none).
    pub huff_type: [u8; MAX_CHANNELS],
    /// Huffman LSB count of each channel.
    pub huff_lsbs: [u32; MAX_CHANNELS],
    /// FIR (filter A) coefficients of each channel.
    pub fir: [FilterCoeffs; MAX_CHANNELS],
    /// IIR (filter B) coefficients of each channel.
    pub iir: [FilterCoeffs; MAX_CHANNELS],
    /// `crc_present` from the directory entry of the current access unit.
    pub crc_present: bool,
}

impl Default for SubstreamState {
    fn default() -> Self {
        Self {
            restart_seen: false,
            sync_word: 0,
            min_chan: 0,
            max_chan: 0,
            max_matrix_chan: 0,
            dither_shift: 0,
            dither_seed: 0,
            max_shift: 0,
            max_lsbs: 24,
            max_bits: 24,
            error_protect: false,
            lossless_check: 0,
            ch_assign: core::array::from_fn(|i| i as u8),
            guards: 0xFF,
            block_size: 8,
            primitive_matrices: 0,
            matrix_ch: [0; MAX_MATRICES],
            frac_bits: [0; MAX_MATRICES],
            lsb_bypass_used: [false; MAX_MATRICES],
            cf_shift_code: [0; MAX_MATRICES],
            lsb_bypass_bit_count: [0; MAX_MATRICES],
            cf_mask: [0; MAX_MATRICES],
            delta_bits: [0; MAX_MATRICES],
            delta_precision: [0; MAX_MATRICES],
            dither_scale: [0; MAX_MATRICES],
            matrix_coeff: [[0; MAX_CHANNELS + 2]; MAX_MATRICES],
            delta_cf: [[0; MAX_CHANNELS]; MAX_MATRICES],
            interpolation_used: false,
            output_shift: [0; MAX_CHANNELS],
            quant_step_size: [0; MAX_CHANNELS],
            huff_offset: [0; MAX_CHANNELS],
            huff_type: [0; MAX_CHANNELS],
            huff_lsbs: [24; MAX_CHANNELS],
            fir: core::array::from_fn(|_| FilterCoeffs::default()),
            iir: core::array::from_fn(|_| FilterCoeffs::default()),
            crc_present: false,
        }
    }
}

impl SubstreamState {
    /// Resets everything a restart header re-initialises, keeping only the fields
    /// the caller sets from the header itself.
    pub fn reset_for_restart(&mut self) {
        let crc_present = self.crc_present;
        *self = Self {
            crc_present,
            ..Self::default()
        };
    }
}

/// Stream-level parser state.
#[derive(Debug, Clone)]
pub struct ParserState {
    /// Samples per access unit.
    pub samples_per_au: usize,
    /// Major sync `flags`.
    pub flags: u16,
    /// `substream_info`.
    pub substream_info: u8,
    /// Sampling frequency in Hz.
    pub sampling_frequency: u32,
    /// Number of substreams.
    pub substreams: usize,
    /// Per-substream state.
    pub substream: [SubstreamState; MAX_PRESENTATIONS],
}

impl ParserState {
    /// Creates the state for a stream configuration.
    #[must_use]
    pub fn new(config: &StreamConfig) -> Self {
        Self {
            samples_per_au: usize::from(config.samples_per_au),
            flags: config.flags,
            substream_info: config.substream_info,
            sampling_frequency: config.sampling_frequency,
            substreams: usize::from(config.substreams),
            substream: core::array::from_fn(|_| SubstreamState::default()),
        }
    }

    /// Adopts a (possibly changed) configuration; a changed layout invalidates the
    /// substream states until their next restart header.
    pub fn update(&mut self, config: &StreamConfig) {
        let changed = self.samples_per_au != usize::from(config.samples_per_au)
            || self.substream_info != config.substream_info
            || self.substreams != usize::from(config.substreams);
        self.samples_per_au = usize::from(config.samples_per_au);
        self.flags = config.flags;
        self.substream_info = config.substream_info;
        self.sampling_frequency = config.sampling_frequency;
        self.substreams = usize::from(config.substreams);
        if changed {
            for ss in &mut self.substream {
                ss.restart_seen = false;
            }
        }
    }
}
