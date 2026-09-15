//! `oadec decode --format damf|adm`: the object presentation and its metadata
//! as a Dolby Atmos Master Format set or an ADM BWF file.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::ValueEnum;
use oadec_emdf::container::{self, PAYLOAD_ID_OAMD};
use oadec_emdf::oamd::{BedChannel, ISF_OBJECTS, Oamd};
use oadec_spatial::{
    AdmError, AdmOptions, AdmWriter, DamfError, DamfOptions, DamfWriter, Event, Interpolation,
    IsfPolicy, LossLedger, Program, Timeline,
};
use oadec_truehd::channel::ExtraChannelMeaning;
use oadec_truehd::{AccessUnit, ChannelLabel, ExtraKind, MajorSync, StreamConfig};
use serde_json::json;

/// `--isf`: what to do with intermediate-spatial-format elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum IsfArg {
    /// Refuse the decode: DAMF and the ADM profile cannot represent them.
    Error,
    /// Write the beds and dynamic objects without them (a declared loss, exit 4).
    Drop,
}

impl From<IsfArg> for IsfPolicy {
    fn from(arg: IsfArg) -> Self {
        match arg {
            IsfArg::Error => Self::Error,
            IsfArg::Drop => Self::Drop,
        }
    }
}

/// `--adm-interpolation`: how ADM interpolation lengths are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum InterpolationArg {
    /// The Dolby Atmos master ADM profile: 0 then 250 samples (the default).
    Profile,
    /// The source ramps (BS.2076, outside the profile; refuses the Dolby origin tag).
    Real,
}

impl From<InterpolationArg> for Interpolation {
    fn from(arg: InterpolationArg) -> Self {
        match arg {
            InterpolationArg::Profile => Self::Profile,
            InterpolationArg::Real => Self::Real,
        }
    }
}

use crate::decode::{Format, Order, Session, format_duration, print_summary};
use crate::input;
use crate::integrity::Verdict;

/// The object formats always decode the object presentation (3); a
/// `--presentation` that says otherwise used to be accepted and ignored.
pub(crate) fn check_presentation(format: Format, presentation: Option<usize>) -> Result<()> {
    let name = match format {
        Format::Damf => "damf",
        Format::Adm => "adm",
        Format::Pcm | Format::Wav => return Ok(()),
    };
    match presentation {
        Some(p) if p != 3 => bail!(
            "--presentation {p} does not apply to --format {name}: the object formats always decode the object presentation (3)"
        ),
        _ => Ok(()),
    }
}

/// A file with the source ramps is outside the Dolby profile and cannot claim
/// Dolby authorship.
pub(crate) fn check_interpolation(
    interpolation: Interpolation,
    dolby_origin_tag: bool,
) -> Result<()> {
    if interpolation == Interpolation::Real && dolby_origin_tag {
        bail!(
            "--adm-interpolation real writes a file outside the Dolby Atmos master ADM profile and cannot carry the Dolby origin tag; drop --dolby-origin-tag"
        );
    }
    Ok(())
}

/// Says on stderr when the ADM file is outside the profile.
pub(crate) fn note_non_profile(opts: &Options) {
    if opts.adm && opts.interpolation == Interpolation::Real {
        eprintln!(
            "ADM BWF written outside the Dolby Atmos master ADM profile: interpolation lengths are the source ramps"
        );
    }
}

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
    /// E-AC-3 JOC: the hidden measurement overrides (`--joc-lag`,
    /// `--joc-low-band`, `--joc-phase`), none by default.
    pub joc: crate::eac3_objects::JocOverrides,
    pub keep_duplicates: bool,
    pub bed_conform: bool,
    pub all_events: bool,
    /// Write an ADM BWF file instead of a DAMF set.
    pub adm: bool,
    /// Write the creator string the Dolby validators require in ADM files.
    pub dolby_origin_tag: bool,
    /// Where to write the loss ledger as JSON, if anywhere.
    pub loss_report: Option<PathBuf>,
    /// Intermediate-spatial-format elements: refuse or drop.
    pub isf: IsfPolicy,
    /// ADM: write a programme that is not at 48 kHz (outside the profile).
    pub allow_non_profile_rate: bool,
    /// DAMF: the frame rate of the `.atmos` header.
    pub fps: String,
    /// ADM: how interpolation lengths are written.
    pub interpolation: Interpolation,
}

/// The message of a refused ISF programme, with the way out.
fn isf_hint(count: usize, isf_type: &str) -> String {
    format!(
        "the programme carries {count} intermediate-spatial-format objects ({isf_type}), which DAMF and the Dolby Atmos master ADM profile cannot represent; pass --isf drop to write the beds and dynamic objects without them"
    )
}

/// Prints the loss ledger of an output, one line per class, and writes it as
/// JSON when a path was given. Quiet when nothing was lost.
/// `overrides` are the measurement overrides the run was given (see
/// `eac3_objects::JocOverrides`); the report always carries the list, empty
/// when there were none.
pub(crate) fn report_losses(
    target: &str,
    losses: &LossLedger,
    path: Option<&Path>,
    overrides: &[serde_json::Value],
) -> Result<()> {
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
            "overrides": overrides,
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

/// The program the extra channel meaning of a major sync declares: bed
/// channels, then the ISF objects of the declared type (table 11b), then the
/// dynamic objects.
fn program_from_extra(extra: &ExtraChannelMeaning) -> Result<Program> {
    let bed: Vec<BedChannel> = ChannelLabel::sixteen_channel(extra)
        .into_iter()
        .map(bed_channel)
        .collect::<Result<_>>()?;
    let (isf_index, isf_objects) = if extra.has_isf() {
        let index = extra.isf_index & 7;
        match ISF_OBJECTS[usize::from(index)] {
            Some(n) => (Some(index), n),
            None => bail!(
                "the 16-channel presentation declares the reserved intermediate spatial format index {index}"
            ),
        }
    } else {
        (None, 0)
    };
    Ok(Program {
        beds: if bed.is_empty() {
            Vec::new()
        } else {
            vec![bed]
        },
        isf_index,
        isf_objects,
        dynamic_objects: usize::from(extra.dynamic_objects()),
    })
}

/// The program the major sync declares for the object presentation.
fn program_from_major_sync(ms: &MajorSync) -> Result<Program> {
    let Some(extra) = &ms.channel_meaning.extra else {
        bail!("the stream has no 16-channel presentation (no extra channel meaning)");
    };
    program_from_extra(extra)
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
                isf: opts.isf,
                allow_non_profile_rate: opts.allow_non_profile_rate,
                interpolation: opts.interpolation,
                ..AdmOptions::default()
            };
            if opts.dolby_origin_tag {
                options.creator = "Created using Dolby equipment".to_string();
            }
            let path = dir.join(format!("{name}.wav"));
            match AdmWriter::create(&path, program, rate, &options) {
                Ok(w) => Ok(Self::Adm(w)),
                Err(AdmError::IsfNotRepresentable { count, isf_type }) => {
                    bail!("{}", isf_hint(count, &isf_type))
                }
                Err(AdmError::NonProfileSampleRate(rate)) => bail!(
                    "the Dolby Atmos master ADM profile requires 48 000 Hz and this programme is {rate} Hz; pass --adm-allow-non-profile-rate to write it anyway, declared outside the profile (exit 4)"
                ),
                Err(e) => Err(e.into()),
            }
        } else {
            let options = DamfOptions {
                bed_conform: opts.bed_conform,
                isf: opts.isf,
                fps: opts.fps.clone(),
                ..DamfOptions::default()
            };
            match DamfWriter::create(dir, name, program, rate, &options) {
                Ok(w) => Ok(Self::Damf(w)),
                Err(DamfError::IsfNotRepresentable { count, isf_type }) => {
                    bail!("{}", isf_hint(count, &isf_type))
                }
                Err(e) => Err(e.into()),
            }
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
    note_non_profile(opts);
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
        &[],
    )?;
    let mut f = crate::decode::truehd_findings(&pass, session.stats());
    f.note(payload_errors, "metadata payload errors");
    f.first_problem(first_payload_error.as_deref());
    f.note_losses(&losses);
    Ok(f.report())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file with the source ramps is outside the Dolby profile and must not
    /// claim Dolby authorship.
    #[test]
    fn real_interpolation_refuses_the_dolby_origin_tag() {
        assert!(check_interpolation(Interpolation::Profile, true).is_ok());
        assert!(check_interpolation(Interpolation::Real, false).is_ok());
        let err = check_interpolation(Interpolation::Real, true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--adm-interpolation real"), "{err}");
        assert!(err.contains("--dolby-origin-tag"), "{err}");
    }

    /// `--presentation` used to be accepted and ignored with the object
    /// formats, which always decode the object presentation (3).
    #[test]
    fn the_object_formats_refuse_a_presentation_other_than_three() {
        assert!(check_presentation(Format::Adm, None).is_ok());
        assert!(check_presentation(Format::Damf, Some(3)).is_ok());
        assert!(check_presentation(Format::Wav, Some(2)).is_ok());
        let err = check_presentation(Format::Adm, Some(2))
            .unwrap_err()
            .to_string();
        assert!(err.contains("--presentation 2"), "{err}");
        assert!(err.contains("--format adm"), "{err}");
        assert!(check_presentation(Format::Damf, Some(0)).is_err());
    }

    /// The 16-channel presentation declares its ISF type in the major sync;
    /// the count used to be hardcoded to zero, which would have shifted every
    /// element after the bed on a stream that carries ISF objects.
    #[test]
    fn the_isf_count_comes_from_the_major_sync() {
        let extra = ExtraChannelMeaning {
            content_description: 0b110, // ISF and dynamic objects, no bed
            isf_index: 0,
            dynamic_object_count: 1,
            ..ExtraChannelMeaning::default()
        };
        let p = program_from_extra(&extra).unwrap();
        assert_eq!(p.isf_objects, 4, "SR3.1.0.0 has four objects");
        assert_eq!(p.isf_index, Some(0));
        assert_eq!(p.dynamic_objects, 2);
        assert!(p.beds.is_empty());
        let reserved = ExtraChannelMeaning {
            isf_index: 6,
            ..extra
        };
        assert!(program_from_extra(&reserved).is_err());
        let none = ExtraChannelMeaning {
            content_description: 0b100,
            ..reserved
        };
        let p = program_from_extra(&none).unwrap();
        assert_eq!((p.isf_objects, p.isf_index), (0, None));
    }
}
