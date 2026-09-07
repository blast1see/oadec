//! Restart headers: the decoder initialisation points of a substream.

use oadec_bits::{BitReader, CRC8_RESTART};

use crate::error::{Error, Result};
use crate::state::{MAX_CHANNELS, ParserState, SYNC_A, SYNC_B, SYNC_C};
use crate::sync::FLAG_HEAVY_DRC;

/// A parsed restart header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartHeader {
    /// `restart_sync_word` (0x31EA, 0x31EB or 0x31EC).
    pub sync_word: u16,
    /// `output_timing`.
    pub output_timing: u16,
    /// `min_chan`.
    pub min_chan: u8,
    /// `max_chan`.
    pub max_chan: u8,
    /// `max_matrix_chan`.
    pub max_matrix_chan: u8,
    /// `dither_shift`.
    pub dither_shift: u8,
    /// `dither_seed` (23 bits).
    pub dither_seed: u32,
    /// `max_shift`.
    pub max_shift: u8,
    /// `max_lsbs`.
    pub max_lsbs: u8,
    /// `max_bits`.
    pub max_bits: u8,
    /// `max_bits_repeat` (must equal `max_bits`).
    pub max_bits_repeat: u8,
    /// `error_protect`.
    pub error_protect: bool,
    /// `lossless_check`.
    pub lossless_check: u8,
    /// `hires_output_timing`.
    pub hires_output_timing: bool,
    /// `heavy_drc_present` (only meaningful with flags bit 13).
    pub heavy_drc_present: bool,
    /// `heavy_drc_gain_update` (signed 9 bits) when present.
    pub heavy_drc_gain_update: Option<i16>,
    /// `heavy_drc_time_update` (3 bits) when present.
    pub heavy_drc_time_update: Option<u8>,
    /// `ch_assign` for channels 0..=max_matrix_chan.
    pub ch_assign: [u8; MAX_CHANNELS],
    /// CRC-8 read from the stream.
    pub crc: u8,
    /// Whether the computed CRC-8 matched.
    pub crc_ok: bool,
}

impl RestartHeader {
    /// Parses a restart header at the reader position and re-initialises the
    /// substream state.
    pub fn parse(
        reader: &mut BitReader<'_>,
        state: &mut ParserState,
        substream: usize,
    ) -> Result<Self> {
        let start = reader.position();
        let sync_word = reader.read(14)? as u16;
        match sync_word {
            SYNC_A | SYNC_B | SYNC_C => {}
            other => {
                return Err(Error::malformed(format!(
                    "invalid restart sync word {other:#06X} in substream {substream}"
                )));
            }
        }
        if sync_word == SYNC_C && substream != 3 {
            return Err(Error::malformed(format!(
                "restart sync 0x31EC in substream {substream}"
            )));
        }
        if sync_word == SYNC_B && substream == 0 {
            return Err(Error::malformed("restart sync 0x31EB in substream 0"));
        }
        if sync_word == SYNC_A && substream == 1 && state.substream_info & 8 == 0 {
            return Err(Error::malformed(
                "restart sync 0x31EA in substream 1 of a stream that requires 0x31EB",
            ));
        }

        let mut rh = Self {
            sync_word,
            output_timing: reader.read(16)? as u16,
            min_chan: reader.read(4)? as u8,
            max_chan: reader.read(4)? as u8,
            max_matrix_chan: reader.read(4)? as u8,
            dither_shift: reader.read(4)? as u8,
            dither_seed: reader.read(23)?,
            max_shift: reader.read(4)? as u8,
            max_lsbs: reader.read(5)? as u8,
            max_bits: reader.read(5)? as u8,
            max_bits_repeat: reader.read(5)? as u8,
            error_protect: reader.read_bool()?,
            lossless_check: reader.read(8)? as u8,
            hires_output_timing: false,
            heavy_drc_present: false,
            heavy_drc_gain_update: None,
            heavy_drc_time_update: None,
            ch_assign: [0; MAX_CHANNELS],
            crc: 0,
            crc_ok: false,
        };
        if rh.max_bits != rh.max_bits_repeat {
            return Err(Error::malformed(format!(
                "max_bits {} differs from its repeat {}",
                rh.max_bits, rh.max_bits_repeat
            )));
        }
        if rh.min_chan > rh.max_chan || rh.max_chan > rh.max_matrix_chan {
            return Err(Error::malformed(format!(
                "channel range {}..={} exceeds max_matrix_chan {}",
                rh.min_chan, rh.max_chan, rh.max_matrix_chan
            )));
        }
        rh.hires_output_timing = reader.read_bool()?;
        reader.skip(2)?;
        if state.flags & FLAG_HEAVY_DRC != 0 {
            rh.heavy_drc_present = reader.read_bool()?;
        } else {
            reader.skip(1)?;
        }
        if rh.heavy_drc_present {
            rh.heavy_drc_gain_update = Some(reader.read_signed(9)? as i16);
            rh.heavy_drc_time_update = Some(reader.read(3)? as u8);
        } else {
            reader.skip(12)?;
        }
        let mut seen = 0u16;
        for i in 0..=usize::from(rh.max_matrix_chan) {
            let assign = reader.read(6)? as u8;
            if assign > rh.max_matrix_chan {
                return Err(Error::malformed(format!(
                    "ch_assign[{i}] = {assign} exceeds max_matrix_chan {}",
                    rh.max_matrix_chan
                )));
            }
            if seen & (1 << assign) != 0 {
                return Err(Error::malformed(format!(
                    "ch_assign repeats channel {assign}"
                )));
            }
            seen |= 1 << assign;
            rh.ch_assign[i] = assign;
        }
        let len = reader.position() - start;
        rh.crc = reader.read(8)? as u8;
        rh.crc_ok = CRC8_RESTART.update_bits(0, reader.data(), start, len) == rh.crc;
        if !rh.crc_ok {
            return Err(Error::malformed(format!(
                "restart header CRC mismatch in substream {substream}"
            )));
        }

        let ss = &mut state.substream[substream];
        ss.reset_for_restart();
        ss.restart_seen = true;
        ss.sync_word = sync_word;
        ss.min_chan = usize::from(rh.min_chan);
        ss.max_chan = usize::from(rh.max_chan);
        ss.max_matrix_chan = usize::from(rh.max_matrix_chan);
        ss.dither_shift = rh.dither_shift;
        ss.dither_seed = rh.dither_seed;
        ss.max_shift = rh.max_shift;
        ss.max_lsbs = u32::from(rh.max_lsbs);
        ss.max_bits = rh.max_bits;
        ss.error_protect = rh.error_protect;
        ss.lossless_check = rh.lossless_check;
        ss.ch_assign = rh.ch_assign;
        Ok(rh)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::au::StreamConfig;
    use crate::testutil::{BitWriter, atmos_major_sync};

    /// Writes a restart header with the given sync word and channel range into `w`
    /// (which must be positioned right after the two block flag bits or anywhere
    /// else: the CRC covers the header only).
    pub(crate) fn push_restart_header(
        w: &mut BitWriter,
        sync_word: u16,
        min_chan: u8,
        max_chan: u8,
        max_matrix_chan: u8,
        error_protect: bool,
    ) {
        let start = w.len;
        w.push(14, u64::from(sync_word));
        w.push(16, 0x0123); // output_timing
        w.push(4, u64::from(min_chan));
        w.push(4, u64::from(max_chan));
        w.push(4, u64::from(max_matrix_chan));
        w.push(4, 0); // dither_shift
        w.push(23, 0x2A_BCDE); // dither_seed
        w.push(4, 0); // max_shift
        w.push(5, 24); // max_lsbs
        w.push(5, 24); // max_bits
        w.push(5, 24); // repeat
        w.push(1, u64::from(error_protect));
        w.push(8, 0x5A); // lossless_check
        w.push(1, 0); // hires_output_timing
        w.push(2, 0);
        w.push(1, 0); // heavy_drc_present
        w.push(12, 0);
        for ch in 0..=max_matrix_chan {
            w.push(6, u64::from(ch));
        }
        let crc = CRC8_RESTART.update_bits(0, &w.bytes, start, w.len - start);
        w.push(8, u64::from(crc));
    }

    pub(crate) fn atmos_state() -> ParserState {
        let ms = crate::sync::MajorSync::parse(&atmos_major_sync()).unwrap();
        ParserState::new(&StreamConfig::from_major_sync(&ms).unwrap())
    }

    #[test]
    fn parses_and_initialises_the_substream_state() {
        let mut w = BitWriter::default();
        w.push(2, 0b11); // block flags, not part of the header
        push_restart_header(&mut w, SYNC_A, 0, 1, 1, true);
        let mut state = atmos_state();
        state.substream[0].block_size = 40;
        let mut r = BitReader::new(&w.bytes);
        r.skip(2).unwrap();
        let rh = RestartHeader::parse(&mut r, &mut state, 0).unwrap();
        assert_eq!(rh.sync_word, SYNC_A);
        assert_eq!(rh.output_timing, 0x0123);
        assert_eq!(rh.dither_seed, 0x2A_BCDE);
        assert!(rh.crc_ok);
        assert_eq!(&rh.ch_assign[..2], &[0, 1]);
        let ss = &state.substream[0];
        assert!(ss.restart_seen);
        assert_eq!(ss.block_size, 8, "a restart header resets the block size");
        assert_eq!(ss.guards, 0xFF);
        assert!(ss.error_protect);
        assert_eq!(ss.lossless_check, 0x5A);
        assert_eq!(r.position(), w.len);
    }

    #[test]
    fn rejects_bad_crc_and_misplaced_sync_words() {
        let mut w = BitWriter::default();
        push_restart_header(&mut w, SYNC_A, 0, 1, 1, false);
        let mut corrupted = w.bytes.clone();
        corrupted[3] ^= 0x04;
        let mut state = atmos_state();
        assert!(RestartHeader::parse(&mut BitReader::new(&corrupted), &mut state, 0).is_err());

        let mut w = BitWriter::default();
        push_restart_header(&mut w, SYNC_C, 8, 15, 15, false);
        assert!(RestartHeader::parse(&mut BitReader::new(&w.bytes), &mut state, 2).is_err());
        assert!(RestartHeader::parse(&mut BitReader::new(&w.bytes), &mut state, 3).is_ok());
        assert_eq!(state.substream[3].max_matrix_chan, 15);

        let mut w = BitWriter::default();
        push_restart_header(&mut w, SYNC_B, 0, 1, 1, false);
        assert!(RestartHeader::parse(&mut BitReader::new(&w.bytes), &mut state, 0).is_err());
        assert!(RestartHeader::parse(&mut BitReader::new(&w.bytes), &mut state, 1).is_ok());
    }
}
