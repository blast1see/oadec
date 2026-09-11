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

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};
use oadec_emdf::oamd::{BedChannel, Gain};
use oadec_spatial::damf::{DamfOptions, DamfWriter};
use oadec_spatial::program::{ElementState, Event, ObjectState, Program};
use serde::{Deserialize, Serialize};

/// What an object sounds like.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Signal {
    /// A steady sine, which identifies the object by frequency.
    Tone { hz: f64, dbfs: f64 },
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

fn to_i24(v: f64) -> i32 {
    (v * 8_388_607.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32
}

impl Signal {
    fn sample(&self, n: u64, rate: u32) -> i32 {
        match self {
            Self::Silence => 0,
            Self::Tone { hz, dbfs } => {
                let a = 10f64.powf(dbfs / 20.0);
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
        snap: false,
        elevation: true,
        zones: 0,
        size: e.size,
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
    }

    let program = Program {
        beds: vec![beds],
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
