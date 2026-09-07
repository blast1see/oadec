//! Bit stream information (`bsi`) of AC-3 (clause 4.3.2, Annex D alternate
//! syntax) and Enhanced AC-3 (clause E.1.2.2).
//!
//! The decoder needs only a few of these fields; the rest is parsed to reach
//! the audio data and kept where it costs nothing.

use oadec_bits::BitReader;

use crate::error::Result;
use crate::header::{FrameHeader, StreamType, Syntax};

/// Parsed `bsi` fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bsi {
    pub bsmod: u8,
    pub dialnorm: u8,
    pub compr: Option<u8>,
    pub dialnorm2: Option<u8>,
    pub compr2: Option<u8>,
    /// AC-3: centre and surround mix levels, Dolby Surround mode.
    pub cmixlev: Option<u8>,
    pub surmixlev: Option<u8>,
    pub dsurmod: Option<u8>,
    /// E-AC-3 dependent substream custom channel map.
    pub chanmap: Option<u16>,
    /// E-AC-3 `mixmdate`.
    pub mixing_metadata: bool,
    /// E-AC-3 `infomdate`.
    pub info_metadata: bool,
    pub convsync: Option<bool>,
    pub blkid: Option<bool>,
    pub converted_frmsizecod: Option<u8>,
    /// Additional bit stream information bytes (`addbsi`).
    pub addbsi: Vec<u8>,
    /// Bit position just after the `bsi` (from the start of the frame).
    pub end_bit: usize,
}

impl Bsi {
    /// Parses the `bsi` that follows the sync information of `frame`.
    pub fn parse(frame: &[u8], header: &FrameHeader) -> Result<Self> {
        let mut r = BitReader::new(frame);
        let mut bsi = Self::default();
        match header.syntax {
            Syntax::Ac3 => parse_ac3(&mut r, header, &mut bsi)?,
            Syntax::Eac3 => parse_eac3(&mut r, header, &mut bsi)?,
        }
        bsi.end_bit = r.position();
        Ok(bsi)
    }

    /// The `addbsi` extension of ETSI TS 103 420 clause 8.3, when present:
    /// `(flag_ec3_extension_type_a, complexity_index_type_a)`.
    #[must_use]
    pub fn joc_extension(&self) -> Option<(bool, u8)> {
        if self.addbsi.len() >= 2 {
            let flag = self.addbsi[0] & 1 != 0;
            Some((flag, self.addbsi[1]))
        } else {
            None
        }
    }
}

fn read_addbsi(r: &mut BitReader<'_>, bsi: &mut Bsi) -> Result<()> {
    if r.read_bool()? {
        let addbsil = r.read(6)? as usize + 1;
        for _ in 0..addbsil {
            bsi.addbsi.push(r.read(8)? as u8);
        }
    }
    Ok(())
}

fn parse_ac3(r: &mut BitReader<'_>, h: &FrameHeader, bsi: &mut Bsi) -> Result<()> {
    r.skip(16 + 16 + 2 + 6)?; // syncword, crc1, fscod, frmsizecod
    r.skip(5)?; // bsid
    bsi.bsmod = r.read(3)? as u8;
    let acmod = r.read(3)? as u8;
    if acmod & 1 != 0 && acmod != 1 {
        bsi.cmixlev = Some(r.read(2)? as u8);
    }
    if acmod & 4 != 0 {
        bsi.surmixlev = Some(r.read(2)? as u8);
    }
    if acmod == 2 {
        bsi.dsurmod = Some(r.read(2)? as u8);
    }
    r.skip(1)?; // lfeon
    bsi.dialnorm = r.read(5)? as u8;
    if r.read_bool()? {
        bsi.compr = Some(r.read(8)? as u8);
    }
    if r.read_bool()? {
        r.skip(8)?; // langcod
    }
    if r.read_bool()? {
        r.skip(5 + 2)?; // mixlevel, roomtyp
    }
    if acmod == 0 {
        bsi.dialnorm2 = Some(r.read(5)? as u8);
        if r.read_bool()? {
            bsi.compr2 = Some(r.read(8)? as u8);
        }
        if r.read_bool()? {
            r.skip(8)?; // langcod2
        }
        if r.read_bool()? {
            r.skip(5 + 2)?; // mixlevel2, roomtyp2
        }
    }
    r.skip(1 + 1)?; // copyrightb, origbs
    if h.bsid == 6 {
        // Annex D alternate syntax
        if r.read_bool()? {
            r.skip(2 + 3 + 3 + 3 + 3)?; // dmixmod, ltrtcmixlev, ltrtsurmixlev, lorocmixlev, lorosurmixlev
        }
        if r.read_bool()? {
            r.skip(2 + 2 + 1 + 8 + 1)?; // dsurexmod, dheadphonmod, adconvtyp, xbsi2, encinfo
        }
    } else {
        if r.read_bool()? {
            r.skip(14)?; // timecod1
        }
        if r.read_bool()? {
            r.skip(14)?; // timecod2
        }
    }
    read_addbsi(r, bsi)
}

fn parse_eac3(r: &mut BitReader<'_>, h: &FrameHeader, bsi: &mut Bsi) -> Result<()> {
    r.skip(16 + 2 + 3 + 11 + 2)?; // syncword, strmtyp, substreamid, frmsiz, fscod
    r.skip(2)?; // numblkscod or fscod2
    r.skip(3 + 1 + 5)?; // acmod, lfeon, bsid
    let acmod = h.acmod;
    let blocks = usize::from(h.blocks);
    bsi.dialnorm = r.read(5)? as u8;
    if r.read_bool()? {
        bsi.compr = Some(r.read(8)? as u8);
    }
    if acmod == 0 {
        bsi.dialnorm2 = Some(r.read(5)? as u8);
        if r.read_bool()? {
            bsi.compr2 = Some(r.read(8)? as u8);
        }
    }
    if h.stream_type == StreamType::Dependent && r.read_bool()? {
        bsi.chanmap = Some(r.read(16)? as u16);
    }
    bsi.mixing_metadata = r.read_bool()?;
    if bsi.mixing_metadata {
        if acmod > 2 {
            r.skip(2)?; // dmixmod
        }
        if acmod & 1 != 0 && acmod > 2 {
            r.skip(3 + 3)?; // ltrtcmixlev, lorocmixlev
        }
        if acmod & 4 != 0 {
            r.skip(3 + 3)?; // ltrtsurmixlev, lorosurmixlev
        }
        if h.lfeon && r.read_bool()? {
            r.skip(5)?; // lfemixlevcod
        }
        if h.stream_type == StreamType::Independent {
            if r.read_bool()? {
                r.skip(6)?; // pgmscl
            }
            if acmod == 0 && r.read_bool()? {
                r.skip(6)?; // pgmscl2
            }
            if r.read_bool()? {
                r.skip(6)?; // extpgmscl
            }
            let mixdef = r.read(2)?;
            match mixdef {
                1 => r.skip(1 + 1 + 3)?, // premixcmpsel, drcsrc, premixcmpscl
                2 => r.skip(12)?,        // mixdata
                3 => {
                    let mixdeflen = r.read(5)? as usize;
                    let start = r.position();
                    if r.read_bool()? {
                        // mixdata2e
                        r.skip(1 + 1 + 3)?; // premixcmpsel, drcsrc, premixcmpscl
                        for _ in 0..6 {
                            // extpgmlscle .. extpgmlfescle
                            if r.read_bool()? {
                                r.skip(4)?;
                            }
                        }
                        if r.read_bool()? {
                            r.skip(4)?; // dmixscl
                        }
                        if r.read_bool()? {
                            // addche
                            if r.read_bool()? {
                                r.skip(4)?; // extpgmaux1scl
                            }
                            if r.read_bool()? {
                                r.skip(4)?; // extpgmaux2scl
                            }
                        }
                    }
                    if r.read_bool()? {
                        // mixdata3e
                        r.skip(5)?; // spchdat
                        if r.read_bool()? {
                            r.skip(5 + 2)?; // spchdat1, spchan1att
                            if r.read_bool()? {
                                r.skip(5 + 3)?; // spchdat2, spchan2att
                            }
                        }
                    }
                    // the field is 8*(mixdeflen+2) bits long including fill
                    let total = 8 * (mixdeflen + 2);
                    let used = r.position() - start;
                    if used < total {
                        r.skip(total - used)?;
                    }
                }
                _ => {}
            }
            if acmod < 2 {
                if r.read_bool()? {
                    r.skip(8 + 6)?; // panmean, paninfo
                }
                if acmod == 0 && r.read_bool()? {
                    r.skip(8 + 6)?; // panmean2, paninfo2
                }
            }
            if r.read_bool()? {
                // frmmixcfginfoe
                if blocks == 1 {
                    r.skip(5)?;
                } else {
                    for _ in 0..blocks {
                        if r.read_bool()? {
                            r.skip(5)?;
                        }
                    }
                }
            }
        }
    }
    bsi.info_metadata = r.read_bool()?;
    if bsi.info_metadata {
        bsi.bsmod = r.read(3)? as u8;
        r.skip(1 + 1)?; // copyrightb, origbs
        if acmod == 2 {
            bsi.dsurmod = Some(r.read(2)? as u8);
            r.skip(2)?; // dheadphonmod
        }
        if acmod >= 6 {
            r.skip(2)?; // dsurexmod
        }
        if r.read_bool()? {
            r.skip(5 + 2 + 1)?; // mixlevel, roomtyp, adconvtyp
        }
        if acmod == 0 && r.read_bool()? {
            r.skip(5 + 2 + 1)?; // mixlevel2, roomtyp2, adconvtyp2
        }
        if h.fscod < 3 {
            r.skip(1)?; // sourcefscod
        }
    }
    if h.stream_type == StreamType::Independent && h.blocks != 6 {
        bsi.convsync = Some(r.read_bool()?);
    }
    if h.stream_type == StreamType::Converted {
        let blkid = if h.blocks == 6 { true } else { r.read_bool()? };
        bsi.blkid = Some(blkid);
        if blkid {
            bsi.converted_frmsizecod = Some(r.read(6)? as u8);
        }
    }
    read_addbsi(r, bsi)
}
