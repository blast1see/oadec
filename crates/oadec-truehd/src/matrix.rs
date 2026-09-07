//! Primitive matrix parameters of a block header.
//!
//! Sync words A and B send the whole set of matrices whenever `new_matrixing` is
//! set. Sync word C (the object presentation) separates the matrix *configuration*
//! (which channels, precisions, masks) from the coefficients, and adds delta
//! coefficients that are interpolated across the access unit.
//!
//! The coefficients themselves land in the [`SubstreamState`] (they persist until
//! the next matrixing block); the returned [`Matrixing`] only records which parts
//! of the syntax were present.

use oadec_bits::BitReader;

use crate::error::{Error, Result};
use crate::state::{SYNC_A, SYNC_B, SYNC_C, SubstreamState};

/// Which parts of `matrixing()` were present in a block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Matrixing {
    /// New coefficients were sent (`new_matrix`; always true for sync words A and B).
    pub new_matrix: bool,
    /// A new configuration was sent (sync word C only; always true for A and B).
    pub new_matrix_config: bool,
    /// `interpolation_used` (sync word C).
    pub interpolation_used: bool,
    /// `new_delta` (sync word C).
    pub new_delta: bool,
    /// `new_delta_config` (sync word C).
    pub new_delta_config: bool,
}

impl Matrixing {
    /// Parses `matrixing()` and updates the persistent matrix parameters of `ss`.
    pub fn parse(reader: &mut BitReader<'_>, ss: &mut SubstreamState) -> Result<Self> {
        let max_matrix_chan = ss.max_matrix_chan;
        let mut m = Self::default();

        match ss.sync_word {
            SYNC_A | SYNC_B => {
                m.new_matrix = true;
                m.new_matrix_config = true;
                ss.primitive_matrices = reader.read(4)? as usize;
                let extra = if ss.sync_word == SYNC_A { 2 } else { 0 };
                for pmi in 0..ss.primitive_matrices {
                    ss.matrix_ch[pmi] = reader.read(4)? as u8;
                    ss.frac_bits[pmi] = reader.read(4)? as u8;
                    ss.lsb_bypass_used[pmi] = reader.read_bool()?;
                    check_matrix(pmi, ss.matrix_ch[pmi], ss.frac_bits[pmi], max_matrix_chan)?;
                    let coeff_bits = u32::from(ss.frac_bits[pmi]) + 2;
                    for ch in 0..=max_matrix_chan + extra {
                        ss.matrix_coeff[pmi][ch] = if reader.read_bool()? {
                            reader.read_signed(coeff_bits)?
                        } else {
                            0
                        };
                    }
                    if ss.sync_word == SYNC_B {
                        ss.dither_scale[pmi] = reader.read(4)? as u8;
                    }
                }
            }
            SYNC_C => {
                m.new_matrix = reader.read_bool()?;
                if m.new_matrix {
                    m.new_matrix_config = reader.read_bool()?;
                    if m.new_matrix_config {
                        ss.primitive_matrices = reader.read(4)? as usize + 1;
                        for pmi in 0..ss.primitive_matrices {
                            ss.matrix_ch[pmi] = reader.read(4)? as u8;
                            ss.frac_bits[pmi] = reader.read(4)? as u8;
                            ss.cf_shift_code[pmi] = reader.read(3)? as i8 - 1;
                            ss.lsb_bypass_bit_count[pmi] = reader.read(2)? as u8;
                            ss.dither_scale[pmi] = reader.read(4)? as u8;
                            ss.cf_mask[pmi] = reader.read(max_matrix_chan as u32 + 1)? as u16;
                            check_matrix(
                                pmi,
                                ss.matrix_ch[pmi],
                                ss.frac_bits[pmi],
                                max_matrix_chan,
                            )?;
                        }
                    }
                    for pmi in 0..ss.primitive_matrices {
                        let coeff_bits = u32::from(ss.frac_bits[pmi]) + 2;
                        for ch in 0..=max_matrix_chan {
                            ss.matrix_coeff[pmi][ch] = if (ss.cf_mask[pmi] >> ch) & 1 != 0 {
                                reader.read_signed(coeff_bits)?
                            } else {
                                0
                            };
                        }
                    }
                }
                m.interpolation_used = reader.read_bool()?;
                ss.interpolation_used = m.interpolation_used;
                if m.interpolation_used {
                    m.new_delta = reader.read_bool()?;
                    if m.new_delta {
                        m.new_delta_config = reader.read_bool()?;
                        if m.new_delta_config {
                            for pmi in 0..ss.primitive_matrices {
                                ss.delta_bits[pmi] = reader.read(4)? as u8;
                                ss.delta_precision[pmi] = reader.read(2)? as u8;
                            }
                        }
                        for pmi in 0..ss.primitive_matrices {
                            let delta_bits = u32::from(ss.delta_bits[pmi]);
                            for ch in 0..=max_matrix_chan {
                                ss.delta_cf[pmi][ch] =
                                    if delta_bits != 0 && (ss.cf_mask[pmi] >> ch) & 1 != 0 {
                                        reader.read_signed(delta_bits + 1)?
                                    } else {
                                        0
                                    };
                            }
                        }
                    }
                } else {
                    // Without interpolation the deltas are not applied; forget them so a
                    // later `interpolation_used` without `new_delta` starts from zero.
                    ss.delta_cf = [[0; crate::state::MAX_CHANNELS]; crate::state::MAX_MATRICES];
                }
            }
            other => {
                return Err(Error::malformed(format!(
                    "matrixing before a restart header (sync word {other:#06X})"
                )));
            }
        }
        Ok(m)
    }
}

fn check_matrix(pmi: usize, matrix_ch: u8, frac_bits: u8, max_matrix_chan: usize) -> Result<()> {
    if usize::from(matrix_ch) > max_matrix_chan {
        return Err(Error::malformed(format!(
            "matrix {pmi} writes channel {matrix_ch} beyond max_matrix_chan {max_matrix_chan}"
        )));
    }
    if frac_bits > 14 {
        return Err(Error::malformed(format!(
            "matrix {pmi} frac_bits {frac_bits} exceeds 14"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::BitWriter;

    fn state(sync_word: u16, max_matrix_chan: usize) -> SubstreamState {
        SubstreamState {
            sync_word,
            max_matrix_chan,
            restart_seen: true,
            ..SubstreamState::default()
        }
    }

    #[test]
    fn sync_a_reads_two_noise_columns_and_sync_b_reads_dither_scale() {
        let mut w = BitWriter::default();
        w.push(4, 1); // one matrix
        w.push(4, 1); // matrix_ch
        w.push(4, 2); // frac_bits -> 4-bit coefficients
        w.push(1, 1); // lsb_bypass_used
        // channels 0..=1 plus two noise columns
        for v in [0b1_0011u64, 0b0, 0b1_1110, 0b1_0001] {
            w.push(if v == 0 { 1 } else { 5 }, v);
        }
        let mut ss = state(SYNC_A, 1);
        let m = Matrixing::parse(&mut BitReader::new(&w.bytes), &mut ss).unwrap();
        assert!(m.new_matrix && m.new_matrix_config);
        assert_eq!(ss.primitive_matrices, 1);
        assert_eq!(ss.matrix_ch[0], 1);
        assert!(ss.lsb_bypass_used[0]);
        assert_eq!(&ss.matrix_coeff[0][..4], &[3, 0, -2, 1]);

        let mut w = BitWriter::default();
        w.push(4, 1);
        w.push(4, 0);
        w.push(4, 0); // frac_bits 0 -> 2-bit coefficients
        w.push(1, 0);
        w.push(3, 0b1_01); // ch0: 1
        w.push(3, 0b1_11); // ch1: -1
        w.push(4, 9); // dither_scale
        let mut ss = state(SYNC_B, 1);
        Matrixing::parse(&mut BitReader::new(&w.bytes), &mut ss).unwrap();
        assert_eq!(&ss.matrix_coeff[0][..2], &[1, -1]);
        assert_eq!(ss.dither_scale[0], 9);
    }

    #[test]
    fn sync_c_config_coefficients_and_deltas() {
        let mut w = BitWriter::default();
        w.push(1, 1); // new_matrix
        w.push(1, 1); // new_matrix_config
        w.push(4, 0); // one matrix
        w.push(4, 9); // matrix_ch
        w.push(4, 1); // frac_bits -> 3-bit coefficients
        w.push(3, 2); // cf_shift_code -> 1
        w.push(2, 3); // lsb_bypass_bit_count
        w.push(4, 0); // dither_scale
        w.push(10, 0b10_0000_0001); // cf_mask: channels 0 and 9
        w.push(3, 0b011); // ch0 = 3
        w.push(3, 0b101); // ch9 = -3
        w.push(1, 1); // interpolation_used
        w.push(1, 1); // new_delta
        w.push(1, 1); // new_delta_config
        w.push(4, 2); // delta_bits -> 3-bit deltas
        w.push(2, 1); // delta_precision
        w.push(3, 0b111); // ch0 delta = -1
        w.push(3, 0b010); // ch9 delta = 2
        let mut ss = state(SYNC_C, 9);
        let m = Matrixing::parse(&mut BitReader::new(&w.bytes), &mut ss).unwrap();
        assert!(
            m.new_matrix
                && m.new_matrix_config
                && m.interpolation_used
                && m.new_delta
                && m.new_delta_config
        );
        assert_eq!(ss.primitive_matrices, 1);
        assert_eq!(ss.matrix_ch[0], 9);
        assert_eq!(ss.cf_shift_code[0], 1);
        assert_eq!(ss.lsb_bypass_bit_count[0], 3);
        assert_eq!(ss.cf_mask[0], 0b10_0000_0001);
        assert_eq!(ss.matrix_coeff[0][0], 3);
        assert_eq!(ss.matrix_coeff[0][9], -3);
        assert_eq!(ss.matrix_coeff[0][1], 0);
        assert_eq!(ss.delta_bits[0], 2);
        assert_eq!(ss.delta_precision[0], 1);
        assert_eq!(ss.delta_cf[0][0], -1);
        assert_eq!(ss.delta_cf[0][9], 2);

        // second block: keep config, no new matrix, interpolation without new deltas
        let mut w = BitWriter::default();
        w.push(1, 0); // new_matrix
        w.push(1, 1); // interpolation_used
        w.push(1, 0); // new_delta
        let m2 = Matrixing::parse(&mut BitReader::new(&w.bytes), &mut ss).unwrap();
        assert!(!m2.new_matrix && m2.interpolation_used && !m2.new_delta);
        assert_eq!(ss.primitive_matrices, 1);
        assert_eq!(
            ss.delta_cf[0][9], 2,
            "deltas persist while interpolation stays on"
        );

        // third block: interpolation off clears the deltas
        let mut w = BitWriter::default();
        w.push(1, 0); // new_matrix
        w.push(1, 0); // interpolation_used
        Matrixing::parse(&mut BitReader::new(&w.bytes), &mut ss).unwrap();
        assert!(!ss.interpolation_used);
        assert_eq!(ss.delta_cf[0][9], 0);
        assert_eq!(ss.matrix_coeff[0][9], -3, "coefficients persist");
    }

    #[test]
    fn matrixing_before_a_restart_header_is_an_error() {
        let mut ss = SubstreamState::default();
        assert!(Matrixing::parse(&mut BitReader::new(&[0u8; 4]), &mut ss).is_err());
    }
}
