//! Decoding a stream as its bytes arrive.
//!
//! The other calls of the engine take a [`Source`](crate::source::Source),
//! which opens again at its beginning as often as it is asked: a command
//! looks at the first bytes to tell the codecs apart, runs the checks of
//! `verify` over the whole stream on a thread of their own, and decodes in a
//! pass of its own beside them. A pipe can be read once, so standard input
//! used to be taken into memory whole before a command began.
//!
//! What is here reads a stream once. The bytes are framed into access units
//! or syncframes as they come, and every unit is handed to the checks of
//! `verify` and to the decode alike: there is no second pass, no source
//! opened again, and nothing read ahead of the decode but what tells the
//! codecs apart, which is a few kilobytes and has a bound. What a decode
//! has made is handed to a [`PcmSink`] unit by unit, so the first samples
//! are out before the stream has ended.
//!
//! There are two doors, and the second stands on the first:
//!
//! - [`StreamDecoder`] is pushed bytes in pieces of any size, by a caller
//!   that has them from somewhere the engine does not know: a socket, a
//!   demultiplexer, a buffer of its own. What comes out does not depend on
//!   where the pieces were cut.
//! - [`decode_reader`] reads a [`Read`] to its end on a thread of its own
//!   and pushes what it read, with a bounded queue between the two: a sink
//!   that is slow holds the reader back, and the queue never holds more
//!   than [`QUEUE_PIECES`] pieces of [`PIECE`] bytes.
//!
//! The output of a stream is one layout from its first unit to its last. A
//! receiver of headerless samples cannot be told that the width changed, so
//! a change of the channels or the sampling frequency is an event of its own
//! ([`StreamEvent::ConfigChange`]) and [`LayoutChange`] says what is done at
//! it: the output ends there, or the codec keeps the layout of the first
//! unit where it can.

use std::io::Read;

use anyhow::Result;

use crate::calls::Format;
use crate::decode::{WavSample, WavSpec};
use crate::integrity::{Findings, Verdict};

/// How many bytes [`decode_reader`] reads at a time, and the size of one
/// piece in its queue.
pub const PIECE: usize = 64 << 10;

/// How many pieces the queue of [`decode_reader`] holds at most. With
/// [`PIECE`] that is a megabyte between the reader and the decode, whatever
/// the length of the stream.
pub const QUEUE_PIECES: usize = 16;

/// The most a stream is read ahead of its decode to tell the codecs apart.
/// A stream that has not said what it is by then is taken to be no stream.
pub const LOOK_AHEAD: usize = 64 << 10;

/// What is done when the channels or the sampling frequency of a stream
/// change in the middle of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutChange {
    /// The output ends at the unit that changed, and the run is not clean
    /// (exit 7). Whoever receives headerless samples could not tell the new
    /// width from the old, so this is what a stream to standard output does
    /// when nothing else is asked.
    Stop,
    /// The output keeps the layout of the first unit where the codec offers
    /// it: an E-AC-3 programme writes silence in the channels a frame group
    /// does not carry, as a decode to a file always has. A TrueHD stream has
    /// no such layout and refuses it. The change is still a fault (exit 7).
    Fixed,
}

/// What [`LayoutChange::Fixed`] on a TrueHD stream is told.
pub const FIXED_LAYOUT_IS_EAC3: &str = "--on-layout-change fixed keeps the layout of the first \
     frame group of an E-AC-3 programme by writing silence in the channels a group does not \
     carry; a TrueHD decoder has no such layout to keep, so a TrueHD stream takes stop";

/// What a policy for a change of layout with an object output is told.
pub const LAYOUT_CHANGE_IS_SAMPLES: &str = "--on-layout-change says what a decode to samples \
     does when the channels or the sampling frequency change: it applies to --format pcm or wav";

/// What a decode of a stream as it arrives is asked.
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// What reports and messages call the stream: `<stdin>` for standard
    /// input.
    pub name: String,
    /// How the samples are decoded, as for a decode of a source.
    pub pcm: crate::api::PcmOptions,
    /// What is done at a change of layout.
    pub on_layout_change: LayoutChange,
}

impl StreamOptions {
    /// The options a decode of `name` has when it is given none: the
    /// default samples, and the output ends at a change of layout.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pcm: crate::api::PcmOptions::default(),
            on_layout_change: LayoutChange::Stop,
        }
    }
}

/// The shape of the samples a decode writes: as many channels in every
/// sample, in the order they are written, and the format of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// Channels in every sample.
    pub channels: u16,
    /// The WAVE `dwChannelMask` of the channels in the order they are
    /// written; 0 where a channel has no bit.
    pub mask: u32,
    /// 24-bit integers (TrueHD, MLP) or 32-bit floats (AC-3, E-AC-3).
    pub sample: WavSample,
}

impl Layout {
    /// Bytes of one sample of every channel.
    #[must_use]
    pub const fn frame_bytes(self) -> usize {
        self.channels as usize
            * match self.sample {
                WavSample::Int24 => 3,
                WavSample::Float32 => 4,
            }
    }
}

/// Something the checks or the decode found wrong in one unit of the stream.
///
/// The checks of `verify` count a rule over the whole stream and list its
/// first findings; this is one of those, said when the unit it is in has
/// been decoded. The order of the findings of a stream is the order of its
/// units, and within a unit the order the rules are checked in, whatever
/// thread found them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The unit it is in: the access unit or the frame group, counted from 0.
    pub unit: u64,
    /// Where that unit starts in the stream.
    pub byte_offset: u64,
    /// The rule, as `oadec rules` names it.
    pub rule: String,
    /// What was found, as `verify` lists it.
    pub message: String,
}

/// What a decode says while it runs, besides its samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// The first unit has decoded and the output begins: what the stream is
    /// and the shape of the samples that follow.
    Opened {
        /// The syntax of the stream.
        format: Format,
        /// The shape of every sample until a change of layout.
        layout: Layout,
        /// The sampling frequency, in hertz.
        rate: u32,
    },
    /// The channels or the sampling frequency changed at a unit. With
    /// [`LayoutChange::Stop`] nothing of that unit or after it is written.
    ConfigChange {
        /// The unit that changed, counted from 0.
        unit: u64,
        /// Where it starts in the stream.
        byte_offset: u64,
        /// What changed, as a phrase ("the sampling frequency").
        what: String,
        /// What was done at it.
        policy: LayoutChange,
    },
    /// Something found wrong.
    Finding(Finding),
}

/// The samples of one unit, in the shape the [`StreamEvent::Opened`] before
/// them said.
#[derive(Debug, Clone, Copy)]
pub struct PcmFrame<'a> {
    /// The unit they were decoded from, counted from 0.
    pub unit: u64,
    /// Every channel of every sample in turn, little-endian: the bytes a
    /// headerless file of the decode holds.
    pub bytes: &'a [u8],
}

/// Where a decode of a stream as it arrives writes what it makes.
///
/// It is told [`StreamEvent::Opened`] before the first samples, every event
/// in the order of the units it belongs to, and the samples of each unit as
/// that unit is decoded. What it hands back as an error ends the decode
/// with that error: a receiver that went away, for one.
pub trait PcmSink {
    /// Something the decode has to say.
    ///
    /// # Errors
    /// Whatever the sink cannot do with it.
    fn event(&mut self, event: &StreamEvent) -> Result<()>;

    /// The samples of a unit.
    ///
    /// # Errors
    /// Whatever the sink cannot do with them.
    fn pcm(&mut self, frame: PcmFrame<'_>) -> Result<()>;
}

/// How the queue of [`decode_reader`] was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueStats {
    /// The most pieces it may hold.
    pub bound: usize,
    /// The most pieces it held at once.
    pub high_water: usize,
}

/// What a decode of a stream as it arrives came to.
#[derive(Debug)]
pub struct StreamEnd {
    /// The syntax of the stream.
    pub format: Format,
    /// Everything found wrong with what was delivered, by the checks of
    /// `verify` and by the decode: the verdict is the exit code of `decode`.
    pub findings: Findings,
    /// Samples written, per channel.
    pub samples: u64,
    /// Units decoded.
    pub units: u64,
    /// Bytes of the stream that were read.
    pub bytes: u64,
    /// The line the decode of a file ends its summary with, in the same
    /// words: what was decoded, and where the output ends if it ends early.
    pub summary: Vec<String>,
    /// How the queue between the reader and the decode was used; the
    /// default where the bytes were pushed.
    pub queue: QueueStats,
}

impl StreamEnd {
    /// The verdict of the decode.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        self.findings.verdict()
    }
}

/// A decode that is pushed the bytes of a stream as they arrive.
///
/// The bytes may be cut anywhere: a unit that is not whole yet waits for the
/// rest, and what comes out is the same whatever the pieces were.
#[derive(Debug)]
pub struct StreamDecoder {
    options: StreamOptions,
    /// Bytes pushed so far.
    pushed: u64,
}

/// What [`StreamDecoder`] says until the decode is built.
const NOT_YET: &str = "decoding a stream as it arrives is not implemented yet";

impl StreamDecoder {
    /// A decode that has been pushed nothing yet.
    #[must_use]
    pub fn new(options: StreamOptions) -> Self {
        Self { options, pushed: 0 }
    }

    /// The options it was made with.
    #[must_use]
    pub fn options(&self) -> &StreamOptions {
        &self.options
    }

    /// Takes the next bytes of the stream, and writes to `sink` what they
    /// complete.
    ///
    /// # Errors
    /// When the stream cannot be decoded as it was asked, when the first
    /// [`LOOK_AHEAD`] bytes hold no stream, and what the sink gives.
    pub fn push(&mut self, bytes: &[u8], sink: &mut dyn PcmSink) -> Result<()> {
        let _ = sink;
        self.pushed += bytes.len() as u64;
        Ok(())
    }

    /// The stream has ended: whatever is still held is decoded and written,
    /// a unit cut off by the end is reported as the decode of a file reports
    /// it, and the verdict is drawn.
    ///
    /// # Errors
    /// When the stream held nothing to decode, which is said in the words a
    /// file of the same bytes is told, and what the sink gives.
    pub fn finish(self, sink: &mut dyn PcmSink) -> Result<StreamEnd> {
        let _ = sink;
        anyhow::bail!("{NOT_YET} ({} bytes pushed)", self.pushed)
    }
}

/// Reads `from` to its end on a thread of its own and decodes what it reads
/// into `sink`, with a queue of at most [`QUEUE_PIECES`] pieces between the
/// two. The reader is read once, and nothing is read that the decode does
/// not take.
///
/// # Errors
/// What reading gives, and what [`StreamDecoder::push`] and
/// [`StreamDecoder::finish`] give.
pub fn decode_reader(
    from: Box<dyn Read + Send>,
    options: StreamOptions,
    sink: &mut dyn PcmSink,
) -> Result<StreamEnd> {
    drop(from);
    StreamDecoder::new(options).finish(sink)
}

/// Bytes of the WAVE header a stream to standard output begins with.
pub const STREAM_WAVE_HEADER_LEN: usize = 12 + 8 + 40 + 8;

/// What the `RIFF` size and the `data` size of a WAVE stream hold: the
/// length is not known when the header goes out, and nothing goes back to
/// it.
pub const UNKNOWN_LENGTH: u32 = 0xFFFF_FFFF;

/// The header of a WAVE stream of samples shaped as `spec`: `RIFF`/`WAVE`, an
/// extensible `fmt ` chunk as a WAVE file of the decode has, and a `data`
/// chunk, both sizes [`UNKNOWN_LENGTH`]. There is no `JUNK` chunk to become
/// `ds64` and no RF64: a stream is never gone back over. See
/// `docs/streaming.md` for what receivers make of it.
#[must_use]
pub fn stream_wave_header(spec: WavSpec) -> Vec<u8> {
    let _ = spec;
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le32(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
    }

    fn le16(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes(bytes[at..at + 2].try_into().expect("two bytes"))
    }

    #[test]
    fn a_wave_stream_says_its_length_is_unknown_and_its_samples_are_as_a_file_has_them() {
        for (spec, bits, subformat) in [
            (
                WavSpec {
                    channels: 8,
                    rate: 48_000,
                    mask: 0x63F,
                    sample: WavSample::Int24,
                },
                24u16,
                1u8,
            ),
            (
                WavSpec {
                    channels: 2,
                    rate: 44_100,
                    mask: 0x3,
                    sample: WavSample::Float32,
                },
                32,
                3,
            ),
        ] {
            let header = stream_wave_header(spec);
            assert_eq!(header.len(), STREAM_WAVE_HEADER_LEN);
            assert_eq!(&header[0..4], b"RIFF");
            assert_eq!(le32(&header, 4), UNKNOWN_LENGTH);
            assert_eq!(&header[8..12], b"WAVE");
            assert_eq!(&header[12..16], b"fmt ");
            assert_eq!(le32(&header, 16), 40);
            assert_eq!(le16(&header, 20), 0xFFFE);
            assert_eq!(le16(&header, 22), spec.channels);
            assert_eq!(le32(&header, 24), spec.rate);
            let align = spec.channels * bits / 8;
            assert_eq!(le32(&header, 28), spec.rate * u32::from(align));
            assert_eq!(le16(&header, 32), align);
            assert_eq!(le16(&header, 34), bits);
            assert_eq!(le16(&header, 36), 22);
            assert_eq!(le16(&header, 38), bits);
            assert_eq!(le32(&header, 40), spec.mask);
            assert_eq!(header[44], subformat);
            assert_eq!(&header[60..64], b"data");
            assert_eq!(le32(&header, 64), UNKNOWN_LENGTH);
        }
    }

    #[test]
    fn the_shape_of_a_sample_is_its_channels_times_their_width() {
        let int = Layout {
            channels: 6,
            mask: 0x3F,
            sample: WavSample::Int24,
        };
        let float = Layout {
            sample: WavSample::Float32,
            ..int
        };
        assert_eq!(int.frame_bytes(), 18);
        assert_eq!(float.frame_bytes(), 24);
    }
}
