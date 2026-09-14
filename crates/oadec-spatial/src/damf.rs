//! Dolby Atmos Master Format writer: `<name>.atmos` (presentation description),
//! `<name>.atmos.metadata` (element events) and `<name>.atmos.audio` (CAF).
//!
//! There is no public specification of DAMF. The layout follows what the Dolby
//! encoder accepts: version 0.5.1 headers as the other public decoder writes
//! them (Apache-2.0, read for facts), with a full first event per element and
//! only the changed fields in later events.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use oadec_emdf::oamd::{BedChannel, Gain};

use crate::caf::CafWriter;
use crate::loss::{LossKind, LossLedger};
use crate::program::{
    BedState, ElementState, Event, FIRST_OBJECT_ID, IsfPolicy, ObjectState, Program, STANDARD_BED,
    bed_channel_id, damf_channel_name, zones_name,
};

/// DAMF version written.
pub const DAMF_VERSION: &str = "0.5.1";

/// Frame rates a `.atmos` header may carry, as the Dolby tools list them. The
/// rate is header data for picture-locked workflows; event timing is in
/// samples and does not depend on it.
pub const DAMF_FRAME_RATES: [&str; 5] = ["23.976", "24", "25", "29.97", "30"];

/// Options of the writer.
#[derive(Debug, Clone)]
pub struct DamfOptions {
    /// Write the full ten-channel 7.1.2 bed (silence in unused slots) so that the
    /// first ten audio channels are always the standard bed.
    pub bed_conform: bool,
    /// Frame rate written to the `.atmos` file.
    pub fps: String,
    /// Tool name and version written to the `.atmos` file.
    pub creation_tool: String,
    pub creation_tool_version: String,
    /// What to do with intermediate-spatial-format elements, which DAMF
    /// cannot represent: refuse (the default) or drop and count.
    pub isf: IsfPolicy,
}

impl Default for DamfOptions {
    fn default() -> Self {
        Self {
            bed_conform: true,
            fps: "24".to_string(),
            creation_tool: "oadec".to_string(),
            creation_tool_version: env!("CARGO_PKG_VERSION").to_string(),
            isf: IsfPolicy::Error,
        }
    }
}

/// Errors of the writer.
#[derive(Debug, thiserror::Error)]
pub enum DamfError {
    /// I/O.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The programme has intermediate-spatial-format objects, which DAMF
    /// cannot represent, and the options said not to drop them.
    #[error(
        "{count} intermediate-spatial-format objects ({isf_type}) cannot be represented in a Dolby Atmos master file"
    )]
    IsfNotRepresentable {
        /// ISF objects in the programme.
        count: usize,
        /// The ISF type (table 11b), or "reserved type".
        isf_type: String,
    },
    /// A frame rate the `.atmos` header cannot carry.
    #[error("frame rate {0} is not one of 23.976, 24, 25, 29.97 or 30")]
    InvalidFps(String),
}

/// Where an element's audio goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// Audio channel index.
    Channel(usize),
    /// Not representable (ISF objects).
    None,
}

/// Summary returned when the set is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DamfSummary {
    /// Frames written to the audio file.
    pub frames: u64,
    /// Events written to the metadata file.
    pub events: u64,
    /// Audio channels.
    pub channels: usize,
    /// What the set does not carry of the programme it was given.
    pub losses: LossLedger,
}

/// Streaming DAMF writer.
#[derive(Debug)]
pub struct DamfWriter {
    audio: CafWriter,
    metadata: BufWriter<File>,
    /// Audio channel of each element in stream order.
    slots: Vec<Slot>,
    channels: usize,
    events: u64,
    seen: BTreeSet<u32>,
    frame: Vec<i32>,
    paths: [PathBuf; 3],
    losses: LossLedger,
    /// Last sample position written per element: the metadata is streamed,
    /// so an earlier event cannot be moved, only declared.
    last_pos: BTreeMap<u32, u64>,
}

impl DamfWriter {
    /// Creates `<dir>/<base>.atmos{,.metadata,.audio}` for `program`.
    pub fn create(
        dir: &Path,
        base: &str,
        program: &Program,
        sample_rate: u32,
        options: &DamfOptions,
    ) -> Result<Self, DamfError> {
        if !DAMF_FRAME_RATES.contains(&options.fps.as_str()) {
            return Err(DamfError::InvalidFps(options.fps.clone()));
        }
        let mut losses = LossLedger::default();
        if program.isf_objects > 0 {
            match options.isf {
                IsfPolicy::Error => {
                    return Err(DamfError::IsfNotRepresentable {
                        count: program.isf_objects,
                        isf_type: program.isf_type().unwrap_or("reserved type").to_string(),
                    });
                }
                IsfPolicy::Drop => {
                    for k in 0..program.isf_objects {
                        losses.note(LossKind::IsfDropped, k as u32, 0);
                    }
                }
            }
        }
        // Audio layout: bed channels (conformed or as coded), then objects.
        let coded_beds = program.bed_channels();
        let mut bed_layout: Vec<(u32, BedChannel)> = Vec::new();
        if options.bed_conform {
            for ch in STANDARD_BED {
                bed_layout.push((bed_channel_id(ch), ch));
            }
            for &ch in &coded_beds {
                if !STANDARD_BED.contains(&ch) {
                    bed_layout.push((bed_channel_id(ch), ch));
                }
            }
        } else {
            for &ch in &coded_beds {
                bed_layout.push((bed_channel_id(ch), ch));
            }
        }
        let mut slots = Vec::with_capacity(program.elements());
        for (i, ch) in coded_beds.iter().enumerate() {
            let slot = bed_layout
                .iter()
                .position(|(_, c)| c == ch)
                .map_or(Slot::None, Slot::Channel);
            debug_assert!(
                matches!(slot, Slot::Channel(_)),
                "bed channel {i} has a slot"
            );
            slots.push(slot);
        }
        for _ in 0..program.isf_objects {
            slots.push(Slot::None);
        }
        for k in 0..program.dynamic_objects {
            slots.push(Slot::Channel(bed_layout.len() + k));
        }
        let channels = bed_layout.len() + program.dynamic_objects;

        let atmos_path = dir.join(format!("{base}.atmos"));
        let metadata_path = dir.join(format!("{base}.atmos.metadata"));
        let audio_path = dir.join(format!("{base}.atmos.audio"));

        let mut atmos = String::new();
        atmos.push_str(&format!("version: {DAMF_VERSION}\n"));
        atmos.push_str("presentations:\n");
        atmos.push_str("  - type: home\n");
        atmos.push_str("    simplified: false\n");
        atmos.push_str(&format!("    metadata: {base}.atmos.metadata\n"));
        atmos.push_str(&format!("    audio: {base}.atmos.audio\n"));
        atmos.push_str("    offset: 0.0\n");
        atmos.push_str(&format!("    fps: {}\n", options.fps));
        let bed_ids: Vec<String> = bed_layout.iter().map(|(id, _)| id.to_string()).collect();
        atmos.push_str(&format!(
            "    scBedConfiguration: [{}]\n",
            bed_ids.join(", ")
        ));
        atmos.push_str(&format!("    creationTool: {}\n", options.creation_tool));
        atmos.push_str(&format!(
            "    creationToolVersion: {}\n",
            options.creation_tool_version
        ));
        atmos.push_str("    bedInstances:\n");
        atmos.push_str("      - channels:\n");
        for (id, ch) in &bed_layout {
            atmos.push_str(&format!(
                "          - channel: {}
",
                damf_channel_name(*ch)
            ));
            atmos.push_str(&format!("            ID: {id}\n"));
        }
        atmos.push_str("    objects:\n");
        for k in 0..program.dynamic_objects {
            atmos.push_str(&format!("      - ID: {}\n", FIRST_OBJECT_ID + k as u32));
        }
        std::fs::write(&atmos_path, atmos)?;

        let mut metadata = BufWriter::new(File::create(&metadata_path)?);
        writeln!(metadata, "sampleRate: {sample_rate}")?;
        writeln!(metadata, "events:")?;

        let audio = CafWriter::create(&audio_path, sample_rate, channels)?;
        Ok(Self {
            audio,
            metadata,
            slots,
            channels,
            events: 0,
            seen: BTreeSet::new(),
            frame: vec![0; channels],
            paths: [atmos_path, metadata_path, audio_path],
            losses,
            last_pos: BTreeMap::new(),
        })
    }

    /// Audio channels of the set.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Paths of the three files.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf; 3] {
        &self.paths
    }

    /// Writes one frame given the element samples in stream order (bed
    /// channels, ISF objects, dynamic objects); missing elements are silence.
    pub fn write_frame(&mut self, elements: &[i32]) -> io::Result<()> {
        self.frame.iter_mut().for_each(|v| *v = 0);
        for (i, &slot) in self.slots.iter().enumerate() {
            if let (Slot::Channel(c), Some(&v)) = (slot, elements.get(i)) {
                self.frame[c] = v;
            }
        }
        self.audio.write_samples(&self.frame)
    }

    /// Writes many frames: `rows` holds the element samples of each frame in
    /// stream order, `elements` of them per row.
    pub fn write_frames<'a>(
        &mut self,
        rows: impl Iterator<Item = &'a [i32]>,
        elements: usize,
    ) -> io::Result<()> {
        let mut buf: Vec<i32> = Vec::with_capacity(160 * self.channels);
        for row in rows {
            let start = buf.len();
            buf.resize(start + self.channels, 0);
            for (i, &slot) in self.slots.iter().enumerate().take(elements) {
                if let (Slot::Channel(c), Some(&v)) = (slot, row.get(i)) {
                    buf[start + c] = v;
                }
            }
        }
        self.audio.write_samples(&buf)
    }

    /// Writes one event (full on the first occurrence of an element, changed
    /// fields only afterwards).
    pub fn push_event(&mut self, event: &Event) -> io::Result<()> {
        let first = self.seen.insert(event.id);
        if let Some(&last) = self.last_pos.get(&event.id)
            && event.sample_pos < last
        {
            self.losses
                .note(LossKind::OutOfOrderWrittenAsIs, event.id, event.sample_pos);
        }
        self.last_pos.insert(event.id, event.sample_pos);
        let mut text = String::new();
        text.push_str(&format!("  - ID: {}\n", event.id));
        text.push_str(&format!("    samplePos: {}\n", event.sample_pos));
        match (&event.state, &event.previous) {
            (ElementState::Bed(s), prev) => {
                let p = match prev {
                    Some(ElementState::Bed(p)) if !first => Some(p),
                    _ => None,
                };
                bed_fields(&mut text, s, p);
            }
            (ElementState::Object(s), prev) => {
                if s.size_axes_differ() {
                    self.losses
                        .note(LossKind::SizeAxesCollapsed, event.id, event.sample_pos);
                }
                let p = match prev {
                    Some(ElementState::Object(p)) if !first => Some(p),
                    _ => None,
                };
                object_fields(&mut text, s, p);
            }
        }
        self.metadata.write_all(text.as_bytes())?;
        self.events += 1;
        Ok(())
    }

    /// Closes the three files.
    pub fn finish(mut self) -> io::Result<DamfSummary> {
        self.metadata.flush()?;
        let frames = self.audio.finish()?;
        Ok(DamfSummary {
            frames,
            events: self.events,
            channels: self.channels,
            losses: self.losses,
        })
    }
}

fn gain_text(g: Gain) -> String {
    match g {
        Gain::Db(db) => db.to_string(),
        Gain::MinusInfinity => "-inf".to_string(),
    }
}

/// Formats a float the way DAMF files carry them (`1.0`, `0.25`, `-1`).
fn num(v: f32) -> String {
    if v == v.trunc() && v.abs() < 1e6 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

fn pos_text(p: [f32; 3]) -> String {
    let c = |v: f32| {
        if v == v.trunc() {
            format!("{}", v as i32)
        } else {
            format!("{v}")
        }
    };
    format!("[{}, {}, {}]", c(p[0]), c(p[1]), c(p[2]))
}

macro_rules! field {
    ($text:expr, $prev:expr, $name:literal, $cur:expr, $old:expr, $fmt:expr) => {
        if $prev.is_none() || $cur != $old {
            $text.push_str(&format!(concat!("    ", $name, ": {}\n"), $fmt($cur)));
        }
    };
}

fn bed_fields(text: &mut String, s: &BedState, prev: Option<&BedState>) {
    let p = prev.cloned().unwrap_or_else(|| s.clone());
    field!(text, prev, "active", s.active, p.active, |v: bool| v);
    field!(text, prev, "importance", s.importance, p.importance, num);
    field!(text, prev, "gain", s.gain, p.gain, gain_text);
    field!(text, prev, "rampLength", s.ramp, p.ramp, |v: u32| v);
    field!(
        text,
        prev,
        "trimBypass",
        s.trim_bypass,
        p.trim_bypass,
        |v: bool| v
    );
    if prev.is_none() {
        text.push_str("    headTrackMode: undefined\n");
        text.push_str("    binauralRenderMode: off\n");
    }
}

fn object_fields(text: &mut String, s: &ObjectState, prev: Option<&ObjectState>) {
    let p = prev.cloned().unwrap_or_else(|| s.clone());
    field!(text, prev, "active", s.active, p.active, |v: bool| v);
    field!(text, prev, "pos", s.pos, p.pos, pos_text);
    field!(text, prev, "snap", s.snap, p.snap, |v: bool| v);
    field!(
        text,
        prev,
        "elevation",
        s.elevation,
        p.elevation,
        |v: bool| v
    );
    field!(text, prev, "zones", s.zones, p.zones, zones_name);
    field!(text, prev, "size", s.uniform_size(), p.uniform_size(), num);
    field!(text, prev, "importance", s.importance, p.importance, num);
    field!(text, prev, "gain", s.gain, p.gain, gain_text);
    field!(text, prev, "rampLength", s.ramp, p.ramp, |v: u32| v);
    field!(
        text,
        prev,
        "trimBypass",
        s.trim_bypass,
        p.trim_bypass,
        |v: bool| v
    );
    if prev.is_none() {
        text.push_str("    dialog: -1\n");
        text.push_str("    music: -1\n");
    }
    field!(
        text,
        prev,
        "screenFactor",
        s.screen_factor,
        p.screen_factor,
        num
    );
    field!(
        text,
        prev,
        "depthFactor",
        s.depth_factor,
        p.depth_factor,
        num
    );
    if prev.is_none() {
        text.push_str("    headTrackMode: undefined\n");
        text.push_str("    binauralRenderMode: undefined\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(pos: [f32; 3], gain: Gain) -> ObjectState {
        ObjectState {
            active: true,
            pos,
            snap: false,
            elevation: true,
            zones: 0,
            size: [0.0; 3],
            importance: 1.0,
            gain,
            ramp: 1536,
            trim_bypass: false,
            screen_factor: 0.0,
            depth_factor: 0.25,
        }
    }

    /// DAMF has one `size`: the width is written, axes that differ are
    /// counted, and a depth- or height-only change writes no line.
    #[test]
    fn size_is_written_from_the_width_and_axes_that_differ_are_counted() {
        use crate::loss::LossKind;
        let dir = std::env::temp_dir().join(format!("oadec-damf-size-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 1,
        };
        let mut w =
            DamfWriter::create(&dir, "t", &program, 48000, &DamfOptions::default()).unwrap();
        let mut first = object([-1.0, 1.0, 0.0], Gain::Db(0));
        first.size = [0.2, 0.5, 0.8];
        let mut second = first.clone();
        second.size = [0.2, 0.9, 0.8];
        w.push_event(&Event {
            id: 10,
            sample_pos: 0,
            state: ElementState::Object(first.clone()),
            previous: None,
        })
        .unwrap();
        w.push_event(&Event {
            id: 10,
            sample_pos: 1536,
            state: ElementState::Object(second),
            previous: Some(ElementState::Object(first)),
        })
        .unwrap();
        let summary = w.finish().unwrap();
        assert_eq!(summary.losses.count(LossKind::SizeAxesCollapsed), 2);
        let md = std::fs::read_to_string(dir.join("t.atmos.metadata")).unwrap();
        assert!(md.contains("    size: 0.2\n"), "{md}");
        let delta = md.rsplit("  - ID: 10\n").next().unwrap();
        assert_eq!(
            delta, "    samplePos: 1536\n",
            "a depth-only change writes nothing"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The `.atmos` header carries the frame rate verbatim; only the rates the
    /// Dolby tools list are accepted, so a typo cannot reach the encoder.
    #[test]
    fn fps_is_written_verbatim_and_validated() {
        let dir = std::env::temp_dir().join(format!("oadec-damf-fps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 1,
        };
        let options = DamfOptions {
            fps: "23.976".to_string(),
            ..DamfOptions::default()
        };
        let w = DamfWriter::create(&dir, "t", &program, 48000, &options).unwrap();
        w.finish().unwrap();
        let atmos = std::fs::read_to_string(dir.join("t.atmos")).unwrap();
        assert!(atmos.contains("    fps: 23.976\n"), "{atmos}");
        let bad = DamfOptions {
            fps: "24.5".to_string(),
            ..DamfOptions::default()
        };
        let refused = DamfWriter::create(&dir, "u", &program, 48000, &bad);
        assert!(
            matches!(refused, Err(DamfError::InvalidFps(ref f)) if f == "24.5"),
            "{refused:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The metadata file is streamed, so an event that arrives earlier than
    /// the previous one of its element is written where it came and declared.
    #[test]
    fn out_of_order_events_are_written_as_delivered_and_counted() {
        use crate::loss::LossKind;
        let dir = std::env::temp_dir().join(format!("oadec-damf-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 1,
        };
        let mut w =
            DamfWriter::create(&dir, "t", &program, 48000, &DamfOptions::default()).unwrap();
        for pos in [0u64, 48_000, 24_000] {
            w.push_event(&Event {
                id: 10,
                sample_pos: pos,
                state: ElementState::Object(object([-1.0, 1.0, 0.0], Gain::Db(0))),
                previous: None,
            })
            .unwrap();
        }
        let summary = w.finish().unwrap();
        assert_eq!(summary.losses.count(LossKind::OutOfOrderWrittenAsIs), 1);
        assert_eq!(
            summary.losses.examples(LossKind::OutOfOrderWrittenAsIs),
            &[(10, 24_000)]
        );
        assert!(summary.losses.declared_loss());
        let md = std::fs::read_to_string(dir.join("t.atmos.metadata")).unwrap();
        let order: Vec<&str> = md
            .lines()
            .filter_map(|l| l.strip_prefix("    samplePos: "))
            .collect();
        assert_eq!(order, ["0", "48000", "24000"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// DAMF has no ISF element either: refused by default, dropped and counted
    /// on request.
    #[test]
    fn isf_elements_are_refused_by_default_and_dropped_on_request() {
        use crate::loss::LossKind;
        use crate::program::IsfPolicy;
        let dir = std::env::temp_dir().join(format!("oadec-damf-isf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: Some(1),
            isf_objects: 8,
            dynamic_objects: 1,
        };
        let refused = DamfWriter::create(&dir, "t", &program, 48000, &DamfOptions::default());
        assert!(
            matches!(
                refused,
                Err(DamfError::IsfNotRepresentable { count: 8, ref isf_type }) if isf_type == "SR5.3.0.0"
            ),
            "{refused:?}"
        );
        let options = DamfOptions {
            isf: IsfPolicy::Drop,
            ..DamfOptions::default()
        };
        let mut w = DamfWriter::create(&dir, "t", &program, 48000, &options).unwrap();
        assert_eq!(w.channels(), 11, "ten bed tracks and one dynamic object");
        w.write_frame(&[0; 10]).unwrap();
        let summary = w.finish().unwrap();
        assert_eq!(summary.losses.count(LossKind::IsfDropped), 8);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writes_the_three_files_with_a_conformed_bed() {
        let dir = std::env::temp_dir().join(format!("oadec-damf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 2,
        };
        let mut w =
            DamfWriter::create(&dir, "t", &program, 48000, &DamfOptions::default()).unwrap();
        assert_eq!(w.channels(), 12);
        // element order: LFE, obj1, obj2
        w.write_frame(&[7, 8, 9]).unwrap();
        let rows = [[1i32, 2, 3], [4, 5, 6]];
        w.write_frames(rows.iter().map(|r| &r[..]), 3).unwrap();
        w.push_event(&Event {
            id: 3,
            sample_pos: 0,
            state: ElementState::Bed(BedState {
                active: true,
                importance: 1.0,
                gain: Gain::Db(0),
                ramp: 32,
                trim_bypass: false,
            }),
            previous: None,
        })
        .unwrap();
        let first = object([-1.0, 1.0, 0.0], Gain::Db(0));
        w.push_event(&Event {
            id: 10,
            sample_pos: 0,
            state: ElementState::Object(first.clone()),
            previous: None,
        })
        .unwrap();
        let second = object([0.5, 1.0, 0.0], Gain::Db(-3));
        w.push_event(&Event {
            id: 10,
            sample_pos: 1536,
            state: ElementState::Object(second),
            previous: Some(ElementState::Object(first)),
        })
        .unwrap();
        let summary = w.finish().unwrap();
        assert_eq!(summary.frames, 3);
        assert_eq!(summary.events, 3);

        let atmos = std::fs::read_to_string(dir.join("t.atmos")).unwrap();
        assert!(atmos.starts_with("version: 0.5.1\npresentations:\n  - type: home\n"));
        assert!(atmos.contains(
            "          - channel: LFE
            ID: 3
"
        ));
        assert!(atmos.contains(
            "          - channel: Lts
            ID: 8
"
        ));
        assert!(atmos.contains("          - channel: LFE\n            ID: 3\n"));
        assert!(atmos.contains("    objects:\n      - ID: 10\n      - ID: 11\n"));

        let md = std::fs::read_to_string(dir.join("t.atmos.metadata")).unwrap();
        assert!(md.starts_with(
            "sampleRate: 48000\nevents:\n  - ID: 3\n    samplePos: 0\n    active: true\n"
        ));
        assert!(md.contains("    binauralRenderMode: off\n  - ID: 10\n    samplePos: 0\n    active: true\n    pos: [-1, 1, 0]\n"));
        let diff = md.rsplit("  - ID: 10\n").next().unwrap();
        assert_eq!(
            diff,
            "    samplePos: 1536\n    pos: [0.5, 1, 0]\n    gain: -3\n"
        );

        let audio = std::fs::read(dir.join("t.atmos.audio")).unwrap();
        // first frame: slot 3 (LFE) = 7, slots 10/11 = 8/9, everything else silent
        let data = &audio[68..];
        let sample =
            |i: usize| i32::from_be_bytes([data[i * 3], data[i * 3 + 1], data[i * 3 + 2], 0]) >> 8;
        assert_eq!(sample(3), 7);
        assert_eq!(sample(10), 8);
        assert_eq!(sample(11), 9);
        assert_eq!(sample(0), 0);
        assert_eq!(sample(12 + 3), 1);
        assert_eq!(sample(24 + 11), 6);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
