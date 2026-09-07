//! Substream segments: the blocks of one substream within one access unit, the
//! optional termination word and the optional parity/CRC trailer.

use oadec_bits::{BitReader, CRC8_SUBSTREAM, CRC8_SUBSTREAM_INIT, xor_bytes};

use crate::block::{Block, SampleBuffer};
use crate::error::{Error, Result};
use crate::state::{MAX_SAMPLES_PER_AU, ParserState};

/// First 18 bits of the termination word (`0xD234D234` as a whole).
pub const TERMINATOR: u32 = 0x348D3;

/// Value of the last 13 bits of a termination word that indicates no zero samples.
pub const TERMINATOR_TAIL: u16 = 0x1234;

/// Value XORed into the substream parity byte.
pub const PARITY_XOR: u8 = 0xA9;

/// Hard limit on blocks per segment (the smallest block is eight samples).
pub const MAX_BLOCKS: usize = MAX_SAMPLES_PER_AU / 8;

/// A parsed termination word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Terminator {
    /// `zero_samples_indicated`.
    pub zero_samples_indicated: bool,
    /// `zero_samples` (only meaningful when indicated): trailing samples of the
    /// access unit that are silence and may be dropped at the end of a stream.
    pub zero_samples: u16,
    /// Without `zero_samples_indicated` the last 13 bits must be `0x1234`.
    pub tail_ok: bool,
}

/// One parsed substream segment.
#[derive(Debug, Clone, Default)]
pub struct Segment {
    /// The blocks, in order.
    pub blocks: Vec<Block>,
    /// A restart header was present in one of the blocks.
    pub has_restart: bool,
    /// Samples coded by the blocks (should equal the samples per access unit).
    pub samples: usize,
    /// Whether `samples` equals the samples per access unit.
    pub sample_count_ok: bool,
    /// The termination word, when present.
    pub terminator: Option<Terminator>,
    /// There was room for a termination word but the bits did not spell one.
    pub unexpected_tail: bool,
    /// `substream_parity` as read (with `crc_present`).
    pub parity: Option<u8>,
    /// Whether the parity byte matched (true without `crc_present`).
    pub parity_ok: bool,
    /// `substream_crc` as read (with `crc_present`).
    pub crc: Option<u8>,
    /// Whether the CRC matched (true without `crc_present`).
    pub crc_ok: bool,
    /// Whether parsing ended exactly at the end pointer.
    pub end_ok: bool,
    /// Bits consumed, including the trailer.
    pub len_bits: usize,
}

impl Segment {
    /// Parses the segment in `bytes` (the byte range given by the directory) for
    /// `substream`, leaving the samples in `buf`.
    pub fn parse(
        bytes: &[u8],
        state: &mut ParserState,
        substream: usize,
        crc_present: bool,
        buf: &mut SampleBuffer,
    ) -> Result<Self> {
        Self::parse_with(bytes, state, substream, crc_present, buf, |_, _, _| Ok(()))
    }

    /// Like [`Segment::parse`], calling `on_block` after every block with the
    /// state and samples in force at that point (decoders recorrelate here).
    pub fn parse_with<F>(
        bytes: &[u8],
        state: &mut ParserState,
        substream: usize,
        crc_present: bool,
        buf: &mut SampleBuffer,
        mut on_block: F,
    ) -> Result<Self>
    where
        F: FnMut(&Block, &ParserState, &SampleBuffer) -> Result<()>,
    {
        let mut reader = BitReader::new(bytes);
        let expected_end = bytes.len() * 8;
        buf.clear();
        state.substream[substream].crc_present = crc_present;
        let mut seg = Self {
            blocks: Vec::with_capacity(4),
            ..Self::default()
        };

        loop {
            if seg.blocks.len() >= MAX_BLOCKS {
                return Err(Error::malformed(format!(
                    "substream {substream}: more than {MAX_BLOCKS} blocks in a segment"
                )));
            }
            let block = Block::parse(&mut reader, state, substream, buf)?;
            seg.has_restart |= block.restart.is_some();
            seg.samples += block.block_size;
            on_block(&block, state, buf)?;
            seg.blocks.push(block);
            if reader.read_bool()? {
                break; // last_block_in_segment
            }
        }
        seg.sample_count_ok = seg.samples == state.samples_per_au;
        reader.align(16)?;

        let trailer_bits = if crc_present { 16 } else { 0 };
        if expected_end >= reader.position() + 32 + trailer_bits {
            if reader.peek(18)? == TERMINATOR {
                reader.skip(18)?;
                let zero_samples_indicated = reader.read_bool()?;
                let tail = reader.read(13)? as u16;
                seg.terminator = Some(Terminator {
                    zero_samples_indicated,
                    zero_samples: if zero_samples_indicated { tail } else { 0 },
                    tail_ok: zero_samples_indicated || tail == TERMINATOR_TAIL,
                });
            } else {
                seg.unexpected_tail = true;
            }
        }

        seg.parity_ok = true;
        seg.crc_ok = true;
        if crc_present {
            let data_end = reader.position() / 8;
            let data = bytes
                .get(..data_end)
                .ok_or_else(|| Error::malformed("segment trailer beyond its end pointer"))?;
            let parity = reader.read(8)? as u8;
            let crc = reader.read(8)? as u8;
            seg.parity = Some(parity);
            seg.crc = Some(crc);
            seg.parity_ok = xor_bytes(data) ^ PARITY_XOR == parity;
            let computed = CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, data);
            seg.crc_ok = computed == crc;
        }
        seg.len_bits = reader.position();
        seg.end_ok = reader.position() == expected_end;
        Ok(seg)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::block::tests::push_plain_block_header;
    use crate::restart::tests::{atmos_state, push_restart_header};
    use crate::state::SYNC_A;
    use crate::testutil::BitWriter;

    /// A complete segment: restart header, one 40-sample plain block, optional
    /// termination word and, with `crc_present`, a correct parity/CRC trailer.
    pub(crate) fn build_segment(terminated: bool, crc_present: bool) -> Vec<u8> {
        let mut w = BitWriter::default();
        w.push(1, 1); // block_header_exists
        w.push(1, 1); // restart_header_exists
        push_restart_header(&mut w, SYNC_A, 0, 1, 1, false);
        push_plain_block_header(&mut w, 40, 0, 1, 4);
        for n in 0..40u64 {
            w.push(4, n & 15);
            w.push(4, 15 - (n & 15));
        }
        w.push(1, 1); // last_block_in_segment
        while !w.len.is_multiple_of(16) {
            w.push(1, 0);
        }
        if terminated {
            w.push(18, u64::from(TERMINATOR));
            w.push(1, 0);
            w.push(13, u64::from(TERMINATOR_TAIL));
        }
        if crc_present {
            let parity = xor_bytes(&w.bytes) ^ PARITY_XOR;
            let crc = CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, &w.bytes);
            w.push(8, u64::from(parity));
            w.push(8, u64::from(crc));
        }
        w.bytes
    }

    #[test]
    fn parses_a_terminated_segment_with_trailer() {
        let bytes = build_segment(true, true);
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let seg = Segment::parse(&bytes, &mut state, 0, true, &mut buf).unwrap();
        assert_eq!(seg.blocks.len(), 1);
        assert!(seg.has_restart);
        assert_eq!(seg.samples, 40);
        assert!(seg.sample_count_ok);
        let t = seg.terminator.unwrap();
        assert!(!t.zero_samples_indicated && t.tail_ok);
        assert!(seg.parity_ok && seg.crc_ok && seg.end_ok && !seg.unexpected_tail);
        assert_eq!(seg.len_bits, bytes.len() * 8);
        assert_eq!(buf.len, 40);
        assert_eq!(buf.samples[3][0], 3 - 8);
        assert_eq!(buf.samples[3][1], 12 - 8);
    }

    #[test]
    fn parses_segments_without_terminator_or_trailer() {
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let bytes = build_segment(false, false);
        let seg = Segment::parse(&bytes, &mut state, 0, false, &mut buf).unwrap();
        assert!(seg.terminator.is_none() && seg.parity.is_none() && seg.end_ok);
        let bytes = build_segment(false, true);
        let seg = Segment::parse(&bytes, &mut state, 0, true, &mut buf).unwrap();
        assert!(seg.terminator.is_none() && seg.parity_ok && seg.crc_ok && seg.end_ok);
    }

    #[test]
    fn detects_corrupted_trailers_and_wrong_end_pointers() {
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let mut bytes = build_segment(true, true);
        let last = bytes.len() - 1;
        bytes[last] ^= 0x10; // CRC byte
        let seg = Segment::parse(&bytes, &mut state, 0, true, &mut buf).unwrap();
        assert!(seg.parity_ok && !seg.crc_ok);
        bytes[last - 1] ^= 0x01; // parity byte
        let seg = Segment::parse(&bytes, &mut state, 0, true, &mut buf).unwrap();
        assert!(!seg.parity_ok);

        // two extra words before the trailer: the terminator is not where expected
        let good = build_segment(true, true);
        let mut padded = good[..good.len() - 2].to_vec();
        padded.extend_from_slice(&[0, 0, 0, 0]);
        padded.extend_from_slice(&good[good.len() - 2..]);
        let seg = Segment::parse(&padded, &mut state, 0, true, &mut buf).unwrap();
        assert!(seg.unexpected_tail || !seg.end_ok || !seg.crc_ok);

        // end pointer beyond the data of an unprotected segment
        let mut longer = build_segment(false, false);
        longer.extend_from_slice(&[0, 0]);
        let seg = Segment::parse(&longer, &mut state, 0, false, &mut buf).unwrap();
        assert!(!seg.end_ok);
    }

    #[test]
    fn zero_samples_are_reported() {
        let mut w = BitWriter::default();
        w.push(1, 1);
        w.push(1, 1);
        push_restart_header(&mut w, SYNC_A, 0, 0, 0, false);
        push_plain_block_header(&mut w, 40, 0, 0, 1);
        for _ in 0..40 {
            w.push(1, 1);
        }
        w.push(1, 1);
        while !w.len.is_multiple_of(16) {
            w.push(1, 0);
        }
        w.push(18, u64::from(TERMINATOR));
        w.push(1, 1);
        w.push(13, 24);
        let mut state = atmos_state();
        let mut buf = SampleBuffer::default();
        let seg = Segment::parse(&w.bytes, &mut state, 0, false, &mut buf).unwrap();
        let t = seg.terminator.unwrap();
        assert!(t.zero_samples_indicated);
        assert_eq!(t.zero_samples, 24);
        assert!(seg.end_ok);
    }
}
