//! `oadec decode stream.ec3 --format damf|adm`: the JOC objects of an E-AC-3
//! stream together with their Object Audio Metadata, as a DAMF set or an ADM
//! BWF file.
//!
//! The core decoder yields the downmix channels frame by frame; the QMF bank
//! splits them into 64 subbands, the JOC decoder applies the transmitted
//! reconstruction matrices, and the synthesis bank turns every object back
//! into samples. The LFE is not part of JOC and passes straight through
//! (ETSI TS 103 420 table 47, note).

use std::collections::VecDeque;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_eac3::{ChannelLoc, Decoded, ProgramDecoder, ProgramFrame};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::joc::{Joc, SparseReading};
use oadec_emdf::oamd::{BedChannel, Oamd};
use oadec_joc::{
    Analysis, BANDS, Carry, Complex, DELAY, JocDecoder, LOW_DELAY, MATRIX_ALIGN, Quadrature,
    SteepReading, Synthesis,
};
use oadec_spatial::{Program, Timeline};

use crate::damf::{Options, Sink};
use crate::decode::format_duration;
use crate::eac3::for_each_frame;
use crate::integrity::{Findings, Verdict};

/// Samples the core decoder emits before the first frame's audio proper (the
/// first block's half window). The Dolby decoder drops them; the metadata
/// timing counts from the first frame's first sample as it does.
const DECODER_DELAY: usize = 256;

fn to_i24(v: f64) -> i32 {
    (v * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// Where each output element comes from.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// A coded core channel (the LFE).
    Core(usize),
    /// A JOC object.
    Object(usize),
}

struct Pipeline {
    joc: JocDecoder,
    analysis: Vec<Analysis>,
    synthesis: Vec<Synthesis>,
    /// Coded channel index of every JOC input channel.
    joc_inputs: Vec<usize>,
    sources: Vec<Source>,
    /// Per element: samples waiting to be written, already aligned.
    queues: Vec<VecDeque<f64>>,
    /// Per element: samples still to drop at the start for alignment.
    to_drop: Vec<usize>,
    slots_in: Vec<[Complex; BANDS]>,
    slots_out: Vec<[Complex; BANDS]>,
    frames: u64,
    /// Frames whose sequence counter said the stream had been spliced, so the
    /// matrix history before them was forgotten (clause 6.3.3.3).
    splices: u64,
    /// Frames whose sequence counter did not follow the previous one and did
    /// not say so with a zero. Reported, not acted on: the counter wraps at
    /// 1 023 and a stream that simply miscounts is not a splice.
    seq_gaps: u64,
    last_seq: Option<u16>,
    /// One per JOC input channel: holds the subband samples back so that
    /// they meet the matrix they were coded with, and takes the 90-degree
    /// phase shift back out of the channels that carry one.
    quad: Vec<Quadrature>,
    /// Subband samples once they are through it.
    slots_rot: Vec<[Complex; BANDS]>,
    /// Samples of decoder delay that costs.
    low_delay: usize,
    /// `joc_clipgain` of the payload in flight, or 1 while it is switched off.
    clip_gain: f64,
    /// Whether to restore the level the encoder took off (`--no-clip-gain`
    /// turns it off, for measuring the difference).
    apply_clip_gain: bool,
}

/// What each JOC downmix channel carries. Configurations 3 and 4 of table 47
/// hand the surround pair over with a 90-degree phase shift; rotating Ls and
/// Rs back by -j puts the objects in phase with the source (measured on the
/// encoder round trip, see `docs/evidence`). Every channel gets a
/// [`Quadrature`] whether or not it carries a shift, because they all need
/// the same hold for the matrix alignment. `OADEC_JOC_PHASE=3,4:-` overrides
/// which channels are shifted and `OADEC_JOC_LOW=untouched` selects the third
/// reading of the shift, both for experiments.
fn carries(dmx_config: u8, channels: usize, flat: bool) -> Vec<Carry> {
    let low = std::env::var("OADEC_JOC_LOW").unwrap_or_default();
    let shift = |plus| match (flat, low.as_str()) {
        (_, "untouched") => Carry::FlatAbove { plus },
        (true, _) => Carry::Flat { plus },
        (false, _) => Carry::Shifted { plus },
    };
    let mut spec: Vec<(usize, bool)> = match std::env::var("OADEC_JOC_PHASE") {
        Err(_) => {
            if matches!(dmx_config, 3 | 4) {
                vec![(3, false), (4, false)]
            } else {
                Vec::new()
            }
        }
        Ok(v) if v.is_empty() || v == "none" => Vec::new(),
        Ok(v) => match v.split_once(':') {
            None => Vec::new(),
            Some((chans, sign)) => {
                let plus = sign.trim() != "-";
                chans
                    .split(',')
                    .filter_map(|c| c.trim().parse::<usize>().ok())
                    .map(|c| (c, plus))
                    .collect()
            }
        },
    };
    spec.retain(|&(ch, _)| ch < channels);
    let mut out = vec![Carry::Plain; channels];
    for (ch, plus) in spec {
        out[ch] = shift(plus);
    }
    out
}

impl Pipeline {
    fn new(
        joc: &Joc,
        program: &Program,
        chans: &[ChannelLoc],
        clip_gain: bool,
        flat_quadrature: bool,
        steep: SteepReading,
    ) -> Result<Self> {
        let mut joc_inputs = vec![usize::MAX; joc.num_channels];
        for (coded, loc) in chans.iter().enumerate() {
            if let Some(i) = loc.joc_input(joc.dmx_config)
                && i < joc.num_channels
            {
                joc_inputs[i] = coded;
            }
        }
        if joc_inputs.contains(&usize::MAX) {
            bail!(
                "the programme channels {:?} do not cover the {}-channel JOC downmix",
                chans.iter().map(|c| c.name()).collect::<Vec<_>>(),
                joc.num_channels
            );
        }
        let lfe = chans.iter().position(|c| *c == ChannelLoc::Lfe);
        // element order: beds, then ISF, then dynamic objects (as the program lists them)
        let mut sources = Vec::new();
        let mut next_object = 0usize;
        for ch in program.bed_channels() {
            if ch == BedChannel::LFE || ch == BedChannel::LFE2 {
                let Some(l) = lfe else {
                    bail!("the program has an LFE bed but the core stream has no LFE channel");
                };
                sources.push(Source::Core(l));
            } else {
                sources.push(Source::Object(next_object));
                next_object += 1;
            }
        }
        for _ in 0..program.isf_objects + program.dynamic_objects {
            sources.push(Source::Object(next_object));
            next_object += 1;
        }
        if next_object > joc.num_objects {
            bail!(
                "the program needs {} JOC objects but the stream carries {}",
                next_object,
                joc.num_objects
            );
        }
        if next_object < joc.num_objects {
            eprintln!(
                "warning: the stream carries {} JOC objects, the program uses {}",
                joc.num_objects, next_object
            );
        }
        let elements = sources.len();
        let carry = carries(joc.dmx_config, joc.num_channels, flat_quadrature);
        // every stream goes through the filter bank, whether or not it
        // carries a phase shift, because the matrix alignment needs the delay
        let low_delay = LOW_DELAY * BANDS;
        let to_drop = sources
            .iter()
            .map(|s| match s {
                Source::Core(_) => DECODER_DELAY,
                Source::Object(_) => DECODER_DELAY + DELAY + low_delay,
            })
            .collect();
        let mut joc_decoder = JocDecoder::new(joc.num_channels, joc.num_objects);
        let lag = std::env::var("OADEC_JOC_LAG")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(LOW_DELAY - MATRIX_ALIGN);
        joc_decoder.set_lag(lag);
        joc_decoder.set_steep_reading(steep);
        Ok(Self {
            joc: joc_decoder,
            analysis: (0..joc.num_channels).map(|_| Analysis::new()).collect(),
            synthesis: (0..joc.num_objects).map(|_| Synthesis::new()).collect(),
            joc_inputs,
            sources,
            queues: (0..elements).map(|_| VecDeque::new()).collect(),
            to_drop,
            slots_in: vec![[Complex::default(); BANDS]; joc.num_channels],
            slots_out: vec![[Complex::default(); BANDS]; joc.num_objects],
            frames: 0,
            splices: 0,
            seq_gaps: 0,
            last_seq: None,
            quad: carry.iter().map(|c| Quadrature::new(*c)).collect(),
            slots_rot: vec![[Complex::default(); BANDS]; joc.num_channels],
            low_delay,
            clip_gain: 1.0,
            apply_clip_gain: clip_gain,
        })
    }

    fn push(&mut self, element: usize, samples: impl Iterator<Item = f64>) {
        let drop = &mut self.to_drop[element];
        for s in samples {
            if *drop > 0 {
                *drop -= 1;
            } else {
                self.queues[element].push_back(s);
            }
        }
    }

    /// Feeds one frame of core PCM (per coded channel) and its JOC payload.
    fn frame(&mut self, pcm: &[&[f32]], joc: Option<&Joc>) {
        let samples = pcm[0].len();
        let num_ts = samples / BANDS;
        match joc {
            Some(j) => {
                // Clause 6.3.3.3: the sequence counter increments every frame
                // and wraps to 1 at 1 023; zero means the first frame of the
                // bitstream or the first frame after a splice. The matrices
                // from before a splice do not belong to what follows it, and
                // clause 6.6.5 requires the history to be zero before the
                // first frame, so it is forgotten here.
                if j.seq_count == 0 && self.frames > 0 {
                    self.joc.reset();
                    self.splices += 1;
                } else if let Some(prev) = self.last_seq {
                    let want = if prev >= 1023 { 1 } else { prev + 1 };
                    if j.seq_count != want {
                        self.seq_gaps += 1;
                    }
                }
                self.last_seq = Some(j.seq_count);
                self.joc.update(j, num_ts);
                self.clip_gain = if self.apply_clip_gain {
                    j.clipgain
                } else {
                    1.0
                };
            }
            None => self.joc.hold(num_ts),
        }
        // objects: analysis -> matrix -> synthesis, per time slot
        let mut object_out: Vec<Vec<f64>> = vec![Vec::with_capacity(samples); self.synthesis.len()];
        let mut chunk = [0.0f64; BANDS];
        let mut out = [0.0f64; BANDS];
        for ts in 0..num_ts {
            for (k, &coded) in self.joc_inputs.iter().enumerate() {
                for (i, c) in chunk.iter_mut().enumerate() {
                    *c = f64::from(pcm[coded][ts * BANDS + i]);
                }
                self.analysis[k].step(&chunk, &mut self.slots_in[k]);
            }
            for (k, q) in self.quad.iter_mut().enumerate() {
                q.step(&self.slots_in[k], &mut self.slots_rot[k]);
            }
            self.joc
                .reconstruct(ts, &self.slots_rot, &mut self.slots_out);
            for (obj, syn) in self.synthesis.iter_mut().enumerate() {
                syn.step(&self.slots_out[obj], &mut out);
                object_out[obj].extend_from_slice(&out);
            }
        }
        // Clause 6.3.3.2 gives the clip gain and never says what to do with
        // it. Encoding the same master at two levels shows the encoder
        // divides the whole downmix, LFE included, by it: the coded core came
        // out at exactly `scale / joc_clipgain` in every frame. The object
        // program is therefore restored by multiplying it back, while the
        // backwards-compatible core stays as coded. See `docs/joc.md`.
        let g = self.clip_gain;
        for (e, src) in self.sources.clone().iter().enumerate() {
            match *src {
                Source::Core(ch) => self.push(e, pcm[ch].iter().map(|&v| f64::from(v) * g)),
                Source::Object(o) => self.push(e, object_out[o].iter().map(|&v| v * g)),
            }
        }
        self.frames += 1;
    }

    /// Runs zeros through the banks so the objects catch up with the core.
    fn flush(&mut self) {
        let zeros = vec![
            vec![0.0f32; DECODER_DELAY + DELAY + self.low_delay + BANDS];
            self.joc_inputs.iter().max().map_or(1, |m| m + 1)
        ];
        // the hold keeps the last matrices
        let samples = zeros[0].len() / BANDS * BANDS;
        let mut trimmed: Vec<Vec<f32>> = zeros.into_iter().map(|z| z[..samples].to_vec()).collect();
        for z in &mut trimmed {
            z.truncate(samples);
        }
        let num_ts = samples / BANDS;
        self.joc.hold(num_ts);
        let mut chunk = [0.0f64; BANDS];
        let mut out = [0.0f64; BANDS];
        for ts in 0..num_ts {
            for k in 0..self.joc_inputs.len() {
                self.analysis[k].step(&chunk, &mut self.slots_in[k]);
            }
            for (k, q) in self.quad.iter_mut().enumerate() {
                q.step(&self.slots_in[k], &mut self.slots_rot[k]);
            }
            self.joc
                .reconstruct(ts, &self.slots_rot, &mut self.slots_out);
            for (obj, syn) in self.synthesis.iter_mut().enumerate() {
                syn.step(&self.slots_out[obj], &mut out);
                let e = self
                    .sources
                    .iter()
                    .position(|s| matches!(s, Source::Object(o) if *o == obj));
                if let Some(e) = e {
                    self.queues[e].extend(out.iter().copied());
                }
            }
            chunk = [0.0; BANDS];
        }
    }

    /// Rows that every element can supply.
    fn ready_rows(&self) -> usize {
        self.queues.iter().map(VecDeque::len).min().unwrap_or(0)
    }

    fn take_rows(&mut self, n: usize) -> Vec<Vec<i32>> {
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            let mut row = Vec::with_capacity(self.queues.len());
            for q in &mut self.queues {
                row.push(to_i24(q.pop_front().unwrap_or(0.0)));
            }
            rows.push(row);
        }
        rows
    }
}

/// Extracts the OAMD and JOC payloads of one substream's skip fields.
///
/// Which substream is [`ProgramFrame::metadata_part`]: the last dependent one
/// when the programme has any, else the independent one (TS 103 420 clause
/// 8.2).
fn frame_payloads(d: &Decoded, sparse: SparseReading) -> (Vec<(Oamd, u32)>, Option<Joc>, u64) {
    let mut oamd = Vec::new();
    let mut joc = None;
    let mut errors = 0u64;
    let total: usize = d.skip_fields.iter().map(Vec::len).sum();
    if total == 0 {
        return (oamd, joc, errors);
    }
    let mut data = Vec::with_capacity(total);
    for s in &d.skip_fields {
        data.extend_from_slice(s);
    }
    let mut pos = 0usize;
    while pos + 4 <= data.len() {
        if data[pos] != 0x58 || data[pos + 1] != 0x38 {
            pos += 1;
            continue;
        }
        match container::parse_emdf_with_sync(&data[pos..]) {
            Ok((c, used)) => {
                for p in &c.payloads {
                    if p.id == PAYLOAD_ID_OAMD {
                        match Oamd::parse(&p.data) {
                            Ok(o) => oamd.push((o, p.config.sample_offset.unwrap_or(0))),
                            Err(_) => errors += 1,
                        }
                    } else if p.id == PAYLOAD_ID_JOC {
                        match Joc::parse(&p.data, sparse) {
                            Ok(j) => joc = Some(j),
                            Err(_) => errors += 1,
                        }
                    }
                }
                pos += used.max(4);
            }
            Err(_) => {
                pos += 2;
            }
        }
    }
    (oamd, joc, errors)
}

/// Runs the object output for an E-AC-3 JOC stream; `base` is the output path
/// without extension.
pub fn run(path: &Path, base: &Path, opts: &Options) -> Result<Verdict> {
    let started = Instant::now();
    let dir = base
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = base
        .file_name()
        .and_then(|n| n.to_str())
        .context("output base name")?
        .to_string();
    std::fs::create_dir_all(dir)?;

    let mut decoder = ProgramDecoder::new(opts.core);
    let mut timeline = Timeline::new(opts.all_events);
    let mut sink: Option<Sink> = None;
    let mut pipeline: Option<Pipeline> = None;
    let mut program: Option<Program> = None;
    let mut core_samples: u64 = 0;
    let mut payload_errors: u64 = 0;
    let mut frames_without_joc: u64 = 0;
    let mut frames_without_oamd: u64 = 0;
    let mut crc_failures: u64 = 0;
    let mut tail_overruns: u64 = 0;
    let mut first_error: Option<String> = None;
    let mut rate = 48_000u32;
    let mut rows_written: u64 = 0;

    let mut handle = |frame: &ProgramFrame| -> Result<()> {
        let frame_index = frame.index;
        let d = frame.core();
        rate = d.header.sample_rate;
        for part in &frame.parts {
            if !part.decoded.crc_ok {
                crc_failures += 1;
                if first_error.is_none() {
                    first_error = Some(format!("group {frame_index}: CRC failure"));
                }
            }
            if part.decoded.tail_overrun {
                tail_overruns += 1;
                if first_error.is_none() {
                    first_error = Some(format!(
                        "group {frame_index}: the audio blocks end inside the frame tail"
                    ));
                }
            }
        }
        let (oamds, joc, errors) = frame_payloads(
            &frame.metadata_part().decoded,
            if opts.sparse_as_printed {
                SparseReading::AsPrinted
            } else {
                SparseReading::Measured
            },
        );
        payload_errors += errors;
        if joc.is_none() {
            frames_without_joc += 1;
        }
        if oamds.is_empty() {
            frames_without_oamd += 1;
        }
        let frame_start = core_samples;
        if pipeline.is_none() {
            let Some(j) = &joc else {
                bail!("the first frame carries no JOC payload");
            };
            let Some((o, _)) = oamds.first() else {
                bail!("the first frame carries no Object Audio Metadata");
            };
            let p = Program::from_oamd(o);
            pipeline = Some(Pipeline::new(
                j,
                &p,
                &frame.layout.channels,
                opts.clip_gain,
                opts.flat_quadrature,
                if opts.steep_as_printed {
                    SteepReading::AsPrinted
                } else {
                    SteepReading::Measured
                },
            )?);
            sink = Some(Sink::create(dir, &name, &p, rate, opts)?);
            eprintln!(
                "program: {} bed channels, {} dynamic objects, {} JOC objects over {} downmix channels (config {}), clip gain {:.3}{}",
                p.bed_channels().len(),
                p.dynamic_objects,
                j.num_objects,
                j.num_channels,
                j.dmx_config,
                j.clipgain,
                if opts.clip_gain {
                    " (applied)"
                } else {
                    " (reported, not applied)"
                }
            );
            program = Some(p);
        }
        let pl = pipeline.as_mut().expect("created above");
        let w = sink.as_mut().expect("created above");
        // metadata: event times count from the first frame's first sample, less the decoder delay
        for (o, smploffst) in &oamds {
            if let Some(p) = &program
                && Program::from_oamd(o) != *p
            {
                eprintln!("warning: frame {frame_index}: the program changed; keeping the first");
            }
            let (base, offset) = if frame_start >= DECODER_DELAY as u64 {
                (frame_start - DECODER_DELAY as u64, u64::from(*smploffst))
            } else {
                let short = DECODER_DELAY as u64 - frame_start;
                (0, u64::from(*smploffst).saturating_sub(short))
            };
            let mut io_error = None;
            let result = timeline.push(o, base, offset, |event| {
                if io_error.is_none()
                    && let Err(e) = w.push_event(event)
                {
                    io_error = Some(e);
                }
            });
            if let Some(e) = io_error {
                return Err(e.into());
            }
            if result.is_err() {
                payload_errors += 1;
            }
        }
        let channels: Vec<&[f32]> = (0..frame.channels()).map(|i| frame.channel(i)).collect();
        pl.frame(&channels, joc.as_ref());
        core_samples += frame.samples() as u64;
        let n = pl.ready_rows();
        if n > 0 {
            let rows = pl.take_rows(n);
            w.write_frames(rows.iter().map(Vec::as_slice), rows[0].len())?;
            rows_written += n as u64;
        }
        Ok(())
    };

    let (_, sync_errors, skipped) = for_each_frame(path, |_offset, bytes, header| {
        decoder.push(bytes, header)?;
        while let Some(frame) = decoder.pop() {
            handle(&frame)?;
        }
        Ok(())
    })?;
    decoder.finish()?;
    while let Some(frame) = decoder.pop() {
        handle(&frame)?;
    }
    let stats = decoder.stats().clone();
    let decode_errors: u64 = stats.decode_errors.values().sum();
    // `handle` borrows the pipeline and the sink; end that borrow.
    let _ = &mut handle;

    let (Some(mut pl), Some(sink)) = (pipeline, sink) else {
        bail!("no decodable frames");
    };
    pl.flush();
    // the output is as long as the core minus the decoder delay
    let target = core_samples.saturating_sub(DECODER_DELAY as u64);
    let remaining = (target - rows_written.min(target)) as usize;
    // pad queues that ran short (the LFE queue ends at the core length)
    for q in &mut pl.queues {
        while q.len() < remaining {
            q.push_back(0.0);
        }
    }
    let mut sink = sink;
    if remaining > 0 {
        let rows = pl.take_rows(remaining);
        sink.write_frames(rows.iter().map(Vec::as_slice), rows[0].len())?;
        rows_written += remaining as u64;
    }
    let summary = sink.finish(timeline.events)?;
    let elapsed = started.elapsed().as_secs_f64();
    eprintln!(
        "{}: {} frames x {} channels ({}) in {}{}",
        if opts.adm { "ADM BWF" } else { "DAMF" },
        summary.frames,
        summary.channels,
        format_duration(summary.frames as f64 / f64::from(rate.max(1))),
        summary
            .paths
            .last()
            .map_or_else(String::new, |p| p.display().to_string()),
        summary.note
    );
    crate::damf::note_non_profile(opts);
    eprintln!(
        "metadata: {} payloads, {} events ({} restating payloads, {} out-of-order events), {} payload errors; {} frames without JOC, {} without OAMD",
        timeline.payloads,
        summary.events,
        timeline.restatements,
        timeline.out_of_order,
        payload_errors,
        frames_without_joc,
        frames_without_oamd
    );
    if pl.splices > 0 || pl.seq_gaps > 0 {
        eprintln!(
            "splices: {} frames restarted the sequence counter, {} did not follow the previous one",
            pl.splices, pl.seq_gaps
        );
    }
    eprintln!(
        "core: {} frames decoded, {} decode errors, {:.2} s ({:.0}x realtime)",
        pl.frames,
        decode_errors,
        elapsed,
        if elapsed > 0.0 {
            rows_written as f64 / f64::from(rate) / elapsed
        } else {
            0.0
        }
    );
    let mut losses = timeline.losses.clone();
    losses.merge(&summary.losses);
    crate::damf::report_losses(
        if opts.adm { "adm" } else { "damf" },
        &losses,
        opts.loss_report.as_deref(),
    )?;
    let mut f = Findings::default();
    f.note_losses(&losses);
    f.note(decode_errors, "frames failed to decode");
    f.note(crc_failures, "CRC failures");
    f.note(tail_overruns, "frames ending inside the frame tail");
    f.note(sync_errors, "sync errors");
    f.note(skipped, "bytes skipped");
    f.note(payload_errors, "metadata payload errors");
    f.note(
        stats.dependent_dropped,
        "dependent substream frames dropped",
    );
    f.note(
        stats.orphan_dependents,
        "dependent frames with no independent substream",
    );
    f.note(stats.location_errors, "unreadable channel maps");
    f.note(stats.layout_changes, "mid-stream channel layout changes");
    f.note(
        stats.duplicate_substream_frames,
        "substream frames repeated within one group",
    );
    f.first_problem(first_error.or_else(|| stats.first_error.clone()).as_deref());
    Ok(f.report())
}
