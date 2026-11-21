//! Streams whose configuration changes at a major sync.
//!
//! A stream may change what it is in the middle: a programme of two
//! channels gives way to one of eight, the sampling rate changes, a
//! presentation is left out and comes back. Each part of such a stream is a
//! recipe of its own, written from its first access unit as if nothing came
//! before it, so it opens with a major sync and restart headers in every
//! substream; the parts are laid end to end. The samples of every part are
//! known before any decoder has read a byte, as those of any recipe are.
//!
//! The input timing of each part starts again from where its recipe starts
//! it, so a decoder sees a jump at every change as well. Nothing here has
//! been put in front of the outside decoders: the parts are recipes that
//! have, or one of them with its `substream_info` changed alone.

use crate::recipe::{Made, find};
use crate::write::Stream;

/// A named stream whose configuration changes.
#[derive(Debug, Clone, Copy)]
pub struct Change {
    /// The name: lower case, words joined by hyphens.
    pub name: &'static str,
    /// What changes, in a sentence.
    pub about: &'static str,
    make: fn() -> Vec<Made>,
}

/// One part of a [`Changed`] stream: a recipe written by itself.
#[derive(Debug, Clone)]
pub struct Part {
    /// The recipe made.
    pub made: Made,
    /// The part written.
    pub stream: Stream,
    /// Index, in the whole stream, of the part's first access unit: the one
    /// that carries the change.
    pub first_unit: usize,
}

impl Part {
    /// Access units of the part.
    #[must_use]
    pub fn units(&self) -> usize {
        self.stream.unit_offsets.len()
    }

    /// The samples of every presentation the part's substreams make, that of
    /// the first substream first ([`Made::presentations`]).
    #[must_use]
    pub fn presentations(&self) -> Vec<Vec<Vec<i32>>> {
        self.made.presentations(&self.stream)
    }
}

/// A [`Change`] written: the parts end to end.
#[derive(Debug, Clone)]
pub struct Changed {
    /// The elementary stream.
    pub bytes: Vec<u8>,
    /// Where each access unit starts.
    pub unit_offsets: Vec<usize>,
    /// The parts, in order.
    pub parts: Vec<Part>,
}

impl Change {
    /// The recipes of the parts, made.
    #[must_use]
    pub fn parts(&self) -> Vec<Made> {
        (self.make)()
    }

    /// The stream.
    ///
    /// # Errors
    /// When a part does not write; no change of this crate does that, and a
    /// test holds every one of them to it.
    pub fn write(&self) -> Result<Changed, String> {
        let mut changed = Changed {
            bytes: Vec::new(),
            unit_offsets: Vec::new(),
            parts: Vec::new(),
        };
        for made in self.parts() {
            let stream = made.stream()?;
            let base = changed.bytes.len();
            let first_unit = changed.unit_offsets.len();
            changed
                .unit_offsets
                .extend(stream.unit_offsets.iter().map(|offset| base + offset));
            changed.bytes.extend_from_slice(&stream.bytes);
            changed.parts.push(Part {
                made,
                stream,
                first_unit,
            });
        }
        Ok(changed)
    }
}

/// Every stream whose configuration changes, in the order they are listed.
pub const CHANGES: &[Change] = &[
    Change {
        name: "change-channels",
        about: "two channels in one substream, then eight in three, then six in two: the substream count and substream_info change",
        make: change_channels,
    },
    Change {
        name: "change-rate",
        about: "48 kHz, then 44.1 kHz at the same 40 samples a unit, then 96 kHz at 80: the sampling frequency alone, then the samples per unit with it",
        make: change_rate,
    },
    Change {
        name: "change-presentation",
        about: "eight channels in three substreams, then six in two whose substream_info leaves the eight-channel presentation out, then eight again",
        make: change_presentation,
    },
];

/// The change of that name.
#[must_use]
pub fn find_change(name: &str) -> Option<&'static Change> {
    CHANGES.iter().find(|change| change.name == name)
}

fn recipe(name: &str) -> Made {
    find(name)
        .unwrap_or_else(|| panic!("no recipe {name}"))
        .make()
}

fn change_channels() -> Vec<Made> {
    vec![
        recipe("plain"),
        recipe("eight-channels"),
        recipe("six-channels"),
    ]
}

fn change_rate() -> Vec<Made> {
    vec![recipe("plain"), recipe("rate-44100"), recipe("rate-96000")]
}

/// `substream_info` of a stream of two substreams whose eight-channel
/// presentation is not there: the six-channel one reads both, and no bit
/// gives the eight-channel one a substream.
const SIX_WITHOUT_EIGHT: u8 = 0x0C;

fn change_presentation() -> Vec<Made> {
    let mut six = recipe("six-channels");
    six.layout.substream_info = Some(SIX_WITHOUT_EIGHT);
    vec![recipe("eight-channels"), six, recipe("eight-channels")]
}
