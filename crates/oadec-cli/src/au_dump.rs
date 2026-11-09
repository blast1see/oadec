//! `oadec au`: one access unit of a TrueHD stream, field by field, with where
//! each part sits in the file -- the place a finding of `verify` points to,
//! opened up.

use std::path::Path;

use anyhow::{Result, anyhow};
use oadec_emdf::container;
use oadec_truehd::{AccessUnit, ExtraKind, ParserState, SampleBuffer, Segment, StreamConfig};
use serde_json::{Value, json};

use crate::input;

/// The walk stops once the unit is described; this is how it says so.
#[derive(Debug)]
struct Stop;

impl core::fmt::Display for Stop {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("stop")
    }
}

impl std::error::Error for Stop {}

fn describe(
    index: u64,
    unit: &oadec_truehd::Unit,
    config_before: Option<&StreamConfig>,
    parser: &mut Option<ParserState>,
    buffer: &mut SampleBuffer,
) -> Value {
    let base = unit.offset;
    let mut out = json!({
        "unit": index,
        "offset": base,
        "bytes": unit.bytes.len(),
        "gap_before": unit.gap_before.map(|g| json!({
            "offset": g.offset, "bytes": g.bytes, "resyncs": g.resyncs,
            "major_sync_crc_failures": g.major_sync_crc_failures, "foreign_syncs": g.foreign_syncs,
        })),
    });
    let (au, config) = match AccessUnit::parse(&unit.bytes, config_before) {
        Ok(x) => x,
        Err(e) => {
            out["framing_error"] = json!(e.to_string());
            return out;
        }
    };
    out["header"] = json!({
        "check_nibble": au.header.check_nibble,
        "length_words": au.header.length_words,
        "input_timing": au.header.input_timing,
        "parity_ok": au.header_parity_ok,
    });
    out["seconds"] = json!(
        index as f64 * f64::from(config.samples_per_au) / f64::from(config.sampling_frequency)
    );
    out["major_sync"] = au.major_sync.as_ref().map_or(Value::Null, |ms| {
        json!({
            "format_sync": format!("{:08X}", ms.format_sync),
            "sampling_frequency": ms.sampling_frequency(),
            "flags": format!("{:#06X}", ms.flags),
            "variable_rate": ms.variable_rate,
            "peak_data_rate": ms.peak_data_rate,
            "substreams": ms.substreams,
            "substream_info": format!("{:#04X}", ms.substream_info),
            "extended_substream_info": ms.extended_substream_info,
            "length_bytes": ms.len_bytes,
            "crc_ok": ms.crc_ok,
        })
    });
    if au.major_sync.is_some() {
        match parser {
            Some(p) => p.update(&config),
            None => *parser = Some(ParserState::new(&config)),
        }
    }
    let mut substreams = Vec::new();
    for (i, entry) in au.directory.iter().enumerate() {
        let range = au.segment_range(i);
        let mut ss = json!({
            "substream": i,
            "offset": base + range.start as u64,
            "bytes": range.len(),
            "end_ptr_words": entry.end_ptr_words,
            "restart_nonexistent": entry.restart_nonexistent,
            "crc_present": entry.crc_present,
            "extra_word": entry.extra_word,
            "drc_gain_update": entry.drc_gain_update,
            "drc_time_update": entry.drc_time_update,
        });
        let Some(state) = parser.as_mut() else {
            ss["segment"] = json!("not parsed: no major sync yet");
            substreams.push(ss);
            continue;
        };
        if !state.substream[i].restart_seen && entry.restart_nonexistent {
            ss["segment"] = json!("not parsed: no restart header since the last fault");
            substreams.push(ss);
            continue;
        }
        let Some(bytes) = unit.bytes.get(range.clone()) else {
            ss["segment"] = json!("outside the unit");
            substreams.push(ss);
            continue;
        };
        match Segment::parse_located(bytes, state, i, entry.crc_present, buffer) {
            Ok(seg) => {
                let restart = seg.blocks.first().and_then(|b| b.restart.as_ref());
                ss["segment"] = json!({
                    "blocks": seg.blocks.len(),
                    "samples": seg.samples,
                    "restart": restart.map(|r| json!({
                        "sync_word": format!("{:#06X}", r.sync_word),
                        "output_timing": r.output_timing,
                        "min_chan": r.min_chan,
                        "max_chan": r.max_chan,
                        "max_matrix_chan": r.max_matrix_chan,
                        "lossless_check": r.lossless_check,
                        "error_protect": r.error_protect,
                        "crc_ok": r.crc_ok,
                    })),
                    "terminator": seg.terminator.map(|t| json!({
                        "zero_samples_indicated": t.zero_samples_indicated,
                        "zero_samples": t.zero_samples,
                        "tail_ok": t.tail_ok,
                    })),
                    "parity_ok": seg.parity_ok,
                    "crc_ok": seg.crc_ok,
                    "length_bits": seg.len_bits,
                    "end_ok": seg.end_ok,
                    "sample_count_ok": seg.sample_count_ok,
                    "unexpected_tail": seg.unexpected_tail,
                });
            }
            Err((e, bit)) => {
                state.substream[i].restart_seen = false;
                ss["segment"] = json!({
                    "error": e.to_string(),
                    "at_bit": bit,
                    "at_byte": base + range.start as u64 + (bit / 8) as u64,
                });
            }
        }
        substreams.push(ss);
    }
    out["substreams"] = json!(substreams);
    out["extra"] = au.extra.as_ref().map_or(Value::Null, |x| {
        let mut v = json!({
            "offset": base + au.segments_end() as u64,
            "length_words": x.length_words,
            "header_parity_ok": x.header_parity_ok,
            "parity_ok": x.parity_ok,
            "padding_zero": x.padding_zero,
            "trailing_bytes": x.trailing_bytes,
        });
        v["kind"] = json!(match &x.kind {
            ExtraKind::Padding => "padding",
            ExtraKind::Opaque(_) => "opaque",
            ExtraKind::Truncated => "truncated",
            ExtraKind::Evolution { .. } => "evolution",
        });
        if let ExtraKind::Evolution { reserved, frame } = &x.kind {
            v["evolution_reserved"] = json!(reserved);
            v["frame_bytes"] = json!(frame.len());
            match container::parse_evolution(frame) {
                Ok(c) => {
                    v["container"] = json!({
                        "version": c.version,
                        "key_id": c.key_id,
                        "payloads": c.payloads.iter().map(|p| json!({"id": p.id, "bytes": p.data.len()})).collect::<Vec<_>>(),
                        "protection_primary_bytes": c.protection.primary.len(),
                        "protection_secondary_bytes": c.protection.secondary.len(),
                    });
                }
                Err(e) => v["container"] = json!({"error": e.to_string()}),
            }
        }
        v
    });
    out
}

fn print_text(v: &Value) {
    let line = |label: &str, text: String| println!("{label:<19}{text}");
    line(
        "Access unit:",
        format!(
            "{} at byte {} (0x{:X}), {} bytes{}",
            v["unit"],
            v["offset"],
            v["offset"].as_u64().unwrap_or(0),
            v["bytes"],
            v["seconds"]
                .as_f64()
                .map_or_else(String::new, |s| format!(", {s:.3} s"))
        ),
    );
    if !v["gap_before"].is_null() {
        line("Skipped before:", v["gap_before"].to_string());
    }
    if let Some(e) = v["framing_error"].as_str() {
        line("Framing:", format!("does not frame: {e}"));
        return;
    }
    line("Header:", v["header"].to_string());
    line(
        "Major sync:",
        if v["major_sync"].is_null() {
            "none".to_string()
        } else {
            v["major_sync"].to_string()
        },
    );
    for ss in v["substreams"].as_array().into_iter().flatten() {
        line(
            &format!("Substream {}:", ss["substream"]),
            format!(
                "{} bytes at 0x{:X}; {}",
                ss["bytes"],
                ss["offset"].as_u64().unwrap_or(0),
                ss["segment"]
            ),
        );
    }
    line(
        "Extra data:",
        if v["extra"].is_null() {
            "none".to_string()
        } else {
            v["extra"].to_string()
        },
    );
}

/// Runs the command.
pub fn run(path: &Path, index: u64, json_out: bool) -> Result<()> {
    let mut config: Option<StreamConfig> = None;
    let mut parser: Option<ParserState> = None;
    let mut buffer = Box::<SampleBuffer>::default();
    let mut seen = 0u64;
    let mut found = None;
    let walk = input::for_each_unit(path, |unit| {
        let i = seen;
        seen += 1;
        if i == index {
            found = Some(describe(
                i,
                &unit,
                config.as_ref(),
                &mut parser,
                &mut buffer,
            ));
            return Err(anyhow::Error::new(Stop));
        }
        // keep the configuration and every substream's state current
        if let Ok((au, cfg)) = AccessUnit::parse(&unit.bytes, config.as_ref()) {
            if au.major_sync.is_some() || parser.is_none() {
                match &mut parser {
                    Some(p) => p.update(&cfg),
                    None => parser = Some(ParserState::new(&cfg)),
                }
            }
            if let Some(state) = parser.as_mut() {
                for (s, entry) in au.directory.iter().enumerate().take(state.substreams) {
                    let Some(bytes) = unit.bytes.get(au.segment_range(s)) else {
                        continue;
                    };
                    if !state.substream[s].restart_seen && entry.restart_nonexistent {
                        continue;
                    }
                    if Segment::parse(bytes, state, s, entry.crc_present, &mut buffer).is_err() {
                        state.substream[s].restart_seen = false;
                    }
                }
            }
            config = Some(cfg);
        }
        Ok(())
    });
    match walk {
        Err(e) if e.downcast_ref::<Stop>().is_some() => {}
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let Some(v) = found else {
        return Err(anyhow!(
            "the stream has {seen} access units; there is no access unit {index}"
        ));
    };
    if json_out {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        print_text(&v);
    }
    Ok(())
}
