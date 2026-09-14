//! `oadec emdf`: walk the frames of an E-AC-3 elementary stream, find the EMDF
//! containers in their skip fields and report the Object Audio Metadata timing
//! they carry.
//!
//! The frames are walked by their sync words and sizes and each one is parsed
//! far enough to reach its skip fields, which is where the containers are; no
//! audio comes out. It settles where the encoder places metadata relative to
//! the 1536-sample frames. The walk ([`for_each_container`]) is shared with
//! `oadec oamd`, which reads the same containers for what the objects carry.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use oadec_bits::BitReader;
use oadec_eac3::frame::{Frame, Noise, Options as FrameOptions};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::oamd::Oamd;
use serde::Serialize;

/// Options of the command.
#[derive(Debug, Clone)]
pub struct Options {
    pub json: bool,
    pub dump: Option<usize>,
}

/// Samples per E-AC-3 audio block.
const BLOCK_SAMPLES: u64 = 256;

/// Summary of the scan.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EmdfSummary {
    pub frames: u64,
    pub independent_frames: u64,
    pub dependent_frames: u64,
    pub bytes: u64,
    pub sync_errors: u64,
    /// Frames the parser could not read far enough to reach the skip fields.
    pub unparsed_frames: u64,
    pub frames_with_emdf: u64,
    pub containers: u64,
    pub container_errors: u64,
    pub payload_ids: BTreeMap<u32, u64>,
    pub oamd_payloads: u64,
    pub oamd_errors: u64,
    pub joc_payloads: u64,
    pub oamd_smploffst: BTreeMap<u32, u64>,
    pub oamd_sample_offsets: BTreeMap<u16, u64>,
    pub oamd_block_offsets: BTreeMap<u8, u64>,
    pub oamd_ramps: BTreeMap<u16, u64>,
    /// Derived event start times (frame start + smploffst + sample_offset +
    /// 32 * block_offset_factor) modulo 1536, with counts.
    pub event_times_mod_frame: BTreeMap<u64, u64>,
    pub first_error: Option<String>,
    /// Sample position of the frame start of every OAMD-carrying frame that
    /// changes at least one value, in order (first 64 only in text mode).
    #[serde(skip)]
    pub event_times: Vec<u64>,
}

/// E-AC-3 frame header fields the scan needs.
struct FrameHead {
    strmtyp: u8,
    substreamid: u8,
    frmsiz: usize,
    numblks: u8,
}

fn parse_head(bytes: &[u8]) -> Option<FrameHead> {
    if bytes.len() < 6 || bytes[0] != 0x0B || bytes[1] != 0x77 {
        return None;
    }
    let mut r = BitReader::new(&bytes[2..]);
    let strmtyp = r.read(2).ok()? as u8;
    let substreamid = r.read(3).ok()? as u8;
    let frmsiz = r.read(11).ok()? as usize;
    let fscod = r.read(2).ok()? as u8;
    let numblkscod = r.read(2).ok()? as u8;
    let numblks = if fscod == 3 {
        6
    } else {
        [1, 2, 3, 6][usize::from(numblkscod)]
    };
    Some(FrameHead {
        strmtyp,
        substreamid,
        frmsiz,
        numblks,
    })
}

/// A container as the walk found it in a skip field: opened, or the error
/// that kept it closed.
pub(crate) type ContainerResult =
    std::result::Result<container::Container, container::ContainerError>;

/// The EMDF containers of one frame.
///
/// The containers live in the skip fields of the audio blocks, which start at
/// a bit offset the frame's own syntax decides, so hunting for the sync word
/// in the raw frame bytes finds only the containers that happen to land on a
/// byte boundary and calls the rest errors. On the streams here that missed
/// half of them and invented twenty false errors. The frame is parsed instead,
/// and the sync word is looked for in the skip fields, where it is
/// byte-aligned by construction.
fn find_emdf(frame: &[u8], noise: &mut Noise, unparsed: &mut u64) -> Vec<(usize, ContainerResult)> {
    let mut out = Vec::new();
    let opts = FrameOptions {
        dither: false,
        ..FrameOptions::default()
    };
    let Ok(parsed) = Frame::parse(frame, noise, opts) else {
        *unparsed += 1;
        return out;
    };
    let total: usize = parsed.skip_fields.iter().map(Vec::len).sum();
    if total == 0 {
        return out;
    }
    let mut data = Vec::with_capacity(total);
    for s in &parsed.skip_fields {
        data.extend_from_slice(s);
    }
    let mut i = 0;
    let mut read_one = false;
    while i + 4 <= data.len() {
        if data[i] == 0x58 && data[i + 1] == 0x38 {
            match container::parse_emdf_with_sync(&data[i..]) {
                Ok((c, used)) => {
                    out.push((i, Ok(c)));
                    read_one = true;
                    i += used.max(4);
                    continue;
                }
                // the bytes after the last container are padding and can
                // carry the sync word by chance; only a sync word before any
                // container has been read is a container we failed to open
                Err(e) if !read_one => out.push((i, Err(e))),
                Err(_) => {}
            }
        }
        i += 1;
    }
    out
}

/// Where the walk found a container.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Site {
    /// The syncframe, counted from 0 over every substream.
    pub frame_index: u64,
    /// The first sample of the independent frame the syncframe belongs to.
    pub sample_pos: u64,
    pub substream_id: u8,
    /// Whether the syncframe belongs to a dependent substream.
    pub dependent: bool,
    /// The byte of the frame's joined skip fields the container starts at.
    pub offset: usize,
}

/// What the walk counted besides the containers it handed out.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Walk {
    pub frames: u64,
    pub independent_frames: u64,
    pub dependent_frames: u64,
    pub bytes: u64,
    pub sync_errors: u64,
    /// Frames the parser could not read far enough to reach the skip fields.
    pub unparsed_frames: u64,
    /// Frames holding at least one container, opened or not.
    pub frames_with_emdf: u64,
}

/// Walks the syncframes of the E-AC-3 stream at `path` and hands
/// `on_container` every EMDF container of their skip fields, opened or not,
/// in stream order.
///
/// `emdf` reads the containers for when their metadata applies and `oamd` for
/// what the objects carry. Both walk the stream here, so the two commands
/// count the same frames and see the same payloads.
pub(crate) fn for_each_container(
    path: &Path,
    mut on_container: impl FnMut(&Site, ContainerResult),
) -> Result<Walk> {
    let mut data = Vec::new();
    File::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .read_to_end(&mut data)?;
    let mut walk = Walk {
        bytes: data.len() as u64,
        ..Walk::default()
    };
    let mut pos = 0usize;
    let mut noise = Noise::default();
    let mut sample_pos: u64 = 0; // first sample of the current independent frame
    let mut last_frame_len: u64 = 0;
    while pos + 6 <= data.len() {
        let Some(head) = parse_head(&data[pos..]) else {
            walk.sync_errors += 1;
            // resync on the next 0B 77
            match data[pos + 1..].windows(2).position(|w| w == [0x0B, 0x77]) {
                Some(k) => {
                    pos += 1 + k;
                    continue;
                }
                None => break,
            }
        };
        let len = (head.frmsiz + 1) * 2;
        if pos + len > data.len() {
            break;
        }
        let frame = &data[pos..pos + len];
        walk.frames += 1;
        if head.strmtyp == 1 {
            walk.dependent_frames += 1;
        } else {
            walk.independent_frames += 1;
            if walk.frames > 1 {
                sample_pos += last_frame_len;
            }
            last_frame_len = u64::from(head.numblks) * BLOCK_SAMPLES;
        }
        let containers = find_emdf(frame, &mut noise, &mut walk.unparsed_frames);
        if !containers.is_empty() {
            walk.frames_with_emdf += 1;
        }
        for (offset, c) in containers {
            let site = Site {
                frame_index: walk.frames - 1,
                sample_pos,
                substream_id: head.substreamid,
                dependent: head.strmtyp == 1,
                offset,
            };
            on_container(&site, c);
        }
        pos += len;
    }
    Ok(walk)
}

/// Runs the command; returns `true` when every container and payload parsed.
pub fn run(path: &Path, opts: &Options) -> Result<bool> {
    // the containers are in E-AC-3 skip fields; a TrueHD stream has none and
    // walking it for E-AC-3 sync words would report nothing but sync errors
    if !crate::eac3::is_eac3(path).unwrap_or(false) {
        anyhow::bail!(
            concat!(
                "{} is not an E-AC-3 stream. TrueHD carries its object metadata ",
                "in the access units instead; use `oadec oamd` for that."
            ),
            path.display()
        );
    }
    let started = Instant::now();
    let mut s = EmdfSummary::default();
    let mut dumped = 0usize;
    let walk = for_each_container(path, |site, c| {
        let &Site {
            frame_index,
            sample_pos,
            substream_id,
            dependent,
            offset,
        } = site;
        match c {
            Err(e) => {
                s.container_errors += 1;
                if s.first_error.is_none() {
                    s.first_error = Some(format!(
                        "frame {frame_index}, skip-field byte {offset}: {e}"
                    ));
                }
            }
            Ok(c) => {
                s.containers += 1;
                if opts.dump.is_some_and(|n| dumped < n) {
                    // the container's own header, which nothing else
                    // reports and which two streams can differ in while
                    // every field above them matches
                    println!(
                        "  container: version {}, key_id {}, protection {:?}, {} payloads {:?}",
                        c.version,
                        c.key_id,
                        c.protection,
                        c.payloads.len(),
                        c.payloads
                            .iter()
                            .map(|p| (
                                p.id,
                                p.data.len(),
                                p.config.sample_offset,
                                p.config.duration,
                                p.config.group_id,
                                p.config.discard_unknown_payload,
                                p.config.payload_frame_aligned,
                                p.config.create_duplicate,
                            ))
                            .collect::<Vec<_>>()
                    );
                    for p in &c.payloads {
                        if p.data.len() <= 8 {
                            println!("    payload {} = {:02x?}", p.id, p.data);
                        }
                    }
                }
                for p in &c.payloads {
                    *s.payload_ids.entry(p.id).or_default() += 1;
                    if p.id == PAYLOAD_ID_JOC {
                        s.joc_payloads += 1;
                    }
                    if p.id != PAYLOAD_ID_OAMD {
                        continue;
                    }
                    let smploffst = p.config.sample_offset.unwrap_or(0);
                    *s.oamd_smploffst.entry(smploffst).or_default() += 1;
                    match Oamd::parse(&p.data) {
                        Ok(oamd) => {
                            s.oamd_payloads += 1;
                            if let Some(o) = oamd.object_element() {
                                *s.oamd_sample_offsets
                                    .entry(o.timing.sample_offset)
                                    .or_default() += 1;
                                for b in &o.timing.blocks {
                                    *s.oamd_block_offsets
                                        .entry(b.block_offset_factor)
                                        .or_default() += 1;
                                    *s.oamd_ramps.entry(b.ramp_duration).or_default() += 1;
                                    let t = sample_pos
                                        + u64::from(smploffst)
                                        + u64::from(o.timing.sample_offset)
                                        + u64::from(b.block_offset_factor) * 32;
                                    *s.event_times_mod_frame.entry(t % 1536).or_default() += 1;
                                    s.event_times.push(t);
                                }
                                if opts.dump.is_some_and(|n| dumped < n) {
                                    dumped += 1;
                                    println!(
                                        "frame {frame_index} (sample {sample_pos}, substream {substream_id}{}): EMDF at byte {offset}, smploffst {smploffst}, OAMD {} objects, sample_offset {}, blocks {:?}",
                                        if dependent { " dependent" } else { "" },
                                        oamd.object_count,
                                        o.timing.sample_offset,
                                        o.timing
                                            .blocks
                                            .iter()
                                            .map(|b| (b.block_offset_factor, b.ramp_duration))
                                            .collect::<Vec<_>>()
                                    );
                                    for (i, updates) in o.objects.iter().enumerate() {
                                        let u = &updates[0];
                                        let p = u.render.position([0; 3]);
                                        println!(
                                            "    obj {i}: {}gain {:?} pos ({:.3}, {:.3}, {:.3}) size {:.2}",
                                            if u.in_bed_or_isf { "bed " } else { "" },
                                            u.basic.gain,
                                            p[0],
                                            p[1],
                                            p[2],
                                            u.render.size[0]
                                        );
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            s.oamd_errors += 1;
                            if s.first_error.is_none() {
                                s.first_error = Some(format!("frame {frame_index}: OAMD: {e}"));
                            }
                        }
                    }
                }
            }
        }
    })?;
    if walk.frames == 0 {
        return Err(crate::info::no_stream(path));
    }
    s.frames = walk.frames;
    s.independent_frames = walk.independent_frames;
    s.dependent_frames = walk.dependent_frames;
    s.bytes = walk.bytes;
    s.sync_errors = walk.sync_errors;
    s.unparsed_frames = walk.unparsed_frames;
    s.frames_with_emdf = walk.frames_with_emdf;
    let elapsed = started.elapsed().as_secs_f64();
    let clean = s.sync_errors == 0
        && s.unparsed_frames == 0
        && s.container_errors == 0
        && s.oamd_errors == 0;
    if opts.json {
        let mut value = serde_json::to_value(&s)?;
        value["clean"] = serde_json::json!(clean);
        value["seconds"] = serde_json::json!(elapsed);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!(
            "Frames:            {} ({} independent, {} dependent), {} bytes, {} sync errors, {} unparsed",
            s.frames,
            s.independent_frames,
            s.dependent_frames,
            s.bytes,
            s.sync_errors,
            s.unparsed_frames
        );
        println!(
            "EMDF:              {} frames with containers, {} containers, {} container errors, payload ids {:?}",
            s.frames_with_emdf, s.containers, s.container_errors, s.payload_ids
        );
        println!(
            "OAMD:              {} payloads ({} errors), {} JOC payloads",
            s.oamd_payloads, s.oamd_errors, s.joc_payloads
        );
        println!("smploffst:         {:?}", s.oamd_smploffst);
        println!("Sample offsets:    {:?}", s.oamd_sample_offsets);
        println!("Block offsets:     {:?}", s.oamd_block_offsets);
        println!("Ramp durations:    {:?}", s.oamd_ramps);
        println!("Event time mod 1536: {:?}", s.event_times_mod_frame);
        println!(
            "Event times:       {:?}{}",
            &s.event_times[..s.event_times.len().min(24)],
            if s.event_times.len() > 24 { " ..." } else { "" }
        );
        if let Some(e) = &s.first_error {
            println!("First problem:     {e}");
        }
        println!("Speed:             {elapsed:.2} s");
        println!(
            "Result:            {}",
            if clean { "CLEAN" } else { "PROBLEMS" }
        );
    }
    Ok(clean)
}
