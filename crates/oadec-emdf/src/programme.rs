//! The two small EMDF payloads every E-AC-3 Atmos stream carries beside its
//! objects: programme loudness and programme information (TS 102 366 clauses
//! H.3.1 and H.3.2).
//!
//! They describe the programme and steer nothing in the decode, so a payload that
//! does not parse is reported and never judged. What they are worth is what a
//! second encode would otherwise guess: the DRC profile of a stream is written here
//! and nowhere else.

use oadec_bits::{BitError, BitReader};

/// `emdf_payload_id` of the programme loudness data.
pub const PAYLOAD_ID_LOUDNESS: u32 = 1;
/// `emdf_payload_id` of the programme information.
pub const PAYLOAD_ID_PROGRAMME_INFORMATION: u32 = 2;

/// A DRC profile (table H.3.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrcProfile {
    /// A constant gain of 0 dB.
    None,
    FilmStandard,
    FilmLight,
    MusicStandard,
    MusicLight,
    Speech,
    /// 6 or 7.
    Reserved(u8),
}

impl DrcProfile {
    const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::None,
            1 => Self::FilmStandard,
            2 => Self::FilmLight,
            3 => Self::MusicStandard,
            4 => Self::MusicLight,
            5 => Self::Speech,
            other => Self::Reserved(other),
        }
    }

    /// The name Dolby's encoders take for it in `drc_profile_line` and
    /// `drc_profile_rf`; a reserved code has none.
    #[must_use]
    pub const fn encoder_name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FilmStandard => "film_standard",
            Self::FilmLight => "film_light",
            Self::MusicStandard => "music_standard",
            Self::MusicLight => "music_light",
            Self::Speech => "speech",
            Self::Reserved(_) => "reserved",
        }
    }
}

/// What was done to the audio before encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preprocessing {
    /// `suratten`: surround attenuation applied.
    pub surround_attenuation: bool,
    /// `ph90filt`: 90 degree phase shift applied.
    pub phase_shift_90: bool,
    /// `lfefilt`: LFE filter applied.
    pub lfe_filter: bool,
    /// `lfemonlevcod`, table H.3.12.
    pub lfe_monitor_level: u8,
}

/// The DRC profiles of the two sets of gain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrcProfiles {
    /// `dynrngprof`: the profile of `dynrng`, which line mode plays.
    pub line: DrcProfile,
    /// `comprprof`: the profile of `compr`, which RF mode plays.
    pub rf: DrcProfile,
}

/// `programme_information()`, clause H.3.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgrammeInformation {
    pub version: u8,
    /// `activechan`.
    pub active_channels: Option<u16>,
    /// `dmixtyp`: the programme has been downmixed, this way.
    pub downmix: Option<u8>,
    /// `upmixtyp`: the programme has been upmixed, this way.
    pub upmix: Option<u8>,
    pub preprocessing: Option<Preprocessing>,
    pub drc: Option<DrcProfiles>,
    /// `specprocstartf`, `specprocendf`.
    pub spectral_processing: Option<(u8, u8)>,
    /// `chancplstartf`, `chancplendf`.
    pub channel_coupling: Option<(u8, u8)>,
    /// `enhncrng`.
    pub enhancement_range: Option<u8>,
    /// Nothing but zero fill followed the syntax, inside one byte.
    pub size_ok: bool,
}

/// Whether what is left of a payload is the fill the syntax ends with.
fn ends_in_fill(r: &mut BitReader<'_>) -> Result<bool, BitError> {
    let left = r.remaining();
    if left >= 8 {
        return Ok(false);
    }
    Ok(left == 0 || r.read(left as u32)? == 0)
}

impl ProgrammeInformation {
    /// Parses a payload of this id.
    ///
    /// # Errors
    /// A payload shorter than its syntax.
    pub fn parse(bytes: &[u8]) -> Result<Self, BitError> {
        let mut reader = BitReader::new(bytes);
        let r = &mut reader;
        let mut version = r.read(2)? as u8;
        if version == 3 {
            version += r.read(4)? as u8;
        }
        let active_channels = if r.read_bool()? {
            Some(r.read(16)? as u16)
        } else {
            None
        };
        let (mut downmix, mut upmix) = (None, None);
        if r.read_bool()? {
            downmix = Some(r.read(4)? as u8);
        } else if r.read_bool()? {
            upmix = Some(r.read(4)? as u8);
        }
        let preprocessing = if r.read_bool()? {
            Some(Preprocessing {
                surround_attenuation: r.read_bool()?,
                phase_shift_90: r.read_bool()?,
                lfe_filter: r.read_bool()?,
                lfe_monitor_level: r.read(2)? as u8,
            })
        } else {
            None
        };
        let drc = if r.read_bool()? {
            Some(DrcProfiles {
                line: DrcProfile::from_code(r.read(3)? as u8),
                rf: DrcProfile::from_code(r.read(3)? as u8),
            })
        } else {
            None
        };
        let spectral_processing = if r.read_bool()? {
            Some((r.read(3)? as u8, r.read(3)? as u8))
        } else {
            None
        };
        let channel_coupling = if r.read_bool()? {
            Some((r.read(5)? as u8, r.read(5)? as u8))
        } else {
            None
        };
        let enhancement_range = if r.read_bool()? {
            Some(r.read(2)? as u8)
        } else {
            None
        };
        let size_ok = ends_in_fill(r)?;
        Ok(Self {
            version,
            active_channels,
            downmix,
            upmix,
            preprocessing,
            drc,
            spectral_processing,
            channel_coupling,
            enhancement_range,
            size_ok,
        })
    }
}

/// How the loudness of the programme was corrected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoudnessCorrection {
    /// `loudcorrdialgat`: dialogue-gated correction applied.
    pub dialogue_gated: bool,
    /// `loudcorrtyp`: 0 file-based, 1 real-time.
    pub real_time: bool,
}

/// A programme boundary (`prgmbndy`, `end_or_start`, `prgmbndyoffst`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgrammeBoundary {
    /// Syncframes to the boundary, a power of two.
    pub frames: u32,
    /// The boundary is the end of a programme, not the start of one.
    pub end_of_programme: bool,
    pub sample_offset: Option<u16>,
}

/// The fields a version 1 payload adds after its first fill.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LoudnessExtension {
    /// `loudcorrdialgattyp`.
    pub correction_gating: Option<u8>,
    /// `loudspchdialgattyp`.
    pub speech_gating: Option<u8>,
    pub max_short_term_lufs: Option<f32>,
    pub max_true_peak_dbtp: Option<f32>,
    pub loudness_range_lu: Option<f32>,
    /// `lrapractyp`.
    pub loudness_range_practice: Option<u8>,
    pub momentary_lufs: Option<f32>,
    pub max_momentary_lufs: Option<f32>,
}

/// `loudness_data()`, clause H.3.1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessData {
    pub version: u8,
    /// `dialchan`: dialogue in L, R and C, in that order from the top bit.
    pub dialogue_channels: Option<u8>,
    /// `loudpractyp`, table H.3.2; 0 is "not indicated".
    pub practice: u8,
    pub correction: Option<LoudnessCorrection>,
    /// `loudrelgat`, with its high-resolution or extended form when present.
    pub relative_gated_lufs: Option<f32>,
    /// `loudspchgat`, likewise.
    pub speech_gated_lufs: Option<f32>,
    /// `loudstrm3s`, likewise.
    pub short_term_lufs: Option<f32>,
    /// `truepk`, likewise.
    pub true_peak_dbtp: Option<f32>,
    /// `dmixloudoffst`; 31 is reserved and reads as 0.
    pub downmix_offset_lu: Option<f32>,
    pub boundary: Option<ProgrammeBoundary>,
    pub extension: Option<LoudnessExtension>,
    /// Nothing but zero fill followed the syntax, inside one byte.
    pub size_ok: bool,
}

/// `(code - 1024) / 10`, the scale of every 11-bit field.
fn extended(code: u32) -> f32 {
    (code as f32 - 1024.0) / 10.0
}

/// Tables H.3.3 and H.3.4: -0.4 to -0.1 and +0.1 to +0.4, no zero.
fn high_resolution(code: u32) -> f32 {
    let steps = code as i32 - if code < 4 { 4 } else { 3 };
    steps as f32 / 10.0
}

/// A flag and the field it announces.
fn flagged(r: &mut BitReader<'_>, bits: u32) -> Result<Option<u32>, BitError> {
    Ok(if r.read_bool()? {
        Some(r.read(bits)?)
    } else {
        None
    })
}

impl LoudnessData {
    /// Parses a payload of this id.
    ///
    /// Version 0, which every stream measured carries, ends at the first fill;
    /// version 1 goes on after it.
    ///
    /// # Errors
    /// A payload shorter than its syntax.
    pub fn parse(bytes: &[u8]) -> Result<Self, BitError> {
        let mut reader = BitReader::new(bytes);
        let r = &mut reader;
        let mut version = r.read(2)? as u8;
        if version == 3 {
            version += r.read(4)? as u8;
        }
        let dialogue_channels = flagged(r, 3)?.map(|v| v as u8);
        let practice = r.read(4)? as u8;
        let correction = if practice == 0 {
            None
        } else {
            Some(LoudnessCorrection {
                dialogue_gated: r.read_bool()?,
                real_time: r.read_bool()?,
            })
        };
        let relative = flagged(r, 7)?;
        let speech = flagged(r, 7)?;
        let short_term = flagged(r, 8)?;
        let true_peak = flagged(r, 8)?;
        let downmix_offset_lu = flagged(r, 5)?.map(|v| {
            if v == 31 {
                0.0
            } else {
                (v as f32 - 15.0) / 2.0
            }
        });
        let boundary = if r.read_bool()? {
            // shifted before each read, so 001 is 8 and not 4
            let mut frames = 1u32;
            loop {
                frames = frames.saturating_mul(2);
                if r.read_bool()? {
                    break;
                }
            }
            let end_of_programme = r.read_bool()?;
            Some(ProgrammeBoundary {
                frames,
                end_of_programme,
                sample_offset: flagged(r, 11)?.map(|v| v as u16),
            })
        } else {
            None
        };
        // the half-unit fields, in the units of their tables
        let half = |code: u32, floor: f32| code as f32 / 2.0 + floor;
        let mut relative_gated_lufs = relative.map(|v| half(v, -58.0));
        let mut speech_gated_lufs = speech.map(|v| half(v, -58.0));
        let mut short_term_lufs = short_term.map(|v| half(v, -116.0));
        let mut true_peak_dbtp = true_peak.map(|v| half(v, -116.0));

        // to the byte boundary
        let fill = r.remaining() % 8;
        let fill_zero = fill == 0 || r.read(fill as u32)? == 0;
        if version == 0 {
            return Ok(Self {
                version,
                dialogue_channels,
                practice,
                correction,
                relative_gated_lufs,
                speech_gated_lufs,
                short_term_lufs,
                true_peak_dbtp,
                downmix_offset_lu,
                boundary,
                extension: None,
                size_ok: fill_zero && r.remaining() == 0,
            });
        }

        let refine = |r: &mut BitReader<'_>, present: bool, value: &mut Option<f32>| {
            if present && let Some(code) = flagged(r, 3)? {
                *value = value.map(|v| v + high_resolution(code));
            }
            Ok::<(), BitError>(())
        };
        refine(r, relative.is_some(), &mut relative_gated_lufs)?;
        refine(r, speech.is_some(), &mut speech_gated_lufs)?;
        refine(r, short_term.is_some(), &mut short_term_lufs)?;
        refine(r, true_peak.is_some(), &mut true_peak_dbtp)?;
        let mut x = LoudnessExtension::default();
        if correction.is_some_and(|c| c.dialogue_gated) {
            x.correction_gating = Some(r.read(3)? as u8);
        }
        if relative.is_none() {
            relative_gated_lufs = flagged(r, 11)?.map(extended);
        }
        if speech.is_none() {
            speech_gated_lufs = flagged(r, 11)?.map(extended);
        }
        if speech_gated_lufs.is_some() {
            x.speech_gating = Some(r.read(3)? as u8);
        }
        if short_term.is_none() {
            short_term_lufs = flagged(r, 11)?.map(extended);
        }
        if true_peak.is_none() {
            true_peak_dbtp = flagged(r, 11)?.map(extended);
        }
        x.max_short_term_lufs = flagged(r, 11)?.map(extended);
        x.max_true_peak_dbtp = flagged(r, 11)?.map(extended);
        if r.read_bool()? {
            x.loudness_range_lu = Some(r.read(10)? as f32 / 10.0);
            x.loudness_range_practice = Some(r.read(3)? as u8);
        }
        x.momentary_lufs = flagged(r, 11)?.map(extended);
        x.max_momentary_lufs = flagged(r, 11)?.map(extended);
        let size_ok = fill_zero && ends_in_fill(r)?;
        Ok(Self {
            version,
            dialogue_channels,
            practice,
            correction,
            relative_gated_lufs,
            speech_gated_lufs,
            short_term_lufs,
            true_peak_dbtp,
            downmix_offset_lu,
            boundary,
            extension: Some(x),
            size_ok,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oadec_bits::BitWriter;

    /// The bytes are what real streams carry, read from the heads of three titles.
    #[test]
    fn programme_information_reads_the_drc_profiles_of_real_streams() {
        for (bytes, line) in [
            ([0x05, 0x90, 0x00], DrcProfile::None),
            ([0x05, 0x94, 0x80], DrcProfile::FilmLight),
            ([0x05, 0x99, 0x00], DrcProfile::MusicLight),
        ] {
            let p = ProgrammeInformation::parse(&bytes).unwrap();
            assert_eq!(p.drc, Some(DrcProfiles { line, rf: line }));
            assert_eq!(p.version, 0);
            assert_eq!(
                p.preprocessing,
                Some(Preprocessing {
                    surround_attenuation: false,
                    phase_shift_90: true,
                    lfe_filter: true,
                    lfe_monitor_level: 0,
                })
            );
            assert_eq!(p.active_channels, None);
            assert_eq!((p.downmix, p.upmix), (None, None));
            assert!(p.size_ok, "21 bits and three of fill");
        }
    }

    #[test]
    fn programme_information_keeps_the_two_profiles_apart() {
        // dynrngprof 001, comprprof 101, with nothing before them but the flags
        let mut w = BitWriter::new();
        w.write(0, 2);
        w.write(0, 1); // no active channels
        w.write(0, 1); // not downmixed
        w.write(0, 1); // not upmixed
        w.write(0, 1); // no preprocessing
        w.write(1, 1);
        w.write(1, 3);
        w.write(5, 3);
        w.write(0, 3); // no spectral, no coupling, no enhancement
        let p = ProgrammeInformation::parse(&w.finish()).unwrap();
        assert_eq!(
            p.drc,
            Some(DrcProfiles {
                line: DrcProfile::FilmStandard,
                rf: DrcProfile::Speech,
            })
        );
    }

    #[test]
    fn programme_information_reads_its_rarer_fields_in_order() {
        let mut w = BitWriter::new();
        w.write(0, 2);
        w.write(1, 1);
        w.write(0xABCD, 16);
        w.write(1, 1); // downmixed
        w.write(2, 4);
        w.write(0, 1); // no preprocessing
        w.write(0, 1); // no DRC
        w.write(1, 1);
        w.write(3, 3);
        w.write(6, 3);
        w.write(1, 1);
        w.write(17, 5);
        w.write(29, 5);
        w.write(1, 1);
        w.write(2, 2);
        let p = ProgrammeInformation::parse(&w.finish()).unwrap();
        assert_eq!(p.active_channels, Some(0xABCD));
        assert_eq!(p.downmix, Some(2));
        assert_eq!(p.upmix, None);
        assert_eq!(p.spectral_processing, Some((3, 6)));
        assert_eq!(p.channel_coupling, Some((17, 29)));
        assert_eq!(p.enhancement_range, Some(2));
        assert!(p.size_ok);

        let mut w = BitWriter::new();
        w.write(0, 2);
        w.write(0, 1);
        w.write(0, 1); // not downmixed
        w.write(1, 1); // upmixed
        w.write(9, 4);
        w.write(0, 5);
        let p = ProgrammeInformation::parse(&w.finish()).unwrap();
        assert_eq!((p.downmix, p.upmix), (None, Some(9)));
    }

    #[test]
    fn a_reserved_drc_code_is_kept_and_a_long_payload_is_flagged() {
        let mut w = BitWriter::new();
        w.write(0, 6);
        w.write(1, 1);
        w.write(6, 3);
        w.write(7, 3);
        w.write(0, 3);
        w.write(0xFF, 8); // a byte past the syntax
        let p = ProgrammeInformation::parse(&w.finish()).unwrap();
        assert_eq!(
            p.drc,
            Some(DrcProfiles {
                line: DrcProfile::Reserved(6),
                rf: DrcProfile::Reserved(7),
            })
        );
        assert!(!p.size_ok);
    }

    /// Real streams again: a version 0 payload ends at its first fill.
    #[test]
    fn loudness_reads_the_payloads_of_real_streams() {
        let p = LoudnessData::parse(&[0x03, 0x00]).unwrap();
        assert_eq!(p.practice, 1, "ATSC A/85");
        assert_eq!(
            p.correction,
            Some(LoudnessCorrection {
                dialogue_gated: true,
                real_time: false,
            })
        );
        assert_eq!(p.relative_gated_lufs, None);
        assert_eq!(p.true_peak_dbtp, None);
        assert!(p.size_ok);

        let p = LoudnessData::parse(&[0x03, 0x5B, 0x00]).unwrap();
        assert_eq!(p.relative_gated_lufs, Some(-31.0));
        assert_eq!(p.speech_gated_lufs, None);
        assert!(p.size_ok);

        let p = LoudnessData::parse(&[0x02, 0x2D, 0x80]).unwrap();
        assert!(!p.correction.unwrap().dialogue_gated);
        assert_eq!(p.speech_gated_lufs, Some(-31.0));
        assert_eq!(p.relative_gated_lufs, None);
        assert!(p.size_ok);

        let p = LoudnessData::parse(&[0x03, 0x0F, 0x28]).unwrap();
        assert_eq!(p.true_peak_dbtp, Some(-1.5));
        assert!(p.size_ok);
    }

    #[test]
    fn loudness_scales_its_short_fields_as_the_tables_say() {
        let mut w = BitWriter::new();
        w.write(0, 2);
        w.write(1, 1);
        w.write(0b101, 3);
        w.write(0, 4); // practice not indicated, so no correction bits
        w.write(1, 1);
        w.write(0, 7);
        w.write(1, 1);
        w.write(127, 7);
        w.write(1, 1);
        w.write(255, 8);
        w.write(1, 1);
        w.write(0, 8);
        w.write(1, 1);
        w.write(30, 5);
        w.write(1, 1); // a boundary
        w.write(0, 1);
        w.write(0, 1);
        w.write(1, 1); // the read that ends it; clause H.3.1.3.18 makes 001 eight frames
        w.write(1, 1); // the end of a programme
        w.write(1, 1);
        w.write(1234, 11);
        let p = LoudnessData::parse(&w.finish()).unwrap();
        assert_eq!(p.dialogue_channels, Some(0b101));
        assert_eq!(p.correction, None);
        assert_eq!(p.relative_gated_lufs, Some(-58.0));
        assert_eq!(p.speech_gated_lufs, Some(5.5));
        assert_eq!(p.short_term_lufs, Some(11.5));
        assert_eq!(p.true_peak_dbtp, Some(-116.0));
        assert_eq!(p.downmix_offset_lu, Some(7.5));
        assert_eq!(
            p.boundary,
            Some(ProgrammeBoundary {
                frames: 8,
                end_of_programme: true,
                sample_offset: Some(1234),
            })
        );
        assert!(p.size_ok);
    }

    #[test]
    fn the_reserved_downmix_offset_reads_as_none() {
        let mut w = BitWriter::new();
        w.write(0, 2);
        w.write(0, 1);
        w.write(0, 4);
        w.write(0, 4); // no relative, speech, short-term or peak
        w.write(1, 1);
        w.write(31, 5);
        w.write(0, 1);
        assert_eq!(
            LoudnessData::parse(&w.finish()).unwrap().downmix_offset_lu,
            Some(0.0)
        );
    }

    /// Version 1 goes on past the first fill, with fields that depend on which
    /// short ones were there.
    #[test]
    fn a_version_one_payload_refines_and_extends() {
        let mut w = BitWriter::new();
        w.write(1, 2);
        w.write(0, 1);
        w.write(2, 4); // EBU R128
        w.write(1, 1); // dialogue gated
        w.write(0, 1);
        w.write(1, 1);
        w.write(54, 7); // relative -31
        w.write(0, 1); // no speech
        w.write(0, 1); // no short-term
        w.write(1, 1);
        w.write(232, 8); // peak 0
        w.write(0, 1);
        w.write(0, 1);
        let used = 2 + 1 + 4 + 2 + 8 + 1 + 1 + 9 + 1 + 1;
        w.write(0, (8 - used % 8) % 8);
        w.write(1, 1);
        w.write(0b001, 3); // relative -0.3
        w.write(1, 1);
        w.write(0b111, 3); // peak +0.4
        w.write(3, 3); // correction gating
        w.write(1, 1);
        w.write(1024 - 240, 11); // speech -24.0, extended
        w.write(4, 3); // speech gating, because speech is there
        w.write(0, 1); // no extended short-term
        w.write(1, 1);
        w.write(1024 - 100, 11); // max short-term -10.0
        w.write(1, 1);
        w.write(1024 + 5, 11); // max peak +0.5
        w.write(1, 1);
        w.write(123, 10); // range 12.3
        w.write(1, 3);
        w.write(1, 1);
        w.write(1024 - 200, 11); // momentary -20.0
        w.write(0, 1);
        let p = LoudnessData::parse(&w.finish()).unwrap();
        assert_eq!(p.version, 1);
        assert_eq!(p.practice, 2);
        assert!((p.relative_gated_lufs.unwrap() - -31.3).abs() < 1e-5);
        assert!((p.true_peak_dbtp.unwrap() - 0.4).abs() < 1e-5);
        assert_eq!(p.speech_gated_lufs, Some(-24.0));
        let x = p.extension.unwrap();
        assert_eq!(x.correction_gating, Some(3));
        assert_eq!(x.speech_gating, Some(4));
        assert_eq!(x.max_short_term_lufs, Some(-10.0));
        assert_eq!(x.max_true_peak_dbtp, Some(0.5));
        assert_eq!(x.loudness_range_lu, Some(12.3));
        assert_eq!(x.loudness_range_practice, Some(1));
        assert_eq!(x.momentary_lufs, Some(-20.0));
        assert_eq!(x.max_momentary_lufs, None);
        assert!(p.size_ok);
    }

    #[test]
    fn the_high_resolution_table_has_no_zero() {
        let steps: Vec<i32> = (0..8)
            .map(|c| (high_resolution(c) * 10.0).round() as i32)
            .collect();
        assert_eq!(steps, [-4, -3, -2, -1, 1, 2, 3, 4]);
    }

    #[test]
    fn a_payload_shorter_than_its_syntax_is_an_error() {
        assert!(LoudnessData::parse(&[]).is_err());
        assert!(ProgrammeInformation::parse(&[0x05]).is_err());
    }
}
