//! `oadec oamd`: parse every Object Audio Metadata payload of a TrueHD stream,
//! tally what they contain and optionally dump them.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use oadec_emdf::container::{self, PAYLOAD_ID_OAMD};
use oadec_emdf::oamd::{Distance, Element, Gain, Oamd, Status};
use oadec_truehd::{AccessUnit, ExtraKind, StreamConfig};
use serde::Serialize;

use crate::input;

/// Options of the command.
#[derive(Debug, Clone)]
pub struct Options {
    /// Machine-readable summary.
    pub json: bool,
    /// Dump the first `n` payloads in full.
    pub dump: Option<usize>,
}

/// What a pass over the payloads found.
#[derive(Debug, Clone, Default, Serialize)]
pub struct OamdSummary {
    pub units: u64,
    pub units_with_oamd: u64,
    pub payloads: u64,
    pub parse_errors: u64,
    pub first_error: Option<String>,
    /// Payloads whose trailing padding was not all zero.
    pub padding_nonzero: u64,
    /// Elements padded by eight bits or more (a sign of a misread element).
    pub padding_long: u64,
    /// Elements that overran their declared size.
    pub size_mismatches: u64,
    pub container_sample_offsets: BTreeMap<u32, u64>,
    pub object_counts: BTreeMap<usize, u64>,
    pub element_ids: BTreeMap<u8, u64>,
    pub blocks_per_payload: BTreeMap<usize, u64>,
    pub sample_offsets: BTreeMap<u16, u64>,
    pub block_offset_factors: BTreeMap<u8, u64>,
    pub ramp_durations: BTreeMap<u16, u64>,
    pub basic_status: BTreeMap<String, u64>,
    pub render_status: BTreeMap<String, u64>,
    pub differential_positions: u64,
    pub inactive_updates: u64,
    pub distances: BTreeMap<String, u64>,
    pub screen_referenced: u64,
    pub snapped: u64,
    pub zone_constraints: BTreeMap<u8, u64>,
    pub additional_table_data: u64,
    /// Gains other than 0 dB, by decibel value written as a string; see
    /// [`oadec_emdf::oamd::GainSizeCounts`] for why the two are asked about
    /// together and why a mute is counted apart.
    pub object_gains_db: BTreeMap<String, u64>,
    /// Mutes, counted apart because an inactive object's default *is* a mute.
    pub muted_updates: u64,
    /// Updates carrying a non-zero `object_size`.
    pub sized_updates: u64,
    /// The first access units carrying each of the two, capped, so a clip can
    /// be cut from one if anything ever does.
    pub first_gain_units: Vec<u64>,
    pub first_size_units: Vec<u64>,
    pub payloads_with_trim: u64,
    pub payloads_with_extended: u64,
    pub payloads_with_unknown_elements: u64,
    pub program: Option<ProgramSummary>,
    pub program_changes: u64,
}

/// The program assignment as first seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgramSummary {
    pub dyn_object_only: bool,
    pub beds: Vec<Vec<String>>,
    pub isf_index: Option<u8>,
    pub dynamic_objects: usize,
    pub objects: usize,
}

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Default => "default",
        Status::Full => "full",
        Status::Reuse => "reuse",
        Status::Mixed => "mixed",
    }
}

/// Records where a rare value occurred, up to a cap. The counts answer "does
/// anything carry this", these answer "where do I cut a clip that does".
fn note_unit(seen: &mut Vec<u64>, unit: u64) {
    if seen.len() < 64 && seen.last() != Some(&unit) {
        seen.push(unit);
    }
}

fn gain_text(g: Gain) -> String {
    match g {
        Gain::Db(db) => format!("{db} dB"),
        Gain::MinusInfinity => "mute".to_string(),
    }
}

impl OamdSummary {
    fn take(&mut self, unit_index: u64, container_offset: Option<u32>, oamd: &Oamd) {
        self.payloads += 1;
        *self
            .container_sample_offsets
            .entry(container_offset.unwrap_or(0))
            .or_default() += 1;
        *self.object_counts.entry(oamd.object_count).or_default() += 1;
        if !oamd.padding_zero {
            self.padding_nonzero += 1;
        }
        let program = ProgramSummary {
            dyn_object_only: oamd.program.dyn_object_only,
            beds: oamd
                .program
                .beds
                .iter()
                .map(|b| b.channels.iter().map(|c| format!("{c:?}")).collect())
                .collect(),
            isf_index: oamd.program.isf_index,
            dynamic_objects: oamd.program.dynamic_objects,
            objects: oamd.program.objects(),
        };
        match &self.program {
            None => self.program = Some(program),
            Some(p) if *p != program => self.program_changes += 1,
            _ => {}
        }
        let counts = oamd.gain_and_size_counts();
        if !counts.gains_db.is_empty() {
            note_unit(&mut self.first_gain_units, unit_index);
        }
        if counts.sized > 0 {
            note_unit(&mut self.first_size_units, unit_index);
        }
        for (db, n) in &counts.gains_db {
            *self.object_gains_db.entry(db.to_string()).or_default() += n;
        }
        self.muted_updates += counts.muted;
        self.sized_updates += counts.sized;
        let mut unknown = false;
        for e in &oamd.elements {
            *self.element_ids.entry(e.id).or_default() += 1;
            if !e.padding_zero {
                self.padding_nonzero += 1;
            }
            if !e.size_ok {
                self.size_mismatches += 1;
                if self.first_error.is_none() {
                    self.first_error = Some(format!(
                        "access unit {unit_index}: element {} overran its {} bytes",
                        e.id, e.size_bytes
                    ));
                }
            }
            if e.padding_bits >= 8 {
                self.padding_long += 1;
                if self.first_error.is_none() {
                    self.first_error = Some(format!(
                        "access unit {unit_index}: element {} padded by {} bits",
                        e.id, e.padding_bits
                    ));
                }
            }
            match &e.element {
                Element::Object(o) => {
                    *self
                        .blocks_per_payload
                        .entry(o.timing.blocks.len())
                        .or_default() += 1;
                    *self
                        .sample_offsets
                        .entry(o.timing.sample_offset)
                        .or_default() += 1;
                    for b in &o.timing.blocks {
                        *self
                            .block_offset_factors
                            .entry(b.block_offset_factor)
                            .or_default() += 1;
                        *self.ramp_durations.entry(b.ramp_duration).or_default() += 1;
                    }
                    for updates in &o.objects {
                        for u in updates {
                            *self
                                .basic_status
                                .entry(status_name(u.basic_status).to_string())
                                .or_default() += 1;
                            *self
                                .render_status
                                .entry(status_name(u.render_status).to_string())
                                .or_default() += 1;
                            if u.not_active {
                                self.inactive_updates += 1;
                            }
                            if !u.additional_table_data.is_empty() {
                                self.additional_table_data += 1;
                            }

                            if u.render_status != Status::Default {
                                if u.render.differential {
                                    self.differential_positions += 1;
                                }
                                let d = match u.render.distance {
                                    Distance::Unspecified => "unspecified".to_string(),
                                    Distance::Infinity => "infinity".to_string(),
                                    Distance::Factor(f) => format!("{f}"),
                                };
                                *self.distances.entry(d).or_default() += 1;
                                if u.render.screen_ref.is_some() {
                                    self.screen_referenced += 1;
                                }
                                if u.render.snap {
                                    self.snapped += 1;
                                }
                                *self
                                    .zone_constraints
                                    .entry(u.render.zone_constraints)
                                    .or_default() += 1;
                            }
                        }
                    }
                }
                Element::Trim(_) => self.payloads_with_trim += 1,
                Element::ExtendedObject(_) => self.payloads_with_extended += 1,
                Element::Unknown(_) => unknown = true,
            }
        }
        if unknown {
            self.payloads_with_unknown_elements += 1;
        }
    }
}

fn dump_payload(unit_index: u64, container_offset: Option<u32>, oamd: &Oamd) {
    println!(
        "access unit {unit_index}: OAMD v{} {} objects, container sample offset {:?}, {} elements, padding {} bits",
        oamd.version,
        oamd.object_count,
        container_offset,
        oamd.elements.len(),
        oamd.padding_bits
    );
    let p = &oamd.program;
    println!(
        "  program: dyn_only {} beds {:?} isf {:?} dynamic {}",
        p.dyn_object_only,
        p.beds.iter().map(|b| &b.channels).collect::<Vec<_>>(),
        p.isf_index,
        p.dynamic_objects
    );
    let ext = oamd.extended_object_element();
    for e in &oamd.elements {
        match &e.element {
            Element::Object(o) => {
                println!(
                    "  object element ({} bytes, padding {}): sample_offset {} blocks {:?}",
                    e.size_bytes,
                    e.padding_bits,
                    o.timing.sample_offset,
                    o.timing
                        .blocks
                        .iter()
                        .map(|b| (b.block_offset_factor, b.ramp_duration))
                        .collect::<Vec<_>>()
                );
                for (i, updates) in o.objects.iter().enumerate() {
                    for (blk, u) in updates.iter().enumerate() {
                        let steps = ext
                            .and_then(|x| x.ext_precision.as_ref())
                            .and_then(|v| v.get(i))
                            .and_then(|v| v.get(blk))
                            .copied()
                            .unwrap_or([0; 3]);
                        let div = ext
                            .and_then(|x| x.divergence.as_ref())
                            .and_then(|v| v.get(i))
                            .and_then(|v| v.get(blk))
                            .copied();
                        let pos = u.render.position(steps);
                        println!(
                            "    obj {i:2} blk {blk}: {}{} gain {} prio {:.3} [{}/{}] pos ({:.4}, {:.4}, {:.4}){} size ({:.3}, {:.3}, {:.3}) zone {}{} {}{}{}",
                            if u.not_active { "inactive " } else { "" },
                            if u.in_bed_or_isf { "bed " } else { "" },
                            gain_text(u.basic.gain),
                            u.basic.priority,
                            status_name(u.basic_status),
                            status_name(u.render_status),
                            pos[0],
                            pos[1],
                            pos[2],
                            if u.render.differential { " diff" } else { "" },
                            u.render.size[0],
                            u.render.size[1],
                            u.render.size[2],
                            u.render.zone_constraints,
                            if u.render.enable_elevation {
                                ""
                            } else {
                                " no-elev"
                            },
                            if u.render.snap { "snap " } else { "" },
                            match u.render.distance {
                                Distance::Unspecified => String::new(),
                                Distance::Infinity => "dist inf ".to_string(),
                                Distance::Factor(f) => format!("dist {f} "),
                            },
                            div.map_or(String::new(), |d| format!("div {d}"))
                        );
                    }
                }
            }
            Element::Trim(t) => println!(
                "  trim element: warp {} global mode {} configs {} per-object {:?}",
                t.warp_mode,
                t.global_trim_mode,
                t.configs.len(),
                t.disable_per_object
            ),
            Element::ExtendedObject(x) => println!(
                "  extended object element: divergence {} ext precision {}",
                x.divergence.is_some(),
                x.ext_precision.is_some()
            ),
            Element::Unknown(raw) => {
                println!("  unknown element id {} ({} bytes)", e.id, raw.len());
            }
        }
    }
}

/// Runs the command; returns `true` when every payload parsed cleanly.
pub fn run(path: &Path, opts: &Options) -> Result<bool> {
    // an E-AC-3 stream carries its Object Audio Metadata in the EMDF
    // containers of the skip fields, not in TrueHD access units; walking it
    // for access units finds nothing and would report a clean zero
    if crate::eac3::is_eac3(path).unwrap_or(false) {
        anyhow::bail!(
            concat!(
                "{} is E-AC-3; its object metadata rides in the EMDF containers, ",
                "not in TrueHD access units. Use `oadec emdf` for the payloads ",
                "and their timing, or `oadec info` for the tallies."
            ),
            path.display()
        );
    }
    let started = Instant::now();
    let mut summary = OamdSummary::default();
    let mut config: Option<StreamConfig> = None;
    let mut dumped = 0usize;
    input::for_each_unit(path, |unit| {
        let index = summary.units;
        summary.units += 1;
        let Ok((au, cfg)) = AccessUnit::parse(&unit.bytes, config.as_ref()) else {
            return Ok(());
        };
        config = Some(cfg);
        let Some(extra) = &au.extra else {
            return Ok(());
        };
        let ExtraKind::Evolution { frame, .. } = &extra.kind else {
            return Ok(());
        };
        if frame.is_empty() {
            return Ok(());
        }
        let Ok(c) = container::parse_evolution(frame) else {
            return Ok(());
        };
        let mut any = false;
        for p in c.payloads.iter().filter(|p| p.id == PAYLOAD_ID_OAMD) {
            any = true;
            match Oamd::parse(&p.data) {
                Ok(oamd) => {
                    summary.take(index, p.config.sample_offset, &oamd);
                    if opts.dump.is_some_and(|n| dumped < n) {
                        dump_payload(index, p.config.sample_offset, &oamd);
                        dumped += 1;
                    }
                }
                Err(e) => {
                    if opts.dump.is_some() {
                        println!(
                            "access unit {index}: payload of {} bytes failed: {e}",
                            p.data.len()
                        );
                        println!(
                            "  {}",
                            p.data
                                .iter()
                                .map(|b| format!("{b:02x}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                    }
                    summary.payloads += 1;
                    summary.parse_errors += 1;
                    if summary.first_error.is_none() {
                        summary.first_error = Some(format!("access unit {index}: {e}"));
                    }
                }
            }
        }
        if any {
            summary.units_with_oamd += 1;
        }
        Ok(())
    })?;
    let elapsed = started.elapsed().as_secs_f64();
    let clean =
        summary.parse_errors == 0 && summary.padding_long == 0 && summary.padding_nonzero == 0;
    if opts.json {
        let mut value = serde_json::to_value(&summary)?;
        value["clean"] = serde_json::json!(clean);
        value["seconds"] = serde_json::json!(elapsed);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!(
            "OAMD payloads:     {} in {} of {} access units; {} parse errors, {} non-zero paddings, {} long paddings, {} size mismatches",
            summary.payloads,
            summary.units_with_oamd,
            summary.units,
            summary.parse_errors,
            summary.padding_nonzero,
            summary.padding_long,
            summary.size_mismatches
        );
        if let Some(p) = &summary.program {
            println!(
                "Program:           dyn_only {} beds {:?} isf {:?} dynamic objects {} ({} objects), {} changes",
                p.dyn_object_only,
                p.beds,
                p.isf_index,
                p.dynamic_objects,
                p.objects,
                summary.program_changes
            );
        }
        println!("Object counts:     {:?}", summary.object_counts);
        println!("Element ids:       {:?}", summary.element_ids);
        println!("Blocks/payload:    {:?}", summary.blocks_per_payload);
        println!("Container offsets: {:?}", summary.container_sample_offsets);
        println!("Sample offsets:    {:?}", summary.sample_offsets);
        println!("Block offsets:     {:?}", summary.block_offset_factors);
        println!("Ramp durations:    {:?}", summary.ramp_durations);
        println!(
            "Statuses:          basic {:?} render {:?}",
            summary.basic_status, summary.render_status
        );
        println!(
            "Updates:           {} inactive, {} differential positions, {} screen-referenced, {} snapped, {} with table data",
            summary.inactive_updates,
            summary.differential_positions,
            summary.screen_referenced,
            summary.snapped,
            summary.additional_table_data
        );
        println!(
            "Gain and size:     {} non-unity gains {:?}, {} mutes, {} non-zero sizes{}",
            summary.object_gains_db.values().sum::<u64>(),
            summary.object_gains_db,
            summary.muted_updates,
            summary.sized_updates,
            if summary.first_gain_units.is_empty() && summary.first_size_units.is_empty() {
                String::new()
            } else {
                format!(
                    " (first access units: gain {:?}, size {:?})",
                    summary.first_gain_units, summary.first_size_units
                )
            }
        );
        println!("Distances:         {:?}", summary.distances);
        println!("Zone constraints:  {:?}", summary.zone_constraints);
        println!(
            "Elements:          {} payloads with trim, {} with extended object data, {} with unknown ids",
            summary.payloads_with_trim,
            summary.payloads_with_extended,
            summary.payloads_with_unknown_elements
        );
        if let Some(e) = &summary.first_error {
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
