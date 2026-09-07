//! `oadec decode --format damf`: the object presentation and its metadata as a
//! Dolby Atmos Master Format set.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use oadec_emdf::container::{self, PAYLOAD_ID_OAMD};
use oadec_emdf::oamd::{BedChannel, Oamd};
use oadec_spatial::{DamfOptions, DamfWriter, Program, Timeline};
use oadec_truehd::{AccessUnit, ChannelLabel, ExtraKind, MajorSync, StreamConfig};

use crate::decode::{Order, Session, format_duration, print_summary};
use crate::input;

/// Options of the DAMF output.
#[derive(Debug, Clone)]
pub struct Options {
    pub keep_duplicates: bool,
    pub bed_conform: bool,
    pub all_events: bool,
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

/// Runs the DAMF output; `base` is the output path without extension.
pub fn run(path: &Path, base: &Path, opts: &Options) -> Result<()> {
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
    let mut writer: Option<DamfWriter> = None;
    let mut config: Option<StreamConfig> = None;
    let mut emitted: u64 = 0;
    let mut payload_errors: u64 = 0;
    let mut first_payload_error: Option<String> = None;
    let mut units_with_payloads: u64 = 0;
    let mut program_from_sync: Option<Program> = None;
    let mut program_mismatch = false;
    let mut damf_opts = DamfOptions {
        bed_conform: opts.bed_conform,
        ..DamfOptions::default()
    };
    damf_opts.creation_tool_version = env!("CARGO_PKG_VERSION").to_string();
    let mut index: u64 = 0;

    input::for_each_unit(path, |unit| {
        let unit_index = index;
        index += 1;
        let (au, cfg) = AccessUnit::parse(&unit.bytes, config.as_ref())?;
        let rate = cfg.sampling_frequency;
        config = Some(cfg);
        if writer.is_none() {
            let Some(ms) = &au.major_sync else {
                bail!("the stream does not start with a major sync");
            };
            let program = program_from_major_sync(ms)?;
            writer = Some(DamfWriter::create(dir, &name, &program, rate, &damf_opts)?);
            program_from_sync = Some(program);
        }
        let w = writer.as_mut().expect("writer created above");

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

    let Some(w) = writer else {
        bail!("no access units found");
    };
    let paths = w.paths().clone();
    let summary = w.finish()?;
    let elapsed = started.elapsed().as_secs_f64();
    print_summary(&session, elapsed);
    eprintln!(
        "DAMF: {} frames x {} channels ({}) in {}",
        summary.frames,
        summary.channels,
        format_duration(summary.frames as f64 / f64::from(session.sampling_frequency.max(1))),
        paths[2].display()
    );
    eprintln!(
        "metadata: {} payloads in {} access units, {} events written ({} restating payloads, {} out-of-order events), {} payload errors",
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
    if session.stats().is_some_and(|s| s.lossless_mismatches != 0) {
        bail!("lossless check failures were reported");
    }
    Ok(())
}
