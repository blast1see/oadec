//! Presentations and the substreams they are decoded from.
//!
//! A TrueHD stream carries up to four presentations: 2-channel (substream 0),
//! 6-channel (substreams 0–1), 8-channel (substreams 0–2) and the 16-channel object
//! presentation (substream 3 together with the substreams `extended_substream_info`
//! names). `substream_info` bits 2–7 and the low two bits of
//! `extended_substream_info` say which presentations exist and how they are built.

/// Number of presentations a stream can carry.
pub const MAX_PRESENTATIONS: usize = 4;

/// How a presentation relates to the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationKind {
    /// Decoded from its own substream (plus the ones below it).
    Independent,
    /// Decoded from its own substream, and a higher presentation also uses it as an
    /// input (this presentation is a downmix of that one).
    DownmixOf(usize),
    /// Not carried: identical to the lower presentation named.
    CopyOf(usize),
    /// The stream does not declare it.
    Invalid,
}

/// Substream masks of the four presentations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationMap {
    masks: [u8; MAX_PRESENTATIONS],
}

impl PresentationMap {
    /// Derives the map from `substream_info` and `extended_substream_info`.
    #[must_use]
    pub const fn from_substream_info(substream_info: u8, extended_substream_info: u8) -> Self {
        Self {
            masks: [
                1,
                (substream_info >> 2) & 3,
                (substream_info >> 4) & 7,
                ((substream_info >> 4) & 8) | (7 ^ (7 >> (extended_substream_info & 3))),
            ],
        }
    }

    /// Bit mask of the substreams presentation `index` is decoded from (bit i = substream i).
    #[must_use]
    pub const fn mask(&self, index: usize) -> u8 {
        if index < MAX_PRESENTATIONS {
            self.masks[index]
        } else {
            0
        }
    }

    /// Classifies presentation `index`.
    #[must_use]
    pub fn kind(&self, index: usize) -> PresentationKind {
        if index >= MAX_PRESENTATIONS {
            return PresentationKind::Invalid;
        }
        let mask = self.masks[index];
        if mask >> index != 0 {
            let downmix_of = (index + 1..MAX_PRESENTATIONS)
                .find(|&i| self.masks[i] >> i != 0 && (self.masks[i] >> index) & 1 != 0);
            return match downmix_of {
                Some(i) => PresentationKind::DownmixOf(i),
                None => PresentationKind::Independent,
            };
        }
        match (0..index).rev().find(|&i| mask >> i != 0) {
            Some(i) => PresentationKind::CopyOf(i),
            None => PresentationKind::Invalid,
        }
    }

    /// Whether presentation `index` is carried (independent or a downmix).
    #[must_use]
    pub fn is_carried(&self, index: usize) -> bool {
        matches!(
            self.kind(index),
            PresentationKind::Independent | PresentationKind::DownmixOf(_)
        )
    }

    /// The highest carried presentation.
    #[must_use]
    pub fn max_carried(&self) -> Option<usize> {
        (0..MAX_PRESENTATIONS).rev().find(|&i| self.is_carried(i))
    }

    /// Union of the substream masks of the presentations selected in `wanted`.
    #[must_use]
    pub fn substreams_for(&self, wanted: [bool; MAX_PRESENTATIONS]) -> u8 {
        let mut union = 0;
        for (i, &w) in wanted.iter().enumerate() {
            if w {
                union |= self.masks[i];
            }
        }
        union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atmos_stream_has_four_presentations() {
        let map = PresentationMap::from_substream_info(0xFC, 3);
        assert_eq!(
            [map.mask(0), map.mask(1), map.mask(2), map.mask(3)],
            [1, 3, 7, 15]
        );
        assert_eq!(map.kind(0), PresentationKind::DownmixOf(1));
        assert_eq!(map.kind(1), PresentationKind::DownmixOf(2));
        assert_eq!(map.kind(2), PresentationKind::DownmixOf(3));
        assert_eq!(map.kind(3), PresentationKind::Independent);
        assert_eq!(map.max_carried(), Some(3));
        assert_eq!(map.substreams_for([true, false, false, true]), 15);
        assert_eq!(map.mask(4), 0);
        assert_eq!(map.kind(4), PresentationKind::Invalid);
    }

    #[test]
    fn extended_info_selects_the_inputs_of_the_object_presentation() {
        assert_eq!(
            PresentationMap::from_substream_info(0xC8, 1).mask(3),
            0b1100
        );
        assert_eq!(
            PresentationMap::from_substream_info(0xE8, 2).mask(3),
            0b1110
        );
    }

    #[test]
    fn two_substream_stream_marks_the_missing_presentations() {
        // 6ch over substreams 0-1, no 8ch (copy of 6ch), no 16ch.
        let map = PresentationMap::from_substream_info(0x2C, 0);
        assert_eq!(map.mask(1), 3);
        assert_eq!(map.mask(2), 2);
        assert_eq!(map.kind(0), PresentationKind::DownmixOf(1));
        assert_eq!(map.kind(1), PresentationKind::Independent);
        assert_eq!(map.kind(2), PresentationKind::CopyOf(1));
        assert_eq!(map.kind(3), PresentationKind::Invalid);
        assert_eq!(map.max_carried(), Some(1));
    }
}
