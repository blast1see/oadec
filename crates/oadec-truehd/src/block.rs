//! Blocks: the block header (parameters that changed) and the block data (coded
//! samples).
//!
//! A block header only carries the parameters that change; the values in force
//! live in the [`SubstreamState`]. The parsed samples of an access unit are
//! appended to a caller-owned [`SampleBuffer`] so that no large structure is
//! moved around per block.

use oadec_bits::{BitReader, CRC8_RESTART, Crc8};

use crate::error::{Error, Result};
use crate::filter::{FilterCoeffs, FilterKind};
use crate::huffman;
use crate::matrix::Matrixing;
use crate::restart::RestartHeader;
use crate::state::{
    GUARD_BLOCK_SIZE, GUARD_FIR, GUARD_HUFF_OFFSET, GUARD_IIR, GUARD_MATRIX, GUARD_OUTPUT_SHIFT,
    GUARD_PRESENCE, GUARD_QUANT_STEP, MAX_CHANNELS, MAX_MATRICES, MAX_SAMPLES_PER_AU, ParserState,
    SYNC_C, SubstreamState,
};

/// Upper bound of `block_data_bits`.
pub const MAX_BLOCK_DATA_BITS: u32 = 16000;

/// The CRC-8 `block_header_crc` is: polynomial `0x1D`, the bits fed most
/// significant first, the register shifted and then the bit XORed in (the
/// restart header's CRC with another start).
///
/// No public document defines the field. The rule here is confirmed against
/// Dolby's decoder as a black box: streams written with it are accepted and
/// decode bit-exact, and a stream whose check bytes break it, every other
/// check right, is refused (`docs/truehd.md`, *Protected blocks*).
pub const BLOCK_HEADER_CRC: Crc8 = CRC8_RESTART;

/// Where the register of `block_header_crc` starts, and so the check of a
/// block that has no block header.
pub const BLOCK_HEADER_CRC_INIT: u8 = 0xA2;

/// The check `block_header_crc` holds of a block: [`BLOCK_HEADER_CRC`] from
/// [`BLOCK_HEADER_CRC_INIT`] over bits `header.0..header.1` of `data`, the
/// bits of the block header alone; [`BLOCK_HEADER_CRC_INIT`] when the block
/// has none.
///
/// # Panics
/// Panics if the range lies outside `data`.
#[must_use]
pub fn block_header_crc(data: &[u8], header: Option<(usize, usize)>) -> u8 {
    match header {
        Some((start, end)) => {
            BLOCK_HEADER_CRC.update_bits(BLOCK_HEADER_CRC_INIT, data, start, end - start)
        }
        None => BLOCK_HEADER_CRC_INIT,
    }
}

/// The coded samples of one substream for one access unit.
///
/// `samples[n][ch]` is the value of channel `ch` at sample `n` after Huffman
/// decoding, offset and quantiser shift (before recorrelation);
/// `bypassed_lsb[n][pmi]` the bypassed LSBs of matrix `pmi`.
#[derive(Debug, Clone)]
pub struct SampleBuffer {
    /// Decoded (pre-recorrelation) samples.
    pub samples: [[i32; MAX_CHANNELS]; MAX_SAMPLES_PER_AU],
    /// Bypassed LSBs per matrix.
    pub bypassed_lsb: [[i32; MAX_MATRICES]; MAX_SAMPLES_PER_AU],
    /// Samples written so far in the current access unit.
    pub len: usize,
}

impl Default for SampleBuffer {
    fn default() -> Self {
        Self {
            samples: [[0; MAX_CHANNELS]; MAX_SAMPLES_PER_AU],
            bypassed_lsb: [[0; MAX_MATRICES]; MAX_SAMPLES_PER_AU],
            len: 0,
        }
    }
}

impl SampleBuffer {
    /// Forgets the samples of the previous access unit.
    pub fn clear(&mut self) {
        self.len = 0;
    }
}

/// What a `channel_parameters()` block changed for one channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelParams {
    /// New FIR (filter A) coefficients were sent.
    pub new_fir: bool,
    /// New IIR (filter B) coefficients were sent.
    pub new_iir: bool,
    /// A new `huff_offset` was sent.
    pub new_huff_offset: bool,
    /// `huff_type` (0 = plain, 1..=3 = code book).
    pub huff_type: u8,
    /// `huff_lsbs`.
    pub huff_lsbs: u8,
}

impl ChannelParams {
    fn parse(reader: &mut BitReader<'_>, ss: &mut SubstreamState, ch: usize) -> Result<Self> {
        let mut cp = Self::default();
        if ss.guards & GUARD_FIR != 0 && reader.read_bool()? {
            ss.fir[ch] = FilterCoeffs::parse(reader, FilterKind::Fir)?;
            cp.new_fir = true;
        }
        if ss.guards & GUARD_IIR != 0 && reader.read_bool()? {
            ss.iir[ch] = FilterCoeffs::parse(reader, FilterKind::Iir)?;
            cp.new_iir = true;
        }
        let (fir, iir) = (&ss.fir[ch], &ss.iir[ch]);
        if fir.order + iir.order > 8 {
            return Err(Error::malformed(format!(
                "channel {ch}: FIR order {} plus IIR order {} exceeds 8",
                fir.order, iir.order
            )));
        }
        if fir.order != 0 && iir.order != 0 && fir.coeff_q != iir.coeff_q {
            return Err(Error::malformed(format!(
                "channel {ch}: FIR coeff_q {} differs from IIR coeff_q {}",
                fir.coeff_q, iir.coeff_q
            )));
        }
        if ss.guards & GUARD_HUFF_OFFSET != 0 && reader.read_bool()? {
            ss.huff_offset[ch] = reader.read_signed(15)?;
            cp.new_huff_offset = true;
        }
        cp.huff_type = reader.read(2)? as u8;
        cp.huff_lsbs = reader.read(5)? as u8;
        let max_lsbs = if ss.sync_word == SYNC_C { 31 } else { 24 };
        if cp.huff_lsbs > max_lsbs {
            return Err(Error::malformed(format!(
                "channel {ch}: huff_lsbs {} exceeds {max_lsbs}",
                cp.huff_lsbs
            )));
        }
        ss.huff_type[ch] = cp.huff_type;
        ss.huff_lsbs[ch] = u32::from(cp.huff_lsbs);
        Ok(cp)
    }
}

/// What a block header changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BlockHeader {
    /// New guard bits.
    pub new_guards: Option<u8>,
    /// New block size.
    pub new_block_size: Option<u16>,
    /// A `matrixing()` block was present.
    pub matrixing: Option<Matrixing>,
    /// New output shifts were sent.
    pub new_output_shift: bool,
    /// New quantiser step sizes were sent.
    pub new_quant_step_size: bool,
    /// Channel parameters that were sent, per channel.
    pub channel: [Option<ChannelParams>; MAX_CHANNELS],
}

impl BlockHeader {
    fn parse(
        reader: &mut BitReader<'_>,
        state: &mut ParserState,
        substream: usize,
    ) -> Result<Self> {
        let samples_per_au = state.samples_per_au;
        let ss = &mut state.substream[substream];
        let mut bh = Self::default();
        if ss.guards & GUARD_PRESENCE != 0 && reader.read_bool()? {
            ss.guards = reader.read(8)? as u8;
            bh.new_guards = Some(ss.guards);
        }
        if ss.guards & GUARD_BLOCK_SIZE != 0 && reader.read_bool()? {
            let block_size = reader.read(9)? as usize;
            if !(8..=MAX_SAMPLES_PER_AU).contains(&block_size) || block_size > samples_per_au {
                return Err(Error::malformed(format!(
                    "block size {block_size} outside 8..={} for {samples_per_au} samples per access unit",
                    MAX_SAMPLES_PER_AU.min(samples_per_au)
                )));
            }
            ss.block_size = block_size;
            bh.new_block_size = Some(block_size as u16);
        }
        if ss.guards & GUARD_MATRIX != 0 && reader.read_bool()? {
            bh.matrixing = Some(Matrixing::parse(reader, ss)?);
        }
        if ss.guards & GUARD_OUTPUT_SHIFT != 0 && reader.read_bool()? {
            for ch in 0..=ss.max_matrix_chan {
                let shift = reader.read_signed(4)? as i8;
                if shift > ss.max_shift as i8 {
                    return Err(Error::malformed(format!(
                        "output_shift {shift} of channel {ch} exceeds max_shift {}",
                        ss.max_shift
                    )));
                }
                ss.output_shift[ch] = shift;
            }
            bh.new_output_shift = true;
        }
        if ss.guards & GUARD_QUANT_STEP != 0 && reader.read_bool()? {
            for ch in 0..=ss.max_chan {
                ss.quant_step_size[ch] = reader.read(4)?;
            }
            bh.new_quant_step_size = true;
        }
        for ch in ss.min_chan..=ss.max_chan {
            if reader.read_bool()? {
                bh.channel[ch] = Some(ChannelParams::parse(reader, ss, ch)?);
            }
        }
        Ok(bh)
    }
}

/// One parsed block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Restart header, when the block carried one.
    pub restart: Option<RestartHeader>,
    /// Block header, when present.
    pub header: Option<BlockHeader>,
    /// Samples in this block.
    pub block_size: usize,
    /// `block_data_bits` (present with `error_protect`).
    pub block_data_bits: Option<u16>,
    /// Whether the number of bits consumed by the block data matched
    /// `block_data_bits` (always true without error protection).
    pub block_data_bits_ok: bool,
    /// `block_header_crc` (present with `error_protect`).
    pub block_header_crc: Option<u8>,
    /// Where the block header lies in the data of the reader, in bits from
    /// its first bit to the one after its last: after the flags that say
    /// a block header and a restart header follow, and after the restart
    /// header; `None` for a block without one.
    pub header_bits: Option<(usize, usize)>,
    /// What `block_header_crc` has to be ([`block_header_crc`]), with
    /// `error_protect`.
    pub block_header_crc_expected: Option<u8>,
}

impl Block {
    /// Parses one block of `substream`, appending its samples to `buf`.
    pub fn parse(
        reader: &mut BitReader<'_>,
        state: &mut ParserState,
        substream: usize,
        buf: &mut SampleBuffer,
    ) -> Result<Self> {
        let mut block = Self {
            restart: None,
            header: None,
            block_size: 0,
            block_data_bits: None,
            block_data_bits_ok: true,
            block_header_crc: None,
            header_bits: None,
            block_header_crc_expected: None,
        };
        if reader.read_bool()? {
            if reader.read_bool()? {
                block.restart = Some(RestartHeader::parse(reader, state, substream)?);
            }
            if !state.substream[substream].restart_seen {
                return Err(Error::malformed(format!(
                    "substream {substream}: block header before any restart header"
                )));
            }
            let start = reader.position();
            block.header = Some(BlockHeader::parse(reader, state, substream)?);
            block.header_bits = Some((start, reader.position()));
        }
        let ss = &mut state.substream[substream];
        if !ss.restart_seen {
            return Err(Error::malformed(format!(
                "substream {substream}: block data before any restart header"
            )));
        }
        block.block_size = ss.block_size;
        if ss.error_protect {
            let bits = reader.read(16)?;
            if bits > MAX_BLOCK_DATA_BITS {
                return Err(Error::malformed(format!(
                    "block_data_bits {bits} exceeds {MAX_BLOCK_DATA_BITS}"
                )));
            }
            block.block_data_bits = Some(bits as u16);
        }
        let start = reader.position();
        read_block_data(reader, ss, block.block_size, buf)?;
        if let Some(expected) = block.block_data_bits {
            block.block_data_bits_ok = reader.position() - start == usize::from(expected);
        }
        if ss.error_protect {
            block.block_header_crc = Some(reader.read(8)? as u8);
            block.block_header_crc_expected =
                Some(block_header_crc(reader.data(), block.header_bits));
        }
        Ok(block)
    }

    /// Whether `block_header_crc` holds the check of the block header; true
    /// for a block without `error_protect`.
    #[must_use]
    pub fn block_header_crc_ok(&self) -> bool {
        match (self.block_header_crc, self.block_header_crc_expected) {
            (Some(stated), Some(expected)) => stated == expected,
            (None, _) => true,
            (Some(_), None) => false,
        }
    }
}

/// Reads `block_size` samples of every coded channel into `buf`.
#[allow(
    clippy::needless_range_loop,
    reason = "several arrays are indexed by the same sample and channel index"
)]
fn read_block_data(
    reader: &mut BitReader<'_>,
    ss: &SubstreamState,
    block_size: usize,
    buf: &mut SampleBuffer,
) -> Result<()> {
    if buf.len + block_size > MAX_SAMPLES_PER_AU {
        return Err(Error::malformed(format!(
            "block of {block_size} samples overflows the access unit ({} already decoded)",
            buf.len
        )));
    }
    for ch in ss.min_chan..=ss.max_chan {
        if ss.huff_lsbs[ch] > ss.max_lsbs {
            return Err(Error::malformed(format!(
                "channel {ch}: huff_lsbs {} exceeds max_lsbs {}",
                ss.huff_lsbs[ch], ss.max_lsbs
            )));
        }
        if ss.quant_step_size[ch] > ss.huff_lsbs[ch] {
            return Err(Error::malformed(format!(
                "channel {ch}: quantiser step size {} exceeds huff_lsbs {}",
                ss.quant_step_size[ch], ss.huff_lsbs[ch]
            )));
        }
    }
    let object_presentation = ss.sync_word == SYNC_C;
    let limit: i64 = if object_presentation {
        1 << 31
    } else {
        1 << 23
    };
    let matrices = ss.primitive_matrices;

    // Per-channel constants of the sample loop. A coded sample is
    // `((lsbs + (code << lsb_bits) + bias) << qss)`: the bias folds the Huffman
    // offset together with the sign convention of the code book (or of the plain
    // two's-complement LSB field).
    struct Chan {
        lsb_bits: u32,
        huff_type: u8,
        qss: u32,
        bias: i64,
    }
    let chans: [Chan; MAX_CHANNELS] = core::array::from_fn(|ch| {
        let qss = ss.quant_step_size[ch];
        let lsb_bits = ss.huff_lsbs[ch].saturating_sub(qss);
        let huff_type = ss.huff_type[ch];
        let sign = if huff_type != 0 {
            let shift = lsb_bits as i32 + 2 - i32::from(huff_type);
            if shift < 0 { 0 } else { 1i64 << shift }
        } else if lsb_bits > 0 {
            1i64 << (lsb_bits - 1)
        } else {
            0
        };
        Chan {
            lsb_bits,
            huff_type,
            qss,
            bias: i64::from(ss.huff_offset[ch]) - sign,
        }
    });

    // Bypassed LSB bits per matrix, and whether any matrix sends them at all.
    let bypass_bits: [u32; MAX_MATRICES] = core::array::from_fn(|pmi| {
        if pmi >= matrices {
            0
        } else if object_presentation {
            u32::from(ss.lsb_bypass_bit_count[pmi])
        } else {
            u32::from(ss.lsb_bypass_used[pmi])
        }
    });
    let any_bypass = bypass_bits.iter().any(|&b| b != 0);

    // The sample loop works on a local cursor over the raw bytes and hands the
    // final position back to the reader once.
    let data = reader.data();
    let len_bits = reader.len_bits();
    let mut pos = reader.position();

    for n in buf.len..buf.len + block_size {
        let lsb = &mut buf.bypassed_lsb[n];
        if any_bypass {
            for pmi in 0..matrices {
                let bits = bypass_bits[pmi];
                lsb[pmi] = if bits != 0 {
                    let (window, valid) = window_at(data, len_bits, pos);
                    if bits as usize > valid {
                        return Err(bit_error(reader, pos, bits));
                    }
                    pos += bits as usize;
                    (window >> (64 - bits)) as i32
                } else {
                    0
                };
            }
        } else {
            *lsb = [0; MAX_MATRICES];
        }
        let row = &mut buf.samples[n];
        for ch in ss.min_chan..=ss.max_chan {
            let c = &chans[ch];
            // One 64-bit window holds the longest code (9 bits) and up to 31 LSBs.
            let (window, valid) = window_at(data, len_bits, pos);
            let (code, code_len) = if c.huff_type != 0 {
                huffman::decode_window(window, c.huff_type)
                    .ok_or_else(|| Error::malformed("Huffman code without a terminating bit"))?
            } else {
                (0, 0)
            };
            let total = code_len + c.lsb_bits;
            if total as usize > valid {
                return Err(bit_error(reader, pos, total));
            }
            let lsbs = if c.lsb_bits > 0 {
                ((window << code_len) >> (64 - c.lsb_bits)) as i64
            } else {
                0
            };
            pos += total as usize;
            let value = (lsbs + (i64::from(code) << c.lsb_bits) + c.bias) << c.qss;
            if value < -limit || value >= limit {
                return Err(Error::malformed(format!(
                    "channel {ch} sample {n}: value {value} outside the {}-bit range",
                    if object_presentation { 32 } else { 24 }
                )));
            }
            row[ch] = value as i32;
        }
    }
    reader.seek(pos)?;
    buf.len += block_size;
    Ok(())
}

/// The next bits at `pos`, left-aligned in a 64-bit word, and how many of them
/// are valid (at least 57 whenever that many remain).
#[inline(always)]
fn window_at(data: &[u8], len_bits: usize, pos: usize) -> (u64, usize) {
    let byte = pos >> 3;
    let offset = pos & 7;
    let word = match data.get(byte..byte + 8) {
        Some(chunk) => u64::from_be_bytes(chunk.try_into().expect("eight bytes")),
        None => {
            let mut tmp = [0u8; 8];
            if let Some(tail) = data.get(byte..) {
                tmp[..tail.len()].copy_from_slice(tail);
            }
            u64::from_be_bytes(tmp)
        }
    };
    (
        word << offset,
        (64 - offset).min(len_bits.saturating_sub(pos)),
    )
}

/// Builds the error a read of `bits` bits at `pos` would have produced.
#[cold]
fn bit_error(reader: &mut BitReader<'_>, pos: usize, bits: u32) -> Error {
    match reader.seek(pos).and_then(|()| reader.skip(bits as usize)) {
        Ok(_) => Error::malformed("block data ends inside a sample"),
        Err(e) => e.into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::restart::tests::{atmos_state, push_restart_header};
    use crate::state::SYNC_A;
    use crate::testutil::BitWriter;

    /// Appends a block header that sets a block size and, per channel, plain
    /// coding with `huff_lsbs` bits (guards left at their reset value 0xFF).
    pub(crate) fn push_plain_block_header(
        w: &mut BitWriter,
        block_size: u16,
        min_chan: u8,
        max_chan: u8,
        huff_lsbs: u8,
    ) {
        w.push(1, 0); // new_guards
        w.push(1, 1); // new_block_size
        w.push(9, u64::from(block_size));
        w.push(1, 0); // new_matrixing
        w.push(1, 0); // new_output_shift
        w.push(1, 0); // new_quantiser_step_size
        for _ in min_chan..=max_chan {
            w.push(1, 1); // channel params present
            w.push(1, 0); // no FIR
            w.push(1, 0); // no IIR
            w.push(1, 0); // no huff_offset
            w.push(2, 0); // plain coding
            w.push(5, u64::from(huff_lsbs));
        }
    }

    #[test]
    fn restart_block_with_plain_samples() {
        let mut w = BitWriter::default();
        w.push(1, 1); // block_header_exists
        w.push(1, 1); // restart_header_exists
        push_restart_header(&mut w, SYNC_A, 0, 1, 1, false);
        push_plain_block_header(&mut w, 8, 0, 1, 4);
        // 8 samples x 2 channels, 4 bits each: value = code - 8
        for n in 0..8u64 {
            w.push(4, n); // ch0: n - 8
            w.push(4, 15 - n); // ch1: 7 - n
        }
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let mut r = BitReader::new(&w.bytes);
        let block = Block::parse(&mut r, &mut state, 0, &mut buf).unwrap();
        assert!(block.restart.is_some());
        let header = block.header.unwrap();
        assert_eq!(header.new_block_size, Some(8));
        assert!(
            header.channel[0].is_some()
                && header.channel[1].is_some()
                && header.channel[2].is_none()
        );
        assert_eq!(block.block_size, 8);
        assert_eq!(buf.len, 8);
        for n in 0..8 {
            assert_eq!(buf.samples[n][0], n as i32 - 8);
            assert_eq!(buf.samples[n][1], 7 - n as i32);
        }
        assert_eq!(r.position(), w.len);
    }

    #[test]
    fn huffman_samples_with_offset_and_quantiser_step() {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, false);
        w.push(1, 0); // new_guards
        w.push(1, 1); // new_block_size
        w.push(9, 8);
        w.push(1, 0); // new_matrixing
        w.push(1, 0); // new_output_shift
        w.push(1, 1); // new_quantiser_step_size
        w.push(4, 2); // qss = 2 for channel 0
        w.push(1, 1); // channel 0 params
        w.push(1, 0);
        w.push(1, 0);
        w.push(1, 1); // huff_offset
        w.push(15, 100);
        w.push(2, 1); // book 1
        w.push(5, 5); // huff_lsbs 5 -> lsb_bits 3, shift = 3 + 2 - 1 = 4
        // book-1 code '100' = 0, lsbs 0b101 = 5: v = 5 + 0 - 16 = -11; +100 = 89; << 2 = 356
        // book-1 code '0001' = -2, lsbs 0: v = 0 - 16 - 16 = -32; +100 = 68; << 2 = 272
        for _ in 0..4 {
            w.push(3, 0b100);
            w.push(3, 0b101);
            w.push(4, 0b0001);
            w.push(3, 0);
        }
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let mut r = BitReader::new(&w.bytes);
        Block::parse(&mut r, &mut state, 0, &mut buf).unwrap();
        assert_eq!(state.substream[0].quant_step_size[0], 2);
        assert_eq!(state.substream[0].huff_offset[0], 100);
        for n in 0..8 {
            assert_eq!(
                buf.samples[n][0],
                if n % 2 == 0 { 356 } else { 272 },
                "sample {n}"
            );
        }
        assert_eq!(r.position(), w.len);
    }

    #[test]
    fn error_protection_reads_bit_count_and_crc() {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, true);
        push_plain_block_header(&mut w, 8, 0, 0, 3);
        w.push(16, 24); // block_data_bits: 8 samples x 3 bits
        for n in 0..8u64 {
            w.push(3, n);
        }
        w.push(8, 0x5C); // block_header_crc
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let mut r = BitReader::new(&w.bytes);
        let block = Block::parse(&mut r, &mut state, 0, &mut buf).unwrap();
        assert_eq!(block.block_data_bits, Some(24));
        assert!(block.block_data_bits_ok);
        assert_eq!(block.block_header_crc, Some(0x5C));
        assert_eq!(buf.samples[7][0], 3);

        // a wrong count is reported, not fatal
        let mut w = BitWriter::default();
        w.push(1, 0); // no block header: state persists
        w.push(16, 23);
        for n in 0..8u64 {
            w.push(3, n);
        }
        w.push(8, 0);
        let mut r = BitReader::new(&w.bytes);
        let block = Block::parse(&mut r, &mut state, 0, &mut buf).unwrap();
        assert!(!block.block_data_bits_ok);
        assert_eq!(buf.len, 16);
    }

    /// A CRC-8 of the bits `start..end` of `data` worked one bit at a time,
    /// from nothing but its arguments: the register shifts left, the bit
    /// that falls out decides the XOR with `poly`, then the data bit goes
    /// in. `msb_first` false feeds each byte from its lowest bit.
    fn crc8_bitwise(
        poly: u8,
        init: u8,
        data: &[u8],
        start: usize,
        end: usize,
        msb_first: bool,
    ) -> u8 {
        let mut crc = init;
        for pos in start..end {
            let byte = data[pos / 8];
            let k = pos % 8;
            let bit = if msb_first {
                (byte >> (7 - k)) & 1
            } else {
                (byte >> k) & 1
            };
            let out = crc & 0x80 != 0;
            crc <<= 1;
            if out {
                crc ^= poly;
            }
            crc ^= bit;
        }
        crc
    }

    /// A protected restart block (restart header, block header, samples)
    /// and after it a protected block with only a block header, the header
    /// of the second starting `pad` bits into its byte. Returns the data,
    /// where the second block starts and where its header starts and ends.
    fn two_protected_blocks(pad: usize) -> (Vec<u8>, usize, usize, usize) {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, true);
        push_plain_block_header(&mut w, 8, 0, 0, 3);
        w.push(16, 24);
        for n in 0..8u64 {
            w.push(3, n);
        }
        w.push(8, 0);
        // padding, so that the header of the second block lands `pad` bits
        // into a byte
        while (w.len + 2) % 8 != pad % 8 {
            w.push(1, 0);
        }
        let second = w.len;
        w.push(1, 1); // block_header_exists
        w.push(1, 0); // no restart header
        let start = w.len;
        push_plain_block_header(&mut w, 8, 0, 0, 4);
        let end = w.len;
        w.push(16, 32);
        for n in 0..8u64 {
            w.push(4, n);
        }
        w.push(8, 0);
        w.push(16, 0);
        (w.bytes, second, start, end)
    }

    /// Parses the first block, then the one at `second`.
    fn second_block(data: &[u8], second: usize) -> Block {
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let mut r = BitReader::new(data);
        Block::parse(&mut r, &mut state, 0, &mut buf).unwrap();
        r.seek(second).unwrap();
        Block::parse(&mut r, &mut state, 0, &mut buf).unwrap()
    }

    /// The polynomial alone: bytes that read the same from either end over a
    /// byte-aligned range, so the order the bits go in cannot matter, and
    /// the start of the register the module declares.
    #[test]
    fn the_header_crc_has_polynomial_0x1d() {
        let data = [0xA5, 0x3C, 0x81, 0x66, 0xFF, 0x18];
        let want = crc8_bitwise(0x1D, BLOCK_HEADER_CRC_INIT, &data, 0, 48, true);
        assert_ne!(
            want,
            crc8_bitwise(0x07, BLOCK_HEADER_CRC_INIT, &data, 0, 48, true),
            "the vector tells the polynomials apart"
        );
        assert_eq!(block_header_crc(&data, Some((0, 48))), want);
    }

    /// The start of the register alone: no bits go in without a header.
    #[test]
    fn the_header_crc_starts_at_0xa2_and_is_that_without_a_header() {
        assert_eq!(block_header_crc(&[], None), 0xA2);
        assert_eq!(block_header_crc(&[0x5A; 4], Some((7, 7))), 0xA2);
    }

    /// The order of the bits alone: a range that starts and ends inside a
    /// byte, over bytes that differ from their mirror image, with the
    /// polynomial and the start the module declares.
    #[test]
    fn the_header_crc_feeds_the_bits_most_significant_first() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let (start, end) = (3, 41);
        let poly = BLOCK_HEADER_CRC.poly();
        let want = crc8_bitwise(poly, BLOCK_HEADER_CRC_INIT, &data, start, end, true);
        assert_ne!(
            want,
            crc8_bitwise(poly, BLOCK_HEADER_CRC_INIT, &data, start, end, false),
            "the vector tells the orders apart"
        );
        assert_eq!(block_header_crc(&data, Some((start, end))), want);
    }

    /// The first bit of the range alone: the one after the two flags.
    #[test]
    fn the_header_crc_starts_after_the_block_and_restart_flags() {
        let (data, second, start, _) = two_protected_blocks(5);
        let block = second_block(&data, second);
        let (got_start, got_end) = block.header_bits.expect("a header");
        assert_eq!(got_start, start);
        assert_eq!(
            block.block_header_crc_expected,
            Some(block_header_crc(&data, Some((start, got_end))))
        );
    }

    /// The last bit of the range alone: the last of the header, before
    /// `block_data_bits`.
    #[test]
    fn the_header_crc_ends_before_block_data_bits() {
        let (data, second, _, end) = two_protected_blocks(5);
        let block = second_block(&data, second);
        let (got_start, got_end) = block.header_bits.expect("a header");
        assert_eq!(got_end, end);
        assert_eq!(
            block.block_header_crc_expected,
            Some(block_header_crc(&data, Some((got_start, end))))
        );
    }

    /// Byte-aligned and not: the check is the bitwise CRC of the header, and
    /// a block whose stated byte is that check is whole.
    #[test]
    fn aligned_and_unaligned_headers_are_checked() {
        for pad in [0, 1, 5, 7] {
            let (mut data, second, start, end) = two_protected_blocks(pad);
            let want = crc8_bitwise(0x1D, 0xA2, &data, start, end, true);
            let block = second_block(&data, second);
            assert_eq!(block.block_header_crc_expected, Some(want), "pad {pad}");
            assert_eq!(block.block_header_crc_ok(), want == 0, "pad {pad}");
            // write the check where the byte is and read the block again
            let at = end + 16 + 32;
            for k in 0..8 {
                let bit = (want >> (7 - k)) & 1;
                let pos = at + k;
                data[pos / 8] = (data[pos / 8] & !(0x80 >> (pos % 8))) | (bit << (7 - pos % 8));
            }
            let block = second_block(&data, second);
            assert_eq!(block.block_header_crc, Some(want), "pad {pad}");
            assert!(block.block_header_crc_ok(), "pad {pad}");
        }
    }

    /// A protected block without a header has the check 0xA2.
    #[test]
    fn a_protected_block_without_a_header_has_the_check_0xa2() {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, true);
        push_plain_block_header(&mut w, 8, 0, 0, 3);
        w.push(16, 24);
        for n in 0..8u64 {
            w.push(3, n);
        }
        w.push(8, 0);
        let second = w.len;
        w.push(1, 0); // no block header
        w.push(16, 24);
        for n in 0..8u64 {
            w.push(3, n);
        }
        w.push(8, 0xA2);
        w.push(16, 0);
        let block = second_block(&w.bytes, second);
        assert_eq!(block.header_bits, None);
        assert_eq!(block.block_header_crc_expected, Some(0xA2));
        assert!(block.block_header_crc_ok());
    }

    /// Without `error_protect` nothing is expected and nothing is wrong.
    #[test]
    fn an_unprotected_block_expects_no_check() {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, false);
        push_plain_block_header(&mut w, 8, 0, 0, 3);
        for n in 0..8u64 {
            w.push(3, n);
        }
        w.push(16, 0);
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let block = Block::parse(&mut BitReader::new(&w.bytes), &mut state, 0, &mut buf).unwrap();
        assert!(
            block.header_bits.is_some(),
            "where the header lies is kept either way"
        );
        assert_eq!(block.block_header_crc_expected, None);
        assert!(block.block_header_crc_ok());
    }

    #[test]
    fn blocks_need_a_restart_header_first() {
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        // block header without restart header
        let data = [0b1000_0000u8, 0, 0, 0];
        assert!(Block::parse(&mut BitReader::new(&data), &mut state, 0, &mut buf).is_err());
        // no block header at all
        let data = [0u8; 4];
        assert!(Block::parse(&mut BitReader::new(&data), &mut state, 0, &mut buf).is_err());
    }

    #[test]
    fn rejects_bad_block_sizes_and_out_of_range_samples() {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, false);
        push_plain_block_header(&mut w, 48, 0, 0, 3); // 48 > 40 samples per AU
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        assert!(Block::parse(&mut BitReader::new(&w.bytes), &mut state, 0, &mut buf).is_err());

        // huff_lsbs 24 with a huff_offset pushes plain samples beyond 24 bits
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, false);
        w.push(1, 0);
        w.push(1, 1);
        w.push(9, 8);
        w.push(1, 0);
        w.push(1, 0);
        w.push(1, 0);
        w.push(1, 1);
        w.push(1, 0);
        w.push(1, 0);
        w.push(1, 1);
        w.push(15, 0x3FFF); // huff_offset 16383
        w.push(2, 0);
        w.push(5, 24);
        for _ in 0..8 {
            w.push(24, 0xFF_FFFF); // 2^23 - 1 before the offset
        }
        let mut state = atmos_state();
        assert!(Block::parse(&mut BitReader::new(&w.bytes), &mut state, 0, &mut buf).is_err());
    }
}
