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
use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_eac3::{ChannelLoc, ProgramDecoder, ProgramFrame};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::joc::{Joc, JocError, JocHeader, SparseReading};
use oadec_emdf::oamd::{BedChannel, Oamd};
use oadec_joc::{
    Analysis, BANDS, Carry, Complex, DELAY, JocDecoder, LOW_DELAY, LOW_TAPS, MATRIX_ALIGN,
    Quadrature, SteepReading, Synthesis,
};
use oadec_spatial::{LossLedger, Program, Timeline, declared_warp};
use serde_json::{Value, json};

use crate::damf::{Options, Sink, Warp, note_no_warp_mode, open_sink};
use crate::decode::format_duration;
use crate::eac3::for_each_frame;
use crate::integrity::{Findings, Verdict};

mod tap;
pub use tap::{JocTap, TapOptions, TapSlot, TapStart};

/// Samples the core decoder emits before the first frame's audio proper (the
/// first block's half window). The Dolby decoder drops them; the metadata
/// timing counts from the first frame's first sample as it does.
///
/// Public so that a test can work out from the constants, before it looks at
/// any sample, where an object and the core channel it was made of meet.
pub const DECODER_DELAY: usize = 256;

/// Samples a change of the clip gain takes: the first block of a frame.
const CLIP_GAIN_RAMP: usize = 256;

/// The clip gain at sample `at` of what a frame decodes to, `before` being
/// the gain of the frame before it.
///
/// Clause 6.3.3.2 gives the gain a value and no use. Read off the object
/// output of the Dolby decoder, which is run as it stands, the gain of a frame
/// is reached in a straight line over the first 256 samples the frame decodes
/// to, from the gain of the frame before, and held for the rest of the frame:
/// 65 changes on one stream give 0,029, 0,092, 0,154 of the way in the first
/// three steps of 16 samples and all of it from the 256th sample, on the LFE
/// and on the objects alike (`docs/joc.md`).
fn clip_gain_at(before: f64, of_the_frame: f64, at: usize) -> f64 {
    if at >= CLIP_GAIN_RAMP {
        of_the_frame
    } else {
        before + (of_the_frame - before) * (at as f64 / CLIP_GAIN_RAMP as f64)
    }
}

fn to_i24(v: f64) -> i32 {
    (v * 8_388_608.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// Puts the samples of an element on its queue, but for the first `to_drop`
/// of them, which are the delay of the decoder and are no part of the output.
fn queue_up(
    queue: &mut VecDeque<f64>,
    to_drop: &mut usize,
    mut samples: impl Iterator<Item = f64>,
) {
    while *to_drop > 0 {
        if samples.next().is_none() {
            return;
        }
        *to_drop -= 1;
    }
    queue.extend(samples);
}

/// Takes `n` rows off the queues into `rows`: a sample of every element in
/// turn, as 24-bit values. A queue that has run short gives silence for what
/// it does not have. `rows` is made the size of the rows and keeps what it
/// has allocated: one buffer serves every frame of a delivery.
fn rows_off(queues: &mut [VecDeque<f64>], n: usize, rows: &mut Vec<i32>) {
    let elements = queues.len();
    rows.clear();
    rows.resize(n * elements, 0);
    for (element, queue) in queues.iter_mut().enumerate() {
        let have = n.min(queue.len());
        for (row, sample) in rows.chunks_exact_mut(elements).zip(queue.drain(..have)) {
            row[element] = to_i24(sample);
        }
    }
}

/// The element each of the `objects` JOC objects of a stream is written as,
/// and none for an object the program does not use.
fn object_elements(sources: &[Source], objects: usize) -> Vec<Option<usize>> {
    (0..objects)
        .map(|object| {
            sources
                .iter()
                .position(|s| matches!(s, Source::Object(o) if *o == object))
        })
        .collect()
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
    /// Per JOC object: the element it is written as, where the program uses
    /// it.
    object_element: Vec<Option<usize>>,
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
    /// `joc_clipgain` of the frame in flight, or 1 while it is switched off;
    /// between frames, of the frame before, which the next one starts from.
    clip_gain: f64,
    /// Whether to restore the level the encoder took off (`--no-clip-gain`
    /// turns it off, for measuring the difference).
    apply_clip_gain: bool,
    /// What each JOC input channel carries into its [`Quadrature`]; told to a
    /// tap.
    carry: Vec<Carry>,
    /// Time slots the matrices are held back by; told to a tap.
    lag: usize,
    /// At a splice, clear the analysis, quadrature and synthesis banks as
    /// well as the matrix history ([`TapOptions::reset_banks`]). Off but for
    /// a measurement that asks for it.
    reset_banks: bool,
}

/// `--joc-low-band`: how the lowest subband of a channel that carries the
/// 90-degree phase shift is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// `--joc-low-fir`: an operator on the lowest subband of the channels that
/// carry the 90-degree phase shift, instead of the one measured on the Dolby
/// decoder. A file of a tap a line, real part then imaginary part, the first
/// line the tap that meets the newest time slot. There is an odd number of
/// them, so that one is the middle, and at least as many as the operator
/// there is has: every channel is held back by half the length, and a matrix
/// cannot be had before its frame is read.
#[derive(Debug, Clone, PartialEq)]
pub struct JocLowFir {
    /// The file as it was named.
    file: String,
    taps: Vec<Complex>,
}

/// Why a `--joc-low-fir` was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JocLowFirError {
    /// The file could not be read.
    Read { file: String, why: String },
    /// A line that is not two finite numbers.
    Line { line: usize, text: String },
    /// A number of taps without a middle, or too few.
    Taps(usize),
}

impl fmt::Display for JocLowFirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { file, why } => write!(f, "cannot read {file}: {why}"),
            Self::Line { line, text } => write!(
                f,
                "line {line} is not a real part and an imaginary part: {text:?}"
            ),
            Self::Taps(n) => write!(
                f,
                "{n} taps: an operator has an odd number of them, {LOW_TAPS} or more"
            ),
        }
    }
}

impl std::error::Error for JocLowFirError {}

impl JocLowFir {
    /// Reads the file a `--joc-low-fir` names.
    pub fn read(file: &str) -> Result<Self, JocLowFirError> {
        let text = std::fs::read_to_string(file).map_err(|e| JocLowFirError::Read {
            file: file.to_string(),
            why: e.to_string(),
        })?;
        Self::parse(file, &text)
    }

    /// The operator a text holds; `file` is what the announcement calls it.
    /// Empty lines are passed over.
    pub fn parse(file: &str, text: &str) -> Result<Self, JocLowFirError> {
        let mut taps = Vec::new();
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace().map(str::parse::<f64>);
            match (parts.next(), parts.next(), parts.next()) {
                (Some(Ok(re)), Some(Ok(im)), None) if re.is_finite() && im.is_finite() => {
                    taps.push(Complex { re, im });
                }
                _ => {
                    return Err(JocLowFirError::Line {
                        line: i + 1,
                        text: line.to_string(),
                    });
                }
            }
        }
        Self::new(file, taps)
    }

    /// An operator that did not come of a file; `name` is what the
    /// announcement calls it.
    pub fn new(name: &str, taps: Vec<Complex>) -> Result<Self, JocLowFirError> {
        if taps.len().is_multiple_of(2) || taps.len() < LOW_TAPS {
            return Err(JocLowFirError::Taps(taps.len()));
        }
        Ok(Self {
            file: name.to_string(),
            taps,
        })
    }

    /// Time slots it reaches either side of its middle.
    fn reach(&self) -> usize {
        self.taps.len() / 2
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JocOverrides {
    /// `--joc-lag`: time slots the matrices are held back by.
    pub lag: Option<usize>,
    /// `--joc-low-band`.
    pub low_band: Option<JocLowBand>,
    /// `--joc-phase`.
    pub phase: Option<JocPhase>,
    /// `--joc-low-fir`.
    pub low_fir: Option<JocLowFir>,
}

impl JocOverrides {
    /// Time slots every channel is held back by: the reach of the operator
    /// on the lowest subband.
    fn reach(&self) -> usize {
        self.low_fir.as_ref().map_or(LOW_DELAY, JocLowFir::reach)
    }

    /// Time slots the matrices are held back by when `--joc-lag` does not say
    /// otherwise: what the operator holds the samples back by, less the
    /// measured alignment. With the operator there is that is nothing; a
    /// longer one from `--joc-low-fir` holds the samples back further and the
    /// matrices follow. `MATRIX_ALIGN` has this one consumer, so the flag
    /// moves the alignment without touching the source.
    fn default_lag(&self) -> usize {
        self.reach() - MATRIX_ALIGN
    }

    /// The overrides given, as the stderr line and the loss report name them.
    fn list(&self) -> Vec<Override> {
        let mut out = Vec::new();
        if let Some(lag) = self.lag {
            out.push(Override {
                flag: "joc-lag",
                value: json!(lag),
                default: json!(self.default_lag()),
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
        if let Some(fir) = &self.low_fir {
            out.push(Override {
                flag: "joc-low-fir",
                value: json!(format!("{} ({} taps)", fir.file, fir.taps.len())),
                default: json!(format!("the {LOW_TAPS} taps measured on the Dolby decoder")),
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
    #[allow(
        clippy::too_many_arguments,
        reason = "the pipeline has this many inputs"
    )]
    fn new(
        joc: &JocHeader,
        program: &Program,
        chans: &[ChannelLoc],
        clip_gain: bool,
        flat_quadrature: bool,
        steep: SteepReading,
        overrides: &JocOverrides,
        remarks: &mut dyn Write,
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
            return Err(MissingDownmix {
                dmx_config: joc.dmx_config,
                missing: joc_inputs
                    .iter()
                    .enumerate()
                    .filter(|&(_, &coded)| coded == usize::MAX)
                    .map(|(i, _)| ChannelLoc::joc_input_name(joc.dmx_config, i).unwrap_or("?"))
                    .collect(),
                channels: chans.iter().map(|c| c.name()).collect(),
            }
            .into());
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
            writeln!(
                remarks,
                "warning: the stream carries {} JOC objects, the program uses {}",
                joc.num_objects, next_object
            )?;
        }
        let elements = sources.len();
        let object_element = object_elements(&sources, joc.num_objects);
        let carry = carries(joc.dmx_config, joc.num_channels, flat_quadrature, overrides);
        // every stream goes through the filter bank, whether or not it
        // carries a phase shift, because the matrix alignment needs the delay
        let low_delay = overrides.reach() * BANDS;
        let to_drop = sources
            .iter()
            .map(|s| match s {
                Source::Core(_) => DECODER_DELAY,
                Source::Object(_) => DECODER_DELAY + DELAY + low_delay,
            })
            .collect();
        let mut joc_decoder = JocDecoder::new(joc.num_channels, joc.num_objects);
        let lag = overrides.lag.unwrap_or_else(|| overrides.default_lag());
        joc_decoder.set_lag(lag);
        joc_decoder.set_steep_reading(steep);
        Ok(Self {
            joc: joc_decoder,
            analysis: (0..joc.num_channels).map(|_| Analysis::new()).collect(),
            synthesis: (0..joc.num_objects).map(|_| Synthesis::new()).collect(),
            joc_inputs,
            sources,
            object_element,
            queues: (0..elements).map(|_| VecDeque::new()).collect(),
            to_drop,
            slots_in: vec![[Complex::default(); BANDS]; joc.num_channels],
            slots_out: vec![[Complex::default(); BANDS]; joc.num_objects],
            frames: 0,
            splices: 0,
            seq_gaps: 0,
            last_seq: None,
            quad: carry
                .iter()
                .map(|c| match &overrides.low_fir {
                    Some(fir) => Quadrature::with_operator(*c, &fir.taps),
                    None => Quadrature::new(*c),
                })
                .collect(),
            slots_rot: vec![[Complex::default(); BANDS]; joc.num_channels],
            low_delay,
            clip_gain: 1.0,
            apply_clip_gain: clip_gain,
            carry,
            lag,
            reset_banks: false,
        })
    }

    /// Feeds one frame of core PCM (per coded channel) and its JOC payload.
    fn frame(&mut self, pcm: &[&[f32]], joc: Option<&Joc>) {
        self.frame_tapped(pcm, joc, None);
    }

    /// [`Self::frame`], telling `tap` what every time slot held.
    fn frame_tapped(
        &mut self,
        pcm: &[&[f32]],
        joc: Option<&Joc>,
        mut tap: Option<&mut dyn JocTap>,
    ) {
        let samples = pcm[0].len();
        let num_ts = samples / BANDS;
        // what this frame starts from: a frame without a payload keeps it
        let before = self.clip_gain;
        let mut reset = false;
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
                    if self.reset_banks {
                        self.reset_the_banks();
                    }
                    reset = true;
                    self.splices += 1;
                } else if let Some(prev) = self.last_seq
                    && j.seq_count != Joc::seq_count_after(prev)
                {
                    self.seq_gaps += 1;
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
        // Clause 6.3.3.2 gives the clip gain and never says what to do with
        // it. Encoding the same master at two levels shows the encoder
        // divides the whole downmix, LFE included, by it: the coded core came
        // out at exactly `scale / joc_clipgain` in every frame. The object
        // program is therefore restored by multiplying it back, while the
        // backwards-compatible core stays as coded. It is multiplied back
        // where the Dolby decoder has it: into the downmix as the core
        // decodes it, before the objects are made of it, and on the way from
        // the gain of the frame before over the first block of the frame.
        // See `docs/joc.md`.
        let of_the_frame = self.clip_gain;
        let gain = |at: usize| clip_gain_at(before, of_the_frame, at);
        // objects: analysis -> matrix -> synthesis, per time slot, and from
        // there onto the queue of the element the object is written as
        let mut chunk = [0.0f64; BANDS];
        let mut out = [0.0f64; BANDS];
        for ts in 0..num_ts {
            for (k, &coded) in self.joc_inputs.iter().enumerate() {
                for (i, c) in chunk.iter_mut().enumerate() {
                    let at = ts * BANDS + i;
                    *c = f64::from(pcm[coded][at]) * gain(at);
                }
                self.analysis[k].step(&chunk, &mut self.slots_in[k]);
            }
            for (k, q) in self.quad.iter_mut().enumerate() {
                q.step(&self.slots_in[k], &mut self.slots_rot[k]);
            }
            self.joc
                .reconstruct(ts, &self.slots_rot, &mut self.slots_out);
            if let Some(tap) = tap.as_deref_mut() {
                tap.slot(&TapSlot {
                    frame: self.frames,
                    ts,
                    seq_count: joc.map(|j| j.seq_count),
                    reset,
                    flush: false,
                    downmix: &self.slots_rot,
                    objects: &self.slots_out,
                    decoder: &self.joc,
                    clip_gain_before: before,
                    clip_gain: of_the_frame,
                });
            }
            for (obj, syn) in self.synthesis.iter_mut().enumerate() {
                syn.step(&self.slots_out[obj], &mut out);
                if let Some(e) = self.object_element[obj] {
                    queue_up(
                        &mut self.queues[e],
                        &mut self.to_drop[e],
                        out.iter().copied(),
                    );
                }
            }
        }
        for (e, source) in self.sources.iter().enumerate() {
            if let Source::Core(ch) = *source {
                queue_up(
                    &mut self.queues[e],
                    &mut self.to_drop[e],
                    pcm[ch]
                        .iter()
                        .enumerate()
                        .map(|(at, &v)| f64::from(v) * gain(at)),
                );
            }
        }
        self.frames += 1;
    }

    /// Clears every bank: the analysis and quadrature of every downmix
    /// channel and the synthesis of every object.
    fn reset_the_banks(&mut self) {
        self.analysis.iter_mut().for_each(Analysis::reset);
        self.quad.iter_mut().for_each(Quadrature::reset);
        self.synthesis.iter_mut().for_each(Synthesis::reset);
    }

    /// Runs zeros through the banks so the objects catch up with the core.
    fn flush(&mut self) {
        self.flush_tapped(None);
    }

    /// [`Self::flush`], telling `tap` what every time slot held.
    fn flush_tapped(&mut self, mut tap: Option<&mut dyn JocTap>) {
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
            if let Some(tap) = tap.as_deref_mut() {
                tap.slot(&TapSlot {
                    frame: self.frames,
                    ts,
                    seq_count: None,
                    reset: false,
                    flush: true,
                    downmix: &self.slots_rot,
                    objects: &self.slots_out,
                    decoder: &self.joc,
                    clip_gain_before: self.clip_gain,
                    clip_gain: self.clip_gain,
                });
            }
            for (obj, syn) in self.synthesis.iter_mut().enumerate() {
                syn.step(&self.slots_out[obj], &mut out);
                if let Some(e) = self.object_element[obj] {
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

    /// Takes `n` rows of the output into `rows`, and says how many elements
    /// a row has.
    fn take_rows(&mut self, n: usize, rows: &mut Vec<i32>) -> usize {
        rows_off(&mut self.queues, n, rows);
        self.queues.len()
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
    /// The skip fields held no EMDF container that opens, so whatever metadata
    /// the frame carried is lost; `verify` counts the same frame.
    container_failed: bool,
}

/// Extracts the OAMD and JOC payloads of one substream's skip fields.
///
/// Which substream is [`ProgramFrame::metadata_part`]: the last dependent one
/// when the programme has any, else the independent one (TS 103 420 clause
/// 8.2).
fn frame_payloads(
    skip_fields: &[Vec<u8>],
    sparse: SparseReading,
    declares_emdf: bool,
) -> FramePayloads {
    let mut out = FramePayloads::default();
    let total: usize = skip_fields.iter().map(Vec::len).sum();
    if total == 0 {
        // The substream declares a JOC extension, so this frame carried a
        // container and it is gone: the matrices of the frame before hold, and
        // the delivery says so. Returning silently was R7F1.
        out.container_failed = declares_emdf;
        return out;
    }
    let data = container::joined(skip_fields);
    let mut opened = false;
    // a sync word that opens nothing is no container, as `verify` reads it
    for (c, _) in container::candidates(&data).filter_map(|(_, candidate)| candidate.ok()) {
        opened = true;
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
    }
    out.container_failed = !opened;
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
    /// Frames whose skip fields held no EMDF container that opens; the matrices
    /// of the previous frame were held in their place.
    container_errors: u64,
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
            self.container_errors,
            "frames whose EMDF container did not open, matrices held from the previous frame",
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
        f.note(
            stats.duplicate_substream_frames,
            "substream frames repeated within one group",
        );
        f.note(
            stats.stalled_substreams,
            "substreams flushed after they stopped supplying frames",
        );
        f.note(
            stats.missing_substream_frames,
            "substream frames that never came out of their decoder",
        );
        f.first_problem(self.first_error.as_deref().or(stats.first_error.as_deref()));
        f
    }
}

/// The programme of a stream does not carry every channel its JOC downmix
/// configuration needs (TS 103 420 table 47).
///
/// Raised by the object decode in place of the bare message it used to give,
/// so that what is missing is named in the words of table 47 and a caller can
/// tell this refusal from any other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingDownmix {
    /// `joc_dmx_config_idx`.
    pub dmx_config: u8,
    /// The table 47 names of the downmix channels no programme channel feeds,
    /// in table order (`"Lb"`, `"Tfl"`, ...).
    pub missing: Vec<&'static str>,
    /// The programme channels as coded, by their TS 102 366 names.
    pub channels: Vec<&'static str>,
}

impl fmt::Display for MissingDownmix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "JOC downmix configuration {} needs {}, which the programme channels {:?} do not carry",
            self.dmx_config,
            self.missing.join(", "),
            self.channels
        )
    }
}

impl std::error::Error for MissingDownmix {}

/// The JOC objects of an E-AC-3 stream as samples, decoded the way the object
/// output decodes them but held in memory and without Object Audio Metadata:
/// every object the payload carries, in payload order, and the LFE beside
/// them. What the tests of the downmix configurations read; it writes nothing
/// and changes nothing the DAMF and ADM outputs write.
#[derive(Debug, Clone)]
pub struct JocPcm {
    /// Sample rate.
    pub rate: u32,
    /// `joc_dmx_config_idx` of the first payload.
    pub dmx_config: u8,
    /// The programme channels as coded.
    pub channels: Vec<ChannelLoc>,
    /// The programme channel each JOC downmix input is taken from, in the
    /// order of table 47.
    pub inputs: Vec<usize>,
    /// The core decode of every programme channel, the first
    /// [`DECODER_DELAY`] samples dropped and no clip gain applied: what the
    /// objects are made of.
    pub core: Vec<Vec<f64>>,
    /// Every JOC object, aligned with `core` sample for sample by the delays
    /// the decode drops (`object_delay`).
    pub objects: Vec<Vec<f64>>,
    /// The LFE as the object programme carries it (clip gain applied), when
    /// the core has one.
    pub lfe: Option<Vec<f64>>,
    /// Samples dropped at the head of every object: [`DECODER_DELAY`], the
    /// analysis and synthesis banks and the hold of the low-band operator.
    pub object_delay: usize,
    /// Frame groups decoded.
    pub frames: u64,
    /// Frames whose sequence counter said the stream was spliced.
    pub splices: u64,
}

/// Decodes the JOC objects of an E-AC-3 stream into memory with the pipeline
/// of [`write`], taking every object the first payload declares and no
/// programme from metadata.
///
/// # Errors
/// What [`write`] refuses, and [`MissingDownmix`] (as the error's source)
/// when the programme does not carry the downmix.
pub fn joc_pcm(source: &crate::source::Source, opts: &Options) -> Result<JocPcm> {
    joc_pcm_with(source, opts, TapOptions::default(), None)
}

/// The body of [`joc_pcm`] and [`joc_pcm_tapped`].
fn joc_pcm_with(
    source: &crate::source::Source,
    opts: &Options,
    measure: TapOptions,
    mut tap: Option<&mut dyn JocTap>,
) -> Result<JocPcm> {
    let mut decoder = ProgramDecoder::new(opts.core);
    let mut pipeline: Option<Pipeline> = None;
    let mut channels: Vec<ChannelLoc> = Vec::new();
    let mut core: Vec<Vec<f64>> = Vec::new();
    let mut dmx_config = 0u8;
    let mut lfe_element = false;
    let mut rate = 48_000u32;
    let mut core_samples = 0usize;

    let mut handle = |frame: &ProgramFrame| -> Result<()> {
        rate = frame.core().header.sample_rate;
        let payloads = frame_payloads(
            &frame.metadata_part().decoded.skip_fields,
            if opts.sparse_as_printed {
                SparseReading::AsPrinted
            } else {
                SparseReading::Measured
            },
            false,
        );
        if pipeline.is_none() {
            let Some(header) = payloads
                .joc
                .as_ref()
                .map(Joc::header)
                .or(payloads.joc_reserved)
            else {
                bail!("the first frame carries no JOC payload");
            };
            channels.clone_from(&frame.layout.channels);
            lfe_element = channels.contains(&ChannelLoc::Lfe);
            // every object the payload declares, and the LFE as a bed
            let program = Program {
                beds: if lfe_element {
                    vec![vec![BedChannel::LFE]]
                } else {
                    Vec::new()
                },
                isf_index: None,
                isf_objects: 0,
                dynamic_objects: header.num_objects,
            };
            pipeline = Some(Pipeline::new(
                &header,
                &program,
                &channels,
                opts.clip_gain,
                opts.flat_quadrature,
                if opts.steep_as_printed {
                    SteepReading::AsPrinted
                } else {
                    SteepReading::Measured
                },
                &opts.joc,
                &mut io::sink(),
            )?);
            dmx_config = header.dmx_config;
            core = vec![Vec::new(); channels.len()];
            let pl = pipeline.as_mut().expect("created above");
            pl.reset_banks = measure.reset_banks;
            if let Some(tap) = tap.as_mut() {
                tap.start(&TapStart {
                    rate,
                    dmx_config,
                    channels: header.num_channels,
                    objects: header.num_objects,
                    carry: pl.carry.clone(),
                    lag: pl.lag,
                    object_delay: DECODER_DELAY + DELAY + pl.low_delay,
                });
            }
        }
        let pl = pipeline.as_mut().expect("created above");
        let pcm: Vec<&[f32]> = (0..frame.channels()).map(|i| frame.channel(i)).collect();
        for (to, from) in core.iter_mut().zip(&pcm) {
            to.extend(from.iter().map(|&v| f64::from(v)));
        }
        pl.frame_tapped(
            &pcm,
            payloads.joc.as_ref(),
            tap.as_mut().map(|t| &mut **t as &mut dyn JocTap),
        );
        core_samples += frame.samples();
        Ok(())
    };
    for_each_frame(source, |_offset, bytes, header| {
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
    let _ = &mut handle;
    let Some(mut pl) = pipeline else {
        bail!("no decodable frames");
    };
    pl.flush_tapped(tap);
    // as long as the core less the decoder delay, as `write` makes it
    let target = core_samples.saturating_sub(DECODER_DELAY);
    let mut elements: Vec<Vec<f64>> = pl
        .queues
        .iter_mut()
        .map(|q| {
            let mut v: Vec<f64> = q.drain(..).collect();
            v.resize(target, 0.0);
            v
        })
        .collect();
    let lfe = lfe_element.then(|| elements.remove(0));
    for c in &mut core {
        c.drain(..DECODER_DELAY.min(c.len()));
    }
    Ok(JocPcm {
        rate,
        dmx_config,
        channels,
        inputs: pl.joc_inputs.clone(),
        core,
        objects: elements,
        lfe,
        object_delay: DECODER_DELAY + DELAY + pl.low_delay,
        frames: pl.frames,
        splices: pl.splices,
    })
}

/// [`joc_pcm`] with a [`JocTap`] reading every time slot of the object path:
/// the same decode, the same [`JocPcm`] to the byte, and the tap told what
/// each stage held (`TapSlot`). The tap is borrowed for the call and runs on
/// the decoding thread. With `TapOptions::default()` the objects are those of
/// [`joc_pcm`]; `measure` turns on what only a measurement asks for.
///
/// # Errors
/// What [`joc_pcm`] refuses.
pub fn joc_pcm_tapped(
    source: &crate::source::Source,
    opts: &Options,
    measure: TapOptions,
    tap: &mut dyn JocTap,
) -> Result<JocPcm> {
    joc_pcm_with(source, opts, measure, Some(tap))
}

/// What a decode of the objects of an E-AC-3 JOC stream wrote.
#[derive(Debug)]
pub struct Written {
    opts: Options,
    rate: u32,
    sink: crate::damf::SinkSummary,
    elapsed: f64,
    payloads: u64,
    restatements: u64,
    out_of_order: u64,
    tally: Tally,
    frames_without_joc: u64,
    frames_without_oamd: u64,
    splices: u64,
    seq_gaps: u64,
    frames: u64,
    rows_written: u64,
    losses: LossLedger,
    stats: oadec_eac3::ProgramStats,
}

impl Written {
    fn target(&self) -> &'static str {
        if self.opts.adm { "adm" } else { "damf" }
    }

    /// Says on stderr what was decoded and written, and what the output
    /// could not carry.
    pub fn print_summary(&self, err: &mut dyn Write) -> io::Result<()> {
        let (summary, tally, rate, elapsed) = (&self.sink, &self.tally, self.rate, self.elapsed);
        writeln!(
            err,
            "{}: {} frames x {} channels ({}) in {}{}",
            if self.opts.adm { "ADM BWF" } else { "DAMF" },
            summary.frames,
            summary.channels,
            format_duration(summary.frames as f64 / f64::from(rate.max(1))),
            summary
                .paths
                .last()
                .map_or_else(String::new, |p| p.display().to_string()),
            summary.note
        )?;
        crate::damf::note_non_profile(&self.opts, err)?;
        writeln!(
            err,
            "metadata: {} payloads, {} events ({} restating payloads, {} out-of-order events), {} payload errors; {} frames without JOC, {} without OAMD",
            self.payloads,
            summary.events,
            self.restatements,
            self.out_of_order,
            tally.payload_errors,
            self.frames_without_joc,
            self.frames_without_oamd
        )?;
        if self.splices > 0 || self.seq_gaps > 0 {
            writeln!(
                err,
                "splices: {} frames restarted the sequence counter, {} did not follow the previous one",
                self.splices, self.seq_gaps
            )?;
        }
        writeln!(
            err,
            "core: {} frames decoded, {} decode errors, {:.2} s ({:.0}x realtime)",
            self.frames,
            tally.decode_errors,
            elapsed,
            if elapsed > 0.0 {
                self.rows_written as f64 / f64::from(rate) / elapsed
            } else {
                0.0
            }
        )?;
        if let Some(group) = tally.first_tail {
            writeln!(
                err,
                "out of spec: {} frames ending inside the frame tail, the first in group {group}; decoded as FFmpeg and Dolby decode them, which is not an integrity fault (`oadec verify` reports it)",
                tally.tail_overruns
            )?;
        }
        crate::damf::print_losses(self.target(), &self.losses, err)?;
        Ok(())
    }

    /// Writes the loss ledger as JSON where the options ask for it.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn write_loss_report(&self) -> Result<()> {
        crate::damf::write_loss_report(
            self.target(),
            &self.losses,
            self.opts.loss_report.as_deref(),
            &self.opts.joc.json(),
        )
    }

    /// Everything that was found wrong or lost.
    #[must_use]
    pub fn findings(&self) -> Findings {
        self.tally.findings(&self.stats, &self.losses)
    }
}

/// Runs the object output for an E-AC-3 JOC stream; `base` is the output path
/// without extension.
pub fn run(
    source: &crate::source::Source,
    base: &Path,
    opts: &Options,
    err: &mut dyn Write,
) -> Result<Verdict> {
    let written = write(source, base, opts, err)?;
    written.print_summary(err)?;
    written.write_loss_report()?;
    Ok(written.findings().report(err)?)
}

/// Decodes the objects of an E-AC-3 JOC stream into a DAMF set or an ADM BWF
/// file at `base`, the output path without extension, and prints nothing.
/// What the decode has to say while it runs goes to `remarks`, a line at a
/// time.
pub fn write(
    source: &crate::source::Source,
    base: &Path,
    opts: &Options,
    remarks: &mut dyn Write,
) -> Result<Written> {
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
        writeln!(remarks, "{line}")?;
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
    // the rows of a frame, in a buffer every frame of the delivery uses
    let mut rows: Vec<i32> = Vec::new();

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
            container_failed,
        } = frame_payloads(
            &frame.metadata_part().decoded.skip_fields,
            if opts.sparse_as_printed {
                SparseReading::AsPrinted
            } else {
                SparseReading::Measured
            },
            // silence in the place of a frame that would not decode has no
            // container to lose: the frame is counted as what it is, once,
            // and the matrices of the frame before hold over it
            !frame.metadata_part().decoded.concealed
                && matches!(
                    frame.metadata_part().decoded.bsi.joc_extension(),
                    Some((true, _))
                ),
        );
        tally.payload_errors += errors;
        tally.joc_size_mismatches += joc_size_mismatches;
        if container_failed {
            tally.container_errors += 1;
            if tally.first_error.is_none() {
                tally.first_error = Some(format!(
                    "frame {frame_index}: no EMDF container in the skip fields opened; matrices held from the previous frame"
                ));
            }
        }
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
                remarks,
            )?);
            if opts.warp == Some(Warp::Stream) && declared_warp(o).is_none() {
                note_no_warp_mode(
                    "the first payload of this stream declares no warp mode",
                    remarks,
                )?;
            }
            sink = Some(open_sink(
                dir,
                &name,
                &p,
                rate,
                opts,
                declared_warp(o),
                &mut timeline,
            )?);
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
            writeln!(
                remarks,
                "program: {} bed channels, {} dynamic objects, {} JOC objects over {} downmix channels (config {}), {clip}",
                p.bed_channels().len(),
                p.dynamic_objects,
                header.num_objects,
                header.num_channels,
                header.dmx_config,
            )?;
            program = Some(p);
        }
        let pl = pipeline.as_mut().expect("created above");
        let w = sink.as_mut().expect("created above");
        // metadata: event times count from the first frame's first sample, less the decoder delay
        for (o, smploffst) in &oamds {
            if let Some(p) = &program
                && Program::from_oamd(o) != *p
            {
                writeln!(
                    remarks,
                    "warning: frame {frame_index}: the program changed; keeping the first"
                )?;
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
            let elements = pl.take_rows(n, &mut rows);
            w.write_frames(rows.chunks_exact(elements), elements)?;
            rows_written += n as u64;
        }
        Ok(())
    };

    let (_, sync_errors, skipped) = for_each_frame(source, |_offset, bytes, header| {
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
        let mut rows = Vec::new();
        let elements = pl.take_rows(remaining, &mut rows);
        sink.write_frames(rows.chunks_exact(elements), elements)?;
        rows_written += remaining as u64;
    }
    let summary = sink.finish(timeline.events)?;
    let mut losses = timeline.losses.clone();
    losses.merge(&summary.losses);
    Ok(Written {
        opts: opts.clone(),
        rate,
        sink: summary,
        elapsed: started.elapsed().as_secs_f64(),
        payloads: timeline.payloads,
        restatements: timeline.restatements,
        out_of_order: timeline.out_of_order,
        tally,
        frames_without_joc,
        frames_without_oamd,
        splices: pl.splices,
        seq_gaps: pl.seq_gaps,
        frames: pl.frames,
        rows_written,
        losses,
        stats,
    })
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

    /// The first samples of an element are the delay of the decoder and are
    /// dropped, however many calls they come in; what follows is queued in
    /// the order it came.
    #[test]
    fn the_samples_of_an_element_are_queued_after_the_delay_is_dropped() {
        let mut queue = VecDeque::new();
        let mut to_drop = 5;
        // fewer than the delay: all dropped, and the delay is that much less
        queue_up(&mut queue, &mut to_drop, [1.0, 2.0, 3.0].into_iter());
        assert_eq!((queue.len(), to_drop), (0, 2));
        // the rest of the delay, and the first samples that are kept
        queue_up(&mut queue, &mut to_drop, [4.0, 5.0, 6.0, 7.0].into_iter());
        assert_eq!((Vec::from(queue.clone()), to_drop), (vec![6.0, 7.0], 0));
        // nothing is dropped after that
        queue_up(&mut queue, &mut to_drop, [8.0].into_iter());
        assert_eq!(
            (Vec::from(queue.clone()), to_drop),
            (vec![6.0, 7.0, 8.0], 0)
        );
        // and nothing queued is nothing changed
        queue_up(&mut queue, &mut to_drop, std::iter::empty());
        assert_eq!(queue.len(), 3);
    }

    /// Rows come off the queues a sample of every element in turn, as 24-bit
    /// values, rounded and clipped. A queue that has run short gives silence,
    /// and what was not taken stays for the next call. The buffer is the rows
    /// it was last asked for, whatever it held.
    #[test]
    fn rows_come_off_the_queues_a_sample_of_every_element_in_turn() {
        let mut queues = vec![
            VecDeque::from(vec![0.25, 0.5, -1.0, 2.0]),
            VecDeque::from(vec![-0.25, 1.0 / 16_777_216.0]),
            VecDeque::from(vec![0.0, 0.0, 0.0, 0.75, 0.125]),
        ];
        let mut rows = vec![7; 40];
        rows_off(&mut queues, 3, &mut rows);
        assert_eq!(
            rows,
            [
                2_097_152, -2_097_152, 0, // the first sample of each
                4_194_304, 1, 0, // half a step rounds away from zero
                -8_388_608, 0, 0, // the second queue has run short
            ]
        );
        assert_eq!(Vec::from(queues[0].clone()), [2.0]);
        assert!(queues[1].is_empty());
        assert_eq!(Vec::from(queues[2].clone()), [0.75, 0.125]);
        // a value beyond full scale is clipped, and fewer rows are fewer values
        rows_off(&mut queues, 1, &mut rows);
        assert_eq!(rows, [8_388_607, 0, 6_291_456]);
        // no rows are no values
        rows_off(&mut queues, 0, &mut rows);
        assert!(rows.is_empty());
        assert_eq!(Vec::from(queues[2].clone()), [0.125]);
    }

    /// An object is written as the element the program gives it: the beds
    /// that are not an LFE first, then the objects, in their order, with an
    /// LFE taken from the core in between. An object the program does not
    /// use is written nowhere.
    #[test]
    fn an_object_is_written_as_the_element_the_program_gives_it() {
        let sources = [
            Source::Object(0),
            Source::Object(1),
            Source::Core(5),
            Source::Object(2),
            Source::Object(3),
        ];
        assert_eq!(
            object_elements(&sources, 6),
            [Some(0), Some(1), Some(3), Some(4), None, None]
        );
        assert_eq!(object_elements(&sources[..0], 2), [None, None]);
        assert!(object_elements(&sources, 0).is_empty());
    }

    /// The first `n` frame groups of the committed E-AC-3 fixture, decoded.
    fn fixture_groups(n: usize) -> Vec<ProgramFrame> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../oadec-cli/tests/fixtures/authored-scene.ec3"
        );
        let source = crate::source::Source::file(Path::new(path));
        let mut decoder = ProgramDecoder::new(oadec_eac3::Options::default());
        let mut groups = Vec::new();
        for_each_frame(&source, |_, bytes, header| {
            decoder.push(bytes, header)?;
            while let Some(group) = decoder.pop() {
                if groups.len() < n {
                    groups.push(group);
                }
            }
            Ok(())
        })
        .expect("the fixture");
        groups
    }

    /// From the gain of the frame before to the gain of the frame, a 256th of
    /// the way a sample, and there from the 256th sample on. Read off the
    /// output of Dolby's decoder in steps of 16 samples
    /// (`docs/audit/evidence/engine-round-2026-11`).
    #[test]
    fn the_clip_gain_goes_to_its_new_value_in_a_straight_line_over_the_first_block() {
        assert_eq!(CLIP_GAIN_RAMP, 256);
        for (before, after) in [(1.0, 1.5), (2.375, 1.0), (1.031_25, 1.062_5)] {
            let step = (after - before) / 256.0;
            assert_eq!(clip_gain_at(before, after, 0), before);
            assert_eq!(clip_gain_at(before, after, 1), before + step);
            assert_eq!(clip_gain_at(before, after, 64), before + 64.0 * step);
            assert_eq!(clip_gain_at(before, after, 255), before + 255.0 * step);
            for at in [256, 257, 1535, 100_000] {
                assert_eq!(clip_gain_at(before, after, at), after, "sample {at}");
            }
        }
        // a gain that does not change is that gain, to the last bit
        for at in 0..300 {
            assert_eq!(clip_gain_at(1.1, 1.1, at), 1.1, "sample {at}");
            assert_eq!(clip_gain_at(1.0, 1.0, at), 1.0, "sample {at}");
        }
    }

    /// The clip gain a JOC payload carries is what the encoder divided the
    /// downmix by, the LFE with it. It is put back where Dolby's decoder puts
    /// it back: on the downmix as the core decodes it, before the objects are
    /// made of it, going from the gain of the frame before to the gain of the
    /// frame over the first 256 samples the frame decodes to. So the LFE is
    /// the LFE times that gain sample for sample, and the objects are the
    /// objects of the same stream without a gain whose downmix was that much
    /// louder. Switched off, the gain of the payload changes nothing.
    #[test]
    fn the_clip_gain_of_a_payload_is_put_back_into_the_downmix_the_objects_are_made_of() {
        let groups = fixture_groups(6);
        let payloads = |group: &ProgramFrame| {
            frame_payloads(
                &group.metadata_part().decoded.skip_fields,
                SparseReading::Measured,
                true,
            )
        };
        let first = payloads(&groups[0]);
        let header = first.joc.as_ref().expect("a JOC payload").header();
        let program = Program::from_oamd(&first.oamd[0].0);
        let pipeline = |clip_gain: bool| {
            Pipeline::new(
                &header,
                &program,
                &groups[0].layout.channels,
                clip_gain,
                false,
                SteepReading::Measured,
                &JocOverrides::default(),
                &mut std::io::sink(),
            )
            .expect("a pipeline")
        };
        let (mut plain, mut gained, mut switched_off, mut louder_downmix) = (
            pipeline(true),
            pipeline(true),
            pipeline(false),
            pipeline(true),
        );
        // none for two frames, one and a half for two, two for the last two
        let gains = [1.0, 1.0, 1.5, 1.5, 2.0, 2.0];
        let frame_len = groups[0].samples();
        let mut before = 1.0;
        for (group, &gain) in groups.iter().zip(&gains) {
            let joc = payloads(group).joc.expect("a JOC payload");
            assert!((joc.clipgain - 1.0).abs() < 1e-12);
            assert_eq!(group.samples(), frame_len);
            let with_gain = Joc {
                clipgain: gain,
                ..joc.clone()
            };
            // the LFE of the fixture is silent: a ramp stands in for it
            let lfe = group
                .layout
                .channels
                .iter()
                .position(|c| *c == ChannelLoc::Lfe)
                .expect("an LFE");
            let ramp: Vec<f32> = (0..group.samples())
                .map(|i| (i as f32 - 700.0) / 4096.0)
                .collect();
            let channels: Vec<&[f32]> = (0..group.channels())
                .map(|i| {
                    if i == lfe {
                        &ramp[..]
                    } else {
                        group.channel(i)
                    }
                })
                .collect();
            // the same downmix with the gain in it already
            let louder: Vec<Vec<f32>> = channels
                .iter()
                .map(|c| {
                    c.iter()
                        .enumerate()
                        .map(|(i, &v)| (f64::from(v) * clip_gain_at(before, gain, i)) as f32)
                        .collect()
                })
                .collect();
            let louder: Vec<&[f32]> = louder.iter().map(Vec::as_slice).collect();
            plain.frame(&channels, Some(&joc));
            gained.frame(&channels, Some(&with_gain));
            switched_off.frame(&channels, Some(&with_gain));
            louder_downmix.frame(&louder, Some(&joc));
            before = gain;
        }
        // an object and a channel of the core both carry signal
        let mut loud = [false, false];
        for (e, (with, without)) in gained.queues.iter().zip(&plain.queues).enumerate() {
            assert_eq!(with.len(), without.len(), "element {e}");
            assert!(with.len() > 4000, "element {e}: {} samples", with.len());
            let of_the_core = matches!(plain.sources[e], Source::Core(_));
            if of_the_core {
                // sample n of the queue is sample n + 256 of what the core decoded
                for (n, (a, b)) in with.iter().zip(without).enumerate() {
                    let at = n + DECODER_DELAY;
                    let (frame, i) = (at / frame_len, at % frame_len);
                    let before = if frame == 0 { 1.0 } else { gains[frame - 1] };
                    assert_eq!(
                        *a,
                        *b * clip_gain_at(before, gains[frame], i),
                        "element {e}, sample {n}"
                    );
                }
            } else {
                // the downmix given to the other pipeline was rounded to 32 bits
                let made_louder = &louder_downmix.queues[e];
                let peak = made_louder.iter().fold(1e-3f64, |m, v| m.max(v.abs()));
                for (n, (a, b)) in with.iter().zip(made_louder).enumerate() {
                    assert!(
                        (a - b).abs() <= 1e-6 * peak,
                        "element {e}, sample {n}: {a} against {b}"
                    );
                }
                // where the gain has been two for longer than the banks
                // remember, an object is twice what it is without it, and
                // twice is exact
                let settled = with.len() - 128;
                for (n, (a, b)) in with.iter().zip(without).enumerate().skip(settled) {
                    assert_eq!(*a, *b * 2.0, "element {e}, sample {n}");
                }
            }
            loud[usize::from(of_the_core)] |= without.iter().any(|v| *v != 0.0);
        }
        assert_eq!(loud, [true, true]);
        assert_eq!(switched_off.queues, plain.queues);
    }

    /// A frame that comes without a JOC payload is given the gain of the frame
    /// before, and does not go back to none on the way.
    #[test]
    fn a_frame_without_a_payload_keeps_the_clip_gain_of_the_frame_before() {
        let groups = fixture_groups(3);
        let payloads = |group: &ProgramFrame| {
            frame_payloads(
                &group.metadata_part().decoded.skip_fields,
                SparseReading::Measured,
                true,
            )
        };
        let first = payloads(&groups[0]);
        let header = first.joc.as_ref().expect("a JOC payload").header();
        let program = Program::from_oamd(&first.oamd[0].0);
        let mut pipeline = Pipeline::new(
            &header,
            &program,
            &groups[0].layout.channels,
            true,
            false,
            SteepReading::Measured,
            &JocOverrides::default(),
            &mut std::io::sink(),
        )
        .expect("a pipeline");
        let lfe = groups[0]
            .layout
            .channels
            .iter()
            .position(|c| *c == ChannelLoc::Lfe)
            .expect("an LFE");
        let frame_len = groups[0].samples();
        let level = vec![0.25f32; frame_len];
        for (n, group) in groups.iter().enumerate() {
            let joc = payloads(group).joc.expect("a JOC payload");
            let with_gain = Joc {
                clipgain: 1.5,
                ..joc
            };
            let channels: Vec<&[f32]> = (0..group.channels())
                .map(|i| {
                    if i == lfe {
                        &level[..]
                    } else {
                        group.channel(i)
                    }
                })
                .collect();
            // the first frame brings the gain, the two after bring nothing
            pipeline.frame(&channels, (n == 0).then_some(&with_gain));
        }
        let e = pipeline
            .sources
            .iter()
            .position(|s| matches!(s, Source::Core(_)))
            .expect("the LFE");
        let queue = &pipeline.queues[e];
        assert_eq!(queue.len(), 3 * frame_len - DECODER_DELAY);
        // the first 256 samples of the first frame are the delay of the decoder
        // and the gain is there by the time they are past
        assert!(
            queue.iter().all(|v| *v == 0.25 * 1.5),
            "{:?}",
            queue.iter().take(4).collect::<Vec<_>>()
        );
    }

    /// What the banks still hold when the last frame is in comes out with
    /// the gain of that frame like everything before it: the gain went into
    /// the downmix, and the tail is made of the downmix. It came out without
    /// the gain while the gain was put on what left the decoder frame by
    /// frame, which the tail does not.
    #[test]
    fn what_the_banks_give_out_after_the_last_frame_has_its_clip_gain() {
        let groups = fixture_groups(4);
        let payloads = |group: &ProgramFrame| {
            frame_payloads(
                &group.metadata_part().decoded.skip_fields,
                SparseReading::Measured,
                true,
            )
        };
        let first = payloads(&groups[0]);
        let header = first.joc.as_ref().expect("a JOC payload").header();
        let program = Program::from_oamd(&first.oamd[0].0);
        let pipeline = || {
            Pipeline::new(
                &header,
                &program,
                &groups[0].layout.channels,
                true,
                false,
                SteepReading::Measured,
                &JocOverrides::default(),
                &mut std::io::sink(),
            )
            .expect("a pipeline")
        };
        let (mut plain, mut gained) = (pipeline(), pipeline());
        for group in &groups {
            let joc = payloads(group).joc.expect("a JOC payload");
            let doubled = Joc {
                clipgain: 2.0,
                ..joc.clone()
            };
            let channels: Vec<&[f32]> = (0..group.channels()).map(|i| group.channel(i)).collect();
            plain.frame(&channels, Some(&joc));
            gained.frame(&channels, Some(&doubled));
        }
        let before_the_tail: Vec<usize> = plain.queues.iter().map(VecDeque::len).collect();
        plain.flush();
        gained.flush();
        let mut objects = 0;
        for (e, (with, without)) in gained.queues.iter().zip(&plain.queues).enumerate() {
            if matches!(plain.sources[e], Source::Core(_)) {
                continue;
            }
            assert!(without.len() > before_the_tail[e] + 1000, "element {e}");
            let tail = without.iter().skip(before_the_tail[e]);
            if tail.clone().all(|v| *v == 0.0) {
                continue;
            }
            objects += 1;
            // the gain is two from the 256th sample of the first frame, which is
            // long past: twice, and twice is exact
            for (n, (a, b)) in with
                .iter()
                .zip(without)
                .enumerate()
                .skip(before_the_tail[e])
            {
                assert_eq!(*a, *b * 2.0, "element {e}, sample {n}");
            }
        }
        assert!(objects >= 2, "only {objects} objects had a tail to show");
    }

    /// The object path loses a frame's metadata when the substream declares the
    /// JOC extension and the frame carries no skip field at all: there is no
    /// container to read, so the matrices of the frame before hold. It returned
    /// an empty payload set without a word, and the delivery stayed clean
    /// (R7F1).
    #[test]
    fn a_frame_without_skip_fields_is_a_lost_container_where_joc_is_declared() {
        let lost = frame_payloads(&[], SparseReading::Measured, true);
        assert!(lost.container_failed, "the frame lost its container");
        assert!(lost.joc.is_none() && lost.oamd.is_empty());

        let plain = frame_payloads(&[], SparseReading::Measured, false);
        assert!(
            !plain.container_failed,
            "a frame of a stream that carries no EMDF has lost nothing"
        );
    }

    /// A JOC payload that parses but declares a byte its syntax never reaches
    /// went straight into the reconstruction, and the object output exited
    /// clean where `verify` and the PCM path exit 7. The payload is still used;
    /// the mismatch is counted.
    #[test]
    fn the_object_path_counts_a_joc_payload_whose_declared_size_is_wrong() {
        let exact = joc_payload(0);
        let payloads = frame_payloads(&[emdf_with_joc(&exact)], SparseReading::Measured, true);
        assert!(payloads.joc.is_some(), "the payload parses");
        assert_eq!((payloads.errors, payloads.joc_size_mismatches), (0, 0));

        let mut long = exact;
        long.push(0);
        let payloads = frame_payloads(&[emdf_with_joc(&long)], SparseReading::Measured, true);
        assert!(
            payloads.joc.is_some(),
            "a trailing byte does not stop the parse"
        );
        assert_eq!(
            payloads.joc_size_mismatches, 1,
            "the trailing byte went unnoticed"
        );
    }

    /// A frame whose skip fields hold no EMDF container that opens has lost its
    /// metadata. `verify` counts such a frame; the object path held the
    /// matrices of the frame before without a word and exited clean.
    #[test]
    fn a_frame_whose_container_does_not_open_is_counted() {
        let good = emdf_with_joc(&joc_payload(0));
        assert!(
            !frame_payloads(std::slice::from_ref(&good), SparseReading::Measured, true)
                .container_failed
        );
        let mut bad = good;
        // the declared length, one byte short of the syntax
        bad[3] = bad[3].wrapping_sub(1);
        let payloads = frame_payloads(&[bad], SparseReading::Measured, true);
        assert!(payloads.joc.is_none(), "nothing is read from it");
        assert!(payloads.container_failed, "and the frame says so");
        let tally = Tally {
            container_errors: 1,
            ..Tally::default()
        };
        let f = tally.findings(&oadec_eac3::ProgramStats::default(), &LossLedger::default());
        assert!(!f.is_clean(), "{f:?}");
    }

    /// A payload whose `joc_ext_config_idx` is reserved is not a parse error
    /// among the others: it is kept apart with its header, so the run can name
    /// it and a stream that opens with one can still start the pipeline, and
    /// its matrices are not used.
    #[test]
    fn a_reserved_joc_extension_is_kept_apart_with_its_header() {
        let payloads = frame_payloads(
            &[emdf_with_joc(&joc_payload(5))],
            SparseReading::Measured,
            true,
        );
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

    /// `--joc-low-fir` reads a tap a line and says what is wrong with anything
    /// else: a line that is not two numbers, a count without a middle, an
    /// operator shorter than the hold the alignment needs.
    #[test]
    fn a_joc_low_fir_is_read_once_with_a_typed_error() {
        let lines = |n: usize| -> String { (0..n).map(|i| format!("{i} -0.5\n")).collect() };
        let fir = JocLowFir::parse("taps.txt", &lines(LOW_TAPS)).unwrap();
        assert_eq!(fir.taps.len(), LOW_TAPS);
        assert_eq!(fir.taps[3], Complex { re: 3.0, im: -0.5 });
        assert_eq!(fir.reach(), LOW_DELAY);
        assert_eq!(
            JocLowFir::parse("f", &lines(LOW_TAPS + 2)).unwrap().reach(),
            11
        );
        // exponents, tabs and empty lines are a matter of how it was written
        let spaced = format!("\n  1e-3\t-2.5E0  \n\n{}", lines(LOW_TAPS - 1));
        let fir = JocLowFir::parse("f", &spaced).unwrap();
        assert_eq!(
            fir.taps[0],
            Complex {
                re: 0.001,
                im: -2.5
            }
        );

        for (n, why) in [(LOW_TAPS + 1, "even"), (LOW_TAPS - 2, "short"), (0, "none")] {
            assert_eq!(
                JocLowFir::parse("f", &lines(n)),
                Err(JocLowFirError::Taps(n)),
                "{why}"
            );
        }
        for bad in ["1", "1 2 3", "1 x", "nan 0", "0 inf"] {
            let text = format!("{}{bad}\n", lines(4));
            assert_eq!(
                JocLowFir::parse("f", &text),
                Err(JocLowFirError::Line {
                    line: 5,
                    text: bad.to_string()
                }),
                "{bad:?}"
            );
        }
        assert_eq!(
            JocLowFirError::Taps(20).to_string(),
            "20 taps: an operator has an odd number of them, 21 or more"
        );
        assert_eq!(
            JocLowFirError::Line {
                line: 5,
                text: "1 x".to_string()
            }
            .to_string(),
            "line 5 is not a real part and an imaginary part: \"1 x\""
        );
        let missing = JocLowFir::read("no-such-directory/taps.txt").unwrap_err();
        assert!(
            matches!(&missing, JocLowFirError::Read { file, .. } if file == "no-such-directory/taps.txt"),
            "{missing:?}"
        );
        assert!(
            missing
                .to_string()
                .starts_with("cannot read no-such-directory/taps.txt: "),
            "{missing}"
        );
    }

    /// The matrices are held back by what the operator holds the samples back
    /// by beyond the measured alignment: by nothing with the operator there
    /// is, and by a slot for every pair of taps a longer one adds.
    #[test]
    fn the_matrices_follow_the_reach_of_the_operator() {
        let none = JocOverrides::default();
        assert_eq!(none.reach(), LOW_DELAY);
        assert_eq!(none.default_lag(), 0);
        let longer = |taps: usize| JocOverrides {
            low_fir: Some(JocLowFir::new("longer", vec![Complex::default(); taps]).unwrap()),
            ..JocOverrides::default()
        };
        assert_eq!(longer(LOW_TAPS).default_lag(), 0);
        assert_eq!(longer(LOW_TAPS + 2).reach(), LOW_DELAY + 1);
        assert_eq!(longer(LOW_TAPS + 2).default_lag(), 1);
        assert_eq!(longer(37).default_lag(), 18 - MATRIX_ALIGN);
        assert_eq!(
            longer(23).line().unwrap(),
            "measurement overrides: joc-low-fir longer (23 taps) \
             (default the 21 taps measured on the Dolby decoder)"
        );
        // the default a lag is announced against is the one in force
        let both = JocOverrides {
            lag: Some(4),
            ..longer(25)
        };
        assert!(
            both.line().unwrap().contains("joc-lag 4 (default 2); "),
            "{:?}",
            both.line()
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
