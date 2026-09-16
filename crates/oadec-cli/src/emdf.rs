//! `oadec emdf`: walk the frames of an E-AC-3 elementary stream, find the EMDF
//! containers in their skip fields and report the Object Audio Metadata timing
//! they carry.
//!
//! The frames are found the way `info` and `verify` find them, by their sync
//! words and their own headers, AC-3 and E-AC-3 alike, and each one is parsed
//! far enough to reach its skip fields, which is where the containers are; no
//! audio comes out. It settles where the encoder places metadata relative to
//! the 1536-sample frames. The walk ([`for_each_container`]) is shared with
//! `oadec oamd`, which reads the same containers for what the objects carry.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use oadec_eac3::StreamType;
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
    /// Bytes the framing skipped: before the first syncframe, where sync was
    /// lost, and a last syncframe cut short. `verify` counts them as faults.
    pub skipped_bytes: u64,
    /// Syncframes whose CRC failed; `verify` counts them as faults.
    pub crc_failures: u64,
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
/// Returns the containers of the frame and whether its skip fields held any data
/// at all.
fn find_emdf(
    frame: &[u8],
    noise: &mut Noise,
    unparsed: &mut u64,
    crc_failures: &mut u64,
) -> (Vec<(usize, ContainerResult)>, bool, bool) {
    let mut out = Vec::new();
    let opts = FrameOptions {
        dither: false,
        ..FrameOptions::default()
    };
    let Ok(parsed) = Frame::parse(frame, noise, opts) else {
        *unparsed += 1;
        return (out, false, false);
    };
    if !parsed.crc_ok {
        *crc_failures += 1;
    }
    // TS 103 420 clause 8.3.1: the extension is declared in the same substream
    // as the container it rides in
    let declares_joc = parsed
        .bsi
        .joc_extension()
        .is_some_and(|(present, _)| present);
    let total: usize = parsed.skip_fields.iter().map(Vec::len).sum();
    if total == 0 {
        return (out, false, declares_joc);
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
    // A frame carries one container (TS 103 420 clause 8.2): a candidate that
    // failed before one that opened was a false sync, as `verify` reads it, and a
    // frame in which none opened reports its first failure once.
    if read_one {
        out.retain(|(_, c)| c.is_ok());
    } else {
        out.truncate(1);
    }
    (out, true, declares_joc)
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
#[derive(Debug, Clone, Default)]
pub(crate) struct Walk {
    pub frames: u64,
    pub independent_frames: u64,
    pub dependent_frames: u64,
    pub bytes: u64,
    pub sync_errors: u64,
    /// Frames the parser could not read far enough to reach the skip fields.
    pub unparsed_frames: u64,
    /// Bytes the framing skipped, a last syncframe cut short included.
    pub skipped_bytes: u64,
    /// Syncframes that parsed and whose CRC failed.
    pub crc_failures: u64,
    /// Frames holding at least one container, opened or not.
    pub frames_with_emdf: u64,
    /// Containers that opened.
    pub containers: u64,
    /// Frames whose skip fields held data and no container that opens, erased
    /// or broken, in a substream in which containers do open: those frames lost
    /// their metadata. Skip fields may carry other data, so a substream that
    /// carries no EMDF at all adds nothing; the AC-3 core of a configuration 4
    /// stream fills them while its dependent substream carries the containers.
    /// A substream whose frames declare a JOC extension carries EMDF as well,
    /// even when not one of its containers opens.
    pub missing_containers: u64,
    /// The first of them, in the words the reports use.
    pub first_missing: Option<String>,
}

/// What the walk saw of the containers of one substream.
#[derive(Debug, Default)]
struct SubstreamContainers {
    /// Containers that opened.
    opened: u64,
    /// Frames with skip data and no container that opens.
    without: u64,
    /// The first of those frames, and the words that name it.
    first_without: Option<(u64, String)>,
    /// A frame of the substream declared a JOC extension in its `addbsi`. That
    /// extension rides in an EMDF container (TS 103 420 clause 8.3.1), so the
    /// substream carries EMDF even when not one container opens.
    declares_joc: bool,
}

/// Walks the syncframes of the AC-3 or E-AC-3 stream at `path` and hands
/// `on_container` every EMDF container of their skip fields, opened or not,
/// in stream order.
///
/// `emdf` reads the containers for when their metadata applies and `oamd` for
/// what the objects carry. Both walk the stream here, so the two commands
/// count the same frames and see the same payloads.
///
/// The syncframes are the ones `info` and `verify` decode, framed by
/// [`crate::eac3::for_each_frame`] with the decoder's own header, so the frame
/// count and the sync errors are theirs as well. The walk used to read the
/// headers with a parser of its own that knew only E-AC-3, and an AC-3
/// syncframe has its CRC where E-AC-3 has the frame size: on a 5.1 AC-3 clip
/// in which `info` decodes 1171 frames and finds no sync error, it counted 902
/// frames and 902 sync errors.
pub(crate) fn for_each_container(
    path: &Path,
    mut on_container: impl FnMut(&Site, ContainerResult),
) -> Result<Walk> {
    let bytes = std::fs::metadata(path)
        .with_context(|| format!("opening {}", path.display()))?
        .len();
    let mut walk = Walk {
        bytes,
        ..Walk::default()
    };
    let mut noise = Noise::default();
    let mut sample_pos: u64 = 0; // first sample of the current independent frame
    let mut substreams: BTreeMap<(bool, u8), SubstreamContainers> = BTreeMap::new();
    let mut last_frame_len: u64 = 0;
    let (_, sync_errors, skipped) = crate::eac3::for_each_frame(path, |_, frame, header| {
        walk.frames += 1;
        let dependent = header.stream_type == StreamType::Dependent;
        if dependent {
            walk.dependent_frames += 1;
        } else {
            walk.independent_frames += 1;
            if walk.frames > 1 {
                sample_pos += last_frame_len;
            }
            last_frame_len = u64::from(header.blocks) * BLOCK_SAMPLES;
        }
        let (containers, has_skip, declares_joc) = find_emdf(
            frame,
            &mut noise,
            &mut walk.unparsed_frames,
            &mut walk.crc_failures,
        );
        if !containers.is_empty() {
            walk.frames_with_emdf += 1;
        }
        let opened = containers.iter().filter(|(_, c)| c.is_ok()).count() as u64;
        walk.containers += opened;
        let substream = substreams
            .entry((dependent, header.substream_id))
            .or_default();
        substream.opened += opened;
        substream.declares_joc |= declares_joc;
        if has_skip && opened == 0 {
            substream.without += 1;
            if substream.first_without.is_none() {
                let index = walk.frames - 1;
                let words = match containers.first() {
                    Some((offset, Err(e))) => {
                        format!("frame {index}, skip-field byte {offset}: {e}")
                    }
                    _ => format!("frame {index}: no EMDF container in the skip fields"),
                };
                substream.first_without = Some((index, words));
            }
        }
        for (offset, c) in containers {
            let site = Site {
                frame_index: walk.frames - 1,
                sample_pos,
                substream_id: header.substream_id,
                dependent,
                offset,
            };
            on_container(&site, c);
        }
        Ok(())
    })
    .with_context(|| format!("reading {}", path.display()))?;
    walk.sync_errors = sync_errors;
    walk.skipped_bytes = skipped;
    // A substream carries EMDF when a container opens in it, or when its frames
    // declare a JOC extension, which rides in one: the metadata of a programme
    // rides in one substream (TS 103 420 clauses 8.2 and 8.3.1), and the skip
    // fields of the others may carry anything. Without the second evidence a
    // substream whose containers are all broken read like one that carries none.
    let mut first: Option<(u64, String)> = None;
    for substream in substreams
        .into_values()
        .filter(|s| s.opened > 0 || s.declares_joc)
    {
        walk.missing_containers += substream.without;
        if let Some((index, words)) = substream.first_without
            && first.as_ref().is_none_or(|(earliest, _)| index < *earliest)
        {
            first = Some((index, words));
        }
    }
    walk.first_missing = first.map(|(_, words)| words);
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
    // the checks of `verify` read the stream in a pass of their own; run it
    // beside the walk instead of after it
    let check = crate::verify::spawn_stream_check(path);
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
            // counted once per frame by the walk, and only in a stream that
            // carries EMDF
            Err(_) => {}
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
    s.skipped_bytes = walk.skipped_bytes;
    s.crc_failures = walk.crc_failures;
    s.unparsed_frames = walk.unparsed_frames;
    s.container_errors = walk.missing_containers;
    if s.first_error.is_none() {
        s.first_error = walk.first_missing.clone();
    }
    if s.first_error.is_none() && walk.crc_failures > 0 {
        s.first_error = Some(format!("{} syncframes whose CRC failed", walk.crc_failures));
    }
    if s.first_error.is_none() && walk.skipped_bytes > 0 {
        s.first_error = Some(format!(
            "{} bytes skipped while framing the stream",
            walk.skipped_bytes
        ));
    }
    s.frames_with_emdf = walk.frames_with_emdf;
    // what the walk does not read, a JOC payload, the payload configuration, the
    // complexity index or the audio, `verify` checks: take its verdict rather
    // than re-derive a part of it
    let stream = crate::verify::join_stream_check(check)?;
    if s.first_error.is_none() && !stream.clean {
        s.first_error = stream.first_problem.clone();
    }
    let elapsed = started.elapsed().as_secs_f64();
    let clean = stream.clean
        && s.sync_errors == 0
        && s.skipped_bytes == 0
        && s.crc_failures == 0
        && s.unparsed_frames == 0
        && s.container_errors == 0
        && s.oamd_errors == 0;
    if opts.json {
        let mut value = serde_json::to_value(&s)?;
        value["verify"] = serde_json::to_value(&stream)?;
        value["clean"] = serde_json::json!(clean);
        value["seconds"] = serde_json::json!(elapsed);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!(
            "Frames:            {} ({} independent, {} dependent), {} bytes, {} sync errors, {} bytes skipped, {} CRC failures, {} unparsed",
            s.frames,
            s.independent_frames,
            s.dependent_frames,
            s.bytes,
            s.sync_errors,
            s.skipped_bytes,
            s.crc_failures,
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
        println!(
            "Verify:            {}",
            if stream.clean {
                "clean"
            } else {
                "non-conformant"
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One AC-3 syncframe of 1792 bytes (bsid 8, 448 kbit/s at 48 kHz, 3/2 with
    /// LFE): the header, then zeros. Its CRC is 0xFFFF, which read as an
    /// E-AC-3 frame size makes a frame of 4096 bytes.
    fn ac3_frame() -> Vec<u8> {
        let mut frame = vec![0u8; 1792];
        frame[..7].copy_from_slice(&[0x0B, 0x77, 0xFF, 0xFF, 0x1E, 0x40, 0xE1]);
        frame
    }

    /// Walks `bytes` as a file, and frames the same file the way `info` and
    /// `verify` do: (frames, sync errors, skipped bytes).
    fn walk_and_verify(tag: &str, bytes: &[u8]) -> (Walk, (u64, u64, u64)) {
        let path =
            std::env::temp_dir().join(format!("oadec-emdf-walk-{tag}-{}.ac3", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        let walk = for_each_container(&path, |_, _| {}).unwrap();
        let verifier = crate::eac3::for_each_frame(&path, |_, _, _| Ok(())).unwrap();
        std::fs::remove_file(&path).unwrap();
        (walk, verifier)
    }

    /// The walk frames AC-3 by the decoder's own header. It read every frame as
    /// E-AC-3 and took the CRC for the frame size, which made three frames one
    /// frame of 4096 bytes and a sync error.
    #[test]
    fn the_walk_frames_ac3_by_its_own_header() {
        let (walk, (frames, sync_errors, _)) = walk_and_verify("clean", &ac3_frame().repeat(3));
        assert_eq!(
            (walk.frames, walk.independent_frames, walk.sync_errors),
            (3, 3, 0)
        );
        assert_eq!((walk.frames, walk.sync_errors), (frames, sync_errors));
    }

    /// Where a stream is damaged, the walk resynchronises where the verifier
    /// does and counts the frames and sync errors it counts, so `emdf` reports
    /// no fault `verify` does not find.
    #[test]
    fn the_walk_resynchronises_where_the_verifier_does() {
        let frame = ac3_frame();
        let stream = [
            &frame[..],
            &frame[..],
            &[1, 2, 3][..],
            &frame[..],
            &frame[..1000],
        ]
        .concat();
        let (walk, (frames, sync_errors, _)) = walk_and_verify("damaged", &stream);
        assert!(walk.sync_errors > 0, "the damage is noticed");
        assert_eq!((walk.frames, walk.sync_errors), (frames, sync_errors));
    }
}
