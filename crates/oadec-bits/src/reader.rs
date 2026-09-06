//! MSB-first bit reader over a byte slice.
//!
//! Every read is bounds-checked and returns a [`BitError`] instead of panicking, so
//! the parsers built on top of it can reject malformed streams gracefully.

use core::fmt;

/// A read ran past the end of the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitError {
    /// Bit position at which the read was attempted.
    pub position: usize,
    /// Number of bits requested.
    pub requested: usize,
    /// Number of bits that were still available.
    pub available: usize,
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bit read out of bounds: {} bits requested at bit {} with {} available",
            self.requested, self.position, self.available
        )
    }
}

impl std::error::Error for BitError {}

/// MSB-first reader with a bit-granular position.
#[derive(Debug, Clone, Copy)]
pub struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    /// Creates a reader positioned at bit 0 of `data`.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// The underlying bytes.
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Current position in bits from the start of the data.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.pos
    }

    /// Total length of the data in bits.
    #[must_use]
    pub const fn len_bits(&self) -> usize {
        self.data.len() * 8
    }

    /// Bits left between the current position and the end of the data.
    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.len_bits() - self.pos
    }

    /// Whether the position is a multiple of `bits` (e.g. 8 or 16).
    #[must_use]
    pub const fn is_aligned(&self, bits: usize) -> bool {
        self.pos.is_multiple_of(bits)
    }

    const fn error(&self, requested: usize) -> BitError {
        BitError {
            position: self.pos,
            requested,
            available: self.remaining(),
        }
    }

    /// Moves to an absolute bit position (may equal the length).
    pub const fn seek(&mut self, pos: usize) -> Result<(), BitError> {
        if pos > self.len_bits() {
            return Err(BitError {
                position: self.pos,
                requested: pos,
                available: self.len_bits(),
            });
        }
        self.pos = pos;
        Ok(())
    }

    /// Advances by `n` bits.
    pub const fn skip(&mut self, n: usize) -> Result<(), BitError> {
        if n > self.remaining() {
            return Err(self.error(n));
        }
        self.pos += n;
        Ok(())
    }

    /// Advances to the next multiple of `bits` (no-op when already aligned).
    pub const fn align(&mut self, bits: usize) -> Result<(), BitError> {
        let rem = self.pos % bits;
        if rem == 0 {
            Ok(())
        } else {
            self.skip(bits - rem)
        }
    }

    /// Loads eight bytes starting at `byte_pos` as a big-endian word, zero-padded
    /// past the end of the data.
    #[inline]
    fn load64(&self, byte_pos: usize) -> u64 {
        if let Some(chunk) = self.data.get(byte_pos..byte_pos + 8) {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(chunk);
            return u64::from_be_bytes(buf);
        }
        let mut buf = [0u8; 8];
        if byte_pos < self.data.len() {
            let tail = &self.data[byte_pos..];
            buf[..tail.len()].copy_from_slice(tail);
        }
        u64::from_be_bytes(buf)
    }

    /// Returns the next `n` bits (`n <= 32`) without advancing.
    #[inline]
    pub fn peek(&self, n: u32) -> Result<u32, BitError> {
        debug_assert!(n <= 32, "peek of more than 32 bits");
        if n == 0 {
            return Ok(0);
        }
        if n as usize > self.remaining() {
            return Err(self.error(n as usize));
        }
        let word = self.load64(self.pos >> 3);
        let shift = 64 - (self.pos & 7) as u32 - n;
        Ok(((word >> shift) & ((1u64 << n) - 1)) as u32)
    }

    /// Reads `n` bits (`n <= 32`) as an unsigned value.
    #[inline]
    pub fn read(&mut self, n: u32) -> Result<u32, BitError> {
        let value = self.peek(n)?;
        self.pos += n as usize;
        Ok(value)
    }

    /// Reads `n` bits (`n <= 64`) as an unsigned value.
    pub fn read_u64(&mut self, n: u32) -> Result<u64, BitError> {
        debug_assert!(n <= 64, "read of more than 64 bits");
        if n as usize > self.remaining() {
            return Err(self.error(n as usize));
        }
        if n <= 32 {
            return self.read(n).map(u64::from);
        }
        let high = u64::from(self.read(n - 32)?);
        let low = u64::from(self.read(32)?);
        Ok((high << 32) | low)
    }

    /// Reads one bit as a flag.
    #[inline]
    pub fn read_bool(&mut self) -> Result<bool, BitError> {
        Ok(self.read(1)? != 0)
    }

    /// Reads `n` bits (`n <= 32`) as a two's-complement signed value.
    #[inline]
    pub fn read_signed(&mut self, n: u32) -> Result<i32, BitError> {
        if n == 0 {
            return Ok(0);
        }
        let value = self.read(n)?;
        let shift = 32 - n;
        Ok(((value << shift) as i32) >> shift)
    }

    /// Reads a `variable_bits_max(n, max_num_groups)` value (ETSI TS 103 420
    /// clause 5.5.1, ETSI TS 102 366 clause H.2.2.2.1).
    ///
    /// Each group carries `n` value bits followed by a continuation flag. When the
    /// flag is set the running value becomes `(value + 1) << n` before the next
    /// group is added. At most `max_num_groups` groups are read; the continuation
    /// flag of the last permitted group is read but not acted upon.
    pub fn read_variable_bits_max(&mut self, n: u32, max_num_groups: u32) -> Result<u32, BitError> {
        let mut value: u32 = 0;
        let mut groups = 0;
        loop {
            value = value.wrapping_add(self.read(n)?);
            let more = self.read_bool()?;
            groups += 1;
            if !more || groups >= max_num_groups {
                return Ok(value);
            }
            value = value.wrapping_add(1).wrapping_shl(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sequential_fields_msb_first() {
        let data = [0b1010_0101u8, 0xFF, 0x00];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read(3).unwrap(), 0b101);
        assert_eq!(r.read(5).unwrap(), 0b00101);
        assert_eq!(r.read(8).unwrap(), 0xFF);
        assert_eq!(r.position(), 16);
        assert_eq!(r.remaining(), 8);
        assert_eq!(r.read(0).unwrap(), 0);
        assert_eq!(r.position(), 16);
    }

    #[test]
    fn peek_does_not_advance() {
        let data = [0xA5u8];
        let r = BitReader::new(&data);
        assert_eq!(r.peek(4).unwrap(), 0xA);
        assert_eq!(r.peek(8).unwrap(), 0xA5);
        assert_eq!(r.position(), 0);
    }

    #[test]
    fn signed_reads_sign_extend() {
        let data = [0b1111_0111u8, 0b1000_0000];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_signed(4).unwrap(), -1);
        assert_eq!(r.read_signed(4).unwrap(), 7);
        assert_eq!(r.read_signed(1).unwrap(), -1);
        assert_eq!(r.read_signed(0).unwrap(), 0);
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let data = [0x00u8];
        let mut r = BitReader::new(&data);
        r.skip(6).unwrap();
        assert_eq!(
            r.read(3),
            Err(BitError {
                position: 6,
                requested: 3,
                available: 2
            })
        );
        assert_eq!(r.read(2).unwrap(), 0);
        assert!(r.read(1).is_err());
        assert!(r.skip(1).is_err());
        assert!(r.seek(9).is_err());
        r.seek(8).unwrap();
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn align_moves_to_the_next_boundary() {
        let data = [0u8; 4];
        let mut r = BitReader::new(&data);
        r.skip(3).unwrap();
        r.align(16).unwrap();
        assert_eq!(r.position(), 16);
        r.align(16).unwrap();
        assert_eq!(r.position(), 16);
        assert!(r.is_aligned(8));
    }

    #[test]
    fn unaligned_32_bit_read_spans_five_bytes() {
        let data = [0b0000_0001u8, 0x23, 0x45, 0x67, 0b1000_0000];
        let mut r = BitReader::new(&data);
        r.skip(7).unwrap();
        assert_eq!(r.read(32).unwrap(), 0x91A2_B3C0);
        assert_eq!(r.position(), 39);
    }

    #[test]
    fn read_u64_crosses_the_32_bit_boundary() {
        let data = [0x12u8, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_u64(40).unwrap(), 0x12_3456_789A);
        assert_eq!(r.read_u64(8).unwrap(), 0xBC);
        assert!(r.read_u64(1).is_err());
    }

    #[test]
    fn variable_bits_max_single_and_multiple_groups() {
        // 0101 0 -> 5
        let data = [0b0101_0000u8];
        assert_eq!(
            BitReader::new(&data).read_variable_bits_max(4, 4).unwrap(),
            5
        );
        // 1111 1 0010 0 -> ((15 + 1) << 4) + 2 = 258
        let data = [0b1111_1001u8, 0b0000_0000];
        let mut r = BitReader::new(&data);
        assert_eq!(r.read_variable_bits_max(4, 4).unwrap(), 258);
        assert_eq!(r.position(), 10);
    }

    #[test]
    fn variable_bits_max_stops_at_the_group_limit() {
        // Two groups allowed: 1111 1 | 1111 1 | (the second flag is read, not acted on)
        let data = [0b1111_1111u8, 0b1100_0000];
        let mut r = BitReader::new(&data);
        assert_eq!(
            r.read_variable_bits_max(4, 2).unwrap(),
            ((15 + 1) << 4) + 15
        );
        assert_eq!(r.position(), 10);
    }
}
