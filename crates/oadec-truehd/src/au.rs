//! Access-unit framing: header, optional major sync, substream directory, segment
//! byte ranges and the extra-data block.
//!
//! This module reads everything *around* the substream segments; the segments
//! themselves are parsed by the substream layer.

use core::ops::Range;

use oadec_bits::{BitReader, fold_nibble, xor_bytes};

use crate::error::{Error, Result};
use crate::extra::ExtraData;
use crate::presentation::PresentationMap;
use crate::sync::MajorSync;

/// Stream-level parameters in force for an access unit, taken from the last major sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamConfig {
    /// Number of substreams.
    pub substreams: u8,
    /// Major sync `flags`.
    pub flags: u16,
    /// Sampling frequency in Hz.
    pub sampling_frequency: u32,
    /// Samples per access unit.
    pub samples_per_au: u16,
    /// `substream_info`.
    pub substream_info: u8,
    /// `extended_substream_info` (raw 4 bits).
    pub extended_substream_info: u8,
    /// Presentation map.
    pub presentations: PresentationMap,
}

impl StreamConfig {
    /// Derives the configuration from a major sync.
    pub fn from_major_sync(ms: &MajorSync) -> Result<Self> {
        let sampling_frequency = ms.sampling_frequency().ok_or_else(|| {
            Error::malformed(format!(
                "reserved sampling frequency code {}",
                ms.format_info.sampling_frequency_code
            ))
        })?;
        if ms.substreams == 0 || ms.substreams as usize > crate::presentation::MAX_PRESENTATIONS {
            return Err(Error::malformed(format!(
                "invalid substream count {}",
                ms.substreams
            )));
        }
        Ok(Self {
            substreams: ms.substreams,
            flags: ms.flags,
            sampling_frequency,
            samples_per_au: ms.samples_per_au().unwrap_or(40),
            substream_info: ms.substream_info,
            extended_substream_info: ms.extended_substream_info,
            presentations: ms.presentation_map(),
        })
    }
}

/// The 32-bit access-unit header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuHeader {
    /// `check_nibble`.
    pub check_nibble: u8,
    /// `access_unit_length` in 16-bit words (including the header).
    pub length_words: u16,
    /// `input_timing`.
    pub input_timing: u16,
}

impl AuHeader {
    /// Parses the header from the first four bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let Some(head) = bytes.get(..4) else {
            return Err(Error::malformed("access unit shorter than its header"));
        };
        let word = u16::from_be_bytes([head[0], head[1]]);
        Ok(Self {
            check_nibble: (word >> 12) as u8,
            length_words: word & 0x0FFF,
            input_timing: u16::from_be_bytes([head[2], head[3]]),
        })
    }

    /// Length of the access unit in bytes.
    #[must_use]
    pub const fn length_bytes(&self) -> usize {
        self.length_words as usize * 2
    }
}

/// One substream directory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// `extra_substream_word`.
    pub extra_word: bool,
    /// `restart_nonexistent`.
    pub restart_nonexistent: bool,
    /// `crc_present`.
    pub crc_present: bool,
    /// Reserved bit.
    pub reserved: bool,
    /// `substream_end_ptr` in 16-bit words from the start of the first segment.
    pub end_ptr_words: u16,
    /// `drc_gain_update` (signed 9 bits) when the extra word is present.
    pub drc_gain_update: Option<i16>,
    /// `drc_time_update` (3 bits) when the extra word is present.
    pub drc_time_update: Option<u8>,
}

/// A framed access unit (segments not yet parsed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    /// Header.
    pub header: AuHeader,
    /// Major sync, when the access unit carries one.
    pub major_sync: Option<MajorSync>,
    /// Directory entries, one per substream.
    pub directory: Vec<DirectoryEntry>,
    /// Nibble parity over the header and directory bytes is `0xF`.
    pub header_parity_ok: bool,
    /// Byte offset of the first substream segment.
    pub segments_start: usize,
    /// Extra-data block, when the access unit has room for one.
    pub extra: Option<ExtraData>,
}

impl AccessUnit {
    /// Parses the framing of an access unit. `config` is the configuration from the
    /// last major sync; it may be `None` only if this access unit carries one.
    ///
    /// Returns the access unit and the configuration in force after it.
    pub fn parse(bytes: &[u8], config: Option<&StreamConfig>) -> Result<(Self, StreamConfig)> {
        let header = AuHeader::parse(bytes)?;
        let len = header.length_bytes();
        if len < 4 {
            return Err(Error::malformed(format!(
                "access unit length {len} bytes is too short"
            )));
        }
        if bytes.len() < len {
            return Err(Error::malformed(format!(
                "access unit truncated: {len} bytes declared, {} available",
                bytes.len()
            )));
        }
        let au = &bytes[..len];

        let mut pos = 4;
        let mut major_sync = None;
        let config = if MajorSync::pattern_at(&au[4..]).is_some() {
            let ms = MajorSync::parse(&au[4..])?;
            pos += ms.len_bytes;
            let cfg = StreamConfig::from_major_sync(&ms)?;
            major_sync = Some(ms);
            cfg
        } else {
            config.cloned().ok_or_else(|| {
                Error::malformed("access unit without a major sync before the first major sync")
            })?
        };

        let dir_start = pos;
        let mut reader = BitReader::new(au);
        reader.seek(dir_start * 8)?;
        let mut directory = Vec::with_capacity(usize::from(config.substreams));
        for _ in 0..config.substreams {
            let extra_word = reader.read_bool()?;
            let restart_nonexistent = reader.read_bool()?;
            let crc_present = reader.read_bool()?;
            let reserved = reader.read_bool()?;
            let end_ptr_words = reader.read(12)? as u16;
            let (drc_gain_update, drc_time_update) = if extra_word {
                let gain = reader.read_signed(9)? as i16;
                let time = reader.read(3)? as u8;
                reader.skip(4)?;
                (Some(gain), Some(time))
            } else {
                (None, None)
            };
            directory.push(DirectoryEntry {
                extra_word,
                restart_nonexistent,
                crc_present,
                reserved,
                end_ptr_words,
                drc_gain_update,
                drc_time_update,
            });
        }
        let segments_start = reader.position() / 8;
        let header_parity_ok =
            fold_nibble(xor_bytes(&au[..4]) ^ xor_bytes(&au[dir_start..segments_start])) == 0xF;

        let mut prev_end = 0usize;
        for (i, entry) in directory.iter().enumerate() {
            let end = usize::from(entry.end_ptr_words) * 2;
            if end < prev_end {
                return Err(Error::malformed(format!(
                    "substream {i} ends before the previous one"
                )));
            }
            if segments_start + end > len {
                return Err(Error::malformed(format!(
                    "substream {i} ends at byte {} beyond the access unit ({len} bytes)",
                    segments_start + end
                )));
            }
            prev_end = end;
        }
        let segments_end = segments_start + prev_end;

        let extra = if len - segments_end >= 2 {
            Some(ExtraData::parse(&au[segments_end..], config.flags)?)
        } else {
            None
        };

        Ok((
            Self {
                header,
                major_sync,
                directory,
                header_parity_ok,
                segments_start,
                extra,
            },
            config,
        ))
    }

    /// Byte range of substream `index` within the access unit.
    #[must_use]
    pub fn segment_range(&self, index: usize) -> Range<usize> {
        let start = if index == 0 {
            self.segments_start
        } else {
            self.segments_start + usize::from(self.directory[index - 1].end_ptr_words) * 2
        };
        let end = self.segments_start + usize::from(self.directory[index].end_ptr_words) * 2;
        start..end
    }

    /// Byte offset where the last segment ends.
    #[must_use]
    pub fn segments_end(&self) -> usize {
        self.directory.last().map_or(self.segments_start, |e| {
            self.segments_start + usize::from(e.end_ptr_words) * 2
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{atmos_major_sync, build_au};

    #[test]
    fn frames_a_major_sync_unit_and_a_minor_unit() {
        let ms = atmos_major_sync();
        let seg0 = [0xAAu8; 6];
        let seg1 = [0xBBu8; 4];
        let seg2 = [0xCCu8; 2];
        let seg3 = [0xDDu8; 8];
        let au1 = build_au(Some(&ms), &[&seg0, &seg1, &seg2, &seg3], &[], 0x1234);
        let (unit, config) = AccessUnit::parse(&au1, None).unwrap();
        assert_eq!(unit.header.input_timing, 0x1234);
        assert_eq!(unit.header.length_bytes(), au1.len());
        assert!(unit.major_sync.is_some());
        assert!(unit.header_parity_ok);
        assert_eq!(unit.directory.len(), 4);
        assert_eq!(unit.segments_start, 4 + ms.len() + 8);
        assert_eq!(&au1[unit.segment_range(0)], &seg0);
        assert_eq!(&au1[unit.segment_range(1)], &seg1);
        assert_eq!(&au1[unit.segment_range(3)], &seg3);
        assert_eq!(unit.segments_end(), au1.len());
        assert!(unit.extra.is_none());
        assert_eq!(config.substreams, 4);
        assert_eq!(config.samples_per_au, 40);

        let au2 = build_au(None, &[&seg0, &seg1, &seg2, &seg3], &[0, 0, 0, 0], 0x1262);
        let (unit2, _) = AccessUnit::parse(&au2, Some(&config)).unwrap();
        assert!(unit2.major_sync.is_none());
        assert!(unit2.header_parity_ok);
        assert!(unit2.directory.iter().all(|e| e.restart_nonexistent));
        assert_eq!(
            unit2.extra.as_ref().unwrap().kind,
            crate::extra::ExtraKind::Padding
        );
        assert!(matches!(
            AccessUnit::parse(&au2, None),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn corrupted_parity_and_bad_pointers_are_reported() {
        let ms = atmos_major_sync();
        let seg = [0x11u8; 4];
        let mut au = build_au(Some(&ms), &[&seg, &seg, &seg, &seg], &[], 1);
        au[2] ^= 0x01; // input timing byte: header parity breaks
        let (unit, _) = AccessUnit::parse(&au, None).unwrap();
        assert!(!unit.header_parity_ok);

        let mut au = build_au(Some(&ms), &[&seg, &seg, &seg, &seg], &[], 1);
        // make the last end pointer exceed the access unit
        let dir = 4 + ms.len() + 6;
        au[dir] = 0x0F;
        au[dir + 1] = 0xFF;
        assert!(matches!(
            AccessUnit::parse(&au, None),
            Err(Error::Malformed(_))
        ));
        assert!(AccessUnit::parse(&au[..10], None).is_err());
    }
}
