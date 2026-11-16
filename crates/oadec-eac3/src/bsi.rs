//! Bit stream information (`bsi`) of AC-3 (clause 4.3.2, Annex D alternate
//! syntax) and Enhanced AC-3 (clause E.1.2.2).
//!
//! The decoder needs only a few of these fields; the rest is parsed to reach
//! the audio data and kept where it costs nothing.

use oadec_bits::BitReader;

use crate::error::Result;
use crate::header::{FrameHeader, StreamType, Syntax};

/// How a stream would be mixed down to two channels, as it says itself: the
/// downmix it prefers and the levels of the centre and of the surround
/// channels in each of the two (clause D.1.3; an E-AC-3 frame has the same
/// five fields in its mixing metadata, clause E.1.2.2).
///
/// The codes are as they stand in the stream. A field is `None` where the
/// frame does not carry it, and where the alternate syntax of AC-3 carries
/// it for a mode without the channels it speaks of: the notes of tables
/// D.1.1 to D.1.5 reserve its meaning there, and the syntax of E-AC-3 leaves
/// it out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Downmix {
    /// `dmixmod`.
    pub preferred: Option<u8>,
    /// `ltrtcmixlev`.
    pub ltrt_centre: Option<u8>,
    /// `ltrtsurmixlev`.
    pub ltrt_surround: Option<u8>,
    /// `lorocmixlev`.
    pub loro_centre: Option<u8>,
    /// `lorosurmixlev`.
    pub loro_surround: Option<u8>,
}

impl Downmix {
    /// Table D.1.1: the downmix a code prefers.
    #[must_use]
    pub const fn preferred_name(code: u8) -> &'static str {
        match code & 3 {
            0 => "not indicated",
            1 => "Lt/Rt",
            2 => "Lo/Ro",
            _ => "reserved",
        }
    }

    /// Tables D.1.2 and D.1.4: the level of the centre channel, in dB.
    #[must_use]
    pub const fn centre_db(code: u8) -> &'static str {
        match code & 7 {
            0 => "+3.0",
            1 => "+1.5",
            2 => "0.0",
            3 => "-1.5",
            4 => "-3.0",
            5 => "-4.5",
            6 => "-6.0",
            _ => "-inf",
        }
    }

    /// Tables D.1.3 and D.1.5: the level of the surround channels, in dB;
    /// `None` for the three codes the tables reserve.
    #[must_use]
    pub const fn surround_db(code: u8) -> Option<&'static str> {
        match code & 7 {
            0..=2 => None,
            3 => Some("-1.5"),
            4 => Some("-3.0"),
            5 => Some("-4.5"),
            6 => Some("-6.0"),
            _ => Some("-inf"),
        }
    }

    /// `self` where it says anything.
    fn said(self) -> Option<Self> {
        (self != Self::default()).then_some(self)
    }
}

/// Whether a mode has a downmix to prefer: three channels or more.
const fn prefers(acmod: u8) -> bool {
    acmod > 2
}

/// Whether a mode has a centre channel beside a left and a right one.
const fn has_centre(acmod: u8) -> bool {
    acmod & 1 != 0 && acmod > 2
}

/// Whether a mode has a surround channel.
const fn has_surround(acmod: u8) -> bool {
    acmod & 4 != 0
}

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
    /// What the frame says of its downmix to two channels, when it says any
    /// of it: in the first extra word of the alternate syntax of AC-3, in
    /// the mixing metadata of E-AC-3.
    pub downmix: Option<Downmix>,
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
            // xbsi1: fourteen bits whatever the mode
            let dmixmod = r.read(2)? as u8;
            let ltrtcmixlev = r.read(3)? as u8;
            let ltrtsurmixlev = r.read(3)? as u8;
            let lorocmixlev = r.read(3)? as u8;
            let lorosurmixlev = r.read(3)? as u8;
            bsi.downmix = Downmix {
                preferred: prefers(acmod).then_some(dmixmod),
                ltrt_centre: has_centre(acmod).then_some(ltrtcmixlev),
                ltrt_surround: has_surround(acmod).then_some(ltrtsurmixlev),
                loro_centre: has_centre(acmod).then_some(lorocmixlev),
                loro_surround: has_surround(acmod).then_some(lorosurmixlev),
            }
            .said();
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
        let mut downmix = Downmix::default();
        if prefers(acmod) {
            downmix.preferred = Some(r.read(2)? as u8); // dmixmod
        }
        if has_centre(acmod) {
            downmix.ltrt_centre = Some(r.read(3)? as u8); // ltrtcmixlev
            downmix.loro_centre = Some(r.read(3)? as u8); // lorocmixlev
        }
        if has_surround(acmod) {
            downmix.ltrt_surround = Some(r.read(3)? as u8); // ltrtsurmixlev
            downmix.loro_surround = Some(r.read(3)? as u8); // lorosurmixlev
        }
        bsi.downmix = downmix.said();
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

#[cfg(test)]
mod tests {
    use super::*;
    use oadec_bits::BitWriter;

    fn parsed(frame: &[u8]) -> Bsi {
        let header = FrameHeader::parse(frame).expect("a header");
        Bsi::parse(frame, &header).expect("a bsi")
    }

    /// An E-AC-3 syncframe of an independent substream, six blocks at 48 kHz,
    /// as far as `mixmdate`: clause E.1.2.2, field by field.
    fn eac3_head(acmod: u32, lfeon: bool, mixmdate: bool) -> BitWriter {
        assert!(
            acmod >= 2,
            "the modes below 2/0 have fields these frames leave out"
        );
        let mut w = BitWriter::new();
        w.write(0x0B77, 16); // syncword
        w.write(0, 2); // strmtyp: independent
        w.write(0, 3); // substreamid
        w.write(255, 11); // frmsiz: 256 words
        w.write(0, 2); // fscod: 48 kHz
        w.write(3, 2); // numblkscod: six blocks
        w.write(acmod, 3);
        w.write_bit(lfeon);
        w.write(16, 5); // bsid
        w.write(27, 5); // dialnorm
        w.write_bit(false); // compre
        w.write_bit(mixmdate);
        w
    }

    /// What follows the downmix fields where a frame says nothing else, and
    /// an `addbsi` of two bytes at the end: a parse that took a field before
    /// it at another width does not find these two bytes.
    fn eac3_tail(mut w: BitWriter, lfeon: bool, mixmdate: bool) -> Vec<u8> {
        if mixmdate {
            if lfeon {
                w.write_bit(false); // lfemixlevcode
            }
            w.write_bit(false); // pgmscle
            w.write_bit(false); // extpgmscle
            w.write(0, 2); // mixdef
            w.write_bit(false); // frmmixcfginfoe
        }
        w.write_bit(false); // infomdate
        addbsi(w)
    }

    fn addbsi(mut w: BitWriter) -> Vec<u8> {
        w.write_bit(true); // addbsie
        w.write(1, 6); // addbsil: two bytes
        w.write(0xA5, 8);
        w.write(0x3C, 8);
        w.write_zeros(64);
        w.finish()
    }

    /// An AC-3 syncframe with an LFE channel as far as the end of what both
    /// syntaxes share: clauses 4.3.1 and 4.3.2, field by field.
    fn ac3_head(bsid: u32, acmod: u32) -> BitWriter {
        assert!(acmod >= 2, "1+1 and 1/0 have fields these frames leave out");
        let mut w = BitWriter::new();
        w.write(0x0B77, 16); // syncword
        w.write(0, 16); // crc1
        w.write(0, 2); // fscod: 48 kHz
        w.write(28, 6); // frmsizecod
        w.write(bsid, 5);
        w.write(0, 3); // bsmod
        w.write(acmod, 3);
        if acmod & 1 != 0 {
            w.write(1, 2); // cmixlev
        }
        if acmod & 4 != 0 {
            w.write(1, 2); // surmixlev
        }
        if acmod == 2 {
            w.write(0, 2); // dsurmod
        }
        w.write_bit(true); // lfeon
        w.write(27, 5); // dialnorm
        w.write_bit(false); // compre
        w.write_bit(false); // langcode
        w.write_bit(false); // audprodie
        w.write_bit(false); // copyrightb
        w.write_bit(true); // origbs
        w
    }

    /// The first extra word of the alternate syntax, clause D.1.2: the
    /// preferred downmix, the two Lt/Rt levels, then the two Lo/Ro levels.
    fn xbsi1(w: &mut BitWriter, codes: [u32; 5]) {
        w.write_bit(true); // xbsi1e
        w.write(codes[0], 2); // dmixmod
        w.write(codes[1], 3); // ltrtcmixlev
        w.write(codes[2], 3); // ltrtsurmixlev
        w.write(codes[3], 3); // lorocmixlev
        w.write(codes[4], 3); // lorosurmixlev
    }

    #[test]
    fn an_eac3_frame_says_how_it_would_be_mixed_down() {
        let mut w = eac3_head(7, true, true);
        w.write(1, 2); // dmixmod
        w.write(5, 3); // ltrtcmixlev
        w.write(3, 3); // lorocmixlev
        w.write(4, 3); // ltrtsurmixlev
        w.write(6, 3); // lorosurmixlev
        let bsi = parsed(&eac3_tail(w, true, true));
        assert_eq!(
            bsi.downmix,
            Some(Downmix {
                preferred: Some(1),
                ltrt_centre: Some(5),
                loro_centre: Some(3),
                ltrt_surround: Some(4),
                loro_surround: Some(6),
            })
        );
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn without_a_centre_channel_no_centre_level_is_said() {
        // 2/2: a preferred downmix and the two surround levels
        let mut w = eac3_head(6, false, true);
        w.write(2, 2);
        w.write(7, 3);
        w.write(3, 3);
        let bsi = parsed(&eac3_tail(w, false, true));
        assert_eq!(
            bsi.downmix,
            Some(Downmix {
                preferred: Some(2),
                ltrt_centre: None,
                loro_centre: None,
                ltrt_surround: Some(7),
                loro_surround: Some(3),
            })
        );
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn without_surround_channels_no_surround_level_is_said() {
        // 3/0: a preferred downmix and the two centre levels
        let mut w = eac3_head(3, false, true);
        w.write(1, 2);
        w.write(0, 3);
        w.write(7, 3);
        let bsi = parsed(&eac3_tail(w, false, true));
        assert_eq!(
            bsi.downmix,
            Some(Downmix {
                preferred: Some(1),
                ltrt_centre: Some(0),
                loro_centre: Some(7),
                ltrt_surround: None,
                loro_surround: None,
            })
        );
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn two_channels_say_nothing_of_a_downmix() {
        // 2/0 with mixing metadata: none of the five fields is in its syntax
        let bsi = parsed(&eac3_tail(eac3_head(2, false, true), false, true));
        assert_eq!(bsi.downmix, None);
        assert!(bsi.mixing_metadata);
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn a_frame_without_mixing_metadata_says_nothing_of_a_downmix() {
        let bsi = parsed(&eac3_tail(eac3_head(7, true, false), true, false));
        assert_eq!(bsi.downmix, None);
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn the_alternate_syntax_of_ac3_says_it_in_an_order_of_its_own() {
        let mut w = ac3_head(6, 7);
        xbsi1(&mut w, [2, 1, 4, 6, 7]);
        w.write_bit(false); // xbsi2e
        let bsi = parsed(&addbsi(w));
        assert_eq!(
            bsi.downmix,
            Some(Downmix {
                preferred: Some(2),
                ltrt_centre: Some(1),
                ltrt_surround: Some(4),
                loro_centre: Some(6),
                loro_surround: Some(7),
            })
        );
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn the_alternate_syntax_keeps_what_the_mode_gives_a_meaning() {
        // The word is fourteen bits whatever the mode, and the notes of
        // tables D.1.1 to D.1.5 reserve the meaning of a field for a mode
        // without the channels it speaks of. 2/1: no centre.
        let mut w = ac3_head(6, 4);
        xbsi1(&mut w, [1, 3, 5, 2, 6]);
        w.write_bit(false); // xbsi2e
        let bsi = parsed(&addbsi(w));
        assert_eq!(
            bsi.downmix,
            Some(Downmix {
                preferred: Some(1),
                ltrt_centre: None,
                ltrt_surround: Some(5),
                loro_centre: None,
                loro_surround: Some(6),
            })
        );
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
        // 2/0: none of the five has a meaning
        let mut w = ac3_head(6, 2);
        xbsi1(&mut w, [1, 3, 5, 2, 6]);
        w.write_bit(false);
        let bsi = parsed(&addbsi(w));
        assert_eq!(bsi.downmix, None);
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn ac3_says_nothing_of_a_downmix_without_the_word_that_holds_it() {
        // the alternate syntax with no first extra word
        let mut w = ac3_head(6, 7);
        w.write_bit(false); // xbsi1e
        w.write_bit(false); // xbsi2e
        let bsi = parsed(&addbsi(w));
        assert_eq!(bsi.downmix, None);
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
        // and the syntax of clause 4, which has time codes where the words are
        let mut w = ac3_head(8, 7);
        w.write_bit(false); // timecod1e
        w.write_bit(false); // timecod2e
        let bsi = parsed(&addbsi(w));
        assert_eq!(bsi.downmix, None);
        assert_eq!(bsi.addbsi, [0xA5, 0x3C]);
    }

    #[test]
    fn the_codes_read_as_the_tables_of_annex_d_have_them() {
        assert_eq!(
            [0, 1, 2, 3].map(Downmix::preferred_name),
            ["not indicated", "Lt/Rt", "Lo/Ro", "reserved"]
        );
        assert_eq!(
            (0..8).map(Downmix::centre_db).collect::<Vec<_>>(),
            [
                "+3.0", "+1.5", "0.0", "-1.5", "-3.0", "-4.5", "-6.0", "-inf"
            ]
        );
        assert_eq!(
            (0..8).map(Downmix::surround_db).collect::<Vec<_>>(),
            [
                None,
                None,
                None,
                Some("-1.5"),
                Some("-3.0"),
                Some("-4.5"),
                Some("-6.0"),
                Some("-inf")
            ]
        );
    }
}
