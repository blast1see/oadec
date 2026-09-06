//! One pass over a TrueHD stream collecting what `info` and `verify` report.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use oadec_emdf::container;
use oadec_truehd::{AccessUnit, ExtraKind, MajorSync, StreamConfig, Unit};
use serde::Serialize;

use crate::input::{self, PassSummary};

/// Integrity counters (all should be zero for a conformant stream).
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Failures {
    pub header_parity: u64,
    pub major_sync_crc: u64,
    pub major_sync_signature: u64,
    pub framing_errors: u64,
    pub extra_header_parity: u64,
    pub extra_truncated: u64,
    pub extra_evolution_parity: u64,
    pub extra_padding_nonzero: u64,
    pub evolution_container_errors: u64,
    pub resyncs: u64,
    pub skipped_bytes: u64,
    pub trailing_bytes: u64,
    pub config_changes: u64,
}

impl Failures {
    /// Whether every counter is zero.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.header_parity == 0
            && self.major_sync_crc == 0
            && self.major_sync_signature == 0
            && self.framing_errors == 0
            && self.extra_header_parity == 0
            && self.extra_truncated == 0
            && self.extra_evolution_parity == 0
            && self.extra_padding_nonzero == 0
            && self.evolution_container_errors == 0
            && self.resyncs == 0
            && self.skipped_bytes == 0
            && self.trailing_bytes == 0
            && self.config_changes == 0
    }
}

/// Extra-data statistics.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExtraStats {
    pub units_with_extra: u64,
    pub padding_blocks: u64,
    pub opaque_blocks: u64,
    pub evolution_blocks: u64,
    pub evolution_frames: u64,
    /// Payload id -> occurrences.
    pub payload_ids: BTreeMap<u32, u64>,
    /// Payload id -> total bytes.
    pub payload_bytes: BTreeMap<u32, u64>,
    pub protected_frames: u64,
}

/// Everything learned in one pass.
#[derive(Debug, Clone, Serialize)]
pub struct Scan {
    pub file: String,
    pub file_bytes: u64,
    pub units: u64,
    pub major_syncs: u64,
    pub max_major_sync_interval: u64,
    pub min_unit_bytes: usize,
    pub max_unit_bytes: usize,
    pub total_unit_bytes: u64,
    pub first_input_timing: Option<u16>,
    pub last_input_timing: Option<u16>,
    pub first_unit_offset: Option<u64>,
    #[serde(skip)]
    pub first_major_sync: Option<MajorSync>,
    #[serde(skip)]
    pub config: Option<StreamConfig>,
    pub crc_present_units: [u64; 4],
    pub drc_updates: [u64; 4],
    pub extra: ExtraStats,
    pub failures: Failures,
    pub first_error: Option<String>,
}

impl Scan {
    fn new(path: &Path) -> Self {
        Self {
            file: path.display().to_string(),
            file_bytes: 0,
            units: 0,
            major_syncs: 0,
            max_major_sync_interval: 0,
            min_unit_bytes: usize::MAX,
            max_unit_bytes: 0,
            total_unit_bytes: 0,
            first_input_timing: None,
            last_input_timing: None,
            first_unit_offset: None,
            first_major_sync: None,
            config: None,
            crc_present_units: [0; 4],
            drc_updates: [0; 4],
            extra: ExtraStats::default(),
            failures: Failures::default(),
            first_error: None,
        }
    }

    fn note_error(&mut self, unit_index: u64, offset: u64, what: &str) {
        self.failures.framing_errors += 1;
        if self.first_error.is_none() {
            self.first_error = Some(format!("access unit {unit_index} at byte {offset}: {what}"));
        }
    }

    fn take_unit(&mut self, unit: &Unit, since_major_sync: &mut u64) {
        let index = self.units;
        self.units += 1;
        self.min_unit_bytes = self.min_unit_bytes.min(unit.bytes.len());
        self.max_unit_bytes = self.max_unit_bytes.max(unit.bytes.len());
        self.total_unit_bytes += unit.bytes.len() as u64;
        if self.first_unit_offset.is_none() {
            self.first_unit_offset = Some(unit.offset);
        }
        if unit.has_major_sync {
            self.major_syncs += 1;
            self.max_major_sync_interval = self.max_major_sync_interval.max(*since_major_sync);
            *since_major_sync = 0;
        }
        *since_major_sync += 1;

        let (au, config) = match AccessUnit::parse(&unit.bytes, self.config.as_ref()) {
            Ok(x) => x,
            Err(e) => {
                self.note_error(index, unit.offset, &e.to_string());
                return;
            }
        };
        if self.first_input_timing.is_none() {
            self.first_input_timing = Some(au.header.input_timing);
        }
        self.last_input_timing = Some(au.header.input_timing);
        if !au.header_parity_ok {
            self.failures.header_parity += 1;
        }
        if let Some(ms) = &au.major_sync {
            if !ms.crc_ok {
                self.failures.major_sync_crc += 1;
            }
            if ms.signature != oadec_truehd::sync::SIGNATURE {
                self.failures.major_sync_signature += 1;
            }
            match &self.first_major_sync {
                None => self.first_major_sync = Some(ms.clone()),
                Some(first) => {
                    if first.substream_info != ms.substream_info
                        || first.extended_substream_info != ms.extended_substream_info
                        || first.substreams != ms.substreams
                        || first.format_info != ms.format_info
                    {
                        self.failures.config_changes += 1;
                    }
                }
            }
        }
        for (i, entry) in au.directory.iter().enumerate().take(4) {
            if entry.crc_present {
                self.crc_present_units[i] += 1;
            }
            if entry.extra_word {
                self.drc_updates[i] += 1;
            }
        }
        if let Some(extra) = &au.extra {
            self.extra.units_with_extra += 1;
            if !extra.header_parity_ok {
                self.failures.extra_header_parity += 1;
            }
            if !extra.padding_zero {
                self.failures.extra_padding_nonzero += 1;
            }
            match &extra.kind {
                ExtraKind::Padding => self.extra.padding_blocks += 1,
                ExtraKind::Opaque(_) => self.extra.opaque_blocks += 1,
                ExtraKind::Truncated => self.failures.extra_truncated += 1,
                ExtraKind::Evolution { frame, .. } => {
                    self.extra.evolution_blocks += 1;
                    if extra.parity_ok == Some(false) {
                        self.failures.extra_evolution_parity += 1;
                    }
                    if !frame.is_empty() {
                        self.extra.evolution_frames += 1;
                        match container::parse_evolution(frame) {
                            Ok(c) => {
                                if !c.protection.primary.is_empty() {
                                    self.extra.protected_frames += 1;
                                }
                                for p in &c.payloads {
                                    *self.extra.payload_ids.entry(p.id).or_default() += 1;
                                    *self.extra.payload_bytes.entry(p.id).or_default() +=
                                        p.data.len() as u64;
                                }
                            }
                            Err(e) => {
                                self.failures.evolution_container_errors += 1;
                                if self.first_error.is_none() {
                                    self.first_error =
                                        Some(format!("access unit {index}: evolution frame: {e}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.config = Some(config);
    }

    fn finish(&mut self, summary: PassSummary) {
        self.file_bytes = summary.file_bytes;
        self.failures.resyncs = summary.stats.resyncs;
        self.failures.skipped_bytes = summary.stats.skipped_bytes;
        self.failures.trailing_bytes = summary.trailing_bytes;
        self.failures.major_sync_crc += summary.stats.major_sync_crc_failures;
        if self.min_unit_bytes == usize::MAX {
            self.min_unit_bytes = 0;
        }
    }

    /// Duration in seconds implied by the access-unit count.
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        let cfg = self.config.as_ref()?;
        Some(self.units as f64 * f64::from(cfg.samples_per_au) / f64::from(cfg.sampling_frequency))
    }

    /// Average bit rate in bits per second.
    #[must_use]
    pub fn average_bit_rate(&self) -> Option<f64> {
        let secs = self.duration_seconds()?;
        (secs > 0.0).then(|| self.total_unit_bytes as f64 * 8.0 / secs)
    }
}

/// Scans a whole file.
pub fn scan(path: &Path) -> Result<Scan> {
    let mut scan = Scan::new(path);
    let mut since_major_sync = 0u64;
    let summary = input::for_each_unit(path, |unit| {
        scan.take_unit(&unit, &mut since_major_sync);
        Ok(())
    })?;
    scan.finish(summary);
    Ok(scan)
}
