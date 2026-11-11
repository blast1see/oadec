//! A bit writer: most significant bit first, as every field of the format is
//! written.
//!
//! `oadec_bits::BitWriter` is not used here, on purpose. It masks a value
//! that is too wide for its field, which suits writing back what a parser
//! read; a recipe whose field lost its top bits would be a stream that says
//! something other than the recipe, so this one refuses. It also lends out
//! the bytes written so far, which the CRC of a restart header is computed
//! over while the block is still being written.

/// Bits appended to a growing buffer.
#[derive(Debug, Clone, Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    /// An empty writer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bits written so far.
    #[must_use]
    pub fn len_bits(&self) -> usize {
        self.bits
    }

    /// Writes the low `n` bits of `value`, the most significant of them first.
    ///
    /// # Panics
    /// Panics if `n` is over 64 or `value` does not fit in `n` bits: a field
    /// that silently lost its top bits would be a stream that says something
    /// other than the recipe.
    pub fn put(&mut self, n: u32, value: u64) {
        assert!(n <= 64, "a field of {n} bits");
        assert!(
            n == 64 || value >> n == 0,
            "{value:#x} does not fit in {n} bits"
        );
        for i in (0..n).rev() {
            if self.bits.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }

    /// Writes `value` in `n` bits of two's complement.
    ///
    /// # Panics
    /// Panics if `value` is outside what `n` bits of two's complement hold.
    pub fn put_signed(&mut self, n: u32, value: i64) {
        assert!((1..=64).contains(&n), "a signed field of {n} bits");
        if n < 64 {
            let half = 1i64 << (n - 1);
            assert!(
                (-half..half).contains(&value),
                "{value} does not fit in {n} signed bits"
            );
        }
        let mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
        self.put(n, value as u64 & mask);
    }

    /// Writes one bit.
    pub fn put_bool(&mut self, bit: bool) {
        self.put(1, u64::from(bit));
    }

    /// Writes whole bytes; the writer need not be on a byte.
    pub fn put_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.put(8, u64::from(byte));
        }
    }

    /// Writes every bit of `other` after the bits written so far.
    pub fn append(&mut self, other: &Self) {
        for i in 0..other.bits {
            self.put(1, u64::from(other.bytes[i / 8] >> (7 - i % 8) & 1));
        }
    }

    /// Writes zero bits up to the next multiple of `to` bits.
    pub fn align(&mut self, to: usize) {
        while !self.bits.is_multiple_of(to) {
            self.put(1, 0);
        }
    }

    /// The bytes written, the last one padded with zero bits.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The bytes written, the last one padded with zero bits.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_written_most_significant_bit_first() {
        let mut w = BitWriter::new();
        w.put(4, 0xF);
        w.put(12, 0x074);
        w.put_signed(4, -1);
        w.put_signed(4, 7);
        w.put_bool(true);
        assert_eq!(w.len_bits(), 25);
        w.align(16);
        assert_eq!(w.len_bits(), 32);
        assert_eq!(w.bytes(), [0xF0, 0x74, 0xF7, 0x80]);
        w.put_bytes(&[0xAB]);
        assert_eq!(w.into_bytes(), [0xF0, 0x74, 0xF7, 0x80, 0xAB]);
    }

    #[test]
    fn bits_are_appended_where_the_writer_stands() {
        let mut tail = BitWriter::new();
        tail.put(11, 0b101_1100_0111);
        for lead in 0..9 {
            let mut w = BitWriter::new();
            w.put(lead, 0);
            w.append(&tail);
            let mut direct = BitWriter::new();
            direct.put(lead, 0);
            direct.put(11, 0b101_1100_0111);
            assert_eq!(w.len_bits(), direct.len_bits(), "{lead}");
            assert_eq!(w.bytes(), direct.bytes(), "{lead}");
        }
    }

    #[test]
    fn a_signed_field_holds_its_whole_range() {
        let mut w = BitWriter::new();
        w.put_signed(15, -16384);
        w.put_signed(15, 16383);
        w.align(8);
        // 100 0000 0000 0000 | 011 1111 1111 1111 | 00
        assert_eq!(w.bytes(), [0x80, 0x00, 0xFF, 0xFC]);
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn a_value_too_wide_for_its_field_is_refused() {
        BitWriter::new().put(4, 16);
    }

    #[test]
    #[should_panic(expected = "does not fit")]
    fn a_signed_value_outside_its_field_is_refused() {
        BitWriter::new().put_signed(4, 8);
    }
}
