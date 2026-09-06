//! XOR parity helpers used by the TrueHD access-unit and substream checks.

/// XOR of all bytes.
#[must_use]
pub fn xor_bytes(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |acc, &b| acc ^ b)
}

/// Folds a byte to a nibble: `(p ^ (p >> 4)) & 0xF`.
#[must_use]
pub const fn fold_nibble(parity: u8) -> u8 {
    (parity ^ (parity >> 4)) & 0xF
}

/// Folds a 32-bit word to a byte by XORing its four bytes together.
#[must_use]
pub const fn fold_u32(word: u32) -> u8 {
    let x = word ^ (word >> 16);
    let x = x ^ (x >> 8);
    (x & 0xFF) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_and_folds() {
        assert_eq!(xor_bytes(&[1, 2, 4]), 7);
        assert_eq!(xor_bytes(&[]), 0);
        assert_eq!(fold_nibble(0xF0), 0xF);
        assert_eq!(fold_nibble(0xFF), 0x0);
        assert_eq!(fold_u32(0x0100_0100), 0);
        assert_eq!(fold_u32(0x8000_0001), 0x81);
    }
}
