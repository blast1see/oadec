//! Prediction filter coefficients of a channel.

use oadec_bits::BitReader;

use crate::error::{Error, Result};

/// Which of the two filters a coefficient set belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// Filter A: FIR, up to eight taps, no coded state.
    Fir,
    /// Filter B: IIR, up to four taps, may carry a coded state.
    Iir,
}

/// A coded set of filter coefficients.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FilterCoeffs {
    /// Filter order (number of taps).
    pub order: u8,
    /// Coefficient precision (`coeff_q`, fractional bits, 8..=15).
    pub coeff_q: u8,
    /// Coded bits per coefficient.
    pub coeff_bits: u8,
    /// Left shift applied to every coefficient.
    pub coeff_shift: u8,
    /// Coefficients, already shifted.
    pub coeff: [i32; 8],
    /// A state was coded (IIR only).
    pub new_states: bool,
    /// State values, already shifted.
    pub state: [i32; 8],
}

impl FilterCoeffs {
    /// Parses a coefficient set.
    pub fn parse(reader: &mut BitReader<'_>, kind: FilterKind) -> Result<Self> {
        let mut fc = Self {
            order: reader.read(4)? as u8,
            ..Self::default()
        };
        let max_order = match kind {
            FilterKind::Fir => 8,
            FilterKind::Iir => 4,
        };
        if fc.order > max_order {
            return Err(Error::malformed(format!(
                "filter order {} exceeds {max_order}",
                fc.order
            )));
        }
        if fc.order == 0 {
            return Ok(fc);
        }
        fc.coeff_q = reader.read(4)? as u8;
        if fc.coeff_q < 8 {
            return Err(Error::malformed(format!(
                "filter coeff_q {} below 8",
                fc.coeff_q
            )));
        }
        fc.coeff_bits = reader.read(5)? as u8;
        if fc.coeff_bits == 0 || fc.coeff_bits > 16 {
            return Err(Error::malformed(format!(
                "filter coeff_bits {} outside 1..=16",
                fc.coeff_bits
            )));
        }
        fc.coeff_shift = reader.read(3)? as u8;
        if fc.coeff_bits + fc.coeff_shift > 16 {
            return Err(Error::malformed(
                "filter coefficient bits plus shift exceed 16",
            ));
        }
        for i in 0..usize::from(fc.order) {
            let coeff = reader.read_signed(u32::from(fc.coeff_bits))? << fc.coeff_shift;
            if coeff == -32768 {
                return Err(Error::malformed("filter coefficient -32768 is not allowed"));
            }
            fc.coeff[i] = coeff;
        }
        fc.new_states = reader.read_bool()?;
        if fc.new_states {
            if kind == FilterKind::Fir {
                return Err(Error::malformed("FIR filter carries state data"));
            }
            let state_bits = reader.read(4)?;
            let state_shift = reader.read(4)?;
            for i in 0..usize::from(fc.order) {
                let state = if state_bits == 0 {
                    0
                } else {
                    reader.read_signed(state_bits)? << state_shift
                };
                if !(-(1 << 23)..(1 << 23)).contains(&state) {
                    return Err(Error::malformed("filter state outside the 24-bit range"));
                }
                fc.state[i] = state;
            }
        }
        Ok(fc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::BitWriter;

    #[test]
    fn parses_fir_and_iir_sets() {
        let mut w = BitWriter::default();
        w.push(4, 2); // order
        w.push(4, 12); // coeff_q
        w.push(5, 8); // coeff_bits
        w.push(3, 2); // coeff_shift
        w.push(8, 0x7F); // 127 << 2
        w.push(8, 0xFF); // -1 << 2
        w.push(1, 0); // no states
        let mut r = BitReader::new(&w.bytes);
        let fir = FilterCoeffs::parse(&mut r, FilterKind::Fir).unwrap();
        assert_eq!(fir.order, 2);
        assert_eq!(fir.coeff_q, 12);
        assert_eq!(&fir.coeff[..2], &[508, -4]);
        assert!(!fir.new_states);

        let mut w = BitWriter::default();
        w.push(4, 1);
        w.push(4, 10);
        w.push(5, 4);
        w.push(3, 0);
        w.push(4, 0b0101); // 5
        w.push(1, 1); // states
        w.push(4, 3); // state_bits
        w.push(4, 4); // state_shift
        w.push(3, 0b110); // -2 << 4 = -32
        let mut r = BitReader::new(&w.bytes);
        let iir = FilterCoeffs::parse(&mut r, FilterKind::Iir).unwrap();
        assert_eq!(iir.order, 1);
        assert_eq!(iir.coeff[0], 5);
        assert!(iir.new_states);
        assert_eq!(iir.state[0], -32);

        // an FIR set with state data is malformed, as is an IIR order above 4
        let mut w = BitWriter::default();
        w.push(4, 1);
        w.push(4, 10);
        w.push(5, 4);
        w.push(3, 0);
        w.push(4, 1);
        w.push(1, 1);
        w.push(8, 0);
        assert!(FilterCoeffs::parse(&mut BitReader::new(&w.bytes), FilterKind::Fir).is_err());
        let data = [0b0101_0000u8, 0, 0];
        assert!(FilterCoeffs::parse(&mut BitReader::new(&data), FilterKind::Iir).is_err());
        let zero = [0u8];
        assert_eq!(
            FilterCoeffs::parse(&mut BitReader::new(&zero), FilterKind::Fir)
                .unwrap()
                .order,
            0
        );
    }
}
