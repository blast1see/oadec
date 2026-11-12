//! Where a finding is.

/// Where a finding is.
///
/// A stream is framed in units, access units in a TrueHD stream and
/// syncframes in an AC-3 family stream, and a finding names the unit it is in
/// where it is in one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Location {
    /// Index of the unit, counted from the first one framed. For bytes the
    /// framing passed over, the unit it found sync again at.
    pub unit: Option<u64>,
    /// Byte offset in the input.
    pub offset: Option<u64>,
    /// Bit within that byte, 0 being the most significant, when the check
    /// knows the field it stopped at.
    pub bit: Option<u8>,
    /// The substream the finding belongs to, when it belongs to one.
    pub substream: Option<u8>,
}

impl Location {
    /// The start of unit `unit`, which begins at byte `offset`.
    #[must_use]
    pub const fn unit(unit: u64, offset: u64) -> Self {
        Self {
            unit: Some(unit),
            offset: Some(offset),
            bit: None,
            substream: None,
        }
    }

    /// A place in the input that belongs to no unit.
    #[must_use]
    pub const fn byte(offset: u64) -> Self {
        Self {
            unit: None,
            offset: Some(offset),
            bit: None,
            substream: None,
        }
    }

    /// The same place, in substream `substream`.
    #[must_use]
    pub const fn in_substream(mut self, substream: usize) -> Self {
        self.substream = Some(substream as u8);
        self
    }

    /// The same unit, at bit `bit_position` counted from byte `base`.
    #[must_use]
    pub const fn at_bit(mut self, base: u64, bit_position: usize) -> Self {
        self.offset = Some(base + (bit_position / 8) as u64);
        self.bit = Some((bit_position % 8) as u8);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bit_position_becomes_a_byte_and_a_bit() {
        let at = Location::unit(7, 1000).at_bit(1010, 19);
        assert_eq!(at.offset, Some(1012));
        assert_eq!(at.bit, Some(3));
        assert_eq!(at.unit, Some(7));
    }

    #[test]
    fn a_place_outside_every_unit_names_none() {
        let at = Location::byte(40).in_substream(2);
        assert_eq!(at.unit, None);
        assert_eq!(at.offset, Some(40));
        assert_eq!(at.substream, Some(2));
        assert_eq!(Location::default().offset, None);
    }
}
