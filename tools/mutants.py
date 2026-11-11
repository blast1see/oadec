#!/usr/bin/env python3
"""Break the decoder on purpose and see whether anything fails.

A green suite says nothing about what it would have caught. This changes one
load-bearing constant at a time, runs the unit tests, and reports which
mutations survive. A survivor is a hole in the suite, named -- and sometimes
the right answer, when the constant has no specification behind it and only a
comparison against another decoder can judge it.

    python tools/mutants.py
    python tools/mutants.py --out mutants.json
    python tools/mutants.py --check

The tree must be clean: every mutation is undone with `git checkout --`, and
the run refuses to start otherwise. Each entry is a bug someone could write,
not a random character: a table entry off by one, an index base, a scale
factor, a channel mapped to the wrong input.

The verdict is the exit code. 0: every mutation was applied and caught, but
for the ones `EXPECTED_SURVIVORS` names, which survived. 1: a mutation
survived that is not expected to, one that is expected to survive was caught,
or the text a mutation changes is no longer in its file. The last used to be
printed as SKIP and counted as nothing, so code that moved took its mutations
out of the run without a word; it is found before any test runs now, and
`--check` looks for nothing else, which is quick enough for CI. 2: the tree is
not clean, or the suite does not pass before any mutation.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

os.chdir(Path(__file__).resolve().parent.parent)

MUTANTS = [
    # (a name, the file, the text to find, what to put instead)
    ('an overrunning last element with a byte of payload after it',
     'crates/oadec-emdf/src/oamd.rs',
     '|| padding_bits >= 8 ||',
     '||'),
    ('an overrunning last element with fill that is not fill',
     'crates/oadec-emdf/src/oamd.rs',
     '|| !padding_zero)',
     ')'),
    ('an overrunning unknown element accepted',
     'crates/oadec-emdf/src/oamd.rs',
     '(matches!(element, Element::Unknown(_)) || padding_bits',
     '(padding_bits'),
    ('an overrunning element losing the reason it was refused',
     'crates/oadec-emdf/src/oamd.rs',
     'Err(OamdError::Bits(_)) if !fits',
     'Err(_) if !fits'),
    ('the DRC profile of dynrng read as the one of compr',
     'crates/oadec-emdf/src/programme.rs',
     '''                line: DrcProfile::from_code(r.read(3)? as u8),
                rf: DrcProfile::from_code(r.read(3)? as u8),''',
     '''                rf: DrcProfile::from_code(r.read(3)? as u8),
                line: DrcProfile::from_code(r.read(3)? as u8),'''),
    ('the DRC table, Film Standard and Film Light swapped',
     'crates/oadec-emdf/src/programme.rs',
     '1 => Self::FilmStandard,',
     '1 => Self::FilmLight,'),
    ('the loudness scale floor, -58 for the 8-bit fields',
     'crates/oadec-emdf/src/programme.rs',
     'true_peak.map(|v| half(v, -116.0))',
     'true_peak.map(|v| half(v, -58.0))'),
    ('the programme boundary shifted after the read',
     'crates/oadec-emdf/src/programme.rs',
     '''                frames = frames.saturating_mul(2);
                if r.read_bool()? {
                    break;
                }''',
     '''                if r.read_bool()? {
                    break;
                }
                frames = frames.saturating_mul(2);'''),
    ('version 0 going on past its first fill',
     'crates/oadec-emdf/src/programme.rs',
     'if version == 0 {',
     'if version == 9 {'),
    ('the descriptive payloads not filed by the E-AC-3 walk',
     'crates/oadec-cli/src/emdf.rs',
     '''                    *s.payload_ids.entry(p.id).or_default() += 1;
                    take_programme_payload(&mut s, p.id, &p.data);''',
     '''                    *s.payload_ids.entry(p.id).or_default() += 1;'''),
    ('the extended precision presence, x read from the top bit',
     'crates/oadec-emdf/src/oamd.rs',
     '''                        if presence & 1 != 0 {
                            steps[0] =''',
     '''                        if presence & 4 != 0 {
                            steps[0] ='''),
    ('an overrunning element tolerated when another follows it',
     'crates/oadec-emdf/src/oamd.rs',
     'if !fits && index + 1 != elements_count {',
     'if false && !fits && index + 1 != elements_count {'),
    ('an overrunning last element not flagged',
     'crates/oadec-emdf/src/oamd.rs',
     'let size_ok = fits && r.position() <= end;',
     'let size_ok = r.position() <= end;'),
    ('the warp mode an output without one reads as, normal instead of warping',
     'crates/oadec-spatial/src/program.rs',
     'self.signalled_warp.unwrap_or(WarpMode::Warping)',
     'self.signalled_warp.unwrap_or(WarpMode::Normal)'),
    ('a warp mode loss counted per payload, not per change',
     'crates/oadec-spatial/src/program.rs',
     'if t.warp_mode != reads_as && self.last_warp != Some(t.warp_mode) {',
     'if t.warp_mode != reads_as {'),
    ('the warp codes, Pro Logic IIx and Lo/Ro swapped',
     'crates/oadec-spatial/src/program.rs',
     'Self::ProLogicIIx => 2,',
     'Self::ProLogicIIx => 3,'),
    ('the warp name of Lo/Ro, the Conversion Tool option instead of the DAMF value',
     'crates/oadec-spatial/src/program.rs',
     'Self::LoRo => "LoRo",',
     'Self::LoRo => "downmix_loro",'),
    ('the dbmd warp byte, the not-indicated bit left set',
     'crates/oadec-spatial/src/dbmd.rs',
     's9[152] = 0x80 | w.code();',
     's9[152] = 0x84 | w.code();'),
    ('the dbmd warp byte, written outside the checksum',
     'crates/oadec-spatial/src/dbmd.rs',
     's9[152] = 0x80 | w.code();',
     's9[151] = 0x80 | w.code();'),
    ('the warp mode written to every DAMF header',
     'crates/oadec-spatial/src/damf.rs',
     'if let Some(w) = options.warp_mode {',
     'if let Some(w) = options.warp_mode.or(Some(WarpMode::Warping)) {'),
    ('--warp-mode stream not reading the stream',
     'crates/oadec-cli/src/damf.rs',
     'Self::Stream => declared,',
     'Self::Stream => None,'),
    ('the timeline not told the warp mode the output reads as',
     'crates/oadec-cli/src/damf.rs',
     '    timeline.signalled_warp = warp;\n',
     ''),
    ('--warp-mode accepted on a sample output',
     'crates/oadec-cli/src/lib.rs',
     'warp_mode.is_some() && !matches!(format, Format::Damf | Format::Adm)',
     'false && !matches!(format, Format::Damf | Format::Adm)'),
    ('a trim configuration tallied whatever it says',
     'crates/oadec-cli/src/oamd.rs',
     '    if c.default_trim {\n        return None;\n    }\n',
     ''),
    ('the branch trigger, without the peak data rate change',
     'crates/oadec-truehd/src/timing.rs',
     'if (r.output_jump || self.input_jump || self.rate_changed_this_unit)',
     'if (r.output_jump || self.input_jump)'),
    ('the branch data rate, the previous unit weighed against the new rate',
     'crates/oadec-truehd/src/timing.rs',
     '<= u64::from(self.prev_peak_data_rate) * u64::from(interval),',
     '<= u64::from(self.config.peak_data_rate) * u64::from(interval),'),
    ('the input timing rate test, the previous unit weighed against the new rate',
     'crates/oadec-truehd/src/timing.rs',
     '> u64::from(interval) * u64::from(self.prev_peak_data_rate);',
     '> u64::from(interval) * u64::from(self.config.peak_data_rate);'),
    ('the branch reaching only the restart header it was judged at',
     'crates/oadec-truehd/src/timing.rs',
     'r.branch = self.unit_branch;',
     'r.branch = None;'),
    ('the missing container, the walk asking for skip bytes first',
     'crates/oadec-cli/src/emdf.rs',
     '    opened == 0 && (has_skip || declares_joc)',
     '    opened == 0 && has_skip'),
    ('the missing container, the statistics not counting a frame without skip fields',
     'crates/oadec-cli/src/eac3.rs',
     '            if declares_emdf {',
     '            if false {'),
    ('the block-offset step, 32 -> 31',
     'crates/oadec-spatial/src/program.rs',
     'pub const BLOCK_OFFSET_STEP: u64 = 32;',
     'pub const BLOCK_OFFSET_STEP: u64 = 31;'),
    ('the steep switch, one slot back -> none',
     'crates/oadec-joc/src/lib.rs',
     'let back = usize::from(steep == SteepReading::Measured);',
     'let back = 0;'),
    ('the steep switch, one slot back -> two',
     'crates/oadec-joc/src/lib.rs',
     'let back = usize::from(steep == SteepReading::Measured);',
     'let back = 2 * usize::from(steep == SteepReading::Measured);'),
    ('dequantisation, nquant 96 -> 95',
     'crates/oadec-joc/src/lib.rs',
     'let nquant = if quant_idx == 0 { 96.0 } else { 192.0 };',
     'let nquant = if quant_idx == 0 { 95.0 } else { 192.0 };'),
    ('dequantisation, the 820 scale -> 821',
     'crates/oadec-joc/src/lib.rs',
     '(f64::from(q) - nquant / 2.0) * 820.0 / (4096.0 * (1.0 + f64::from(quant_idx)))',
     '(f64::from(q) - nquant / 2.0) * 821.0 / (4096.0 * (1.0 + f64::from(quant_idx)))'),
    ('the smooth ramp, ts + 1 -> ts',
     'crates/oadec-joc/src/lib.rs',
     'let f = (ts as f64 + 1.0) / n;',
     'let f = (ts as f64) / n;'),
    ('the WAVE mask of the left channel',
     'crates/oadec-eac3/src/program.rs',
     '            L | Ch1 => 0x1,',
     '            L | Ch1 => 0x4,'),
    ('the interchange rank of an unmapped location, 32 -> 0',
     'crates/oadec-eac3/src/program.rs',
     '            0 => 32,',
     '            0 => 0,'),
    ('the JOC input of the left surround, 3 -> 4',
     'crates/oadec-eac3/src/program.rs',
     '            Ls => 3,' + chr(10) + '            Rs => 4,',
     '            Ls => 4,' + chr(10) + '            Rs => 3,'),
    ('the height pair of configuration 4 mapped like configuration 1',
     'crates/oadec-eac3/src/program.rs',
     '            Vhl if dmx_config == 2 || dmx_config == 4 => 5,',
     '            Vhl if dmx_config == 2 => 5,'),
    ('the OAMD sample-offset table, 8 -> 9',
     'crates/oadec-emdf/src/oamd.rs',
     'pub const SAMPLE_OFFSET: [u16; 4] = [8, 16, 18, 24];',
     'pub const SAMPLE_OFFSET: [u16; 4] = [9, 16, 18, 24];'),
    ('the ramp-duration table, 32 -> 33',
     'crates/oadec-emdf/src/oamd.rs',
     '    32, 64, 128, 256, 320, 480, 1000, 1001, 1024, 1600, 1601, 1602, 1920, 2000, 2002, 2048,',
     '    33, 64, 128, 256, 320, 480, 1000, 1001, 1024, 1600, 1601, 1602, 1920, 2000, 2002, 2048,'),
    # The one that survives this pass on purpose. `MATRIX_ALIGN` has no clause
    # behind it, so no unit test can judge it without writing down the number
    # the code already holds. Two media tests do judge it, differentially
    # against Dolby: `the_steep_switch_is_measured_where_the_branch_is_the
    # _common_case` and `the_matrix_alignment_is_the_best_one_on_a_clip_it_was
    # _not_fitted_on`.
    ('the matrix alignment, 10 -> 9',
     'crates/oadec-joc/src/quadrature.rs',
     'pub const MATRIX_ALIGN: usize = 10;',
     'pub const MATRIX_ALIGN: usize = 9;'),
    ('the sparse chain, measured -> as printed',
     'crates/oadec-emdf/src/joc.rs',
     '(_, SparseReading::Measured) => chained as u8,',
     '(_, SparseReading::Measured) => ((u32::from(q[ch][pb - 1]) + vec[pb]) % nquant) as u8,'),
    ('the TrueHD major-sync CRC polynomial, 0x002D -> 0x002F',
     'crates/oadec-bits/src/crc.rs',
     'pub const CRC16_MAJOR_SYNC: Crc16 = Crc16::new(0x002D);',
     'pub const CRC16_MAJOR_SYNC: Crc16 = Crc16::new(0x002F);'),
    ('the restart-header CRC polynomial, 0x1D -> 0x1F',
     'crates/oadec-bits/src/crc.rs',
     'pub const CRC8_RESTART: Crc8 = Crc8::new(0x1D);',
     'pub const CRC8_RESTART: Crc8 = Crc8::new(0x1F);'),
    ('the substream CRC initial value, 0xA2 -> 0xA3',
     'crates/oadec-bits/src/crc.rs',
     'pub const CRC8_SUBSTREAM_INIT: u8 = 0xA2;',
     'pub const CRC8_SUBSTREAM_INIT: u8 = 0xA3;'),
    ('the DAMF y coordinate, front and back swapped',
     'crates/oadec-spatial/src/program.rs',
     '    [(p[0] - 0.5) * 2.0, (0.5 - p[1]) * 2.0, p[2]]',
     '    [(p[0] - 0.5) * 2.0, (p[1] - 0.5) * 2.0, p[2]]'),
    ('a dropped dependent substream no longer makes a stream unclean',
     'crates/oadec-cli/src/eac3.rs',
     '    p.decode_errors == 0',
     '    true || p.decode_errors == 0'),
    ('the programme verdict ignores a dropped dependent substream',
     'crates/oadec-eac3/src/program.rs',
     '            && self.dependent_dropped == 0',
     '            && true'),
    ('the programme verdict counts a second programme as unclean',
     'crates/oadec-eac3/src/program.rs',
     '        self.orphan_dependents == 0',
     '        self.other_program_frames == 0 && self.orphan_dependents == 0'),
    ('a mid-stream sampling-rate change is allowed again',
     'crates/oadec-truehd/src/au.rs',
     '        } else if self.sampling_frequency != next.sampling_frequency {',
     '        } else if false && self.sampling_frequency != next.sampling_frequency {'),
    ('the TrueHD verdict stops reading a counter',
     'crates/oadec-cli/src/scan.rs',
     '            && self.oamd_errors == 0',
     '            && true'),
    ('an inactive object counts as an authored mute',
     'crates/oadec-emdf/src/oamd.rs',
     '                    if u.basic_status != Status::Default {',
     '                    if true {'),
    # --- the rules --start added -------------------------------------------
    ('the start boundary, a unit that ends exactly at the target counted as after it',
     'crates/oadec-cli/src/decode.rs',
     'if self.skipped_samples + per_unit <= target {',
     'if self.skipped_samples + per_unit < target {'),
    ('the announced sample, reported as the beginning of the stream',
     'crates/oadec-cli/src/decode.rs',
     'self.started_at_sample = Some(self.skipped_samples);',
     'self.started_at_sample = Some(0);'),
    ('a start that was never reached reported as a file carrying no stream',
     'crates/oadec-cli/src/decode.rs',
     'if self.started_at_sample.is_some() || self.skipped_units == 0 {',
     'if self.started_at_sample.is_none() || self.skipped_units == 0 {'),
    ('--start accepted on E-AC-3 and the object outputs instead of refused',
     'crates/oadec-cli/src/lib.rs',
     '            } else if start.is_some()',
     '            } else if false && start.is_some()'),
    # --- verification upgrade: a foreign sync pattern ---------------------
    ("the hunt for sync takes the other format's pattern for a major sync again",
     'crates/oadec-truehd/src/extract.rs',
     'if self.format_sync.is_some_and(|f| f != pattern) {',
     'if self.format_sync.is_some_and(|f| f != pattern) && false {'),
    ('a DVD-Audio major sync believed without its CRC',
     'crates/oadec-truehd/src/sync.rs',
     'let computed = CRC16_MAJOR_SYNC.update_bytes(0, &head[..26]);',
     'let computed = crc;'),
    ('the framing takes the other format\'s pattern for a major sync',
     'crates/oadec-truehd/src/au.rs',
     '(Some(pattern), Some(cfg)) => pattern == cfg.format_sync,',
     '(Some(_), Some(_)) => true,'),
    ('the extractor takes the other format\'s pattern for a major sync',
     'crates/oadec-truehd/src/extract.rs',
     'let has_major_sync = pattern.is_some() && pattern == self.format_sync;',
     'let has_major_sync = pattern.is_some();'),
    # --- engine round: the search for sync and the unit it just refused -----
    ('the search for sync locks onto the major sync of the unit just refused',
     'crates/oadec-truehd/src/extract.rs',
     'let unit_at = sync_at.checked_sub(4).filter(|&at| at >= self.pos);',
     'let unit_at = sync_at.checked_sub(4);'),
    ('a major sync that fails its CRC counted where its unit is refused as well',
     'crates/oadec-truehd/src/extract.rs',
     '''                    _ => {
                        self.locked = false;
                        self.skip_to(start + 1);''',
     '''                    _ => {
                        self.note_crc_failure();
                        self.locked = false;
                        self.skip_to(start + 1);'''),
    ('the other pattern counted in a unit that is refused as well as by the search',
     'crates/oadec-truehd/src/extract.rs',
     '''            if !self.header_parity_ok(start, len, major_sync_len) {
                self.locked = false;''',
     '''            if !self.header_parity_ok(start, len, major_sync_len) {
                self.stats.foreign_syncs += u64::from(foreign);
                self.locked = false;'''),
    ('sync found again forgotten when its unit has not all arrived',
     'crates/oadec-truehd/src/extract.rs',
     '''            if self.pending() < len {
                return Ok(None);
            }
            let start = self.pos;''',
     '''            if self.pending() < len {
                self.resync_pending = false;
                return Ok(None);
            }
            let start = self.pos;'''),
    ('sync found again said of every unit after it',
     'crates/oadec-truehd/src/extract.rs',
     'resynced: std::mem::take(&mut self.resync_pending),',
     'resynced: self.resync_pending,'),
    # --- engine round: putting checksums right, for tests and fuzzers --------
    ('the parser reading on past a restart header whose CRC is wrong',
     'crates/oadec-truehd/src/restart.rs',
     '        } else if !rh.crc_ok {',
     '        } else if false {'),
    ('the CRC of a restart header written a bit late',
     'crates/oadec-truehd/src/restart.rs',
     'bit: start + len,',
     'bit: start + len + 1,'),
    ('the CRC of a major sync computed over the CRC as well',
     'crates/oadec-truehd/src/repair.rs',
     'let crc = CRC16_MAJOR_SYNC.update_bytes(0, &unit[4..at]);',
     'let crc = CRC16_MAJOR_SYNC.update_bytes(0, &unit[4..at + 2]);'),
    ('the check nibble of a unit computed over its header alone',
     'crates/oadec-truehd/src/repair.rs',
     'done.header_nibble = set_check_nibble(unit, 4, more);',
     'done.header_nibble = set_check_nibble(unit, 4, 0);'),
    ('the parity that closes a segment written without its constant',
     'crates/oadec-truehd/src/repair.rs',
     'let parity = xor_bytes(&segment[..at]) ^ PARITY_XOR;',
     'let parity = xor_bytes(&segment[..at]);'),
    ('a repair writing into the frame it wraps',
     'crates/oadec-truehd/src/repair.rs',
     'done.extra_parity = put(body, last, &[parity]);',
     'done.extra_parity = put(body, last - 1, &[parity]);'),
    ('crc1 taken back sixteen bits too far',
     'crates/oadec-eac3/src/repair.rs',
     'for _ in 0..data.len() * 8 {',
     'for _ in 0..data.len() * 8 + 16 {'),
    ('crc2 written without the sixteen zero bits behind the data',
     'crates/oadec-eac3/src/repair.rs',
     'CRC16.update_bytes(CRC16.update_bytes(0, data), &[0, 0])',
     'CRC16.update_bytes(0, data)'),
    # --- verification upgrade: rules, findings, regions --------------------
    ('the default threshold of verify raised to fatal',
     'crates/oadec-cli/src/lib.rs',
     '.unwrap_or(Severity::Error),',
     '.unwrap_or(Severity::Fatal),'),
    ('a finding that bumps no counter',
     'crates/oadec-cli/src/scan.rs',
     '    failures.bump(rule, 1);',
     ''),
    ('a listing limit that caps the count as well',
     'crates/oadec-truehd/src/diag.rs',
     '                t.count += 1;',
     '                if limit == 0 || t.listed.len() < limit {\n                    t.count += 1;\n                }'),
    ('any restart ends the damage of every substream',
     'crates/oadec-truehd/src/damage.rs',
     'open.damaged &= !(1u8 << substream);',
     'open.damaged = 0;'),
    ('a restart in a faulted segment ends its own damage',
     'crates/oadec-cli/src/scan.rs',
     'if seg.has_restart && clean_segment {',
     'if seg.has_restart {'),
    # --- verification upgrade: integrity rules ------------------------------
    ('an Evolution frame longer than its container passes unseen',
     'crates/oadec-cli/src/scan.rs',
     'if c.len_bits.div_ceil(8) != frame.len() {',
     'if false {'),
    ('bytes after the extra data pass unseen',
     'crates/oadec-cli/src/scan.rs',
     'if extra.trailing_bytes > 0 {',
     'if false {'),
    ('the delivering commands read only the counters, not the rules',
     'crates/oadec-cli/src/verify.rs',
     'clean: scan.first_major_sync.is_some() && scan.diag.tally.passes(Severity::Error),',
     'clean: scan.first_major_sync.is_some() && scan.failures.is_clean(),'),
    ('the clip a branch joins may not open with two major syncs',
     'crates/oadec-cli/src/scan.rs',
     'index <= 1 || branch_units.any(|b| b == index || b + 1 == index)',
     'index <= 1 || branch_units.any(|b| b == index)'),
    ('the reserved primary protection length passes unseen',
     'crates/oadec-cli/src/scan.rs',
     'if c.protection.primary_code == 0 {',
     'if false {'),
    ('the secondary protection length read from the primary code',
     'crates/oadec-emdf/src/container.rs',
     'let secondary_len = PROTECTION_SIZES[usize::from(secondary_code)];',
     'let secondary_len = PROTECTION_SIZES[usize::from(primary_code)];'),
    # --- verification upgrade: the decoder buffer --------------------------
    ('a unit leaves the buffer at its output time, not a frame later',
     'crates/oadec-truehd/src/buffer.rs',
     'self.live.push(Reverse((output + self.spa, add)));',
     'self.live.push(Reverse((output, add)));'),
    ('the allowance of a substream, 10 000 bytes a channel',
     'crates/oadec-truehd/src/buffer.rs',
     '    15_000 * channels',
     '    10_000 * channels'),
    ('the 75 ms bound, 3 samples in 41 instead of 40',
     'crates/oadec-truehd/src/buffer.rs',
     'if latency * 40 > self.fs * 3 {',
     'if latency * 41 > self.fs * 3 {'),
    ('a rate judged after rounding instead of exactly',
     'crates/oadec-truehd/src/buffer.rs',
     'if self.declared_peak_bps > 0 && bits_fs > self.declared_peak_bps * interval {',
     'if self.declared_peak_bps > 0 && bps > self.declared_peak_bps + 1 {'),
    ('Blu-ray taking eight channels at 192 kHz',
     'crates/oadec-truehd/src/disc.rs',
     'if fs == 192_000\n                && let Some((p, c)) = widest_over(f, 6)',
     'if fs == 192_000\n                && let Some((p, c)) = widest_over(f, 8)'),
    ("HD DVD's two-channel bound held at 96 kHz too",
     'crates/oadec-truehd/src/disc.rs',
     'if matches!(fs, 176_400 | 192_000)\n                && let Some((p, c)) = widest_over(f, 2)',
     'if fs > 48_000\n                && let Some((p, c)) = widest_over(f, 2)'),
    ('Blu-ray taking 44.1 kHz',
     'crates/oadec-truehd/src/disc.rs',
     'if !matches!(fs, 48_000 | 96_000 | 192_000) {',
     'if !matches!(fs, 44_100 | 48_000 | 96_000 | 192_000) {'),
    ('a single delivery finding let through',
     'crates/oadec-truehd/src/disc.rs',
     'if fba && disc != Disc::DvdAudio && !f.delivery.is_empty() {',
     'if fba && disc != Disc::DvdAudio && f.delivery.len() > 1 {'),
    ('a gap filled whatever the bytes it skipped say',
     'crates/oadec-cli/src/decode.rs',
     '(2.0 * estimate < 65_536.0 && (clock - estimate).abs() <= slack).then_some(u64::from(delta))',
     '(2.0 * estimate < 65_536.0 || slack >= 0.0).then_some(u64::from(delta))'),
    ('a damaged substream read again before its restart header',
     'crates/oadec-truehd/src/decoder.rs',
     '                if entry.restart_nonexistent {',
     '                if entry.restart_nonexistent && false {'),
    ('the check word of a section that took silence held against its header',
     'crates/oadec-truehd/src/decoder.rs',
     'if rt.branch.is_some_and(|b| b.is_valid()) || sd.spoiled {',
     'if rt.branch.is_some_and(|b| b.is_valid()) {'),
    ("a gap's silence written after the unit instead of before it",
     'crates/oadec-cli/src/decode.rs',
     '''        buf.resize(frame.lead_silence * frame.order.len() * 3, 0);
        let clipped = pack_24le(frame.pcm, frame.order, &mut buf);''',
     '''        let clipped = pack_24le(frame.pcm, frame.order, &mut buf);
        buf.resize(buf.len() + frame.lead_silence * frame.order.len() * 3, 0);'''),
    ("DVD-Audio assignment 20 read in the order of FFmpeg's output",
     'crates/oadec-truehd/src/channel.rs',
     '20 => vec![L, R, Lb, Rb, C, LFE],',
     '20 => vec![L, R, C, LFE, Lb, Rb],'),
    ("an FBB stream's second group judged by its word length, not its rate",
     'crates/oadec-truehd/src/au.rs',
     'if fbb.has_group2() && fbb.group2_rate_code != fbb.group1_rate_code {',
     'if fbb.has_group2() && fbb.group2_bits_code != fbb.group1_bits_code {'),
    ('an FBB stream restarting on 0x31EB',
     'crates/oadec-truehd/src/restart.rs',
     'if state.fbb && sync_word != SYNC_A {',
     'if state.fbb && sync_word == SYNC_C {'),
    ("an FBB stream decoded at TrueHD's default presentation",
     'crates/oadec-cli/src/decode.rs',
     '.and_then(|ms| ms.presentation_map().max_carried())',
     '.map(|_| 2)'),
    ('the DVD-Audio flag read at bit 15',
     'crates/oadec-truehd/src/disc.rs',
     'pub const FLAG_DVD_AUDIO: u16 = 0x4000;',
     'pub const FLAG_DVD_AUDIO: u16 = 0x8000;'),
    ('the WAVE header naming 48 kHz whatever the stream runs at',
     'crates/oadec-cli/src/decode.rs',
     'rate: frame.sampling_frequency,',
     'rate: 48_000,'),
    ('switching coupling off leaving the first-coordinate flags as they were',
     'crates/oadec-cli/src/ecpl_inject.rs',
     '        firstcplcos.fill(true);\n        return None;',
     '        return None;'),
    ('delta bit allocation segments left out of the bit allocation',
     'crates/oadec-eac3/src/frame.rs',
     '            dba,\n            he,',
     '            None,\n            he,'),
    ('a reused delta allocation (deltbae 0) treated as none',
     'crates/oadec-eac3/src/frame.rs',
     'let dba = if self.deltbae[slot] == 0 || self.deltbae[slot] == 1 {',
     'let dba = if self.deltbae[slot] == 1 {'),
    ('delta allocation counted where a block only carries the fields',
     'crates/oadec-eac3/src/frame.rs',
     'if dba.is_some_and(|d| d.lengths.iter().any(|&l| l > 0)) {',
     'if dba.is_some() {'),
    # --- the census of the TrueHD syntax -------------------------------------
    ('the census files an IIR set under the FIR sets',
     'crates/oadec-truehd/src/census.rs',
     '                    if is_iir {\n                        census.filter.iir_sent.add(order);',
     '                    if !is_iir {\n                        census.filter.iir_sent.add(order);'),
    ('the census counts a filter in force as a filter sent',
     'crates/oadec-truehd/src/census.rs',
     '                    if !new {\n                        continue;\n                    }',
     '                    if false && !new {\n                        continue;\n                    }'),
    ('a census value counted one place off',
     'crates/oadec-truehd/src/census.rs',
     '        match usize::try_from(value - i64::from(LOW))',
     '        match usize::try_from(value - i64::from(LOW) + 1)'),
    ('the census takes a filter of either kind for both at once',
     'crates/oadec-truehd/src/census.rs',
     'census.filter.fir_and_iir += u64::from(fir != 0 && iir != 0);',
     'census.filter.fir_and_iir += u64::from(fir != 0 || iir != 0);'),
    ('a matrix coefficient of the object syntax read at the wrong scale',
     'crates/oadec-truehd/src/census.rs',
     'let shift = 18 + i32::from(cf_shift_code) - i32::from(frac_bits);',
     'let shift = 18 - i32::from(cf_shift_code) - i32::from(frac_bits);'),
    ('the census files every block under substream 0',
     'crates/oadec-cli/src/scan.rs',
     'census.take_block(i, block, &state.substream[i]);',
     'census.take_block(0, block, &state.substream[i]);'),
    ('the coding tools of a TrueHD stream left out of the report',
     'crates/oadec-cli/src/scan.rs',
     '        self.coverage = self.census.tools();',
     '        self.coverage = Vec::new();'),
]


# The mutations that survive the unit tests on purpose, and why. Anything else
# that survives is a hole in the suite.
EXPECTED_SURVIVORS = {
    'the matrix alignment, 10 -> 9':
        'no clause states it; two media tests judge it against another decoder',
}


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=1800)


def clean():
    return not run(['git', 'status', '--porcelain']).stdout.strip()


_ap = argparse.ArgumentParser(description=(__doc__ or '').split('\n\n')[0])
_ap.add_argument('--out')
_ap.add_argument('--only', help='run the mutations whose name matches this regular expression')
_ap.add_argument('--check', action='store_true',
                 help='only check that the text every mutation changes is still in its file')
_args = _ap.parse_args()

selected = [m for m in MUTANTS if not _args.only or re.search(_args.only, m[0])]
names = [m[0] for m in MUTANTS]
duplicates = sorted({n for n in names if names.count(n) > 1})
unknown = sorted(set(EXPECTED_SURVIVORS) - set(names))
missing = [(name, path) for name, path, old, _ in selected
           if old not in open(path, encoding='utf-8').read()]
for name in duplicates:
    print(f'  two mutations share the name: {name}')
for name in unknown:
    print(f'  expected to survive, but no mutation has the name: {name}')
for name, path in missing:
    print(f'  MISSING  {name}: the text is not in {path}')
if duplicates or unknown or missing:
    print(f'\n{len(selected)} mutations, {len(missing)} cannot be applied; nothing was run')
    sys.exit(1)
if _args.check:
    print(f'{len(selected)} mutations, the text of every one is in its file')
    sys.exit(0)

if not clean():
    print('the tree is not clean; refusing to mutate it')
    sys.exit(2)

base = run(['cargo', 'test', '--workspace', '--locked'])
if base.returncode != 0:
    print('the suite does not pass before any mutation')
    sys.exit(2)
print('baseline green\n')

rows = []

for name, path, old, new in selected:
    src = open(path, encoding='utf-8').read()
    open(path, 'w', encoding='utf-8', newline='').write(src.replace(old, new, 1))
    r = run(['cargo', 'test', '--workspace', '--locked'])
    caught = r.returncode != 0
    failing = sorted({ln.split()[1] for ln in r.stdout.splitlines()
                      if ln.startswith('test ') and ln.rstrip().endswith('FAILED')})
    subprocess.run(['git', 'checkout', '--', path], check=True)
    rows.append({'mutation': name, 'applied': True, 'caught': caught,
                 'failing_tests': failing[:6], 'failing_count': len(failing)})
    expected = name in EXPECTED_SURVIVORS
    rows[-1]['expected_to_survive'] = expected
    mark = 'caught' if caught else ('survived' if expected else 'SURVIVED')
    print(f'  {mark:9s} {name}'
          f'{"  by " + ", ".join(failing[:3]) if failing else ""}'
          f'{f" (+{len(failing) - 3} more)" if len(failing) > 3 else ""}', flush=True)

if _args.out:
    json.dump(rows, open(_args.out, 'w'), indent=1)
survived = [r for r in rows if not r['caught']]
holes = [r for r in survived if not r['expected_to_survive']]
stale = [r for r in rows if r['caught'] and r['expected_to_survive']]
print(f'\n{len(rows)} mutations, {len(survived)} survived, {len(holes)} of them unexpected')
for r in survived:
    why = EXPECTED_SURVIVORS.get(r['mutation'])
    print('  survived:', r['mutation'], f'(expected: {why})' if why else '(a hole in the suite)')
for r in stale:
    print('  caught, though expected to survive:', r['mutation'],
          '-- take it out of EXPECTED_SURVIVORS')
assert clean(), 'the tree was left dirty'
print('tree clean')
sys.exit(1 if holes or stale else 0)
