//! The AC-3 recipes: named streams about delta bit allocation.
//!
//! As with the TrueHD recipes, a name gives the same bytes on every machine:
//! exponents and mantissas are drawn from a seeded generator in integer
//! arithmetic. What a stream comes to is known as coefficients exactly and
//! as samples to the accuracy of floating point, so a decoder is judged on
//! the samples by a signal-to-noise ratio and not by a digest.

use crate::ac3::{
    Ac3Block, Ac3Channel, Ac3Layout, Ac3Stream, Allocation, BLOCKS, CHANNELS, Delta, Offsets,
    Segment, write_ac3,
};
use crate::signal::Rng;

/// What a block says of the delta bit allocation, by frame and block.
type Said = fn(usize, usize) -> Option<[Delta; CHANNELS]>;

/// A named AC-3 stream.
#[derive(Debug, Clone, Copy)]
pub struct Ac3Recipe {
    /// The name: lower case, words joined by hyphens, beginning `ac3-`, or
    /// `probe-ac3-` for a probe.
    pub name: &'static str,
    /// What the stream carries, in a sentence.
    pub about: &'static str,
    layout: fn() -> Ac3Layout,
    /// Coefficients a channel.
    coefficients: usize,
    /// The SNR offsets every frame sends.
    offsets: Offsets,
    said: Said,
}

impl Ac3Recipe {
    /// The layout and the frames of the recipe.
    #[must_use]
    pub fn make(&self) -> (Ac3Layout, Vec<Vec<Ac3Block>>) {
        let mut rng = Rng::new(7);
        let frames = (0..FRAMES).map(|f| self.frame(&mut rng, f)).collect();
        ((self.layout)(), frames)
    }

    /// The stream.
    ///
    /// # Errors
    /// When the frames do not fit the syntax or the size of a frame; no
    /// recipe of this crate does that, and a test holds every one to it.
    pub fn stream(&self) -> Result<Ac3Stream, String> {
        let (layout, frames) = self.make();
        write_ac3(&layout, &frames)
    }

    /// One frame: new exponents in the first and the fourth block, the
    /// parameters and the offsets in the first, mantissas of noise, and of
    /// the delta bit allocation what the recipe says.
    fn frame(&self, rng: &mut Rng, f: usize) -> Vec<Ac3Block> {
        let len = self.coefficients;
        (0..BLOCKS)
            .map(|b| Ac3Block {
                channels: std::array::from_fn(|ch| Ac3Channel {
                    exponents: (b % 3 == 0).then(|| exponents(rng, len, 9 + 3 * ch as u8)),
                    mantissas: (0..len).map(|_| rng.signed(16) as i16).collect(),
                }),
                allocation: (b == 0).then_some(USUAL),
                offsets: (b == 0).then_some(self.offsets),
                delta: (self.said)(f, b),
            })
            .collect()
    }
}

/// A stream written to ask a question: past what the standard allows, or
/// where its text and its pseudo code can be read apart.
#[derive(Debug, Clone, Copy)]
pub struct Ac3Probe {
    /// The stream.
    pub recipe: Ac3Recipe,
    /// What the decoder of this workspace makes of it today.
    pub read: Read,
}

/// What a decoder makes of a stream, against the samples it was written to
/// come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    /// Those samples.
    Samples,
    /// Others, or none: the stream is not read as it was laid out.
    Otherwise,
}

/// Frames to a recipe, 1536 samples each.
const FRAMES: usize = 8;
/// Coefficients a channel at a bandwidth code of 20: up to band 46 of the
/// fifty, which is 12.4 kHz at 48 kHz.
const NARROW: usize = 133;
/// And at the widest code there is, 60: every band.
const WIDEST: usize = 253;

/// The parameters an encoder most often sends.
const USUAL: Allocation = Allocation {
    slow_decay: 2,
    fast_decay: 1,
    slow_gain: 1,
    db_per_bit: 2,
    floor: 7,
};

/// Offsets under which the pointers of the narrow spectra reach from none to
/// the widest quantiser.
const OFFSETS: Offsets = Offsets {
    coarse: 27,
    fine: [5, 11],
    fast_gain: [4, 2],
};

/// And 6 dB under them, a bit less to a mantissa: what leaves room in a
/// frame for two channels of the widest bandwidth.
const LOWER: Offsets = Offsets {
    coarse: 25,
    ..OFFSETS
};

/// Exponents that wander from `first` by up to two a step, as D15 can say,
/// and stay at 8 or over: coefficients no louder than `2^-8` that have
/// nothing to do with one another add up to samples well within full scale.
fn exponents(rng: &mut Rng, len: usize, first: u8) -> Vec<u8> {
    let mut out = vec![first];
    while out.len() < len {
        let last = i32::from(out[out.len() - 1]);
        let step = (rng.next_u64() % 5) as i32 - 2;
        out.push((last + step).clamp(8, 22) as u8);
    }
    out
}

/// Four segments that between them reach most of the bands, some lowering
/// the masking curve and some raising it, one of them of no length.
fn segments(seed: u8) -> Vec<Segment> {
    vec![
        Segment {
            offset: 3 + seed % 4,
            length: 5,
            code: seed % 8,
        },
        Segment {
            offset: 2,
            length: 9,
            code: (seed + 5) % 8,
        },
        Segment {
            offset: 0,
            length: 0,
            code: 7,
        },
        Segment {
            offset: 6,
            length: 12,
            code: (seed + 2) % 8,
        },
    ]
}

/// 640 kbit/s at 48 kHz.
fn at_48000() -> Ac3Layout {
    Ac3Layout::new(48_000, 36)
}

fn nothing(_: usize, _: usize) -> Option<[Delta; CHANNELS]> {
    None
}

/// Every state a block can leave a channel in, and every way into it that
/// decoders read alike: new segments, the same again, none, a block that
/// says nothing, and new ones on one channel while the other goes on. No
/// "reuse" follows a "none", which the standard can be read two ways about,
/// and none stands in a first block, where it forbids one.
fn every_state(f: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    let seed = f as u8;
    match b {
        0 => Some([Delta::New(segments(seed + 1)), Delta::None]),
        1 => Some([Delta::Reuse, Delta::New(segments(seed + 4))]),
        2 => None,
        3 => Some([Delta::None, Delta::Reuse]),
        4 => Some([Delta::New(segments(seed + 6)), Delta::Reuse]),
        _ => Some([Delta::Reuse, Delta::None]),
    }
}

/// One code a frame, over thirty bands of the left channel: 24 dB down to
/// 24 dB up in steps of 6, without the nought there is no code for.
fn every_code(f: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    let segment = |offset: u8| Segment {
        offset,
        length: 15,
        code: (f % 8) as u8,
    };
    (b == 0).then(|| [Delta::New(vec![segment(10), segment(0)]), Delta::None])
}

/// New segments in every frame and every one of no length, sent again by a
/// "reuse": every field of the tool is there, and no band is moved.
fn no_length(f: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    let empty = |offset: u8, code: usize| Segment {
        offset,
        length: 0,
        code: (code % 8) as u8,
    };
    match b {
        0 => Some([
            Delta::New(vec![empty(5, f), empty(9, f + 3)]),
            Delta::New(vec![empty(31, f + 5)]),
        ]),
        2 => Some([Delta::Reuse, Delta::Reuse]),
        _ => None,
    }
}

/// Eight segments a channel, which is the most a block sends, and the last
/// of them ending where the bands end. The left channel's cover the bands
/// from the first; the right channel's begin at the widest offset there is.
/// The recipe gives its channels every band, so the last of the segments
/// moves pointers too.
fn eight_segments(f: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    const FROM_THE_FIRST: [(u8, u8); 8] = [
        (0, 15),
        (0, 15),
        (0, 1),
        (1, 2),
        (2, 3),
        (0, 0),
        (3, 4),
        (0, 4),
    ];
    const FROM_THE_WIDEST: [(u8, u8); 8] = [
        (31, 15),
        (0, 1),
        (0, 0),
        (0, 1),
        (0, 0),
        (0, 1),
        (0, 0),
        (0, 1),
    ];
    let segments = |spans: &[(u8, u8); 8], turn: usize| {
        spans
            .iter()
            .enumerate()
            .map(|(n, &(offset, length))| Segment {
                offset,
                length,
                code: ((f + n + turn) % 8) as u8,
            })
            .collect()
    };
    match b {
        0 => Some([
            Delta::New(segments(&FROM_THE_FIRST, 0)),
            Delta::New(segments(&FROM_THE_WIDEST, 3)),
        ]),
        3 => Some([
            Delta::New(segments(&FROM_THE_WIDEST, 5)),
            Delta::New(segments(&FROM_THE_FIRST, 6)),
        ]),
        _ => None,
    }
}

/// A frame that ends with a delta in force on both channels, and a frame
/// that says nothing of it in its first block: there is none in that frame
/// (clause 4.4.3.47).
fn silent_first_block(f: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    (f.is_multiple_of(2) && b == 0).then(|| [Delta::New(segments(1)), Delta::New(segments(4))])
}

/// New segments, then "none", then "reuse", in three blocks of one frame.
fn reuse_after_none(_: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    match b {
        0 => Some([Delta::New(segments(1)), Delta::New(segments(4))]),
        1 => Some([Delta::None, Delta::None]),
        2 => Some([Delta::Reuse, Delta::Reuse]),
        _ => None,
    }
}

/// New segments in the first block of one frame and "reuse" in the first
/// block of the next, which the standard forbids (clause 4.4.3.49).
fn reuse_in_first_block(f: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    match (f % 2, b) {
        (0, 0) => Some([Delta::New(segments(1)), Delta::New(segments(4))]),
        (1, 0) => Some([Delta::Reuse, Delta::Reuse]),
        _ => None,
    }
}

/// New segments in the first block and the reserved code in the second.
fn reserved_code(_: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    match b {
        0 => Some([Delta::New(segments(1)), Delta::New(segments(4))]),
        1 => Some([Delta::Reserved, Delta::Reserved]),
        _ => None,
    }
}

/// On the left channel a segment that begins at band 44 and is fifteen
/// bands long, where there are fifty.
fn past_the_bands(_: usize, b: usize) -> Option<[Delta; CHANNELS]> {
    let over = vec![
        Segment {
            offset: 30,
            length: 10,
            code: 1,
        },
        Segment {
            offset: 4,
            length: 15,
            code: 6,
        },
    ];
    (b == 0).then(|| [Delta::New(over), Delta::New(segments(4))])
}

/// The recipe of that name, a probe's among them.
#[must_use]
pub fn find_ac3(name: &str) -> Option<&'static Ac3Recipe> {
    AC3_RECIPES
        .iter()
        .chain(AC3_PROBES.iter().map(|probe| &probe.recipe))
        .find(|recipe| recipe.name == name)
}

/// Every AC-3 recipe, in the order they are listed.
pub const AC3_RECIPES: &[Ac3Recipe] = &[
    Ac3Recipe {
        name: "ac3-plain",
        about: "two channels at 48 kHz with no delta bit allocation: exponents sent one by one and reused, and every quantiser from no bits to sixteen",
        layout: at_48000,
        coefficients: NARROW,
        offsets: OFFSETS,
        said: nothing,
    },
    Ac3Recipe {
        name: "ac3-delta",
        about: "delta bit allocation in every state the standard allows: new segments, the same again, none, a block that says nothing, on two channels out of step",
        layout: at_48000,
        coefficients: NARROW,
        offsets: OFFSETS,
        said: every_state,
    },
    Ac3Recipe {
        name: "ac3-delta-codes",
        about: "each of the eight codes of a segment in turn, from 24 dB down to 24 dB up, over thirty bands",
        layout: at_48000,
        coefficients: NARROW,
        offsets: OFFSETS,
        said: every_code,
    },
    Ac3Recipe {
        name: "ac3-delta-no-length",
        about: "new segments in every frame and every one of no length: the fields are sent and the masking curve stays where it is",
        layout: at_48000,
        coefficients: NARROW,
        offsets: OFFSETS,
        said: no_length,
    },
    Ac3Recipe {
        name: "ac3-delta-eight-segments",
        about: "eight segments a channel, the most a block sends, the last ending where the fifty bands end and one beginning at the widest offset there is, over channels of the widest bandwidth",
        layout: at_48000,
        coefficients: WIDEST,
        offsets: LOWER,
        said: eight_segments,
    },
    Ac3Recipe {
        name: "ac3-delta-silent-first-block",
        about: "a frame that says nothing of delta bit allocation in its first block, after a frame that ended with one in force: there is none",
        layout: at_48000,
        coefficients: NARROW,
        offsets: OFFSETS,
        said: silent_first_block,
    },
    Ac3Recipe {
        name: "ac3-delta-44100",
        about: "the same states at 44.1 kHz, where the hearing threshold under the masking curve is another",
        layout: || Ac3Layout::new(44_100, 36),
        coefficients: NARROW,
        offsets: OFFSETS,
        said: every_state,
    },
    Ac3Recipe {
        name: "ac3-delta-32000",
        about: "the same states at 32 kHz",
        layout: || Ac3Layout::new(32_000, 36),
        coefficients: NARROW,
        offsets: OFFSETS,
        said: every_state,
    },
];

/// Every AC-3 probe, in the order they are listed.
pub const AC3_PROBES: &[Ac3Probe] = &[
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-delta-unsaid",
            about: "mantissas laid out for a delta bit allocation the frame does not state: a decoder that works the widths out without it is reading other bits",
            layout: || Ac3Layout {
                delta_unsaid: true,
                ..at_48000()
            },
            coefficients: NARROW,
            offsets: OFFSETS,
            said: every_state,
        },
        read: Read::Otherwise,
    },
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-reuse-after-none",
            about: "\"reuse\" after \"none\" in one frame, laid out as the pseudo code of the standard has it: the segments sent before the \"none\" are applied again",
            layout: at_48000,
            coefficients: NARROW,
            offsets: OFFSETS,
            said: reuse_after_none,
        },
        read: Read::Otherwise,
    },
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-reuse-after-none-kept",
            about: "the same said, laid out as Dolby's decoder reads it: after \"none\" the state that \"reuse\" keeps is none",
            layout: || Ac3Layout {
                reuse_keeps_none: true,
                ..at_48000()
            },
            coefficients: NARROW,
            offsets: OFFSETS,
            said: reuse_after_none,
        },
        read: Read::Samples,
    },
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-reuse-in-first-block",
            about: "\"reuse\" in the first block of a frame, which the standard forbids, laid out as FFmpeg's decoder reads it: the segments of the frame before are applied",
            layout: || Ac3Layout {
                segments_outlive_frame: true,
                ..at_48000()
            },
            coefficients: NARROW,
            offsets: OFFSETS,
            said: reuse_in_first_block,
        },
        read: Read::Otherwise,
    },
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-reuse-in-first-block-empty",
            about: "the same said, laid out as the decoder of this repository reads it: a frame begins with no segments, so there are none to apply",
            layout: at_48000,
            coefficients: NARROW,
            offsets: OFFSETS,
            said: reuse_in_first_block,
        },
        read: Read::Samples,
    },
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-delta-reserved",
            about: "the code of the two bits that the standard reserves, after new segments, laid out as no delta bit allocation",
            layout: at_48000,
            coefficients: NARROW,
            offsets: OFFSETS,
            said: reserved_code,
        },
        read: Read::Samples,
    },
    Ac3Probe {
        recipe: Ac3Recipe {
            name: "probe-ac3-delta-past-the-bands",
            about: "a segment that runs nine bands past the fifty there are, laid out with the part of it that is within them applied",
            layout: || Ac3Layout {
                segments_past_the_bands: true,
                ..at_48000()
            },
            coefficients: NARROW,
            offsets: OFFSETS,
            said: past_the_bands,
        },
        read: Read::Samples,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ac3_recipe_is_written_and_has_a_name_of_its_own() {
        let mut names = Vec::new();
        for recipe in AC3_RECIPES
            .iter()
            .chain(AC3_PROBES.iter().map(|probe| &probe.recipe))
        {
            let probe = AC3_PROBES.iter().any(|p| p.recipe.name == recipe.name);
            assert_eq!(
                recipe.name.starts_with("probe-ac3-"),
                probe,
                "{}",
                recipe.name
            );
            assert_eq!(recipe.name.starts_with("ac3-"), !probe, "{}", recipe.name);
            assert!(
                recipe
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{}",
                recipe.name
            );
            let stream = recipe
                .stream()
                .unwrap_or_else(|e| panic!("{}: {e}", recipe.name));
            assert_eq!(stream.frames.len(), FRAMES, "{}", recipe.name);
            assert_eq!(stream.bytes.len(), FRAMES * stream.frame_bytes);
            assert!(!names.contains(&recipe.name), "{} twice", recipe.name);
            names.push(recipe.name);
            assert_eq!(find_ac3(recipe.name).map(|r| r.name), Some(recipe.name));
        }
        assert_eq!(find_ac3("ac3-no-such").map(|r| r.name), None);
    }

    #[test]
    fn an_ac3_recipe_is_the_same_bytes_every_time() {
        for recipe in AC3_RECIPES
            .iter()
            .chain(AC3_PROBES.iter().map(|probe| &probe.recipe))
        {
            assert!(
                recipe.stream().expect("written").bytes == recipe.stream().expect("written").bytes,
                "{}",
                recipe.name
            );
        }
    }

    /// A reference has no "reuse" in a first block, which the standard
    /// forbids, and none after a "none" with nothing new between, which the
    /// decoders read apart. Those are for the probes.
    #[test]
    fn the_recipes_keep_clear_of_a_reuse_that_is_forbidden_or_read_two_ways() {
        for said in [
            every_state as Said,
            every_code,
            no_length,
            eight_segments,
            silent_first_block,
            nothing,
        ] {
            for f in 0..FRAMES {
                let mut off = [true; CHANNELS];
                for b in 0..BLOCKS {
                    let Some(delta) = said(f, b) else { continue };
                    for (ch, state) in delta.iter().enumerate() {
                        match state {
                            Delta::Reuse => {
                                assert!(b > 0, "frame {f}: reuse in the first block");
                                assert!(!off[ch], "frame {f}, block {b}: reuse after none");
                            }
                            Delta::New(_) => off[ch] = false,
                            Delta::None => off[ch] = true,
                            Delta::Reserved => panic!("frame {f}, block {b}: the reserved code"),
                        }
                    }
                }
            }
        }
    }
}
