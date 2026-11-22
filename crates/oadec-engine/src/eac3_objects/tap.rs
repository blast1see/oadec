//! A read-only tap on the JOC object path, for measuring where the objects
//! part from those of the Dolby decoder.
//!
//! The Dolby decoder writes only PCM and refuses a rewritten JOC payload, so
//! nothing between its core decode and its objects can be read off it. What
//! can be read is every stage of this decoder: the downmix as the analysis
//! bank and the quadrature hand it to the matrix, the matrix each time slot
//! meets, and the objects the matrix makes before the synthesis bank. A tool
//! that fits Dolby's objects onto the first (`tools/joc_slot_error.py`) then
//! says, slot by slot and band by band, whether what is left is the matrix or
//! the core.
//!
//! The tap only reads. It is handed borrowed slices of the pipeline's own
//! buffers, one time slot at a time, on the thread that decodes; it changes
//! nothing the pipeline computes, and without a tap the object path is the
//! one it has always been. It is not part of [`super::JocOverrides`], which is
//! compared and defaulted as a value.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Result, bail};
use oadec_emdf::joc::MAX_CHANNELS;
use oadec_joc::{BANDS, Carry, Complex, JocDecoder};
use serde_json::{Value, json};

/// Measurement switches of a tapped decode, kept out of
/// [`super::JocOverrides`] until the command line carries them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TapOptions {
    /// At a splice of the JOC sequence counter, clear the analysis,
    /// quadrature and synthesis banks as well as the matrix history
    /// (`--joc-reset-banks` once the command line has it). Off by default:
    /// the decode is then the one without a tap.
    pub reset_banks: bool,
}

/// What a tap is told once, before the first time slot.
#[derive(Debug, Clone, PartialEq)]
pub struct TapStart {
    /// Sample rate.
    pub rate: u32,
    /// `joc_dmx_config_idx` of the first payload.
    pub dmx_config: u8,
    /// JOC downmix channels, in the order of table 47.
    pub channels: usize,
    /// JOC objects the payload declares.
    pub objects: usize,
    /// What each downmix channel carries into the quadrature.
    pub carry: Vec<Carry>,
    /// Time slots the matrices are held back by (`--joc-lag` or its default).
    pub lag: usize,
    /// Samples an object runs late by against the core it is made of: the
    /// decoder delay, the two banks and the hold of the low-band operator.
    pub object_delay: usize,
}

/// One time slot of the object path, as the pipeline has it.
///
/// Every slice is the pipeline's own buffer, borrowed for the call: a tap that
/// keeps anything copies it.
#[derive(Debug)]
pub struct TapSlot<'a> {
    /// Frames given to the pipeline before this one (0 for the first). The
    /// slots of the flush after the last frame carry the count of frames.
    pub frame: u64,
    /// Time slot within the frame.
    pub ts: usize,
    /// `joc_seq_count` of the frame's payload, `None` for a frame without one
    /// and for the flush.
    pub seq_count: Option<u16>,
    /// This frame's counter said the stream was spliced, and the matrix
    /// history was forgotten before its first slot (clause 6.3.3.3).
    pub reset: bool,
    /// The slot belongs to the zeros run through the banks after the last
    /// frame.
    pub flush: bool,
    /// The downmix after the analysis bank and the quadrature, `[ch][sb]`:
    /// what the matrix of this slot is applied to.
    pub downmix: &'a [[Complex; BANDS]],
    /// The objects the matrix makes of it, `[obj][sb]`, before synthesis.
    pub objects: &'a [[Complex; BANDS]],
    /// The decoder, for the matrix each object meets in this slot
    /// ([`TapSlot::matrix`]).
    pub(super) decoder: &'a JocDecoder,
    /// The clip gain the frame starts from: that of the frame before.
    pub clip_gain_before: f64,
    /// The clip gain of the frame, reached over its first block.
    pub clip_gain: f64,
}

impl TapSlot<'_> {
    /// The interpolated matrix object `obj` meets in this slot, `[ch][sb]`,
    /// after the lag has moved it against the subband samples: the one
    /// `objects[obj]` was made with. `None` outside the objects.
    #[must_use]
    pub fn matrix(&self, obj: usize) -> Option<&[[f64; BANDS]; MAX_CHANNELS]> {
        self.decoder.applied(obj, self.ts)
    }
}

/// What reads the object path a time slot at a time.
pub trait JocTap {
    /// Called once, before the first slot.
    fn start(&mut self, start: &TapStart) {
        let _ = start;
    }

    /// Called for every time slot the pipeline runs, the flush included, in
    /// order, after the matrix and before the synthesis bank.
    fn slot(&mut self, slot: &TapSlot<'_>);
}

/// A tap that writes every slot into the planes of a directory.
struct DirTap {
    downmix: BufWriter<File>,
    matrix: BufWriter<File>,
    objects: BufWriter<File>,
    start: Option<TapStart>,
    slots: Vec<Value>,
    error: Option<std::io::Error>,
}

fn plane(dir: &Path, name: &str) -> std::io::Result<BufWriter<File>> {
    Ok(BufWriter::with_capacity(
        1 << 20,
        File::create(dir.join(name))?,
    ))
}

fn put(w: &mut BufWriter<File>, v: f64) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

impl DirTap {
    fn create(dir: &Path) -> std::io::Result<Self> {
        Ok(Self {
            downmix: plane(dir, "downmix.f64")?,
            matrix: plane(dir, "matrix.f64")?,
            objects: plane(dir, "objects.f64")?,
            start: None,
            slots: Vec::new(),
            error: None,
        })
    }

    fn write_slot(&mut self, slot: &TapSlot<'_>) -> std::io::Result<()> {
        let Some(start) = self.start.as_ref() else {
            return Ok(());
        };
        for ch in slot.downmix.iter().take(start.channels) {
            for c in ch {
                put(&mut self.downmix, c.re)?;
                put(&mut self.downmix, c.im)?;
            }
        }
        for obj in 0..start.objects {
            let Some(m) = slot.matrix(obj) else {
                return Err(std::io::Error::other("an object without a matrix"));
            };
            for row in m.iter().take(start.channels) {
                for &v in row {
                    put(&mut self.matrix, v)?;
                }
            }
        }
        for obj in slot.objects.iter().take(start.objects) {
            for c in obj {
                put(&mut self.objects, c.re)?;
                put(&mut self.objects, c.im)?;
            }
        }
        self.slots.push(json!([
            slot.frame,
            slot.ts,
            slot.seq_count,
            slot.reset,
            slot.flush,
            slot.clip_gain_before,
            slot.clip_gain
        ]));
        Ok(())
    }
}

impl JocTap for DirTap {
    fn start(&mut self, start: &TapStart) {
        self.start = Some(start.clone());
    }

    fn slot(&mut self, slot: &TapSlot<'_>) {
        if self.error.is_none()
            && let Err(e) = self.write_slot(slot)
        {
            self.error = Some(e);
        }
    }
}

fn interleaved(path: &Path, columns: &[&[f64]]) -> std::io::Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    let n = columns.iter().map(|c| c.len()).min().unwrap_or(0);
    for i in 0..n {
        for c in columns {
            w.write_all(&c[i].to_le_bytes())?;
        }
    }
    w.flush()
}

/// Decodes the objects of an E-AC-3 JOC stream with a tap that writes what
/// the object path holds, time slot by time slot, into `dir`, for the
/// measuring tools (`tools/joc_slot_error.py`, `tools/joc_qmf_reference.py`).
///
/// `dir` must not hold files. It gets little-endian f64 planes and an
/// `index.json` that says what each holds and how it is shaped:
///
/// - `downmix.f64`: `[slot][ch][sb][re, im]`, after the analysis bank and the
///   quadrature: what the matrix of the slot is applied to;
/// - `matrix.f64`: `[slot][obj][ch][sb]`, the matrix each object meets;
/// - `objects.f64`: `[slot][obj][sb][re, im]`, before the synthesis bank;
/// - `pcm_objects.f64`: `[sample][obj]`, the objects as [`super::joc_pcm`]
///   gives them, aligned with the core;
/// - `pcm_core.f64`: `[sample][ch]`, the core decode of every programme
///   channel without the clip gain, the decoder delay dropped;
/// - `slots` in `index.json`: frame, slot, sequence counter, reset, flush and
///   the two clip gains of every slot.
///
/// Slots run from the first slot of the first frame; an object sample `i` is
/// synthesis output sample `i + object_delay`. The decode is one of its own:
/// it writes no object output and changes none.
///
/// # Errors
/// A directory that holds files, what [`super::joc_pcm`] refuses, and what
/// cannot be written into the directory.
pub fn tap_to_dir(
    source: &crate::source::Source,
    dir: &Path,
    opts: &crate::damf::Options,
    measure: TapOptions,
) -> Result<()> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        bail!(
            "{} holds files; a dump goes into an empty directory",
            dir.display()
        );
    }
    fs::create_dir_all(dir)?;
    let mut tap = DirTap::create(dir)?;
    let pcm = super::joc_pcm_tapped(source, opts, measure, &mut tap)?;
    if let Some(e) = tap.error.take() {
        return Err(e.into());
    }
    for w in [&mut tap.downmix, &mut tap.matrix, &mut tap.objects] {
        w.flush()?;
    }
    let objects: Vec<&[f64]> = pcm.objects.iter().map(Vec::as_slice).collect();
    interleaved(&dir.join("pcm_objects.f64"), &objects)?;
    let core: Vec<&[f64]> = pcm.core.iter().map(Vec::as_slice).collect();
    interleaved(&dir.join("pcm_core.f64"), &core)?;
    let Some(start) = tap.start.as_ref() else {
        bail!("the stream gave the tap no JOC payload to start from");
    };
    let index = json!({
        "what": "the JOC object path of oadec, slot by slot (eac3_objects::tap_to_dir)",
        "planes": {
            "downmix.f64": ["slot", "ch", "sb", "re_im"],
            "matrix.f64": ["slot", "obj", "ch", "sb"],
            "objects.f64": ["slot", "obj", "sb", "re_im"],
            "pcm_objects.f64": ["sample", "obj"],
            "pcm_core.f64": ["sample", "ch"],
        },
        "rate": start.rate,
        "dmx_config": start.dmx_config,
        "channels": start.channels,
        "objects": start.objects,
        "carry": start.carry.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>(),
        "lag": start.lag,
        "object_delay": pcm.object_delay,
        "inputs": pcm.inputs,
        "core_channels": pcm.channels.iter().map(|c| c.name()).collect::<Vec<_>>(),
        "samples": pcm.objects.first().map_or(0, Vec::len),
        "frames": pcm.frames,
        "splices": pcm.splices,
        "reset_banks": measure.reset_banks,
        "dither": opts.core.dither,
        "slot_fields": ["frame", "ts", "seq_count", "reset", "flush", "clip_gain_before", "clip_gain"],
        "slots": tap.slots,
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index)?)?;
    Ok(())
}
