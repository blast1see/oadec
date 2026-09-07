//! Syncframe header: enough of `syncinfo`/`bsi` to size and classify a frame.
//!
//! AC-3 (clause 4.3.1) and Enhanced AC-3 (clause E.1.2.2) differ from the
//! third byte on, but both place `bsid` at bit 40 of the frame, which is how
//! the syntax is told apart (clause E.1.1).

use oadec_bits::BitReader;

use crate::error::{Eac3Error, Result};
use crate::tables::{
    BLOCKS_PER_FRAME, FRAME_SIZE_WORDS, NFCHANS, REDUCED_SAMPLE_RATES, SAMPLE_RATES,
};

/// The sync word of every AC-3 family syncframe.
pub const SYNC_WORD: [u8; 2] = [0x0B, 0x77];

/// Samples per audio block.
pub const BLOCK_SAMPLES: usize = 256;

/// Which syntax a frame uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// AC-3, `bsid <= 8`.
    Ac3,
    /// Enhanced AC-3, `bsid` 11 to 16.
    Eac3,
}

/// Stream type of an E-AC-3 frame (table E.1.1); AC-3 frames are independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    Independent,
    Dependent,
    /// An independent stream converted from AC-3 (type 2).
    Converted,
}

/// The fixed-position header fields of a syncframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub syntax: Syntax,
    pub stream_type: StreamType,
    pub substream_id: u8,
    /// Frame size in bytes, sync word included.
    pub frame_bytes: usize,
    pub fscod: u8,
    /// Present when `fscod == 3` (reduced sample rate).
    pub fscod2: Option<u8>,
    pub sample_rate: u32,
    pub blocks: u8,
    pub acmod: u8,
    pub lfeon: bool,
    pub bsid: u8,
    /// AC-3 only.
    pub frmsizecod: Option<u8>,
}

impl FrameHeader {
    /// Number of full-bandwidth channels.
    #[must_use]
    pub fn nfchans(&self) -> usize {
        NFCHANS[usize::from(self.acmod)]
    }

    /// Number of coded channels including the LFE.
    #[must_use]
    pub fn nchans(&self) -> usize {
        self.nfchans() + usize::from(self.lfeon)
    }

    /// PCM samples per channel the frame decodes to.
    #[must_use]
    pub fn samples(&self) -> usize {
        usize::from(self.blocks) * BLOCK_SAMPLES
    }

    /// Bit rate in bit/s implied by the frame size.
    #[must_use]
    pub fn bit_rate(&self) -> u32 {
        let per_frame = self.frame_bytes as u64 * 8;
        (per_frame * u64::from(self.sample_rate) / self.samples() as u64) as u32
    }

    /// Parses the header at the start of `bytes` (at least 8 bytes needed).
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(Eac3Error::Truncated {
                needed: 8,
                available: bytes.len(),
            });
        }
        if bytes[..2] != SYNC_WORD {
            return Err(Eac3Error::NoSync);
        }
        let bsid = (bytes[5] >> 3) & 0x1F;
        let mut r = BitReader::new(&bytes[2..]);
        if bsid <= 8 {
            let _crc1 = r.read(16)?;
            let fscod = r.read(2)? as u8;
            let frmsizecod = r.read(6)? as u8;
            let _bsid = r.read(5)?;
            let _bsmod = r.read(3)?;
            let acmod = r.read(3)? as u8;
            if acmod & 1 != 0 && acmod != 1 {
                r.skip(2)?; // cmixlev
            }
            if acmod & 4 != 0 {
                r.skip(2)?; // surmixlev
            }
            if acmod == 2 {
                r.skip(2)?; // dsurmod
            }
            let lfeon = r.read_bool()?;
            if fscod == 3 {
                return Err(Eac3Error::Reserved {
                    field: "fscod",
                    value: 3,
                });
            }
            if usize::from(frmsizecod) >= FRAME_SIZE_WORDS.len() {
                return Err(Eac3Error::Reserved {
                    field: "frmsizecod",
                    value: u32::from(frmsizecod),
                });
            }
            let words = FRAME_SIZE_WORDS[usize::from(frmsizecod)][usize::from(fscod)];
            Ok(Self {
                syntax: Syntax::Ac3,
                stream_type: StreamType::Independent,
                substream_id: 0,
                frame_bytes: usize::from(words) * 2,
                fscod,
                fscod2: None,
                sample_rate: SAMPLE_RATES[usize::from(fscod)],
                blocks: 6,
                acmod,
                lfeon,
                bsid,
                frmsizecod: Some(frmsizecod),
            })
        } else if (11..=16).contains(&bsid) {
            let strmtyp = r.read(2)? as u8;
            let substream_id = r.read(3)? as u8;
            let frmsiz = r.read(11)? as usize;
            let fscod = r.read(2)? as u8;
            let (fscod2, sample_rate, blocks) = if fscod == 3 {
                let fscod2 = r.read(2)? as u8;
                if fscod2 == 3 {
                    return Err(Eac3Error::Reserved {
                        field: "fscod2",
                        value: 3,
                    });
                }
                (Some(fscod2), REDUCED_SAMPLE_RATES[usize::from(fscod2)], 6)
            } else {
                let numblkscod = r.read(2)? as usize;
                (
                    None,
                    SAMPLE_RATES[usize::from(fscod)],
                    BLOCKS_PER_FRAME[numblkscod],
                )
            };
            let acmod = r.read(3)? as u8;
            let lfeon = r.read_bool()?;
            let _bsid = r.read(5)?;
            let stream_type = match strmtyp {
                0 => StreamType::Independent,
                1 => StreamType::Dependent,
                2 => StreamType::Converted,
                _ => {
                    return Err(Eac3Error::Reserved {
                        field: "strmtyp",
                        value: 3,
                    });
                }
            };
            Ok(Self {
                syntax: Syntax::Eac3,
                stream_type,
                substream_id,
                frame_bytes: (frmsiz + 1) * 2,
                fscod,
                fscod2,
                sample_rate,
                blocks,
                acmod,
                lfeon,
                bsid,
                frmsizecod: None,
            })
        } else {
            Err(Eac3Error::Bsid(bsid))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eac3_header_fields() {
        // strmtyp 0, substreamid 0, frmsiz 1535 (3072 bytes), fscod 0, numblkscod 3,
        // acmod 7, lfeon 1, bsid 16
        // strmtyp 0 and substreamid 0 fill the top five bits, fscod 0 bits 15..14
        let bits: u64 = (1535 << 16) | (3 << 12) | (7 << 9) | (1 << 8) | (16 << 3);
        let b = bits.to_be_bytes();
        let frame = [0x0B, 0x77, b[4], b[5], b[6], b[7], 0, 0];
        let h = FrameHeader::parse(&frame).unwrap();
        assert_eq!(h.syntax, Syntax::Eac3);
        assert_eq!(h.frame_bytes, 3072);
        assert_eq!(h.blocks, 6);
        assert_eq!(h.acmod, 7);
        assert!(h.lfeon);
        assert_eq!(h.bsid, 16);
        assert_eq!(h.nchans(), 6);
        assert_eq!(h.sample_rate, 48_000);
        assert_eq!(h.bit_rate(), 768_000);
    }

    #[test]
    fn ac3_header_fields() {
        // crc1 0, fscod 0, frmsizecod 28 (640 kbps -> 1280 words), bsid 8, bsmod 0,
        // acmod 7, cmixlev, surmixlev, lfeon 1
        let mut bits: u64 = 0;
        // fscod 0 at bits 47..46
        bits |= 36 << 40; // frmsizecod 36: 640 kbps, 1280 words at 48 kHz
        bits |= 8 << 35; // bsid
        // bsmod 0 at bits 34..32
        bits |= 7 << 29; // acmod
        bits |= 1 << 27; // cmixlev
        bits |= 1 << 25; // surmixlev
        bits |= 1 << 24; // lfeon
        let b = bits.to_be_bytes();
        let frame = [0x0B, 0x77, b[0], b[1], b[2], b[3], b[4], b[5]];
        let h = FrameHeader::parse(&frame).unwrap();
        assert_eq!(h.syntax, Syntax::Ac3);
        assert_eq!(h.frame_bytes, 2560);
        assert_eq!(h.acmod, 7);
        assert!(h.lfeon);
        assert_eq!(h.bit_rate(), 640_000);
    }

    #[test]
    fn rejects_other_bsids() {
        let frame = [0x0B, 0x77, 0, 0, 0, 9 << 3, 0, 0];
        assert_eq!(FrameHeader::parse(&frame), Err(Eac3Error::Bsid(9)));
    }
}
