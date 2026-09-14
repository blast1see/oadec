//! E-AC-3 / AC-3 commands: `info`, `verify`, `decode` and `compare` for
//! streams that start with the `0x0B77` sync word.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_eac3::{
    ChannelLoc, Coverage, Decoder, FrameHeader, Options, ProgramDecoder, ProgramFrame, Syntax,
    find_sync,
};
use oadec_emdf::container::{self, PAYLOAD_ID_JOC, PAYLOAD_ID_OAMD};
use oadec_emdf::joc::{Joc, Slope, SparseReading};
use oadec_emdf::oamd::Oamd;
use serde_json::{Value, json};

use crate::decode::{Format, Order, format_duration};
use crate::integrity::Findings;

/// Whether the file starts with an AC-3 family sync word.
pub fn is_eac3(path: &Path) -> Result<bool> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
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
    path: &Path,
    mut on_frame: impl FnMut(u64, &[u8], &FrameHeader) -> Result<()>,
) -> Result<(u64, u64, u64)> {
    let mut file = BufReader::with_capacity(4 << 20, File::open(path)?);
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
                skipped += buf.len() as u64;
                break;
            }
            continue;
        }
        let Some(pos) = find_sync(&buf, 0) else {
            skipped += buf.len() as u64;
            base += buf.len() as u64;
            buf.clear();
            if eof {
                break;
            }
            continue;
        };
        if pos > 0 {
            sync_errors += 1;
            skipped += pos as u64;
            buf.drain(..pos);
            base += pos as u64;
            continue;
        }
        let header = match FrameHeader::parse(&buf) {
            Ok(h) => h,
            Err(_) => {
                sync_errors += 1;
                skipped += 1;
                buf.drain(..1);
                base += 1;
                continue;
            }
        };
        if buf.len() < header.frame_bytes {
            if eof {
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
fn channel_mask(chans: &[ChannelLoc]) -> u32 {
    let mask = chans.iter().fold(0, |a, c| a | c.wave_mask());
    if mask.count_ones() as usize == chans.len() {
        mask
    } else {
        let unnamed: Vec<&str> = chans
            .iter()
            .filter(|c| c.wave_mask() == 0)
            .map(|c| c.name())
            .collect();
        eprintln!(
            "warning: WAVE has no channel mask bit for {}; writing an unassigned mask",
            unnamed.join(", ")
        );
        0
    }
}

#[derive(Debug, Default)]
struct EmdfStats {
    frames_with_skip: u64,
    skip_bytes: u64,
    containers: u64,
    /// Frames whose skip fields held no parsable container.
    container_errors: u64,
    /// Sync words inside payload bytes that did not start a container.
    false_syncs: u64,
    payload_ids: BTreeMap<u32, u64>,
    oamd_ok: u64,
    oamd_errors: u64,
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
    joc_size_mismatch: u64,
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
    joc_clipgain: BTreeMap<u32, u64>,
    /// Per clip gain bucket: the largest and the summed peak matrix
    /// coefficient, and how many payloads went in.
    joc_peak: BTreeMap<u32, (f64, f64, u64)>,
    /// The clip gain of the payload the last scan saw, so the caller can pair
    /// it with the decoded samples of the same frame.
    last_clipgain: Option<f64>,
}

impl EmdfStats {
    /// Records where a rare syntax branch occurred, up to a cap: the counts
    /// answer "does anything use this", the frame numbers answer "where do I
    /// cut a clip that does".
    fn note_rare(&mut self, what: &'static str, frame: u64) {
        let seen = self.joc_rare_frames.entry(what).or_default();
        if seen.len() < 64 && seen.last() != Some(&frame) {
            seen.push(frame);
        }
    }

    fn scan(&mut self, frame_index: u64, skip_fields: &[Vec<u8>]) {
        self.last_clipgain = None;
        let total: usize = skip_fields.iter().map(Vec::len).sum();
        if total == 0 {
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
                    for p in &c.payloads {
                        *self.payload_ids.entry(p.id).or_default() += 1;
                        if p.id == PAYLOAD_ID_JOC {
                            self.joc += 1;
                            match Joc::parse(&p.data, SparseReading::default()) {
                                Ok(j) => {
                                    self.joc_ok += 1;
                                    if !j.size_ok(p.data.len()) {
                                        self.joc_size_mismatch += 1;
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
                                Err(e) => {
                                    self.joc_errors += 1;
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
            if self.first_error.is_none() {
                self.first_error = Some(format!(
                    "frame {frame_index}: no EMDF container in the skip fields"
                ));
            }
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
    coverage: Coverage,
    emdf: EmdfStats,
    first: Option<(FrameHeader, oadec_eac3::Bsi)>,
    dialnorm: BTreeMap<u8, u64>,
    aht_frames: u64,
    spx_frames: u64,
    ecpl_frames: u64,
    tpnp_frames: u64,
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
fn account_sub(sub: &mut SubStats, index: u64, part: &oadec_eac3::Part) {
    let d = &part.decoded;
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
        if sub.first_error.is_none() {
            sub.first_error = Some(format!("group {index}: CRC failure"));
        }
    }
    if d.tail_overrun {
        sub.tail_overruns += 1;
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
    path: &Path,
    opts: Options,
    mut on_pcm: impl FnMut(&ProgramFrame) -> Result<()>,
) -> Result<(Pass, u64, u64)> {
    let mut dec = ProgramDecoder::new(opts);
    let mut p = Pass::default();
    let (frames, sync_errors, skipped) = for_each_frame(path, |_offset, bytes, header| {
        let key = (header.stream_type as u8, header.substream_id);
        *p.substreams.entry(key).or_default() += 1;
        match header.stream_type {
            oadec_eac3::StreamType::Dependent => p.dependent += 1,
            _ => p.independent += 1,
        }
        dec.push(bytes, header)?;
        while let Some(frame) = dec.pop() {
            account(&mut p, &frame, &mut on_pcm)?;
        }
        Ok(())
    })?;
    dec.finish()?;
    while let Some(frame) = dec.pop() {
        account(&mut p, &frame, &mut on_pcm)?;
    }
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
    let _ = frames;
    Ok((p, sync_errors, skipped))
}

/// Folds one decoded frame into the pass statistics.
fn account(
    p: &mut Pass,
    frame: &ProgramFrame,
    on_pcm: &mut impl FnMut(&ProgramFrame) -> Result<()>,
) -> Result<()> {
    let index = frame.index;
    let d = frame.core();
    p.frames += 1;
    for part in frame.parts.iter().skip(1) {
        account_sub(p.subs.entry(part.key).or_default(), index, part);
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
        if p.first_error.is_none() {
            p.first_error = Some(format!(
                "frame {index}: the audio blocks end inside the frame tail"
            ));
        }
    }
    if !d.crc_ok {
        p.crc_failures += 1;
        if p.first_error.is_none() {
            p.first_error = Some(format!("frame {index}: CRC failure"));
        }
    }
    // TS 103 420 clause 8.2: with dependent substreams present the EMDF
    // container carrying OAMD and JOC is in the last dependent substream. With
    // none, `metadata_part` is the independent substream and this is exactly
    // what it was before.
    p.emdf
        .scan(index, &frame.metadata_part().decoded.skip_fields);
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

/// Whether a pass found nothing wrong: every frame decoded, every CRC and
/// every metadata payload checked out, and no byte of the file was skipped.
fn is_clean(p: &Pass, sync_errors: u64, skipped: u64) -> bool {
    p.decode_errors == 0
        && p.crc_failures == 0
        && p.tail_overruns == 0
        && sync_errors == 0
        && skipped == 0
        && p.emdf.oamd_errors == 0
        && p.emdf.joc_errors == 0
        && p.emdf.joc_size_mismatch == 0
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

/// The same faults `is_clean` weighs, in the form a delivery path reports.
fn findings(p: &Pass, sync_errors: u64, skipped: u64) -> Findings {
    let mut f = Findings::default();
    f.note(p.decode_errors, "frames failed to decode");
    f.note(p.crc_failures, "CRC failures");
    f.note(p.tail_overruns, "frames ending inside the frame tail");
    f.note(sync_errors, "sync errors");
    f.note(skipped, "bytes skipped");
    f.note(
        p.emdf.oamd_errors,
        "Object Audio Metadata payloads failed to parse",
    );
    f.note(p.emdf.joc_errors, "JOC payloads failed to parse");
    f.note(
        p.emdf.joc_size_mismatch,
        "JOC payloads whose declared size was wrong",
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
        f.note(
            sub.tail_overruns,
            &format!("frames of dependent substream {id} ending inside the frame tail"),
        );
    }
    f.first_problem(p.first_error.as_deref());
    f
}

fn print_pass(path: &Path, p: &Pass, sync_errors: u64, skipped: u64, elapsed: f64, json: bool) {
    let Some((h, bsi)) = &p.first else {
        eprintln!("{}: no decodable frames", path.display());
        return;
    };
    let names = p.layout.as_ref().map_or_else(
        || Decoder::channel_names(h),
        oadec_eac3::ProgramLayout::names,
    );
    let duration = p.samples as f64 / f64::from(h.sample_rate);
    let joc = bsi.joc_extension();
    if json {
        let e = &p.emdf;
        let payload_ids: serde_json::Map<String, Value> = e
            .payload_ids
            .iter()
            .map(|(k, v)| (k.to_string(), Value::from(*v)))
            .collect();
        let report = json!({
            "file": path.display().to_string(),
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
            "clean": is_clean(p, sync_errors, skipped),
            "failures": {
                "sync_errors": sync_errors,
                "skipped_bytes": skipped,
                "decode_errors": p.decode_errors,
                "crc_failures": p.crc_failures,
                "tail_overruns": p.tail_overruns,
                "oamd_errors": e.oamd_errors,
                "joc_errors": e.joc_errors,
                "joc_size_mismatches": e.joc_size_mismatch,
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
            },
            "program": program_parts(p, h),
            "other_program_frames": p.program.other_program_frames,
            "coverage": coverage_list(&p.coverage),
            "aht_frames": p.aht_frames,
            "spx_frames": p.spx_frames,
            "ecpl_frames": p.ecpl_frames,
            "tpnp_frames": p.tpnp_frames,
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
            },
            "joc": (e.joc > 0).then(|| json!({
                "parsed": e.joc_ok,
                "errors": e.joc_errors,
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
        println!("{report}");
        return;
    }
    println!("File:              {}", path.display());
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
                "                     {} frames merged, {} CRC failures, {} decode errors, {} bits unread",
                sub.frames, sub.crc_failures, sub.decode_errors, slack
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
            bsi.addbsi.len()
        ),
        None => println!(
            "JOC extension:     none (addbsi {} bytes)",
            bsi.addbsi.len()
        ),
    }
    println!(
        "Coding tools:      {} (AHT in {} frames, spectral extension in {}, enhanced coupling in {}, transient pre-noise in {})",
        coverage_list(&p.coverage).join(", "),
        p.aht_frames,
        p.spx_frames,
        p.ecpl_frames,
        p.tpnp_frames
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
    println!(
        "Metadata:          {} OAMD payloads ({} errors), {} JOC payloads",
        p.emdf.oamd_ok, p.emdf.oamd_errors, p.emdf.joc
    );
    if p.emdf.joc > 0 {
        let e = &p.emdf;
        println!(
            "JOC parse:         {} ok, {} errors, {} size mismatches, {} non-zero paddings; dmx configs {:?}; objects per payload {:?}; seq_count 0 in {} payloads; clipgain x1000 {:?}",
            e.joc_ok,
            e.joc_errors,
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
    }
    println!(
        "Integrity:         {} decode errors, {} CRC failures, {} frames ending inside the frame tail",
        p.decode_errors, p.crc_failures, p.tail_overruns
    );
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

/// `oadec info` for an AC-3 family stream (decodes everything to gather the
/// coverage and metadata statistics).
pub fn info(path: &Path, json: bool) -> Result<()> {
    let started = Instant::now();
    let (p, sync_errors, skipped) = pass(path, Options::default(), |_| Ok(()))?;
    print_pass(
        path,
        &p,
        sync_errors,
        skipped,
        started.elapsed().as_secs_f64(),
        json,
    );
    Ok(())
}

/// `oadec verify`: every frame decodes, every CRC checks, no sync errors.
pub fn verify(path: &Path, json: bool) -> Result<bool> {
    let started = Instant::now();
    let (p, sync_errors, skipped) = pass(path, Options::default(), |_| Ok(()))?;
    print_pass(
        path,
        &p,
        sync_errors,
        skipped,
        started.elapsed().as_secs_f64(),
        json,
    );
    let clean = is_clean(&p, sync_errors, skipped);
    if !json {
        println!(
            "Result:            {}",
            if clean { "CLEAN" } else { "PROBLEMS" }
        );
    }
    Ok(clean)
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

fn write_float_wav_header(
    out: &mut impl Write,
    channels: u16,
    rate: u32,
    mask: u32,
    data_len: u32,
) -> std::io::Result<()> {
    let block_align = channels * 4;
    out.write_all(b"RIFF")?;
    out.write_all(&(data_len + 12 + 8 + 40 + 8 - 8).to_le_bytes())?;
    out.write_all(b"WAVE")?;
    out.write_all(b"fmt ")?;
    out.write_all(&40u32.to_le_bytes())?;
    out.write_all(&0xFFFEu16.to_le_bytes())?;
    out.write_all(&channels.to_le_bytes())?;
    out.write_all(&rate.to_le_bytes())?;
    out.write_all(&(rate * u32::from(block_align)).to_le_bytes())?;
    out.write_all(&block_align.to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(&22u16.to_le_bytes())?;
    out.write_all(&32u16.to_le_bytes())?;
    out.write_all(&mask.to_le_bytes())?;
    // KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
    out.write_all(&[
        0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B,
        0x71,
    ])?;
    out.write_all(b"data")?;
    out.write_all(&data_len.to_le_bytes())?;
    Ok(())
}

/// `oadec decode` for AC-3 family streams: 32-bit float samples, as raw
/// little-endian PCM or as WAVE.
pub fn decode(path: &Path, output: &Path, opts: &DecodeOptions) -> Result<bool> {
    if matches!(opts.format, Format::Damf | Format::Adm) {
        bail!("object output of E-AC-3 JOC streams is not implemented yet");
    }
    let started = Instant::now();
    let file = File::create(output).with_context(|| format!("creating {}", output.display()))?;
    let mut out = BufWriter::with_capacity(4 << 20, file);
    let mut header_written = false;
    let mut data_len: u64 = 0;
    let mut order: Vec<usize> = Vec::new();
    let mut channels = 0u16;
    let mut rate = 0u32;
    let mut mask = 0u32;
    let wav = opts.format == Format::Wav;
    let (p, sync_errors, skipped) = pass(
        path,
        Options {
            dither: opts.dither,
            tpnp: opts.tpnp,
            ecpl_full: opts.ecpl_full,
        },
        |frame| {
            let (chans, pcm) = output_channels(frame, opts.core_only);
            if !header_written {
                order = output_order(&chans, opts.order);
                channels = chans.len() as u16;
                rate = frame.core().header.sample_rate;
                let ordered: Vec<ChannelLoc> = order.iter().map(|&i| chans[i]).collect();
                mask = channel_mask(&ordered);
                if wav {
                    write_float_wav_header(&mut out, channels, rate, mask, 0)?;
                }
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
            data_len += buf.len() as u64;
            if wav && data_len > u64::from(u32::MAX) - 68 {
                bail!("output exceeds the 4 GiB WAVE limit; use --format pcm");
            }
            out.write_all(&buf)?;
            Ok(())
        },
    )?;
    if wav && header_written {
        out.flush()?;
        let mut file = out.into_inner().map_err(|e| e.into_error())?;
        file.seek(SeekFrom::Start(0))?;
        write_float_wav_header(&mut file, channels, rate, mask, data_len as u32)?;
        file.flush()?;
    } else {
        out.flush()?;
    }
    print_pass(
        path,
        &p,
        sync_errors,
        skipped,
        started.elapsed().as_secs_f64(),
        false,
    );
    Ok(findings(&p, sync_errors, skipped).report_clean())
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
pub fn blocks(path: &Path, index: u64, part: usize) -> Result<()> {
    let mut group = 0u64;
    let mut in_group = 0usize;
    let mut started = false;
    let mut found = false;
    for_each_frame(path, |offset, bytes, header| {
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
                if std::env::var_os("OADEC_DETAIL").is_some() {
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
pub fn compare(path: &Path, reference: &Path, opts: &CompareOptions) -> Result<bool> {
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
        path,
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
    print_pass(path, &p, sync_errors, skipped, elapsed, false);
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
        let unclean: [(&str, Set); 7] = [
            ("decode_errors", |p| p.decode_errors = 1),
            ("crc_failures", |p| p.crc_failures = 1),
            ("tail_overruns", |p| p.tail_overruns = 1),
            ("emdf.oamd_errors", |p| p.emdf.oamd_errors = 1),
            ("emdf.joc_errors", |p| p.emdf.joc_errors = 1),
            ("emdf.joc_size_mismatch", |p| p.emdf.joc_size_mismatch = 1),
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
}
