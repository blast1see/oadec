//! `oadec compare`: decode a presentation and compare it sample by sample with
//! a reference PCM file produced by another decoder.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::ValueEnum;

use crate::decode::{Order, Session, format_duration, print_summary};
use crate::input;

/// Sample format of the reference file (interleaved).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RefFormat {
    /// 32-bit signed little-endian; compared after discarding the low eight bits.
    S32le,
    /// 24-bit signed packed little-endian.
    S24le,
    /// 24-bit signed packed big-endian (CAF, AIFF).
    S24be,
    /// 16-bit signed little-endian; compared after shifting up by eight bits.
    S16le,
}

impl RefFormat {
    const fn bytes(self) -> usize {
        match self {
            Self::S32le => 4,
            Self::S24le | Self::S24be => 3,
            Self::S16le => 2,
        }
    }

    fn sample(self, b: &[u8]) -> i32 {
        match self {
            Self::S32le => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) >> 8,
            Self::S24le => i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8,
            Self::S24be => i32::from_be_bytes([b[0], b[1], b[2], 0]) >> 8,
            Self::S16le => i32::from(i16::from_le_bytes([b[0], b[1]])) << 8,
        }
    }
}

/// Options of the compare command.
#[derive(Debug, Clone)]
pub struct Options {
    pub presentation: usize,
    pub format: RefFormat,
    pub order: Order,
    pub keep_duplicates: bool,
    pub report: usize,
    /// Bytes to skip at the start of the reference (a container header).
    pub skip: u64,
}

/// Reads whole samples from the reference file (or standard input for `-`).
struct RefReader {
    reader: Box<dyn Read>,
    format: RefFormat,
    buf: Vec<u8>,
    filled: usize,
    pos: usize,
    eof: bool,
}

impl RefReader {
    fn open(path: &Path, format: RefFormat, skip: u64) -> Result<Self> {
        let reader: Box<dyn Read> = if path.as_os_str() == "-" {
            Box::new(std::io::stdin().lock())
        } else {
            let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
            Box::new(BufReader::with_capacity(4 << 20, file))
        };
        let mut this = Self {
            reader,
            format,
            buf: vec![0; 1 << 20],
            filled: 0,
            pos: 0,
            eof: false,
        };
        let mut to_skip = skip;
        while to_skip > 0 {
            let n = to_skip.min(this.buf.len() as u64) as usize;
            let got = this.reader.read(&mut this.buf[..n])?;
            if got == 0 {
                anyhow::bail!("reference shorter than the {skip} bytes to skip");
            }
            to_skip -= got as u64;
        }
        Ok(this)
    }

    /// Next sample, or `None` at the end of the file.
    fn next(&mut self) -> Result<Option<i32>> {
        let n = self.format.bytes();
        if self.filled - self.pos < n {
            let rest = self.filled - self.pos;
            self.buf.copy_within(self.pos..self.filled, 0);
            self.filled = rest;
            self.pos = 0;
            while self.filled < n && !self.eof {
                let got = self.reader.read(&mut self.buf[self.filled..])?;
                if got == 0 {
                    self.eof = true;
                } else {
                    self.filled += got;
                }
            }
            if self.filled < n {
                return Ok(None);
            }
        }
        let v = self.format.sample(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(Some(v))
    }

    /// Bytes left unread (called after the decode finished).
    fn remaining(&mut self) -> Result<u64> {
        let mut rest = (self.filled - self.pos) as u64;
        loop {
            let got = self.reader.read(&mut self.buf)?;
            if got == 0 {
                break;
            }
            rest += got as u64;
        }
        Ok(rest)
    }
}

/// Runs the command; returns `true` when every sample matched and the lengths agree.
pub fn run(path: &Path, reference: &Path, opts: &Options) -> Result<bool> {
    let started = Instant::now();
    // the checks of `verify` read the stream in a pass of their own, beside the
    // decode: a presentation reads only its substreams, and only some outputs
    // parse the object metadata
    let check = crate::verify::spawn_stream_check(path);
    let mut reference = RefReader::open(reference, opts.format, opts.skip)?;
    let mut session = Session::new(opts.presentation, opts.keep_duplicates, opts.order);
    let mut compared: u64 = 0;
    let mut mismatches: u64 = 0;
    let mut max_diff: u32 = 0;
    let mut reported: Vec<String> = Vec::new();
    let mut reference_short = false;
    let mut sample_index: u64 = 0;
    let mut channels = 0usize;
    let pass = input::for_each_unit(path, |unit| {
        if reference_short {
            return Ok(());
        }
        let Some(frame) = session.decode(&unit)? else {
            return Ok(());
        };
        channels = frame.channels;
        for row in frame.pcm {
            for (k, &ch) in frame.order.iter().enumerate() {
                let Some(expected) = reference.next()? else {
                    reference_short = true;
                    return Ok(());
                };
                let ours = row[ch];
                compared += 1;
                if ours != expected {
                    mismatches += 1;
                    max_diff = max_diff.max(ours.abs_diff(expected));
                    if reported.len() < opts.report {
                        reported.push(format!(
                            "sample {sample_index} channel {k}: ours {ours} reference {expected}"
                        ));
                    }
                }
            }
            sample_index += 1;
        }
        Ok(())
    })?;
    let leftover = reference.remaining()?;
    let leftover_samples = leftover / (opts.format.bytes() as u64 * channels.max(1) as u64);
    if session.stats().is_none() {
        // Nothing framed: the file is not a stream to judge but an unsupported
        // input, exit 2, as it is for every command that reports one.
        return Err(crate::info::no_stream(path));
    }
    let elapsed = started.elapsed().as_secs_f64();
    print_summary(&session, elapsed);
    let rate = f64::from(session.sampling_frequency.max(1));
    println!(
        "compared {} samples x {} channels ({}) with the reference",
        sample_index,
        channels,
        format_duration(sample_index as f64 / rate)
    );
    println!("mismatching samples: {mismatches} (max difference {max_diff})");
    for line in &reported {
        println!("  {line}");
    }
    if reference_short {
        println!("reference ended before the decoded stream (at sample {sample_index})");
    } else if leftover_samples > 0 {
        println!(
            "reference continues for {leftover_samples} more samples after the decoded stream"
        );
    } else {
        println!("lengths agree");
    }
    let equal = mismatches == 0 && !reference_short && leftover_samples == 0 && compared > 0;
    println!("result: {}", if equal { "BIT-EXACT" } else { "DIFFERENT" });
    let stream = crate::verify::join_stream_check(check)?;
    let clean = crate::decode::truehd_findings(&pass, session.stats(), &stream).report_clean();
    Ok(equal && clean)
}
