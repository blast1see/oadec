//! `oadec verify`: integrity counters over a whole stream.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;

use crate::scan;

/// Runs the command; returns `true` when the stream is clean.
pub fn run(path: &Path, json: bool) -> Result<bool> {
    let started = Instant::now();
    let scan = scan::scan(path)?;
    let elapsed = started.elapsed().as_secs_f64();
    let clean = scan.failures.is_clean() && scan.first_major_sync.is_some();
    let speed = scan.duration_seconds().map(|d| d / elapsed.max(1e-9));
    if json {
        let mut value = serde_json::to_value(&scan)?;
        value["clean"] = serde_json::json!(clean);
        value["seconds"] = serde_json::json!(elapsed);
        value["realtime_factor"] = serde_json::json!(speed);
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("File:              {}", scan.file);
        println!(
            "Access units:      {} ({} major syncs, {} bytes)",
            scan.units, scan.major_syncs, scan.total_unit_bytes
        );
        let f = &scan.failures;
        println!("Header parity:     {} failures", f.header_parity);
        println!(
            "Major sync CRC:    {} failures, {} bad signatures",
            f.major_sync_crc, f.major_sync_signature
        );
        println!(
            "Framing:           {} errors, {} configuration changes",
            f.framing_errors, f.config_changes
        );
        println!(
            "Extra data:        {} header parity failures, {} truncated, {} Evolution parity failures, {} non-zero padding, {} container errors",
            f.extra_header_parity,
            f.extra_truncated,
            f.extra_evolution_parity,
            f.extra_padding_nonzero,
            f.evolution_container_errors
        );
        println!(
            "Sync:              {} resyncs, {} skipped bytes, {} trailing bytes",
            f.resyncs, f.skipped_bytes, f.trailing_bytes
        );
        let substreams = scan
            .config
            .as_ref()
            .map_or(0, |c| usize::from(c.substreams));
        for (i, st) in scan.substreams.iter().enumerate().take(substreams) {
            let words = st
                .sync_words
                .iter()
                .map(|(w, n)| format!("{w} x{n}"))
                .collect::<Vec<_>>()
                .join(", ");
            let first = st.first_restart.as_ref().map_or_else(String::new, |r| {
                format!(
                    ", channels {}..={} (matrix {}), error_protect {}",
                    r.min_chan, r.max_chan, r.max_matrix_chan, r.error_protect
                )
            });
            println!(
                "Substream {i}:       {} segments ({} skipped), {} restart headers [{words}]{first}",
                st.segments, st.skipped, st.restart_headers
            );
            println!(
                "                   {} blocks (max {} per segment, {} protected), {} matrixing ({} interpolated), up to {} matrices, {} CRC segments, {} terminated ({} with zero samples)",
                st.blocks,
                st.max_blocks_per_segment,
                st.protected_blocks,
                st.matrixing_blocks,
                st.interpolated_blocks,
                st.max_primitive_matrices,
                st.crc_segments,
                st.terminated_segments,
                st.zero_sample_segments
            );
        }
        println!(
            "Substream checks:  {} parse errors, {} block bit-count mismatches, {} parity failures, {} CRC failures, {} end-pointer mismatches, {} sample-count mismatches, {} restart flag mismatches, {} bad terminators, {} unexpected tails",
            f.substream_errors,
            f.block_data_bits,
            f.segment_parity,
            f.segment_crc,
            f.segment_end,
            f.sample_count,
            f.restart_flag,
            f.terminator_tail,
            f.unexpected_tail
        );
        if let Some(err) = &scan.first_error {
            println!("First error:       {err}");
        }
        match speed {
            Some(x) => println!("Speed:             {elapsed:.2} s ({x:.0}x realtime)"),
            None => println!("Speed:             {elapsed:.2} s"),
        }
        println!(
            "Result:            {}",
            if clean { "CLEAN" } else { "NON-CONFORMANT" }
        );
    }
    Ok(clean)
}
