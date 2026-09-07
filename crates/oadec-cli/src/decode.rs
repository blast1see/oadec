//! `oadec decode`: PCM output of one presentation, and the decode session that
//! `compare` shares.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use oadec_truehd::{AccessUnit, ChannelLabel, DecodeStats, Decoder, Unit};

use crate::input;

/// Output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Headerless interleaved 24-bit little-endian PCM.
    Pcm,
    /// 24-bit WAVE (format extensible), up to 4 GiB.
    Wav,
    /// Dolby Atmos Master Format set (`.atmos`, `.atmos.metadata`, `.atmos.audio`),
    /// object presentation only; the output path is the base name.
    Damf,
}

/// Channel order of the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Order {
    /// WAVE / FFmpeg order (front pair, centre, LFE, back pair, side pair, ...).
    Interchange,
    /// The order of the stream itself (front pair, centre, LFE, side pair, ...).
    Stream,
}

/// One decoded access unit, ready to be written.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    /// Output rows in stream channel order.
    pub pcm: &'a [[i32; 16]],
    /// Output channels.
    pub channels: usize,
    /// `order[k]` is the stream channel written at output position `k`.
    pub order: &'a [usize],
    /// Sampling frequency in Hz.
    pub sampling_frequency: u32,
    /// WAVE channel mask of the labelled output channels.
    pub channel_mask: u32,
}

/// A decoder plus the bookkeeping the commands share.
pub struct Session {
    presentation: usize,
    keep_duplicates: bool,
    order_kind: Order,
    decoder: Option<Decoder>,
    order: Vec<usize>,
    /// Labels of the output channels in stream order (bed channels only for the
    /// object presentation).
    pub labels: Vec<ChannelLabel>,
    /// Sampling frequency in Hz.
    pub sampling_frequency: u32,
    /// Access units dropped as duplicates.
    pub duplicates_dropped: u64,
}

impl Session {
    /// Creates a session for `presentation`.
    #[must_use]
    pub fn new(presentation: usize, keep_duplicates: bool, order_kind: Order) -> Self {
        Self {
            presentation,
            keep_duplicates,
            order_kind,
            decoder: None,
            order: Vec::new(),
            labels: Vec::new(),
            sampling_frequency: 0,
            duplicates_dropped: 0,
        }
    }

    /// Decodes one access unit; `None` for a dropped duplicate.
    pub fn decode(&mut self, unit: &Unit) -> Result<Option<Frame<'_>>> {
        if self.decoder.is_none() {
            let (au, _) = AccessUnit::parse(&unit.bytes, None)?;
            let Some(ms) = &au.major_sync else {
                bail!("the stream does not start with a major sync");
            };
            let mut decoder = Decoder::new(ms, self.presentation)?;
            decoder.keep_duplicates(self.keep_duplicates);
            self.labels = ChannelLabel::presentation(ms, decoder.source_presentation());
            self.sampling_frequency = decoder.config().sampling_frequency;
            self.decoder = Some(decoder);
        }
        let mask = self.channel_mask();
        let sampling_frequency = self.sampling_frequency;
        let decoder = self.decoder.as_mut().expect("decoder created above");
        let decoded = decoder
            .decode(&unit.bytes)
            .with_context(|| format!("decoding access unit at byte {}", unit.offset))?;
        if decoded.duplicate {
            self.duplicates_dropped += 1;
            return Ok(None);
        }
        if self.order.len() != decoded.channels {
            self.order = match self.order_kind {
                Order::Interchange => {
                    ChannelLabel::interchange_order(&self.labels, decoded.channels)
                }
                Order::Stream => (0..decoded.channels).collect(),
            };
        }
        Ok(Some(Frame {
            pcm: decoded.pcm,
            channels: decoded.channels,
            order: &self.order,
            sampling_frequency,
            channel_mask: mask,
        }))
    }

    /// Decoder counters.
    #[must_use]
    pub fn stats(&self) -> Option<&DecodeStats> {
        self.decoder.as_ref().map(Decoder::stats)
    }

    /// Output channel labels in output order (objects unlabelled).
    #[must_use]
    pub fn output_labels(&self) -> Vec<String> {
        self.order
            .iter()
            .map(|&i| {
                self.labels.get(i).map_or_else(
                    || format!("obj{}", i + 1 - self.labels.len()),
                    ToString::to_string,
                )
            })
            .collect()
    }

    /// WAVE channel mask of the output (labelled channels only).
    #[must_use]
    pub fn channel_mask(&self) -> u32 {
        self.labels
            .iter()
            .map(|l| l.interchange_index())
            .filter(|&i| i < 32)
            .fold(0, |m, i| m | (1u32 << i))
    }
}

/// Formats a duration in seconds as `h:mm:ss.mmm`.
pub fn format_duration(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    format!(
        "{}:{:02}:{:02}.{:03}",
        total_ms / 3_600_000,
        (total_ms / 60_000) % 60,
        (total_ms / 1000) % 60,
        total_ms % 1000
    )
}

/// Prints the decoder summary to stderr.
pub fn print_summary(session: &Session, elapsed: f64) {
    let Some(stats) = session.stats() else {
        eprintln!("nothing decoded");
        return;
    };
    let seconds = stats.samples as f64 / f64::from(session.sampling_frequency.max(1));
    eprintln!(
        "decoded {} access units, {} samples x {} channels ({}), {} dropped duplicates",
        stats.units,
        stats.samples,
        session.order.len(),
        format_duration(seconds),
        session.duplicates_dropped
    );
    eprintln!(
        "lossless checks: {} performed, {} failed; max_bits violations: {}; segment problems: {}",
        stats.lossless_checks,
        stats.lossless_mismatches,
        stats.max_bits_violations,
        stats.segment_problems
    );
    if let Some(p) = &stats.first_problem {
        eprintln!("first problem: {p}");
    }
    eprintln!("channels: {}", session.output_labels().join(" "));
    eprintln!(
        "speed: {elapsed:.2} s ({:.0}x realtime)",
        seconds / elapsed.max(1e-9)
    );
}

/// Options of the decode command.
#[derive(Debug, Clone)]
pub struct Options {
    pub presentation: usize,
    pub format: Format,
    pub order: Order,
    pub keep_duplicates: bool,
}

const WAV_HEADER_LEN: u64 = 12 + 8 + 40 + 8;
const MAX_WAV_DATA: u64 = u32::MAX as u64 - WAV_HEADER_LEN;

fn write_wav_header(
    out: &mut impl Write,
    channels: u16,
    rate: u32,
    mask: u32,
    data_len: u32,
) -> std::io::Result<()> {
    let block_align = channels * 3;
    out.write_all(b"RIFF")?;
    out.write_all(&(data_len + (WAV_HEADER_LEN - 8) as u32).to_le_bytes())?;
    out.write_all(b"WAVE")?;
    out.write_all(b"fmt ")?;
    out.write_all(&40u32.to_le_bytes())?;
    out.write_all(&0xFFFEu16.to_le_bytes())?; // WAVE_FORMAT_EXTENSIBLE
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * u32::from(block_align)).to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&24u16.to_le_bytes())?;
    out.write_all(&22u16.to_le_bytes())?; // cbSize
    out.write_all(&24u16.to_le_bytes())?; // valid bits
    out.write_all(&mask.to_le_bytes())?;
    // KSDATAFORMAT_SUBTYPE_PCM
    out.write_all(&[
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ])?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

/// Runs the command.
pub fn run(path: &Path, output: &Path, opts: &Options) -> Result<()> {
    let started = Instant::now();
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut out = BufWriter::with_capacity(4 << 20, file);
    let mut session = Session::new(opts.presentation, opts.keep_duplicates, opts.order);
    let mut header_written = false;
    let mut data_len: u64 = 0;
    let mut channels = 0usize;
    let mut rate = 0u32;
    let mut mask = 0u32;
    let mut buf = Vec::with_capacity(160 * 16 * 3);
    input::for_each_unit(path, |unit| {
        let Some(frame) = session.decode(&unit)? else {
            return Ok(());
        };
        if !header_written {
            channels = frame.channels;
            rate = frame.sampling_frequency;
            mask = frame.channel_mask;
            if opts.format == Format::Wav {
                write_wav_header(&mut out, channels as u16, rate, mask, 0)?;
            }
            header_written = true;
        }
        buf.clear();
        for row in frame.pcm {
            for &ch in frame.order {
                let b = row[ch].to_le_bytes();
                buf.extend_from_slice(&b[..3]);
            }
        }
        data_len += buf.len() as u64;
        if opts.format == Format::Wav && data_len > MAX_WAV_DATA {
            bail!("output exceeds the 4 GiB WAVE limit; use --format pcm");
        }
        out.write_all(&buf)?;
        Ok(())
    })?;
    if opts.format == Format::Wav && header_written {
        out.flush()?;
        let mut file = out.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        write_wav_header(
            &mut file,
            channels as u16,
            rate,
            session.channel_mask(),
            data_len as u32,
        )?;
        file.flush()?;
    } else {
        out.flush()?;
    }
    print_summary(&session, started.elapsed().as_secs_f64());
    if session.stats().is_some_and(|s| s.lossless_mismatches != 0) {
        bail!("lossless check failures were reported");
    }
    Ok(())
}
