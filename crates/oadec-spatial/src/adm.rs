//! ADM BWF writer: a Broadcast Wave file (RIFF, or RF64 beyond 4 GiB) carrying
//! the object presentation as PCM tracks plus the `axml` (Audio Definition
//! Model, ITU-R BS.2076), `chna` (EBU Tech 3285 s7) and `dbmd` chunks the Dolby
//! tools expect, following the Dolby Atmos master ADM profile v1.0.
//!
//! Layout as the Dolby converter writes it: `RIFF/WAVE`, a 28-byte `JUNK`
//! placeholder that becomes `ds64` for RF64, `fmt ` (plain PCM, 24 bit), `data`,
//! then `axml`, `chna`, `dbmd` after the samples. The XML is generated from
//! the element events with one `audioBlockFormat` per event (the profile
//! treats blocks as discrete metadata events: `jumpPosition` with a fixed
//! 250-sample interpolation length after the first block).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use oadec_emdf::oamd::{BedChannel, Gain};

use crate::dbmd;
use crate::loss::{LossKind, LossLedger};
use crate::program::{
    BedState, ElementState, Event, IsfPolicy, ObjectState, Program, STANDARD_BED,
};

/// Interpolation length of every block after the first, in samples.
pub const INTERPOLATION_SAMPLES: u32 = 250;

/// How `interpolationLength` is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interpolation {
    /// The Dolby Atmos master ADM profile (table 11): 0 on the first block,
    /// 250 samples afterwards, whatever the source ramp was. The default;
    /// Dolby's converters write the same.
    #[default]
    Profile,
    /// The source ramp of every block, as BS.2076 allows. Outside the
    /// profile: the file's `dbmd` tool string says so, and it must not carry
    /// the Dolby origin tag.
    Real,
}

/// Options of the writer.
#[derive(Debug, Clone)]
pub struct AdmOptions {
    /// Write the full ten-channel 7.1.2 bed (silence in unused slots).
    pub bed_conform: bool,
    /// `audioProgrammeName`.
    pub programme_name: String,
    /// `audioContentName`.
    pub content_name: String,
    /// Creator string written into `dbmd`.
    pub creator: String,
    /// Tool string written into `dbmd`.
    pub tool: String,
    /// What to do with intermediate-spatial-format elements, which the
    /// profile cannot represent: refuse (the default) or drop and count.
    pub isf: IsfPolicy,
    /// Write a programme whose sample rate is not the profile's 48 000 Hz
    /// (a declared loss) instead of refusing it.
    pub allow_non_profile_rate: bool,
    /// How interpolation lengths are written.
    pub interpolation: Interpolation,
}

impl Default for AdmOptions {
    fn default() -> Self {
        Self {
            bed_conform: true,
            programme_name: "Atmos_Master".to_string(),
            content_name: "Atmos_Master_Content".to_string(),
            creator: "Created using oadec".to_string(),
            tool: format!("oadec {}", env!("CARGO_PKG_VERSION")),
            isf: IsfPolicy::Error,
            allow_non_profile_rate: false,
            interpolation: Interpolation::Profile,
        }
    }
}

/// What the `dbmd` tool string says after the tool when the file is outside
/// the profile.
pub const NON_PROFILE_MARK: &str = "non-profile: real interpolation lengths";

/// The only sample rate the profile allows (table 23).
pub const PROFILE_SAMPLE_RATE: u32 = 48_000;

/// Objects the profile can number: `AO_100b` to `AO_1080` (table 17).
pub const MAX_OBJECTS: usize = 118;

/// Tracks the profile allows in one file.
pub const MAX_TRACKS: usize = 128;

/// Summary returned when the file is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmSummary {
    /// Frames written.
    pub frames: u64,
    /// Audio channels.
    pub channels: usize,
    /// `audioBlockFormat` instances written for objects.
    pub blocks: u64,
    /// The file was written as RF64.
    pub rf64: bool,
    /// Total bytes.
    pub bytes: u64,
    /// What the file does not carry of the programme it was given.
    pub losses: LossLedger,
}

/// Where an element's audio goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Channel(usize),
    None,
}

/// Errors of the writer.
#[derive(Debug, thiserror::Error)]
pub enum AdmError {
    /// I/O.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// The bed uses a channel the profile has no DirectSpeakers definition for.
    #[error("bed channel {0:?} is not allowed in a Dolby Atmos master ADM bed")]
    UnsupportedBedChannel(BedChannel),
    /// The programme has intermediate-spatial-format objects, which the
    /// profile cannot represent, and the options said not to drop them.
    #[error(
        "{count} intermediate-spatial-format objects ({isf_type}) cannot be represented in a Dolby Atmos master ADM file"
    )]
    IsfNotRepresentable {
        /// ISF objects in the programme.
        count: usize,
        /// The ISF type (table 11b), or "reserved type".
        isf_type: String,
    },
    /// The programme is not at 48 000 Hz and the options did not allow a
    /// file outside the profile.
    #[error(
        "the Dolby Atmos master ADM profile requires 48 000 Hz (table 23); this programme is {0} Hz"
    )]
    NonProfileSampleRate(u32),
    /// More dynamic objects than the profile can number.
    #[error(
        "the profile numbers at most {max} objects (AO_100b to AO_1080); the programme has {objects}"
    )]
    TooManyObjects {
        /// Dynamic objects in the programme.
        objects: usize,
        /// The limit.
        max: usize,
    },
    /// More tracks than the profile allows in one file.
    #[error("the profile allows at most {max} tracks in one file; the output would have {tracks}")]
    TooManyTracks {
        /// Tracks the output would have.
        tracks: usize,
        /// The limit.
        max: usize,
    },
}

/// Per-bed-channel constants of the profile (name suffix, speaker label, position).
fn bed_profile(ch: BedChannel) -> Result<(&'static str, &'static str, [f32; 3]), AdmError> {
    Ok(match ch {
        BedChannel::L => ("Left", "RC_L", [-1.0, 1.0, 0.0]),
        BedChannel::R => ("Right", "RC_R", [1.0, 1.0, 0.0]),
        BedChannel::C => ("Center", "RC_C", [0.0, 1.0, 0.0]),
        BedChannel::LFE => ("LFE", "RC_LFE", [-1.0, 1.0, -1.0]),
        BedChannel::Ls => ("LeftSideSurround", "RC_Lss", [-1.0, 0.0, 0.0]),
        BedChannel::Rs => ("RightSideSurround", "RC_Rss", [1.0, 0.0, 0.0]),
        BedChannel::Lb => ("LeftRearSurround", "RC_Lrs", [-1.0, -1.0, 0.0]),
        BedChannel::Rb => ("RightRearSurround", "RC_Rrs", [1.0, -1.0, 0.0]),
        BedChannel::Tsl => ("LeftTopSurround", "RC_Lts", [-1.0, 0.0, 1.0]),
        BedChannel::Tsr => ("RightTopSurround", "RC_Rts", [1.0, 0.0, 1.0]),
        other => return Err(AdmError::UnsupportedBedChannel(other)),
    })
}

/// Bit of a bed channel in the `dbmd` bed mask.
fn bed_mask_bit(ch: BedChannel) -> u16 {
    match ch {
        BedChannel::L => 0,
        BedChannel::R => 1,
        BedChannel::C => 2,
        BedChannel::LFE => 3,
        BedChannel::Ls => 4,
        BedChannel::Rs => 5,
        BedChannel::Lb => 6,
        BedChannel::Rb => 7,
        BedChannel::Tfl => 8,
        BedChannel::Tfr => 9,
        BedChannel::Tsl => 10,
        BedChannel::Tsr => 11,
        BedChannel::Tbl => 12,
        BedChannel::Tbr => 13,
        BedChannel::Lw => 14,
        BedChannel::Rw => 15,
        BedChannel::LFE2 => 15,
    }
}

/// Streaming ADM BWF writer.
#[derive(Debug)]
pub struct AdmWriter {
    out: BufWriter<File>,
    path: PathBuf,
    sample_rate: u32,
    channels: usize,
    slots: Vec<Slot>,
    bed: Vec<BedChannel>,
    objects: usize,
    frames: u64,
    /// Events per element id, in order.
    events: BTreeMap<u32, Vec<(u64, ObjectState)>>,
    options: AdmOptions,
    frame: Vec<u8>,
    /// The last state seen per bed channel id: a DirectSpeakers block has no
    /// time, so every later change is a loss to count.
    bed_last: BTreeMap<u32, BedState>,
    losses: LossLedger,
}

const JUNK_LEN: u32 = 28;
/// Byte offset of the `data` chunk size field.
const DATA_SIZE_POS: u64 = 12 + 8 + JUNK_LEN as u64 + 8 + 16 + 4;

impl AdmWriter {
    /// Creates the file for `program`.
    pub fn create(
        path: &Path,
        program: &Program,
        sample_rate: u32,
        options: &AdmOptions,
    ) -> Result<Self, AdmError> {
        let coded = program.bed_channels();
        let mut bed: Vec<BedChannel> = Vec::new();
        if options.bed_conform {
            bed.extend(STANDARD_BED);
            for &ch in &coded {
                if !bed.contains(&ch) {
                    bed.push(ch);
                }
            }
        } else {
            bed.extend(coded.iter().copied());
        }
        for &ch in &bed {
            bed_profile(ch)?;
        }
        let mut losses = LossLedger::default();
        if sample_rate != PROFILE_SAMPLE_RATE {
            if !options.allow_non_profile_rate {
                return Err(AdmError::NonProfileSampleRate(sample_rate));
            }
            losses.note(LossKind::NonProfileSampleRate, 0, 0);
        }
        if program.isf_objects > 0 {
            match options.isf {
                IsfPolicy::Error => {
                    return Err(AdmError::IsfNotRepresentable {
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
        let mut slots = Vec::with_capacity(program.elements());
        for ch in &coded {
            slots.push(
                bed.iter()
                    .position(|c| c == ch)
                    .map_or(Slot::None, Slot::Channel),
            );
        }
        for _ in 0..program.isf_objects {
            slots.push(Slot::None);
        }
        for k in 0..program.dynamic_objects {
            slots.push(Slot::Channel(bed.len() + k));
        }
        let channels = bed.len() + program.dynamic_objects;
        if program.dynamic_objects > MAX_OBJECTS {
            return Err(AdmError::TooManyObjects {
                objects: program.dynamic_objects,
                max: MAX_OBJECTS,
            });
        }
        if channels > MAX_TRACKS {
            return Err(AdmError::TooManyTracks {
                tracks: channels,
                max: MAX_TRACKS,
            });
        }

        let file = File::create(path)?;
        let mut out = BufWriter::with_capacity(4 << 20, file);
        out.write_all(b"RIFF")?;
        out.write_all(&0u32.to_le_bytes())?;
        out.write_all(b"WAVE")?;
        out.write_all(b"JUNK")?;
        out.write_all(&JUNK_LEN.to_le_bytes())?;
        out.write_all(&[0u8; JUNK_LEN as usize])?;
        out.write_all(b"fmt ")?;
        out.write_all(&16u32.to_le_bytes())?;
        let block_align = channels as u16 * 3;
        out.write_all(&1u16.to_le_bytes())?;
        out.write_all(&(channels as u16).to_le_bytes())?;
        out.write_all(&sample_rate.to_le_bytes())?;
        out.write_all(&(sample_rate * u32::from(block_align)).to_le_bytes())?;
        out.write_all(&block_align.to_le_bytes())?;
        out.write_all(&24u16.to_le_bytes())?;
        out.write_all(b"data")?;
        out.write_all(&0u32.to_le_bytes())?;
        Ok(Self {
            out,
            path: path.to_path_buf(),
            sample_rate,
            channels,
            slots,
            bed,
            objects: program.dynamic_objects,
            frames: 0,
            events: BTreeMap::new(),
            options: options.clone(),
            frame: Vec::with_capacity(channels * 3 * 160),
            bed_last: BTreeMap::new(),
            losses,
        })
    }

    /// Audio channels.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Writes frames: `rows` holds the element samples of each frame in stream
    /// order, `elements` of them per row.
    pub fn write_frames<'a>(
        &mut self,
        rows: impl Iterator<Item = &'a [i32]>,
        elements: usize,
    ) -> io::Result<()> {
        self.frame.clear();
        let mut samples = vec![0i32; self.channels];
        for row in rows {
            samples.iter_mut().for_each(|s| *s = 0);
            for (i, &slot) in self.slots.iter().enumerate().take(elements) {
                if let (Slot::Channel(c), Some(&v)) = (slot, row.get(i)) {
                    samples[c] = v;
                }
            }
            for &v in &samples {
                let b = v.clamp(-(1 << 23), (1 << 23) - 1).to_le_bytes();
                self.frame.extend_from_slice(&b[..3]);
            }
            self.frames += 1;
        }
        self.out.write_all(&self.frame)
    }

    /// Records an event. Object events become blocks. Bed channels have static
    /// metadata in the profile, so a bed event is not written; a first state
    /// the bed block cannot express and every later change are counted.
    pub fn push_event(&mut self, event: &Event) {
        match &event.state {
            ElementState::Object(s) => {
                // Counted per event, not per block: an event that only changed
                // the depth or height never becomes a block, and is a loss.
                if s.size_axes_differ() {
                    self.losses
                        .note(LossKind::SizeAxesCollapsed, event.id, event.sample_pos);
                }
                self.events
                    .entry(event.id)
                    .or_default()
                    .push((event.sample_pos, s.clone()));
            }
            ElementState::Bed(s) => {
                match self.bed_last.get(&event.id) {
                    None => {
                        if s.gain != Gain::Db(0) || !s.active {
                            self.losses
                                .note(LossKind::BedGainDropped, event.id, event.sample_pos);
                        }
                    }
                    Some(last) => {
                        if s.active != last.active
                            || s.gain != last.gain
                            || s.importance != last.importance
                            || s.trim_bypass != last.trim_bypass
                        {
                            self.losses
                                .note(LossKind::BedEventDropped, event.id, event.sample_pos);
                        }
                    }
                }
                self.bed_last.insert(event.id, s.clone());
            }
        }
    }

    /// Writes the metadata chunks, patches the sizes and closes the file.
    pub fn finish(mut self) -> Result<AdmSummary, AdmError> {
        let data_bytes = self.frames * self.channels as u64 * 3;
        if data_bytes % 2 == 1 {
            self.out.write_all(&[0])?;
        }
        let (xml, blocks, ledger) = self.axml();
        self.losses.merge(&ledger);
        write_chunk(&mut self.out, b"axml", xml.as_bytes())?;
        let chna = self.chna();
        write_chunk(&mut self.out, b"chna", &chna)?;
        let bed_mask = self
            .bed
            .iter()
            .fold(0u16, |m, &c| m | (1 << bed_mask_bit(c)));
        let mut lfe = vec![false; self.channels];
        for (i, &c) in self.bed.iter().enumerate() {
            lfe[i] = matches!(c, BedChannel::LFE | BedChannel::LFE2);
        }
        let tool = match self.options.interpolation {
            Interpolation::Profile => self.options.tool.clone(),
            Interpolation::Real => format!("{} ({NON_PROFILE_MARK})", self.options.tool),
        };
        let dbmd = dbmd::build(bed_mask, &lfe, &self.options.creator, &tool);
        write_chunk(&mut self.out, b"dbmd", &dbmd)?;
        self.out.flush()?;
        let mut file = self
            .out
            .into_inner()
            .map_err(io::IntoInnerError::into_error)?;
        let total = file.metadata()?.len();
        let riff_size = total - 8;
        let rf64 = riff_size > u64::from(u32::MAX) || data_bytes > u64::from(u32::MAX);
        if rf64 {
            file.seek(SeekFrom::Start(0))?;
            file.write_all(b"RF64")?;
            file.write_all(&u32::MAX.to_le_bytes())?;
            file.seek(SeekFrom::Start(12))?;
            file.write_all(b"ds64")?;
            file.write_all(&JUNK_LEN.to_le_bytes())?;
            file.write_all(&riff_size.to_le_bytes())?;
            file.write_all(&data_bytes.to_le_bytes())?;
            file.write_all(&self.frames.to_le_bytes())?;
            file.write_all(&0u32.to_le_bytes())?;
            file.seek(SeekFrom::Start(DATA_SIZE_POS))?;
            file.write_all(&u32::MAX.to_le_bytes())?;
        } else {
            file.seek(SeekFrom::Start(4))?;
            file.write_all(&(riff_size as u32).to_le_bytes())?;
            file.seek(SeekFrom::Start(DATA_SIZE_POS))?;
            file.write_all(&(data_bytes as u32).to_le_bytes())?;
        }
        file.flush()?;
        Ok(AdmSummary {
            frames: self.frames,
            channels: self.channels,
            blocks,
            rf64,
            bytes: total,
            losses: self.losses,
        })
    }

    /// Path of the file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn timecode(&self, samples: u64) -> String {
        timecode(samples, self.sample_rate)
    }

    /// The `chna` chunk: one entry per track.
    fn chna(&self) -> Vec<u8> {
        let n = self.channels as u16;
        let mut v = Vec::with_capacity(4 + 40 * self.channels);
        v.extend_from_slice(&n.to_le_bytes());
        v.extend_from_slice(&n.to_le_bytes());
        for i in 0..self.channels {
            let (track_ref, pack_ref) = if i < self.bed.len() {
                (
                    format!("AT_{:08x}_01", 0x0001_1001 + i),
                    "AP_00011001".to_string(),
                )
            } else {
                let k = i - self.bed.len() + 1;
                (
                    format!("AT_{:08x}_01", 0x0003_1000 + k),
                    format!("AP_{:08x}", 0x0003_1000 + k),
                )
            };
            v.extend_from_slice(&(i as u16 + 1).to_le_bytes());
            v.extend_from_slice(fixed(&format!("ATU_{:08x}", i + 1), 12).as_slice());
            v.extend_from_slice(fixed(&track_ref, 14).as_slice());
            v.extend_from_slice(fixed(&pack_ref, 11).as_slice());
            v.push(0);
        }
        v
    }

    /// The `axml` chunk, the number of object blocks written, and what the
    /// blocks could not carry.
    fn axml(&self) -> (String, u64, LossLedger) {
        let mut ledger = LossLedger::default();
        let t = |s: u64| self.timecode(s);
        let end = t(self.frames);
        let mut x = String::with_capacity(64 * 1024);
        let bed_tracks = self.bed.len();
        x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        x.push_str("<ebuCoreMain xmlns=\"urn:ebu:metadata-schema:ebuCore_2016\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:schemaLocation=\"urn:ebu:metadata-schema:ebuCore_2016 ebucore.xsd\" xml:lang=\"en\">\n");
        x.push_str("\t<coreMetadata>\n\t\t<format>\n\t\t\t<audioFormatExtended>\n");
        // programme and content
        x.push_str(&format!(
            "\t\t\t\t<audioProgramme audioProgrammeID=\"APR_1001\" audioProgrammeName=\"{}\" start=\"00:00:00.00000\" end=\"{end}\">\n\t\t\t\t\t<audioContentIDRef>ACO_1001</audioContentIDRef>\n\t\t\t\t</audioProgramme>\n",
            xml_escape(&self.options.programme_name)
        ));
        x.push_str(&format!(
            "\t\t\t\t<audioContent audioContentID=\"ACO_1001\" audioContentName=\"{}\">\n",
            xml_escape(&self.options.content_name)
        ));
        // Objects are AO_100b.. whatever the bed holds (table 17; Dolby's
        // converters number an LFE-only bed's objects the same way).
        let object_id = |k: usize| 0x100a + k; // k is 1-based for objects
        if bed_tracks > 0 {
            x.push_str("\t\t\t\t\t<audioObjectIDRef>AO_1001</audioObjectIDRef>\n");
        }
        for k in 1..=self.objects {
            x.push_str(&format!(
                "\t\t\t\t\t<audioObjectIDRef>AO_{:04x}</audioObjectIDRef>\n",
                object_id(k)
            ));
        }
        x.push_str(
            "\t\t\t\t\t<dialogue mixedContentKind=\"0\">2</dialogue>\n\t\t\t\t</audioContent>\n",
        );
        // objects
        if bed_tracks > 0 {
            x.push_str(&format!(
                "\t\t\t\t<audioObject audioObjectID=\"AO_1001\" audioObjectName=\"Atmos_Bed_1\" start=\"00:00:00.00000\" duration=\"{end}\">\n\t\t\t\t\t<audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef>\n"
            ));
            for i in 1..=bed_tracks {
                x.push_str(&format!(
                    "\t\t\t\t\t<audioTrackUIDRef>ATU_{i:08x}</audioTrackUIDRef>\n"
                ));
            }
            x.push_str("\t\t\t\t</audioObject>\n");
        }
        for k in 1..=self.objects {
            x.push_str(&format!(
                "\t\t\t\t<audioObject audioObjectID=\"AO_{:04x}\" audioObjectName=\"Atmos_Obj_{k}\" start=\"00:00:00.00000\" duration=\"{end}\">\n\t\t\t\t\t<audioPackFormatIDRef>AP_{:08x}</audioPackFormatIDRef>\n\t\t\t\t\t<audioTrackUIDRef>ATU_{:08x}</audioTrackUIDRef>\n\t\t\t\t</audioObject>\n",
                object_id(k),
                0x0003_1000 + k,
                bed_tracks + k
            ));
        }
        // pack formats
        if bed_tracks > 0 {
            x.push_str("\t\t\t\t<audioPackFormat audioPackFormatID=\"AP_00011001\" audioPackFormatName=\"AtmosCustomPackFormat1\" typeDefinition=\"DirectSpeakers\" typeLabel=\"0001\">\n");
            for i in 1..=bed_tracks {
                x.push_str(&format!(
                    "\t\t\t\t\t<audioChannelFormatIDRef>AC_{:08x}</audioChannelFormatIDRef>\n",
                    0x0001_1000 + i
                ));
            }
            x.push_str("\t\t\t\t</audioPackFormat>\n");
        }
        for k in 1..=self.objects {
            x.push_str(&format!(
                "\t\t\t\t<audioPackFormat audioPackFormatID=\"AP_{0:08x}\" audioPackFormatName=\"Atmos_Obj_{k}\" typeDefinition=\"Objects\" typeLabel=\"0003\">\n\t\t\t\t\t<audioChannelFormatIDRef>AC_{0:08x}</audioChannelFormatIDRef>\n\t\t\t\t</audioPackFormat>\n",
                0x0003_1000 + k
            ));
        }
        // channel formats: bed
        for (i, &ch) in self.bed.iter().enumerate() {
            let (name, label, pos) = bed_profile(ch).expect("checked at creation");
            let id = 0x0001_1001 + i;
            x.push_str(&format!(
                "\t\t\t\t<audioChannelFormat audioChannelFormatID=\"AC_{id:08x}\" audioChannelFormatName=\"RoomCentric{name}\" typeDefinition=\"DirectSpeakers\" typeLabel=\"0001\">\n\t\t\t\t\t<audioBlockFormat audioBlockFormatID=\"AB_{id:08x}_00000001\">\n\t\t\t\t\t\t<speakerLabel>{label}</speakerLabel>\n\t\t\t\t\t\t<cartesian>1</cartesian>\n\t\t\t\t\t\t<position coordinate=\"X\">{}</position>\n\t\t\t\t\t\t<position coordinate=\"Y\">{}</position>\n\t\t\t\t\t\t<position coordinate=\"Z\">{}</position>\n\t\t\t\t\t</audioBlockFormat>\n\t\t\t\t</audioChannelFormat>\n",
                coord(pos[0]),
                coord(pos[1]),
                coord(pos[2])
            ));
        }
        // channel formats: objects, one block per event
        let mut blocks_total = 0u64;
        let real = self.options.interpolation == Interpolation::Real;
        let interpolation_text = |n: usize, s: &ObjectState| -> String {
            match self.options.interpolation {
                Interpolation::Profile => format!(
                    "{:.6}",
                    if n == 0 {
                        0.0
                    } else {
                        f64::from(INTERPOLATION_SAMPLES) / f64::from(self.sample_rate)
                    }
                ),
                Interpolation::Real => format!(
                    "{:.10}",
                    if n == 0 {
                        0.0
                    } else {
                        f64::from(s.ramp) / f64::from(self.sample_rate)
                    }
                ),
            }
        };
        for k in 1..=self.objects {
            let id = 0x0003_1000 + k;
            let element_id = 10 + (k as u32 - 1);
            x.push_str(&format!(
                "\t\t\t\t<audioChannelFormat audioChannelFormatID=\"AC_{id:08x}\" audioChannelFormatName=\"Atmos_Obj_{k}\" typeDefinition=\"Objects\" typeLabel=\"0003\">\n"
            ));
            let mut events: Vec<(u64, ObjectState)> =
                self.events.get(&element_id).cloned().unwrap_or_default();
            // Blocks tile in time; the events may not have arrived in it. The
            // sort is stable, so two events at one sample keep their order.
            events.sort_by_key(|(pos, _)| *pos);
            // Nothing can start at or after the end, and the last block ends at
            // `frames` (Dolby's converters drop such events the same way).
            let in_range = events.partition_point(|(pos, _)| *pos < self.frames);
            for (pos, _) in events.drain(in_range..) {
                ledger.note(LossKind::EventBeyondEndDropped, element_id, pos);
            }
            // Two events at one sample: the last one is the block.
            let mut deduped: Vec<(u64, ObjectState)> = Vec::with_capacity(events.len());
            for e in events {
                if deduped.last().is_some_and(|(p, _)| *p == e.0) {
                    ledger.note(LossKind::SamePositionSuperseded, element_id, e.0);
                    deduped.pop();
                }
                deduped.push(e);
            }
            let mut events = deduped;
            // The profile wants the first block at time zero: hold the first
            // known state (or a default) from the start. The real first event
            // keeps its own block, so its arrival time stays in the file; Dolby
            // writes an active default block at the room centre instead.
            let synthetic = events.first().is_none_or(|(pos, _)| *pos > 0);
            if synthetic {
                if let Some((pos, _)) = events.first() {
                    ledger.note(LossKind::LateFirstEventHeld, element_id, *pos);
                }
                let state = events.first().map_or_else(
                    || ObjectState {
                        active: false,
                        pos: [0.0, 0.0, 0.0],
                        snap: false,
                        elevation: true,
                        zones: 0,
                        size: [0.0; 3],
                        importance: 1.0,
                        gain: Gain::Db(0),
                        ramp: 0,
                        trim_bypass: false,
                        screen_factor: 0.0,
                        depth_factor: 0.25,
                    },
                    |(_, s)| s.clone(),
                );
                events.insert(0, (0, state));
            }
            // The Dolby converters write one block per event but drop a trailing
            // event whose ADM content equals the previous one (an event that only
            // changed the ramp or the trim, which ADM has no fields for). The
            // real first event behind a synthetic block is never popped.
            let keep = if synthetic { 2 } else { 1 };
            while events.len() > keep
                && adm_equal(
                    &events[events.len() - 2].1,
                    &events[events.len() - 1].1,
                    real,
                )
            {
                events.pop();
            }
            for (n, (pos, s)) in events.iter().enumerate() {
                let next = events.get(n + 1).map_or(self.frames, |(p, _)| *p);
                if next <= *pos {
                    continue;
                }
                blocks_total += 1;
                if !real && n > 0 && s.ramp != INTERPOLATION_SAMPLES {
                    ledger.note_ramp(element_id, *pos, s.ramp);
                }
                if s.active && s.importance != 1.0 {
                    ledger.note(LossKind::ImportanceOmitted, element_id, *pos);
                }
                if s.screen_factor != 0.0 {
                    ledger.note(LossKind::ScreenReferenceDropped, element_id, *pos);
                }
                if s.trim_bypass {
                    ledger.note(LossKind::TrimBypassDropped, element_id, *pos);
                }
                x.push_str(&format!(
                    "\t\t\t\t\t<audioBlockFormat audioBlockFormatID=\"AB_{id:08x}_{:08x}\" rtime=\"{}\" duration=\"{}\">\n\t\t\t\t\t\t<cartesian>1</cartesian>\n",
                    n + 1,
                    t(*pos),
                    t(next - pos)
                ));
                if s.active {
                    if let Some(g) = active_gain_text(s.gain) {
                        x.push_str(&format!("\t\t\t\t\t\t<gain>{g}</gain>\n"));
                    }
                } else {
                    x.push_str(
                        "\t\t\t\t\t\t<gain>0.0</gain>\n\t\t\t\t\t\t<importance>0</importance>\n",
                    );
                }
                x.push_str(&format!(
                    "\t\t\t\t\t\t<position coordinate=\"X\">{}</position>\n\t\t\t\t\t\t<position coordinate=\"Y\">{}</position>\n",
                    coord(s.pos[0]),
                    coord(s.pos[1])
                ));
                if s.pos[2] != 0.0 {
                    x.push_str(&format!(
                        "\t\t\t\t\t\t<position coordinate=\"Z\">{}</position>\n",
                        coord(s.pos[2])
                    ));
                }
                if s.uniform_size() != 0.0 {
                    let sz = coord(s.uniform_size());
                    x.push_str(&format!(
                        "\t\t\t\t\t\t<width>{sz}</width>\n\t\t\t\t\t\t<depth>{sz}</depth>\n\t\t\t\t\t\t<height>{sz}</height>\n"
                    ));
                }
                if s.snap {
                    x.push_str("\t\t\t\t\t\t<channelLock>1</channelLock>\n");
                }
                x.push_str(&format!(
                    "\t\t\t\t\t\t<jumpPosition interpolationLength=\"{}\">1</jumpPosition>\n",
                    interpolation_text(n, s)
                ));
                if s.zones != 0 || !s.elevation {
                    x.push_str("\t\t\t\t\t\t<zoneExclusion>\n");
                    for (min_x, max_x, min_y, max_y, min_z, max_z, name) in zone_rectangles(s.zones)
                    {
                        x.push_str(&format!(
                            "\t\t\t\t\t\t\t<zone minX=\"{min_x}\" maxX=\"{max_x}\" minY=\"{min_y}\" maxY=\"{max_y}\" minZ=\"{min_z}\" maxZ=\"{max_z}\">{name}</zone>\n"
                        ));
                    }
                    if !s.elevation {
                        x.push_str("\t\t\t\t\t\t\t<zone minX=\"-1\" maxX=\"1\" minY=\"-1\" maxY=\"1\" minZ=\"-1\" maxZ=\"-0.4995\">ZB</zone>\n");
                        x.push_str("\t\t\t\t\t\t\t<zone minX=\"-1\" maxX=\"1\" minY=\"-1\" maxY=\"1\" minZ=\"0.4995\" maxZ=\"1\">ZT</zone>\n");
                    }
                    x.push_str("\t\t\t\t\t\t</zoneExclusion>\n");
                }
                x.push_str("\t\t\t\t\t</audioBlockFormat>\n");
            }
            x.push_str("\t\t\t\t</audioChannelFormat>\n");
        }
        // stream formats, track formats, track UIDs
        for i in 0..self.channels {
            let (id, name, pack) = if i < bed_tracks {
                let (name, _, _) = bed_profile(self.bed[i]).expect("checked at creation");
                (
                    0x0001_1001 + i,
                    format!("PCM_RoomCentric{name}"),
                    "AP_00011001".to_string(),
                )
            } else {
                let k = i - bed_tracks + 1;
                (
                    0x0003_1000 + k,
                    format!("PCM_Atmos_Obj_{k}"),
                    format!("AP_{:08x}", 0x0003_1000 + k),
                )
            };
            x.push_str(&format!(
                "\t\t\t\t<audioStreamFormat audioStreamFormatID=\"AS_{id:08x}\" audioStreamFormatName=\"{name}\" formatDefinition=\"PCM\" formatLabel=\"0001\">\n\t\t\t\t\t<audioChannelFormatIDRef>AC_{id:08x}</audioChannelFormatIDRef>\n\t\t\t\t\t<audioPackFormatIDRef>{pack}</audioPackFormatIDRef>\n\t\t\t\t\t<audioTrackFormatIDRef>AT_{id:08x}_01</audioTrackFormatIDRef>\n\t\t\t\t</audioStreamFormat>\n"
            ));
        }
        for i in 0..self.channels {
            let (id, name) = if i < bed_tracks {
                let (name, _, _) = bed_profile(self.bed[i]).expect("checked at creation");
                (0x0001_1001 + i, format!("PCM_RoomCentric{name}"))
            } else {
                let k = i - bed_tracks + 1;
                (0x0003_1000 + k, format!("PCM_Atmos_Obj_{k}"))
            };
            x.push_str(&format!(
                "\t\t\t\t<audioTrackFormat audioTrackFormatID=\"AT_{id:08x}_01\" audioTrackFormatName=\"{name}\" formatDefinition=\"PCM\" formatLabel=\"0001\">\n\t\t\t\t\t<audioStreamFormatIDRef>AS_{id:08x}</audioStreamFormatIDRef>\n\t\t\t\t</audioTrackFormat>\n"
            ));
        }
        for i in 0..self.channels {
            let (id, pack) = if i < bed_tracks {
                (0x0001_1001 + i, "AP_00011001".to_string())
            } else {
                let k = i - bed_tracks + 1;
                (0x0003_1000 + k, format!("AP_{:08x}", 0x0003_1000 + k))
            };
            x.push_str(&format!(
                "\t\t\t\t<audioTrackUID UID=\"ATU_{:08x}\" bitDepth=\"24\" sampleRate=\"{}\">\n\t\t\t\t\t<audioTrackFormatIDRef>AT_{id:08x}_01</audioTrackFormatIDRef>\n\t\t\t\t\t<audioPackFormatIDRef>{pack}</audioPackFormatIDRef>\n\t\t\t\t</audioTrackUID>\n",
                i + 1,
                self.sample_rate
            ));
        }
        x.push_str(
            "\t\t\t</audioFormatExtended>\n\t\t</format>\n\t</coreMetadata>\n</ebuCoreMain>\n",
        );
        (x, blocks_total, ledger)
    }
}

/// The `<gain>` text of an active object, as the Dolby converters print it:
/// nothing at 0 dB, ten decimals of the float32 linear factor otherwise
/// (0.5011872053 for −6 dB, as the reference files carry it, where float64
/// arithmetic would print 0.5011872336), and `0.0` alone when the object is
/// muted. The profile text reserves `gain` for inactive objects; Dolby's
/// converters write it on active ones and Dolby's validators accept it.
fn active_gain_text(g: Gain) -> Option<String> {
    match g {
        Gain::Db(0) => None,
        Gain::Db(_) => Some(format!("{:.10}", f64::from(g.linear()))),
        Gain::MinusInfinity => Some("0.0".to_string()),
    }
}

/// Whether two states are the same as far as an ADM block can tell; with
/// real interpolation lengths the ramp is a block field too.
fn adm_equal(a: &ObjectState, b: &ObjectState, real_ramps: bool) -> bool {
    a.active == b.active
        && a.pos == b.pos
        && a.snap == b.snap
        && a.elevation == b.elevation
        && a.zones == b.zones
        && a.uniform_size() == b.uniform_size()
        && a.gain == b.gain
        && (!real_ramps || a.ramp == b.ramp)
}

/// Zone rectangles of a horizontal zone constraint (profile tables 12 and 13).
fn zone_rectangles(
    zones: u8,
) -> Vec<(
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    match zones {
        1 => vec![("-1", "1", "-1", "-0.41934", "-0.499", "0.499", "ZM1")],
        2 => vec![
            (
                "-1", "-0.75806", "-0.41934", "0.83871", "-0.499", "0.499", "ZM2L",
            ),
            (
                "0.75806", "1", "-0.41934", "0.83871", "-0.499", "0.499", "ZM2R",
            ),
        ],
        3 => vec![
            ("-1", "-0.16129", "0.5", "1", "-0.499", "0.499", "ZM3L"),
            (
                "-1", "-0.51611", "-0.707", "0.49999", "-0.499", "0.499", "ZM3Lss",
            ),
            ("0.16129", "1", "0.5", "1", "-0.499", "0.499", "ZM3R"),
            (
                "0.51611", "1", "-0.707", "0.49999", "-0.499", "0.499", "ZM3Rss",
            ),
        ],
        4 => vec![("-1", "1", "-1", "0.83871", "-0.499", "0.499", "ZM4")],
        5 => vec![("-1", "1", "0.5", "1", "-0.499", "0.499", "ZM5")],
        _ => Vec::new(),
    }
}

/// Writes a RIFF chunk with its pad byte.
fn write_chunk(out: &mut impl Write, id: &[u8; 4], payload: &[u8]) -> io::Result<()> {
    out.write_all(id)?;
    out.write_all(&(payload.len() as u32).to_le_bytes())?;
    out.write_all(payload)?;
    if payload.len() % 2 == 1 {
        out.write_all(&[0])?;
    }
    Ok(())
}

/// A string padded with zeros to a fixed width.
fn fixed(s: &str, width: usize) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.resize(width, 0);
    v
}

/// A coordinate with ten decimals, as the reference files carry them.
fn coord(v: f32) -> String {
    format!("{:.10}", f64::from(v))
}

/// `hh:mm:ss.fffff` (five fractional digits) of a sample count.
fn timecode(samples: u64, rate: u32) -> String {
    let total = samples as f64 / f64::from(rate);
    let whole = total.floor() as u64;
    let frac = ((total - whole as f64) * 100_000.0).round() as u64;
    let (whole, frac) = if frac >= 100_000 {
        (whole + 1, 0)
    } else {
        (whole, frac)
    };
    format!(
        "{:02}:{:02}:{:02}.{:05}",
        whole / 3600,
        (whole / 60) % 60,
        whole % 60,
        frac
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecodes_use_five_fraction_digits() {
        assert_eq!(timecode(0, 48000), "00:00:00.00000");
        assert_eq!(timecode(64, 48000), "00:00:00.00133");
        assert_eq!(timecode(5_075_800, 48000), "00:01:45.74583");
        assert_eq!(timecode(2_058_304, 48000), "00:00:42.88133");
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("oadec-adm-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn state() -> ObjectState {
        ObjectState {
            active: true,
            pos: [-1.0, 1.0, 0.0],
            snap: false,
            elevation: true,
            zones: 0,
            size: [0.0; 3],
            importance: 1.0,
            gain: Gain::Db(0),
            ramp: 1536,
            trim_bypass: false,
            screen_factor: 0.0,
            depth_factor: 0.25,
        }
    }

    fn object_event(id: u32, sample_pos: u64, s: ObjectState) -> Event {
        Event {
            id,
            sample_pos,
            state: ElementState::Object(s),
            previous: None,
        }
    }

    fn one_object_writer(dir: &Path, frames: usize) -> AdmWriter {
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 1,
        };
        let mut w =
            AdmWriter::create(&dir.join("t.wav"), &program, 48000, &AdmOptions::default()).unwrap();
        let rows = vec![[0i32, 0]; frames];
        w.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
        w
    }

    /// The profile has no field for a ramp other than 250 samples, an
    /// importance on an active object, a screen reference or a trim bypass;
    /// each is counted once per block it is dropped from.
    #[test]
    fn profile_reductions_are_counted() {
        use crate::loss::LossKind;
        let dir = temp_dir("reductions");
        let mut w = one_object_writer(&dir, 4000);
        let mut first = state();
        first.importance = 0.5;
        first.trim_bypass = true;
        let mut second = state();
        second.pos = [0.5, 1.0, 0.0];
        second.importance = 0.5;
        second.screen_factor = 0.5;
        w.push_event(&object_event(10, 0, first));
        w.push_event(&object_event(10, 2000, second));
        let summary = w.finish().unwrap();
        let l = &summary.losses;
        assert_eq!(
            l.count(LossKind::RampReplaced),
            1,
            "only blocks after the first carry 250"
        );
        assert_eq!(l.ramp_sources().get(&1536), Some(&1));
        assert_eq!(l.count(LossKind::ImportanceOmitted), 2);
        assert_eq!(l.count(LossKind::ScreenReferenceDropped), 1);
        assert_eq!(l.examples(LossKind::ScreenReferenceDropped), &[(10, 2000)]);
        assert_eq!(l.count(LossKind::TrimBypassDropped), 1);
        assert!(!l.declared_loss());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// DirectSpeakers blocks have no time: a bed channel's first state is all
    /// the ADM can carry, and it carries neither its gain nor its active flag.
    #[test]
    fn bed_events_after_the_first_are_counted() {
        use crate::loss::LossKind;
        let dir = temp_dir("bed-events");
        let mut w = one_object_writer(&dir, 4000);
        let bed = |gain: Gain, ramp: u32| BedState {
            active: true,
            importance: 1.0,
            gain,
            ramp,
            trim_bypass: false,
        };
        let bed_event = |sample_pos: u64, s: BedState| Event {
            id: 3,
            sample_pos,
            state: ElementState::Bed(s),
            previous: None,
        };
        w.push_event(&bed_event(0, bed(Gain::Db(-6), 0)));
        w.push_event(&bed_event(1000, bed(Gain::Db(-12), 0)));
        w.push_event(&bed_event(2000, bed(Gain::Db(-12), 32)));
        w.push_event(&object_event(10, 0, state()));
        let summary = w.finish().unwrap();
        assert_eq!(summary.losses.count(LossKind::BedGainDropped), 1);
        assert_eq!(
            summary.losses.count(LossKind::BedEventDropped),
            1,
            "a ramp-only change is not an event the bed could carry"
        );
        assert_eq!(
            summary.losses.examples(LossKind::BedEventDropped),
            &[(3, 1000)]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The channel format of object `k` in the written file.
    fn channel_format(text: &str, k: usize) -> String {
        let start = text
            .find(&format!("audioChannelFormatName=\"Atmos_Obj_{k}\""))
            .expect("object channel format");
        let rest = &text[start..];
        let end = rest.find("</audioChannelFormat>").expect("closing tag");
        rest[..end].to_string()
    }

    fn written_text(dir: &Path) -> String {
        String::from_utf8_lossy(&std::fs::read(dir.join("t.wav")).unwrap()).into_owned()
    }

    /// The Dolby converters write the gain of an active object as a linear
    /// factor with ten decimals of float32 arithmetic, nothing at 0 dB, and
    /// `0.0` alone when the object is muted; the inactive marker keeps both
    /// gain and importance.
    #[test]
    fn active_gain_is_written_as_dolby_prints_it() {
        let dir = temp_dir("gain");
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 4,
        };
        let mut w =
            AdmWriter::create(&dir.join("t.wav"), &program, 48000, &AdmOptions::default()).unwrap();
        let rows = vec![[0i32; 5]; 100];
        w.write_frames(rows.iter().map(|r| &r[..]), 5).unwrap();
        for (k, gain) in [Gain::Db(-6), Gain::Db(3), Gain::MinusInfinity, Gain::Db(0)]
            .into_iter()
            .enumerate()
        {
            let mut s = state();
            s.gain = gain;
            w.push_event(&object_event(10 + k as u32, 0, s));
        }
        w.finish().unwrap();
        let text = written_text(&dir);
        let minus_six = channel_format(&text, 1);
        assert!(
            minus_six.contains("<gain>0.5011872053</gain>"),
            "{minus_six}"
        );
        assert!(!minus_six.contains("<importance>"));
        assert!(channel_format(&text, 2).contains("<gain>1.4125375748</gain>"));
        let muted = channel_format(&text, 3);
        assert!(muted.contains("<gain>0.0</gain>"), "{muted}");
        assert!(
            !muted.contains("<importance>"),
            "a muted active object is not the inactive marker"
        );
        assert!(
            !channel_format(&text, 4).contains("<gain>"),
            "0 dB is written as nothing"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A change of gain alone is an ADM difference now, so it gets its own
    /// block instead of being popped or folded into the previous one.
    #[test]
    fn a_gain_only_change_gets_its_own_block() {
        let dir = temp_dir("gain-change");
        let mut w = one_object_writer(&dir, 96_000);
        w.push_event(&object_event(10, 0, state()));
        let mut quieter = state();
        quieter.gain = Gain::Db(-12);
        w.push_event(&object_event(10, 48_000, quieter));
        let summary = w.finish().unwrap();
        assert_eq!(summary.blocks, 2);
        let cf = channel_format(&written_text(&dir), 1);
        assert!(
            cf.contains("rtime=\"00:00:01.00000\" duration=\"00:00:01.00000\""),
            "{cf}"
        );
        assert!(cf.contains("<gain>0.2511886358</gain>"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The inactive marker (gain 0.0 with importance 0) is untouched, and the
    /// inactive object's own gain is not written on top of it.
    #[test]
    fn the_inactive_marker_is_unchanged() {
        let dir = temp_dir("inactive");
        let mut w = one_object_writer(&dir, 4000);
        let mut off = state();
        off.active = false;
        off.gain = Gain::Db(-6);
        w.push_event(&object_event(10, 0, off));
        w.finish().unwrap();
        let cf = channel_format(&written_text(&dir), 1);
        assert!(
            cf.contains("<gain>0.0</gain>\n\t\t\t\t\t\t<importance>0</importance>"),
            "{cf}"
        );
        assert!(!cf.contains("0.5011872053"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// ISF elements have no representation in a Dolby Atmos master ADM file:
    /// the writer refuses them by default and drops them, counted, on request.
    #[test]
    fn isf_elements_are_refused_by_default_and_dropped_on_request() {
        use crate::loss::LossKind;
        use crate::program::IsfPolicy;
        let dir = temp_dir("isf");
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: Some(0),
            isf_objects: 4,
            dynamic_objects: 2,
        };
        let refused =
            AdmWriter::create(&dir.join("t.wav"), &program, 48000, &AdmOptions::default());
        assert!(
            matches!(
                refused,
                Err(AdmError::IsfNotRepresentable { count: 4, ref isf_type }) if isf_type == "SR3.1.0.0"
            ),
            "{refused:?}"
        );
        let options = AdmOptions {
            isf: IsfPolicy::Drop,
            ..AdmOptions::default()
        };
        let mut w = AdmWriter::create(&dir.join("t.wav"), &program, 48000, &options).unwrap();
        assert_eq!(
            w.channels(),
            12,
            "ten bed tracks and the two dynamic objects"
        );
        let rows = vec![[0i32; 7]; 10];
        w.write_frames(rows.iter().map(|r| &r[..]), 7).unwrap();
        w.push_event(&object_event(10, 0, state()));
        let summary = w.finish().unwrap();
        assert_eq!(summary.losses.count(LossKind::IsfDropped), 4);
        assert!(summary.losses.declared_loss());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The profile requires width, depth and height to be identical, so the
    /// width is written for all three; axes that differ are counted, not lost
    /// in silence. A change of depth or height alone is not a new block.
    #[test]
    fn size_axes_that_differ_are_counted_and_the_width_is_written() {
        use crate::loss::LossKind;
        let dir = temp_dir("size-axes");
        let mut w = one_object_writer(&dir, 4000);
        let mut boxy = state();
        boxy.size = [0.2, 0.5, 0.8];
        w.push_event(&object_event(10, 0, boxy));
        let mut taller = state();
        taller.size = [0.2, 0.5, 0.9];
        w.push_event(&object_event(10, 2000, taller));
        let summary = w.finish().unwrap();
        assert_eq!(
            summary.blocks, 1,
            "a height-only change is not an ADM difference"
        );
        assert_eq!(summary.losses.count(LossKind::SizeAxesCollapsed), 2);
        assert_eq!(
            summary.losses.examples(LossKind::SizeAxesCollapsed),
            &[(10, 0), (10, 2000)]
        );
        let cf = channel_format(&written_text(&dir), 1);
        assert!(
            cf.contains("<width>0.2000000030</width>\n\t\t\t\t\t\t<depth>0.2000000030</depth>\n\t\t\t\t\t\t<height>0.2000000030</height>"),
            "{cf}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Profile table 23 requires 48 000 Hz. Anything else is refused unless the
    /// caller asks for a file outside the profile, which is then a declared
    /// loss; the 250-sample interpolation length scales with the rate.
    #[test]
    fn a_non_profile_sample_rate_is_refused_unless_allowed() {
        use crate::loss::LossKind;
        let dir = temp_dir("rate");
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 1,
        };
        let refused =
            AdmWriter::create(&dir.join("t.wav"), &program, 96_000, &AdmOptions::default());
        assert!(
            matches!(refused, Err(AdmError::NonProfileSampleRate(96_000))),
            "{refused:?}"
        );
        let options = AdmOptions {
            allow_non_profile_rate: true,
            ..AdmOptions::default()
        };
        let mut w = AdmWriter::create(&dir.join("t.wav"), &program, 96_000, &options).unwrap();
        let rows = vec![[0i32, 0]; 8000];
        w.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
        w.push_event(&object_event(10, 0, state()));
        let mut moved = state();
        moved.pos = [0.5, 1.0, 0.0];
        w.push_event(&object_event(10, 4000, moved));
        let summary = w.finish().unwrap();
        assert_eq!(summary.losses.count(LossKind::NonProfileSampleRate), 1);
        assert!(summary.losses.declared_loss());
        let cf = channel_format(&written_text(&dir), 1);
        assert!(cf.contains("interpolationLength=\"0.002604\""), "{cf}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Profile table 17 numbers the objects AO_100b.. regardless of the bed, as
    /// Dolby's converters do; the old formula counted on from the bed size.
    #[test]
    fn objects_are_numbered_from_ao_100b_whatever_the_bed() {
        let dir = temp_dir("ids");
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 2,
        };
        let options = AdmOptions {
            bed_conform: false,
            ..AdmOptions::default()
        };
        let mut w = AdmWriter::create(&dir.join("t.wav"), &program, 48000, &options).unwrap();
        assert_eq!(w.channels(), 3);
        let rows = [[0i32; 3]; 10];
        w.write_frames(rows.iter().map(|r| &r[..]), 3).unwrap();
        w.push_event(&object_event(10, 0, state()));
        w.push_event(&object_event(11, 0, state()));
        w.finish().unwrap();
        let text = written_text(&dir);
        assert!(text.contains("<audioObjectIDRef>AO_1001</audioObjectIDRef>"));
        assert!(text.contains("<audioObjectIDRef>AO_100b</audioObjectIDRef>"));
        assert!(
            text.contains("<audioObject audioObjectID=\"AO_100b\" audioObjectName=\"Atmos_Obj_1\"")
        );
        assert!(
            text.contains("<audioObject audioObjectID=\"AO_100c\" audioObjectName=\"Atmos_Obj_2\"")
        );
        assert!(
            !text.contains("AO_1002"),
            "the bed size no longer shifts the object ids"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Table 17 leaves room for 118 objects (AO_100b..AO_1080) and the profile
    /// for 128 tracks; more is refused rather than numbered out of range.
    #[test]
    fn more_than_118_objects_is_an_error() {
        let dir = temp_dir("limit");
        let mut program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 118,
        };
        assert!(
            AdmWriter::create(&dir.join("t.wav"), &program, 48000, &AdmOptions::default()).is_ok(),
            "118 objects and the ten-channel bed are exactly the 128 tracks allowed"
        );
        program.dynamic_objects = 119;
        let refused =
            AdmWriter::create(&dir.join("t.wav"), &program, 48000, &AdmOptions::default());
        assert!(
            matches!(
                refused,
                Err(AdmError::TooManyObjects {
                    objects: 119,
                    max: 118
                })
            ),
            "{refused:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The profile has labels for the 7.1.2 set only; wide and second-LFE
    /// channels are refused before anything is written, which also keeps the
    /// dbmd bed-mask bit they would share out of reach.
    #[test]
    fn unsupported_wide_and_second_lfe_channels_are_refused() {
        let dir = temp_dir("wide");
        for ch in [
            BedChannel::Lw,
            BedChannel::Rw,
            BedChannel::LFE2,
            BedChannel::Tfl,
        ] {
            let program = Program {
                beds: vec![vec![BedChannel::L, BedChannel::R, ch]],
                isf_index: None,
                isf_objects: 0,
                dynamic_objects: 1,
            };
            let options = AdmOptions {
                bed_conform: false,
                ..AdmOptions::default()
            };
            let refused = AdmWriter::create(&dir.join("t.wav"), &program, 48000, &options);
            assert!(
                matches!(refused, Err(AdmError::UnsupportedBedChannel(c)) if c == ch),
                "{refused:?}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The `rtime`/`duration` pairs of object `k`'s blocks, in file order.
    fn block_times(text: &str, k: usize) -> Vec<(String, String)> {
        let cf = channel_format(text, k);
        cf.match_indices("rtime=\"")
            .map(|(i, _)| {
                let rest = &cf[i + 7..];
                let rtime = &rest[..rest.find('"').unwrap()];
                let d = rest.find("duration=\"").unwrap() + 10;
                let dur = &rest[d..d + rest[d..].find('"').unwrap()];
                (rtime.to_string(), dur.to_string())
            })
            .collect()
    }

    /// Events are written in time order whatever order they arrived in, an
    /// event at or after the end is dropped, and the last block ends exactly
    /// at the programme end (Dolby's converters do the same: audit C07, C09).
    #[test]
    fn events_are_sorted_and_the_last_block_ends_at_the_programme_end() {
        use crate::loss::LossKind;
        let dir = temp_dir("tiling");
        let mut w = one_object_writer(&dir, 96_000);
        let at = |x: f32| {
            let mut s = state();
            s.pos = [x, 1.0, 0.0];
            s
        };
        w.push_event(&object_event(10, 0, at(-1.0)));
        w.push_event(&object_event(10, 48_000, at(0.0)));
        w.push_event(&object_event(10, 24_000, at(1.0)));
        w.push_event(&object_event(10, 96_000, at(0.5)));
        w.push_event(&object_event(10, 96_040, at(0.7)));
        let summary = w.finish().unwrap();
        assert_eq!(summary.blocks, 3);
        assert_eq!(summary.losses.count(LossKind::EventBeyondEndDropped), 2);
        assert_eq!(
            summary.losses.examples(LossKind::EventBeyondEndDropped),
            &[(10, 96_000), (10, 96_040)]
        );
        let text = written_text(&dir);
        assert_eq!(
            block_times(&text, 1),
            vec![
                ("00:00:00.00000".to_string(), "00:00:00.50000".to_string()),
                ("00:00:00.50000".to_string(), "00:00:00.50000".to_string()),
                ("00:00:01.00000".to_string(), "00:00:01.00000".to_string()),
            ]
        );
        let cf = channel_format(&text, 1);
        let x1 = cf.find("<position coordinate=\"X\">1.0000000000").unwrap();
        let x0 = cf.find("<position coordinate=\"X\">0.0000000000").unwrap();
        assert!(x1 < x0, "the 24000 state comes before the 48000 state");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An object whose first event arrives after sample 0 is held from 0 in
    /// its first state, and the real event keeps its own block so that its
    /// arrival time stays in the file (the audit's C06 lost it).
    #[test]
    fn a_late_first_event_keeps_its_own_block() {
        use crate::loss::LossKind;
        let dir = temp_dir("late-first");
        let mut w = one_object_writer(&dir, 192_000);
        w.push_event(&object_event(10, 96_000, state()));
        let summary = w.finish().unwrap();
        assert_eq!(summary.blocks, 2);
        assert_eq!(summary.losses.count(LossKind::LateFirstEventHeld), 1);
        assert_eq!(
            summary.losses.examples(LossKind::LateFirstEventHeld),
            &[(10, 96_000)]
        );
        assert_eq!(
            block_times(&written_text(&dir), 1),
            vec![
                ("00:00:00.00000".to_string(), "00:00:02.00000".to_string()),
                ("00:00:02.00000".to_string(), "00:00:02.00000".to_string()),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two events at one sample position: the last one is the block, counted.
    #[test]
    fn two_events_at_one_sample_keep_the_last() {
        use crate::loss::LossKind;
        let dir = temp_dir("same-pos");
        let mut w = one_object_writer(&dir, 4000);
        let at = |x: f32| {
            let mut s = state();
            s.pos = [x, 1.0, 0.0];
            s
        };
        w.push_event(&object_event(10, 0, at(-1.0)));
        w.push_event(&object_event(10, 1536, at(0.0)));
        w.push_event(&object_event(10, 1536, at(1.0)));
        let summary = w.finish().unwrap();
        assert_eq!(summary.blocks, 2);
        assert_eq!(summary.losses.count(LossKind::SamePositionSuperseded), 1);
        assert_eq!(
            summary.losses.examples(LossKind::SamePositionSuperseded),
            &[(10, 1536)]
        );
        let cf = channel_format(&written_text(&dir), 1);
        assert!(cf.contains("<position coordinate=\"X\">1.0000000000"));
        assert!(!cf.contains("<position coordinate=\"X\">0.0000000000"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `Interpolation::Real` writes the source ramp as the interpolation
    /// length (ten decimals, so 32 samples round-trip exactly), keeps
    /// ramp-only changes as blocks, counts no replaced ramps, and marks the
    /// file as outside the profile. It is opt-in.
    #[test]
    fn real_interpolation_writes_the_ramp_and_keeps_ramp_only_changes() {
        use crate::loss::LossKind;
        let dir = temp_dir("real-ramp");
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 1,
        };
        let options = AdmOptions {
            interpolation: Interpolation::Real,
            ..AdmOptions::default()
        };
        let mut w = AdmWriter::create(&dir.join("t.wav"), &program, 48000, &options).unwrap();
        let rows = vec![[0i32, 0]; 96_000];
        w.write_frames(rows.iter().map(|r| &r[..]), 2).unwrap();
        let mut first = state();
        first.ramp = 32;
        w.push_event(&object_event(10, 0, first));
        let mut moved = state();
        moved.pos = [0.5, 1.0, 0.0];
        moved.ramp = 1536;
        w.push_event(&object_event(10, 24_000, moved.clone()));
        let mut slower = moved;
        slower.ramp = 32;
        w.push_event(&object_event(10, 48_000, slower));
        let summary = w.finish().unwrap();
        assert_eq!(
            summary.blocks, 3,
            "a ramp-only change is a block in real mode"
        );
        assert_eq!(summary.losses.count(LossKind::RampReplaced), 0);
        let text = written_text(&dir);
        let cf = channel_format(&text, 1);
        assert!(
            cf.contains("interpolationLength=\"0.0000000000\""),
            "the first block has nothing to interpolate from: {cf}"
        );
        assert!(cf.contains("interpolationLength=\"0.0320000000\""), "{cf}");
        assert!(cf.contains("interpolationLength=\"0.0006666667\""), "{cf}");
        assert!(
            text.contains("non-profile: real interpolation lengths"),
            "the dbmd tool string marks the file"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn profile_interpolation_is_the_default() {
        assert_eq!(AdmOptions::default().interpolation, Interpolation::Profile);
    }

    #[test]
    fn writes_a_profile_shaped_file() {
        let dir = std::env::temp_dir().join(format!("oadec-adm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.wav");
        let program = Program {
            beds: vec![vec![BedChannel::LFE]],
            isf_index: None,
            isf_objects: 0,
            dynamic_objects: 2,
        };
        let mut w = AdmWriter::create(&path, &program, 48000, &AdmOptions::default()).unwrap();
        assert_eq!(w.channels(), 12);
        let rows = [[7i32, 8, 9], [1, 2, 3]];
        w.write_frames(rows.iter().map(|r| &r[..]), 3).unwrap();
        let state = |x: f32, active: bool| ObjectState {
            active,
            pos: [x, 1.0, 0.0],
            snap: false,
            elevation: true,
            zones: 0,
            size: [0.0; 3],
            importance: 1.0,
            gain: Gain::Db(0),
            ramp: 1536,
            trim_bypass: false,
            screen_factor: 0.0,
            depth_factor: 0.25,
        };
        w.push_event(&Event {
            id: 10,
            sample_pos: 0,
            state: ElementState::Object(state(-1.0, true)),
            previous: None,
        });
        w.push_event(&Event {
            id: 10,
            sample_pos: 1,
            state: ElementState::Object(state(0.5, false)),
            previous: None,
        });
        let summary = w.finish().unwrap();
        assert_eq!(summary.frames, 2);
        assert_eq!(summary.channels, 12);
        assert_eq!(
            summary.blocks, 3,
            "two blocks for object 1, one held block for object 2"
        );
        assert!(!summary.rf64);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[12..16], b"JUNK");
        assert_eq!(
            &bytes[DATA_SIZE_POS as usize - 4..DATA_SIZE_POS as usize],
            b"data"
        );
        assert_eq!(
            u32::from_le_bytes(
                bytes[DATA_SIZE_POS as usize..DATA_SIZE_POS as usize + 4]
                    .try_into()
                    .unwrap()
            ),
            2 * 12 * 3
        );
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("audioProgrammeID=\"APR_1001\""));
        assert!(
            text.contains("<audioObject audioObjectID=\"AO_100b\" audioObjectName=\"Atmos_Obj_1\"")
        );
        assert!(text.contains("audioChannelFormatName=\"RoomCentricLFE\""));
        assert!(text.contains("<speakerLabel>RC_Lts</speakerLabel>"));
        assert!(text.contains("<gain>0.0</gain>"));
        assert!(text.contains("interpolationLength=\"0.005208\""));
        assert!(text.contains("chna"));
        assert!(text.contains("dbmd"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
