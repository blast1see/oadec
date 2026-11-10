//! Major sync information.
//!
//! A major sync appears at the start of the first access unit and at least every 128
//! access units afterwards. It describes the stream: sampling rate, channel
//! assignments, presentation layout, data rate and the presentation-level metadata of
//! the `channel_meaning` block. It is protected by a CRC-16.

use oadec_bits::{BitReader, CRC16_MAJOR_SYNC};

use crate::channel::ChannelMeaning;
use crate::error::{Error, Result};
use crate::presentation::PresentationMap;

/// Major sync pattern of Dolby TrueHD (FBA) streams.
pub const SYNC_FBA: u32 = 0xF872_6FBA;

/// Major sync pattern of Meridian MLP (FBB, DVD-Audio) streams.
pub const SYNC_FBB: u32 = 0xF872_6FBB;

/// Expected `signature` value.
pub const SIGNATURE: u16 = 0xB752;

/// `flags` bit 12: the extra data carries Evolution frames.
pub const FLAG_EVOLUTION_IN_EXTRA_DATA: u16 = 0x1000;

/// `flags` bit 13: restart headers carry heavy DRC updates.
pub const FLAG_HEAVY_DRC: u16 = 0x2000;

/// `flags` bit 11: the eight-channel presentation is restricted (five-bit assignment).
pub const FLAG_RESTRICTED_8CH: u16 = 0x0800;

/// Base sampling rate of the 48 kHz family.
pub const BASE_RATE_48K: u32 = 48_000;

/// Base sampling rate of the 44.1 kHz family.
pub const BASE_RATE_44K1: u32 = 44_100;

/// Samples per access unit at the base rates.
pub const BASE_SAMPLES_PER_AU: u16 = 40;

/// Smallest major sync in bytes (no extra channel meaning).
pub const MIN_LEN_BYTES: usize = 28;

/// Length of an FBB (DVD-Audio) major sync in bytes, CRC included. FBB has no
/// extension: 26 bytes of information, then the CRC-16 over them.
pub const FBB_LEN_BYTES: usize = 28;

/// What an FBB (DVD-Audio) major sync says in the place an FBA one has its
/// `format_info`: two groups of channels, each with its word length and
/// sampling rate, and the channel assignment that names them.
///
/// The layout is the one FFmpeg's `ff_mlp_read_major_sync` reads (32 bits:
/// group 1 and group 2 word length codes, group 1 and group 2 rate codes, 11
/// bits it skips, a 5-bit channel assignment), confirmed on three DVD-Audio
/// streams, one written by Meridian's own encoder, whose log names the
/// assignment and the groups the fields decode to (docs/truehd.md, FBB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FbbFormat {
    /// Group 1 word length code: 0, 1, 2 = 16, 20, 24 bits.
    pub group1_bits_code: u8,
    /// Group 2 word length code, 0xF without a group 2.
    pub group2_bits_code: u8,
    /// Group 1 sampling frequency code, as `audio_sampling_frequency_1`.
    pub group1_rate_code: u8,
    /// Group 2 sampling frequency code, 0xF without a group 2.
    pub group2_rate_code: u8,
    /// The 11 bits between the rates and the assignment, as read.
    pub between: u16,
    /// The four bits after the substream count, which an FBA major sync
    /// spends on `extended_substream_info`, as read.
    pub after_substreams: u8,
    /// `channel_assignment`, 0 to 20 defined.
    pub channel_assignment: u8,
    /// The eight bytes between `substream_info` and the CRC, as read.
    pub channel_meaning: [u8; 8],
}

impl FbbFormat {
    /// A word length code in bits, `None` for one not defined.
    #[must_use]
    pub const fn bits(code: u8) -> Option<u8> {
        match code {
            0 => Some(16),
            1 => Some(20),
            2 => Some(24),
            _ => None,
        }
    }

    /// A rate code in Hz, `None` for one not defined (0xF: no group).
    #[must_use]
    pub const fn rate(code: u8) -> Option<u32> {
        match code {
            code @ 0..=2 => Some(BASE_RATE_48K << code),
            code @ 8..=10 => Some(BASE_RATE_44K1 << (code - 8)),
            _ => None,
        }
    }

    /// Whether the stream has a group 2 of channels.
    #[must_use]
    pub const fn has_group2(&self) -> bool {
        self.group2_rate_code != 0xF
    }
}

/// The 32-bit `format_info` word of an FBA major sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormatInfo {
    /// `audio_sampling_frequency_1` (4 bits): 0–2 = 48/96/192 kHz, 8–10 = 44.1/88.2/176.4 kHz.
    pub sampling_frequency_code: u8,
    /// `sixch_multi_channel_type`.
    pub sixch_multichannel_type: bool,
    /// `eightch_multi_channel_type`.
    pub eightch_multichannel_type: bool,
    /// `twoch_decoder_channel_modifier` (2 bits).
    pub twoch_channel_modifier: u8,
    /// `sixch_decoder_channel_modifier` (2 bits).
    pub sixch_channel_modifier: u8,
    /// `sixch_decoder_channel_assignment` (5 bits).
    pub sixch_channel_assignment: u8,
    /// `eightch_decoder_channel_modifier` (2 bits).
    pub eightch_channel_modifier: u8,
    /// `eightch_decoder_channel_assignment` (13 bits).
    pub eightch_channel_assignment: u16,
}

impl FormatInfo {
    /// Sampling frequency in Hz, or `None` for a reserved code.
    #[must_use]
    pub const fn sampling_frequency(&self) -> Option<u32> {
        match self.sampling_frequency_code {
            code @ 0..=2 => Some(BASE_RATE_48K << code),
            code @ 8..=10 => Some(BASE_RATE_44K1 << (code - 8)),
            _ => None,
        }
    }

    /// Samples per access unit (40 at 44.1/48 kHz, 80 and 160 at the doubled rates).
    #[must_use]
    pub const fn samples_per_au(&self) -> Option<u16> {
        match self.sampling_frequency() {
            Some(fs) => Some(BASE_SAMPLES_PER_AU * (fs / BASE_RATE_44K1) as u16),
            None => None,
        }
    }
}

/// A parsed major sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MajorSync {
    /// `format_sync` (always [`SYNC_FBA`] here).
    pub format_sync: u32,
    /// `format_info`.
    pub format_info: FormatInfo,
    /// `signature` (expected [`SIGNATURE`]).
    pub signature: u16,
    /// `flags`.
    pub flags: u16,
    /// `reserved` 16 bits.
    pub reserved: u16,
    /// `variable_rate`.
    pub variable_rate: bool,
    /// `peak_data_rate` (15 bits, in units of `sampling_frequency / 16` bits per second).
    pub peak_data_rate: u16,
    /// `substreams` (4 bits).
    pub substreams: u8,
    /// `extended_substream_info` (4 raw bits; the low two are meaningful).
    pub extended_substream_info: u8,
    /// `substream_info`.
    pub substream_info: u8,
    /// `channel_meaning` (with the optional extra channel meaning).
    pub channel_meaning: ChannelMeaning,
    /// CRC-16 read from the stream.
    pub crc: u16,
    /// Whether the computed CRC-16 matches.
    pub crc_ok: bool,
    /// Total length in bytes, CRC included.
    pub len_bytes: usize,
    /// The FBB fields of a DVD-Audio stream, where `format_info` carries
    /// only the group 1 rate and `channel_meaning` nothing.
    pub fbb: Option<FbbFormat>,
}

impl MajorSync {
    /// Returns the sync pattern found at the start of `bytes`, if any.
    #[must_use]
    pub fn pattern_at(bytes: &[u8]) -> Option<u32> {
        let word = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?);
        (word == SYNC_FBA || word == SYNC_FBB).then_some(word)
    }

    /// Whether `bytes` start with an FBB (DVD-Audio) major sync whose CRC-16 holds.
    ///
    /// An FBB major sync is [`FBB_LEN_BYTES`] long, with the CRC-16 of the FBA
    /// major sync (same generator, same register) over its first 26 bytes. That
    /// is measured, not assumed: three DVD-Audio streams, one of them written by
    /// Meridian's own encoder, verify this way at byte 26 and at no other offset
    /// (docs/truehd.md). The extractor needs it to tell a DVD-Audio stream from
    /// the same four bytes met by chance inside damaged data.
    #[must_use]
    pub fn fbb_crc_ok(bytes: &[u8]) -> bool {
        let Some(head) = bytes.get(..FBB_LEN_BYTES) else {
            return false;
        };
        let crc_offset = FBB_LEN_BYTES - 2;
        u32::from_be_bytes([head[0], head[1], head[2], head[3]]) == SYNC_FBB
            && CRC16_MAJOR_SYNC.update_bytes(0, &head[..crc_offset])
                == u16::from_be_bytes([head[crc_offset], head[crc_offset + 1]])
    }

    /// Parses a major sync that starts at `bytes[0]`.
    ///
    /// Only the bytes the major sync occupies are read; trailing data is ignored.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < MIN_LEN_BYTES {
            return Err(Error::malformed("major sync truncated"));
        }
        let mut reader = BitReader::new(bytes);
        let format_sync = reader.read(32)?;
        match format_sync {
            SYNC_FBA => {}
            SYNC_FBB => return Self::parse_fbb(bytes),
            other => {
                return Err(Error::malformed(format!(
                    "bad major sync pattern {other:08X}"
                )));
            }
        }

        let format_info = FormatInfo {
            sampling_frequency_code: reader.read(4)? as u8,
            sixch_multichannel_type: reader.read_bool()?,
            eightch_multichannel_type: reader.read_bool()?,
            twoch_channel_modifier: {
                reader.skip(2)?;
                reader.read(2)? as u8
            },
            sixch_channel_modifier: reader.read(2)? as u8,
            sixch_channel_assignment: reader.read(5)? as u8,
            eightch_channel_modifier: reader.read(2)? as u8,
            eightch_channel_assignment: reader.read(13)? as u16,
        };
        let signature = reader.read(16)? as u16;
        let flags = reader.read(16)? as u16;
        let reserved = reader.read(16)? as u16;
        let variable_rate = reader.read_bool()?;
        let peak_data_rate = reader.read(15)? as u16;
        let substreams = reader.read(4)? as u8;
        let extended_substream_info = reader.read(4)? as u8;
        let substream_info = reader.read(8)? as u8;
        let has_16ch = substream_info >> 7 != 0;
        let channel_meaning = ChannelMeaning::parse(&mut reader, has_16ch)?;

        if !reader.is_aligned(16) {
            return Err(Error::malformed(
                "major sync not 16-bit aligned before its CRC",
            ));
        }
        let crc_offset = reader.position() / 8;
        let len_bytes = crc_offset + 2;
        if bytes.len() < len_bytes {
            return Err(Error::malformed("major sync truncated before its CRC"));
        }
        let crc = u16::from_be_bytes([bytes[crc_offset], bytes[crc_offset + 1]]);
        let computed = CRC16_MAJOR_SYNC.update_bytes(0, &bytes[..crc_offset]);

        Ok(Self {
            format_sync,
            format_info,
            signature,
            flags,
            reserved,
            variable_rate,
            peak_data_rate,
            substreams,
            extended_substream_info,
            substream_info,
            channel_meaning,
            crc,
            crc_ok: computed == crc,
            len_bytes,
            fbb: None,
        })
    }

    /// Parses an FBB major sync: [`FBB_LEN_BYTES`] long, its fields where
    /// [`FbbFormat`] says, the rest in the places of the FBA ones, and the
    /// CRC-16 of the FBA major sync over the first 26 bytes.
    fn parse_fbb(bytes: &[u8]) -> Result<Self> {
        let Some(head) = bytes.get(..FBB_LEN_BYTES) else {
            return Err(Error::malformed("major sync truncated"));
        };
        let mut reader = BitReader::new(head);
        reader.skip(32)?;
        let group1_bits_code = reader.read(4)? as u8;
        let group2_bits_code = reader.read(4)? as u8;
        let group1_rate_code = reader.read(4)? as u8;
        let group2_rate_code = reader.read(4)? as u8;
        let between = reader.read(11)? as u16;
        let channel_assignment = reader.read(5)? as u8;
        let signature = reader.read(16)? as u16;
        let flags = reader.read(16)? as u16;
        let reserved = reader.read(16)? as u16;
        let variable_rate = reader.read_bool()?;
        let peak_data_rate = reader.read(15)? as u16;
        let substreams = reader.read(4)? as u8;
        let after_substreams = reader.read(4)? as u8;
        let substream_info = reader.read(8)? as u8;
        let mut channel_meaning = [0u8; 8];
        channel_meaning.copy_from_slice(&head[18..26]);
        let crc = u16::from_be_bytes([head[26], head[27]]);
        let computed = CRC16_MAJOR_SYNC.update_bytes(0, &head[..26]);
        Ok(Self {
            format_sync: SYNC_FBB,
            format_info: FormatInfo {
                sampling_frequency_code: group1_rate_code,
                ..FormatInfo::default()
            },
            signature,
            flags,
            reserved,
            variable_rate,
            peak_data_rate,
            substreams,
            // FBB has no object presentation for these bits to describe
            extended_substream_info: 0,
            substream_info,
            channel_meaning: ChannelMeaning::default(),
            crc,
            crc_ok: computed == crc,
            len_bytes: FBB_LEN_BYTES,
            fbb: Some(FbbFormat {
                group1_bits_code,
                group2_bits_code,
                group1_rate_code,
                group2_rate_code,
                between,
                after_substreams,
                channel_assignment,
                channel_meaning,
            }),
        })
    }

    /// Whether this is the major sync of an FBB (DVD-Audio) stream.
    #[must_use]
    pub const fn is_fbb(&self) -> bool {
        self.format_sync == SYNC_FBB
    }

    /// Sampling frequency in Hz.
    #[must_use]
    pub const fn sampling_frequency(&self) -> Option<u32> {
        self.format_info.sampling_frequency()
    }

    /// Samples per access unit.
    #[must_use]
    pub const fn samples_per_au(&self) -> Option<u16> {
        self.format_info.samples_per_au()
    }

    /// Peak data rate in bits per second (`peak_data_rate * fs / 16`, rounded).
    #[must_use]
    pub fn peak_bit_rate(&self) -> Option<u64> {
        let fs = u64::from(self.sampling_frequency()?);
        Some((u64::from(self.peak_data_rate) * fs + 8) >> 4)
    }

    /// The presentation map declared by `substream_info` / `extended_substream_info`.
    #[must_use]
    pub const fn presentation_map(&self) -> PresentationMap {
        PresentationMap::from_substream_info(self.substream_info, self.extended_substream_info)
    }

    /// Whether the stream declares a 16-channel (object) presentation.
    #[must_use]
    pub const fn has_16ch_presentation(&self) -> bool {
        self.substream_info >> 7 != 0 && self.substreams == 4
    }

    /// Whether the extra data of access units carries Evolution frames.
    #[must_use]
    pub const fn extra_data_is_evolution(&self) -> bool {
        self.flags & FLAG_EVOLUTION_IN_EXTRA_DATA != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::atmos_major_sync;

    #[test]
    fn parses_fields_and_checks_crc() {
        let bytes = atmos_major_sync();
        let ms = MajorSync::parse(&bytes).unwrap();
        assert_eq!(ms.len_bytes, 34);
        assert!(ms.crc_ok);
        assert_eq!(ms.sampling_frequency(), Some(48_000));
        assert_eq!(ms.samples_per_au(), Some(40));
        assert_eq!(ms.signature, SIGNATURE);
        assert_eq!(ms.flags, 0x1000);
        assert!(ms.variable_rate);
        assert_eq!(ms.peak_data_rate, 0x0800);
        assert_eq!(ms.peak_bit_rate(), Some((0x0800 * 48_000 + 8) >> 4));
        assert_eq!(ms.substreams, 4);
        assert_eq!(ms.extended_substream_info, 3);
        assert_eq!(ms.substream_info, 0xFC);
        assert!(ms.has_16ch_presentation());
        assert!(ms.extra_data_is_evolution());
        assert_eq!(ms.format_info.sixch_channel_assignment, 0b01111);
        assert_eq!(
            ms.format_info.eightch_channel_assignment,
            0b0_0000_0100_1111
        );
        let extra = ms.channel_meaning.extra.as_ref().unwrap();
        assert_eq!(extra.sixteench_channel_count, 15);
        assert!(!extra.dyn_object_only);
        assert_eq!(extra.content_description, 0b0101);
        assert_eq!(extra.dynamic_object_count, 5);
        assert_eq!(ms.presentation_map().mask(3), 0b1111);

        let mut corrupted = bytes.clone();
        corrupted[12] ^= 0x01;
        assert!(!MajorSync::parse(&corrupted).unwrap().crc_ok);
    }

    #[test]
    fn an_fbb_major_sync_is_read_where_its_fields_are() {
        let ms = MajorSync::parse(&crate::testutil::fbb_major_sync()).unwrap();
        assert!(ms.is_fbb());
        assert!(ms.crc_ok);
        assert_eq!(ms.len_bytes, FBB_LEN_BYTES);
        assert_eq!(ms.sampling_frequency(), Some(44_100));
        assert_eq!(ms.samples_per_au(), Some(40));
        assert_eq!(ms.signature, SIGNATURE);
        assert_eq!(ms.flags, 0x4000);
        assert_eq!(ms.substreams, 1);
        assert_eq!(ms.substream_info, 0x05);
        let f = ms.fbb.unwrap();
        assert_eq!(FbbFormat::bits(f.group1_bits_code), Some(16));
        assert!(!f.has_group2());
        assert_eq!(f.channel_assignment, 1);
        let mut bytes = crate::testutil::fbb_major_sync();
        bytes[9] ^= 0x01;
        assert!(!MajorSync::parse(&bytes).unwrap().crc_ok);
    }

    #[test]
    fn garbage_is_rejected() {
        let mut bytes = atmos_major_sync();
        bytes[3] = 0x00;
        assert!(matches!(MajorSync::parse(&bytes), Err(Error::Malformed(_))));
        assert!(MajorSync::parse(&bytes[..10]).is_err());
    }

    #[test]
    fn sampling_codes() {
        let mut fi = FormatInfo::default();
        for (code, fs, spa) in [
            (0, 48_000, 40),
            (1, 96_000, 80),
            (2, 192_000, 160),
            (8, 44_100, 40),
            (9, 88_200, 80),
            (10, 176_400, 160),
        ] {
            fi.sampling_frequency_code = code;
            assert_eq!(fi.sampling_frequency(), Some(fs));
            assert_eq!(fi.samples_per_au(), Some(spa));
        }
        fi.sampling_frequency_code = 5;
        assert_eq!(fi.sampling_frequency(), None);
    }
}
