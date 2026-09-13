//! `oadec decode --format damf|adm`: the object presentation and its metadata
//! as a Dolby Atmos Master Format set or an ADM BWF file.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_emdf::container::{self, PAYLOAD_ID_OAMD};
use oadec_emdf::oamd::{BedChannel, Oamd};
use oadec_spatial::{
    AdmOptions, AdmWriter, DamfOptions, DamfWriter, Event, LossLedger, Program, Timeline,
};
use oadec_truehd::{AccessUnit, ChannelLabel, ExtraKind, MajorSync, StreamConfig};
use serde_json::json;

use crate::decode::{Order, Session, format_duration, print_summary};
use crate::input;
use crate::integrity::Verdict;

/// Options of the object output.
#[derive(Debug, Clone)]
pub struct Options {
    /// E-AC-3 JOC: restore the level the encoder took off the downmix to keep
    /// it from clipping (`joc_clipgain`, clause 6.3.3.2).
    pub clip_gain: bool,
    /// E-AC-3 JOC: take the 90-degree phase shift of downmix configurations
    /// 3 and 4 back out with a plain rotation in every subband, the lowest
    /// one included, instead of the low-band filter the Dolby decoder uses.
    pub flat_quadrature: bool,
    /// JOC: read a sparse matrix exactly as clause 6.6.2 prints it, rather
    /// than as Dolby's decoder reads it. For measuring the difference.
    pub sparse_as_printed: bool,
    /// JOC: put the steep switch of clause 6.6.5 where the printed
    /// pseudo-code puts it, one slot after the offset names, rather than
    /// where Dolby's decoder puts it. For measuring the difference.
    pub steep_as_printed: bool,
    /// E-AC-3 only: how the core is decoded. `decode --format damf` on an
    /// E-AC-3 stream used to ignore `--no-dither`, `--no-tpnp` and
    /// `--ecpl-spec` entirely, which made three measurement flags read as
    /// applied while the object path decoded with the defaults.
    pub core: oadec_eac3::Options,
    pub keep_duplicates: bool,
    pub bed_conform: bool,
    pub all_events: bool,
    /// Write an ADM BWF file instead of a DAMF set.
    pub adm: bool,
    /// Write the creator string the Dolby validators require in ADM files.
    pub dolby_origin_tag: bool,
    /// Where to write the loss ledger as JSON, if anywhere.
    pub loss_report: Option<PathBuf>,
}

/// Prints the loss ledger of an output, one line per class, and writes it as
/// JSON when a path was given. Quiet when nothing was lost.
pub(crate) fn report_losses(target: &str, losses: &LossLedger, path: Option<&Path>) -> Result<()> {
    for line in losses.lines(target) {
        eprintln!("{line}");
    }
    if let Some(p) = path {
        let kinds: Vec<serde_json::Value> = losses
            .iter()
            .map(|(kind, count)| {
                json!({
                    "kind": kind.name(),
                    "class": kind.class().name(),
                    "count": count,
                    "examples": losses
                        .examples(kind)
                        .iter()
                        .map(|(element, sample)| json!({"element": element, "sample": sample}))
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let ramps: serde_json::Map<String, serde_json::Value> = losses
            .ramp_sources()
            .iter()
            .map(|(ramp, n)| (ramp.to_string(), json!(n)))
            .collect();
        let doc = json!({
            "target": target,
            "declared_loss": losses.declared_loss(),
            "losses": kinds,
            "ramp_sources": ramps,
        });
        let mut text = serde_json::to_string_pretty(&doc)?;
        text.push('\n');
        std::fs::write(p, text).with_context(|| format!("writing {}", p.display()))?;
    }
    Ok(())
}

fn bed_channel(label: ChannelLabel) -> Result<BedChannel> {
    Ok(match label {
        ChannelLabel::L => BedChannel::L,
        ChannelLabel::R => BedChannel::R,
        ChannelLabel::C => BedChannel::C,
        ChannelLabel::LFE => BedChannel::LFE,
        ChannelLabel::Ls => BedChannel::Ls,
        ChannelLabel::Rs => BedChannel::Rs,
        ChannelLabel::Lb => BedChannel::Lb,
        ChannelLabel::Rb => BedChannel::Rb,
        ChannelLabel::Tfl => BedChannel::Tfl,
        ChannelLabel::Tfr => BedChannel::Tfr,
        ChannelLabel::Tsl => BedChannel::Tsl,
        ChannelLabel::Tsr => BedChannel::Tsr,
        ChannelLabel::Tbl => BedChannel::Tbl,
        ChannelLabel::Tbr => BedChannel::Tbr,
        ChannelLabel::Lw => BedChannel::Lw,
        ChannelLabel::Rw => BedChannel::Rw,
        ChannelLabel::LFE2 => BedChannel::LFE2,
        other => bail!("{other:?} is not a bed channel of the object presentation"),
    })
}

/// The program the major sync declares for the object presentation.
fn program_from_major_sync(ms: &MajorSync) -> Result<Program> {
    let Some(extra) = &ms.channel_meaning.extra else {
        bail!("the stream has no 16-channel presentation (no extra channel meaning)");
    };
    let bed: Vec<BedChannel> = ChannelLabel::sixteen_channel(extra)
        .into_iter()
        .map(bed_channel)
        .collect::<Result<_>>()?;
    Ok(Program {
        beds: if bed.is_empty() {
            Vec::new()
        } else {
            vec![bed]
        },
        isf_objects: 0,
        dynamic_objects: usize::from(extra.dynamic_objects()),
    })
}

/// The two object containers behind one interface.
pub(crate) enum Sink {
    Damf(DamfWriter),
    Adm(AdmWriter),
}

/// What a closed sink reports.
pub(crate) struct SinkSummary {
    pub(crate) frames: u64,
    pub(crate) channels: usize,
    pub(crate) events: u64,
    pub(crate) paths: Vec<PathBuf>,
    pub(crate) note: String,
    pub(crate) losses: LossLedger,
}

impl Sink {
    pub(crate) fn create(
        dir: &Path,
        name: &str,
        program: &Program,
        rate: u32,
        opts: &Options,
    ) -> Result<Self> {
        if opts.adm {
            let mut options = AdmOptions {
                bed_conform: opts.bed_conform,
                ..AdmOptions::default()
            };
            if opts.dolby_origin_tag {
                options.creator = "Created using Dolby equipment".to_string();
            }
            let path = dir.join(format!("{name}.wav"));
            Ok(Self::Adm(AdmWriter::create(
                &path, program, rate, &options,
            )?))
        } else {
            let options = DamfOptions {
                bed_conform: opts.bed_conform,
                ..DamfOptions::default()
            };
            Ok(Self::Damf(DamfWriter::create(
                dir, name, program, rate, &options,
            )?))
        }
    }

    pub(crate) fn push_event(&mut self, event: &Event) -> io::Result<()> {
        match self {
            Self::Damf(w) => w.push_event(event),
            Self::Adm(w) => {
                w.push_event(event);
                Ok(())
            }
        }
    }

    pub(crate) fn write_frames<'a>(
        &mut self,
        rows: impl Iterator<Item = &'a [i32]>,
        elements: usize,
    ) -> io::Result<()> {
        match self {
            Self::Damf(w) => w.write_frames(rows, elements),
            Self::Adm(w) => w.write_frames(rows, elements),
        }
    }

    pub(crate) fn finish(self, events: u64) -> Result<SinkSummary> {
        match self {
            Self::Damf(w) => {
                let paths = w.paths().to_vec();
                let s = w.finish()?;
                Ok(SinkSummary {
                    frames: s.frames,
                    channels: s.channels,
                    events: s.events,
                    paths,
                    note: String::new(),
                    losses: s.losses,
                })
            }
            Self::Adm(w) => {
                let path = w.path().to_path_buf();
                let s = w.finish()?;
                Ok(SinkSummary {
                    frames: s.frames,
                    channels: s.channels,
                    events,
                    paths: vec![path],
                    note: format!(
                        ", {} object blocks, {} bytes{}",
                        s.blocks,
                        s.bytes,
                        if s.rf64 { ", RF64" } else { "" }
                    ),
                    losses: s.losses,
                })
            }
        }
    }
}

/// Runs the object output; `base` is the output path without extension.
pub fn run(path: &Path, base: &Path, opts: &Options) -> Result<Verdict> {
    let started = Instant::now();
    let dir = base
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = base
        .file_name()
        .and_then(|n| n.to_str())
        .context("output base name")?
        .to_string();
    std::fs::create_dir_all(dir)?;

    let mut session = Session::new(3, opts.keep_duplicates, Order::Stream);
    let mut timeline = Timeline::new(opts.all_events);
    let mut sink: Option<Sink> = None;
    let mut config: Option<StreamConfig> = None;
    let mut emitted: u64 = 0;
    let mut payload_errors: u64 = 0;
    let mut first_payload_error: Option<String> = None;
    let mut units_with_payloads: u64 = 0;
    let mut program_from_sync: Option<Program> = None;
    let mut program_mismatch = false;
    let mut index: u64 = 0;

    let pass = input::for_each_unit(path, |unit| {
        let unit_index = index;
        index += 1;
        let (au, cfg) = AccessUnit::parse(&unit.bytes, config.as_ref())?;
        let rate = cfg.sampling_frequency;
        config = Some(cfg);
        if sink.is_none() {
            let Some(ms) = &au.major_sync else {
                bail!("the stream does not start with a major sync");
            };
            let program = program_from_major_sync(ms)?;
            sink = Some(Sink::create(dir, &name, &program, rate, opts)?);
            program_from_sync = Some(program);
        }
        let w = sink.as_mut().expect("sink created above");

        let Some(frame) = session.decode(&unit)? else {
            return Ok(()); // duplicate: its audio and metadata are dropped
        };

        // Metadata of this access unit applies from its first emitted sample.
        if let Some(extra) = &au.extra
            && let ExtraKind::Evolution { frame: evo, .. } = &extra.kind
            && !evo.is_empty()
            && let Ok(c) = container::parse_evolution(evo)
        {
            let mut any = false;
            for p in c.payloads.iter().filter(|p| p.id == PAYLOAD_ID_OAMD) {
                any = true;
                match Oamd::parse(&p.data) {
                    Ok(oamd) => {
                        if !program_mismatch
                            && let Some(ps) = &program_from_sync
                            && Program::from_oamd(&oamd) != *ps
                        {
                            program_mismatch = true;
                            eprintln!(
                                "warning: access unit {unit_index}: the OAMD program ({} bed channels, {} objects) differs from the major sync ({} bed channels, {} objects)",
                                Program::from_oamd(&oamd).bed_channels().len(),
                                oamd.program.dynamic_objects,
                                ps.bed_channels().len(),
                                ps.dynamic_objects
                            );
                        }
                        let offset = u64::from(p.config.sample_offset.unwrap_or(0));
                        let mut io_error = None;
                        let result = timeline.push(&oamd, emitted, offset, |event| {
                            if io_error.is_none()
                                && let Err(e) = w.push_event(event)
                            {
                                io_error = Some(e);
                            }
                        });
                        if let Some(e) = io_error {
                            return Err(e.into());
                        }
                        if let Err(e) = result {
                            payload_errors += 1;
                            if first_payload_error.is_none() {
                                first_payload_error =
                                    Some(format!("access unit {unit_index}: {e}"));
                            }
                        }
                    }
                    Err(e) => {
                        payload_errors += 1;
                        if first_payload_error.is_none() {
                            first_payload_error = Some(format!("access unit {unit_index}: {e}"));
                        }
                    }
                }
            }
            if any {
                units_with_payloads += 1;
            }
        }

        w.write_frames(
            frame.pcm.iter().map(|row| &row[..frame.channels]),
            frame.channels,
        )?;
        emitted += frame.pcm.len() as u64;
        Ok(())
    })?;

    let Some(sink) = sink else {
        bail!("no access units found");
    };
    let summary = sink.finish(timeline.events)?;
    let elapsed = started.elapsed().as_secs_f64();
    print_summary(&session, elapsed);
    eprintln!(
        "{}: {} frames x {} channels ({}) in {}{}",
        if opts.adm { "ADM BWF" } else { "DAMF" },
        summary.frames,
        summary.channels,
        format_duration(summary.frames as f64 / f64::from(session.sampling_frequency.max(1))),
        summary
            .paths
            .last()
            .map_or_else(String::new, |p| p.display().to_string()),
        summary.note
    );
    eprintln!(
        "metadata: {} payloads in {} access units, {} events ({} restating payloads, {} out-of-order events), {} payload errors",
        timeline.payloads,
        units_with_payloads,
        summary.events,
        timeline.restatements,
        timeline.out_of_order,
        payload_errors
    );
    if let Some(e) = &first_payload_error {
        eprintln!("first payload error: {e}");
    }
    if let Some(s) = session.stats()
        && s.valid_branches + s.invalid_branches + s.duplicates > 0
    {
        eprintln!(
            "timing: {} input timing jumps, {} seamless branches, {} restarts, {} duplicates dropped",
            s.input_jumps, s.valid_branches, s.invalid_branches, s.duplicates
        );
    }
    let mut losses = timeline.losses.clone();
    losses.merge(&summary.losses);
    report_losses(
        if opts.adm { "adm" } else { "damf" },
        &losses,
        opts.loss_report.as_deref(),
    )?;
    let mut f = crate::decode::truehd_findings(&pass, session.stats());
    f.note(payload_errors, "metadata payload errors");
    f.first_problem(first_payload_error.as_deref());
    f.note_losses(&losses);
    Ok(f.report())
}
