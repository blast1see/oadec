//! `oadec decode`: PCM output of one presentation, and the decode session that
//! `compare` shares.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use oadec_truehd::{AccessUnit, ChannelLabel, DecodeStats, Decoder, Unit};

use crate::input;
use crate::integrity::Findings;

/// Output container.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Headerless interleaved 24-bit little-endian PCM.
    Pcm,
    /// 24-bit WAVE (format extensible); RF64 once it passes 4 GiB.
    Wav,
    /// Dolby Atmos Master Format set (`.atmos`, `.atmos.metadata`, `.atmos.audio`),
    /// object presentation only; the output path is the base name.
    Damf,
    /// ADM BWF (Broadcast Wave with axml/chna/dbmd chunks), object presentation
    /// only; the output path is the base name (`.wav` is appended).
    Adm,
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
    /// WAVE channel mask of the output: zero unless it names every channel in
    /// the order written (see [`wave_channel_mask`]).
    pub channel_mask: u32,
    /// Why the mask is zero, when it is; printed by the paths that write one.
    pub mask_warning: Option<&'a str>,
}

/// A decoder plus the bookkeeping the commands share.
pub struct Session {
    presentation: usize,
    keep_duplicates: bool,
    order_kind: Order,
    decoder: Option<Decoder>,
    order: Vec<usize>,
    /// WAVE channel mask of the output order.
    mask: u32,
    /// Why the mask is zero, when it is.
    mask_warning: Option<String>,
    /// Labels of the output channels in stream order (bed channels only for the
    /// object presentation).
    pub labels: Vec<ChannelLabel>,
    /// Sampling frequency in Hz.
    pub sampling_frequency: u32,
    /// Access units dropped as duplicates.
    pub duplicates_dropped: u64,
    /// Samples a 24-bit writer had to saturate (counted by the PCM and WAVE
    /// path, which is the one that writes the decoder's samples as they are).
    pub clipped: u64,
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
            mask: 0,
            mask_warning: None,
            labels: Vec::new(),
            sampling_frequency: 0,
            duplicates_dropped: 0,
            clipped: 0,
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
            (self.mask, self.mask_warning) = wave_channel_mask(&self.labels, &self.order);
        }
        Ok(Some(Frame {
            pcm: decoded.pcm,
            channels: decoded.channels,
            order: &self.order,
            sampling_frequency,
            channel_mask: self.mask,
            mask_warning: self.mask_warning.as_deref(),
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
            .map(|&i| channel_name(&self.labels, i))
            .collect()
    }
}

/// The name of stream channel `index` in a summary: its label, or `objN` for
/// the unlabelled channels after the labelled ones (the objects).
fn channel_name(labels: &[ChannelLabel], index: usize) -> String {
    labels.get(index).map_or_else(
        || format!("obj{}", index + 1 - labels.len()),
        ToString::to_string,
    )
}

/// The WAVE channel mask of an output whose position `k` carries stream
/// channel `order[k]`, and the warning to print when the mask is zero.
///
/// A `WAVEFORMATEXTENSIBLE` mask assigns its set bits, lowest first, to the
/// channels in the order they are written, so a mask is only true when every
/// channel has a speaker bit and the channels come in bit order. Anything
/// else is written as zero, the format's way of saying the assignment is not
/// stated, which is the rule the E-AC-3 writer follows. The mask used to set
/// the interchange index of every label below 32: presentation 3 wrote its
/// objects under the bed's bits, wide left set bit 31 (`SPEAKER_ALL`), the
/// labels past it fell off, and `--order stream` wrote the 7.1 mask over a
/// side pair where the mask says back.
pub fn wave_channel_mask(labels: &[ChannelLabel], order: &[usize]) -> (u32, Option<String>) {
    let unnamed: Vec<String> = order
        .iter()
        .filter(|&&i| labels.get(i).and_then(|l| l.wave_bit()).is_none())
        .map(|&i| channel_name(labels, i))
        .collect();
    if !unnamed.is_empty() {
        return (
            0,
            Some(format!(
                "WAVE has no channel mask bit for {}; writing an unassigned mask",
                unnamed.join(", ")
            )),
        );
    }
    let bits: Vec<u8> = order.iter().filter_map(|&i| labels[i].wave_bit()).collect();
    if bits.windows(2).any(|w| w[0] >= w[1]) {
        return (
            0,
            Some(
                "the channels are not written in WAVE channel mask order (--order stream); writing an unassigned mask"
                    .to_string(),
            ),
        );
    }
    (bits.iter().fold(0, |m, &b| m | (1 << b)), None)
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

/// The faults a TrueHD pass found, in the form a delivery path reports.
///
/// `verify` reads all of these through `scan::Failures`; a decode used to read
/// one of them. The extractor summary is the part that used to be dropped
/// outright: `for_each_unit` returns it and every caller but `verify` threw it
/// away, so a decode that resynchronised past a corrupt major sync said nothing
/// at all.
pub fn truehd_findings(pass: &input::PassSummary, stats: Option<&DecodeStats>) -> Findings {
    let mut f = Findings::default();
    f.note(
        pass.stats.major_sync_crc_failures,
        "major sync CRC failures",
    );
    f.note(pass.stats.resyncs, "resynchronisations");
    f.note(pass.stats.skipped_bytes, "bytes skipped");
    f.note(
        pass.trailing_bytes,
        "trailing bytes that formed no access unit",
    );
    if let Some(s) = stats {
        f.note(s.lossless_mismatches, "lossless check failures");
        f.note(s.segment_problems, "substream segment problems");
        f.note(s.max_bits_violations, "max_bits violations");
        f.note(s.invalid_branches, "invalid seamless branches");
        f.note(s.extra_header_parity, "extra-data header parity failures");
        f.note(s.extra_truncated, "truncated extra-data blocks");
        f.note(s.extra_evolution_parity, "Evolution parity failures");
        f.note(
            s.extra_padding_nonzero,
            "extra-data blocks with non-zero padding",
        );
        f.note(s.evolution_container_errors, "Evolution container errors");
        f.first_problem(s.first_problem.as_deref());
    }
    f
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
    if session.clipped > 0 {
        eprintln!("{} samples clipped to 24 bits", session.clipped);
    }
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

/// Bytes of the WAVE header `decode` writes: `RIFF`/`WAVE`, the `JUNK` chunk
/// that becomes `ds64` past 4 GiB, a 40-byte extensible `fmt `, and the
/// `data` chunk header.
pub const WAV_HEADER_LEN: u64 = 12 + 8 + oadec_spatial::DS64_LEN as u64 + 8 + 40 + 8;

/// Byte offset of the 32-bit size of the `data` chunk.
pub const WAV_DATA_SIZE_POS: u64 = WAV_HEADER_LEN - 4;

/// Sample format of a WAVE output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavSample {
    /// 24-bit integer PCM (TrueHD).
    Int24,
    /// 32-bit IEEE float (E-AC-3).
    Float32,
}

/// What the `fmt ` chunk of a WAVE output says.
#[derive(Debug, Clone, Copy)]
pub struct WavSpec {
    pub channels: u16,
    pub rate: u32,
    /// `dwChannelMask`.
    pub mask: u32,
    pub sample: WavSample,
}

impl WavSpec {
    fn block_align(self) -> u16 {
        self.channels
            * match self.sample {
                WavSample::Int24 => 3,
                WavSample::Float32 => 4,
            }
    }
}

/// A WAVE file being written.
///
/// The header goes out first with its sizes unknown, the samples after it,
/// and the sizes are written when the file is closed: by `finish`, or by
/// `Drop` when the writer is dropped first because the decode stopped or
/// failed, so the file on disk always declares the samples it holds. The
/// header used to be patched on success only, which left a failed decode's
/// file declaring no data at all. A file whose sizes outgrow 32 bits becomes
/// RF64 when it is closed ([`oadec_spatial::patch_riff_sizes`]); there used to
/// be a 4 GiB limit instead, found after the bytes past it had been written.
#[derive(Debug)]
pub struct WavOut {
    out: BufWriter<File>,
    spec: WavSpec,
    data_len: u64,
    closed: bool,
}

impl WavOut {
    /// Writes the header to `out`.
    pub fn create(mut out: BufWriter<File>, spec: WavSpec) -> std::io::Result<Self> {
        let bits = spec.block_align() / spec.channels.max(1) * 8;
        out.write_all(b"RIFF")?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(b"WAVE")?;
        out.write_all(b"JUNK")?;
        out.write_all(&oadec_spatial::DS64_LEN.to_le_bytes())?;
        out.write_all(&[0; oadec_spatial::DS64_LEN as usize])?;
        out.write_all(b"fmt ")?;
        out.write_all(&40u32.to_le_bytes())?;
        out.write_all(&0xFFFEu16.to_le_bytes())?; // WAVE_FORMAT_EXTENSIBLE
        out.write_all(&spec.channels.to_le_bytes())?;
        out.write_all(&spec.rate.to_le_bytes())?;
        out.write_all(&(spec.rate * u32::from(spec.block_align())).to_le_bytes())?;
        out.write_all(&spec.block_align().to_le_bytes())?;
        out.write_all(&bits.to_le_bytes())?;
        out.write_all(&22u16.to_le_bytes())?; // cbSize
        out.write_all(&bits.to_le_bytes())?; // valid bits
        out.write_all(&spec.mask.to_le_bytes())?;
        // KSDATAFORMAT_SUBTYPE_PCM (1) or KSDATAFORMAT_SUBTYPE_IEEE_FLOAT (3)
        let format = match spec.sample {
            WavSample::Int24 => 0x01,
            WavSample::Float32 => 0x03,
        };
        out.write_all(&[
            format, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38,
            0x9B, 0x71,
        ])?;
        out.write_all(b"data")?;
        out.write_all(&0u32.to_le_bytes())?;
        Ok(Self {
            out,
            spec,
            data_len: 0,
            closed: false,
        })
    }

    /// Appends sample bytes.
    pub fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.out.write_all(bytes)?;
        self.data_len += bytes.len() as u64;
        Ok(())
    }

    /// Writes the sizes and closes the file; returns whether it became RF64.
    pub fn finish(mut self) -> std::io::Result<bool> {
        self.close()
    }

    fn close(&mut self) -> std::io::Result<bool> {
        self.closed = true;
        let pad = self.data_len % 2;
        if pad == 1 {
            self.out.write_all(&[0])?; // the RIFF pad byte, outside the data size
        }
        self.out.flush()?;
        let frames = self.data_len / u64::from(self.spec.block_align().max(1));
        let total = WAV_HEADER_LEN + self.data_len + pad;
        let file = self.out.get_mut();
        let long_form =
            oadec_spatial::patch_riff_sizes(file, total, WAV_DATA_SIZE_POS, self.data_len, frames)?;
        file.flush()?;
        Ok(long_form)
    }
}

impl Drop for WavOut {
    fn drop(&mut self) {
        if !self.closed {
            // nothing to report an error to; the decode is already failing
            let _ = self.close();
        }
    }
}

/// Where a decode stopped: an access unit whose major sync changed what the
/// decoder was set up for (`oadec_truehd::Error::ConfigChanged`).
///
/// Everything before that unit decoded soundly, so a delivery that has
/// started its output ends it there, finishes the file and exits 7 --
/// `verify` counts the same change and exits 7 on it. The change used to stop
/// the decode with exit 2 before the header was written. A change before any
/// output is still an error: there is nothing to deliver.
#[derive(Debug, Clone, Copy)]
pub struct ConfigStop {
    /// Index of the refused access unit.
    pub unit: u64,
    /// Its byte offset in the file.
    pub offset: u64,
    /// What changed, as a phrase ("the sampling frequency").
    pub what: &'static str,
}

impl ConfigStop {
    /// The stop `err` describes, if it is a configuration change.
    pub fn from_error(err: &anyhow::Error, unit: u64, offset: u64) -> Option<Self> {
        match err.downcast_ref::<oadec_truehd::Error>() {
            Some(&oadec_truehd::Error::ConfigChanged { what }) => Some(Self { unit, offset, what }),
            _ => None,
        }
    }

    /// Says where the output ends.
    pub fn print(&self, samples: u64) {
        eprintln!(
            "stopped at access unit {} (byte {}): {} changed at a major sync; {samples} samples written",
            self.unit, self.offset, self.what
        );
    }

    /// Notes the change in a verdict.
    pub fn note(&self, f: &mut Findings) {
        f.note(
            1,
            &format!(
                "mid-stream configuration change ({}); the output ends there",
                self.what
            ),
        );
    }
}

/// Where `run` writes the samples.
enum Output {
    /// Headerless PCM, or a WAVE file whose header waits for the first frame.
    Raw(BufWriter<File>),
    /// A WAVE file with its header written.
    Wav(WavOut),
}

/// Appends `pcm` to `buf` as interleaved 24-bit little-endian samples in
/// output order (`order[k]` is the channel written at position `k`); returns
/// how many samples were outside the 24-bit range and were written saturated.
fn pack_24le(pcm: &[[i32; 16]], order: &[usize], buf: &mut Vec<u8>) -> u64 {
    let mut clipped = 0;
    for row in pcm {
        for &ch in order {
            let (v, saturated) = oadec_spatial::clamp_i24(row[ch]);
            clipped += u64::from(saturated);
            buf.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    clipped
}

/// Runs the command.
pub fn run(path: &Path, output: &Path, opts: &Options) -> Result<bool> {
    let started = Instant::now();
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut out = Some(Output::Raw(BufWriter::with_capacity(4 << 20, file)));
    let mut session = Session::new(opts.presentation, opts.keep_duplicates, opts.order);
    let mut output_started = false;
    let mut samples: u64 = 0;
    let mut stopped: Option<ConfigStop> = None;
    let mut buf = Vec::with_capacity(160 * 16 * 3);
    let pass = input::for_each_unit(path, |unit| {
        if stopped.is_some() {
            // the rest is still framed, so the verdict covers the whole file
            return Ok(());
        }
        let index = session.stats().map_or(0, |s| s.units);
        let frame = match session.decode(&unit) {
            Ok(Some(frame)) => frame,
            Ok(None) => return Ok(()),
            Err(e) => {
                return match ConfigStop::from_error(&e, index, unit.offset) {
                    Some(stop) if output_started => {
                        stopped = Some(stop);
                        Ok(())
                    }
                    _ => Err(e),
                };
            }
        };
        if !output_started {
            output_started = true;
            if opts.format == Format::Wav
                && let Some(Output::Raw(raw)) = out.take()
            {
                if let Some(warning) = frame.mask_warning {
                    eprintln!("warning: {warning}");
                }
                let spec = WavSpec {
                    channels: frame.channels as u16,
                    rate: frame.sampling_frequency,
                    mask: frame.channel_mask,
                    sample: WavSample::Int24,
                };
                out = Some(Output::Wav(WavOut::create(raw, spec)?));
            }
        }
        buf.clear();
        let clipped = pack_24le(frame.pcm, frame.order, &mut buf);
        samples += frame.pcm.len() as u64;
        session.clipped += clipped;
        match out.as_mut() {
            Some(Output::Wav(w)) => w.write(&buf)?,
            Some(Output::Raw(w)) => w.write_all(&buf)?,
            None => unreachable!("the output stays open until the pass ends"),
        }
        Ok(())
    })?;
    if session.stats().is_none() {
        // Nothing framed as TrueHD: there is no output to deliver and nothing
        // to judge, which is an unusable input (exit 2), not a clean run.
        drop(out);
        let _ = std::fs::remove_file(output);
        anyhow::bail!("no TrueHD access unit found in {}", path.display());
    }
    match out {
        Some(Output::Wav(w)) => {
            w.finish()?;
        }
        Some(Output::Raw(mut w)) => w.flush()?,
        None => {}
    }
    if let Some(stop) = &stopped {
        stop.print(samples);
    }
    print_summary(&session, started.elapsed().as_secs_f64());
    let mut f = truehd_findings(&pass, session.stats());
    f.note(session.clipped, "samples clipped to 24 bits");
    if let Some(stop) = &stopped {
        stop.note(&mut f);
    }
    Ok(f.report_clean())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_pass() -> input::PassSummary {
        input::PassSummary {
            stats: oadec_truehd::ExtractStats::default(),
            trailing_bytes: 0,
            file_bytes: 0,
        }
    }

    /// `verify` counts five extra-data faults; the decode verdict used to read
    /// none of them, so a corrupted Evolution frame left `decode` at 0 while
    /// `verify` said 7 about the same file.
    #[test]
    fn every_extra_data_fault_makes_a_decode_unclean() {
        let pass = empty_pass();
        assert!(truehd_findings(&pass, Some(&DecodeStats::default())).is_clean());
        type Set = fn(&mut DecodeStats);
        let faults: [(&str, Set); 5] = [
            ("extra_header_parity", |s| s.extra_header_parity = 1),
            ("extra_truncated", |s| s.extra_truncated = 1),
            ("extra_evolution_parity", |s| s.extra_evolution_parity = 1),
            ("extra_padding_nonzero", |s| s.extra_padding_nonzero = 1),
            ("evolution_container_errors", |s| {
                s.evolution_container_errors = 1;
            }),
        ];
        for (name, set) in faults {
            let mut stats = DecodeStats::default();
            set(&mut stats);
            assert!(
                !truehd_findings(&pass, Some(&stats)).is_clean(),
                "{name} left the decode looking clean"
            );
        }
    }

    use oadec_truehd::ChannelLabel as L;

    const SEVEN_ONE: [L; 8] = [L::L, L::R, L::C, L::LFE, L::Ls, L::Rs, L::Lb, L::Rb];

    /// A mask is written only when it names every output channel, in the
    /// order the samples are written; otherwise the mask is zero, which is the
    /// format's own way of saying the assignment is not stated.
    #[test]
    fn the_wave_mask_names_every_channel_in_bit_order_or_is_zero() {
        // 7.1 in interchange order is L R C LFE Lb Rb Ls Rs: bits 0-5, 9, 10
        let order = L::interchange_order(&SEVEN_ONE, 8);
        assert_eq!(wave_channel_mask(&SEVEN_ONE, &order), (0x63F, None));

        // the same mask over stream order would call Ls Rs the back pair
        let stream: Vec<usize> = (0..8).collect();
        let (mask, warning) = wave_channel_mask(&SEVEN_ONE, &stream);
        assert_eq!(mask, 0);
        assert!(warning.is_some_and(|w| w.contains("order")));

        // 7.1.2 with top side speakers, which WAVE cannot name
        let mut seven_one_two = SEVEN_ONE.to_vec();
        seven_one_two.extend([L::Tsl, L::Tsr]);
        let order = L::interchange_order(&seven_one_two, 10);
        let (mask, warning) = wave_channel_mask(&seven_one_two, &order);
        assert_eq!(mask, 0);
        let warning = warning.unwrap();
        assert!(
            warning.contains("Tsl, Tsr") && warning.contains("unassigned mask"),
            "{warning}"
        );

        // fewer labels than channels: the objects after a one-channel bed
        let order = L::interchange_order(&[L::LFE], 12);
        let (mask, warning) = wave_channel_mask(&[L::LFE], &order);
        assert_eq!(mask, 0);
        assert!(warning.is_some_and(|w| w.contains("obj1, obj2")));

        // and the wide pair, whose old index set bit 31
        let wide = [L::L, L::R, L::Lw, L::Rw];
        let order = L::interchange_order(&wide, 4);
        assert_eq!(wave_channel_mask(&wide, &order).0, 0);
    }

    /// A sample past the 24-bit range saturates, as the CAF and ADM writers
    /// saturate it, instead of losing its top byte -- 1 << 24 used to be
    /// written as zero -- and is counted.
    #[test]
    fn the_24_bit_packer_saturates_and_counts() {
        let mut rows = [[0i32; 16]; 3];
        rows[0][1] = 1 << 24;
        rows[1][1] = -(1 << 24);
        rows[2][0] = -5;
        let mut buf = Vec::new();
        let clipped = pack_24le(&rows, &[1, 0], &mut buf);
        assert_eq!(
            buf,
            [
                0xFF, 0xFF, 0x7F, 0x00, 0x00, 0x00, // 1 << 24, then channel 0
                0x00, 0x00, 0x80, 0x00, 0x00, 0x00, // -(1 << 24)
                0x00, 0x00, 0x00, 0xFB, 0xFF, 0xFF, // -5 is inside the range
            ]
        );
        assert_eq!(clipped, 2);
    }

    /// A WAVE output dropped before `finish` -- an error, or a decode that
    /// stopped part way -- still declares the samples it holds. The header
    /// used to be patched on success only, so a decode that failed left
    /// `data` at zero bytes over everything written before the failure.
    #[test]
    fn a_dropped_wav_output_declares_the_bytes_it_wrote() {
        let dir = std::env::temp_dir().join(format!("oadec-wavout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let u32_at = |b: &[u8], at: u64| {
            u64::from(u32::from_le_bytes(
                b[at as usize..at as usize + 4].try_into().unwrap(),
            ))
        };
        let spec = |channels| WavSpec {
            channels,
            rate: 48_000,
            mask: 0,
            sample: WavSample::Int24,
        };

        let path = dir.join("dropped.wav");
        let file = BufWriter::new(File::create(&path).unwrap());
        let mut out = WavOut::create(file, spec(2)).unwrap();
        out.write(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]).unwrap();
        drop(out);
        let b = std::fs::read(&path).unwrap();
        assert_eq!(b.len() as u64, WAV_HEADER_LEN + 12);
        assert_eq!(b[..4], *b"RIFF");
        assert_eq!(u32_at(&b, 4), b.len() as u64 - 8, "RIFF size");
        assert_eq!(b[12..16], *b"JUNK", "room for ds64");
        assert_eq!(
            b[WAV_DATA_SIZE_POS as usize - 4..WAV_DATA_SIZE_POS as usize],
            *b"data"
        );
        assert_eq!(u32_at(&b, WAV_DATA_SIZE_POS), 12, "data size");

        // finished on purpose, with an odd data chunk and so a pad byte
        let path = dir.join("odd.wav");
        let file = BufWriter::new(File::create(&path).unwrap());
        let mut out = WavOut::create(file, spec(1)).unwrap();
        out.write(&[1, 2, 3]).unwrap();
        assert!(!out.finish().unwrap(), "a short file keeps the RIFF form");
        let b = std::fs::read(&path).unwrap();
        assert_eq!(b.len() as u64, WAV_HEADER_LEN + 3 + 1);
        assert_eq!(u32_at(&b, 4), b.len() as u64 - 8, "RIFF size");
        assert_eq!(u32_at(&b, WAV_DATA_SIZE_POS), 3, "data size");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
