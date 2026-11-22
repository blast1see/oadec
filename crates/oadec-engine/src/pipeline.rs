//! One pass over a stream that is read once, handed unit by unit to the
//! stages that each do one part of a command.
//!
//! A [`Source`](crate::source::Source) opens again at its beginning as often
//! as it is asked, and the commands that report on a stream use that: they
//! read its head to tell the codecs apart, run the checks of `verify` in a
//! pass of their own on a second thread, and walk the stream for their own
//! report in a third. A pipe can be read once, so standard input is taken
//! into memory whole before such a command starts.
//!
//! What is here reads a stream once and on the thread that calls. The head
//! is held back in a [`Once`] to tell the codecs apart and then read again
//! from memory; the bytes are framed into access units or syncframes by the
//! framer a walk over a file uses; and every unit is handed to every
//! [`Stage`] in the order the stages were given, before the next is framed.
//! Nothing is read ahead of the stages but one read of the framer and the
//! head, so what the pass holds does not grow with the stream.
//!
//! # Who owns what
//!
//! A unit is lent to a stage for the length of one call of [`Stage::unit`]:
//! its bytes are in the buffer of the framer, which takes the next read into
//! the same memory. A stage that wants to keep something of a unit copies it
//! into a state of its own. The compiler holds a stage to that: the bytes of
//! a [`UnitRef`] have a lifetime that ends with the call, and a stage that
//! tries to keep them does not build.
//!
//! ```compile_fail
//! use oadec_engine::pipeline::{PassEnd, Stage, UnitRef};
//!
//! /// Keeps the bytes of the last unit it was lent, which it may not.
//! struct Keeper<'k> {
//!     last: Option<&'k [u8]>,
//! }
//!
//! impl<'k> Stage for Keeper<'k> {
//!     fn unit(&mut self, unit: &UnitRef<'_>) -> anyhow::Result<()> {
//!         // the borrow of the unit ends with this call: `'k` outlives it
//!         self.last = Some(unit.bytes);
//!         Ok(())
//!     }
//!
//!     fn finish(&mut self, _: &PassEnd) -> anyhow::Result<()> {
//!         Ok(())
//!     }
//! }
//! ```
//!
//! The same stage that copies what it keeps builds; that it does is what
//! shows the stage above fails for the borrow and for nothing else:
//!
//! ```
//! use oadec_engine::pipeline::{PassEnd, Stage, UnitRef};
//!
//! /// Keeps a copy of the bytes of the last unit it was lent.
//! struct Keeper {
//!     last: Option<Vec<u8>>,
//! }
//!
//! impl Stage for Keeper {
//!     fn unit(&mut self, unit: &UnitRef<'_>) -> anyhow::Result<()> {
//!         self.last = Some(unit.bytes.to_vec());
//!         Ok(())
//!     }
//!
//!     fn finish(&mut self, _: &PassEnd) -> anyhow::Result<()> {
//!         Ok(())
//!     }
//! }
//! # let _ = Keeper { last: None };
//! ```
//!
//! What copies is the stage's to choose, and so is what it costs: a count
//! keeps nothing, a scan keeps its findings, a decode of a presentation on a
//! thread of its own keeps whole units, which it owns (`Arc`) and sends.
//!
//! # When something goes wrong
//!
//! - **A stage gives an error.** The pass ends at that unit: the stages after
//!   it in the list are not lent the unit, no stage is lent another one, no
//!   stage is told [`Stage::finish`], and the error is what [`Pass::run`]
//!   gives, as it is. A stage that ends a walk on purpose (`oadec au`, once
//!   it has described its unit) does so with an error of its own type, which
//!   its caller recognises, as the walks over a [`Source`](crate::source::Source)
//!   do today.
//! - **Reading gives an error.** The pass ends with that error, with the
//!   name of the stream in its context, and no stage is told it has
//!   finished: what a walk over a file does when the file cannot be read, and
//!   what the command line ends with exit code 2 for. A read that is
//!   interrupted ([`std::io::ErrorKind::Interrupted`]) is read again.
//! - **The stream ends inside a unit.** That is no error of the pass: the
//!   end is told to every stage in [`PassEnd`], with the bytes that formed no
//!   unit, and each stage reports them as it reports them in a file that
//!   ends there (exit code 7 for the commands that judge).
//! - **A panic** is not caught, as it is not today.
//!
//! The pass has no thread, no channel and nothing shared, so the order in
//! which the stages are told is the order of the stream and of the list,
//! whatever the size of the reads: a stream gives the same stages the same
//! calls however it was cut into pieces.

use std::io::{self, Read};
use std::sync::Arc;

use anyhow::Result;
use oadec_eac3::FrameHeader;
use oadec_truehd::{Extractor, Unit};

use crate::eac3::{Split, Splitter};
use crate::input::PassSummary;
use crate::source::{Walking, Watcher};

/// How much of the head of a stream is held back to tell the codecs apart:
/// what a walk over a file reads of its head to do the same.
pub const HEAD: usize = crate::eac3::SNIFF_BYTES;

/// The two families of stream a pass frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Dolby TrueHD or MLP: access units.
    TrueHd,
    /// AC-3 or E-AC-3: syncframes.
    Ac3,
}

/// A reader whose first bytes were read to tell the codecs apart, and are
/// read again from memory before the rest.
///
/// The head is up to [`HEAD`] bytes. A file gives them in one read; a pipe
/// gives what was written into it so far, so the head is read until it is
/// whole or the stream has ended, and the codec is told from the same bytes
/// a walk over a file of the stream tells it from.
#[derive(Debug)]
pub struct Once<R> {
    head: Vec<u8>,
    /// What of `head` was read again.
    at: usize,
    rest: R,
    codec: Codec,
}

impl<R: Read> Once<R> {
    /// Reads the head of `rest` and tells the codec from it.
    ///
    /// # Errors
    /// What reading gives.
    pub fn new(mut rest: R) -> io::Result<Self> {
        let mut head = vec![0u8; HEAD];
        let mut held = 0;
        while held < HEAD {
            match rest.read(&mut head[held..]) {
                Ok(0) => break,
                Ok(n) => held += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        head.truncate(held);
        let codec = if crate::eac3::is_eac3_head(&head) {
            Codec::Ac3
        } else {
            Codec::TrueHd
        };
        Ok(Self {
            head,
            at: 0,
            rest,
            codec,
        })
    }

    /// The codec the head told.
    #[must_use]
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// The bytes held back.
    #[must_use]
    pub fn head(&self) -> &[u8] {
        &self.head
    }

    /// The same stream framed as `codec`, whatever its head told: for a walk
    /// that frames a stream one way whatever it is, as `oadec au` frames
    /// every stream as TrueHD.
    #[must_use]
    pub fn framed_as(mut self, codec: Codec) -> Self {
        self.codec = codec;
        self
    }
}

impl<R: Read> Read for Once<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let held = &self.head[self.at..];
        if held.is_empty() {
            return self.rest.read(buf);
        }
        let n = held.len().min(buf.len());
        buf[..n].copy_from_slice(&held[..n]);
        self.at += n;
        Ok(n)
    }
}

/// What a stage is lent of one unit of the stream.
#[derive(Debug, Clone, Copy)]
pub struct UnitRef<'a> {
    /// Units told before this one: access units, or syncframes (not frame
    /// groups).
    pub index: u64,
    /// Where the unit begins in the stream.
    pub offset: u64,
    /// Its bytes, lent for the call.
    pub bytes: &'a [u8],
    /// What the framer read of it.
    pub head: Head<'a>,
}

/// What the framer read of a unit, by codec.
#[derive(Debug, Clone, Copy)]
pub enum Head<'a> {
    /// A TrueHD access unit as the extractor hands it out: the walks over a
    /// [`Source`](crate::source::Source) and
    /// [`Scan`](crate::scan::Scan)`::take_unit` take this, with what the
    /// extractor passed over in front of it (`gap_before`).
    TrueHd(&'a Unit),
    /// The header of an AC-3 or E-AC-3 syncframe, as the decoder parses it.
    Ac3(&'a FrameHeader),
}

impl<'a> UnitRef<'a> {
    /// The access unit, of a TrueHD stream.
    #[must_use]
    pub fn truehd(&self) -> Option<&'a Unit> {
        match self.head {
            Head::TrueHd(unit) => Some(unit),
            Head::Ac3(_) => None,
        }
    }

    /// The syncframe header, of an AC-3 family stream.
    #[must_use]
    pub fn ac3(&self) -> Option<&'a FrameHeader> {
        match self.head {
            Head::Ac3(header) => Some(header),
            Head::TrueHd(_) => None,
        }
    }
}

/// Bytes of an AC-3 family stream that are no syncframe: what a walk over a
/// file tells its `on_gap` of. In a TrueHD stream what the extractor passed
/// over rides on the next unit ([`Unit::gap_before`]) and on the end
/// ([`PassSummary::final_gap`]), so it is told no gap of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ac3Gap {
    /// Where the bytes begin in the stream.
    pub offset: u64,
    /// The bytes were counted as a sync error.
    pub sync_error: bool,
    /// How many there are; none for the call that closes the pass.
    pub bytes: u64,
}

/// How a pass ended: what every stage is told at the end.
#[derive(Debug, Clone, Copy)]
pub struct PassEnd {
    /// The codec the head told.
    pub codec: Codec,
    /// Bytes read, the head among them: the length of the stream, which a
    /// walk over a file asks the file system for before it begins.
    pub bytes: u64,
    /// Units told.
    pub units: u64,
    /// What the framer counted.
    pub framed: Framed,
}

/// What the framer of a codec counted over the whole pass.
#[derive(Debug, Clone, Copy)]
pub enum Framed {
    /// What a walk over a TrueHD file returns; its `file_bytes` is
    /// [`PassEnd::bytes`].
    TrueHd(PassSummary),
    /// What a walk over an AC-3 family file returns.
    Ac3 {
        /// Syncframes told.
        frames: u64,
        /// Sync errors counted.
        sync_errors: u64,
        /// Bytes skipped.
        skipped: u64,
    },
}

/// One part of what a command does with every unit of a stream.
pub trait Stage {
    /// The next unit, lent for this call.
    ///
    /// # Errors
    /// Whatever ends the pass here; see the module.
    fn unit(&mut self, unit: &UnitRef<'_>) -> Result<()>;

    /// Bytes of an AC-3 family stream that are no syncframe, between the
    /// units around them. A stage that does not count them leaves this out.
    fn gap(&mut self, gap: &Ac3Gap) {
        let _ = gap;
    }

    /// The stream has ended, and every unit was told.
    ///
    /// # Errors
    /// Whatever the stage cannot conclude.
    fn finish(&mut self, end: &PassEnd) -> Result<()>;
}

/// One pass over a stream that is read once.
#[derive(Debug)]
pub struct Pass;

impl Pass {
    /// Reads `input` to its end, frames it, and lends every unit to every
    /// stage in `stages`, in their order; then tells each stage the end, in
    /// the same order. How far the pass has read is told to `watcher` in
    /// the walk "reading", whose total is not known: it is told 0.
    ///
    /// # Errors
    /// What reading gives, with `name` in its context; the first error a
    /// stage gives, as it is. See the module.
    pub fn run<R: Read>(
        mut input: Once<R>,
        name: &str,
        watcher: Option<&Arc<dyn Watcher>>,
        stages: &mut [&mut dyn Stage],
    ) -> Result<PassEnd> {
        let codec = input.codec;
        // Told to whoever watches; the length of a stream read once is not
        // known, so the walk has no total.
        let mut progress = Walking::of(watcher, "reading", 0);
        let mut buf = vec![0u8; READ];
        let mut bytes = 0u64;
        let mut units = 0u64;
        let mut read = |buf: &mut [u8]| -> Result<usize> {
            let n = read_some(&mut input, buf).map_err(|e| read_error(name, e))?;
            bytes += n as u64;
            Ok(n)
        };
        let framed = match codec {
            Codec::TrueHd => {
                let mut extractor = Extractor::new();
                loop {
                    let n = read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    extractor.push(&buf[..n]);
                    while let Some(unit) = extractor.next_unit()? {
                        progress.at(unit.offset);
                        lend_unit(&unit, &mut units, stages)?;
                    }
                }
                let (rest, trailing_bytes) = extractor.finish()?;
                for unit in &rest {
                    lend_unit(unit, &mut units, stages)?;
                }
                Framed::TrueHd(PassSummary {
                    stats: extractor.stats,
                    trailing_bytes,
                    final_gap: extractor.take_gap(),
                    file_bytes: bytes,
                })
            }
            Codec::Ac3 => {
                // the splitter tells what the walk over a file read
                // `READ_BYTES` at a time tells, however the bytes come
                let mut splitter = Splitter::new(crate::eac3::READ_BYTES);
                let mut ended = false;
                loop {
                    match splitter.next() {
                        Split::Frame {
                            offset,
                            bytes: frame,
                            header,
                        } => {
                            progress.at(offset);
                            let unit = UnitRef {
                                index: units,
                                offset,
                                bytes: frame,
                                head: Head::Ac3(&header),
                            };
                            units += 1;
                            for stage in stages.iter_mut() {
                                stage.unit(&unit)?;
                            }
                        }
                        Split::Gap {
                            offset,
                            sync_error,
                            bytes: skipped,
                        } => {
                            let gap = Ac3Gap {
                                offset,
                                sync_error,
                                bytes: skipped,
                            };
                            for stage in stages.iter_mut() {
                                stage.gap(&gap);
                            }
                        }
                        Split::End => break,
                        Split::Wait if ended => break,
                        Split::Wait => {
                            let n = read(&mut buf)?;
                            if n == 0 {
                                splitter.finish();
                                ended = true;
                            } else {
                                splitter.push(&buf[..n]);
                            }
                        }
                    }
                }
                Framed::Ac3 {
                    frames: units,
                    sync_errors: splitter.sync_errors,
                    skipped: splitter.skipped,
                }
            }
        };
        let end = PassEnd {
            codec,
            bytes,
            units,
            framed,
        };
        for stage in stages.iter_mut() {
            stage.finish(&end)?;
        }
        // the line is cleared when `progress` is dropped, which covers the
        // `?` paths above as well as this one
        Ok(end)
    }
}

/// What one read of a pass asks for: what a walk over a file asks for.
const READ: usize = 4 << 20;

/// Lends `unit` to every stage, in order.
fn lend_unit(unit: &Unit, units: &mut u64, stages: &mut [&mut dyn Stage]) -> Result<()> {
    let lent = UnitRef {
        index: *units,
        offset: unit.offset,
        bytes: &unit.bytes,
        head: Head::TrueHd(unit),
    };
    *units += 1;
    for stage in stages.iter_mut() {
        stage.unit(&lent)?;
    }
    Ok(())
}

/// One read that gives something or the end: a read that was interrupted is
/// read again.
fn read_some(from: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        match from.read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            read => return read,
        }
    }
}

/// What a read of the stream `name` that failed says: in the words standard
/// input has always been refused in when it could not be read, and with the
/// name of any other stream.
#[must_use]
pub fn read_error(name: &str, e: io::Error) -> anyhow::Error {
    let what = if name == crate::source::STDIN_NAME {
        "reading standard input".to_string()
    } else {
        format!("reading {name}")
    };
    anyhow::Error::new(io::Error::new(e.kind(), format!("{what}: {e}")))
}

/// A stage that hands every access unit of a TrueHD stream to a function: a
/// walk written as the function a walk over a [`Source`](crate::source::Source)
/// calls, lent to a pass instead. It is told nothing of an AC-3 stream.
pub(crate) struct OnUnit<'f>(pub(crate) UnitFn<'f>);

/// What a walk over the access units of a stream hands every unit to.
pub(crate) type UnitFn<'f> = &'f mut dyn FnMut(&Unit) -> Result<()>;

/// What a walk over the syncframes of a stream hands every syncframe to.
pub(crate) type FrameFn<'f> = &'f mut dyn FnMut(u64, &[u8], &FrameHeader) -> Result<()>;

impl Stage for OnUnit<'_> {
    fn unit(&mut self, unit: &UnitRef<'_>) -> Result<()> {
        match unit.truehd() {
            Some(unit) => (self.0)(unit),
            None => Ok(()),
        }
    }

    fn finish(&mut self, _: &PassEnd) -> Result<()> {
        Ok(())
    }
}

/// A stage that hands every syncframe of an AC-3 family stream to a
/// function, as [`OnUnit`] does an access unit.
pub(crate) struct OnFrame<'f>(pub(crate) FrameFn<'f>);

impl Stage for OnFrame<'_> {
    fn unit(&mut self, unit: &UnitRef<'_>) -> Result<()> {
        match unit.ac3() {
            Some(header) => (self.0)(unit.offset, unit.bytes, header),
            None => Ok(()),
        }
    }

    fn finish(&mut self, _: &PassEnd) -> Result<()> {
        Ok(())
    }
}

/// The totals of a walk over an AC-3 family stream, `(frames, sync_errors,
/// skipped)`, as `eac3::for_each_frame` returns them; zeros for a TrueHD
/// pass.
pub(crate) fn ac3_totals(end: &PassEnd) -> (u64, u64, u64) {
    match end.framed {
        Framed::Ac3 {
            frames,
            sync_errors,
            skipped,
        } => (frames, sync_errors, skipped),
        Framed::TrueHd(_) => (0, 0, 0),
    }
}

/// What a walk over a TrueHD stream returns; the default for an AC-3 pass.
pub(crate) fn truehd_summary(end: &PassEnd) -> PassSummary {
    match end.framed {
        Framed::TrueHd(summary) => summary,
        Framed::Ac3 { .. } => PassSummary {
            stats: oadec_truehd::ExtractStats::default(),
            trailing_bytes: 0,
            final_gap: None,
            file_bytes: end.bytes,
        },
    }
}
