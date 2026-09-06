//! CRC-8 and CRC-16 in the convention used by the TrueHD bitstream.
//!
//! For every data bit (most significant bit first) the register is shifted left,
//! reduced by the polynomial when the bit that fell out was set, and then the data
//! bit is XORed into the least significant position. Processing eight bits this way
//! equals shifting the register eight times and XORing the whole byte, which is what
//! the byte tables implement. This differs from the textbook CRC (where the data byte
//! is XORed *before* shifting) only by where the data enters the register.

/// Returns bit `pos` (MSB-first numbering) of `data`.
#[inline]
const fn bit_at(data: &[u8], pos: usize) -> bool {
    (data[pos / 8] >> (7 - (pos % 8))) & 1 != 0
}

/// Table-driven CRC-8 with a run-time polynomial.
#[derive(Debug, Clone, Copy)]
pub struct Crc8 {
    poly: u8,
    table: [u8; 256],
}

impl Crc8 {
    /// Builds the byte table for `poly` (the low eight coefficients of the generator).
    #[must_use]
    pub const fn new(poly: u8) -> Self {
        let mut table = [0u8; 256];
        let mut i = 0;
        while i < 256 {
            table[i] = Self::shift_only(poly, i as u8, 8);
            i += 1;
        }
        Self { poly, table }
    }

    const fn shift_only(poly: u8, mut value: u8, steps: usize) -> u8 {
        let mut i = 0;
        while i < steps {
            let msb = value >> 7;
            value <<= 1;
            if msb != 0 {
                value ^= poly;
            }
            i += 1;
        }
        value
    }

    /// The generator polynomial (without the implicit x^8 term).
    #[must_use]
    pub const fn poly(&self) -> u8 {
        self.poly
    }

    /// Advances the register by one data bit.
    #[inline]
    #[must_use]
    pub const fn step_bit(&self, crc: u8, bit: bool) -> u8 {
        Self::shift_only(self.poly, crc, 1) ^ (bit as u8)
    }

    /// Advances the register by one data byte.
    #[inline]
    #[must_use]
    pub const fn update_byte(&self, crc: u8, byte: u8) -> u8 {
        self.table[crc as usize] ^ byte
    }

    /// Advances the register over `bytes`.
    #[must_use]
    pub fn update_bytes(&self, mut crc: u8, bytes: &[u8]) -> u8 {
        for &b in bytes {
            crc = self.update_byte(crc, b);
        }
        crc
    }

    /// Advances the register over `len_bits` bits of `data` starting at `start_bit`
    /// (any alignment).
    ///
    /// # Panics
    /// Panics if the bit range lies outside `data`.
    #[must_use]
    pub fn update_bits(&self, mut crc: u8, data: &[u8], start_bit: usize, len_bits: usize) -> u8 {
        let end = start_bit + len_bits;
        assert!(end <= data.len() * 8, "bit range outside the data");
        let mut pos = start_bit;
        while pos < end && !pos.is_multiple_of(8) {
            crc = self.step_bit(crc, bit_at(data, pos));
            pos += 1;
        }
        while pos + 8 <= end {
            crc = self.update_byte(crc, data[pos / 8]);
            pos += 8;
        }
        while pos < end {
            crc = self.step_bit(crc, bit_at(data, pos));
            pos += 1;
        }
        crc
    }
}

/// Table-driven CRC-16 with a run-time polynomial.
#[derive(Debug, Clone, Copy)]
pub struct Crc16 {
    poly: u16,
    table: [u16; 256],
}

impl Crc16 {
    /// Builds the byte table for `poly` (the low sixteen coefficients of the generator).
    #[must_use]
    pub const fn new(poly: u16) -> Self {
        let mut table = [0u16; 256];
        let mut i = 0;
        while i < 256 {
            table[i] = Self::shift_only(poly, (i as u16) << 8, 8);
            i += 1;
        }
        Self { poly, table }
    }

    const fn shift_only(poly: u16, mut value: u16, steps: usize) -> u16 {
        let mut i = 0;
        while i < steps {
            let msb = value >> 15;
            value <<= 1;
            if msb != 0 {
                value ^= poly;
            }
            i += 1;
        }
        value
    }

    /// The generator polynomial (without the implicit x^16 term).
    #[must_use]
    pub const fn poly(&self) -> u16 {
        self.poly
    }

    /// Advances the register by one data bit.
    #[inline]
    #[must_use]
    pub const fn step_bit(&self, crc: u16, bit: bool) -> u16 {
        Self::shift_only(self.poly, crc, 1) ^ (bit as u16)
    }

    /// Advances the register by one data byte.
    #[inline]
    #[must_use]
    pub const fn update_byte(&self, crc: u16, byte: u8) -> u16 {
        self.table[(crc >> 8) as usize] ^ (crc << 8) ^ (byte as u16)
    }

    /// Advances the register over `bytes`.
    #[must_use]
    pub fn update_bytes(&self, mut crc: u16, bytes: &[u8]) -> u16 {
        for &b in bytes {
            crc = self.update_byte(crc, b);
        }
        crc
    }

    /// Advances the register over `len_bits` bits of `data` starting at `start_bit`.
    ///
    /// # Panics
    /// Panics if the bit range lies outside `data`.
    #[must_use]
    pub fn update_bits(&self, mut crc: u16, data: &[u8], start_bit: usize, len_bits: usize) -> u16 {
        let end = start_bit + len_bits;
        assert!(end <= data.len() * 8, "bit range outside the data");
        let mut pos = start_bit;
        while pos < end && !pos.is_multiple_of(8) {
            crc = self.step_bit(crc, bit_at(data, pos));
            pos += 1;
        }
        while pos + 8 <= end {
            crc = self.update_byte(crc, data[pos / 8]);
            pos += 8;
        }
        while pos < end {
            crc = self.step_bit(crc, bit_at(data, pos));
            pos += 1;
        }
        crc
    }
}

/// CRC-8 of the TrueHD restart header (generator x^8 + x^4 + x^3 + x^2 + 1), initial
/// register 0, covering the header from the restart sync word up to the CRC field.
pub const CRC8_RESTART: Crc8 = Crc8::new(0x1D);

/// CRC-8 of a TrueHD substream segment (generator x^8 + x^6 + x^5 + x + 1).
pub const CRC8_SUBSTREAM: Crc8 = Crc8::new(0x63);

/// Initial register for [`CRC8_SUBSTREAM`] when the whole segment (all bytes before
/// the parity and CRC bytes) is fed through [`Crc8::update_bytes`].
pub const CRC8_SUBSTREAM_INIT: u8 = 0xA2;

/// CRC-16 of the TrueHD major sync information (generator x^16 + x^5 + x^3 + x^2 + 1),
/// initial register 0.
pub const CRC16_MAJOR_SYNC: Crc16 = Crc16::new(0x002D);

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random bytes for the equivalence tests.
    fn noise(seed: u32, len: usize) -> Vec<u8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 24) as u8
            })
            .collect()
    }

    /// Textbook (xor-then-shift) CRC-8, as computed by a generic library.
    fn textbook_crc8(poly: u8, init: u8, bytes: &[u8]) -> u8 {
        let mut crc = init;
        for &b in bytes {
            crc ^= b;
            for _ in 0..8 {
                let msb = crc & 0x80 != 0;
                crc <<= 1;
                if msb {
                    crc ^= poly;
                }
            }
        }
        crc
    }

    #[test]
    fn bit_update_matches_byte_update_on_aligned_ranges() {
        let data = noise(1, 64);
        for len in [0usize, 1, 7, 8, 9, 40, 64] {
            let a = CRC8_SUBSTREAM.update_bytes(0xA2, &data[..len]);
            let b = CRC8_SUBSTREAM.update_bits(0xA2, &data, 0, len * 8);
            assert_eq!(a, b, "len {len}");
            let a16 = CRC16_MAJOR_SYNC.update_bytes(0, &data[..len]);
            let b16 = CRC16_MAJOR_SYNC.update_bits(0, &data, 0, len * 8);
            assert_eq!(a16, b16, "len {len}");
        }
    }

    #[test]
    fn substream_crc_equals_the_textbook_formulation() {
        // The other public decoder computes a textbook CRC with initial value 0x3C
        // over all bytes but the last and XORs the last byte in raw. Feeding every
        // byte through the shift-then-xor register with initial value 0xA2 must give
        // the same result: 0x3C is 0xA2 shifted through eight steps.
        assert_eq!(Crc8::shift_only(0x63, 0xA2, 8), 0x3C);
        for seed in 1..40u32 {
            let data = noise(seed, 3 + seed as usize);
            let (body, last) = data.split_at(data.len() - 1);
            let reference = textbook_crc8(0x63, 0x3C, body) ^ last[0];
            assert_eq!(
                CRC8_SUBSTREAM.update_bytes(CRC8_SUBSTREAM_INIT, &data),
                reference
            );
        }
    }

    /// Port of the restart-header checksum used by the other public decoder: the
    /// first byte enters with its two flag bits masked, whole bytes follow, the last
    /// full byte is XORed raw and the remaining bits are shifted in one by one.
    fn reference_restart_checksum(buf: &[u8], bit_size: usize) -> u8 {
        let num_bytes = (bit_size + 2) / 8;
        let mut crc = textbook_crc8(0x1D, buf[0] & 0xC0, &buf[..num_bytes - 1]);
        crc ^= buf[num_bytes - 1];
        for i in 0..((bit_size + 2) & 7) {
            let msb = crc & 0x80 != 0;
            crc <<= 1;
            if msb {
                crc ^= 0x1D;
            }
            crc ^= (buf[num_bytes] >> (7 - i)) & 1;
        }
        crc
    }

    #[test]
    fn restart_crc_over_a_bit_range_matches_the_reference() {
        for seed in 1..40u32 {
            let data = noise(seed, 32);
            for bit_size in [70usize, 88, 93, 100, 121] {
                let reference = reference_restart_checksum(&data, bit_size);
                let ours = CRC8_RESTART.update_bits(0, &data, 2, bit_size);
                assert_eq!(ours, reference, "seed {seed} bits {bit_size}");
            }
        }
    }

    #[test]
    fn crc16_table_update_equals_eight_bit_steps() {
        for byte in [0u8, 1, 0x80, 0xFF, 0x5A] {
            for init in [0u16, 1, 0x8000, 0xBEEF] {
                let mut crc = init;
                for i in 0..8 {
                    crc = CRC16_MAJOR_SYNC.step_bit(crc, (byte >> (7 - i)) & 1 != 0);
                }
                assert_eq!(crc, CRC16_MAJOR_SYNC.update_byte(init, byte));
            }
        }
    }

    #[test]
    fn crc8_table_update_equals_eight_bit_steps() {
        for byte in [0u8, 1, 0x80, 0xFF, 0x5A] {
            for init in [0u8, 1, 0x80, 0xA2] {
                let mut crc = init;
                for i in 0..8 {
                    crc = CRC8_RESTART.step_bit(crc, (byte >> (7 - i)) & 1 != 0);
                }
                assert_eq!(crc, CRC8_RESTART.update_byte(init, byte));
            }
        }
    }
}
