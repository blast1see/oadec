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
use std::fmt;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use oadec_eac3::{ChannelLoc, ProgramDecoder, ProgramFrame};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::joc::{Joc, JocError, JocHeader, SparseReading};
use oadec_emdf::oamd::{BedChannel, Oamd};
use oadec_joc::{
    Analysis, BANDS, Carry, Complex, DELAY, JocDecoder, LOW_DELAY, MATRIX_ALIGN, Quadrature,
    SteepReading, Synthesis,
};
use oadec_spatial::{LossLedger, Program, Timeline};
use serde_json::{Value, json};

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

/// Time slots the matrices are held back by when `--joc-lag` does not say
/// otherwise: what the low-band filter costs, less the measured alignment.
/// `MATRIX_ALIGN` has this one consumer, so the flag moves the alignment
/// without touching the source.
const DEFAULT_LAG: usize = LOW_DELAY - MATRIX_ALIGN;

/// `--joc-low-band`: how the lowest subband of a channel that carries the
/// 90-degree phase shift is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum JocLowBand {
    /// Corrected the way the Dolby decoder corrects it, or rotated with the
    /// other subbands under `--flat-quadrature` (the default).
    Filtered,
    /// Left alone while every other subband is rotated: the other end of the
    /// measurement the correction was fitted between.
    Untouched,
}

impl JocLowBand {
    const fn name(self) -> &'static str {
        match self {
            Self::Filtered => "filtered",
            Self::Untouched => "untouched",
        }
    }
}

/// `--joc-phase`: which downmix channels carry the 90-degree phase shift and
/// which way it is taken out, instead of what the downmix configuration says.
/// Written `CH,CH:-` or `CH,CH:+` (rotated back by -j or by +j), or `none`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JocPhase {
    channels: Vec<usize>,
    plus: bool,
}

/// Why a `--joc-phase` value was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JocPhaseError {
    /// No `:` between the channels and the direction.
    NoSign,
    /// A direction other than `+` or `-`.
    Sign(String),
    /// A channel that is not a number.
    Channel(String),
}

impl fmt::Display for JocPhaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSign => write!(f, "expected CH,CH:-, CH,CH:+ or none"),
            Self::Sign(sign) => write!(f, "the direction {sign:?} is neither + nor -"),
            Self::Channel(ch) => write!(f, "{ch:?} is not a channel index"),
        }
    }
}

impl std::error::Error for JocPhaseError {}

impl JocPhase {
    /// Parses what `OADEC_JOC_PHASE` took, and refuses anything else.
    pub fn parse(spec: &str) -> Result<Self, JocPhaseError> {
        let spec = spec.trim();
        if spec == "none" {
            return Ok(Self {
                channels: Vec::new(),
                plus: false,
            });
        }
        let (channels, sign) = spec.split_once(':').ok_or(JocPhaseError::NoSign)?;
        let plus = match sign.trim() {
            "+" => true,
            "-" => false,
            other => return Err(JocPhaseError::Sign(other.to_string())),
        };
        let channels = channels
            .split(',')
            .map(|ch| {
                ch.trim()
                    .parse()
                    .map_err(|_| JocPhaseError::Channel(ch.trim().to_string()))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { channels, plus })
    }
}

impl fmt::Display for JocPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.channels.is_empty() {
            return write!(f, "none");
        }
        let channels: Vec<String> = self.channels.iter().map(ToString::to_string).collect();
        write!(
            f,
            "{}:{}",
            channels.join(","),
            if self.plus { '+' } else { '-' }
        )
    }
}

/// One measurement override, as the stderr line and the loss report name it.
#[derive(Debug, Clone, PartialEq)]
struct Override {
    flag: &'static str,
    value: Value,
    default: Value,
}

/// A JSON value as a person reads it: a string without its quotes.
fn plain(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

/// Measurement overrides of the JOC reconstruction, none by default. They were
/// environment variables that changed the decode without a word; as flags they
/// are announced on stderr and recorded in the loss report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JocOverrides {
    /// `--joc-lag`: time slots the matrices are held back by.
    pub lag: Option<usize>,
    /// `--joc-low-band`.
    pub low_band: Option<JocLowBand>,
    /// `--joc-phase`.
    pub phase: Option<JocPhase>,
}

impl JocOverrides {
    /// The overrides given, as the stderr line and the loss report name them.
    fn list(&self) -> Vec<Override> {
        let mut out = Vec::new();
        if let Some(lag) = self.lag {
            out.push(Override {
                flag: "joc-lag",
                value: json!(lag),
                default: json!(DEFAULT_LAG),
            });
        }
        if let Some(band) = self.low_band {
            out.push(Override {
                flag: "joc-low-band",
                value: json!(band.name()),
                default: json!(JocLowBand::Filtered.name()),
            });
        }
        if let Some(phase) = &self.phase {
            out.push(Override {
                flag: "joc-phase",
                value: json!(phase.to_string()),
                default: json!("3,4:- for downmix configurations 3 and 4, none otherwise"),
            });
        }
        out
    }

    /// The stderr line naming every override given, or `None` when there is none.
    pub(crate) fn line(&self) -> Option<String> {
        let list = self.list();
        if list.is_empty() {
            return None;
        }
        let parts: Vec<String> = list
            .iter()
            .map(|o| {
                format!(
                    "{} {} (default {})",
                    o.flag,
                    plain(&o.value),
                    plain(&o.default)
                )
            })
            .collect();
        Some(format!("measurement overrides: {}", parts.join("; ")))
    }

    /// The overrides as the loss report records them; empty when there is none.
    pub(crate) fn json(&self) -> Vec<Value> {
        self.list()
            .into_iter()
            .map(|o| json!({"flag": o.flag, "value": o.value, "default": o.default}))
            .collect()
    }
}

/// What each JOC downmix channel carries. Configurations 3 and 4 of table 47
/// hand the surround pair over with a 90-degree phase shift; rotating Ls and
/// Rs back by -j puts the objects in phase with the source (measured on the
/// encoder round trip, see `docs/evidence`). Every channel gets a
/// [`Quadrature`] whether or not it carries a shift, because they all need
/// the same hold for the matrix alignment. `--joc-phase` overrides which
/// channels are shifted and `--joc-low-band untouched` selects the third
/// reading of the shift, both for measuring.
fn carries(dmx_config: u8, channels: usize, flat: bool, overrides: &JocOverrides) -> Vec<Carry> {
    let shift = |plus| match (flat, overrides.low_band) {
        (_, Some(JocLowBand::Untouched)) => Carry::FlatAbove { plus },
        (true, _) => Carry::Flat { plus },
        (false, _) => Carry::Shifted { plus },
    };
    let mut spec: Vec<(usize, bool)> = match &overrides.phase {
        None if matches!(dmx_config, 3 | 4) => vec![(3, false), (4, false)],
        None => Vec::new(),
        Some(phase) => phase.channels.iter().map(|&ch| (ch, phase.plus)).collect(),
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
        joc: &JocHeader,
        program: &Program,
        chans: &[ChannelLoc],
        clip_gain: bool,
        flat_quadrature: bool,
        steep: SteepReading,
        overrides: &JocOverrides,
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
        let carry = carries(joc.dmx_config, joc.num_channels, flat_quadrature, overrides);
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
        let lag = overrides.lag.unwrap_or(DEFAULT_LAG);
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

/// What the skip fields of one substream carried for the object path.
#[derive(Debug, Default)]
struct FramePayloads {
    /// Every Object Audio Metadata payload that parsed, with its sample offset.
    oamd: Vec<(Oamd, u32)>,
    /// The JOC payload, when one parsed.
    joc: Option<Joc>,
    /// Payloads that would not parse.
    errors: u64,
    /// JOC payloads that parsed but whose declared size is not what their
    /// syntax consumed.
    joc_size_mismatches: u64,
    /// The header of a JOC payload whose `joc_ext_config_idx` is reserved
    /// (TS 103 420 table 49): the payload was read through and refused, so its
    /// matrices are not used, and the header is all that is known of it.
    joc_reserved: Option<JocHeader>,
}

/// Extracts the OAMD and JOC payloads of one substream's skip fields.
///
/// Which substream is [`ProgramFrame::metadata_part`]: the last dependent one
/// when the programme has any, else the independent one (TS 103 420 clause
/// 8.2).
fn frame_payloads(skip_fields: &[Vec<u8>], sparse: SparseReading) -> FramePayloads {
    let mut out = FramePayloads::default();
    let total: usize = skip_fields.iter().map(Vec::len).sum();
    if total == 0 {
        return out;
    }
    let mut data = Vec::with_capacity(total);
    for s in skip_fields {
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
                            Ok(o) => out.oamd.push((o, p.config.sample_offset.unwrap_or(0))),
                            Err(_) => out.errors += 1,
                        }
                    } else if p.id == PAYLOAD_ID_JOC {
                        match Joc::parse(&p.data, sparse) {
                            Ok(j) => {
                                // the check verify and the PCM path make
                                if !j.size_ok(p.data.len()) {
                                    out.joc_size_mismatches += 1;
                                }
                                out.joc = Some(j);
                            }
                            // read through and refused, so the header is intact
                            Err(JocError::ExtConfig(_)) => match JocHeader::parse(&p.data) {
                                Ok(h) => out.joc_reserved = Some(h),
                                Err(_) => out.errors += 1,
                            },
                            Err(_) => out.errors += 1,
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
    out
}

/// What the object decode counted that bears on its verdict.
#[derive(Debug, Default)]
struct Tally {
    decode_errors: u64,
    crc_failures: u64,
    /// Frames that end inside their own tail. Out of spec, and decoded to the
    /// audio FFmpeg, Dolby and oadec agree on, so they are printed and are not
    /// a fault of the delivery (`docs/exit-codes.md`); `verify` still calls the
    /// file non-conformant.
    tail_overruns: u64,
    /// The frame group the first of them is in.
    first_tail: Option<u64>,
    sync_errors: u64,
    skipped: u64,
    payload_errors: u64,
    /// JOC payloads that parsed but declared a size their syntax did not fill.
    joc_size_mismatches: u64,
    /// JOC payloads whose `joc_ext_config_idx` is reserved; the matrices of
    /// the previous frame were held in their place.
    joc_reserved_ext: u64,
    first_error: Option<String>,
}

impl Tally {
    /// The verdict of the object decode, from what it counted, what the
    /// programme assembly found and what the writers could not carry.
    fn findings(&self, stats: &oadec_eac3::ProgramStats, losses: &LossLedger) -> Findings {
        let mut f = Findings::default();
        f.note_losses(losses);
        f.note(self.decode_errors, "frames failed to decode");
        f.note(self.crc_failures, "CRC failures");
        f.note(self.sync_errors, "sync errors");
        f.note(self.skipped, "bytes skipped");
        f.note(self.payload_errors, "metadata payload errors");
        f.note(
            self.joc_size_mismatches,
            "JOC payloads whose declared size was wrong",
        );
        f.note(
            self.joc_reserved_ext,
            "JOC payloads with a reserved joc_ext_config_idx, matrices held from the previous frame",
        );
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
        f.first_problem(self.first_error.as_deref().or(stats.first_error.as_deref()));
        f
    }
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
    if let Some(line) = opts.joc.line() {
        eprintln!("{line}");
    }

    let mut decoder = ProgramDecoder::new(opts.core);
    let mut timeline = Timeline::new(opts.all_events);
    let mut sink: Option<Sink> = None;
    let mut pipeline: Option<Pipeline> = None;
    let mut program: Option<Program> = None;
    let mut core_samples: u64 = 0;
    let mut tally = Tally::default();
    let mut frames_without_joc: u64 = 0;
    let mut frames_without_oamd: u64 = 0;
    let mut rate = 48_000u32;
    let mut rows_written: u64 = 0;

    let mut handle = |frame: &ProgramFrame| -> Result<()> {
        let frame_index = frame.index;
        let d = frame.core();
        rate = d.header.sample_rate;
        for part in &frame.parts {
            if !part.decoded.crc_ok {
                tally.crc_failures += 1;
                if tally.first_error.is_none() {
                    tally.first_error = Some(format!("group {frame_index}: CRC failure"));
                }
            }
            if part.decoded.tail_overrun {
                tally.tail_overruns += 1;
                tally.first_tail.get_or_insert(frame_index);
            }
        }
        let FramePayloads {
            oamd: oamds,
            joc,
            errors,
            joc_size_mismatches,
            joc_reserved,
        } = frame_payloads(
            &frame.metadata_part().decoded.skip_fields,
            if opts.sparse_as_printed {
                SparseReading::AsPrinted
            } else {
                SparseReading::Measured
            },
        );
        tally.payload_errors += errors;
        tally.joc_size_mismatches += joc_size_mismatches;
        if let Some(h) = joc_reserved {
            // nothing after joc_data can be read, so the payload is not used
            // and the matrices of the previous frame hold
            tally.joc_reserved_ext += 1;
            if tally.first_error.is_none() {
                tally.first_error = Some(format!(
                    "frame {frame_index}: joc_ext_config_idx {} is reserved; matrices held from the previous frame",
                    h.ext_config
                ));
            }
        }
        if joc.is_none() {
            frames_without_joc += 1;
        }
        if oamds.is_empty() {
            frames_without_oamd += 1;
        }
        let frame_start = core_samples;
        if pipeline.is_none() {
            // A payload with a reserved extension still says what the downmix
            // and the objects are, so a stream that starts with one decodes
            // from the zero history of clause 6.6.5 instead of not at all.
            let Some(header) = joc.as_ref().map(Joc::header).or(joc_reserved) else {
                bail!("the first frame carries no JOC payload");
            };
            let Some((o, _)) = oamds.first() else {
                bail!("the first frame carries no Object Audio Metadata");
            };
            let p = Program::from_oamd(o);
            pipeline = Some(Pipeline::new(
                &header,
                &p,
                &frame.layout.channels,
                opts.clip_gain,
                opts.flat_quadrature,
                if opts.steep_as_printed {
                    SteepReading::AsPrinted
                } else {
                    SteepReading::Measured
                },
                &opts.joc,
            )?);
            sink = Some(Sink::create(dir, &name, &p, rate, opts)?);
            let clip = match &joc {
                Some(j) => format!(
                    "clip gain {:.3}{}",
                    j.clipgain,
                    if opts.clip_gain {
                        " (applied)"
                    } else {
                        " (reported, not applied)"
                    }
                ),
                None => "clip gain not read (the first JOC payload has a reserved extension)"
                    .to_string(),
            };
            eprintln!(
                "program: {} bed channels, {} dynamic objects, {} JOC objects over {} downmix channels (config {}), {clip}",
                p.bed_channels().len(),
                p.dynamic_objects,
                header.num_objects,
                header.num_channels,
                header.dmx_config,
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
                tally.payload_errors += 1;
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
    // `handle` borrows the pipeline, the sink and the tally; end that borrow.
    let _ = &mut handle;
    tally.decode_errors = stats.decode_errors.values().sum();
    tally.sync_errors = sync_errors;
    tally.skipped = skipped;

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
        tally.payload_errors,
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
        tally.decode_errors,
        elapsed,
        if elapsed > 0.0 {
            rows_written as f64 / f64::from(rate) / elapsed
        } else {
            0.0
        }
    );
    if let Some(group) = tally.first_tail {
        eprintln!(
            "out of spec: {} frames ending inside the frame tail, the first in group {group}; decoded as FFmpeg and Dolby decode them, which is not an integrity fault (`oadec verify` reports it)",
            tally.tail_overruns
        );
    }
    let mut losses = timeline.losses.clone();
    losses.merge(&summary.losses);
    crate::damf::report_losses(
        if opts.adm { "adm" } else { "damf" },
        &losses,
        opts.loss_report.as_deref(),
        &opts.joc.json(),
    )?;
    Ok(tally.findings(&stats, &losses).report())
}

#[cfg(test)]
mod tests {
    use super::*;

    use oadec_bits::BitWriter;

    /// The smallest JOC payload there is: a 5.X downmix and one object, absent.
    fn joc_payload(ext_config: u32) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write(0, 3); // joc_dmx_config_idx
        w.write(0, 6); // joc_num_objects_bits: one object
        w.write(ext_config, 3); // joc_ext_config_idx
        w.write(4, 3); // joc_clipgain_x_bits: 2^0
        w.write(0, 5); // joc_clipgain_y_bits
        w.write(7, 10); // joc_seq_count_bits
        w.write(0, 1); // b_joc_obj_present
        w.finish()
    }

    /// An EMDF container as a skip field carries it, sync word and length
    /// included, holding one JOC payload of `data` configured as DEE writes it.
    fn emdf_with_joc(data: &[u8]) -> Vec<u8> {
        let mut w = BitWriter::new();
        w.write(0, 2); // emdf_version
        w.write(0, 3); // key_id
        w.write(PAYLOAD_ID_JOC, 5);
        w.write(0, 1); // smploffste
        w.write(0, 1); // duratione
        w.write(1, 1); // groupide
        w.write(0, 3); // groupid 0, no further group
        w.write(0, 1); // codecdatae
        w.write(0, 1); // discard_unknown_payload
        w.write(1, 1); // payload_frame_aligned
        w.write(0, 2); // create_duplicate, remove_duplicate
        w.write(0, 7); // priority, proc_allowed
        w.write(data.len() as u32, 8); // emdf_payload_size
        w.write(0, 1); // no further group
        for &b in data {
            w.write(u32::from(b), 8);
        }
        w.write(0, 5); // the payload list ends
        w.write(0, 4); // no protection words
        let body = w.finish();
        let mut out = vec![0x58, 0x38];
        out.extend_from_slice(&(body.len() as u16).to_be_bytes());
        out.extend(body);
        out
    }

    /// A JOC payload that parses but declares a byte its syntax never reaches
    /// went straight into the reconstruction, and the object output exited
    /// clean where `verify` and the PCM path exit 7. The payload is still used;
    /// the mismatch is counted.
    #[test]
    fn the_object_path_counts_a_joc_payload_whose_declared_size_is_wrong() {
        let exact = joc_payload(0);
        let payloads = frame_payloads(&[emdf_with_joc(&exact)], SparseReading::Measured);
        assert!(payloads.joc.is_some(), "the payload parses");
        assert_eq!((payloads.errors, payloads.joc_size_mismatches), (0, 0));

        let mut long = exact;
        long.push(0);
        let payloads = frame_payloads(&[emdf_with_joc(&long)], SparseReading::Measured);
        assert!(
            payloads.joc.is_some(),
            "a trailing byte does not stop the parse"
        );
        assert_eq!(
            payloads.joc_size_mismatches, 1,
            "the trailing byte went unnoticed"
        );
    }

    /// A payload whose `joc_ext_config_idx` is reserved is not a parse error
    /// among the others: it is kept apart with its header, so the run can name
    /// it and a stream that opens with one can still start the pipeline, and
    /// its matrices are not used.
    #[test]
    fn a_reserved_joc_extension_is_kept_apart_with_its_header() {
        let payloads = frame_payloads(&[emdf_with_joc(&joc_payload(5))], SparseReading::Measured);
        assert!(payloads.joc.is_none(), "its matrices must not be used");
        assert_eq!(payloads.errors, 0, "it is not counted as a parse error");
        let header = payloads.joc_reserved.expect("the reserved payload is kept");
        assert_eq!(
            (header.ext_config, header.num_channels, header.num_objects),
            (5, 5, 1)
        );

        let tally = Tally {
            joc_reserved_ext: 1,
            ..Tally::default()
        };
        let f = tally.findings(&oadec_eac3::ProgramStats::default(), &LossLedger::default());
        let shown = format!("{f:?}");
        assert!(
            !f.is_clean() && shown.contains("1 JOC payloads with a reserved joc_ext_config_idx"),
            "{shown}"
        );
    }

    /// And the count reaches the verdict.
    #[test]
    fn a_joc_size_mismatch_fails_the_object_output() {
        let tally = Tally {
            joc_size_mismatches: 1,
            ..Tally::default()
        };
        let f = tally.findings(&oadec_eac3::ProgramStats::default(), &LossLedger::default());
        let shown = format!("{f:?}");
        assert!(
            !f.is_clean() && shown.contains("1 JOC payloads whose declared size was wrong"),
            "{shown}"
        );
    }

    /// Downmix configurations 3 and 4 carry the 90-degree phase shift on Ls
    /// and Rs (table 47), and without an override that is what the carriers
    /// follow; `--joc-low-band` and `--joc-phase` change them, for measuring,
    /// and nothing else does.
    #[test]
    fn carries_follows_the_downmix_configuration_unless_overridden() {
        use Carry::{Flat, FlatAbove, Plain, Shifted};
        let minus = Shifted { plus: false };
        let none = JocOverrides::default();
        assert_eq!(
            carries(3, 5, false, &none),
            [Plain, Plain, Plain, minus, minus]
        );
        assert_eq!(carries(4, 7, false, &none)[3..5], [minus, minus]);
        assert_eq!(carries(0, 5, false, &none), [Plain; 5]);
        assert_eq!(carries(3, 5, true, &none)[3..5], [Flat { plus: false }; 2]);

        let low = |band| JocOverrides {
            low_band: Some(band),
            ..JocOverrides::default()
        };
        assert_eq!(
            carries(3, 5, false, &low(JocLowBand::Untouched))[3..5],
            [FlatAbove { plus: false }; 2]
        );
        assert_eq!(
            carries(3, 5, true, &low(JocLowBand::Untouched))[3],
            FlatAbove { plus: false },
            "untouched wins over --flat-quadrature, as the variable did"
        );
        assert_eq!(
            carries(3, 5, false, &low(JocLowBand::Filtered)),
            carries(3, 5, false, &none)
        );

        let phase = |spec: &str| JocOverrides {
            phase: Some(JocPhase::parse(spec).unwrap()),
            ..JocOverrides::default()
        };
        assert_eq!(carries(3, 5, false, &phase("none")), [Plain; 5]);
        assert_eq!(
            carries(0, 5, false, &phase("1,2:+")),
            [
                Plain,
                Shifted { plus: true },
                Shifted { plus: true },
                Plain,
                Plain
            ]
        );
        assert_eq!(
            carries(3, 5, false, &phase("4,9:-")),
            [Plain, Plain, Plain, Plain, minus],
            "a channel the downmix does not have is ignored"
        );
    }

    /// `--joc-phase` takes what `OADEC_JOC_PHASE` took, parses it once, and
    /// says what is wrong with anything else instead of reading it as "none".
    #[test]
    fn a_joc_phase_is_parsed_once_with_a_typed_error() {
        for spec in ["3,4:-", "1,2:+", "none"] {
            assert_eq!(JocPhase::parse(spec).unwrap().to_string(), spec);
        }
        assert_eq!(JocPhase::parse(" 3, 4 : - ").unwrap().to_string(), "3,4:-");
        assert_eq!(JocPhase::parse("3,4"), Err(JocPhaseError::NoSign));
        assert_eq!(JocPhase::parse(""), Err(JocPhaseError::NoSign));
        assert_eq!(
            JocPhase::parse("3,4:x"),
            Err(JocPhaseError::Sign("x".to_string()))
        );
        assert_eq!(
            JocPhase::parse("3,Ls:-"),
            Err(JocPhaseError::Channel("Ls".to_string()))
        );
    }

    /// The object output weighs a frame that ends inside its own tail as the
    /// PCM path does: counted and printed, and not a fault
    /// (`docs/exit-codes.md`).
    #[test]
    fn a_tail_overrun_alone_does_not_fail_the_object_output() {
        let stats = oadec_eac3::ProgramStats::default();
        let losses = LossLedger::default();
        let tail = Tally {
            tail_overruns: 1,
            first_tail: Some(224),
            ..Tally::default()
        };
        let f = tail.findings(&stats, &losses);
        assert!(
            f.is_clean(),
            "the object output failed on a tail overrun alone: {f:?}"
        );
        let crc = Tally {
            tail_overruns: 1,
            crc_failures: 1,
            ..Tally::default()
        };
        assert!(
            !crc.findings(&stats, &losses).is_clean(),
            "a real fault beside it still fails"
        );
    }
}
