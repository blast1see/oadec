//! `oadec info`: what the stream declares about itself.

use std::path::Path;

use anyhow::Result;
use oadec_truehd::{ChannelLabel, MajorSync, PresentationKind};

use crate::scan::{self, Scan};

fn labels(v: &[ChannelLabel]) -> String {
    v.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_text(kind: PresentationKind) -> String {
    match kind {
        PresentationKind::Independent => "independent".to_string(),
        PresentationKind::DownmixOf(i) => format!("downmix of presentation {i}"),
        PresentationKind::CopyOf(i) => format!("copy of presentation {i}"),
        PresentationKind::Invalid => "not carried".to_string(),
    }
}

fn format_duration(seconds: f64) -> String {
    let total_ms = (seconds * 1000.0).round() as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms / 60_000) % 60;
    let s = (total_ms / 1000) % 60;
    let ms = total_ms % 1000;
    format!("{h}:{m:02}:{s:02}.{ms:03}")
}

fn print_presentations(ms: &MajorSync) {
    let map = ms.presentation_map();
    let fi = &ms.format_info;
    let restricted = ms.flags & oadec_truehd::sync::FLAG_RESTRICTED_8CH != 0;
    let rows: [(usize, String); 4] = [
        (0, labels(&ChannelLabel::two_channel(false))),
        (
            1,
            labels(&ChannelLabel::six_channel(fi.sixch_channel_assignment)),
        ),
        (
            2,
            labels(&ChannelLabel::eight_channel(
                fi.eightch_channel_assignment,
                restricted,
            )),
        ),
        (
            3,
            match &ms.channel_meaning.extra {
                Some(extra) if ms.has_16ch_presentation() => {
                    let bed = ChannelLabel::sixteen_channel(extra);
                    let mut parts = Vec::new();
                    if !bed.is_empty() {
                        parts.push(format!("bed {} ({})", labels(&bed), bed.len()));
                    }
                    if extra.has_isf() {
                        parts.push(format!("ISF index {}", extra.isf_index));
                    }
                    let dyn_objects = extra.dynamic_objects();
                    if dyn_objects > 0 {
                        parts.push(format!("{dyn_objects} dynamic objects"));
                    }
                    format!("{} channels: {}", extra.channels(), parts.join(" + "))
                }
                _ => "-".to_string(),
            },
        ),
    ];
    println!("Presentations:");
    for (index, text) in rows {
        let kind = map.kind(index);
        if kind == PresentationKind::Invalid && index > 0 {
            continue;
        }
        println!(
            "  {index}: substreams {:04b}  {text}  [{}]",
            map.mask(index),
            kind_text(kind)
        );
    }
}

fn print_text(scan: &Scan) {
    println!(
        "File:             {} ({} bytes)",
        scan.file, scan.file_bytes
    );
    let Some(ms) = &scan.first_major_sync else {
        println!("No major sync found: not a Dolby TrueHD stream.");
        return;
    };
    let fs = ms.sampling_frequency().unwrap_or(0);
    println!(
        "Format:           Dolby TrueHD (FBA), {fs} Hz, {} samples per access unit{}",
        ms.samples_per_au().unwrap_or(0),
        if ms.has_16ch_presentation() {
            ", 16-channel object presentation"
        } else {
            ""
        }
    );
    if let Some(secs) = scan.duration_seconds() {
        println!(
            "Duration:         {} ({} access units)",
            format_duration(secs),
            scan.units
        );
    }
    println!(
        "Data rate:        peak {} kbps ({}), average {:.0} kbps, access units {}..{} bytes",
        ms.peak_bit_rate().unwrap_or(0) / 1000,
        if ms.variable_rate {
            "variable"
        } else {
            "fixed"
        },
        scan.average_bit_rate().unwrap_or(0.0) / 1000.0,
        scan.min_unit_bytes,
        scan.max_unit_bytes
    );
    println!(
        "Substreams:       {} (substream_info 0x{:02X}, extended_substream_info {}), flags 0x{:04X}",
        ms.substreams,
        ms.substream_info,
        ms.extended_substream_info & 3,
        ms.flags
    );
    println!(
        "Major syncs:      {} (at most every {} access units)",
        scan.major_syncs, scan.max_major_sync_interval
    );
    print_presentations(ms);
    let cm = &ms.channel_meaning;
    print!(
        "Dialogue norm:    2ch -{} dB, 6ch -{} dB, 8ch -{} dB",
        cm.twoch_dialogue_norm, cm.sixch_dialogue_norm, cm.eightch_dialogue_norm
    );
    if let Some(extra) = &cm.extra {
        print!(", 16ch -{} dB", extra.sixteench_dialogue_norm);
    }
    println!();
    println!(
        "Control:          2ch {}, 6ch {}, 8ch {}; DRC start-up gain {}, heavy DRC start-up gain {}",
        cm.twoch_control_enabled,
        cm.sixch_control_enabled,
        cm.eightch_control_enabled,
        cm.drc_start_up_gain,
        cm.heavy_drc_start_up_gain
    );
    println!(
        "Error protection: substream CRC present in {:?} access units; DRC updates {:?}",
        scan.crc_present_units, scan.drc_updates
    );
    let e = &scan.extra;
    println!(
        "Extra data:       {} access units ({} padding, {} opaque, {} Evolution blocks, {} Evolution frames, {} protected)",
        e.units_with_extra,
        e.padding_blocks,
        e.opaque_blocks,
        e.evolution_blocks,
        e.evolution_frames,
        e.protected_frames
    );
    if !e.payload_ids.is_empty() {
        let list: Vec<String> = e
            .payload_ids
            .iter()
            .map(|(id, n)| {
                let name = match *id {
                    oadec_emdf::PAYLOAD_ID_OAMD => " (object audio metadata)",
                    oadec_emdf::PAYLOAD_ID_JOC => " (joint object coding)",
                    _ => "",
                };
                format!(
                    "id {id}{name}: {n} payloads, {} bytes",
                    e.payload_bytes.get(id).copied().unwrap_or(0)
                )
            })
            .collect();
        println!("Evolution payloads: {}", list.join("; "));
    }
    let f = &scan.failures;
    println!(
        "Integrity:        header parity failures {}, major sync CRC failures {}, framing errors {}, configuration changes {}, extra-data parity failures {}, resyncs {}, skipped bytes {}, trailing bytes {}",
        f.header_parity,
        f.major_sync_crc,
        f.framing_errors,
        f.config_changes,
        f.extra_evolution_parity + f.extra_header_parity,
        f.resyncs,
        f.skipped_bytes,
        f.trailing_bytes
    );
    if let Some(err) = &scan.first_error {
        println!("First error:      {err}");
    }
}

/// Runs the command.
pub fn run(path: &Path, json: bool) -> Result<()> {
    let scan = scan::scan(path)?;
    if json {
        let mut value = serde_json::to_value(&scan)?;
        if let Some(ms) = &scan.first_major_sync {
            let map = ms.presentation_map();
            let restricted = ms.flags & oadec_truehd::sync::FLAG_RESTRICTED_8CH != 0;
            let fi = &ms.format_info;
            let extra = ms.channel_meaning.extra.as_ref();
            value["sampling_frequency"] = serde_json::json!(ms.sampling_frequency());
            value["samples_per_au"] = serde_json::json!(ms.samples_per_au());
            value["duration_seconds"] = serde_json::json!(scan.duration_seconds());
            value["peak_bit_rate"] = serde_json::json!(ms.peak_bit_rate());
            value["average_bit_rate"] = serde_json::json!(scan.average_bit_rate());
            value["variable_rate"] = serde_json::json!(ms.variable_rate);
            value["flags"] = serde_json::json!(ms.flags);
            value["substreams"] = serde_json::json!(ms.substreams);
            value["substream_info"] = serde_json::json!(ms.substream_info);
            value["extended_substream_info"] = serde_json::json!(ms.extended_substream_info & 3);
            value["has_16ch_presentation"] = serde_json::json!(ms.has_16ch_presentation());
            value["presentations"] =
                serde_json::json!((0..4)
                .map(|i| {
                    let chans = match i {
                        0 => ChannelLabel::two_channel(false),
                        1 => ChannelLabel::six_channel(fi.sixch_channel_assignment),
                        2 => ChannelLabel::eight_channel(fi.eightch_channel_assignment, restricted),
                        _ => extra.map(ChannelLabel::sixteen_channel).unwrap_or_default(),
                    };
                    serde_json::json!({
                        "index": i,
                        "substream_mask": map.mask(i),
                        "kind": kind_text(map.kind(i)),
                        "channels": chans.iter().map(ToString::to_string).collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>());
            // The channel meaning proper. `info` has printed these since it was
            // written and `--json` has not carried them, which makes a sweep
            // over a library blind to exactly the fields a differential wants.
            let cm = &ms.channel_meaning;
            value["channel_meaning"] = serde_json::json!({
                "heavy_drc_start_up_gain": cm.heavy_drc_start_up_gain,
                "drc_start_up_gain": cm.drc_start_up_gain,
                "twoch_control_enabled": cm.twoch_control_enabled,
                "sixch_control_enabled": cm.sixch_control_enabled,
                "eightch_control_enabled": cm.eightch_control_enabled,
                "twoch_dialogue_norm": cm.twoch_dialogue_norm,
                "twoch_mix_level": cm.twoch_mix_level,
                "sixch_dialogue_norm": cm.sixch_dialogue_norm,
                "sixch_mix_level": cm.sixch_mix_level,
                "sixch_source_format": cm.sixch_source_format,
                "eightch_dialogue_norm": cm.eightch_dialogue_norm,
                "eightch_mix_level": cm.eightch_mix_level,
                "eightch_source_format": cm.eightch_source_format,
                "reserved1": cm.reserved1,
                "reserved2": cm.reserved2,
                "extra_present": cm.extra_present,
            });
            value["sixteen_channel"] = serde_json::json!(extra.map(|x| serde_json::json!({
                "channels": x.channels(),
                "dyn_object_only": x.dyn_object_only,
                "lfe_present": x.lfe_present,
                "content_description": x.content_description,
                "lfe_only": x.lfe_only,
                "bed_assignment": x.sixteench_channel_assignment,
                "isf_index": x.isf_index,
                "dynamic_objects": x.dynamic_objects(),
                "dialogue_norm": x.sixteench_dialogue_norm,
                "mix_level": x.sixteench_mix_level,
            })));
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print_text(&scan);
    }
    Ok(())
}
