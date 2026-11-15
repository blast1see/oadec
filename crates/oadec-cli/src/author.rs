//! Authoring a controlled Atmos master, so a decode can be checked against
//! something other than another decoder.
//!
//! Every result this project has for object audio is differential: oadec
//! against `truehdd`, oadec against Dolby. That answers "do two decoders agree"
//! and never "is either of them right". The way to ask the second question is
//! to author a programme whose object positions and event times are known
//! exactly, encode it with Dolby's own encoder, decode it back, and compare
//! with the numbers that went in.
//!
//! This writes the master. The scene is a JSON file and is itself the ground
//! truth: it is committed next to the measurements, and the comparison is
//! against it rather than against anything the decode produced.
//!
//! Each object carries a signal that identifies it — a tone at a distinct
//! frequency, or a train of impulses at named samples — because the encoder's
//! spatial coding is free to renumber objects, and an object that can be
//! recognised from its audio survives that.

// A command that prints as it goes and is not offered as a call.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_emdf::oamd::{BedChannel, Gain};
use oadec_spatial::damf::{DamfOptions, DamfWriter};
use oadec_spatial::program::{ElementState, Event, ObjectState, Program, WarpMode, zones_name};
use serde::{Deserialize, Serialize};

/// What an object sounds like.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Signal {
    /// A steady sine, which identifies the object by frequency. It sounds
    /// through the whole scene unless it names the sample it begins at, the
    /// sample it has ended by, or both; an end it names it comes to, and a
    /// beginning after the first sample it comes from, over [`EDGE`] samples,
    /// so that neither is a click. An element that sounds alone for a part of
    /// a scene is then known by when it sounds as well as by its pitch.
    Tone {
        hz: f64,
        dbfs: f64,
        #[serde(default)]
        from: u64,
        #[serde(default)]
        until: Option<u64>,
    },
    /// One impulse at each named sample, which identifies the object by
    /// frequency-independent timing.
    Impulses { at: Vec<u64>, dbfs: f64 },
    /// Nothing.
    Silence,
}

/// A state an object takes at a named sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEvent {
    /// Sample position, counted from the first sample of the programme.
    pub sample: u64,
    /// DAMF room coordinates: x −1 left to 1 right, y −1 back to 1 front,
    /// z 0 floor to 1 ceiling.
    pub pos: [f32; 3],
    /// Gain in decibels.
    #[serde(default)]
    pub gain_db: i8,
    /// Object size, 0 being a point source.
    #[serde(default)]
    pub size: f32,
    /// Lock to the nearest channel.
    #[serde(default)]
    pub snap: bool,
    /// The zone the object keeps to, by the name a master gives it: `all`
    /// (what a state says when it names none), `no back`, `no sides`,
    /// `center back`, `screen only` or `surround only`.
    #[serde(default)]
    pub zones: Option<String>,
}

/// One object of the scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneObject {
    /// A name for the reader; it is not carried in the master.
    pub name: String,
    pub signal: Signal,
    /// States in sample order; the first is the object's initial state.
    pub events: Vec<SceneEvent>,
}

/// A programme whose metadata is known exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    #[serde(default = "default_rate")]
    pub sample_rate: u32,
    pub duration_samples: u64,
    /// Bed channels, by DAMF name. An Atmos master needs a bed; the smallest
    /// one that real streams use is a single silent LFE, and keeping it that
    /// way leaves the encoder nothing to cluster the objects into.
    #[serde(default = "default_bed")]
    pub bed: Vec<String>,
    /// What a channel of the bed sounds like, by its name. A channel without
    /// one is silent; with one, a bed that comes back can be known by its
    /// sound, as an object can.
    #[serde(default)]
    pub bed_signals: BTreeMap<String, Signal>,
    /// The frame rate the master names: `23.976`, `24` (what a scene that
    /// names none gets), `25`, `29.97` or `30`. It is header data; the events
    /// are timed in samples.
    #[serde(default)]
    pub fps: Option<String>,
    /// The warp mode the master names: `normal`, `warping`, `prologiciix` or
    /// `loro`. A scene that names none writes none, which Dolby's encoders
    /// read as warping.
    #[serde(default)]
    pub warp_mode: Option<String>,
    pub objects: Vec<SceneObject>,
}

fn default_rate() -> u32 {
    48_000
}

fn default_bed() -> Vec<String> {
    vec!["LFE".to_string()]
}

fn bed_channel(name: &str) -> Option<BedChannel> {
    BedChannel::ALL
        .into_iter()
        .find(|c| oadec_spatial::program::damf_channel_name(*c).eq_ignore_ascii_case(name))
}

/// The zone of a name a master gives one.
fn zone(name: &str) -> Option<u8> {
    (0..=5).find(|&index| zones_name(index) == name)
}

/// The warp mode of a name a scene gives one.
fn warp_mode(name: &str) -> Option<WarpMode> {
    match name {
        "normal" => Some(WarpMode::Normal),
        "warping" => Some(WarpMode::Warping),
        "prologiciix" => Some(WarpMode::ProLogicIIx),
        "loro" => Some(WarpMode::LoRo),
        _ => None,
    }
}

fn to_i24(v: f64) -> i32 {
    (v * 8_388_607.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// The samples a tone takes to come and to go: a hundredth of a second at
/// 48 kHz.
const EDGE: u64 = 480;

/// How much of a tone there is `k` samples inside one of its edges: a raised
/// cosine, all of it from [`EDGE`] samples on.
fn edge(k: u64) -> f64 {
    if k >= EDGE {
        return 1.0;
    }
    let x = (k as f64 + 0.5) / EDGE as f64;
    (std::f64::consts::FRAC_PI_2 * x).sin().powi(2)
}

impl Signal {
    /// Where a tone that sounds for no time at all would begin and end, in a
    /// scene of `duration` samples.
    fn never_sounds(&self, duration: u64) -> Option<(u64, u64)> {
        match self {
            Self::Tone { from, until, .. } => {
                let end = until.unwrap_or(duration);
                (*from >= end).then_some((*from, end))
            }
            Self::Impulses { .. } | Self::Silence => None,
        }
    }

    fn sample(&self, n: u64, rate: u32) -> i32 {
        match self {
            Self::Silence => 0,
            Self::Tone {
                hz,
                dbfs,
                from,
                until,
            } => {
                if n < *from || until.is_some_and(|end| n >= end) {
                    return 0;
                }
                // a tone that names no beginning is at its level from the
                // first sample, and one that names no end to the last
                let coming = if *from > 0 { edge(n - from) } else { 1.0 };
                let going = until.map_or(1.0, |end| edge(end - 1 - n));
                let a = 10f64.powf(dbfs / 20.0) * coming * going;
                let t = n as f64 / f64::from(rate);
                to_i24(a * (std::f64::consts::TAU * hz * t).sin())
            }
            Self::Impulses { at, dbfs } => {
                let a = 10f64.powf(dbfs / 20.0);
                if at.contains(&n) { to_i24(a) } else { 0 }
            }
        }
    }
}

fn object_state(e: &SceneEvent) -> ObjectState {
    ObjectState {
        active: true,
        pos: e.pos,
        snap: e.snap,
        elevation: true,
        // a name that is no zone was refused before anything was written
        zones: e.zones.as_deref().and_then(zone).unwrap_or(0),
        size: [e.size; 3],
        importance: 1.0,
        gain: Gain::Db(e.gain_db),
        ramp: 0,
        trim_bypass: false,
        screen_factor: 0.0,
        depth_factor: 0.0,
    }
}

/// Writes `<base>.atmos`, `.atmos.metadata` and `.atmos.audio` from a scene.
pub fn run(scene_path: &Path, base: &Path) -> Result<()> {
    let scene: Scene = serde_json::from_reader(BufReader::new(
        File::open(scene_path).with_context(|| format!("opening {}", scene_path.display()))?,
    ))
    .with_context(|| format!("reading {}", scene_path.display()))?;

    if scene.objects.is_empty() {
        bail!("the scene has no objects");
    }
    let mut beds = Vec::new();
    for name in &scene.bed {
        beds.push(bed_channel(name).with_context(|| format!("unknown bed channel {name}"))?);
    }
    if beds.is_empty() {
        bail!("the scene has no bed; an Atmos master needs one");
    }
    for (i, o) in scene.objects.iter().enumerate() {
        if o.events.is_empty() {
            bail!("object {i} ({}) has no state", o.name);
        }
        if o.events.windows(2).any(|w| w[1].sample <= w[0].sample) {
            bail!("object {i} ({}) has events out of order", o.name);
        }
        if o.events[0].sample != 0 {
            bail!("object {i} ({}) has no state at sample 0", o.name);
        }
        for e in &o.events {
            if let Some(name) = e.zones.as_deref()
                && zone(name).is_none()
            {
                bail!(
                    "object {i} ({}) keeps to {name:?}, which is no zone of a master",
                    o.name
                );
            }
        }
    }
    let signals = scene
        .objects
        .iter()
        .map(|o| (o.name.as_str(), &o.signal))
        .chain(scene.bed_signals.iter().map(|(name, s)| (name.as_str(), s)));
    for (name, signal) in signals {
        if let Some((from, end)) = signal.never_sounds(scene.duration_samples) {
            bail!("the tone of {name} would sound from sample {from} until sample {end}");
        }
    }
    // the signal of each channel of the bed, in the order of the bed
    let mut bed_signals: Vec<Option<&Signal>> = vec![None; scene.bed.len()];
    for (name, signal) in &scene.bed_signals {
        let at = scene
            .bed
            .iter()
            .position(|channel| channel.eq_ignore_ascii_case(name))
            .with_context(|| format!("{name} has a signal and is not a channel of the bed"))?;
        bed_signals[at] = Some(signal);
    }
    let warp = match scene.warp_mode.as_deref() {
        None => None,
        Some(name) => Some(warp_mode(name).with_context(|| format!("{name:?} is no warp mode"))?),
    };

    let program = Program {
        beds: vec![beds],
        isf_index: None,
        isf_objects: 0,
        dynamic_objects: scene.objects.len(),
    };
    let dir = base
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = base
        .file_name()
        .and_then(|n| n.to_str())
        .context("output base name")?;
    std::fs::create_dir_all(dir)?;
    let mut writer = DamfWriter::create(
        dir,
        name,
        &program,
        scene.sample_rate,
        &DamfOptions {
            // The bed is written as coded: conforming it to 7.1.2 would add
            // nine silent channels the scene never asked for.
            bed_conform: false,
            fps: scene.fps.clone().unwrap_or(DamfOptions::default().fps),
            warp_mode: warp,
            ..DamfOptions::default()
        },
    )?;

    // metadata first: every element states itself at sample 0, then changes
    let bed_count = program.bed_channels().len();
    for i in 0..bed_count {
        let id = program.element_id(i).context("bed element id")?;
        writer.push_event(&Event {
            id,
            sample_pos: 0,
            state: ElementState::Bed(oadec_spatial::program::BedState {
                active: true,
                importance: 1.0,
                gain: Gain::Db(0),
                ramp: 0,
                trim_bypass: false,
            }),
            previous: None,
        })?;
    }
    let mut pending: Vec<(u64, usize, usize)> = Vec::new();
    for (o, obj) in scene.objects.iter().enumerate() {
        for (e, ev) in obj.events.iter().enumerate() {
            pending.push((ev.sample, o, e));
        }
    }
    pending.sort_unstable();
    for (sample, o, e) in pending {
        let obj = &scene.objects[o];
        let id = program
            .element_id(bed_count + o)
            .context("object element id")?;
        let previous = e
            .checked_sub(1)
            .map(|p| ElementState::Object(object_state(&obj.events[p])));
        writer.push_event(&Event {
            id,
            sample_pos: sample,
            state: ElementState::Object(object_state(&obj.events[e])),
            previous,
        })?;
    }

    // then the audio
    let elements = bed_count + scene.objects.len();
    let mut row = vec![0i32; elements];
    for n in 0..scene.duration_samples {
        row.iter_mut().for_each(|v| *v = 0);
        for (channel, signal) in bed_signals.iter().enumerate() {
            if let Some(signal) = signal {
                row[channel] = signal.sample(n, scene.sample_rate);
            }
        }
        for (o, obj) in scene.objects.iter().enumerate() {
            row[bed_count + o] = obj.signal.sample(n, scene.sample_rate);
        }
        writer.write_frame(&row)?;
    }
    let summary = writer.finish()?;
    eprintln!(
        "authored {} elements ({} bed + {} objects), {} frames, {} events -> {}.atmos",
        elements,
        bed_count,
        scene.objects.len(),
        summary.frames,
        summary.events,
        base.display()
    );
    Ok(())
}
