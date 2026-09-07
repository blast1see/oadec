//! `oadec emdf`: walk the frames of an E-AC-3 elementary stream, find the EMDF
//! containers in their skip fields and report the Object Audio Metadata timing
//! they carry.
//!
//! This is a frame-level scan (sync word, frame size, block count), not the
//! core decoder; it settles where the encoder places metadata relative to the
//! 1536-sample frames.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use oadec_bits::BitReader;
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

/// Finds byte-aligned EMDF sync words in a frame and returns the containers.
fn find_emdf(
    frame: &[u8],
) -> Vec<(
    usize,
    std::result::Result<container::Container, container::ContainerError>,
)> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 4 <= frame.len() {
        if frame[i] == 0x58 && frame[i + 1] == 0x38 {
            match container::parse_emdf_with_sync(&frame[i..]) {
                Ok((c, used)) => {
                    let len = used.max(4);
                    out.push((i, Ok(c)));
                    i += len;
                    continue;
                }
                Err(e) => {
                    // a false sync inside audio data is common; only keep errors that
                    // look like a real header (a plausible length field)
                    if frame.len() - i > 16 {
                        out.push((i, Err(e)));
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// Runs the command; returns `true` when every container and payload parsed.
pub fn run(path: &Path, opts: &Options) -> Result<bool> {
    let started = Instant::now();
    let mut data = Vec::new();
    File::open(path)
        .with_context(|| format!("opening {}", path.display()))?
        .read_to_end(&mut data)?;
    let mut s = EmdfSummary {
        bytes: data.len() as u64,
        ..EmdfSummary::default()
    };
    let mut pos = 0usize;
    let mut sample_pos: u64 = 0; // first sample of the current independent frame
    let mut dumped = 0usize;
    let mut last_frame_len: u64 = 0;
    while pos + 6 <= data.len() {
        let Some(head) = parse_head(&data[pos..]) else {
            s.sync_errors += 1;
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
        s.frames += 1;
        if head.strmtyp == 1 {
            s.dependent_frames += 1;
        } else {
            s.independent_frames += 1;
            if s.frames > 1 {
                sample_pos += last_frame_len;
            }
            last_frame_len = u64::from(head.numblks) * BLOCK_SAMPLES;
        }
        let frame_index = s.frames - 1;
        let containers = find_emdf(frame);
        if !containers.is_empty() {
            s.frames_with_emdf += 1;
        }
        for (offset, c) in containers {
            match c {
                Err(e) => {
                    s.container_errors += 1;
                    if s.first_error.is_none() {
                        s.first_error = Some(format!("frame {frame_index} byte {offset}: {e}"));
                    }
                }
                Ok(c) => {
                    s.containers += 1;
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
                                            "frame {frame_index} (sample {sample_pos}, substream {}{}): EMDF at byte {offset}, smploffst {smploffst}, OAMD {} objects, sample_offset {}, blocks {:?}",
                                            head.substreamid,
                                            if head.strmtyp == 1 { " dependent" } else { "" },
                                            oamd.object_count,
                                            o.timing.sample_offset,
                                            o.timing
                                                .blocks
                                                .iter()
                                                .map(|b| (b.block_offset_factor, b.ramp_duration))
                                                .collect::<Vec<_>>()
                                        );
                                        for (i, updates) in o.objects.iter().enumerate().take(4) {
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
        }
        pos += len;
    }
    let elapsed = started.elapsed().as_secs_f64();
    let clean = s.sync_errors == 0 && s.container_errors == 0 && s.oamd_errors == 0;
    if opts.json {
        let mut value = serde_json::to_value(&s)?;
        value["clean"] = serde_json::json!(clean);
        value["seconds"] = serde_json::json!(elapsed);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!(
            "Frames:            {} ({} independent, {} dependent), {} bytes, {} sync errors",
            s.frames, s.independent_frames, s.dependent_frames, s.bytes, s.sync_errors
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
