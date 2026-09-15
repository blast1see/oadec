//! The `channel_meaning` block of a major sync and channel labelling.

use core::fmt;

use oadec_bits::BitReader;

use crate::error::Result;

/// Presentation-level metadata carried by every major sync (64 bits) plus the
/// optional extra channel meaning block that describes the 16-channel presentation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChannelMeaning {
    /// `heavy_drc_start_up_gain` (signed 6 bits).
    pub heavy_drc_start_up_gain: i8,
    /// `2ch_control_enabled`.
    pub twoch_control_enabled: bool,
    /// `6ch_control_enabled`.
    pub sixch_control_enabled: bool,
    /// `8ch_control_enabled`.
    pub eightch_control_enabled: bool,
    /// Reserved bit.
    pub reserved1: bool,
    /// `drc_start_up_gain` (signed 7 bits).
    pub drc_start_up_gain: i8,
    /// `2ch_dialogue_norm` (6 bits).
    pub twoch_dialogue_norm: u8,
    /// `2ch_mix_level` (6 bits).
    pub twoch_mix_level: u8,
    /// `6ch_dialogue_norm` (5 bits).
    pub sixch_dialogue_norm: u8,
    /// `6ch_mix_level` (6 bits).
    pub sixch_mix_level: u8,
    /// `6ch_source_format` (5 bits).
    pub sixch_source_format: u8,
    /// `8ch_dialogue_norm` (5 bits).
    pub eightch_dialogue_norm: u8,
    /// `8ch_mix_level` (6 bits).
    pub eightch_mix_level: u8,
    /// `8ch_source_format` (6 bits).
    pub eightch_source_format: u8,
    /// Reserved bit.
    pub reserved2: bool,
    /// `extra_channel_meaning_present`.
    pub extra_present: bool,
    /// The extra channel meaning block, when present.
    pub extra: Option<ExtraChannelMeaning>,
}

/// The extra channel meaning block (16-channel presentation description).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExtraChannelMeaning {
    /// `extra_channel_meaning_length` (4 bits): the block is `(length + 1) * 16` bits.
    pub length: u8,
    /// `16ch_dialogue_norm` (5 bits).
    pub sixteench_dialogue_norm: u8,
    /// `16ch_mix_level` (6 bits).
    pub sixteench_mix_level: u8,
    /// `16ch_channel_count` (5 bits): channels of the presentation minus one.
    pub sixteench_channel_count: u8,
    /// `dyn_object_only`: no bed, dynamic objects only.
    pub dyn_object_only: bool,
    /// `lfe_present` (only when `dyn_object_only`).
    pub lfe_present: bool,
    /// `16ch_content_description` (4 bits): bit 0 bed, bit 1 ISF, bit 2 dynamic objects.
    pub content_description: u8,
    /// `chan_distribute`.
    pub chan_distribute: bool,
    /// `lfe_only`: the bed is only an LFE channel.
    pub lfe_only: bool,
    /// `16ch_channel_assignment` (10 bits, same layout as the OAMD bed assignment).
    pub sixteench_channel_assignment: u16,
    /// `16ch_intermediate_spatial_format_idx` (3 bits).
    pub isf_index: u8,
    /// `16ch_dynamic_object_count` (5 bits): dynamic objects minus one.
    pub dynamic_object_count: u8,
}

impl ExtraChannelMeaning {
    /// Whether the presentation carries a bed.
    #[must_use]
    pub const fn has_bed(&self) -> bool {
        !self.dyn_object_only && self.content_description & 1 != 0
    }

    /// Whether the presentation carries intermediate spatial format objects.
    #[must_use]
    pub const fn has_isf(&self) -> bool {
        !self.dyn_object_only && self.content_description & 2 != 0
    }

    /// Whether the presentation carries dynamic objects.
    #[must_use]
    pub const fn has_dynamic_objects(&self) -> bool {
        self.dyn_object_only || self.content_description & 4 != 0
    }

    /// Number of dynamic objects. A dynamic-object-only program does not code the
    /// count: every channel except an LFE is an object.
    #[must_use]
    pub const fn dynamic_objects(&self) -> u8 {
        if self.dyn_object_only {
            self.channels() - (self.lfe_present as u8)
        } else if self.has_dynamic_objects() {
            self.dynamic_object_count + 1
        } else {
            0
        }
    }

    /// Number of channels of the 16-channel presentation.
    #[must_use]
    pub const fn channels(&self) -> u8 {
        self.sixteench_channel_count + 1
    }

    fn parse(reader: &mut BitReader<'_>, has_16ch: bool) -> Result<Self> {
        let start = reader.position();
        let length = reader.read(4)? as u8;
        let end = start + (usize::from(length) + 1) * 16;
        let mut extra = Self {
            length,
            ..Self::default()
        };
        if has_16ch {
            extra.sixteench_dialogue_norm = reader.read(5)? as u8;
            extra.sixteench_mix_level = reader.read(6)? as u8;
            extra.sixteench_channel_count = reader.read(5)? as u8;
            extra.dyn_object_only = reader.read_bool()?;
            if extra.dyn_object_only {
                extra.lfe_present = reader.read_bool()?;
            } else {
                extra.content_description = reader.read(4)? as u8;
                if extra.content_description & 1 != 0 {
                    extra.chan_distribute = reader.read_bool()?;
                    reader.skip(1)?;
                    extra.lfe_only = reader.read_bool()?;
                    if !extra.lfe_only {
                        reader.skip(1)?;
                        extra.sixteench_channel_assignment = reader.read(10)? as u16;
                    }
                }
                if extra.content_description & 2 != 0 {
                    extra.isf_index = reader.read(3)? as u8;
                }
                if extra.content_description & 4 != 0 {
                    extra.dynamic_object_count = reader.read(5)? as u8;
                }
            }
        }
        reader.seek(end)?;
        Ok(extra)
    }
}

impl ChannelMeaning {
    /// Parses the block at the reader position. `has_16ch` says whether
    /// `substream_info` declares a 16-channel presentation (its fields are only
    /// present then).
    pub fn parse(reader: &mut BitReader<'_>, has_16ch: bool) -> Result<Self> {
        let mut cm = Self {
            heavy_drc_start_up_gain: reader.read_signed(6)? as i8,
            twoch_control_enabled: reader.read_bool()?,
            sixch_control_enabled: reader.read_bool()?,
            eightch_control_enabled: reader.read_bool()?,
            reserved1: reader.read_bool()?,
            drc_start_up_gain: reader.read_signed(7)? as i8,
            twoch_dialogue_norm: reader.read(6)? as u8,
            twoch_mix_level: reader.read(6)? as u8,
            sixch_dialogue_norm: reader.read(5)? as u8,
            sixch_mix_level: reader.read(6)? as u8,
            sixch_source_format: reader.read(5)? as u8,
            eightch_dialogue_norm: reader.read(5)? as u8,
            eightch_mix_level: reader.read(6)? as u8,
            eightch_source_format: reader.read(6)? as u8,
            reserved2: reader.read_bool()?,
            extra_present: reader.read_bool()?,
            extra: None,
        };
        if cm.extra_present {
            cm.extra = Some(ExtraChannelMeaning::parse(reader, has_16ch)?);
        }
        Ok(cm)
    }
}

/// Speaker label of a decoded channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::upper_case_acronyms)]
pub enum ChannelLabel {
    L,
    R,
    C,
    LFE,
    Ls,
    Rs,
    Lb,
    Rb,
    Tfl,
    Tfr,
    Tsl,
    Tsr,
    Tbl,
    Tbr,
    Lsc,
    Rsc,
    Cb,
    Tc,
    Lsd,
    Rsd,
    Lw,
    Rw,
    Tfc,
    LFE2,
}

impl fmt::Display for ChannelLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl ChannelLabel {
    /// Channels of the two-channel presentation.
    #[must_use]
    pub fn two_channel(mono: bool) -> Vec<Self> {
        if mono {
            vec![Self::C]
        } else {
            vec![Self::L, Self::R]
        }
    }

    /// Channels of the six-channel presentation from `6ch_decoder_channel_assignment`.
    #[must_use]
    pub fn six_channel(assignment: u8) -> Vec<Self> {
        let mut labels = Vec::new();
        for bit in 0..5 {
            if (assignment >> bit) & 1 == 1 {
                match bit {
                    0 => labels.extend([Self::L, Self::R]),
                    1 => labels.push(Self::C),
                    2 => labels.push(Self::LFE),
                    3 => labels.extend([Self::Ls, Self::Rs]),
                    _ => labels.extend([Self::Tfl, Self::Tfr]),
                }
            }
        }
        labels
    }

    /// Channels of the eight-channel presentation from
    /// `8ch_decoder_channel_assignment`; the restricted form (flags bit 11) reuses the
    /// six-channel layout with top side channels for bit 4.
    #[must_use]
    pub fn eight_channel(assignment: u16, restricted: bool) -> Vec<Self> {
        let mut labels = Vec::new();
        if restricted {
            for bit in 0..5 {
                if (assignment >> bit) & 1 == 1 {
                    match bit {
                        0 => labels.extend([Self::L, Self::R]),
                        1 => labels.push(Self::C),
                        2 => labels.push(Self::LFE),
                        3 => labels.extend([Self::Ls, Self::Rs]),
                        _ => labels.extend([Self::Tsl, Self::Tsr]),
                    }
                }
            }
            return labels;
        }
        for bit in 0..13 {
            if (assignment >> bit) & 1 == 1 {
                match bit {
                    0 => labels.extend([Self::L, Self::R]),
                    1 => labels.push(Self::C),
                    2 => labels.push(Self::LFE),
                    3 => labels.extend([Self::Ls, Self::Rs]),
                    4 => labels.extend([Self::Tfl, Self::Tfr]),
                    5 => labels.extend([Self::Lsc, Self::Rsc]),
                    6 => labels.extend([Self::Lb, Self::Rb]),
                    7 => labels.push(Self::Cb),
                    8 => labels.push(Self::Tc),
                    9 => labels.extend([Self::Lsd, Self::Rsd]),
                    10 => labels.extend([Self::Lw, Self::Rw]),
                    11 => labels.push(Self::Tfc),
                    _ => labels.push(Self::LFE2),
                }
            }
        }
        labels
    }

    /// Bed channels of the sixteen-channel presentation from
    /// `16ch_channel_assignment` (the OAMD standard bed layout).
    #[must_use]
    pub fn sixteen_channel_bed(assignment: u16) -> Vec<Self> {
        let mut labels = Vec::new();
        for bit in 0..10 {
            if (assignment >> bit) & 1 == 1 {
                match bit {
                    0 => labels.extend([Self::L, Self::R]),
                    1 => labels.push(Self::C),
                    2 => labels.push(Self::LFE),
                    3 => labels.extend([Self::Ls, Self::Rs]),
                    4 => labels.extend([Self::Lb, Self::Rb]),
                    5 => labels.extend([Self::Tfl, Self::Tfr]),
                    6 => labels.extend([Self::Tsl, Self::Tsr]),
                    7 => labels.extend([Self::Tbl, Self::Tbr]),
                    8 => labels.extend([Self::Lw, Self::Rw]),
                    _ => labels.push(Self::LFE2),
                }
            }
        }
        labels
    }

    /// Bed channels of the sixteen-channel presentation as the extra channel meaning
    /// declares them: an LFE alone for LFE-only beds and dynamic-object-only programs
    /// with an LFE, nothing for dynamic-object-only programs without one.
    #[must_use]
    pub fn sixteen_channel(extra: &ExtraChannelMeaning) -> Vec<Self> {
        if extra.dyn_object_only {
            return if extra.lfe_present {
                vec![Self::LFE]
            } else {
                Vec::new()
            };
        }
        if extra.lfe_only {
            return vec![Self::LFE];
        }
        Self::sixteen_channel_bed(extra.sixteench_channel_assignment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_bit_order() {
        assert_eq!(
            ChannelLabel::six_channel(0b01111),
            vec![
                ChannelLabel::L,
                ChannelLabel::R,
                ChannelLabel::C,
                ChannelLabel::LFE,
                ChannelLabel::Ls,
                ChannelLabel::Rs
            ]
        );
        assert_eq!(
            ChannelLabel::eight_channel(0b0_0000_0100_1111, false),
            vec![
                ChannelLabel::L,
                ChannelLabel::R,
                ChannelLabel::C,
                ChannelLabel::LFE,
                ChannelLabel::Ls,
                ChannelLabel::Rs,
                ChannelLabel::Lb,
                ChannelLabel::Rb
            ]
        );
        assert_eq!(
            ChannelLabel::eight_channel(0b10001, true),
            vec![
                ChannelLabel::L,
                ChannelLabel::R,
                ChannelLabel::Tsl,
                ChannelLabel::Tsr
            ]
        );
        assert_eq!(
            ChannelLabel::sixteen_channel_bed(0b00_0011_1111),
            vec![
                ChannelLabel::L,
                ChannelLabel::R,
                ChannelLabel::C,
                ChannelLabel::LFE,
                ChannelLabel::Ls,
                ChannelLabel::Rs,
                ChannelLabel::Lb,
                ChannelLabel::Rb,
                ChannelLabel::Tfl,
                ChannelLabel::Tfr
            ]
        );
        assert_eq!(ChannelLabel::two_channel(true), vec![ChannelLabel::C]);
        assert_eq!(ChannelLabel::LFE2.to_string(), "LFE2");
    }

    #[test]
    fn sixteen_channel_special_cases() {
        let mut extra = ExtraChannelMeaning {
            dyn_object_only: true,
            lfe_present: true,
            ..Default::default()
        };
        assert_eq!(
            ChannelLabel::sixteen_channel(&extra),
            vec![ChannelLabel::LFE]
        );
        assert!(extra.has_dynamic_objects());
        extra.lfe_present = false;
        assert!(ChannelLabel::sixteen_channel(&extra).is_empty());
        let extra = ExtraChannelMeaning {
            content_description: 0b101,
            lfe_only: true,
            dynamic_object_count: 10,
            ..Default::default()
        };
        assert_eq!(
            ChannelLabel::sixteen_channel(&extra),
            vec![ChannelLabel::LFE]
        );
        assert!(extra.has_bed());
        assert!(!extra.has_isf());
        assert_eq!(extra.dynamic_objects(), 11);
    }
}

impl ChannelLabel {
    /// Position of the label in the interchange channel order shared by WAVE
    /// format extensible masks and the FFmpeg native layouts (front left, front
    /// right, centre, LFE, back pair, front-of-centre pair, back centre, side
    /// pair, top centre, top front trio, top back trio, ...). Sorting a
    /// presentation by this index gives the order those tools use.
    #[must_use]
    pub fn interchange_index(self) -> u8 {
        match self {
            Self::L => 0,
            Self::R => 1,
            Self::C => 2,
            Self::LFE => 3,
            Self::Lb => 4,
            Self::Rb => 5,
            Self::Lsc => 6,
            Self::Rsc => 7,
            Self::Cb => 8,
            Self::Ls => 9,
            Self::Rs => 10,
            Self::Tc => 11,
            Self::Tfl => 12,
            Self::Tfc => 13,
            Self::Tfr => 14,
            Self::Tbl => 15,
            Self::Tbr => 17,
            Self::Lw => 31,
            Self::Rw => 32,
            Self::Lsd => 33,
            Self::Rsd => 34,
            Self::LFE2 => 35,
            Self::Tsl => 36,
            Self::Tsr => 37,
        }
    }

    /// The `dwChannelMask` bit of `WAVEFORMATEXTENSIBLE` for the label, or
    /// `None` for a speaker WAVE has no bit for.
    ///
    /// WAVE defines eighteen speaker bits, front left (0) to top back right
    /// (17), and the interchange index follows them that far. Past that the
    /// index is FFmpeg's channel id and not a mask bit: wide left is 31, which
    /// in a mask means `SPEAKER_ALL`, and the rest do not fit in 32 bits.
    #[must_use]
    pub fn wave_bit(self) -> Option<u8> {
        let index = self.interchange_index();
        (index <= 17).then_some(index)
    }

    /// Labels of the channels a presentation outputs, in stream order. For the
    /// object presentation only the bed channels are labelled; the remaining
    /// output channels are the dynamic objects, in order.
    #[must_use]
    pub fn presentation(ms: &crate::sync::MajorSync, presentation: usize) -> Vec<Self> {
        let fi = &ms.format_info;
        match presentation {
            0 => Self::two_channel(false),
            1 => Self::six_channel(fi.sixch_channel_assignment),
            2 => Self::eight_channel(
                fi.eightch_channel_assignment,
                ms.flags & crate::sync::FLAG_RESTRICTED_8CH != 0,
            ),
            _ => ms
                .channel_meaning
                .extra
                .as_ref()
                .map(Self::sixteen_channel)
                .unwrap_or_default(),
        }
    }

    /// Permutation that lists `labels` (stream order) in interchange order:
    /// `order[k]` is the stream index of the k-th interchange channel. Channels
    /// beyond the labelled ones (objects) keep their places after the labelled set.
    #[must_use]
    pub fn interchange_order(labels: &[Self], channels: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..labels.len().min(channels)).collect();
        order.sort_by_key(|&i| labels[i].interchange_index());
        order.extend(labels.len().min(channels)..channels);
        order
    }
}

#[cfg(test)]
mod order_tests {
    use super::ChannelLabel as L;

    #[test]
    fn seven_one_maps_back_pair_before_side_pair() {
        let labels = [L::L, L::R, L::C, L::LFE, L::Ls, L::Rs, L::Lb, L::Rb];
        assert_eq!(
            L::interchange_order(&labels, 8),
            vec![0, 1, 2, 3, 6, 7, 4, 5]
        );
        assert_eq!(
            L::interchange_order(&labels[..6], 6),
            vec![0, 1, 2, 3, 4, 5]
        );
        // objects after a one-channel bed keep their order
        assert_eq!(L::interchange_order(&[L::LFE], 4), vec![0, 1, 2, 3]);
    }

    /// WAVE names eighteen speakers, front left (bit 0) to top back right
    /// (bit 17). Every label past that has no bit, however it sorts in the
    /// interchange order: wide left used to set bit 31, which in a mask is
    /// `SPEAKER_ALL`, and the six after it fell off the 32-bit mask.
    #[test]
    fn wave_bits_stop_at_top_back_right() {
        let named = [
            (L::L, 0),
            (L::R, 1),
            (L::C, 2),
            (L::LFE, 3),
            (L::Lb, 4),
            (L::Rb, 5),
            (L::Lsc, 6),
            (L::Rsc, 7),
            (L::Cb, 8),
            (L::Ls, 9),
            (L::Rs, 10),
            (L::Tc, 11),
            (L::Tfl, 12),
            (L::Tfc, 13),
            (L::Tfr, 14),
            (L::Tbl, 15),
            (L::Tbr, 17),
        ];
        for (label, bit) in named {
            assert_eq!(label.wave_bit(), Some(bit), "{label}");
            assert_eq!(label.interchange_index(), bit, "{label}");
        }
        for label in [L::Lw, L::Rw, L::Lsd, L::Rsd, L::LFE2, L::Tsl, L::Tsr] {
            assert_eq!(label.wave_bit(), None, "{label} has no WAVE speaker bit");
        }
    }
}
