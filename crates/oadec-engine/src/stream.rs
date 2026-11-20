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
//! codecs apart, which is [`LOOK_AHEAD`] bytes at most. What a decode has
//! made is handed to a [`StreamSink`] unit by unit, so the first samples are
//! out before the stream has ended.
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
//! What is decoded, and what is found wrong, is what a decode of the same
//! bytes from a file gives: the decode of a TrueHD stream runs the checks of
//! `verify` on the units it decodes, on a thread of their own, and an AC-3
//! family stream is checked by the pass that decodes it. A finding of the
//! checks is told when the unit it is in is told, before the samples of that
//! unit, so what a sink is told, and in what order, is the same on every run
//! whatever the threads did.
//!
//! The output of a stream is one layout from its first unit to its last. A
//! receiver of headerless samples cannot be told that the width changed, so
//! a change of the channels or the sampling frequency is an event of its own
//! ([`StreamEvent::ConfigChange`]) and [`LayoutChange`] says what is done at
//! it: the output ends there, or the codec keeps the layout of the first
//! unit where it can.

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use oadec_diag::{Rule, Tally};
use oadec_truehd::{Extractor, Unit};

use crate::api::{
    CORE_DECODE_IS_AC3, KEEP_DUPLICATES_IS_TRUEHD, ON_ERROR_IS_TRUEHD, OnError,
    PRESENTATION_IS_TRUEHD, PROGRAMME_IS_AC3, PcmOptions, START_IS_TRUEHD_SAMPLES,
};
use crate::calls::{Format, refusal};
use crate::decode::{self, ConfigStop, Delivery, Taken, WavSample, WavSpec};
use crate::eac3::{self, Split, Splitter, Walk};
use crate::input::PassSummary;
use crate::integrity::{Findings, Verdict};
use crate::scan::Scan;
use crate::source::{Walking, Watcher};

/// How many bytes [`decode_reader`] reads at a time, and the size of one
/// piece in its queue.
pub const PIECE: usize = 64 << 10;

/// How many pieces the queue of [`decode_reader`] holds at most. With
/// [`PIECE`] that is a megabyte between the reader and the decode, whatever
/// the length of the stream.
pub const QUEUE_PIECES: usize = 16;

/// The most a stream is read ahead of its decode to tell the codecs apart:
/// what a decode of a file reads of its head to do the same.
pub const LOOK_AHEAD: usize = eac3::SNIFF_BYTES;

/// Access units the checks of a TrueHD stream may be behind its decode, and
/// decoded units that may wait for them to catch up.
const UNITS_AHEAD: usize = 1024;

/// Access units handed to the checks at a time. A unit is a few hundred
/// bytes and there are 1 200 of them in a second of audio: one by one, the
/// two threads spent more time waking each other than checking.
const BATCH: usize = 64;

/// What [`LayoutChange::Fixed`] on a TrueHD stream is told.
pub const FIXED_LAYOUT_IS_EAC3: &str = "--on-layout-change fixed keeps the layout of the first \
     frame group of an E-AC-3 programme by writing silence in the channels a group does not \
     carry; a TrueHD decoder has no such layout to keep, so a TrueHD stream takes stop";

/// What a policy for a change of layout with an object output is told.
pub const LAYOUT_CHANGE_IS_SAMPLES: &str = "--on-layout-change says what a decode to samples \
     does when the channels or the sampling frequency change: it applies to --format pcm or wav";

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
    /// The samples go on in the layout the stream changed to, and the sink
    /// is told so by [`StreamEvent::ConfigChange`] with the new layout, so
    /// that it can begin anew: a writer of files writes the next part. A
    /// TrueHD presentation a change takes away ends there, told with no new
    /// layout, and the change that brings it back is told as any other. A
    /// change the stream signals and the decode followed is not a fault of
    /// what was delivered.
    Split,
}

impl LayoutChange {
    /// What the policy is called on the command line and in a report.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Stop => "stop",
            Self::Fixed => "fixed",
            Self::Split => "split",
        }
    }
}

/// What a decode of a stream as it arrives is asked.
#[derive(Debug, Clone)]
pub struct StreamOptions {
    /// What reports and messages call the stream: `<stdin>` for standard
    /// input, its path for a file.
    pub name: String,
    /// How the samples are decoded, as for a decode of a source.
    pub pcm: PcmOptions,
    /// What is done at a change of layout. `None` is what a decode to a file
    /// has always done: a TrueHD stream ends its output there, and an AC-3
    /// family stream keeps its layout.
    pub on_layout_change: Option<LayoutChange>,
    /// The samples go into a WAVE stream or file, which names its channels:
    /// a TrueHD channel its mask has no bit for is warned about.
    pub wave: bool,
    /// Who is told how far the decode has read; nobody where `None`.
    pub watcher: Option<Arc<dyn Watcher>>,
    /// The length of the stream where it is known, for the watcher.
    pub size: Option<u64>,
}

impl StreamOptions {
    /// The options a decode of `name` has when it is given none: the
    /// default samples, headerless, and the output ends at a change of
    /// layout.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            pcm: PcmOptions::default(),
            on_layout_change: Some(LayoutChange::Stop),
            wave: false,
            watcher: None,
            size: None,
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

    /// The WAVE `fmt ` of samples of this shape at `rate`.
    #[must_use]
    pub const fn spec(self, rate: u32) -> WavSpec {
        WavSpec {
            channels: self.channels,
            rate,
            mask: self.mask,
            sample: self.sample,
        }
    }
}

/// Something the checks or the decode found wrong in one unit of the stream.
///
/// The checks of `verify` count a rule over the whole stream and list its
/// first findings; this is one of those listed, said when the unit it is in
/// is told. The order of the findings of a stream is the order of its units,
/// and within a unit the order of the catalog of rules, whatever thread found
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The unit it is in: the access unit or the frame group, counted from 0.
    /// A finding of the end of the stream is in the unit after the last.
    pub unit: u64,
    /// Where it is in the stream: the byte the check points at, or where its
    /// unit starts.
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
        /// The shape of the output before the change.
        old: Layout,
        /// The sampling frequency before the change.
        old_rate: u32,
        /// The shape after it, where the decode knows it: a TrueHD decode
        /// that stops does not decode the unit that changed, and the AC-3
        /// family decoder does not say which channels a group it fills with
        /// silence carried.
        new: Option<Layout>,
        /// The sampling frequency after it, where the decode knows it.
        new_rate: Option<u32>,
    },
    /// Something found wrong.
    Finding(Finding),
    /// A line the decode has to say, such as the warning for a channel a
    /// WAVE mask has no bit for: what a decode to a file says on stderr.
    Remark(String),
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

/// What a [`StreamSink`] gives back: [`anyhow::Result`], named here for a
/// sink whose crate does not use `anyhow` itself.
pub type SinkResult = Result<()>;

/// Where a decode of a stream as it arrives writes what it makes.
///
/// It is told [`StreamEvent::Opened`] before the first samples, every event
/// in the order of the units it belongs to, and the samples of each unit
/// after the events of that unit. What it hands back as an error ends the
/// decode with that error: a receiver that went away, for one.
pub trait StreamSink {
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

    /// The decode is about to wait for more of the stream: what the sink
    /// holds back is to go out now. [`decode_reader`] says it whenever its
    /// queue is empty; a sink that holds nothing back has nothing to do.
    ///
    /// # Errors
    /// Whatever the sink cannot do.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// How the queues of a decode were used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueStats {
    /// The most pieces the queue of [`decode_reader`] may hold; 0 where the
    /// bytes were pushed.
    pub bound: usize,
    /// The most pieces it held at once.
    pub high_water: usize,
    /// The most decoded units of a TrueHD stream that may wait for their
    /// checks; 0 for an AC-3 family stream, which nothing waits in.
    pub units_bound: usize,
    /// The most that waited at once.
    pub units_high_water: usize,
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
    /// Units framed: access units, or frame groups.
    pub units: u64,
    /// Bytes of the stream that were read.
    pub bytes: u64,
    /// How the queue between the reader and the decode was used; the
    /// default where the bytes were pushed.
    pub queue: QueueStats,
    told: Told,
}

/// What the summary of a decode is drawn from.
#[derive(Debug)]
enum Told {
    TrueHd {
        summary: Box<decode::Summary>,
        stopped: Option<ConfigStop>,
        elapsed: f64,
    },
    Ac3 {
        report: Box<eac3::Report>,
        /// The frame group and the byte the output ended at.
        stopped: Option<(u64, u64)>,
    },
}

impl StreamEnd {
    /// The verdict of the decode.
    #[must_use]
    pub fn verdict(&self) -> Verdict {
        self.findings.verdict()
    }

    /// Writes what a decode of the same stream from a file says of it: the
    /// summary of a TrueHD decode to `err`, and the report of the pass of an
    /// AC-3 family decode to `out`, with where the output ended if it ended
    /// early.
    ///
    /// # Errors
    /// What writing gives.
    pub fn print_summary(&self, out: &mut dyn Write, err: &mut dyn Write) -> io::Result<()> {
        match &self.told {
            Told::TrueHd {
                summary,
                stopped,
                elapsed,
            } => {
                if let Some(stop) = stopped {
                    stop.print(self.samples, err)?;
                }
                decode::print_summary(summary, *elapsed, err)
            }
            Told::Ac3 { report, stopped } => {
                if let Some((group, offset)) = stopped {
                    writeln!(
                        err,
                        "stopped at frame group {group} (byte {offset}): the channels of the \
                         programme changed; {} samples written",
                        self.samples
                    )?;
                }
                report.print(false, out, err)
            }
        }
    }
}

/// A decode that is pushed the bytes of a stream as they arrive.
///
/// The bytes may be cut anywhere: a unit that is not whole yet waits for the
/// rest, and what comes out is the same whatever the pieces were.
///
/// A call that ends with an error ends the decode: every call after it,
/// [`push`](Self::push), [`flush`](Self::flush) and
/// [`finish`](Self::finish), gives an error too, and nothing more is told.
#[derive(Debug)]
pub struct StreamDecoder {
    options: StreamOptions,
    started: Instant,
    /// Bytes pushed so far.
    pushed: u64,
    /// What came before the codec was told apart.
    head: Vec<u8>,
    codec: Option<Codec>,
    walking: Walking,
    /// A call ended with an error.
    failed: bool,
    /// A pause the checks of a TrueHD stream make before each batch: for a
    /// test that makes them the slower of the two.
    pace: Option<Duration>,
}

#[derive(Debug)]
enum Codec {
    TrueHd(Box<TrueHd>),
    Ac3(Box<Ac3>),
}

impl StreamDecoder {
    /// A decode that has been pushed nothing yet.
    #[must_use]
    pub fn new(options: StreamOptions) -> Self {
        let walking = Walking::of(
            options.watcher.as_ref(),
            "reading",
            options.size.unwrap_or(0),
        );
        Self {
            options,
            started: Instant::now(),
            pushed: 0,
            head: Vec::new(),
            codec: None,
            walking,
            failed: false,
            pace: None,
        }
    }

    /// The same decode with checks that pause for `pace` before each batch
    /// of units they take.
    #[cfg(test)]
    fn with_slow_checks(mut self, pace: Duration) -> Self {
        self.pace = Some(pace);
        self
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
    /// When the stream cannot be decoded as it was asked, an option that
    /// does not apply to its codec among them, what the sink gives, and
    /// when a call before this one ended with an error.
    pub fn push(&mut self, bytes: &[u8], sink: &mut dyn StreamSink) -> Result<()> {
        self.going()?;
        let pushed = self.push_on(bytes, sink);
        self.failed = pushed.is_err();
        pushed
    }

    fn push_on(&mut self, bytes: &[u8], sink: &mut dyn StreamSink) -> Result<()> {
        self.pushed += bytes.len() as u64;
        if let Some(codec) = &mut self.codec {
            return feed(codec, bytes, &mut self.walking, sink);
        }
        self.head.extend_from_slice(bytes);
        if self.head.len() < LOOK_AHEAD {
            return Ok(());
        }
        let head = std::mem::take(&mut self.head);
        let codec = self.codec.insert(self.begin(&head[..LOOK_AHEAD])?);
        feed(codec, &head, &mut self.walking, sink)
    }

    /// The stream has ended: whatever is still held is decoded and written,
    /// a unit cut off by the end is reported as the decode of a file reports
    /// it, and the verdict is drawn.
    ///
    /// # Errors
    /// When the stream held nothing to decode, which is said in the words a
    /// file of the same bytes is told, what the sink gives, and when a call
    /// before this one ended with an error. A stream that held nothing, and
    /// a start past its end, deliver nothing, and the error says so by its
    /// kind: [`crate::Error::NoStream`].
    pub fn finish(mut self, sink: &mut dyn StreamSink) -> Result<StreamEnd> {
        self.going()?;
        let codec = match self.codec.take() {
            Some(codec) => codec,
            None => {
                let head = std::mem::take(&mut self.head);
                let mut codec = self.begin(&head)?;
                feed(&mut codec, &head, &mut self.walking, sink)?;
                codec
            }
        };
        let elapsed = || self.started.elapsed().as_secs_f64();
        match codec {
            Codec::TrueHd(t) => t.finish(&self.options, self.pushed, sink, elapsed),
            Codec::Ac3(a) => a.finish(&self.options, self.pushed, sink, elapsed),
        }
    }

    /// Tells `sink` everything decoded so far: what waits for the checks of
    /// `verify` is waited for. A caller that is about to wait for more of the
    /// stream says it, so that what was decoded is out before then;
    /// [`decode_reader`] does. Nothing else waits for it: what is told is
    /// the same whether it is said or not.
    ///
    /// # Errors
    /// What the sink gives, and when a call before this one ended with an
    /// error.
    pub fn flush(&mut self, sink: &mut dyn StreamSink) -> Result<()> {
        self.going()?;
        let flushed = match &mut self.codec {
            Some(Codec::TrueHd(t)) => t.tell(sink, true),
            Some(Codec::Ac3(_)) | None => Ok(()),
        };
        self.failed = flushed.is_err();
        flushed
    }

    /// An error where a call before ended with one.
    fn going(&self) -> Result<()> {
        if self.failed {
            anyhow::bail!(
                "the decode of {} ended with an error, and takes nothing more",
                self.options.name
            );
        }
        Ok(())
    }

    /// Tells the codec apart by the first bytes of the stream, and refuses
    /// what the options ask that does not apply to it, in the words a decode
    /// of a file refuses it in.
    fn begin(&self, head: &[u8]) -> Result<Codec> {
        let pcm = &self.options.pcm;
        Ok(if eac3::is_eac3_head(head) {
            if pcm.on_error.is_some() {
                return Err(refusal(ON_ERROR_IS_TRUEHD));
            }
            if pcm.start.is_some() {
                return Err(refusal(START_IS_TRUEHD_SAMPLES));
            }
            if pcm.keep_duplicates {
                return Err(refusal(KEEP_DUPLICATES_IS_TRUEHD));
            }
            if pcm.presentation.is_some() {
                return Err(refusal(PRESENTATION_IS_TRUEHD));
            }
            Codec::Ac3(Box::new(Ac3::new(&self.options)))
        } else {
            if pcm.core_only || !pcm.dither || !pcm.tpnp || pcm.ecpl_full {
                return Err(refusal(CORE_DECODE_IS_AC3));
            }
            if pcm.programme.is_some() {
                return Err(refusal(PROGRAMME_IS_AC3));
            }
            if self.options.on_layout_change == Some(LayoutChange::Fixed) {
                return Err(refusal(FIXED_LAYOUT_IS_EAC3));
            }
            Codec::TrueHd(Box::new(TrueHd::new(&self.options, self.pace)))
        })
    }
}

/// Feeds `bytes` to the decode of the codec told apart.
fn feed(
    codec: &mut Codec,
    bytes: &[u8],
    walking: &mut Walking,
    sink: &mut dyn StreamSink,
) -> Result<()> {
    match codec {
        Codec::TrueHd(t) => t.push(bytes, walking, sink),
        Codec::Ac3(a) => a.push(bytes, walking, sink),
    }
}

/// The findings a tally lists that were not told yet.
#[derive(Debug, Default)]
struct Listed {
    told: BTreeMap<&'static str, usize>,
}

impl Listed {
    /// What `tally` lists that was not told before, in the order of the
    /// catalog and then of the listing; a finding with no unit or byte of its
    /// own is in `unit`, at `offset`.
    fn new_in<R: Rule>(&mut self, tally: &Tally<R>, unit: u64, offset: u64) -> Vec<Finding> {
        let mut found = Vec::new();
        for (rule, t) in tally.iter() {
            let id = rule.info().id;
            let told = self.told.entry(id).or_default();
            for f in &t.listed[*told..] {
                found.push(Finding {
                    unit: f.at.unit.unwrap_or(unit),
                    byte_offset: f.at.offset.unwrap_or(offset),
                    rule: id.to_string(),
                    message: f.message.clone(),
                });
            }
            *told = t.listed.len();
        }
        found
    }
}

/// The lines `remarks` holds, as events.
fn remarks(said: &[u8], into: &mut Vec<StreamEvent>) {
    let text = String::from_utf8_lossy(said);
    into.extend(text.lines().map(|l| StreamEvent::Remark(l.to_string())));
}

/// Where a TrueHD delivery writes a unit: kept, to be told in order.
#[derive(Debug, Default)]
struct Kept {
    wave: bool,
    spec: Option<WavSpec>,
    bytes: Vec<u8>,
}

impl decode::PcmSink for Kept {
    fn states_the_channels(&self) -> bool {
        self.wave
    }

    fn begin(&mut self, spec: WavSpec) -> io::Result<()> {
        self.spec = Some(spec);
        Ok(())
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn finish(&mut self) -> io::Result<()> {
        Ok(())
    }

    fn nothing(&mut self) {}
}

/// One access unit decoded and waiting for its checks.
#[derive(Debug)]
struct Waiting {
    unit: u64,
    events: Vec<StreamEvent>,
    pcm: Vec<u8>,
}

/// The checks of `verify` over the units of a TrueHD stream, on a thread of
/// their own.
#[derive(Debug)]
struct Checks {
    to: Option<SyncSender<Vec<Unit>>>,
    from: Receiver<Vec<Vec<Finding>>>,
    thread: Option<std::thread::JoinHandle<Box<Scan>>>,
    /// Units that go to the checks together, once there are [`BATCH`].
    batch: Vec<Unit>,
    /// The findings of each unit the checks have told and the decode has
    /// not, in order.
    found: VecDeque<Vec<Finding>>,
}

impl Checks {
    fn spawn(name: &str, pace: Option<Duration>) -> Self {
        let (to, batches) = mpsc::sync_channel::<Vec<Unit>>(UNITS_AHEAD / BATCH);
        let (found, from) = mpsc::channel();
        let name = name.to_string();
        let thread = std::thread::spawn(move || {
            let mut scan = Box::new(Scan::named(
                &name,
                oadec_truehd::Tally::DEFAULT_MAX_PER_RULE,
            ));
            let mut since_major_sync = 0;
            let mut listed = Listed::default();
            for units in batches {
                if let Some(pace) = pace {
                    std::thread::sleep(pace);
                }
                let mut told = Vec::with_capacity(units.len());
                for unit in &units {
                    let index = scan.units;
                    scan.take_unit(unit, &mut since_major_sync);
                    told.push(listed.new_in(&scan.diag.tally, index, unit.offset));
                }
                if found.send(told).is_err() {
                    break;
                }
            }
            scan
        });
        Self {
            to: Some(to),
            from,
            thread: Some(thread),
            batch: Vec::with_capacity(BATCH),
            found: VecDeque::new(),
        }
    }

    /// The next unit, checked with the units of its batch.
    fn take(&mut self, unit: Unit) -> Result<()> {
        self.batch.push(unit);
        if self.batch.len() >= BATCH {
            self.send()?;
        }
        Ok(())
    }

    /// The units of the batch go to the checks, however many there are.
    fn send(&mut self) -> Result<()> {
        if self.batch.is_empty() {
            return Ok(());
        }
        let batch = std::mem::replace(&mut self.batch, Vec::with_capacity(BATCH));
        let sent = self.to.as_ref().map(|to| to.send(batch));
        if matches!(sent, Some(Ok(()))) {
            Ok(())
        } else {
            Err(self.failed())
        }
    }

    /// The findings of the next unit: waiting for them, or `None` where they
    /// are not in yet. With `all`, the units that wait for a batch are sent
    /// first; without, the next unit is in a batch that was sent, because
    /// more units wait than a batch holds, and a batch is not cut short for
    /// it: once the checks are the slower, cut short they would go one unit
    /// at a time.
    fn next(&mut self, wait: bool, all: bool) -> Result<Option<Vec<Finding>>> {
        if let Some(found) = self.found.pop_front() {
            return Ok(Some(found));
        }
        let told = if wait {
            if all {
                self.send()?;
            }
            self.from.recv().map_err(|_| self.failed())?
        } else {
            match self.from.try_recv() {
                Ok(told) => told,
                Err(TryRecvError::Empty) => return Ok(None),
                Err(TryRecvError::Disconnected) => return Err(self.failed()),
            }
        };
        self.found.extend(told);
        Ok(self.found.pop_front())
    }

    /// The scan, once every unit was sent; `None` where it was joined
    /// before. A panic of the checks is the panic of whoever joins them.
    fn join(&mut self) -> Option<Box<Scan>> {
        drop(self.to.take());
        let thread = self.thread.take()?;
        Some(
            thread
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        )
    }

    /// The thread of the checks ended before its units did: it panicked,
    /// which is told where it is joined.
    fn failed(&mut self) -> anyhow::Error {
        let _ = self.join();
        anyhow::anyhow!("the checks of the stream ended early")
    }
}

impl Drop for Checks {
    fn drop(&mut self) {
        drop(self.to.take());
        if let Some(thread) = self.thread.take() {
            // the decode is on its way out with an error of its own
            let _ = thread.join();
        }
    }
}

/// A TrueHD or MLP stream as it arrives.
#[derive(Debug)]
struct TrueHd {
    extractor: Extractor,
    opts: decode::Options,
    /// A change of configuration is followed ([`LayoutChange::Split`]).
    follow: bool,
    delivery: Option<Delivery>,
    checks: Checks,
    waiting: VecDeque<Waiting>,
    /// Units framed so far.
    units: u64,
    format: Format,
    opened: Option<(Layout, u32)>,
    kept: Kept,
    said: Vec<u8>,
    /// Buffers of samples that were told, to be written into again: a unit
    /// is a few hundred samples, and there are a million of them in an hour.
    spare: Vec<Vec<u8>>,
    /// The most units that waited for their checks at once.
    waiting_high: usize,
}

impl TrueHd {
    fn new(options: &StreamOptions, pace: Option<Duration>) -> Self {
        let pcm = &options.pcm;
        Self {
            extractor: Extractor::new(),
            follow: options.on_layout_change == Some(LayoutChange::Split),
            opts: decode::Options {
                presentation: pcm.presentation,
                format: decode::Format::Pcm,
                order: pcm.order,
                keep_duplicates: pcm.keep_duplicates,
                start: pcm.start,
                on_error: pcm.on_error.unwrap_or(OnError::Conceal),
            },
            delivery: None,
            checks: Checks::spawn(&options.name, pace),
            waiting: VecDeque::new(),
            units: 0,
            format: Format::TrueHd,
            opened: None,
            kept: Kept {
                wave: options.wave,
                ..Kept::default()
            },
            said: Vec::new(),
            spare: Vec::new(),
            waiting_high: 0,
        }
    }

    fn push(
        &mut self,
        bytes: &[u8],
        walking: &mut Walking,
        sink: &mut dyn StreamSink,
    ) -> Result<()> {
        self.extractor.push(bytes);
        while let Some(unit) = self.extractor.next_unit()? {
            walking.at(unit.offset);
            self.unit(unit, sink)?;
        }
        Ok(())
    }

    fn unit(&mut self, unit: Unit, sink: &mut dyn StreamSink) -> Result<()> {
        let index = self.units;
        self.units += 1;
        let delivery = if let Some(delivery) = &mut self.delivery {
            delivery
        } else {
            let presentation = self
                .opts
                .presentation
                .unwrap_or_else(|| decode::default_presentation_of(Some(&unit)));
            if unit.has_major_sync
                && unit.bytes.get(4..8) == Some(&oadec_truehd::sync::SYNC_FBB.to_be_bytes()[..])
            {
                self.format = Format::Mlp;
            }
            let mut delivery = Delivery::new(&self.opts, presentation)?;
            if self.follow {
                delivery.session.follow_changes();
            }
            self.delivery.insert(delivery)
        };
        self.kept.bytes.clear();
        self.said.clear();
        let opened_before = self.kept.spec.is_some();
        let taken = match delivery.take(&unit, &mut self.kept, &mut self.said) {
            Ok(taken) => taken,
            Err(e) => {
                // what was decoded before goes out before the error does
                self.tell(sink, true)?;
                return Err(e);
            }
        };
        let mut events = Vec::new();
        remarks(&self.said, &mut events);
        if !opened_before && let Some(spec) = self.kept.spec {
            let layout = Layout {
                channels: spec.channels,
                mask: spec.mask,
                sample: spec.sample,
            };
            self.opened = Some((layout, spec.rate));
            events.push(StreamEvent::Opened {
                format: self.format,
                layout,
                rate: spec.rate,
            });
        }
        if taken == Taken::Stopped
            && let (Some(stop), Some((old, old_rate))) = (delivery.stopped, self.opened)
        {
            events.push(StreamEvent::ConfigChange {
                unit: index,
                byte_offset: stop.offset,
                what: stop.what.to_string(),
                policy: LayoutChange::Stop,
                old,
                old_rate,
                new: None,
                new_rate: None,
            });
        }
        if let (Taken::Changed | Taken::Gone, Some((old, old_rate))) = (taken, self.opened) {
            let what = delivery.session.last_change.unwrap_or("the configuration");
            let new = (taken == Taken::Changed)
                .then_some(self.kept.spec)
                .flatten()
                .map(|spec| {
                    (
                        Layout {
                            channels: spec.channels,
                            mask: spec.mask,
                            sample: spec.sample,
                        },
                        spec.rate,
                    )
                });
            events.push(StreamEvent::ConfigChange {
                unit: index,
                byte_offset: unit.offset,
                what: if new.is_some() {
                    what.to_string()
                } else {
                    format!("{what}, and the presentation is not in the new configuration")
                },
                policy: LayoutChange::Split,
                old,
                old_rate,
                new: new.map(|(layout, _)| layout),
                new_rate: new.map(|(_, rate)| rate),
            });
            if let Some(new) = new {
                self.opened = Some(new);
            }
        }
        let pcm = if matches!(taken, Taken::Written | Taken::Changed) {
            let spare = self.spare.pop().unwrap_or_default();
            std::mem::replace(&mut self.kept.bytes, spare)
        } else {
            Vec::new()
        };
        self.waiting.push_back(Waiting {
            unit: index,
            events,
            pcm,
        });
        self.waiting_high = self.waiting_high.max(self.waiting.len());
        // decoded, the unit goes to the checks
        self.checks.take(unit)?;
        self.tell(sink, false)
    }

    /// Tells `sink` every unit whose checks are in, in order: its findings,
    /// its events, its samples. With `all`, or with too many units waiting,
    /// it waits for the checks.
    fn tell(&mut self, sink: &mut dyn StreamSink, all: bool) -> Result<()> {
        while !self.waiting.is_empty() {
            let wait = all || self.waiting.len() > UNITS_AHEAD;
            let Some(found) = self.checks.next(wait, all)? else {
                return Ok(());
            };
            let Some(unit) = self.waiting.pop_front() else {
                return Ok(());
            };
            for f in found {
                sink.event(&StreamEvent::Finding(f))?;
            }
            for e in &unit.events {
                sink.event(e)?;
            }
            if !unit.pcm.is_empty() {
                sink.pcm(PcmFrame {
                    unit: unit.unit,
                    bytes: &unit.pcm,
                })?;
                if self.spare.len() < BATCH {
                    let mut spare = unit.pcm;
                    spare.clear();
                    self.spare.push(spare);
                }
            }
        }
        Ok(())
    }

    fn finish(
        mut self,
        options: &StreamOptions,
        pushed: u64,
        sink: &mut dyn StreamSink,
        elapsed: impl Fn() -> f64,
    ) -> Result<StreamEnd> {
        let (rest, trailing_bytes) = self.extractor.finish()?;
        for unit in rest {
            self.unit(unit, sink)?;
        }
        self.tell(sink, true)?;
        let pass = PassSummary {
            stats: self.extractor.stats,
            trailing_bytes,
            final_gap: self.extractor.take_gap(),
            file_bytes: pushed,
        };
        let mut scan = self
            .checks
            .join()
            .ok_or_else(|| anyhow::anyhow!("the checks of the stream ended early"))?;
        let mut listed = Listed::default();
        // what was told already is told again by nobody
        let _ = listed.new_in(&scan.diag.tally, 0, 0);
        scan.finish(pass);
        for f in listed.new_in(&scan.diag.tally, self.units, pushed) {
            sink.event(&StreamEvent::Finding(f))?;
        }
        let check = crate::verify::truehd_check_of(*scan, self.follow);
        let Some(delivery) = self.delivery else {
            return Err(crate::info::no_stream_named(&options.name));
        };
        if delivery.session.stats().is_none() {
            // a start that was never reached delivers nothing either, and is
            // told so by its own cause
            if let Some(complaint) = delivery.session.start_never_reached() {
                return Err(crate::calls::no_stream(complaint));
            }
            return Err(crate::info::no_stream_named(&options.name));
        }
        let summary = delivery.session.summary();
        let mut findings = decode::truehd_findings(&pass, summary.stats.as_ref(), &check);
        findings.note(summary.clipped, "samples clipped to 24 bits");
        decode::note_concealment(&summary.concealment, &mut findings);
        if let Some(stop) = &delivery.stopped {
            stop.note(&mut findings);
        }
        Ok(StreamEnd {
            format: self.format,
            findings,
            samples: delivery.samples,
            units: self.units,
            bytes: pushed,
            queue: QueueStats {
                units_bound: UNITS_AHEAD + 1,
                units_high_water: self.waiting_high,
                ..QueueStats::default()
            },
            told: Told::TrueHd {
                summary: Box::new(summary),
                stopped: delivery.stopped,
                elapsed: elapsed(),
            },
        })
    }
}

/// What the frame groups of an AC-3 family stream come to, kept to be told
/// in order.
#[derive(Debug)]
struct Groups {
    core_only: bool,
    order_kind: decode::Order,
    policy: LayoutChange,
    opened: Option<(Layout, u32)>,
    order: Vec<usize>,
    /// Where each independent frame starts, by its count from 0: where the
    /// frame group of that index starts.
    starts: VecDeque<(u64, u64)>,
    independents: u64,
    changed: bool,
    stopped: Option<(u64, u64)>,
    samples: u64,
    groups: u64,
    /// What the groups of the frame being taken came to.
    told: Vec<Out>,
}

/// What a frame group came to: an event, or the samples of a group.
#[derive(Debug)]
enum Out {
    Event(StreamEvent),
    Pcm(u64, Vec<u8>),
}

impl Groups {
    /// Where frame group `index` starts.
    fn start_of(&mut self, index: u64) -> u64 {
        while self.starts.len() > 1 && self.starts.front().is_some_and(|&(k, _)| k < index) {
            self.starts.pop_front();
        }
        self.starts.front().map_or(0, |&(_, at)| at)
    }

    fn event(&mut self, event: StreamEvent) {
        self.told.push(Out::Event(event));
    }

    fn group(&mut self, frame: &oadec_eac3::ProgramFrame) -> Result<()> {
        self.groups = self.groups.max(frame.index + 1);
        // the starts of the groups before this one are given up here, so
        // they are held for the groups in flight and no longer
        let start = self.start_of(frame.index);
        let (chans, pcm) = eac3::output_channels(frame, self.core_only);
        let rate = frame.core().header.sample_rate;
        let (old, old_rate) = if let Some(opened) = self.opened {
            opened
        } else {
            self.order = eac3::output_order(&chans, self.order_kind);
            let ordered: Vec<_> = self.order.iter().map(|&i| chans[i]).collect();
            let (mask, warning) = eac3::channel_mask(&ordered);
            if let Some(warning) = warning {
                self.event(StreamEvent::Remark(warning));
            }
            let layout = Layout {
                channels: chans.len() as u16,
                mask,
                sample: WavSample::Float32,
            };
            self.opened = Some((layout, rate));
            self.event(StreamEvent::Opened {
                format: match frame.core().header.syntax {
                    oadec_eac3::Syntax::Ac3 => Format::Ac3,
                    oadec_eac3::Syntax::Eac3 => Format::Eac3,
                },
                layout,
                rate,
            });
            (layout, rate)
        };
        if self.policy == LayoutChange::Split {
            // each group has its own channels: a group whose channels differ
            // from the group before it begins the output anew
            if frame.layout_changed {
                self.order = eac3::output_order(&chans, self.order_kind);
                let ordered: Vec<_> = self.order.iter().map(|&i| chans[i]).collect();
                let (mask, warning) = eac3::channel_mask(&ordered);
                if let Some(warning) = warning {
                    self.event(StreamEvent::Remark(warning));
                }
                let layout = Layout {
                    channels: chans.len() as u16,
                    mask,
                    sample: WavSample::Float32,
                };
                self.event(StreamEvent::ConfigChange {
                    unit: frame.index,
                    byte_offset: start,
                    what: "the channels of the programme".to_string(),
                    policy: LayoutChange::Split,
                    old,
                    old_rate,
                    new: Some(layout),
                    new_rate: Some(rate),
                });
                self.opened = Some((layout, rate));
            }
        } else if !self.core_only {
            if frame.layout_changed && !self.changed {
                self.changed = true;
                let byte_offset = start;
                self.event(StreamEvent::ConfigChange {
                    unit: frame.index,
                    byte_offset,
                    what: "the channels of the programme".to_string(),
                    policy: self.policy,
                    old,
                    old_rate,
                    new: None,
                    new_rate: Some(rate),
                });
                if self.policy == LayoutChange::Stop && self.stopped.is_none() {
                    self.stopped = Some((frame.index, byte_offset));
                }
            } else if !frame.layout_changed {
                self.changed = false;
            }
        }
        if self.stopped.is_some() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        eac3::interleave(&mut bytes, &pcm, &self.order, frame.samples());
        self.samples += frame.samples() as u64;
        self.told.push(Out::Pcm(frame.index, bytes));
        Ok(())
    }
}

/// An AC-3 or E-AC-3 stream as it arrives.
#[derive(Debug)]
struct Ac3 {
    splitter: Splitter,
    walk: Walk,
    groups: Groups,
    listed: Listed,
}

impl Ac3 {
    fn new(options: &StreamOptions) -> Self {
        let pcm = &options.pcm;
        Self {
            splitter: Splitter::new(eac3::READ_BYTES),
            walk: Walk::new(
                oadec_eac3::Options {
                    dither: pcm.dither,
                    tpnp: pcm.tpnp,
                    ecpl_full: pcm.ecpl_full,
                    block_info: false,
                    samples: true,
                },
                oadec_eac3::ProgramConfig {
                    programme: pcm.programme,
                    layout: if options.on_layout_change == Some(LayoutChange::Split) {
                        oadec_eac3::LayoutPolicy::Follow
                    } else {
                        oadec_eac3::LayoutPolicy::Fixed
                    },
                },
                Tally::<oadec_eac3::RuleId>::DEFAULT_MAX_PER_RULE,
            ),
            groups: Groups {
                core_only: pcm.core_only,
                order_kind: pcm.order,
                policy: options.on_layout_change.unwrap_or(LayoutChange::Fixed),
                opened: None,
                order: Vec::new(),
                starts: VecDeque::new(),
                independents: 0,
                changed: false,
                stopped: None,
                samples: 0,
                groups: 0,
                told: Vec::new(),
            },
            listed: Listed::default(),
        }
    }

    fn push(
        &mut self,
        bytes: &[u8],
        walking: &mut Walking,
        sink: &mut dyn StreamSink,
    ) -> Result<()> {
        self.splitter.push(bytes);
        self.split(walking, sink)
    }

    /// Takes whatever the splitter can tell, and tells `sink`.
    fn split(&mut self, walking: &mut Walking, sink: &mut dyn StreamSink) -> Result<()> {
        let walk = &mut self.walk;
        loop {
            let offset = match self.splitter.next() {
                Split::Wait | Split::End => return Ok(()),
                Split::Gap {
                    offset,
                    sync_error,
                    bytes,
                } => {
                    walk.gap(offset, sync_error, bytes);
                    offset
                }
                Split::Frame {
                    offset,
                    bytes,
                    header,
                } => {
                    walking.at(offset);
                    if header.stream_type != oadec_eac3::StreamType::Dependent {
                        let k = self.groups.independents;
                        self.groups.independents += 1;
                        self.groups.starts.push_back((k, offset));
                    }
                    let groups = &mut self.groups;
                    walk.frame(offset, bytes, &header, &mut |frame| groups.group(frame))?;
                    offset
                }
            };
            let found = self.listed.new_in(walk.diag(), self.groups.groups, offset);
            tell_groups(&mut self.groups, found, sink)?;
        }
    }

    fn finish(
        mut self,
        options: &StreamOptions,
        pushed: u64,
        sink: &mut dyn StreamSink,
        elapsed: impl Fn() -> f64,
    ) -> Result<StreamEnd> {
        self.splitter.finish();
        let mut nobody = Walking::of(None, "reading", 0);
        self.split(&mut nobody, sink)?;
        let Self {
            splitter,
            walk,
            mut groups,
            mut listed,
        } = self;
        let pass = walk.end(splitter.sync_errors, splitter.skipped, &mut |frame| {
            groups.group(frame)
        })?;
        let found = listed.new_in(pass.0.diag(), groups.groups, pushed);
        tell_groups(&mut groups, found, sink)?;
        let Some(report) = eac3::Report::delivered(&options.name, pass, elapsed()) else {
            return Err(crate::info::no_stream_named(&options.name));
        };
        Ok(StreamEnd {
            format: match report.syntax() {
                Some(oadec_eac3::Syntax::Ac3) => Format::Ac3,
                _ => Format::Eac3,
            },
            findings: report.findings(),
            samples: groups.samples,
            units: groups.groups,
            bytes: pushed,
            queue: QueueStats::default(),
            told: Told::Ac3 {
                report: Box::new(report),
                stopped: groups.stopped,
            },
        })
    }
}

/// Tells `sink` the findings of a frame, then what its groups came to.
fn tell_groups(groups: &mut Groups, found: Vec<Finding>, sink: &mut dyn StreamSink) -> Result<()> {
    for f in found {
        sink.event(&StreamEvent::Finding(f))?;
    }
    for out in groups.told.drain(..) {
        match out {
            Out::Pcm(unit, bytes) => sink.pcm(PcmFrame {
                unit,
                bytes: &bytes,
            })?,
            Out::Event(event) => sink.event(&event)?,
        }
    }
    Ok(())
}

/// Reads `from` to its end on a thread of its own and decodes what it reads
/// into `sink`, with a queue of at most [`QUEUE_PIECES`] pieces between the
/// two. The reader is read once, and nothing is read that the decode does
/// not take.
///
/// When the decode ends with an error before the reader has ended, the
/// reader is not waited for: it stops when it next has a piece to put in
/// the queue. A read that does not return, of a pipe or a socket whose
/// other end stays open and silent, holds its thread, and the reader with
/// it, until the read returns; nothing safe can cut a read short. A caller
/// that keeps running after an error, as a process that ends does not, is
/// to close the source, the other end of the pipe or the socket, and the
/// thread then ends.
///
/// # Errors
/// What reading gives, and what [`StreamDecoder::push`] and
/// [`StreamDecoder::finish`] give.
pub fn decode_reader(
    from: Box<dyn Read + Send>,
    options: StreamOptions,
    sink: &mut dyn StreamSink,
) -> Result<StreamEnd> {
    let name = options.name.clone();
    let (to, pieces) = mpsc::sync_channel::<io::Result<Vec<u8>>>(QUEUE_PIECES);
    let sent = Arc::new(AtomicUsize::new(0));
    let read = Arc::new(AtomicU64::new(0));
    let reader = {
        let (sent, read) = (Arc::clone(&sent), Arc::clone(&read));
        std::thread::spawn(move || {
            let mut from = from;
            loop {
                let mut piece = vec![0u8; PIECE];
                let n = match from.read(&mut piece) {
                    Ok(0) => return,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = to.send(Err(e));
                        return;
                    }
                };
                piece.truncate(n);
                read.fetch_add(n as u64, Ordering::Relaxed);
                if to.send(Ok(piece)).is_err() {
                    return;
                }
                sent.fetch_add(1, Ordering::Release);
            }
        })
    };
    let mut decoder = StreamDecoder::new(options);
    let mut received = 0usize;
    let mut high_water = 0usize;
    loop {
        // what the queue holds as the decode comes to it
        let held = sent.load(Ordering::Acquire).saturating_sub(received);
        high_water = high_water.max(held);
        let piece = match pieces.try_recv() {
            Ok(piece) => piece,
            Err(TryRecvError::Empty) => {
                decoder.flush(sink)?;
                sink.flush()?;
                match pieces.recv() {
                    Ok(piece) => piece,
                    Err(_) => break,
                }
            }
            Err(TryRecvError::Disconnected) => break,
        };
        received += 1;
        let piece = piece.with_context(|| format!("reading {name}"))?;
        decoder.push(&piece, sink)?;
    }
    let _ = reader.join();
    let mut end = decoder.finish(sink)?;
    sink.flush()?;
    end.queue.bound = QUEUE_PIECES;
    end.queue.high_water = high_water;
    debug_assert_eq!(end.bytes, read.load(Ordering::Relaxed));
    Ok(end)
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
    let mut out = Vec::with_capacity(STREAM_WAVE_HEADER_LEN);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&UNKNOWN_LENGTH.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(&decode::wave_fmt_chunk(spec));
    out.extend_from_slice(b"data");
    out.extend_from_slice(&UNKNOWN_LENGTH.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TrueHD stream of 2 400 access units, with four bytes changed.
    fn damaged_truehd() -> Vec<u8> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../oadec-cli/tests/fixtures/authored-scene.mlp"
        );
        let mut bytes = std::fs::read(path).expect("the fixture");
        for at in [10_000, 100_000, 200_000, 300_000] {
            bytes[at] ^= 0x41;
        }
        bytes
    }

    /// What a sink was told, in one list.
    #[derive(Debug, Default)]
    struct InOrder(Vec<Told>);

    #[derive(Debug, PartialEq)]
    enum Told {
        Finding(Finding),
        Pcm(u64),
    }

    impl StreamSink for InOrder {
        fn event(&mut self, event: &StreamEvent) -> Result<()> {
            if let StreamEvent::Finding(f) = event {
                self.0.push(Told::Finding(f.clone()));
            }
            Ok(())
        }

        fn pcm(&mut self, frame: PcmFrame<'_>) -> Result<()> {
            self.0.push(Told::Pcm(frame.unit));
            Ok(())
        }
    }

    /// The findings the checks make at each access unit of `bytes`, by the
    /// unit they are made at, from a scan of the units outside the decode.
    fn made_at(bytes: &[u8]) -> Vec<Vec<Finding>> {
        let mut extractor = Extractor::new();
        extractor.push(bytes);
        let mut units = Vec::new();
        while let Some(unit) = extractor.next_unit().expect("framed") {
            units.push(unit);
        }
        units.extend(extractor.finish().expect("framed").0);
        let mut scan = Scan::named("", oadec_truehd::Tally::DEFAULT_MAX_PER_RULE);
        let (mut since, mut listed) = (0, Listed::default());
        units
            .iter()
            .map(|unit| {
                let index = scan.units;
                scan.take_unit(unit, &mut since);
                listed.new_in(&scan.diag.tally, index, unit.offset)
            })
            .collect()
    }

    /// The findings the checks make at a unit are told before the samples of
    /// that unit and after the samples of the units before it. A finding
    /// says which unit it is about, which can be the unit before the one it
    /// was made at, so the unit it was made at is taken from a scan of the
    /// same units outside the decode.
    #[test]
    fn the_findings_made_at_a_unit_are_told_before_its_samples() {
        let bytes = damaged_truehd();
        let made = made_at(&bytes);
        let expected: Vec<(u64, &Finding)> = made
            .iter()
            .enumerate()
            .flat_map(|(k, found)| found.iter().map(move |f| (k as u64, f)))
            .collect();
        assert!(expected.len() > 4, "damage is found");
        let mut told = InOrder::default();
        let mut decoder = StreamDecoder::new(StreamOptions::new("<stdin>"));
        for piece in bytes.chunks(70_000) {
            decoder.push(piece, &mut told).expect("pushed");
        }
        decoder.finish(&mut told).expect("decoded");
        let (mut next, mut last_pcm) = (0, None::<u64>);
        for event in &told.0 {
            match event {
                Told::Finding(f) if next < expected.len() => {
                    let (at, made) = expected[next];
                    assert_eq!(f, made, "the findings are the checks' own, in order");
                    assert!(
                        last_pcm.is_none_or(|u| u < at),
                        "a finding made at unit {at} after the samples of unit {last_pcm:?}"
                    );
                    next += 1;
                }
                // what the end of the stream adds, after every unit
                Told::Finding(_) => {}
                Told::Pcm(unit) => {
                    assert!(
                        expected.get(next).is_none_or(|&(at, _)| at > *unit),
                        "the samples of unit {unit} before a finding made at it"
                    );
                    last_pcm = Some(*unit);
                }
            }
        }
        assert_eq!(next, expected.len(), "every finding was told");
    }

    /// Checks slower than the decode hold it back: no more decoded units
    /// wait for them than the bound, and with checks this slow as many do.
    #[test]
    fn slow_checks_hold_the_decode_to_its_bound() {
        let bytes = damaged_truehd();
        let mut told = InOrder::default();
        let mut decoder = StreamDecoder::new(StreamOptions::new("<stdin>"))
            .with_slow_checks(Duration::from_millis(15));
        for piece in bytes.chunks(64 << 10) {
            decoder.push(piece, &mut told).expect("pushed");
        }
        let end = decoder.finish(&mut told).expect("decoded");
        assert_eq!(end.queue.units_bound, UNITS_AHEAD + 1);
        assert_eq!(
            end.queue.units_high_water, end.queue.units_bound,
            "{:?}",
            end.queue
        );
        assert_eq!(end.units, 2400);
    }

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
