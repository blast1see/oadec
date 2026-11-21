//! Writes what the JOC object path holds, time slot by time slot, for the
//! measuring tools (`tools/joc_slot_error.py`, `tools/joc_qmf_reference.py`).
//!
//! ```text
//! cargo run --release -p oadec-engine --example joc_tap -- <stream.ec3> <dir> [--reset-banks] [--no-dither]
//! ```
//!
//! `<dir>` must not hold files. It gets little-endian f64 planes and an
//! `index.json` that says what each holds and how it is shaped:
//!
//! - `downmix.f64`: `[slot][ch][sb][re, im]`, after the analysis bank and the
//!   quadrature: what the matrix of the slot is applied to;
//! - `matrix.f64`: `[slot][obj][ch][sb]`, the matrix each object meets;
//! - `objects.f64`: `[slot][obj][sb][re, im]`, before the synthesis bank;
//! - `pcm_objects.f64`: `[sample][obj]`, the objects as `joc_pcm` gives them,
//!   aligned with the core;
//! - `pcm_core.f64`: `[sample][ch]`, the core decode of every programme
//!   channel without the clip gain, the decoder delay dropped;
//! - `slots` in `index.json`: frame, slot, sequence counter, reset, flush and
//!   the two clip gains of every slot.
//!
//! Slots run from the first slot of the first frame; an object sample `i` is
//! synthesis output sample `i + object_delay`.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use oadec_engine::damf::Options;
use oadec_engine::eac3_objects::{JocTap, TapOptions, TapSlot, TapStart, joc_pcm_tapped};
use oadec_engine::source::Source;
use serde_json::{Value, json};

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
        let start = self.start.as_ref().expect("started");
        for ch in slot.downmix.iter().take(start.channels) {
            for c in ch {
                put(&mut self.downmix, c.re)?;
                put(&mut self.downmix, c.im)?;
            }
        }
        for obj in 0..start.objects {
            let m = slot.matrix(obj).expect("a matrix for every object");
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

fn run(stream: &Path, dir: &Path, measure: TapOptions, dither: bool) -> anyhow::Result<()> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        anyhow::bail!(
            "{} holds files; a dump goes into an empty directory",
            dir.display()
        );
    }
    fs::create_dir_all(dir)?;
    let mut opts = Options::default();
    opts.core.dither = dither;
    let mut tap = DirTap::create(dir)?;
    let pcm = joc_pcm_tapped(&Source::file(stream), &opts, measure, &mut tap)?;
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
    let start = tap.start.as_ref().expect("the tap was started");
    let index = json!({
        "what": "the JOC object path of oadec, slot by slot (examples/joc_tap.rs)",
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
        "dither": dither,
        "slot_fields": ["frame", "ts", "seq_count", "reset", "flush", "clip_gain_before", "clip_gain"],
        "slots": tap.slots,
    });
    fs::write(dir.join("index.json"), serde_json::to_vec(&index)?)?;
    Ok(())
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut flag = |name: &str| {
        let had = args.iter().any(|a| a == name);
        args.retain(|a| a != name);
        had
    };
    let measure = TapOptions {
        reset_banks: flag("--reset-banks"),
    };
    let dither = !flag("--no-dither");
    let [stream, dir] = args.as_slice() else {
        eprintln!("usage: joc_tap <stream.ec3> <dir> [--reset-banks] [--no-dither]");
        return ExitCode::from(2);
    };
    match run(&PathBuf::from(stream), &PathBuf::from(dir), measure, dither) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("joc_tap: {e:#}");
            ExitCode::FAILURE
        }
    }
}
