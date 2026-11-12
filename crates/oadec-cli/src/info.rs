//! `oadec info`: what the stream declares about itself.

use anyhow::Result;
use oadec_truehd::sync::FbbFormat;
use oadec_truehd::{ChannelLabel, MajorSync, PresentationKind};

use crate::scan::{self, Scan};
use crate::source::Source;

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

#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
fn print_presentations(ms: &MajorSync) {
    let map = ms.presentation_map();
    if ms.is_fbb() {
        println!("Presentations:");
        for index in 0..2 {
            let kind = map.kind(index);
            if kind == PresentationKind::Invalid {
                continue;
            }
            println!(
                "  {index}: substreams {:04b}  {}  [{}]",
                map.mask(index),
                labels(&ChannelLabel::presentation(ms, index)),
                kind_text(kind)
            );
        }
        return;
    }
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

#[allow(clippy::print_stdout, reason = "printing is what this one is for")]
fn print_text(scan: &Scan) {
    println!(
        "File:             {} ({} bytes)",
        scan.file, scan.file_bytes
    );
    let Some(ms) = &scan.first_major_sync else {
        // A sync word that was found and could not be read is not a stream
        // without one: `verify` counts its access units and names the fault,
        // and saying here that there is no TrueHD stream would be another
        // story of the same file.
        match &scan.first_error {
            Some(error) if scan.major_syncs > 0 => {
                println!("No major sync could be read: {error}");
            }
            _ => println!("No major sync found: not a Dolby TrueHD stream."),
        }
        return;
    };
    let fs = ms.sampling_frequency().unwrap_or(0);
    if let Some(fbb) = &ms.fbb {
        let all = ChannelLabel::mlp_assignment(fbb.channel_assignment);
        let split = ChannelLabel::mlp_group1_channels(fbb.channel_assignment).min(all.len());
        let groups = if split < all.len() {
            format!("{} | {}", labels(&all[..split]), labels(&all[split..]))
        } else {
            labels(&all)
        };
        let bits = |code| FbbFormat::bits(code).map(|b| format!("{b}-bit"));
        let words = match (
            bits(fbb.group1_bits_code),
            fbb.has_group2()
                .then(|| bits(fbb.group2_bits_code))
                .flatten(),
        ) {
            (Some(g1), Some(g2)) if g1 != g2 => format!("group 1 {g1}, group 2 {g2}"),
            (Some(g1), _) => g1,
            (None, _) => "word length not declared".to_string(),
        };
        println!(
            "Format:           Meridian MLP (FBB, DVD-Audio), {fs} Hz, {} samples per access unit; channel assignment {} ({groups}), {words}",
            ms.samples_per_au().unwrap_or(0),
            fbb.channel_assignment,
        );
    } else {
        println!(
            "Format:           Dolby TrueHD (FBA), {fs} Hz, {} samples per access unit{}",
            ms.samples_per_au().unwrap_or(0),
            if ms.has_16ch_presentation() {
                ", 16-channel object presentation"
            } else {
                ""
            }
        );
    }
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
    // the dialogue level and the control fields are FBA's channel meaning;
    // an FBB one is laid out otherwise and is not read
    if !ms.is_fbb() {
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
    }
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
    if e.protection.containers > 0 {
        println!("Protection:       {}", e.protection.summary());
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
    if scan.extra.oamd_ok > 0 || f.oamd_errors > 0 {
        println!(
            "Object metadata:  {} payloads parsed, {} errors; {} non-unity gains {:?}, {} mutes, {} non-zero sizes",
            scan.extra.oamd_ok,
            f.oamd_errors,
            scan.extra.oamd_gains_db.values().sum::<u64>(),
            scan.extra.oamd_gains_db,
            scan.extra.oamd_muted_updates,
            scan.extra.oamd_sized_updates
        );
        crate::scan::print_stale_sizes(scan.extra.oamd_stale_sizes);
    }
    if let Some(err) = &scan.first_error {
        println!("First error:      {err}");
    }
}

/// The error of a reporting command (`info`, `emdf`, `oamd`) for a file in
/// which it found no stream at all.
///
/// Those commands report what a stream holds, and a report of nothing reads
/// like a clean stream that holds nothing: `info` printed the size of a
/// megabyte of random bytes and exited 0, and `oamd` reported "0 in 0 of 0
/// access units". A file that is not a stream is an unsupported input, exit 2
/// (`docs/exit-codes.md`).
pub(crate) fn no_stream(source: &Source) -> anyhow::Error {
    anyhow::anyhow!(
        "no TrueHD access unit or E-AC-3 syncframe found in {}",
        source.name()
    )
}

/// What `info` reads of a TrueHD stream.
#[derive(Debug)]
pub struct Report {
    // on the heap: a scan is large, and a report is handed from frame to frame
    scan: Box<Scan>,
}

/// Scans the stream and prints nothing.
pub fn report(source: &Source) -> Result<Report> {
    let scan = Box::new(scan::scan(source)?);
    if scan.units == 0 {
        return Err(no_stream(source));
    }
    Ok(Report { scan })
}

impl Report {
    /// The report as `info --json` prints it.
    ///
    /// # Errors
    /// When a part of the report does not serialise, which none does.
    pub fn json(&self) -> Result<serde_json::Value> {
        let scan: &Scan = &self.scan;
        let mut value = serde_json::to_value(scan)?;
        // `substreams` below is the declared count, which replaces the scan's
        // per-substream statistics under the same key; they stay reachable here.
        value["substream_stats"] = value["substreams"].clone();
        if let Some(ms) = &scan.first_major_sync {
            let map = ms.presentation_map();
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
                let chans = ChannelLabel::presentation(ms, i);
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
        Ok(value)
    }

    /// Prints the report on stdout, as JSON or as text.
    ///
    /// # Errors
    /// As [`Report::json`].
    #[allow(clippy::print_stdout, reason = "printing is what this one is for")]
    pub fn print(&self, json: bool) -> Result<()> {
        if json {
            println!("{}", serde_json::to_string_pretty(&self.json()?)?);
        } else {
            print_text(&self.scan);
        }
        Ok(())
    }
}
