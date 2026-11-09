//! Whether a TrueHD stream could go on a disc as it is: Blu-ray, HD DVD,
//! DVD-Audio.
//!
//! Each format takes TrueHD on terms of its own, and only some of the terms
//! are public. What each row holds a stream to, and the source of it:
//!
//! * **Blu-ray**: the FBA syntax, which Dolby's high-level description names
//!   the Blu-ray one (section 1); 48, 96 or 192 kHz, and at 192 kHz no
//!   presentation over six channels (BDA white paper, version 2.5, Figure 3-4).
//!   The figure's eight channels at 48 and 96 kHz bound the 8-channel
//!   presentation, which cannot have more; the object presentation came after
//!   the figure, and Dolby's encoder passes object streams for Blu-ray.
//! * **HD DVD**: at 176.4 and 192 kHz no presentation over two channels, a
//!   bound only the messages of Dolby's own TrueHD encoder state (secondary).
//!   No public source gives the rates HD DVD takes: at 44.1, 88.2 and 176.4 kHz
//!   the row is not judged.
//! * **DVD-Audio**: the FBB syntax (Dolby, section 1); an FBA stream is not one.
//!
//! The Blu-ray and HD DVD rows also hold an FBA stream to the bounds of its
//! delivery, the buffer and rate rules of `verify`: a stream Dolby's own
//! verifier would refuse goes on neither disc.

use crate::channel::ChannelLabel;
use crate::presentation::MAX_PRESENTATIONS;
use crate::sync::{FLAG_RESTRICTED_8CH, MajorSync, SYNC_FBA, SYNC_FBB};

/// The Blu-ray figure for Dolby TrueHD ("Dolby Lossless").
pub const BDA_FIGURE: &str = "BDA, White Paper Blu-ray Disc Read-Only Format: Audio Visual Application Format Specifications for BD-ROM, version 2.5 (2011), Figure 3-4";

/// Which syntax goes on which disc.
pub const DOLBY_SCOPE: &str =
    "Dolby TrueHD (MLP) High-Level Bitstream Description (2018), section 1";

/// The delivery bounds: Dolby's section 2.7 and the verification report of
/// its encoder, as the buffer and rate rules hold a stream to them.
pub const DELIVERY: &str =
    "the buffer and rate rules of verify (oadec rules lists the source of each)";

/// The HD DVD bound.
pub const ENCODER_HD_DVD: &str =
    "secondary: the verifier messages of Dolby's TrueHD encoder (Dolby Encoding Engine 5.7.2)";

/// What no public source states.
pub const NO_SOURCE: &str = "no public source";

/// A disc format a stream is judged for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Disc {
    /// Blu-ray Disc (BD-ROM).
    BluRay,
    /// HD DVD-Video.
    HdDvd,
    /// DVD-Audio.
    DvdAudio,
}

impl Disc {
    /// Every format, in the order a report gives them.
    pub const ALL: [Self; 3] = [Self::BluRay, Self::HdDvd, Self::DvdAudio];

    /// The name the command line takes.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::BluRay => "bd",
            Self::HdDvd => "hddvd",
            Self::DvdAudio => "dvda",
        }
    }

    /// The name a report prints.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BluRay => "BD",
            Self::HdDvd => "HD DVD",
            Self::DvdAudio => "DVD-A",
        }
    }

    /// The format the command line names `id`.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.id() == id)
    }
}

/// What the rows read of a stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The stream's format sync ([`SYNC_FBA`] or [`SYNC_FBB`]).
    pub format_sync: u32,
    /// Sampling frequency, Hz.
    pub sampling_frequency: u32,
    /// The channels of each presentation the stream carries in its own right;
    /// `None` for a copy and for one the stream does not declare.
    pub presentation_channels: [Option<u8>; MAX_PRESENTATIONS],
    /// Findings of the delivery rules -- buffer and rate -- by rule name.
    pub delivery: Vec<(&'static str, u64)>,
}

impl Facts {
    /// The facts a major sync states, with no delivery findings yet.
    #[must_use]
    pub fn from_major_sync(ms: &MajorSync) -> Self {
        Self {
            format_sync: ms.format_sync,
            sampling_frequency: ms.sampling_frequency().unwrap_or(0),
            presentation_channels: presentation_channels(ms),
            delivery: Vec::new(),
        }
    }
}

/// The channels of each presentation a major sync says the stream carries in
/// its own right (independent or a downmix); a copy repeats another and is
/// `None`, as is a presentation the stream does not declare.
#[must_use]
pub fn presentation_channels(ms: &MajorSync) -> [Option<u8>; MAX_PRESENTATIONS] {
    let map = ms.presentation_map();
    let fi = &ms.format_info;
    let restricted = ms.flags & FLAG_RESTRICTED_8CH != 0;
    core::array::from_fn(|p| {
        if !map.is_carried(p) {
            return None;
        }
        let n = match p {
            0 => ChannelLabel::two_channel(false).len(),
            1 => ChannelLabel::six_channel(fi.sixch_channel_assignment).len(),
            2 => ChannelLabel::eight_channel(fi.eightch_channel_assignment, restricted).len(),
            _ => usize::from(
                ms.channel_meaning
                    .extra
                    .as_ref()
                    .map_or(0, crate::channel::ExtraChannelMeaning::channels),
            ),
        };
        Some(n as u8)
    })
}

/// One reason a row gives, and the source the bound rests on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reason {
    /// What the stream does against the format.
    pub text: String,
    /// Where the bound comes from.
    pub source: &'static str,
}

/// A row's verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing the row knows of keeps the stream off the disc.
    Yes,
    /// The stream breaks the format's bounds.
    No(Vec<Reason>),
    /// The row cannot say: what it would need is in no public source.
    Unjudged(Reason),
}

impl Verdict {
    /// The word a report prints.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No(_) => "no",
            Self::Unjudged(_) => "unjudged",
        }
    }

    /// Whether the stream fits.
    #[must_use]
    pub const fn is_yes(&self) -> bool {
        matches!(self, Self::Yes)
    }

    /// The reasons behind a verdict other than yes.
    #[must_use]
    pub fn reasons(&self) -> &[Reason] {
        match self {
            Self::Yes => &[],
            Self::No(r) => r,
            Self::Unjudged(r) => core::slice::from_ref(r),
        }
    }
}

/// The verdict for one format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The format.
    pub disc: Disc,
    /// What the row found.
    pub verdict: Verdict,
}

/// A sampling frequency in kHz as a report writes it: 48, 44.1, 176.4.
fn khz(fs: u32) -> String {
    if fs.is_multiple_of(1000) {
        format!("{} kHz", fs / 1000)
    } else {
        format!("{:.1} kHz", f64::from(fs) / 1000.0)
    }
}

/// The widest presentation over `limit` channels, if there is one.
fn widest_over(f: &Facts, limit: u8) -> Option<(usize, u8)> {
    f.presentation_channels
        .iter()
        .enumerate()
        .filter_map(|(p, c)| c.map(|c| (p, c)))
        .filter(|&(_, c)| c > limit)
        .max_by_key(|&(p, c)| (c, core::cmp::Reverse(p)))
}

fn judge_one(disc: Disc, f: &Facts) -> Verdict {
    let fs = f.sampling_frequency;
    let fba = f.format_sync == SYNC_FBA;
    let mut no = Vec::new();
    match disc {
        Disc::BluRay => {
            if !fba {
                no.push(Reason {
                    text: "Blu-ray carries the FBA syntax; this stream is not FBA".to_string(),
                    source: DOLBY_SCOPE,
                });
            }
            if !matches!(fs, 48_000 | 96_000 | 192_000) {
                no.push(Reason {
                    text: format!(
                        "Blu-ray takes TrueHD at 48, 96 and 192 kHz; this stream is at {}",
                        khz(fs)
                    ),
                    source: BDA_FIGURE,
                });
            }
            if fs == 192_000
                && let Some((p, c)) = widest_over(f, 6)
            {
                no.push(Reason {
                    text: format!("Blu-ray takes 6 channels at 192 kHz; presentation {p} has {c}"),
                    source: BDA_FIGURE,
                });
            }
        }
        Disc::HdDvd => {
            if matches!(fs, 176_400 | 192_000)
                && let Some((p, c)) = widest_over(f, 2)
            {
                no.push(Reason {
                    text: format!(
                        "HD DVD takes 2 channels at 176.4 and 192 kHz; presentation {p} has {c} at {}",
                        khz(fs)
                    ),
                    source: ENCODER_HD_DVD,
                });
            }
        }
        Disc::DvdAudio => {
            if f.format_sync != SYNC_FBB {
                no.push(Reason {
                    text: "DVD-Audio carries the FBB syntax; this stream is FBA".to_string(),
                    source: DOLBY_SCOPE,
                });
            }
        }
    }
    if fba && disc != Disc::DvdAudio && !f.delivery.is_empty() {
        let rules = f
            .delivery
            .iter()
            .map(|(id, n)| format!("{id} x{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        no.push(Reason {
            text: format!(
                "a film disc takes a stream within its delivery bounds; this one breaks {rules}"
            ),
            source: DELIVERY,
        });
    }
    if !no.is_empty() {
        return Verdict::No(no);
    }
    if disc == Disc::HdDvd {
        if !fba {
            return Verdict::Unjudged(Reason {
                text: "what HD DVD asks of an FBB stream is not judged".to_string(),
                source: NO_SOURCE,
            });
        }
        if fs.is_multiple_of(44_100) {
            return Verdict::Unjudged(Reason {
                text: format!(
                    "no public source gives the rates HD DVD takes; this stream is at {}",
                    khz(fs)
                ),
                source: NO_SOURCE,
            });
        }
    }
    Verdict::Yes
}

/// Judges a stream for every format, in [`Disc::ALL`] order.
#[must_use]
pub fn judge(facts: &Facts) -> [Row; 3] {
    Disc::ALL.map(|disc| Row {
        disc,
        verdict: judge_one(disc, facts),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(fs: u32, channels: [Option<u8>; 4]) -> Facts {
        Facts {
            format_sync: SYNC_FBA,
            sampling_frequency: fs,
            presentation_channels: channels,
            delivery: Vec::new(),
        }
    }

    fn verdicts(f: &Facts) -> [&'static str; 3] {
        judge(f).map(|r| r.verdict.word())
    }

    #[test]
    fn a_stream_at_48_or_96_khz_fits_both_film_discs() {
        let atmos = facts(48_000, [Some(2), Some(6), Some(8), Some(16)]);
        assert_eq!(verdicts(&atmos), ["yes", "yes", "no"]);
        let s71 = facts(96_000, [Some(2), Some(6), Some(8), None]);
        assert_eq!(verdicts(&s71), ["yes", "yes", "no"]);
    }

    /// Blu-ray takes six channels at 192 kHz, HD DVD two.
    #[test]
    fn at_192_khz_the_discs_take_six_and_two_channels() {
        assert_eq!(
            verdicts(&facts(192_000, [Some(2), None, None, None])),
            ["yes", "yes", "no"]
        );
        let six = facts(192_000, [Some(2), Some(6), None, None]);
        assert_eq!(verdicts(&six), ["yes", "no", "no"]);
        let Verdict::No(r) = &judge(&six)[1].verdict else {
            panic!()
        };
        assert_eq!(r[0].source, ENCODER_HD_DVD);
        assert!(r[0].text.ends_with("presentation 1 has 6 at 192 kHz"));
        let eight = facts(192_000, [Some(2), Some(6), Some(8), None]);
        let Verdict::No(r) = &judge(&eight)[0].verdict else {
            panic!()
        };
        assert_eq!(r.len(), 1);
        assert!(r[0].text.ends_with("presentation 2 has 8"));
        assert_eq!(r[0].source, BDA_FIGURE);
    }

    #[test]
    fn blu_ray_takes_neither_the_44_1_khz_family_nor_176_4_khz() {
        for fs in [44_100, 88_200, 176_400] {
            let f = facts(fs, [Some(2), None, None, None]);
            let rows = judge(&f);
            assert_eq!(rows[0].verdict.word(), "no", "{fs}");
            assert_eq!(rows[1].verdict.word(), "unjudged", "{fs}: HD DVD's rates");
        }
        // HD DVD's channel bound still holds where its rates are not known
        let f = facts(176_400, [Some(2), Some(6), None, None]);
        assert_eq!(judge(&f)[1].verdict.word(), "no");
    }

    #[test]
    fn dvd_audio_carries_fbb_and_blu_ray_fba() {
        let mut f = facts(96_000, [Some(2), Some(6), None, None]);
        let rows = judge(&f);
        assert_eq!(rows[2].verdict.reasons()[0].source, DOLBY_SCOPE);
        f.format_sync = SYNC_FBB;
        let rows = judge(&f);
        assert_eq!(rows[0].verdict.word(), "no");
        assert_eq!(rows[1].verdict.word(), "unjudged");
        assert_eq!(rows[2].verdict.word(), "yes");
    }

    /// A stream its own verifier would refuse goes on neither film disc.
    #[test]
    fn delivery_outside_the_bounds_keeps_a_stream_off_both_film_discs() {
        let mut f = facts(48_000, [Some(2), Some(6), Some(8), None]);
        f.delivery = vec![("rate.over_ceiling", 3)];
        let rows = judge(&f);
        assert_eq!(rows.clone().map(|r| r.verdict.word()), ["no", "no", "no"]);
        assert!(
            rows[0].verdict.reasons()[0]
                .text
                .ends_with("rate.over_ceiling x3")
        );
    }

    #[test]
    fn rates_print_as_a_report_writes_them() {
        assert_eq!(khz(48_000), "48 kHz");
        assert_eq!(khz(44_100), "44.1 kHz");
        assert_eq!(khz(176_400), "176.4 kHz");
        for d in Disc::ALL {
            assert_eq!(Disc::from_id(d.id()), Some(d));
        }
    }
}
