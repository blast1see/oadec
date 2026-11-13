//! E-AC-3 / AC-3 commands: `info`, `verify`, `decode` and `compare` for
//! streams that start with the `0x0B77` sync word.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_diag::{Location, Provenance, Severity};
use oadec_eac3::{
    ChannelLoc, Coverage, Decoder, FrameHeader, Options, ProgramDecoder, ProgramFrame, RuleId,
    Syntax, find_sync,
};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::joc::{Joc, JocError, Slope, SparseReading};
use oadec_emdf::oamd::Oamd;
use serde_json::{Value, json};

use crate::decode::{Format, Order, format_duration};
use crate::integrity::Findings;
use crate::source::Source;

/// The findings of a pass over an AC-3 family stream, by rule.
type Diag = oadec_diag::Tally<RuleId>;

/// The place of a finding in frame group `index`.
fn frame_at(index: u64) -> Location {
    Location {
        unit: Some(index),
        ..Location::default()
    }
}

/// Whether the file starts with an AC-3 family sync word.
pub fn is_eac3(source: &Source) -> Result<bool> {
    let mut f = source
        .open()
        .with_context(|| format!("opening {}", source.name()))?;
    let mut head = vec![0u8; SNIFF_BYTES];
    let n = f.read(&mut head)?;
    head.truncate(n);
    // The first two bytes are not enough. A Blu-ray TrueHD track can carry an
    // AC-3 core frame in front of the MLP stream, so a file that opens with
    // the AC-3 sync word may still be TrueHD; FFmpeg and MediaInfo both give
    // up on one. A major sync with an access-unit chain behind it decides.
    if truehd_start(&head).is_some() {
        return Ok(false);
    }
    Ok(head.starts_with(&[0x0B, 0x77]))
}

/// How far into a file to look for the shape of the stream.
const SNIFF_BYTES: usize = 64 << 10;

/// The offset where a TrueHD stream starts inside `head`, if one does.
///
/// A four-byte sync word turns up in audio data about once every four
/// gigabytes, so finding one proves nothing on its own; the access-unit
/// lengths in front of it have to chain as well.
fn truehd_start(head: &[u8]) -> Option<usize> {
    const CHAIN: usize = 8;
    let mut at = 0usize;
    while at + 8 <= head.len() {
        let word = u32::from_be_bytes(head[at + 4..at + 8].try_into().ok()?);
        if word == oadec_truehd::sync::SYNC_FBA || word == oadec_truehd::sync::SYNC_FBB {
            let mut off = at;
            let mut linked = 0;
            while linked < CHAIN && off + 2 <= head.len() {
                let len = usize::from(u16::from_be_bytes([head[off], head[off + 1]]) & 0x0FFF) * 2;
                if len < 8 || off + len > head.len() {
                    break;
                }
                off += len;
                linked += 1;
            }
            if linked >= CHAIN || off >= head.len() {
                return Some(at);
            }
        }
        at += 1;
    }
    None
}

/// Streams the syncframes of a file: `on_frame(offset, bytes, header)`.
/// Returns `(frames, sync_errors, skipped_bytes)`.
pub(crate) fn for_each_frame(
    source: &Source,
    on_frame: impl FnMut(u64, &[u8], &FrameHeader) -> Result<()>,
) -> Result<(u64, u64, u64)> {
    for_each_frame_and_gap(source, on_frame, |_, _, _| {})
}

/// [`for_each_frame`], which also says where the stream was no syncframe:
/// `on_gap(offset, sync_error, bytes)` is called for every sync error that is
/// counted and for every run of bytes that is skipped, with the offset in the
/// file it is at, so that the calls add up to the two totals returned.
fn for_each_frame_and_gap(
    source: &Source,
    mut on_frame: impl FnMut(u64, &[u8], &FrameHeader) -> Result<()>,
    mut on_gap: impl FnMut(u64, bool, u64),
) -> Result<(u64, u64, u64)> {
    let file_bytes = source.size().unwrap_or(0);
    let mut progress = crate::progress::Progress::new("reading", file_bytes);
    let mut file = BufReader::with_capacity(4 << 20, source.open()?);
    let mut buf: Vec<u8> = Vec::with_capacity(8 << 20);
    let mut base: u64 = 0; // file offset of buf[0]
    let mut frames = 0u64;
    let mut sync_errors = 0u64;
    let mut skipped = 0u64;
    let mut eof = false;
    loop {
        if buf.len() < 4096 && !eof {
            let mut chunk = vec![0u8; 4 << 20];
            let n = file.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                buf.extend_from_slice(&chunk[..n]);
            }
        }
        if buf.len() < 8 {
            if eof {
                on_gap(base, false, buf.len() as u64);
                skipped += buf.len() as u64;
                break;
            }
            continue;
        }
        progress.at(base);
        let Some(pos) = find_sync(&buf, 0) else {
            on_gap(base, false, buf.len() as u64);
            skipped += buf.len() as u64;
            base += buf.len() as u64;
            buf.clear();
            if eof {
                break;
            }
            continue;
        };
        if pos > 0 {
            on_gap(base, true, pos as u64);
            sync_errors += 1;
            skipped += pos as u64;
            buf.drain(..pos);
            base += pos as u64;
            continue;
        }
        let header = match FrameHeader::parse(&buf) {
            Ok(h) => h,
            Err(_) => {
                on_gap(base, true, 1);
                sync_errors += 1;
                skipped += 1;
                buf.drain(..1);
                base += 1;
                continue;
            }
        };
        if buf.len() < header.frame_bytes {
            if eof {
                on_gap(base, false, buf.len() as u64);
                skipped += buf.len() as u64;
                break;
            }
            let mut chunk = vec![0u8; (header.frame_bytes - buf.len()).max(4 << 20)];
            let n = file.read(&mut chunk)?;
            if n == 0 {
                eof = true;
            } else {
                buf.extend_from_slice(&chunk[..n]);
            }
            continue;
        }
        // the next frame must start with a sync word (or the file must end)
        let next_ok = buf.len() == header.frame_bytes && eof
            || buf.len() < header.frame_bytes + 2
            || buf[header.frame_bytes..header.frame_bytes + 2] == [0x0B, 0x77];
        if !next_ok {
            on_gap(base, true, 1);
            sync_errors += 1;
            skipped += 1;
            buf.drain(..1);
            base += 1;
            continue;
        }
        on_frame(base, &buf[..header.frame_bytes], &header)?;
        frames += 1;
        base += header.frame_bytes as u64;
        buf.drain(..header.frame_bytes);
        if buf.is_empty() && eof {
            break;
        }
    }
    // The line is cleared when `progress` is dropped, which also covers the
    // `?` on on_frame above.
    Ok((frames, sync_errors, skipped))
}

fn syntax_name(h: &FrameHeader) -> &'static str {
    match h.syntax {
        Syntax::Ac3 => "AC-3",
        Syntax::Eac3 => "E-AC-3",
    }
}

/// The channels a decode writes and the samples of each, in programme order.
///
/// The whole programme by default. With `core_only` the independent
/// substream's own channels, which is the 5.1-compatible downmix a decoder
/// limited to 5.1 would produce and clause E.2.8.2 allows -- useful for
/// comparing against a reference decoder that only renders that far.
fn output_channels(frame: &ProgramFrame, core_only: bool) -> (Vec<ChannelLoc>, Vec<&[f32]>) {
    if core_only {
        let part = &frame.parts[0];
        return (
            part.locations.clone(),
            part.decoded.pcm.iter().map(Vec::as_slice).collect(),
        );
    }
    (
        frame.layout.channels.clone(),
        (0..frame.channels()).map(|i| frame.channel(i)).collect(),
    )
}

/// Output channel indices (into the coded order) for the requested order.
///
/// Interchange order is the order of the WAVE mask bits, which each channel
/// location carries with it, so the order and the mask below can never
/// disagree.
fn output_order(chans: &[ChannelLoc], order: Order) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..chans.len()).collect();
    if order == Order::Interchange {
        idx.sort_by_key(|&i| chans[i].interchange_rank());
    }
    idx
}

/// WAVE channel mask bits in WAVE order.
///
/// Zero when the programme holds a location WAVE cannot name (the
/// surround-direct and wide pairs, the second LFE): a mask with fewer bits set
/// than there are channels is not a valid `WAVEFORMATEXTENSIBLE`, and zero is
/// the format's own way of saying the assignment is not stated.
///
/// With a mask of zero comes the warning that says why, for the caller to
/// pass on.
fn channel_mask(chans: &[ChannelLoc]) -> (u32, Option<String>) {
    let mask = chans.iter().fold(0, |a, c| a | c.wave_mask());
    if mask.count_ones() as usize == chans.len() {
        (mask, None)
    } else {
        let unnamed: Vec<&str> = chans
            .iter()
            .filter(|c| c.wave_mask() == 0)
            .map(|c| c.name())
            .collect();
        (
            0,
            Some(format!(
                "warning: WAVE has no channel mask bit for {}; writing an unassigned mask",
                unnamed.join(", ")
            )),
        )
    }
}

#[derive(Debug, Default)]
struct EmdfStats {
    frames_with_skip: u64,
    skip_bytes: u64,
    containers: u64,
    /// Frames whose skip fields held no parsable container. Skip fields may
    /// carry other data, so these are a fault only in a stream that carries
    /// EMDF; see [`EmdfStats::missing_containers`].
    container_errors: u64,
    /// Sync words inside payload bytes that did not start a container.
    false_syncs: u64,
    /// The first frame whose skip fields held no parsable container.
    first_missing: Option<u64>,
    /// The scanned substream declared a JOC extension in its `addbsi`, which
    /// rides in an EMDF container (TS 103 420 clause 8.3.1): it carries EMDF
    /// even when not one container opens.
    declares_joc: bool,
    payload_ids: BTreeMap<u32, u64>,
    /// What an object decode asks of every payload, asked here too.
    payloads: oadec_spatial::PayloadCheck,
    /// The frames whose skip fields held no container that opens. Whether
    /// that is a fault is known at the end of the pass
    /// (`missing_containers`), and the findings are filed then.
    missing_frames: Vec<u64>,
    oamd_ok: u64,
    oamd_errors: u64,
    /// Payloads that parse and that an object output can make nothing of:
    /// without an object element, or of another programme than the first.
    oamd_unusable: u64,
    /// Elements whose declared size is not theirs, as `scan` counts them for TrueHD.
    oamd_stale_sizes: u64,
    /// Object gains other than unity, by decibel value, over updates that
    /// signalled one; the mutes counted apart, because table 28's default for
    /// an inactive object *is* a mute; and updates carrying a non-zero
    /// `object_size`. Both Dolby encoders drop these two fields, so an
    /// authored stream cannot carry either and only the wild can answer
    /// whether anything does.
    oamd_gain_size: oadec_emdf::oamd::GainSizeCounts,
    joc: u64,
    first_error: Option<String>,
    // JOC side information statistics
    joc_ok: u64,
    joc_errors: u64,
    /// JOC payloads whose `joc_ext_config_idx` is reserved (TS 103 420 table
    /// 49), counted apart from the payloads that would not parse.
    joc_reserved_ext: u64,
    joc_size_mismatch: u64,
    /// Fields of the OAMD and JOC payload configurations outside table 56 of
    /// TS 103 420, as far as `container::table_56_violations` enforces it.
    payload_config_violations: u64,
    /// The object total of the first OAMD payload that parsed: the bed, ISF
    /// and dynamic objects `complexity_index_type_a` has to equal.
    oamd_total: Option<usize>,
    /// Frames carrying auxiliary data user bits (clause 4.4.4), and how many
    /// bytes of them, and how many EMDF containers they hold. Annex H names
    /// `auxdata` as a place a container may be carried, next to the skip
    /// fields, so this is what says whether anything in the wild uses it.
    auxdata_frames: u64,
    auxdata_bytes: u64,
    containers_in_auxdata: u64,
    auxdata_overruns: u64,
    /// Containers found in the independent substream of a programme that also
    /// has dependent substreams. TS 103 420 clause 8.2 puts the container in
    /// the last dependent substream when one exists, so this should be zero;
    /// it is reported and never acted on, because an encoder that disagrees
    /// with the clause is worth knowing about.
    containers_in_independent: u64,
    joc_padding_nonzero: u64,
    joc_dmx: BTreeMap<u8, u64>,
    joc_objects: BTreeMap<usize, u64>,
    joc_bands: BTreeMap<usize, u64>,
    joc_absent_objects: u64,
    joc_sparse: u64,
    /// Objects that are sparse and steep at once.
    joc_sparse_steep: u64,
    joc_dense: u64,
    joc_two_dpoints: u64,
    joc_steep: u64,
    joc_fine: u64,
    joc_coarse: u64,
    /// The four temporal-interpolation branches of clause 6.6.5, pseudo-code 6,
    /// counted separately: slope smooth or steep crossed with one or two data
    /// points. Two of them have never been seen in any stream measured, and a
    /// total that only says "2 760 steep" cannot tell you which.
    joc_interp: BTreeMap<&'static str, u64>,
    /// The distribution of `joc_offset_ts`, the time slot a steep slope
    /// switches at.
    joc_offset_ts: BTreeMap<u8, u64>,
    /// The first frames carrying each rare branch, capped. Counts say whether a
    /// stream exercises a branch; these say where to cut a clip that does.
    joc_rare_frames: BTreeMap<&'static str, Vec<u64>>,
    joc_seq_zero: u64,
    /// The sequence counter of the first JOC payload and of the last one, and
    /// how many payloads after the first started again at 0 or did not follow
    /// the one before them (TS 103 420 clause 6.3.3.3).
    joc_seq_first: Option<u16>,
    joc_seq_last: Option<u16>,
    joc_seq_restarts: u64,
    joc_seq_gaps: u64,
    /// What the containers declare of their protection: a version, a key
    /// identifier and the lengths of two words. The words are not read.
    protection: crate::protection::ProtectionStats,
    joc_clipgain: BTreeMap<u32, u64>,
    /// Per clip gain bucket: the largest and the summed peak matrix
    /// coefficient, and how many payloads went in.
    joc_peak: BTreeMap<u32, (f64, f64, u64)>,
    /// The clip gain of the payload the last scan saw, so the caller can pair
    /// it with the decoded samples of the same frame.
    last_clipgain: Option<f64>,
}

impl EmdfStats {
    /// Frames that lost their container: frames whose skip fields held no
    /// container that opens, in a stream in which containers do open or whose
    /// frames declare a JOC extension. Skip fields may carry other data, and two
    /// AC-3 clips of the corpus fill them in some 1 100 frames without a single
    /// container, so in a stream that carries no EMDF at all the count is not a
    /// fault; a stream whose containers are all broken still declares the
    /// extension, and its frames did lose their metadata.
    fn missing_containers(&self) -> u64 {
        if self.containers > 0 || self.declares_joc {
            self.container_errors
        } else {
            0
        }
    }

    /// Files the frames that lost their container, at the end of the pass:
    /// a frame whose skip fields held no container is a fault where the
    /// stream carries EMDF, and whether it does is known only then.
    fn file_missing(&self, diag: &mut Diag) {
        if self.missing_containers() > 0 {
            for &frame in &self.missing_frames {
                diag.add(RuleId::EmdfMissingContainer, frame_at(frame), 1, || {
                    "no EMDF container that opens in the skip fields".to_string()
                });
            }
        }
    }

    /// Records where a rare syntax branch occurred, up to a cap: the counts
    /// answer "does anything use this", the frame numbers answer "where do I
    /// cut a clip that does".
    fn note_rare(&mut self, what: &'static str, frame: u64) {
        let seen = self.joc_rare_frames.entry(what).or_default();
        if seen.len() < 64 && seen.last() != Some(&frame) {
            seen.push(frame);
        }
    }

    fn scan(
        &mut self,
        frame_index: u64,
        skip_fields: &[Vec<u8>],
        declares_emdf: bool,
        diag: &mut Diag,
    ) {
        let at = frame_at(frame_index);
        self.last_clipgain = None;
        let total: usize = skip_fields.iter().map(Vec::len).sum();
        if total == 0 {
            // A substream that declares the JOC extension carries a container in
            // every frame, so a frame with no skip field at all has lost one.
            // Returning here left that loss unreported (R7F1). A frame of a
            // stream that carries no EMDF has lost nothing.
            if declares_emdf {
                self.container_errors += 1;
                self.missing_frames.push(frame_index);
                self.first_missing.get_or_insert(frame_index);
            }
            return;
        }
        self.frames_with_skip += 1;
        self.skip_bytes += total as u64;
        let mut data = Vec::with_capacity(total);
        for s in skip_fields {
            data.extend_from_slice(s);
        }
        // A frame carries one EMDF container (ETSI TS 103 420 clause 8.2), but
        // its payload bytes may hold the sync word again. A candidate that does
        // not parse is such a false sync, not a broken container; only a frame
        // that yields no container at all is a failure.
        let mut found = false;
        let mut pos = 0usize;
        while pos + 4 <= data.len() {
            if data[pos] != 0x58 || data[pos + 1] != 0x38 {
                pos += 1;
                continue;
            }
            match container::parse_emdf_with_sync(&data[pos..]) {
                Ok((c, used)) => {
                    self.containers += 1;
                    found = true;
                    // What the container declares of itself. None of it is a
                    // fault of the stream; each says something of where the
                    // stream comes from, and is counted until a count of
                    // streams says what it is worth.
                    let changed = self.protection.take(&c);
                    if c.version != 0 {
                        diag.add(RuleId::EmdfSyntaxVersion, at, 1, || {
                            format!("emdf_version {}", c.version)
                        });
                    }
                    if c.protection.primary_code == 0 {
                        diag.add(RuleId::EmdfProtectionReserved, at, 1, || {
                            "primary protection length coded 00".to_string()
                        });
                    }
                    if changed {
                        diag.add(RuleId::EmdfProtectionChanged, at, 1, || {
                            format!(
                                "key_id {}, protection words of {} and {} bytes",
                                c.key_id,
                                c.protection.primary.len(),
                                c.protection.secondary.len()
                            )
                        });
                    }
                    // TS 103 420 table 56 fixes most of the configuration of
                    // the OAMD and JOC payloads
                    let mut violations: Vec<String> = Vec::new();
                    for p in &c.payloads {
                        for v in container::table_56_violations(p.id, &p.config) {
                            violations.push(format!("payload {}: {v}", p.id));
                        }
                    }
                    if let Some(v) = container::table_56_group_violation(&c) {
                        violations.push(format!(
                            "payloads {PAYLOAD_ID_OAMD} and {PAYLOAD_ID_JOC}: {v}"
                        ));
                    }
                    self.payload_config_violations += violations.len() as u64;
                    for v in &violations {
                        diag.add(RuleId::EmdfPayloadConfig, at, 1, || v.clone());
                    }
                    if self.first_error.is_none()
                        && let Some(v) = violations.first()
                    {
                        self.first_error = Some(format!("frame {frame_index}: {v}"));
                    }
                    for p in &c.payloads {
                        *self.payload_ids.entry(p.id).or_default() += 1;
                        if p.id == PAYLOAD_ID_JOC {
                            self.joc += 1;
                            match Joc::parse(&p.data, SparseReading::default()) {
                                Ok(j) => {
                                    self.joc_ok += 1;
                                    if !j.size_ok(p.data.len()) {
                                        self.joc_size_mismatch += 1;
                                        diag.add(RuleId::JocSizeMismatch, at, 1, || {
                                            format!(
                                                "a JOC payload of {} bytes is not the size its syntax takes",
                                                p.data.len()
                                            )
                                        });
                                    }
                                    if !j.padding_zero {
                                        self.joc_padding_nonzero += 1;
                                    }
                                    *self.joc_dmx.entry(j.dmx_config).or_default() += 1;
                                    *self.joc_objects.entry(j.num_objects).or_default() += 1;
                                    let bucket = (j.clipgain * 1000.0).round() as u32;
                                    *self.joc_clipgain.entry(bucket).or_default() += 1;
                                    self.last_clipgain = Some(j.clipgain);
                                    // Clause 6.3.3.2 defines the clip gain but
                                    // never says what a decoder does with it.
                                    // If it exists to keep the matrix inside
                                    // its quantized range, then the peak
                                    // coefficient times the clip gain should
                                    // sit near the ceiling whenever the gain
                                    // is not 1; this records the evidence.
                                    let peak = joc_peak(&j);
                                    let e = self.joc_peak.entry(bucket).or_insert((0.0, 0.0, 0));
                                    e.0 = e.0.max(peak);
                                    e.1 += peak;
                                    e.2 += 1;
                                    if j.seq_count == 0 {
                                        self.joc_seq_zero += 1;
                                        // clause 6.3.3.3: the first frame of
                                        // the bitstream, or the first after a
                                        // splice. Where they are is what a cut
                                        // between two of them needs.
                                        self.note_rare("seq-count-zero", frame_index);
                                    }
                                    // the same clause: the counter goes up
                                    // by one from frame to frame
                                    match self.joc_seq_last {
                                        None => self.joc_seq_first = Some(j.seq_count),
                                        Some(_) if j.seq_count == 0 => {
                                            self.joc_seq_restarts += 1;
                                            diag.add(RuleId::JocSequenceRestart, at, 1, || {
                                                "joc_seq_count 0 after the first payload"
                                                    .to_string()
                                            });
                                        }
                                        Some(before)
                                            if j.seq_count != Joc::seq_count_after(before) =>
                                        {
                                            self.joc_seq_gaps += 1;
                                            diag.add(RuleId::JocSequenceGap, at, 1, || {
                                                format!(
                                                    "joc_seq_count {} after {before}",
                                                    j.seq_count
                                                )
                                            });
                                        }
                                        Some(_) => {}
                                    }
                                    self.joc_seq_last = Some(j.seq_count);
                                    for o in &j.objects {
                                        match o {
                                            None => self.joc_absent_objects += 1,
                                            Some(o) => {
                                                *self.joc_bands.entry(o.num_bands).or_default() +=
                                                    1;
                                                if o.sparse {
                                                    self.joc_sparse += 1;
                                                    self.note_rare("sparse", frame_index);
                                                    // the combination is what
                                                    // the one unexplained frame
                                                    // has, and a clip that
                                                    // carries more of them is
                                                    // what turns an anomaly of
                                                    // one into a measurement
                                                    if o.slope == Slope::Steep {
                                                        self.joc_sparse_steep += 1;
                                                        self.note_rare(
                                                            "sparse-and-steep",
                                                            frame_index,
                                                        );
                                                    }
                                                } else {
                                                    self.joc_dense += 1;
                                                }
                                                if o.num_dpoints == 2 {
                                                    self.joc_two_dpoints += 1;
                                                    self.note_rare("two-data-points", frame_index);
                                                }
                                                if o.slope == Slope::Steep {
                                                    self.joc_steep += 1;
                                                }
                                                if o.quant_idx == 1 {
                                                    self.joc_fine += 1;
                                                } else {
                                                    self.joc_coarse += 1;
                                                    self.note_rare("coarse", frame_index);
                                                }
                                                let branch = match (o.slope, o.num_dpoints) {
                                                    (Slope::Smooth, 1) => "smooth-1",
                                                    (Slope::Smooth, _) => "smooth-2",
                                                    (Slope::Steep, 1) => "steep-1",
                                                    (Slope::Steep, _) => "steep-2",
                                                };
                                                *self.joc_interp.entry(branch).or_default() += 1;
                                                if branch != "smooth-1" {
                                                    self.note_rare(branch, frame_index);
                                                }
                                                for ts in o.offset_ts.iter().take(o.num_dpoints) {
                                                    *self.joc_offset_ts.entry(*ts).or_default() +=
                                                        1;
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(JocError::ExtConfig(ext)) => {
                                    self.joc_reserved_ext += 1;
                                    diag.add(RuleId::JocReservedExtConfig, at, 1, || {
                                        format!("joc_ext_config_idx {ext} is reserved")
                                    });
                                    if self.first_error.is_none() {
                                        self.first_error = Some(format!(
                                            "frame {frame_index}: joc_ext_config_idx {ext} is reserved"
                                        ));
                                    }
                                }
                                Err(e) => {
                                    self.joc_errors += 1;
                                    diag.add(RuleId::JocParse, at, 1, || e.to_string());
                                    if self.first_error.is_none() {
                                        self.first_error =
                                            Some(format!("frame {frame_index}: JOC: {e}"));
                                    }
                                }
                            }
                        }
                        if p.id == PAYLOAD_ID_OAMD {
                            match Oamd::parse(&p.data) {
                                Ok(oamd) => {
                                    self.oamd_ok += 1;
                                    // A payload that parses is not yet one an
                                    // object output can use: its timeline
                                    // refuses one without an object element
                                    // and one of another programme, and counts
                                    // it against what it delivers.
                                    if let Err(e) = self.payloads.take(&oamd) {
                                        self.oamd_unusable += 1;
                                        diag.add(RuleId::OamdUnusable, at, 1, || e.to_string());
                                        if self.first_error.is_none() {
                                            self.first_error =
                                                Some(format!("frame {frame_index}: OAMD: {e}"));
                                        }
                                    }
                                    for element in oamd.elements.iter().filter(|e| !e.size_ok) {
                                        self.oamd_stale_sizes += 1;
                                        diag.add(RuleId::OamdStaleSize, at, 1, || {
                                            format!(
                                                "element {} declares a size that is not its own",
                                                element.id
                                            )
                                        });
                                    }
                                    let program = &oamd.program;
                                    self.oamd_total.get_or_insert(
                                        program.bed_objects()
                                            + program.isf_objects()
                                            + program.dynamic_objects,
                                    );
                                    let counts = oamd.gain_and_size_counts();
                                    if !counts.gains_db.is_empty() {
                                        self.note_rare("oamd-object-gain", frame_index);
                                    }
                                    if counts.sized > 0 {
                                        self.note_rare("oamd-object-size", frame_index);
                                    }
                                    self.oamd_gain_size.add(&counts);
                                }
                                Err(e) => {
                                    self.oamd_errors += 1;
                                    diag.add(RuleId::OamdParse, at, 1, || e.to_string());
                                    if self.first_error.is_none() {
                                        self.first_error =
                                            Some(format!("frame {frame_index}: OAMD: {e}"));
                                    }
                                }
                            }
                        }
                    }
                    pos += used.max(4);
                }
                Err(_) => {
                    self.false_syncs += 1;
                    pos += 1;
                }
            }
        }
        if !found {
            self.container_errors += 1;
            self.missing_frames.push(frame_index);
            // a fault only if the stream turns out to carry EMDF, which is known
            // at the end of the pass (`missing_containers`)
            self.first_missing.get_or_insert(frame_index);
        }
    }
}

fn coverage_list(c: &Coverage) -> Vec<&'static str> {
    let mut v = Vec::new();
    if c.coupling {
        v.push("coupling");
    }
    if c.enhanced_coupling {
        v.push("enhanced-coupling");
    }
    if c.spectral_extension {
        v.push("spectral-extension");
    }
    if c.aht {
        v.push("aht");
    }
    if c.transient_pre_noise {
        v.push("transient-pre-noise");
    }
    if c.block_switching {
        v.push("block-switching");
    }
    if c.dither {
        v.push("dither");
    }
    if c.delta_allocation {
        v.push("delta-allocation");
    }
    if c.skip_fields {
        v.push("skip-fields");
    }
    if c.rematrixing {
        v.push("rematrixing");
    }
    v
}

fn merge(into: &mut Coverage, c: &Coverage) {
    into.coupling |= c.coupling;
    into.enhanced_coupling |= c.enhanced_coupling;
    into.spectral_extension |= c.spectral_extension;
    into.aht |= c.aht;
    into.transient_pre_noise |= c.transient_pre_noise;
    into.block_switching |= c.block_switching;
    into.dither |= c.dither;
    into.delta_allocation |= c.delta_allocation;
    into.skip_fields |= c.skip_fields;
    into.rematrixing |= c.rematrixing;
}

/// What one substream of a programme carried, for the substreams that are
/// parsed but whose audio does not reach the output.
#[derive(Debug, Default)]
struct SubStats {
    frames: u64,
    decode_errors: u64,
    crc_failures: u64,
    tail_overruns: u64,
    /// Smallest and largest number of bits of a frame the parser left unread.
    /// A well-formed frame is consumed nearly to its end, so a systematic
    /// misparse of syntax that has never run shows up here as wild or negative
    /// slack long before it shows up as bad audio.
    slack_bits: Option<(i64, i64)>,
    first_error: Option<String>,
    header: Option<FrameHeader>,
    /// Channel locations, from the custom channel map or from `acmod`.
    locations: Vec<ChannelLoc>,
    /// Every distinct `chanmap` seen, so a mid-stream change is visible.
    chanmaps: BTreeMap<u16, u64>,
    lfe_implied: u64,
}

/// Summary of a full pass over a stream.
#[derive(Debug, Default)]
struct Pass {
    frames: u64,
    independent: u64,
    dependent: u64,
    substreams: BTreeMap<(u8, u8), u64>,
    /// The dependent substreams of the programme, keyed by
    /// `(strmtyp, substreamid)`.
    subs: BTreeMap<(u8, u8), SubStats>,
    /// What assembling the programme found.
    program: oadec_eac3::ProgramStats,
    /// The programme's channels and where each comes from.
    layout: Option<oadec_eac3::ProgramLayout>,
    samples: u64,
    decode_errors: u64,
    crc_failures: u64,
    tail_overruns: u64,
    first_error: Option<String>,
    /// The first problem that is a fault of a delivery: `first_error` less the
    /// conformance findings a delivery does not fail on (see `findings`).
    first_fault: Option<String>,
    /// Frames whose `complexity_index_type_a` breaks TS 103 420 clause 8.3.2.2.
    complexity_mismatches: u64,
    /// The first frame's `bsi` of the substream that carries the EMDF
    /// container, whose `addbsi` holds the extension of TS 103 420 clause 8.3
    /// (clause 8.3.1): the last dependent substream when there is one.
    metadata_bsi: Option<oadec_eac3::Bsi>,
    coverage: Coverage,
    emdf: EmdfStats,
    /// The findings of the pass by rule: every counter above that is a check
    /// of `verify`, filed where it was raised.
    diag: Diag,
    first: Option<(FrameHeader, oadec_eac3::Bsi)>,
    dialnorm: BTreeMap<u8, u64>,
    aht_frames: u64,
    spx_frames: u64,
    ecpl_frames: u64,
    tpnp_frames: u64,
    /// Frames in which a delta bit allocation segment reached the masking
    /// curve; carrying the fields with "perform no delta" does not count.
    dba_frames: u64,
    /// Every transient pre-noise parameter set, for the evidence tooling.
    transients: Vec<(u64, usize, usize, usize)>,
    /// Per clip gain bucket: the largest and the summed peak sample of the
    /// decoded core, and how many frames went in.
    core_peak: BTreeMap<u32, (f64, f64, u64)>,
    /// Frames whose JOC payload carries a clip gain other than 1, for the
    /// evidence tooling.
    clipgains: Vec<(u64, f64)>,
}

/// Folds what one substream of a group carried into [`SubStats`].
///
/// The bit slack is the point. Clause E.1.3.1 syntax that only dependent
/// substreams carry -- `chanmape`, `chanmap`, and the branches guarded on
/// `strmtyp` -- never ran on real material until this pass reached it, because
/// the filter that used to sit here dropped the frame before it was parsed at
/// all. A single bit wrong there does not give a slightly wrong frame:
/// `Frame::parse` seeks the audio blocks to the bit position the `bsi` ended
/// at, so the whole frame is misread, and the frame CRC will not catch it
/// because the CRC is over the bytes and never touches the parse. Bit slack
/// does catch it.
fn account_sub(sub: &mut SubStats, index: u64, part: &oadec_eac3::Part, diag: &mut Diag) {
    let d = &part.decoded;
    let at = frame_at(index).in_substream(usize::from(part.key.1));
    sub.frames += 1;
    if sub.header.is_none() {
        sub.header = Some(d.header.clone());
    }
    let slack = (d.header.frame_bytes as i64) * 8 - d.used_bits as i64;
    sub.slack_bits = Some(match sub.slack_bits {
        None => (slack, slack),
        Some((lo, hi)) => (lo.min(slack), hi.max(slack)),
    });
    if !d.crc_ok {
        sub.crc_failures += 1;
        diag.add(RuleId::SubstreamCrc, at, 1, || "CRC failure".to_string());
        if sub.first_error.is_none() {
            sub.first_error = Some(format!("group {index}: CRC failure"));
        }
    }
    if d.tail_overrun {
        sub.tail_overruns += 1;
        diag.add(RuleId::SubstreamTailOverrun, at, 1, || {
            "the audio blocks end inside the frame tail".to_string()
        });
        if sub.first_error.is_none() {
            sub.first_error = Some(format!(
                "group {index}: the audio blocks end inside the frame tail"
            ));
        }
    }
    if let Some(m) = d.bsi.chanmap {
        *sub.chanmaps.entry(m).or_default() += 1;
    }
    if part.lfe_implied {
        sub.lfe_implied += 1;
    }
    if sub.locations.is_empty() {
        sub.locations = part.locations.clone();
    }
}

/// Decodes the programme carried by independent substream 0 and the dependent
/// substreams that follow it, and collects statistics; `on_pcm` receives the
/// merged frame groups in order.
fn pass(
    source: &Source,
    opts: Options,
    on_pcm: impl FnMut(&ProgramFrame) -> Result<()>,
) -> Result<(Pass, u64, u64)> {
    pass_while(source, opts, &|| true, on_pcm)
}

/// [`pass`], given up with an error at the first syncframe at which `go_on`
/// says no.
fn pass_while(
    source: &Source,
    opts: Options,
    go_on: &dyn Fn() -> bool,
    on_pcm: impl FnMut(&ProgramFrame) -> Result<()>,
) -> Result<(Pass, u64, u64)> {
    pass_listing(source, opts, go_on, Diag::DEFAULT_MAX_PER_RULE, on_pcm)
}

/// [`pass_while`], keeping at most `listed` findings of a rule in full (0
/// keeps every one). The limit changes what is listed and never a count.
fn pass_listing(
    source: &Source,
    opts: Options,
    go_on: &dyn Fn() -> bool,
    listed: usize,
    mut on_pcm: impl FnMut(&ProgramFrame) -> Result<()>,
) -> Result<(Pass, u64, u64)> {
    let mut dec = ProgramDecoder::new(opts);
    let mut p = Pass::default();
    // The findings are filed from two places that are two closures: the walk
    // over the bytes, and the frames it yields.
    let diag = std::cell::RefCell::new(Diag::new(listed));
    let mut seen = ProgramSeen::default();
    let (frames, sync_errors, skipped) = for_each_frame_and_gap(
        source,
        |offset, bytes, header| {
            if !go_on() {
                bail!("the pass was given up");
            }
            let key = (header.stream_type as u8, header.substream_id);
            *p.substreams.entry(key).or_default() += 1;
            match header.stream_type {
                oadec_eac3::StreamType::Dependent => p.dependent += 1,
                _ => p.independent += 1,
            }
            dec.push(bytes, header)?;
            let diag = &mut *diag.borrow_mut();
            while let Some(frame) = dec.pop() {
                account(&mut p, &frame, &mut on_pcm, diag)?;
            }
            // what assembling the programme counted while it took this frame
            seen.file(dec.stats(), Location::byte(offset), diag);
            Ok(())
        },
        |offset, sync_error, bytes| {
            let diag = &mut *diag.borrow_mut();
            if sync_error {
                diag.add(RuleId::FrameSync, Location::byte(offset), 1, || {
                    "no syncframe starts here".to_string()
                });
            }
            if bytes > 0 {
                diag.add(RuleId::FrameSkipped, Location::byte(offset), bytes, || {
                    let s = if bytes == 1 { "" } else { "s" };
                    format!("{bytes} byte{s} that belong to no syncframe")
                });
            }
        },
    )?;
    let mut diag = diag.into_inner();
    dec.finish()?;
    while let Some(frame) = dec.pop() {
        account(&mut p, &frame, &mut on_pcm, &mut diag)?;
    }
    seen.file(dec.stats(), Location::default(), &mut diag);
    p.program = dec.stats().clone();
    p.layout = dec.layout().cloned();
    // decode errors of the independent substream are the ones that stop a
    // decode; a dependent substream's are reported against that substream
    p.decode_errors = p
        .program
        .decode_errors
        .iter()
        .filter(|((t, _), _)| *t != 1)
        .map(|(_, n)| *n)
        .sum();
    for ((t, id), n) in &p.program.decode_errors {
        if *t == 1 {
            p.subs.entry((*t, *id)).or_default().decode_errors = *n;
        }
    }
    if p.first_error.is_none() {
        p.first_error = p.program.first_error.clone();
    }
    if p.first_fault.is_none() {
        p.first_fault = p.program.first_error.clone();
    }
    if p.emdf.missing_containers() > 0
        && p.emdf.first_error.is_none()
        && let Some(frame) = p.emdf.first_missing
    {
        p.emdf.first_error = Some(format!(
            "frame {frame}: no EMDF container in the skip fields"
        ));
    }
    p.emdf.file_missing(&mut diag);
    p.diag = diag;
    // a rule and its counter are one fact told twice
    debug_assert_eq!(disagreement(&p, sync_errors, skipped), None);
    let _ = frames;
    Ok((p, sync_errors, skipped))
}

/// What assembling the programme had counted the last time it was looked
/// at, so that what it counts while it takes one frame is filed where that
/// frame is.
#[derive(Debug, Default)]
struct ProgramSeen {
    counts: [u64; 9],
    decode_errors: BTreeMap<(u8, u8), u64>,
}

impl ProgramSeen {
    /// The rules the counters of a programme are, in the order of `counts`,
    /// and what a finding of each says.
    const RULES: [(RuleId, &'static str); 9] = [
        (
            RuleId::ProgramDependentDropped,
            "the channels of a dependent frame did not reach the programme",
        ),
        (
            RuleId::ProgramOrphanDependent,
            "a dependent frame with no independent substream before it",
        ),
        (
            RuleId::ProgramMisaligned,
            "a dependent frame of another block count or sample rate than its independent substream",
        ),
        (
            RuleId::ProgramLocation,
            "the channel map of a dependent frame cannot be read",
        ),
        (
            RuleId::ProgramOverCapacity,
            "a channel past the sixteen of a programme was dropped",
        ),
        (
            RuleId::ProgramLayoutChange,
            "the channels of the group are not the channels the programme began with",
        ),
        (
            RuleId::ProgramDuplicateFrame,
            "a second frame of a substream in one group",
        ),
        (
            RuleId::ProgramStalledSubstream,
            "a substream stopped supplying frames and was flushed",
        ),
        (
            RuleId::ProgramMissingFrame,
            "a group was delivered without the frame of one of its substreams",
        ),
    ];

    fn counts(stats: &oadec_eac3::ProgramStats) -> [u64; 9] {
        [
            stats.dependent_dropped,
            stats.orphan_dependents,
            stats.misaligned,
            stats.location_errors,
            stats.over_capacity,
            stats.layout_changes,
            stats.duplicate_substream_frames,
            stats.stalled_substreams,
            stats.missing_substream_frames,
        ]
    }

    /// Files what `stats` has counted since the last call, at `at`.
    fn file(&mut self, stats: &oadec_eac3::ProgramStats, at: Location, diag: &mut Diag) {
        let now = Self::counts(stats);
        for (((rule, says), before), after) in Self::RULES.iter().zip(&mut self.counts).zip(now) {
            for _ in *before..after {
                diag.add(*rule, at, 1, || (*says).to_string());
            }
            *before = after;
        }
        for (key, &after) in &stats.decode_errors {
            let before = self.decode_errors.entry(*key).or_default();
            // a dependent substream's are told apart from the ones that stop
            // a decode
            let (rule, place) = if key.0 == 1 {
                (RuleId::SubstreamDecode, at.in_substream(usize::from(key.1)))
            } else {
                (RuleId::FrameDecode, at)
            };
            for _ in *before..after {
                diag.add(rule, place, 1, || "the frame would not decode".to_string());
            }
            *before = after;
        }
    }
}

/// Each rule of the catalog with the counter it is counted under, as the
/// pass counted it.
fn counters(p: &Pass, sync_errors: u64, skipped: u64) -> [(RuleId, u64); 31] {
    let e = &p.emdf;
    let subs = |of: fn(&SubStats) -> u64| p.subs.values().map(of).sum::<u64>();
    [
        (RuleId::FrameSync, sync_errors),
        (RuleId::FrameSkipped, skipped),
        (RuleId::FrameDecode, p.decode_errors),
        (RuleId::FrameCrc, p.crc_failures),
        (RuleId::FrameTailOverrun, p.tail_overruns),
        (RuleId::SubstreamDecode, subs(|s| s.decode_errors)),
        (RuleId::SubstreamCrc, subs(|s| s.crc_failures)),
        (RuleId::SubstreamTailOverrun, subs(|s| s.tail_overruns)),
        (RuleId::ProgramDependentDropped, p.program.dependent_dropped),
        (RuleId::ProgramOrphanDependent, p.program.orphan_dependents),
        (RuleId::ProgramMisaligned, p.program.misaligned),
        (RuleId::ProgramLocation, p.program.location_errors),
        (RuleId::ProgramOverCapacity, p.program.over_capacity),
        (RuleId::ProgramLayoutChange, p.program.layout_changes),
        (
            RuleId::ProgramDuplicateFrame,
            p.program.duplicate_substream_frames,
        ),
        (
            RuleId::ProgramStalledSubstream,
            p.program.stalled_substreams,
        ),
        (
            RuleId::ProgramMissingFrame,
            p.program.missing_substream_frames,
        ),
        (RuleId::EmdfMissingContainer, e.missing_containers()),
        (RuleId::EmdfPayloadConfig, e.payload_config_violations),
        (RuleId::BsiComplexityIndex, p.complexity_mismatches),
        (RuleId::OamdParse, e.oamd_errors),
        (RuleId::OamdUnusable, e.oamd_unusable),
        (RuleId::OamdStaleSize, e.oamd_stale_sizes),
        (RuleId::JocParse, e.joc_errors),
        (RuleId::JocReservedExtConfig, e.joc_reserved_ext),
        (RuleId::JocSizeMismatch, e.joc_size_mismatch),
        (RuleId::EmdfSyntaxVersion, e.protection.other_versions()),
        (
            RuleId::EmdfProtectionReserved,
            e.protection.reserved_primary_code,
        ),
        (RuleId::EmdfProtectionChanged, e.protection.changes),
        (RuleId::JocSequenceRestart, e.joc_seq_restarts),
        (RuleId::JocSequenceGap, e.joc_seq_gaps),
    ]
}

/// What is not the same in the findings and in the counters of a pass, if
/// anything: a rule that fired another number of times than its counter
/// counted, or a verdict the two do not share.
fn disagreement(p: &Pass, sync_errors: u64, skipped: u64) -> Option<String> {
    for (rule, counter) in counters(p, sync_errors, skipped) {
        let filed = p.diag.get(rule).map_or(0, |t| t.magnitude);
        if filed != counter {
            return Some(format!(
                "{}: findings of {filed} and a counter of {counter}",
                rule.info().id
            ));
        }
    }
    (p.diag.passes(Severity::Error) != is_clean(p, sync_errors, skipped))
        .then(|| "the findings and the counters give two verdicts".to_string())
}

/// Folds one decoded frame into the pass statistics.
fn account(
    p: &mut Pass,
    frame: &ProgramFrame,
    on_pcm: &mut impl FnMut(&ProgramFrame) -> Result<()>,
    diag: &mut Diag,
) -> Result<()> {
    let index = frame.index;
    let d = frame.core();
    p.frames += 1;
    for part in frame.parts.iter().skip(1) {
        account_sub(p.subs.entry(part.key).or_default(), index, part, diag);
    }
    if p.first.is_none() {
        p.first = Some((d.header.clone(), d.bsi.clone()));
    }
    *p.dialnorm.entry(d.bsi.dialnorm).or_default() += 1;
    merge(&mut p.coverage, &d.coverage);
    if d.coverage.aht {
        p.aht_frames += 1;
    }
    if d.coverage.spectral_extension {
        p.spx_frames += 1;
    }
    if d.coverage.enhanced_coupling {
        p.ecpl_frames += 1;
    }
    if d.coverage.delta_allocation {
        p.dba_frames += 1;
    }
    if d.coverage.transient_pre_noise {
        p.tpnp_frames += 1;
        for (ch, t) in d.transproc.iter().enumerate() {
            if let Some(t) = t {
                p.transients.push((index, ch, t.loc, t.len));
            }
        }
    }
    if d.tail_overrun {
        p.tail_overruns += 1;
        diag.add(RuleId::FrameTailOverrun, frame_at(index), 1, || {
            "the audio blocks end inside the frame tail".to_string()
        });
        if p.first_error.is_none() {
            p.first_error = Some(format!(
                "frame {index}: the audio blocks end inside the frame tail"
            ));
        }
    }
    if !d.crc_ok {
        p.crc_failures += 1;
        diag.add(RuleId::FrameCrc, frame_at(index), 1, || {
            "CRC failure".to_string()
        });
        let problem = format!("frame {index}: CRC failure");
        if p.first_fault.is_none() {
            p.first_fault = Some(problem.clone());
        }
        if p.first_error.is_none() {
            p.first_error = Some(problem);
        }
    }
    // TS 103 420 clause 8.2: with dependent substreams present the EMDF
    // container carrying OAMD and JOC is in the last dependent substream. With
    // none, `metadata_part` is the independent substream and this is exactly
    // what it was before.
    // TS 103 420 clause 8.3.1: the addbsi extension is in the same substream
    // as the container. It is read before the scan, because a frame that
    // carries no skip field at all is a lost container exactly where the
    // substream declares one.
    let extension = frame.metadata_part().decoded.bsi.joc_extension();
    let declares_joc = matches!(extension, Some((true, _)));
    let carries_emdf = declares_joc || p.emdf.declares_joc;
    p.emdf.scan(
        index,
        &frame.metadata_part().decoded.skip_fields,
        carries_emdf,
        diag,
    );
    p.emdf.declares_joc |= declares_joc;
    if p.metadata_bsi.is_none() {
        p.metadata_bsi = Some(frame.metadata_part().decoded.bsi.clone());
    }
    if let Some((true, complexity)) = extension
        && let Some(total) = p.emdf.oamd_total
        && let Some(problem) = complexity_problem(complexity, total)
    {
        p.complexity_mismatches += 1;
        diag.add(RuleId::BsiComplexityIndex, frame_at(index), 1, || {
            problem.clone()
        });
        if p.first_error.is_none() {
            p.first_error = Some(format!("frame {index}: {problem}"));
        }
    }
    if frame.parts.len() > 1 {
        p.emdf.containers_in_independent += count_containers(&d.skip_fields);
    }
    for part in &frame.parts {
        if !part.decoded.auxdata.is_empty() {
            p.emdf.auxdata_frames += 1;
            p.emdf.auxdata_bytes += part.decoded.auxdata.len() as u64;
            p.emdf.containers_in_auxdata +=
                count_containers(std::slice::from_ref(&part.decoded.auxdata));
        }
        if part.decoded.auxdata_overrun {
            p.emdf.auxdata_overruns += 1;
        }
    }
    if let Some(g) = p.emdf.last_clipgain {
        if (g - 1.0).abs() > 1e-9 {
            p.clipgains.push((index, g));
        }
        let peak = d
            .pcm
            .iter()
            .flat_map(|c| c.iter())
            .fold(0.0f32, |a, &v| a.max(v.abs()));
        let e = p
            .core_peak
            .entry((g * 1000.0).round() as u32)
            .or_insert((0.0, 0.0, 0));
        e.0 = e.0.max(f64::from(peak));
        e.1 += f64::from(peak);
        e.2 += 1;
    }
    p.samples += d.header.samples() as u64;
    on_pcm(frame)
}

/// How many EMDF containers a substream's skip fields hold, for the evidence
/// counter above. Nothing is parsed beyond the container itself.
fn count_containers(skip: &[Vec<u8>]) -> u64 {
    let data: Vec<u8> = skip.iter().flatten().copied().collect();
    let mut pos = 0usize;
    let mut found = 0u64;
    while pos + 2 <= data.len() {
        if u16::from_be_bytes([data[pos], data[pos + 1]]) != container::EMDF_SYNCWORD {
            pos += 1;
            continue;
        }
        match container::parse_emdf_with_sync(&data[pos..]) {
            Ok((_, used)) => {
                found += 1;
                pos += used.max(1);
            }
            Err(_) => pos += 1,
        }
    }
    found
}

/// The largest absolute dequantized matrix coefficient of one JOC payload.
fn joc_peak(j: &Joc) -> f64 {
    let mut peak = 0.0f64;
    for o in j.objects.iter().flatten() {
        for dp in &o.mtx_q {
            for ch in dp.iter().take(j.num_channels) {
                for &q in ch.iter().take(o.num_bands) {
                    peak = peak.max(oadec_joc::dequantize(q, o.quant_idx).abs());
                }
            }
        }
    }
    peak
}

/// TS 103 420 clause 8.3.2.2: `complexity_index_type_a` equals the total of
/// bed, ISF and dynamic objects the OAMD programme assignment declares, and is
/// at most 16. What is wrong with an index, if anything.
fn complexity_problem(complexity: u8, oamd_total: usize) -> Option<String> {
    if usize::from(complexity) != oamd_total {
        Some(format!(
            "complexity_index_type_a {complexity} differs from the OAMD object total {oamd_total} (clause 8.3)"
        ))
    } else if complexity > 16 {
        Some(format!(
            "complexity_index_type_a {complexity} exceeds 16 (clause 8.3)"
        ))
    } else {
        None
    }
}

/// Whether a pass found nothing wrong: every frame decoded, every CRC and
/// every metadata payload checked out, in a stream that carries EMDF every frame
/// with skip fields held a container that opens, no byte of the file was
/// skipped, and the payload configuration and the complexity index are what
/// TS 103 420 requires.
fn is_clean(p: &Pass, sync_errors: u64, skipped: u64) -> bool {
    p.decode_errors == 0
        && p.crc_failures == 0
        && p.tail_overruns == 0
        && sync_errors == 0
        && skipped == 0
        && p.emdf.oamd_errors == 0
        && p.emdf.oamd_unusable == 0
        && p.emdf.joc_errors == 0
        && p.emdf.joc_reserved_ext == 0
        && p.emdf.joc_size_mismatch == 0
        && p.emdf.missing_containers() == 0
        && p.emdf.payload_config_violations == 0
        && p.complexity_mismatches == 0
        // A dependent substream that was seen and whose channels did not reach
        // the output means the programme was truncated, whatever the frames
        // that did decode looked like. A second programme is legal and is not
        // counted here.
        && p.program.is_clean()
        && p.subs
            .values()
            .all(|s| s.decode_errors == 0 && s.crc_failures == 0 && s.tail_overruns == 0)
}

/// One line per substream of the selected programme, in bitstream order:
/// what it codes, what its channels are, and whether they reached the output.
fn program_parts(p: &Pass, h: &FrameHeader) -> Vec<Value> {
    let mut parts = vec![json!({
        "substream": "independent 0",
        "syntax": syntax_name(h),
        "acmod": h.acmod,
        "lfeon": h.lfeon,
        "chanmap": Value::Null,
        "channels": Decoder::channel_names(h),
        "frames": p.frames,
        "merged": true,
        "bit_rate": h.bit_rate(),
        "crc_failures": p.crc_failures,
        "decode_errors": p.decode_errors,
        "tail_overruns": p.tail_overruns,
    })];
    for ((t, id), sub) in &p.subs {
        let sh = sub.header.as_ref();
        parts.push(json!({
            "substream": format!("{} {id}", if *t == 1 { "dependent" } else { "independent" }),
            "syntax": sh.map(syntax_name),
            "acmod": sh.map(|h| h.acmod),
            "lfeon": sh.map(|h| h.lfeon),
            "chanmap": sub.chanmaps.keys().map(|m| format!("0x{m:04x}")).collect::<Vec<_>>(),
            "channels": sub.locations.iter().map(|c| c.name()).collect::<Vec<_>>(),
            "frames": sub.frames,
            "merged": true,
            "bit_rate": sh.map(oadec_eac3::FrameHeader::bit_rate),
            "crc_failures": sub.crc_failures,
            "decode_errors": sub.decode_errors,
            "tail_overruns": sub.tail_overruns,
            "lfe_implied": sub.lfe_implied,
            "unread_bits": sub.slack_bits.map(|(lo, hi)| json!([lo, hi])),
            "first_error": sub.first_error,
        }));
    }
    parts
}

/// The faults `is_clean` weighs, in the form a delivery path reports, less the
/// findings that make a file non-conformant without making what was delivered
/// untrustworthy (`docs/exit-codes.md`): frames that end inside their own tail,
/// which are out of spec and decode to the audio FFmpeg, Dolby and oadec agree
/// on; payload configurations outside table 56 of TS 103 420; and a complexity
/// index that disagrees with the OAMD. A delivery prints them and keeps its
/// verdict; `verify` still calls the file non-conformant.
fn findings(p: &Pass, sync_errors: u64, skipped: u64) -> Findings {
    let mut f = Findings::default();
    f.note(p.decode_errors, "frames failed to decode");
    f.note(p.crc_failures, "CRC failures");
    f.note(sync_errors, "sync errors");
    f.note(skipped, "bytes skipped");
    f.note(
        p.emdf.oamd_errors,
        "Object Audio Metadata payloads failed to parse",
    );
    f.note(
        p.emdf.oamd_unusable,
        "Object Audio Metadata payloads the programme cannot take",
    );
    f.note(p.emdf.joc_errors, "JOC payloads failed to parse");
    f.note(
        p.emdf.joc_reserved_ext,
        "JOC payloads with a reserved joc_ext_config_idx",
    );
    f.note(
        p.emdf.joc_size_mismatch,
        "JOC payloads whose declared size was wrong",
    );
    f.note(
        p.emdf.missing_containers(),
        "frames whose skip fields held no EMDF container that opens",
    );
    f.note(
        p.program.dependent_dropped,
        "dependent substream frames dropped",
    );
    f.note(
        p.program.orphan_dependents,
        "dependent frames with no independent substream",
    );
    f.note(
        p.program.misaligned,
        "misaligned dependent substream frames",
    );
    f.note(p.program.location_errors, "unreadable channel maps");
    f.note(
        p.program.over_capacity,
        "channels past the sixteen a programme may carry",
    );
    f.note(
        p.program.layout_changes,
        "mid-stream channel layout changes",
    );
    f.note(
        p.program.duplicate_substream_frames,
        "substream frames repeated within one group",
    );
    f.note(
        p.program.stalled_substreams,
        "substreams flushed after they stopped supplying frames",
    );
    f.note(
        p.program.missing_substream_frames,
        "substream frames that never came out of their decoder",
    );
    for (key, sub) in &p.subs {
        let id = key.1;
        f.note(
            sub.decode_errors,
            &format!("frames of dependent substream {id} failed to decode"),
        );
        f.note(
            sub.crc_failures,
            &format!("CRC failures in dependent substream {id}"),
        );
    }
    f.first_problem(p.first_fault.as_deref());
    f
}

/// The report of a pass as `--json` prints it, or `None` when no frame of the
/// stream decoded: there is nothing to report of it then but that.
fn pass_json(
    name: &str,
    p: &Pass,
    sync_errors: u64,
    skipped: u64,
    elapsed: f64,
    fail_on: Severity,
) -> Option<Value> {
    let (h, bsi) = p.first.as_ref()?;
    let names = p.layout.as_ref().map_or_else(
        || Decoder::channel_names(h),
        oadec_eac3::ProgramLayout::names,
    );
    let duration = p.samples as f64 / f64::from(h.sample_rate);
    // TS 103 420 clause 8.3.1: the extension is in the substream that carries
    // the container, which is not the independent one when a dependent follows
    let meta = p.metadata_bsi.as_ref().unwrap_or(bsi);
    let joc = meta.joc_extension();
    let e = &p.emdf;
    let payload_ids: serde_json::Map<String, Value> = e
        .payload_ids
        .iter()
        .map(|(k, v)| (k.to_string(), Value::from(*v)))
        .collect();
    let report = json!({
        "file": name,
        "syntax": syntax_name(h),
        "sample_rate": h.sample_rate,
        "channels": names,
        "blocks_per_frame": h.blocks,
        "bit_rate": h.bit_rate(),
        "bsid": h.bsid,
        "frames": p.frames,
        "independent_frames": p.independent,
        "dependent_frames": p.dependent,
        "samples": p.samples,
        "duration": duration,
        "clean": verdict(p, sync_errors, skipped, fail_on),
        "failures": {
            "sync_errors": sync_errors,
            "skipped_bytes": skipped,
            "decode_errors": p.decode_errors,
            "crc_failures": p.crc_failures,
            "tail_overruns": p.tail_overruns,
            "oamd_errors": e.oamd_errors,
            "oamd_unusable": e.oamd_unusable,
            "missing_containers": e.missing_containers(),
            "oamd_stale_sizes": e.oamd_stale_sizes,
            "joc_errors": e.joc_errors,
            "joc_reserved_ext_config": e.joc_reserved_ext,
            "joc_size_mismatches": e.joc_size_mismatch,
            "payload_config_violations": e.payload_config_violations,
            "complexity_mismatches": p.complexity_mismatches,
            "dependent_dropped": p.program.dependent_dropped,
            "orphan_dependents": p.program.orphan_dependents,
            "substream_decode_errors": p.subs.values().map(|s| s.decode_errors).sum::<u64>(),
            "substream_crc_failures": p.subs.values().map(|s| s.crc_failures).sum::<u64>(),
            "substream_tail_overruns": p.subs.values().map(|s| s.tail_overruns).sum::<u64>(),
            "location_errors": p.program.location_errors,
            "misaligned_substreams": p.program.misaligned,
            "channels_over_capacity": p.program.over_capacity,
            "layout_changes": p.program.layout_changes,
            "duplicate_substream_frames": p.program.duplicate_substream_frames,
            "stalled_substreams": p.program.stalled_substreams,
            "missing_substream_frames": p.program.missing_substream_frames,
        },
        "diagnostics": diagnostics_json(&p.diag, frame_seconds(h), fail_on),
        "program": program_parts(p, h),
        "other_program_frames": p.program.other_program_frames,
        "max_pending_groups": p.program.max_pending_groups,
        "coverage": coverage_list(&p.coverage),
        "aht_frames": p.aht_frames,
        "spx_frames": p.spx_frames,
        "ecpl_frames": p.ecpl_frames,
        "tpnp_frames": p.tpnp_frames,
        "dba_frames": p.dba_frames,
        "clipgains": p
            .clipgains
            .iter()
            .map(|&(frame, gain)| serde_json::json!({ "frame": frame, "gain": gain }))
            .collect::<Vec<_>>(),
        "transients": p
            .transients
            .iter()
            .map(|&(frame, channel, loc, len)| {
                serde_json::json!({
                    "frame": frame,
                    "channel": channel,
                    "loc": loc,
                    "len": len,
                })
            })
            .collect::<Vec<_>>(),
        "joc_extension": joc.map(|(flag, complexity)| json!({
            "flag": flag,
            "complexity_index": complexity,
        })),
        "dialnorm": p.dialnorm.keys().collect::<Vec<_>>(),
        "emdf": {
            "frames_with_skip": e.frames_with_skip,
            "skip_bytes": e.skip_bytes,
            "containers": e.containers,
            "container_errors": e.container_errors,
            "containers_in_independent_substream": e.containers_in_independent,
            "auxdata_frames": e.auxdata_frames,
            "auxdata_bytes": e.auxdata_bytes,
            "containers_in_auxdata": e.containers_in_auxdata,
            "auxdata_overruns": e.auxdata_overruns,
            "false_syncs": e.false_syncs,
            "payload_ids": payload_ids,
            "oamd_ok": e.oamd_ok,
            "oamd_errors": e.oamd_errors,
            "oamd_object_gains_db": e.oamd_gain_size.gains_db.iter()
                .map(|(db, n)| (db.to_string(), *n))
                .collect::<BTreeMap<_, _>>(),
            "oamd_muted_updates": e.oamd_gain_size.muted,
            "oamd_sized_updates": e.oamd_gain_size.sized,
            "joc_payloads": e.joc,
            "protection": e.protection,
        },
        "joc": (e.joc > 0).then(|| json!({
            "parsed": e.joc_ok,
            "errors": e.joc_errors,
            "reserved_ext_config": e.joc_reserved_ext,
            "size_mismatches": e.joc_size_mismatch,
            "non_zero_padding": e.joc_padding_nonzero,
            "downmix_configs": e.joc_dmx.keys().collect::<Vec<_>>(),
            "objects_per_payload": e.joc_objects.keys().collect::<Vec<_>>(),
            "bands": e.joc_bands.keys().collect::<Vec<_>>(),
            "sparse_objects": e.joc_sparse,
            "sparse_and_steep_objects": e.joc_sparse_steep,
            "dense_objects": e.joc_dense,
            "absent_objects": e.joc_absent_objects,
            "steep_objects": e.joc_steep,
            "fine_quantized_objects": e.joc_fine,
            "coarse_quantized_objects": e.joc_coarse,
            "two_data_points": e.joc_two_dpoints,
            "interpolation_branches": e.joc_interp.iter()
                .map(|(k, v)| ((*k).to_string(), Value::from(*v)))
                .collect::<serde_json::Map<String, Value>>(),
            "rare_branch_frames": e.joc_rare_frames.iter()
                .map(|(k, v)| ((*k).to_string(), Value::from(v.clone())))
                .collect::<serde_json::Map<String, Value>>(),
            "offset_ts": e.joc_offset_ts.iter()
                .map(|(k, v)| (k.to_string(), Value::from(*v)))
                .collect::<serde_json::Map<String, Value>>(),
            "seq_count_zero": e.joc_seq_zero,
            "seq_count_first": e.joc_seq_first,
            "seq_count_restarts": e.joc_seq_restarts,
            "seq_count_gaps": e.joc_seq_gaps,
            // clause 6.3.3.2, x1000 so the ladder stays exact in JSON
            "clipgain_x1000": e.joc_clipgain.keys().collect::<Vec<_>>(),
            "clipgain_frames": e
                .joc_clipgain
                .iter()
                .filter(|(g, _)| **g != 1000)
                .map(|(_, n)| n)
                .sum::<u64>(),
        })),
        "first_error": p.first_error.as_ref().or(e.first_error.as_ref()),
        "seconds": elapsed,
    });
    Some(report)
}

/// Prints the report of a pass on stdout, as JSON or as text.
#[allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "printing is what this one is for"
)]
fn print_pass(
    name: &str,
    p: &Pass,
    sync_errors: u64,
    skipped: u64,
    elapsed: f64,
    json: bool,
    fail_on: Severity,
) {
    let Some((h, bsi)) = &p.first else {
        eprintln!("{name}: no decodable frames");
        return;
    };
    if json {
        if let Some(report) = pass_json(name, p, sync_errors, skipped, elapsed, fail_on) {
            println!("{report}");
        }
        return;
    }
    let names = p.layout.as_ref().map_or_else(
        || Decoder::channel_names(h),
        oadec_eac3::ProgramLayout::names,
    );
    let duration = p.samples as f64 / f64::from(h.sample_rate);
    // TS 103 420 clause 8.3.1: the extension is in the substream that carries
    // the container, which is not the independent one when a dependent follows
    let meta = p.metadata_bsi.as_ref().unwrap_or(bsi);
    let joc = meta.joc_extension();
    println!("File:              {name}");
    println!(
        "Stream:            {} bsid {}, {} Hz, {} blocks/frame, {} kbit/s",
        syntax_name(h),
        h.bsid,
        h.sample_rate,
        h.blocks,
        h.bit_rate() / 1000
    );
    if p.subs.is_empty() {
        println!(
            "Channels:          {} (acmod {}{}): {}",
            names.len(),
            h.acmod,
            if h.lfeon { " + LFE" } else { "" },
            names.join(" ")
        );
    } else {
        println!(
            "Channels:          {} (programme, {} substreams): {}",
            names.len(),
            p.subs.len() + 1,
            names.join(" ")
        );
    }
    println!(
        "Frames:            {} decoded ({} independent, {} dependent in the file), {} sync errors, {} bytes skipped",
        p.frames, p.independent, p.dependent, sync_errors, skipped
    );
    println!(
        "Duration:          {} ({} samples)",
        format_duration(duration),
        p.samples
    );
    println!(
        "Substreams:        {}",
        p.substreams
            .iter()
            .map(|((t, id), n)| format!("type {t} id {id}: {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if !p.subs.is_empty() {
        println!(
            "Programme:         independent 0: {} acmod {}{}, {} kbit/s -> {}, {} frames",
            syntax_name(h),
            h.acmod,
            if h.lfeon { " + LFE" } else { "" },
            h.bit_rate() / 1000,
            Decoder::channel_names(h).join(" "),
            p.frames
        );
        for ((t, id), sub) in &p.subs {
            let kind = if *t == 1 { "dependent" } else { "independent" };
            let map = sub
                .chanmaps
                .keys()
                .map(|m| format!("chanmap 0x{m:04x}"))
                .collect::<Vec<_>>()
                .join(" then ");
            let chans = sub
                .locations
                .iter()
                .map(|c| c.name())
                .collect::<Vec<_>>()
                .join(" ");
            println!(
                "                   {kind} {id}: {} acmod {}{}{} -> {}",
                sub.header.as_ref().map_or("?", |h| syntax_name(h)),
                sub.header.as_ref().map_or(0, |h| h.acmod),
                sub.header
                    .as_ref()
                    .map_or("", |h| if h.lfeon { " + LFE" } else { "" }),
                if map.is_empty() {
                    String::new()
                } else {
                    format!(", {map}")
                },
                if chans.is_empty() { "?" } else { &chans }
            );
            let slack = sub
                .slack_bits
                .map_or_else(|| "-".to_string(), |(lo, hi)| format!("{lo} to {hi}"));
            println!(
                "                     {} frames merged, {} CRC failures, {} decode errors, {} frames ending inside the frame tail, {} bits unread",
                sub.frames, sub.crc_failures, sub.decode_errors, sub.tail_overruns, slack
            );
            if sub.lfe_implied > 0 {
                println!(
                    "                     {} frames whose channel map left the LFE implied",
                    sub.lfe_implied
                );
            }
        }
    }
    if !p.program.is_clean() {
        println!(
            "Programme faults:  {} dependent frames dropped, {} misaligned, {} channel map errors, {} channels past sixteen, {} layout changes, {} orphan dependents",
            p.program.dependent_dropped,
            p.program.misaligned,
            p.program.location_errors,
            p.program.over_capacity,
            p.program.layout_changes,
            p.program.orphan_dependents
        );
    }
    if p.program.duplicate_substream_frames > 0 {
        println!(
            "Repeated frames:   {} substream frames repeated within one group, only the first of each decoded",
            p.program.duplicate_substream_frames
        );
    }
    if p.program.stalled_substreams > 0 || p.program.missing_substream_frames > 0 {
        println!(
            "Stalled groups:    {} substreams flushed after they stopped supplying frames, {} substream frames that never came out of their decoder",
            p.program.stalled_substreams, p.program.missing_substream_frames
        );
    }
    if p.program.other_program_frames > 0 {
        println!(
            "Other programmes:  {} frames skipped (only the first independent substream is decoded)",
            p.program.other_program_frames
        );
    }
    println!(
        "Dialnorm:          {}",
        p.dialnorm
            .iter()
            .map(|(k, n)| format!("-{k} dB x{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    match joc {
        Some((flag, complexity)) => println!(
            "JOC extension:     flag {}, complexity index {} (addbsi {} bytes)",
            flag,
            complexity,
            meta.addbsi.len()
        ),
        None => println!(
            "JOC extension:     none (addbsi {} bytes)",
            meta.addbsi.len()
        ),
    }
    if p.emdf.payload_config_violations > 0 || p.complexity_mismatches > 0 {
        println!(
            "Conformance:       {} payload configuration fields outside TS 103 420 table 56, {} frames whose complexity index breaks clause 8.3",
            p.emdf.payload_config_violations, p.complexity_mismatches
        );
    }
    println!(
        "Coding tools:      {} (AHT in {} frames, spectral extension in {}, enhanced coupling in {}, transient pre-noise in {}, delta bit allocation in {})",
        coverage_list(&p.coverage).join(", "),
        p.aht_frames,
        p.spx_frames,
        p.ecpl_frames,
        p.tpnp_frames,
        p.dba_frames
    );
    println!(
        "EMDF:              {} frames with skip fields ({} bytes), {} containers, {} frames without one, {} false sync words, payload ids {:?}",
        p.emdf.frames_with_skip,
        p.emdf.skip_bytes,
        p.emdf.containers,
        p.emdf.container_errors,
        p.emdf.false_syncs,
        p.emdf.payload_ids
    );
    if p.emdf.protection.containers > 0 {
        // what the containers declare, as `info` says it of a TrueHD stream
        println!("Protection:        {}", p.emdf.protection.summary());
    }
    // said only where there is one, so that the line of every other stream
    // is the line it was
    let refused = match p.emdf.oamd_unusable {
        0 => String::new(),
        n => format!(", {n} the programme cannot take"),
    };
    println!(
        "Metadata:          {} OAMD payloads ({} errors{refused}), {} JOC payloads",
        p.emdf.oamd_ok, p.emdf.oamd_errors, p.emdf.joc
    );
    if p.emdf.joc > 0 {
        let e = &p.emdf;
        // said only where the counter did not simply count on, so that the
        // line of every other stream is the line it was
        let sequence = match (e.joc_seq_restarts, e.joc_seq_gaps) {
            (0, 0) => String::new(),
            (restarts, gaps) => format!(
                " ({restarts} after the first, {gaps} payloads did not follow the one before)"
            ),
        };
        println!(
            "JOC parse:         {} ok, {} errors, {} reserved extensions, {} size mismatches, {} non-zero paddings; dmx configs {:?}; objects per payload {:?}; seq_count 0 in {} payloads{sequence}; clipgain x1000 {:?}",
            e.joc_ok,
            e.joc_errors,
            e.joc_reserved_ext,
            e.joc_size_mismatch,
            e.joc_padding_nonzero,
            e.joc_dmx,
            e.joc_objects,
            e.joc_seq_zero,
            e.joc_clipgain
        );
        if e.joc_clipgain.len() > 1 {
            let rows: Vec<String> = e
                .joc_peak
                .iter()
                .map(|(g, (max, sum, n))| {
                    let gain = f64::from(*g) / 1000.0;
                    format!(
                        "{gain:.3}: peak {max:.2}, mean {:.2}, x gain {:.2} ({n})",
                        sum / *n as f64,
                        max * gain
                    )
                })
                .collect();
            println!("JOC clip gain:     {}", rows.join("; "));
            let rows: Vec<String> = p
                .core_peak
                .iter()
                .map(|(g, (max, sum, n))| {
                    let gain = f64::from(*g) / 1000.0;
                    format!(
                        "{gain:.3}: core peak {max:.3}, mean {:.3}, x gain {:.3} ({n})",
                        sum / *n as f64,
                        max * gain
                    )
                })
                .collect();
            println!("JOC core level:    {}", rows.join("; "));
        }
        println!(
            "JOC objects:       bands {:?}; {} sparse, {} dense, {} absent; {} with two data points, {} steep, {} fine-quantized",
            e.joc_bands,
            e.joc_sparse,
            e.joc_dense,
            e.joc_absent_objects,
            e.joc_two_dpoints,
            e.joc_steep,
            e.joc_fine
        );
    }
    if p.emdf.oamd_ok > 0 {
        println!(
            "OAMD gain/size:    {} non-unity gains {:?}, {} mutes, {} non-zero sizes",
            p.emdf.oamd_gain_size.gains_db.values().sum::<u64>(),
            p.emdf.oamd_gain_size.gains_db,
            p.emdf.oamd_gain_size.muted,
            p.emdf.oamd_gain_size.sized
        );
        crate::scan::print_stale_sizes(p.emdf.oamd_stale_sizes);
    }
    println!(
        "Integrity:         {} decode errors, {} CRC failures, {} frames ending inside the frame tail",
        p.decode_errors, p.crc_failures, p.tail_overruns
    );
    print_diagnostics(&p.diag, frame_seconds(h), fail_on);
    if let Some(e) = p.first_error.as_ref().or(p.emdf.first_error.as_ref()) {
        println!("First problem:     {e}");
    }
    println!(
        "Speed:             {:.2} s ({:.0}x realtime)",
        elapsed,
        if elapsed > 0.0 {
            duration / elapsed
        } else {
            0.0
        }
    );
}

/// How long one frame group of a stream that opens with `h` lasts.
fn frame_seconds(h: &FrameHeader) -> f64 {
    (h.blocks as f64) * (oadec_eac3::BLOCK_SAMPLES as f64) / f64::from(h.sample_rate)
}

/// The verdict of `verify` at the threshold `fail_on`. At `error`, the
/// threshold it has when it is given none, the counters decide, as they did
/// before a check had a name; the findings are held to them at the end of
/// every pass, so the two readings are one. At any other threshold the worst
/// finding decides.
fn verdict(p: &Pass, sync_errors: u64, skipped: u64, fail_on: Severity) -> bool {
    if fail_on == Severity::Error {
        is_clean(p, sync_errors, skipped)
    } else {
        p.diag.passes(fail_on)
    }
}

/// The `diagnostics` object of the JSON report: the findings of the pass by
/// rule, as `verify` writes them of a TrueHD stream. A finding names its
/// frame group, or the offset in the file where it belongs to none.
fn diagnostics_json(d: &Diag, frame_seconds: f64, fail_on: Severity) -> Value {
    let place = |at: &Location| {
        json!({
            "frame": at.unit,
            "offset": at.offset,
            "substream": at.substream,
            "seconds": at.unit.map(|u| u as f64 * frame_seconds),
        })
    };
    let rules: Vec<Value> = d
        .iter()
        .map(|(rule, t)| {
            let info = rule.info();
            json!({
                "id": info.id,
                "severity": info.severity.name(),
                "delivery": rule.faults_a_delivery(),
                "count": t.count,
                "magnitude": t.magnitude,
                "provenance": info.provenance.iter().map(Provenance::kind).collect::<Vec<_>>(),
                "legacy_counter": info.legacy_counter,
                "first": place(&t.first.at),
                "first_message": t.first.message,
                "last": place(&t.last),
                "listed": t.listed.len(),
                "suppressed": t.suppressed(),
            })
        })
        .collect();
    let findings: Vec<Value> = d
        .iter()
        .flat_map(|(_, t)| t.listed.iter())
        .map(|f| {
            let mut v = place(&f.at);
            v["rule"] = json!(f.rule.info().id);
            v["severity"] = json!(f.rule.info().severity.name());
            v["magnitude"] = json!(f.magnitude);
            v["message"] = json!(f.message);
            v
        })
        .collect();
    let counts: serde_json::Map<String, Value> = Severity::ALL
        .iter()
        .map(|s| (s.name().to_string(), json!(d.count_at(*s))))
        .collect();
    json!({
        "schema": 1,
        "fail_on": fail_on.name(),
        "worst": d.worst().map(Severity::name),
        "max_per_rule": d.max_per_rule(),
        "counts": counts,
        "rules": rules,
        "findings": findings,
    })
}

/// Where a finding is, as the text report says it.
fn place_text(at: &Location, frame_seconds: f64) -> String {
    let mut parts = Vec::new();
    if let Some(frame) = at.unit {
        parts.push(format!("frame {frame}"));
    }
    if let Some(offset) = at.offset {
        parts.push(format!("@0x{offset:X}"));
    }
    if let Some(substream) = at.substream {
        parts.push(format!("ss{substream}"));
    }
    if let Some(frame) = at.unit {
        parts.push(crate::verify::clock(frame as f64 * frame_seconds));
    }
    if parts.is_empty() {
        "--".to_string()
    } else {
        parts.join(" ")
    }
}

/// The `Findings:` and `Rules:` blocks of the text report, as `verify` writes
/// them of a TrueHD stream: every listed finding of a rule that can fail a
/// stream, then one line per rule that fired, at most six of them, the
/// heaviest first. A stream nothing was found in has neither.
#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
fn print_diagnostics(d: &Diag, frame_seconds: f64, fail_on: Severity) {
    if d.is_empty() {
        return;
    }
    let mut first = true;
    // findings of an info rule are counted below and listed only when the
    // threshold asks for them
    for (rule, t) in d
        .iter()
        .filter(|(r, _)| fail_on == Severity::Info || r.info().severity > Severity::Info)
    {
        for f in &t.listed {
            println!(
                "{}{:<8} {:<26} {}  {}",
                if first {
                    "Findings:          "
                } else {
                    "                   "
                },
                rule.info().severity.name(),
                rule.info().id,
                place_text(&f.at, frame_seconds),
                f.message
            );
            first = false;
        }
        if t.suppressed() > 0 {
            println!(
                "                   {}: {} listed, {} more not listed (--max-per-rule), last at {}",
                rule.info().id,
                t.listed.len(),
                t.suppressed(),
                place_text(&t.last, frame_seconds)
            );
        }
    }
    let counts = Severity::ALL
        .iter()
        .rev()
        .map(|s| format!("{} {}", d.count_at(*s), s.name()))
        .collect::<Vec<_>>()
        .join(", ");
    println!("Rules:             {counts} (fail on {})", fail_on.name());
    let mut fired: Vec<_> = d.iter().collect();
    fired.sort_by_key(|(r, _)| core::cmp::Reverse(r.info().severity));
    for (rule, t) in fired.iter().take(6) {
        let span = if t.count > 1 {
            format!(
                "{} .. {}",
                place_text(&t.first.at, frame_seconds),
                place_text(&t.last, frame_seconds)
            )
        } else {
            place_text(&t.first.at, frame_seconds)
        };
        println!(
            "                   {:<8} {:<26} {:>6}x  {}",
            rule.info().severity.name(),
            rule.info().id,
            t.count,
            span
        );
    }
    if fired.len() > 6 {
        println!(
            "                   and {} more rules (verify --json)",
            fired.len() - 6
        );
    }
}

/// What a pass over an AC-3 or E-AC-3 stream found: the report `info` and
/// `verify` print, and the verdict of `verify`.
#[derive(Debug)]
pub struct Report {
    name: String,
    // on the heap: a report is handed from frame to frame
    pass: Box<Pass>,
    sync_errors: u64,
    skipped: u64,
    elapsed: f64,
    /// The threshold of the verdict.
    fail_on: Severity,
}

/// What `verify` may be told about the rules of an AC-3 family stream.
#[derive(Debug, Clone, Copy)]
pub struct ReportOptions {
    /// The verdict fails when the worst finding is at or above this.
    pub fail_on: Severity,
    /// Findings listed in full per rule; 0 lists every one.
    pub max_per_rule: usize,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            fail_on: Severity::Error,
            max_per_rule: Diag::DEFAULT_MAX_PER_RULE,
        }
    }
}

/// Decodes the whole stream to gather what `info` and `verify` report, and
/// prints nothing.
pub fn report(source: &Source) -> Result<Report> {
    report_with(source, &ReportOptions::default())
}

/// [`report`], with the threshold of the verdict and the number of findings
/// a rule lists.
pub fn report_with(source: &Source, options: &ReportOptions) -> Result<Report> {
    let started = Instant::now();
    let (p, sync_errors, skipped) = pass_listing(
        source,
        Options::default(),
        &|| true,
        options.max_per_rule,
        |_| Ok(()),
    )?;
    if p.independent + p.dependent == 0 {
        // A sync word opened the file and no whole syncframe followed. A report
        // of nothing reads like a clean stream that holds nothing: the file is
        // not a stream to judge but an unsupported input, exit 2, as it is for
        // every command that reports one.
        return Err(crate::info::no_stream(source));
    }
    Ok(Report {
        name: source.name().to_string(),
        pass: Box::new(p),
        sync_errors,
        skipped,
        elapsed: started.elapsed().as_secs_f64(),
        fail_on: options.fail_on,
    })
}

impl Report {
    /// Whether every frame decodes, every CRC checks and nothing was skipped:
    /// the verdict of `verify`, at the threshold the report was made with.
    #[must_use]
    pub fn clean(&self) -> bool {
        verdict(&self.pass, self.sync_errors, self.skipped, self.fail_on)
    }

    /// The report as `--json` prints it, or `None` when no frame of the
    /// stream decoded.
    #[must_use]
    pub fn json(&self) -> Option<Value> {
        pass_json(
            &self.name,
            &self.pass,
            self.sync_errors,
            self.skipped,
            self.elapsed,
            self.fail_on,
        )
    }

    /// Prints the report on stdout, as JSON or as text.
    pub fn print(&self, json: bool) {
        print_pass(
            &self.name,
            &self.pass,
            self.sync_errors,
            self.skipped,
            self.elapsed,
            json,
            self.fail_on,
        );
    }

    /// What a delivery of the stream has to answer for: the faults of the
    /// pass, in the form a delivery reports them.
    #[must_use]
    pub fn findings(&self) -> Findings {
        findings(&self.pass, self.sync_errors, self.skipped)
    }

    /// Prints the line that closes the text of `verify`.
    #[allow(clippy::print_stdout, reason = "printing is what this one is for")]
    pub fn print_verdict(&self) {
        println!(
            "Result:            {}",
            if self.clean() { "CLEAN" } else { "PROBLEMS" }
        );
    }
}

/// The checks of `verify` over an AC-3 or E-AC-3 stream: its verdict and the
/// first problem its report names. Given up with an error at the first
/// syncframe at which `go_on` says no.
pub fn stream_check(
    source: &Source,
    go_on: &dyn Fn() -> bool,
) -> Result<crate::verify::StreamCheck> {
    let (p, sync_errors, skipped) = pass_while(source, Options::default(), go_on, |_| Ok(()))?;
    Ok(crate::verify::StreamCheck {
        clean: is_clean(&p, sync_errors, skipped),
        first_problem: p.first_error.clone().or_else(|| p.emdf.first_error.clone()),
    })
}

/// Options of `decode` for AC-3 family streams.
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    pub format: Format,
    pub order: Order,
    pub dither: bool,
    pub tpnp: bool,
    pub ecpl_full: bool,
    /// Write only the independent substream's channels.
    pub core_only: bool,
}

/// `oadec decode` for AC-3 family streams: 32-bit float samples, as raw
/// little-endian PCM or as WAVE.
pub fn decode(source: &Source, output: &Path, opts: &DecodeOptions) -> Result<bool> {
    if matches!(opts.format, Format::Damf | Format::Adm) {
        bail!("object output of E-AC-3 JOC streams is not implemented yet");
    }
    let mut file = crate::decode::PcmFile::create(output, opts.format == Format::Wav)?;
    let report = decode_into(source, opts, &mut file, &mut std::io::stderr())?;
    report.print(false);
    Ok(report.findings().report_clean())
}

/// Decodes the programme of an AC-3 or E-AC-3 stream into `sink`, as 32-bit
/// float samples, and prints nothing. What the decode has to say while it
/// runs goes to `remarks`, a line at a time.
pub fn decode_into(
    source: &Source,
    opts: &DecodeOptions,
    sink: &mut dyn crate::decode::PcmSink,
    remarks: &mut dyn Write,
) -> Result<Report> {
    let started = Instant::now();
    let mut header_written = false;
    let mut order: Vec<usize> = Vec::new();
    let (p, sync_errors, skipped) = pass(
        source,
        Options {
            dither: opts.dither,
            tpnp: opts.tpnp,
            ecpl_full: opts.ecpl_full,
        },
        |frame| {
            let (chans, pcm) = output_channels(frame, opts.core_only);
            if !header_written {
                order = output_order(&chans, opts.order);
                let ordered: Vec<ChannelLoc> = order.iter().map(|&i| chans[i]).collect();
                let (mask, warning) = channel_mask(&ordered);
                if let Some(warning) = warning {
                    writeln!(remarks, "{warning}")?;
                }
                // the WAVE header waits for the first frame, which says what
                // the file holds
                sink.begin(crate::decode::WavSpec {
                    channels: chans.len() as u16,
                    rate: frame.core().header.sample_rate,
                    mask,
                    sample: crate::decode::WavSample::Float32,
                })?;
                header_written = true;
            }
            let n = frame.samples();
            let ordered: Vec<&[f32]> = order.iter().map(|&ch| pcm[ch]).collect();
            let mut buf = Vec::with_capacity(n * ordered.len() * 4);
            for i in 0..n {
                for c in &ordered {
                    buf.extend_from_slice(&c[i].to_le_bytes());
                }
            }
            sink.write(&buf)?;
            Ok(())
        },
    )?;
    if p.independent + p.dependent == 0 {
        // No whole syncframe: nothing to deliver and nothing to judge, which is
        // an unusable input (exit 2) and not a faulty run, as in a TrueHD decode.
        sink.nothing();
        return Err(crate::info::no_stream(source));
    }
    // a WAVE file an error drops on the way here closes itself
    sink.finish()?;
    Ok(Report {
        name: source.name().to_string(),
        pass: Box::new(p),
        sync_errors,
        skipped,
        elapsed: started.elapsed().as_secs_f64(),
        // a decode is given no threshold: the report of its pass has the
        // one `verify` has when it is given none
        fail_on: Severity::Error,
    })
}

/// Options of `compare` for AC-3 family streams.
#[derive(Debug, Clone, Copy)]
pub struct CompareOptions {
    pub order: Order,
    pub tpnp: bool,
    pub ecpl_full: bool,
    /// Compare only the independent substream's channels.
    pub core_only: bool,
    pub report: usize,
    /// Bytes to skip at the start of the reference.
    pub skip: u64,
    pub dither: bool,
    /// List the N (frame, block) pairs with the largest deviation.
    pub worst: usize,
}

/// Prints the side information of every block of one substream of frame group
/// `index`.
///
/// Groups are counted from 0 and are what clause E.1.3.1.2 describes: an
/// independent substream and the dependent substreams that immediately follow
/// it. `part` selects within the group, 0 being the independent substream, so
/// `--part 1` reads the dependent substream of a 7.1 stream, which is the only
/// way to check a `bsi` path by hand.
#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
pub fn blocks(source: &Source, index: u64, part: usize, detail: bool) -> Result<()> {
    let mut group = 0u64;
    let mut in_group = 0usize;
    let mut started = false;
    let mut found = false;
    for_each_frame(source, |offset, bytes, header| {
        if found {
            return Ok(());
        }
        if header.stream_type == oadec_eac3::StreamType::Dependent {
            if !started {
                return Ok(());
            }
            in_group += 1;
        } else {
            if started {
                group += 1;
            }
            started = true;
            in_group = 0;
        }
        if group != index || in_group != part {
            return Ok(());
        }
        found = true;
        let mut noise = oadec_eac3::Noise::default();
        println!(
            "group {index} part {part} at byte {offset}: {} bytes, {:?}",
            header.frame_bytes, header
        );
        let blocks = match oadec_eac3::Frame::parse_partial(bytes, &mut noise, Options::default()) {
            Ok(frame) => {
                println!("bsi: {:?}", frame.bsi);
                println!("coverage: {:?}", frame.coverage);
                println!(
                    "crc ok: {}, audio blocks end at bit {} of {}",
                    frame.crc_ok,
                    frame.end_bit,
                    header.frame_bytes * 8
                );
                frame.blocks
            }
            Err(partial) => {
                println!("bsi: {:?}", partial.bsi);
                println!(
                    "PARSE ERROR after {} blocks at bit {} of {}: {}",
                    partial.blocks.len(),
                    partial.bit,
                    header.frame_bytes * 8,
                    partial.error
                );
                if let Some(state) = &partial.state {
                    println!("state at failure: {state:?}");
                }
                partial.blocks
            }
        };
        for (b, block) in blocks.iter().enumerate() {
            println!("block {b}: blksw {:?}", block.blksw);
            println!("  {:?}", block.info);
            for (ch, c) in block.coeffs.iter().enumerate() {
                let peak = c.iter().fold(0.0f64, |m, v| m.max(v.abs()));
                let last = c.iter().rposition(|v| *v != 0.0).map_or(0, |i| i + 1);
                println!("  ch {ch}: peak {peak:.6}, {last} coefficients");
                if detail {
                    let exps = &block.info.exps[ch];
                    let bap = &block.info.bap[ch];
                    let shown = exps.len().min(48);
                    println!("    exps {:?}", &exps[..shown]);
                    println!("    bap  {:?}", &bap[..shown]);
                    println!(
                        "    coef {:?}",
                        c[..shown]
                            .iter()
                            .map(|v| format!("{v:.5}"))
                            .collect::<Vec<_>>()
                    );
                }
            }
        }
        Ok(())
    })?;
    if !found {
        bail!("group {index} part {part} not found");
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct ChannelStats {
    samples: u64,
    max_abs: f64,
    sum_sq_diff: f64,
    sum_sq_ref: f64,
    sum_ref_ours: f64,
    sum_sq_ours: f64,
    over_1e6: u64,
    over_1e4: u64,
    over_1e3: u64,
    over_1e2: u64,
}

/// `oadec compare` for AC-3 family streams against a 32-bit float
/// little-endian reference (for example FFmpeg with `-drc_scale 0`).
#[allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "printing is what this one is for"
)]
pub fn compare(source: &Source, reference: &Path, opts: &CompareOptions) -> Result<bool> {
    let started = Instant::now();
    let file = File::open(reference).with_context(|| format!("opening {}", reference.display()))?;
    let mut reader = BufReader::with_capacity(4 << 20, file);
    if opts.skip > 0 {
        reader.seek(SeekFrom::Start(opts.skip))?;
    }
    let mut order: Vec<usize> = Vec::new();
    let mut stats: Vec<ChannelStats> = Vec::new();
    let mut names: Vec<&str> = Vec::new();
    let mut ref_exhausted = false;
    let mut ours_samples: u64 = 0;
    let mut ref_samples: u64 = 0;
    let mut reported = 0usize;
    let mut rbuf: Vec<u8> = Vec::new();
    let mut frame_index: u64 = 0;
    let mut worst_list: Vec<(f64, u64, usize, usize)> = Vec::new();
    let (p, sync_errors, skipped) = pass(
        source,
        Options {
            dither: opts.dither,
            tpnp: opts.tpnp,
            ecpl_full: opts.ecpl_full,
        },
        |frame| {
            let (chans, pcm) = output_channels(frame, opts.core_only);
            if order.is_empty() {
                names = chans.iter().map(|c| c.name()).collect();
                order = output_order(&chans, opts.order);
                stats = vec![ChannelStats::default(); order.len()];
            }
            let n = frame.samples();
            ours_samples += n as u64;
            if ref_exhausted {
                return Ok(());
            }
            let need = n * order.len() * 4;
            rbuf.resize(need, 0);
            let mut got = 0;
            while got < need {
                let k = reader.read(&mut rbuf[got..])?;
                if k == 0 {
                    ref_exhausted = true;
                    break;
                }
                got += k;
            }
            let frames_avail = got / (order.len() * 4);
            ref_samples += frames_avail as u64;
            let mut block_worst: Vec<(f64, usize)> = vec![(0.0, 0); frames_avail.div_ceil(256)];
            for i in 0..frames_avail {
                for (k, &ch) in order.iter().enumerate() {
                    let o = (i * order.len() + k) * 4;
                    let r = f64::from(f32::from_le_bytes([
                        rbuf[o],
                        rbuf[o + 1],
                        rbuf[o + 2],
                        rbuf[o + 3],
                    ]));
                    let v = f64::from(pcm[ch][i]);
                    let s = &mut stats[k];
                    let diff = (v - r).abs();
                    if diff > block_worst[i / 256].0 {
                        block_worst[i / 256] = (diff, ch);
                    }
                    s.samples += 1;
                    s.max_abs = s.max_abs.max(diff);
                    s.sum_sq_diff += diff * diff;
                    s.sum_sq_ref += r * r;
                    s.sum_sq_ours += v * v;
                    s.sum_ref_ours += r * v;
                    if diff > 1e-6 {
                        s.over_1e6 += 1;
                    }
                    if diff > 1e-4 {
                        s.over_1e4 += 1;
                    }
                    if diff > 1e-3 {
                        s.over_1e3 += 1;
                        if reported < opts.report {
                            reported += 1;
                            eprintln!(
                                "mismatch: sample {} channel {}: ours {v:.7} reference {r:.7}",
                                ours_samples - n as u64 + i as u64,
                                names[ch]
                            );
                        }
                    }
                    if diff > 1e-2 {
                        s.over_1e2 += 1;
                    }
                }
            }
            if opts.worst > 0 {
                for (b, &(d, ch)) in block_worst.iter().enumerate() {
                    worst_list.push((d, frame_index, b, ch));
                }
            }
            frame_index += 1;
            Ok(())
        },
    )?;
    if p.independent + p.dependent == 0 {
        // Nothing framed: the file is not a stream to judge but an unsupported
        // input, exit 2, as it is for every command that reports one.
        return Err(crate::info::no_stream(source));
    }
    let elapsed = started.elapsed().as_secs_f64();
    if opts.worst > 0 {
        worst_list.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        println!("Worst blocks:");
        for (d, f, b, ch) in worst_list.iter().take(opts.worst) {
            println!(
                "  frame {f} block {b} channel {}: max |diff| {d:.4e}",
                names[*ch]
            );
        }
    }
    print_pass(
        source.name(),
        &p,
        sync_errors,
        skipped,
        elapsed,
        false,
        Severity::Error,
    );
    let clean = findings(&p, sync_errors, skipped).report_clean();
    // drain the rest of the reference to learn its length
    let mut tail = [0u8; 1 << 16];
    let mut extra = 0u64;
    loop {
        let k = reader.read(&mut tail)?;
        if k == 0 {
            break;
        }
        extra += k as u64;
    }
    let ref_total = ref_samples + extra / (order.len().max(1) as u64 * 4);
    println!(
        "Lengths:           ours {} samples, reference {} samples{}",
        ours_samples,
        ref_total,
        if ours_samples == ref_total {
            " (equal)"
        } else {
            " (DIFFER)"
        }
    );
    println!(
        "Channel   max|diff|   rms diff     SNR dB    gain   >1e-6    >1e-4    >1e-3    >1e-2"
    );
    let mut worst = 0.0f64;
    let mut worst_snr = f64::INFINITY;
    for (k, &ch) in order.iter().enumerate() {
        let s = &stats[k];
        let n = s.samples.max(1) as f64;
        let rms = (s.sum_sq_diff / n).sqrt();
        let snr = if s.sum_sq_diff > 0.0 {
            10.0 * (s.sum_sq_ref / s.sum_sq_diff).log10()
        } else {
            f64::INFINITY
        };
        let gain = if s.sum_sq_ours > 0.0 {
            s.sum_ref_ours / s.sum_sq_ours
        } else {
            0.0
        };
        worst = worst.max(s.max_abs);
        worst_snr = worst_snr.min(snr);
        println!(
            "{:<8} {:>10.3e} {:>10.3e} {:>10.1} {:>7.4} {:>8} {:>8} {:>8} {:>8}",
            names[ch], s.max_abs, rms, snr, gain, s.over_1e6, s.over_1e4, s.over_1e3, s.over_1e2
        );
    }
    // Decoders differ by their dither sequences (clause 6.3.4), so equality is
    // judged on the SNR: 30 dB on every channel is far above any structural
    // decoding error and within the range two conforming decoders show.
    let equal_length = ours_samples == ref_total;
    let close = worst <= 1e-4;
    let dither_level = worst_snr >= 30.0;
    println!(
        "Result:            {}",
        if equal_length && close {
            "MATCH (within 1e-4)"
        } else if equal_length && dither_level {
            "MATCH (dither-level differences only, SNR at least 30 dB on every channel)"
        } else {
            "DIFFERENT"
        }
    );
    Ok(equal_length && (close || dither_level) && clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A substream that declares the JOC extension in its `addbsi` carries an
    /// EMDF container in every frame (TS 103 420 clauses 8.2 and 8.3.1), so a
    /// frame with no skip field at all has lost one. The scan returned before
    /// counting when the skip bytes totalled zero, which left `verify` clean
    /// over the loss while the object decode held the matrices of the frame
    /// before it. Round 7 of the review loop found it (R7F1).
    #[test]
    fn a_frame_without_skip_fields_loses_a_container_where_emdf_is_declared() {
        let mut declared = EmdfStats::default();
        declared.scan(7, &[], true, &mut Diag::default());
        assert_eq!(declared.container_errors, 1);
        assert_eq!(declared.first_missing, Some(7));
        assert_eq!(
            (declared.frames_with_skip, declared.skip_bytes),
            (0, 0),
            "there were no skip bytes to count"
        );

        // A frame of a stream that carries no EMDF has lost nothing: most
        // AC-3 and E-AC-3 frames carry no skip field, and two AC-3 clips of
        // the corpus fill theirs with something else entirely.
        let mut plain = EmdfStats::default();
        plain.scan(7, &[], false, &mut Diag::default());
        assert_eq!(plain.container_errors, 0);
        assert_eq!(plain.first_missing, None);
    }

    /// Every input the exit code is decided from, one at a time.
    ///
    /// This function had no test at all: a mutation pass short-circuited it to
    /// `true` and nothing failed. It is what turns a pass over a stream into an
    /// exit code, and the second defect of this round was that corruption
    /// reached the counters and not the exit. The list is the whole content, so
    /// the list is what is pinned.
    #[test]
    fn every_input_to_the_verdict_can_make_a_stream_unclean() {
        assert!(is_clean(&Pass::default(), 0, 0), "an empty pass is clean");
        assert!(
            !is_clean(&Pass::default(), 1, 0),
            "a sync error left it clean"
        );
        assert!(
            !is_clean(&Pass::default(), 0, 1),
            "skipped bytes left it clean"
        );

        type Set = fn(&mut Pass);
        let unclean: [(&str, Set); 12] = [
            ("decode_errors", |p| p.decode_errors = 1),
            ("crc_failures", |p| p.crc_failures = 1),
            ("tail_overruns", |p| p.tail_overruns = 1),
            ("emdf.oamd_errors", |p| p.emdf.oamd_errors = 1),
            ("emdf.oamd_unusable", |p| p.emdf.oamd_unusable = 1),
            ("emdf.joc_errors", |p| p.emdf.joc_errors = 1),
            ("emdf.joc_reserved_ext", |p| p.emdf.joc_reserved_ext = 1),
            ("emdf.payload_config_violations", |p| {
                p.emdf.payload_config_violations = 1
            }),
            ("complexity_mismatches", |p| p.complexity_mismatches = 1),
            ("emdf.joc_size_mismatch", |p| p.emdf.joc_size_mismatch = 1),
            ("emdf.container_errors", |p| {
                p.emdf.containers = 1;
                p.emdf.container_errors = 1
            }),
            ("program.dependent_dropped", |p| {
                p.program.dependent_dropped = 1
            }),
        ];
        for (name, set) in unclean {
            let mut pass = Pass::default();
            set(&mut pass);
            assert!(
                !is_clean(&pass, 0, 0),
                "{name} left the stream looking clean"
            );
        }

        // a fault in a dependent substream counts as much as one in the core
        for (name, set) in [
            (
                "decode_errors",
                (|s: &mut SubStats| s.decode_errors = 1) as fn(&mut SubStats),
            ),
            ("crc_failures", |s: &mut SubStats| s.crc_failures = 1),
            ("tail_overruns", |s: &mut SubStats| s.tail_overruns = 1),
        ] {
            let mut pass = Pass::default();
            let mut sub = SubStats::default();
            set(&mut sub);
            pass.subs.insert((1, 0), sub);
            assert!(
                !is_clean(&pass, 0, 0),
                "a dependent substream's {name} left the stream clean"
            );
        }

        // and a second programme is legal, so it does not
        let mut other = Pass::default();
        other.program.other_program_frames = 1;
        assert!(
            is_clean(&other, 0, 0),
            "a second programme should not make a stream unclean"
        );
    }

    /// Clause 8.3.2.2 of TS 103 420, both halves: the index equals the OAMD
    /// object total, and it is at most 16.
    #[test]
    fn the_complexity_index_is_the_oamd_object_total_and_at_most_16() {
        assert_eq!(complexity_problem(16, 16), None);
        assert_eq!(
            complexity_problem(12, 12),
            None,
            "pi-head-joc384 declares 12"
        );
        assert_eq!(
            complexity_problem(15, 16).as_deref(),
            Some("complexity_index_type_a 15 differs from the OAMD object total 16 (clause 8.3)")
        );
        assert_eq!(
            complexity_problem(17, 17).as_deref(),
            Some("complexity_index_type_a 17 exceeds 16 (clause 8.3)")
        );
    }

    /// Skip fields may carry other data: two AC-3 clips of the corpus fill them
    /// in some 1 100 frames without a single EMDF container. A frame whose skip
    /// fields hold no container that opens is a fault only in a stream that
    /// carries EMDF, where it means a container was lost; counted anywhere else
    /// it called clean AC-3 streams non-conformant.
    #[test]
    fn skip_fields_that_carry_no_emdf_are_not_a_fault() {
        let mut plain = Pass::default();
        plain.emdf.frames_with_skip = 1099;
        plain.emdf.container_errors = 1099;
        assert!(
            is_clean(&plain, 0, 0),
            "verify called a stream without EMDF non-conformant"
        );
        assert!(
            findings(&plain, 0, 0).is_clean(),
            "a delivery failed on a stream without EMDF"
        );
        let mut lost = Pass::default();
        lost.emdf.containers = 62;
        lost.emdf.container_errors = 1;
        assert!(!is_clean(&lost, 0, 0), "a lost container left verify clean");
        assert!(
            !findings(&lost, 0, 0).is_clean(),
            "a lost container left a delivery clean"
        );
    }

    /// A payload configuration outside table 56 and a complexity index that
    /// disagrees with the OAMD make a file non-conformant and change nothing
    /// that was delivered, so, like a tail overrun, they fail `verify` and not
    /// a delivery.
    #[test]
    fn the_checks_of_clause_8_fail_verify_and_not_a_delivery() {
        let mut table_56 = Pass::default();
        table_56.emdf.payload_config_violations = 1;
        let complexity = Pass {
            complexity_mismatches: 1,
            ..Pass::default()
        };
        for (name, pass) in [("table 56", table_56), ("complexity", complexity)] {
            assert!(!is_clean(&pass, 0, 0), "{name} left verify clean");
            let f = findings(&pass, 0, 0);
            assert!(f.is_clean(), "{name} failed a delivery: {f:?}");
        }
    }

    /// A frame that ends inside its own tail is out of spec and decodes to
    /// audio FFmpeg, Dolby and oadec agree on (`docs/eac3.md`), so a delivery
    /// counts and prints it without failing on it, while `verify` still calls
    /// the file non-conformant (`docs/exit-codes.md`). `findings` noted both
    /// tail counters, so `decode` and `compare` exited 7 on exactly the frame
    /// the policy was written about.
    #[test]
    fn a_tail_overrun_alone_fails_verify_and_not_a_delivery() {
        let core = Pass {
            tail_overruns: 1,
            first_error: Some("frame 224: the audio blocks end inside the frame tail".into()),
            ..Pass::default()
        };
        assert!(
            !is_clean(&core, 0, 0),
            "verify must still call the file non-conformant"
        );
        let f = findings(&core, 0, 0);
        assert!(
            f.is_clean(),
            "a delivery failed on a tail overrun alone: {f:?}"
        );

        let mut dependent = Pass::default();
        dependent.subs.insert(
            (1, 0),
            SubStats {
                tail_overruns: 1,
                ..SubStats::default()
            },
        );
        assert!(!is_clean(&dependent, 0, 0));
        let f = findings(&dependent, 0, 0);
        assert!(
            f.is_clean(),
            "a delivery failed on a dependent substream's tail overrun alone: {f:?}"
        );

        // beside a real fault the delivery fails, and names the fault first
        let both = Pass {
            tail_overruns: 1,
            crc_failures: 1,
            first_error: Some("frame 5: the audio blocks end inside the frame tail".into()),
            first_fault: Some("frame 22: CRC failure".into()),
            ..Pass::default()
        };
        let f = findings(&both, 0, 0);
        assert!(!f.is_clean());
        let shown = format!("{f:?}");
        assert!(
            shown.contains("frame 22: CRC failure") && !shown.contains("frame tail"),
            "the delivery's first problem is not its first fault: {shown}"
        );
    }

    /// The first frame group of the committed E-AC-3 fixture, decoded.
    fn first_group() -> ProgramFrame {
        first_groups(1).remove(0)
    }

    /// The first `n` frame groups of the committed E-AC-3 fixture, decoded.
    fn first_groups(n: usize) -> Vec<ProgramFrame> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/authored-scene.ec3"
        );
        let stream = std::fs::read(path).expect("the fixture");
        let mut dec = ProgramDecoder::new(Options::default());
        let mut groups = Vec::new();
        let mut at = 0;
        while groups.len() < n {
            let header = FrameHeader::parse(&stream[at..]).expect("a syncframe");
            dec.push(&stream[at..at + header.frame_bytes], &header)
                .expect("a frame that decodes");
            if let Some(frame) = dec.pop() {
                groups.push(frame);
            }
            at += header.frame_bytes;
        }
        groups
    }

    /// The rules that fired, with how often. Each finding of these tests is
    /// one event, so a rule weighs as much as it counts.
    fn fired(diag: &Diag) -> Vec<(RuleId, u64)> {
        diag.iter()
            .map(|(rule, t)| {
                assert_eq!(t.magnitude, t.count, "{rule:?}");
                (rule, t.count)
            })
            .collect()
    }

    /// What assembling a programme counts is filed under the rule each
    /// counter is, where the frame that raised it is, and once: a counter
    /// that has not moved since it was last looked at files nothing.
    #[test]
    fn what_the_programme_counts_is_filed_under_its_rule_where_it_was_counted() {
        type Raise = fn(&mut oadec_eac3::ProgramStats);
        let counters: [(RuleId, Raise); 9] = [
            (RuleId::ProgramDependentDropped, |s| {
                s.dependent_dropped += 2
            }),
            (RuleId::ProgramOrphanDependent, |s| s.orphan_dependents += 2),
            (RuleId::ProgramMisaligned, |s| s.misaligned += 2),
            (RuleId::ProgramLocation, |s| s.location_errors += 2),
            (RuleId::ProgramOverCapacity, |s| s.over_capacity += 2),
            (RuleId::ProgramLayoutChange, |s| s.layout_changes += 2),
            (RuleId::ProgramDuplicateFrame, |s| {
                s.duplicate_substream_frames += 2
            }),
            (RuleId::ProgramStalledSubstream, |s| {
                s.stalled_substreams += 2
            }),
            (RuleId::ProgramMissingFrame, |s| {
                s.missing_substream_frames += 2
            }),
        ];
        for (rule, raise) in counters {
            let mut stats = oadec_eac3::ProgramStats::default();
            let mut seen = ProgramSeen::default();
            let mut diag = Diag::default();
            seen.file(&stats, Location::byte(0), &mut diag);
            assert!(diag.is_empty(), "{rule:?}: nothing was counted yet");
            raise(&mut stats);
            seen.file(&stats, Location::byte(4096), &mut diag);
            assert_eq!(fired(&diag), [(rule, 2)], "{rule:?}");
            assert_eq!(diag.get(rule).unwrap().first.at, Location::byte(4096));
            // looked at again, the same count is no new finding
            seen.file(&stats, Location::byte(8192), &mut diag);
            assert_eq!(fired(&diag), [(rule, 2)], "{rule:?}");
            raise(&mut stats);
            seen.file(&stats, Location::byte(8192), &mut diag);
            assert_eq!(fired(&diag), [(rule, 4)], "{rule:?}");
            assert_eq!(diag.get(rule).unwrap().last, Location::byte(8192));
        }

        // A frame that would not decode: of the independent substream it is
        // what stops a decode, of a dependent one it is that substream's.
        let mut stats = oadec_eac3::ProgramStats::default();
        let mut seen = ProgramSeen::default();
        let mut diag = Diag::default();
        stats.decode_errors.insert((0, 0), 1);
        stats.decode_errors.insert((1, 3), 2);
        seen.file(&stats, Location::byte(64), &mut diag);
        assert_eq!(
            fired(&diag),
            [(RuleId::FrameDecode, 1), (RuleId::SubstreamDecode, 2)]
        );
        assert_eq!(
            diag.get(RuleId::FrameDecode).unwrap().first.at,
            Location::byte(64)
        );
        assert_eq!(
            diag.get(RuleId::SubstreamDecode).unwrap().first.at,
            Location::byte(64).in_substream(3)
        );
        seen.file(&stats, Location::byte(128), &mut diag);
        assert_eq!(
            fired(&diag),
            [(RuleId::FrameDecode, 1), (RuleId::SubstreamDecode, 2)]
        );
    }

    /// A rule and its counter are one fact told twice. Each of the 26 rules
    /// is held to the counter it is counted under: a counter raised without
    /// its finding is a disagreement, and so is a finding without its
    /// counter, and with both there is none.
    #[test]
    fn every_rule_is_held_to_its_counter() {
        type Raise = fn(&mut Pass);
        let table: [(RuleId, Raise); 29] = [
            (RuleId::FrameDecode, |p| p.decode_errors += 1),
            (RuleId::FrameCrc, |p| p.crc_failures += 1),
            (RuleId::FrameTailOverrun, |p| p.tail_overruns += 1),
            (RuleId::SubstreamDecode, |p| {
                p.subs.entry((1, 0)).or_default().decode_errors += 1
            }),
            (RuleId::SubstreamCrc, |p| {
                p.subs.entry((1, 0)).or_default().crc_failures += 1
            }),
            (RuleId::SubstreamTailOverrun, |p| {
                p.subs.entry((1, 0)).or_default().tail_overruns += 1
            }),
            (RuleId::ProgramDependentDropped, |p| {
                p.program.dependent_dropped += 1
            }),
            (RuleId::ProgramOrphanDependent, |p| {
                p.program.orphan_dependents += 1
            }),
            (RuleId::ProgramMisaligned, |p| p.program.misaligned += 1),
            (RuleId::ProgramLocation, |p| p.program.location_errors += 1),
            (RuleId::ProgramOverCapacity, |p| {
                p.program.over_capacity += 1
            }),
            (RuleId::ProgramLayoutChange, |p| {
                p.program.layout_changes += 1
            }),
            (RuleId::ProgramDuplicateFrame, |p| {
                p.program.duplicate_substream_frames += 1
            }),
            (RuleId::ProgramStalledSubstream, |p| {
                p.program.stalled_substreams += 1
            }),
            (RuleId::ProgramMissingFrame, |p| {
                p.program.missing_substream_frames += 1
            }),
            (RuleId::EmdfMissingContainer, |p| {
                p.emdf.containers = 1;
                p.emdf.container_errors += 1
            }),
            (RuleId::EmdfPayloadConfig, |p| {
                p.emdf.payload_config_violations += 1
            }),
            (RuleId::BsiComplexityIndex, |p| p.complexity_mismatches += 1),
            (RuleId::OamdParse, |p| p.emdf.oamd_errors += 1),
            (RuleId::OamdUnusable, |p| p.emdf.oamd_unusable += 1),
            (RuleId::OamdStaleSize, |p| p.emdf.oamd_stale_sizes += 1),
            (RuleId::JocParse, |p| p.emdf.joc_errors += 1),
            (RuleId::JocReservedExtConfig, |p| {
                p.emdf.joc_reserved_ext += 1
            }),
            (RuleId::JocSizeMismatch, |p| p.emdf.joc_size_mismatch += 1),
            (RuleId::EmdfSyntaxVersion, |p| {
                *p.emdf.protection.versions.entry(1).or_default() += 1
            }),
            (RuleId::EmdfProtectionReserved, |p| {
                p.emdf.protection.reserved_primary_code += 1
            }),
            (RuleId::EmdfProtectionChanged, |p| {
                p.emdf.protection.changes += 1
            }),
            (RuleId::JocSequenceRestart, |p| p.emdf.joc_seq_restarts += 1),
            (RuleId::JocSequenceGap, |p| p.emdf.joc_seq_gaps += 1),
        ];
        assert_eq!(disagreement(&Pass::default(), 0, 0), None);
        let mut held = vec![RuleId::FrameSync, RuleId::FrameSkipped];
        for (rule, raise) in table {
            let mut pass = Pass::default();
            raise(&mut pass);
            let said = disagreement(&pass, 0, 0).expect("a counter without its finding");
            assert!(said.starts_with(rule.info().id), "{rule:?}: {said}");
            pass.diag.add(rule, Location::default(), 1, String::new);
            assert_eq!(disagreement(&pass, 0, 0), None, "{rule:?}");
            // a finding more than the counter counted
            pass.diag.add(rule, Location::default(), 1, String::new);
            assert!(disagreement(&pass, 0, 0).is_some(), "{rule:?}");
            held.push(rule);
        }
        // the two the walk over the bytes counts, the second by its bytes
        let mut pass = Pass::default();
        assert!(disagreement(&pass, 1, 0).unwrap().starts_with("frame.sync"));
        pass.diag
            .add(RuleId::FrameSync, Location::byte(0), 1, String::new);
        assert_eq!(disagreement(&pass, 1, 0), None);
        assert!(
            disagreement(&pass, 1, 7)
                .unwrap()
                .starts_with("frame.skipped")
        );
        pass.diag
            .add(RuleId::FrameSkipped, Location::byte(0), 7, String::new);
        assert_eq!(disagreement(&pass, 1, 7), None);
        // and that was every rule of the catalog
        held.sort();
        assert_eq!(held, RuleId::ALL);
        // a rule that fails no stream leaves the verdict alone, and the
        // counters agree: a warning is no disagreement
        let mut pass = Pass::default();
        pass.emdf.oamd_stale_sizes = 1;
        pass.diag
            .add(RuleId::OamdStaleSize, Location::default(), 1, String::new);
        assert!(is_clean(&pass, 0, 0) && pass.diag.passes(Severity::Error));
        assert_eq!(disagreement(&pass, 0, 0), None);
    }

    /// What a frame group shows of its frames is filed at that group: a CRC
    /// that does not hold and blocks that end inside the tail, of the
    /// independent substream and of a dependent one, which is told apart by
    /// its substream.
    #[test]
    fn a_frame_and_a_dependent_frame_are_filed_at_their_group() {
        let mut frame = first_group();
        let index = frame.index;
        let mut pass = Pass::default();
        let mut diag = Diag::default();
        account(&mut pass, &frame, &mut |_| Ok(()), &mut diag).unwrap();
        assert!(diag.is_empty(), "the fixture is sound: {:?}", fired(&diag));

        frame.parts[0].decoded.crc_ok = false;
        frame.parts[0].decoded.tail_overrun = true;
        let mut pass = Pass::default();
        let mut diag = Diag::default();
        account(&mut pass, &frame, &mut |_| Ok(()), &mut diag).unwrap();
        assert_eq!(
            fired(&diag),
            [(RuleId::FrameCrc, 1), (RuleId::FrameTailOverrun, 1)]
        );
        assert_eq!((pass.crc_failures, pass.tail_overruns), (1, 1));
        for rule in [RuleId::FrameCrc, RuleId::FrameTailOverrun] {
            assert_eq!(diag.get(rule).unwrap().first.at, frame_at(index));
        }

        // the same two of a dependent substream, here substream 2
        let part = oadec_eac3::Part {
            key: (1, 2),
            locations: Vec::new(),
            lfe_implied: false,
            decoded: frame.parts[0].decoded.clone(),
        };
        let mut sub = SubStats::default();
        let mut diag = Diag::default();
        account_sub(&mut sub, 41, &part, &mut diag);
        assert_eq!(
            fired(&diag),
            [(RuleId::SubstreamCrc, 1), (RuleId::SubstreamTailOverrun, 1)]
        );
        assert_eq!((sub.crc_failures, sub.tail_overruns), (1, 1));
        for rule in [RuleId::SubstreamCrc, RuleId::SubstreamTailOverrun] {
            assert_eq!(
                diag.get(rule).unwrap().first.at,
                frame_at(41).in_substream(2)
            );
        }
    }

    /// A frame without a container is filed where the stream carries EMDF,
    /// and nowhere else: skip fields may hold other things, and two AC-3
    /// clips of the corpus fill theirs in some 1 100 frames without one
    /// container.
    #[test]
    fn a_frame_without_a_container_is_filed_only_where_the_stream_carries_emdf() {
        let mut stats = EmdfStats::default();
        stats.scan(3, &[vec![0u8; 16]], false, &mut Diag::default());
        stats.scan(9, &[vec![0u8; 16]], false, &mut Diag::default());
        assert_eq!(stats.missing_frames, [3, 9]);
        assert_eq!((stats.container_errors, stats.missing_containers()), (2, 0));
        let mut diag = Diag::default();
        stats.file_missing(&mut diag);
        assert!(
            diag.is_empty(),
            "the stream carries no EMDF: {:?}",
            fired(&diag)
        );

        // one container that opens anywhere in the stream, and they are faults
        stats.containers = 1;
        assert_eq!(stats.missing_containers(), 2);
        stats.file_missing(&mut diag);
        assert_eq!(fired(&diag), [(RuleId::EmdfMissingContainer, 2)]);
        let filed = diag.get(RuleId::EmdfMissingContainer).unwrap();
        assert_eq!((filed.first.at, filed.last), (frame_at(3), frame_at(9)));

        // and so they are where the substream declares the extension and not
        // one container opens
        let mut declared = EmdfStats::default();
        declared.scan(4, &[], true, &mut Diag::default());
        declared.declares_joc = true;
        let mut diag = Diag::default();
        declared.file_missing(&mut diag);
        assert_eq!(fired(&diag), [(RuleId::EmdfMissingContainer, 1)]);
    }

    /// What a container declares of itself is counted and filed at its frame,
    /// and fails nothing: a syntax version other than 0, the reserved code
    /// for the length of its primary word, and a key identifier or a length
    /// of a word that the container before did not declare. No stream of the
    /// tests has any of the three, so the containers are written by hand,
    /// without a payload and with words of zeroes, which are never read.
    #[test]
    fn what_a_container_declares_of_itself_is_filed_at_its_frame() {
        use crate::protection::bare_container;
        let mut stats = EmdfStats::default();
        let mut diag = Diag::default();
        let mut scan = |frame: u64, version, key, primary, secondary| {
            stats.scan(
                frame,
                &[bare_container(version, key, primary, secondary)],
                false,
                &mut diag,
            );
        };
        // one key and one length throughout: nothing to say
        scan(0, 0, 0, 1, 0);
        scan(1, 0, 0, 1, 0);
        // another key
        scan(2, 0, 5, 1, 0);
        scan(3, 0, 5, 1, 0);
        // another version, and the first key again
        scan(4, 1, 0, 1, 0);
        // the reserved code, which is another length as well
        scan(5, 0, 0, 0, 0);
        assert_eq!(
            fired(&diag),
            [
                (RuleId::EmdfSyntaxVersion, 1),
                (RuleId::EmdfProtectionReserved, 1),
                (RuleId::EmdfProtectionChanged, 3),
            ]
        );
        let at = |rule| {
            let filed = diag.get(rule).unwrap();
            (filed.first.at, filed.last)
        };
        assert_eq!(at(RuleId::EmdfSyntaxVersion), (frame_at(4), frame_at(4)));
        assert_eq!(
            at(RuleId::EmdfProtectionReserved),
            (frame_at(5), frame_at(5))
        );
        assert_eq!(
            at(RuleId::EmdfProtectionChanged),
            (frame_at(2), frame_at(5))
        );
        let p = &stats.protection;
        assert_eq!(
            (
                p.containers,
                p.other_versions(),
                p.reserved_primary_code,
                p.changes
            ),
            (6, 1, 1, 3)
        );
        assert_eq!(stats.containers, 6);
        // counted, and no fault: each is information
        assert!(diag.passes(Severity::Warning) && !diag.passes(Severity::Info));
        let mut pass = Pass {
            emdf: stats,
            diag,
            ..Pass::default()
        };
        assert!(is_clean(&pass, 0, 0));
        assert_eq!(disagreement(&pass, 0, 0), None);
        // a container the findings do not know of is a disagreement
        pass.emdf.protection.changes += 1;
        assert!(
            disagreement(&pass, 0, 0)
                .unwrap()
                .starts_with("emdf.protection_changed")
        );
    }

    /// The sequence counter of the JOC payloads is followed from one payload
    /// to the next (TS 103 420 clause 6.3.3.3). A counter of 0 after the
    /// first payload is the first frame after a splice; one that is neither
    /// 0 nor the one after the counter before it says that frames are
    /// missing, repeated or out of order. Both are counted and filed at the
    /// frame, and fail nothing. The payloads are those of the fixture, put
    /// to the pass in another order than the stream has them.
    #[test]
    fn the_joc_sequence_counter_is_followed_from_payload_to_payload() {
        let groups = first_groups(4);
        let fields = |i: usize| &groups[i].metadata_part().decoded.skip_fields;
        // in the order of the stream there is nothing to file, and the
        // stream starts where a bitstream starts
        let mut stats = EmdfStats::default();
        let mut diag = Diag::default();
        for i in 0..4 {
            stats.scan(i as u64, fields(i), true, &mut diag);
        }
        assert!(diag.is_empty(), "{:?}", fired(&diag));
        assert_eq!(
            (
                stats.joc_seq_first,
                stats.joc_seq_last,
                stats.joc_seq_restarts,
                stats.joc_seq_gaps
            ),
            (Some(0), Some(3), 0, 0)
        );

        // a stream cut out of the middle of another starts where it was
        // cut, and there is nothing to file in that
        let mut cut = EmdfStats::default();
        let mut none = Diag::default();
        cut.scan(0, fields(2), true, &mut none);
        cut.scan(1, fields(3), true, &mut none);
        assert!(none.is_empty(), "{:?}", fired(&none));
        assert_eq!(
            (
                cut.joc_seq_first,
                cut.joc_seq_last,
                cut.joc_seq_restarts,
                cut.joc_seq_gaps
            ),
            (Some(2), Some(3), 0, 0)
        );

        // the stream again behind itself: a splice
        stats.scan(4, fields(0), true, &mut diag);
        // 2 after 0: one frame is missing
        stats.scan(5, fields(2), true, &mut diag);
        // 2 after 2: a frame twice
        stats.scan(6, fields(2), true, &mut diag);
        // 3 after 2 follows
        stats.scan(7, fields(3), true, &mut diag);
        // 1 after 3: out of order
        stats.scan(8, fields(1), true, &mut diag);
        assert_eq!(
            fired(&diag),
            [(RuleId::JocSequenceRestart, 1), (RuleId::JocSequenceGap, 3)]
        );
        let restart = diag.get(RuleId::JocSequenceRestart).unwrap();
        assert_eq!((restart.first.at, restart.last), (frame_at(4), frame_at(4)));
        let gap = diag.get(RuleId::JocSequenceGap).unwrap();
        assert_eq!((gap.first.at, gap.last), (frame_at(5), frame_at(8)));
        assert_eq!(
            (
                stats.joc_seq_first,
                stats.joc_seq_last,
                stats.joc_seq_restarts,
                stats.joc_seq_gaps,
                stats.joc_seq_zero
            ),
            (Some(0), Some(1), 1, 3, 2)
        );
        assert!(diag.passes(Severity::Warning) && !diag.passes(Severity::Info));
        let pass = Pass {
            emdf: stats,
            diag,
            ..Pass::default()
        };
        assert_eq!(disagreement(&pass, 0, 0), None);
    }

    /// An element of an Object Audio Metadata payload whose declared size is
    /// not its own is counted and fails nothing. No stream of the tests has
    /// one, so one is made: the bits of the first frame's skip fields are
    /// changed one at a time until a payload parses with such an element and
    /// nothing else wrong.
    #[test]
    fn a_stale_element_size_is_a_warning_filed_at_its_frame() {
        let frame = first_group();
        let fields = &frame.metadata_part().decoded.skip_fields;
        let mut sound = EmdfStats::default();
        let mut diag = Diag::default();
        sound.scan(5, fields, true, &mut diag);
        assert!(
            diag.is_empty() && sound.oamd_ok == 1,
            "the fixture is sound"
        );

        let bits: usize = fields.iter().map(|f| f.len() * 8).sum();
        let mut made = 0;
        for bit in 0..bits {
            let mut changed = fields.clone();
            let mut at = bit;
            for field in &mut changed {
                if at < field.len() * 8 {
                    field[at / 8] ^= 0x80 >> (at % 8);
                    break;
                }
                at -= field.len() * 8;
            }
            let mut stats = EmdfStats::default();
            let mut diag = Diag::default();
            stats.scan(5, &changed, true, &mut diag);
            if stats.oamd_stale_sizes == 0 {
                continue;
            }
            // whatever else the change did, the rule fired as often as the
            // counter counted, at the frame
            let filed = diag.get(RuleId::OamdStaleSize).expect("the rule");
            assert_eq!(filed.count, stats.oamd_stale_sizes, "bit {bit}");
            assert_eq!(filed.magnitude, stats.oamd_stale_sizes, "bit {bit}");
            assert_eq!(filed.first.at, frame_at(5));
            if fired(&diag).len() == 1 {
                // and alone it fails nothing
                assert!(diag.passes(Severity::Error) && !diag.passes(Severity::Warning));
                made += 1;
            }
        }
        assert!(
            made > 0,
            "no bit of the skip fields makes a stale size alone"
        );
    }
}
