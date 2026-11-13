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
     'crates/oadec-cli/src/api.rs',
     'fail_on: options.fail_on.unwrap_or(Severity::Error),',
     'fail_on: options.fail_on.unwrap_or(Severity::Fatal),'),
    ('a finding that bumps no counter',
     'crates/oadec-cli/src/scan.rs',
     '    failures.bump(rule, 1);',
     ''),
    ('a listing limit that caps the count as well',
     'crates/oadec-diag/src/tally.rs',
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
     '        0 => before,\n        _ => false,',
     '        _ => false,'),
    ('delta allocation counted where a block only carries the fields',
     'crates/oadec-eac3/src/frame.rs',
     'if dba.is_some_and(|d| d.lengths.iter().any(|&l| l > 0)) {',
     'if dba.is_some() {'),
    # --- where a command reads from, and the check beside it -----------------
    ('a check that is dropped is not told to stop',
     'crates/oadec-cli/src/verify.rs',
     '            self.given_up.store(true, Ordering::Relaxed);\n',
     ''),
    ('the check on a thread of its own never looks at its flag',
     'crates/oadec-cli/src/verify.rs',
     'stream_check(&source, &|| !flag.load(Ordering::Relaxed))',
     'stream_check(&source, &|| flag.load(Ordering::Relaxed) || true)'),
    ('a TrueHD scan that is told to stop goes on',
     'crates/oadec-cli/src/scan.rs',
     '        if !go_on() {\n            anyhow::bail!("the scan was given up");\n        }\n',
     ''),
    ('an E-AC-3 pass that is told to stop goes on',
     'crates/oadec-cli/src/eac3.rs',
     '            if !go_on() {\n                bail!("the pass was given up");\n            }\n',
     ''),
    ('bytes in memory said to have no size',
     'crates/oadec-cli/src/source.rs',
     'From::Memory(bytes) => bytes.len() as u64,',
     'From::Memory(_) => 0,'),
    # --- what a command finds, apart from how it prints it -------------------
    ('a declared loss put before a fault in the verdict of a delivery',
     'crates/oadec-cli/src/integrity.rs',
     '        if !self.is_clean() {\n            Verdict::Faulty\n        } else if self.lossy {\n            Verdict::Lossy\n        } else {',
     '        if self.lossy {\n            Verdict::Lossy\n        } else if !self.is_clean() {\n            Verdict::Faulty\n        } else {'),
    ('a report of verify said to be clean whatever it found',
     'crates/oadec-cli/src/verify.rs',
     '    pub fn clean(&self) -> bool {\n        self.clean\n    }',
     '    pub fn clean(&self) -> bool {\n        true\n    }'),
    ('a WAVE file written without its header',
     'crates/oadec-cli/src/decode.rs',
     '            self.out = Some(Output::Wav(WavOut::create(raw, spec)?));',
     '            let _ = spec;\n            self.out = Some(Output::Raw(raw));'),
    ('a decode that held nothing leaving its empty file behind',
     'crates/oadec-cli/src/decode.rs',
     '        drop(self.out.take());\n        let _ = std::fs::remove_file(&self.path);',
     '        drop(self.out.take());'),
    ('the warning for a channel mask not passed on by a decode',
     'crates/oadec-cli/src/decode.rs',
     '                writeln!(remarks, "warning: {warning}")?;',
     '                let _ = warning;'),
    ('the options of the TrueHD checks let through for an AC-3 family stream',
     'crates/oadec-cli/src/api.rs',
     '            return Err(anyhow!(BUFFER_AND_DISC_ARE_TRUEHD));',
     '            let _ = BUFFER_AND_DISC_ARE_TRUEHD;'),
    ('the verdict line left off the text of verify for an AC-3 family stream',
     'crates/oadec-cli/src/api.rs',
     '                if !json {\n                    report.print_verdict();\n                }\n',
     ''),
    ('a call that decodes in the order of the stream whatever it is asked',
     'crates/oadec-cli/src/api.rs',
     '            format: decode::Format::Pcm,\n            order: options.order,\n            keep_duplicates',
     '            format: decode::Format::Pcm,\n            order: Order::Stream,\n            keep_duplicates'),
    ('the text of a walk over access units printed for one over syncframes',
     'crates/oadec-cli/src/emdf.rs',
     '        } else if self.truehd {',
     '        } else if !self.truehd {'),
    ('the loss report of a TrueHD object decode not written',
     'crates/oadec-cli/src/damf.rs',
     '    written.print_summary();\n    written.write_loss_report()?;\n    Ok(written.findings()?.report())',
     '    written.print_summary();\n    Ok(written.findings()?.report())'),
    ('a call that lists a payload in full as it walks',
     'crates/oadec-cli/src/api.rs',
     '        &oamd::Options {\n            json: false,\n            dump: None,',
     '        &oamd::Options {\n            json: false,\n            dump: Some(1),'),
    ('the JOC measurement overrides let through for a TrueHD stream',
     'crates/oadec-cli/src/api.rs',
     '        bail!(JOC_MEASURES_EAC3_OBJECTS);',
     '        let _ = JOC_MEASURES_EAC3_OBJECTS;'),
    ('what a TrueHD decode does with a bad unit let through for an E-AC-3 object decode',
     'crates/oadec-cli/src/api.rs',
     '    if ac3 && options.on_error.is_some() {\n        bail!(ON_ERROR_IS_TRUEHD);\n    }\n',
     ''),
    ('one of the options of the TrueHD checks let through for an AC-3 family stream',
     'crates/oadec-cli/src/api.rs',
     '        if options.trace_fifo.is_some() || !options.require_disc.is_empty() {',
     '        if options.trace_fifo.is_some() {'),
    # --- a stream on standard input -------------------------------------------
    ('a dash taken for the name of a file',
     'crates/oadec-cli/src/source.rs',
     '        if path.as_os_str() == STDIN {',
     '        if false {'),
    ('a stream on standard input named by the dash that asked for it',
     'crates/oadec-cli/src/source.rs',
     'Self::read_whole(STDIN_NAME, stdin.lock())',
     'Self::read_whole(STDIN, stdin.lock())'),
    ('the last piece of a stream read to its end left out',
     'crates/oadec-cli/src/source.rs',
     '        if !piece.is_empty() {\n            out.push(piece.into_boxed_slice());',
     '        if whole {\n            out.push(piece.into_boxed_slice());'),
    ('a stream in pieces read on from where the piece before was left',
     'crates/oadec-cli/src/source.rs',
     '                self.piece += 1;\n                self.at = 0;',
     '                self.piece += 1;'),
    ('the size of a stream in pieces taken from its first piece',
     'crates/oadec-cli/src/source.rs',
     'pieces.iter().map(|p| p.len() as u64).sum()',
     'pieces.first().map_or(0, |p| p.len() as u64)'),
    # --- one verdict on the same bytes ----------------------------------------
    ('a payload of another programme taken as one of the first',
     'crates/oadec-spatial/src/program.rs',
     '        Some(p) if *p != program => {\n            return Err(ProgramError::ProgramChanged(format!(',
     '        Some(p) if *p != program && p.beds.len() > 99 => {\n            return Err(ProgramError::ProgramChanged(format!('),
    ('a payload without an object element taken by a command that only reads',
     'crates/oadec-spatial/src/program.rs',
     '        take_payload(&mut self.program, oamd).map(|_| ())',
     '        take_payload(&mut self.program, oamd).map(|_| ()).or(Ok(()))'),
    ('verify not making the refusals of an object output on a TrueHD stream',
     'crates/oadec-cli/src/scan.rs',
     'if let Err(e) = self.payloads.take(&oamd) {',
     'if let Err(e) = Ok::<(), oadec_spatial::ProgramError>(()) {'),
    ('a payload the programme cannot take demoted to a warning',
     'crates/oadec-truehd/src/rules.rs',
     '        id: "evo.oamd_unusable",\n        severity: Error,',
     '        id: "evo.oamd_unusable",\n        severity: Warning,'),
    ('verify not making the refusals of an object output on an E-AC-3 stream',
     'crates/oadec-cli/src/eac3.rs',
     'if let Err(e) = self.payloads.take(&oamd) {',
     'if let Err(e) = Ok::<(), oadec_spatial::ProgramError>(()) {'),
    ('a refused payload left out of the verdict of an E-AC-3 stream',
     'crates/oadec-cli/src/eac3.rs',
     '        && p.emdf.oamd_errors == 0\n        && p.emdf.oamd_unusable == 0\n',
     '        && p.emdf.oamd_errors == 0\n'),
    ('a refused payload left out of what a decode of an E-AC-3 stream found',
     'crates/oadec-cli/src/eac3.rs',
     '    f.note(\n        p.emdf.oamd_unusable,\n        "Object Audio Metadata payloads the programme cannot take",\n    );\n',
     ''),
    ('info calling a major sync it could not read no major sync',
     'crates/oadec-cli/src/info.rs',
     '            Some(error) if scan.major_syncs > 0 => {',
     '            Some(error) if scan.major_syncs > 1 => {'),
    ('a delivery let clear what verify found',
     'crates/oadec-fuzz/src/parity.rs',
     '            return fail("a delivery clears what verify found");',
     '            let _ = fail("a delivery clears what verify found");'),
    ('a faulty delivery let stand beside a clean verify --decode',
     'crates/oadec-fuzz/src/parity.rs',
     '        return fail("a delivery is faulty and verify --decode is clean");',
     '        let _ = fail("a delivery is faulty and verify --decode is clean");'),
    ('one report let find a stream where another finds none',
     'crates/oadec-fuzz/src/parity.rs',
     '        if refusing != reports.len() {',
     '        if refusing > reports.len() {'),
    ('every counter of verify taken for a fault of a delivery',
     'crates/oadec-fuzz/src/parity.rs',
     '        !NOT_OF_A_DELIVERY.contains(&name.as_str()) && count.as_u64() != Some(0)',
     '        count.as_u64() != Some(0)'),
    # --- a repair that leaves nothing for a second one --------------------------
    ('a restart CRC written into the bytes that close its segment',
     'crates/oadec-truehd/src/repair.rs',
     '                    if trailer.is_some_and(|at| site.bit + 8 > at * 8) {',
     '                    if trailer.is_some_and(|at| site.bit + 8 > at * 8 + 16) {'),
    ('a restart CRC that begins before the trailer and ends in it written all the same',
     'crates/oadec-truehd/src/repair.rs',
     '                    if trailer.is_some_and(|at| site.bit + 8 > at * 8) {',
     '                    if trailer.is_some_and(|at| site.bit >= at * 8) {'),
    # --- what every catalog keeps (crates/oadec-diag) -----------------------
    ('a name two rules share passing the catalog',
     'crates/oadec-diag/src/rule.rs',
     '        if !names.insert(id) {',
     '        if !names.insert(id) && id.is_empty() {'),
    ('a rule that fails a stream on the word of another tool alone passing the catalog',
     'crates/oadec-diag/src/rule.rs',
     '        if !independent && info.severity > Severity::Warning {',
     '        if !independent && info.severity > Severity::Error {'),
    ('a run passing with a finding at the threshold it fails at',
     'crates/oadec-diag/src/tally.rs',
     '        self.worst().is_none_or(|w| w < fail_on)',
     '        self.worst().is_none_or(|w| w <= fail_on)'),
    # --- the checks of the AC-3 family, by name -------------------------------
    # A rule and its counter are one fact told twice, and the pass is held to
    # that with every stream a debug build reads: a finding that is not filed,
    # or weighs another amount than its counter counts, stops it.
    ('a sync error filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::FrameSync, Location::byte(offset), 1, || {',
     'diag.add(RuleId::FrameSync, Location::byte(offset), 0, || {'),
    ('a run of skipped bytes filed as one',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::FrameSkipped, Location::byte(offset), bytes, || {',
     'diag.add(RuleId::FrameSkipped, Location::byte(offset), 1, || {'),
    ('a frame CRC failure filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::FrameCrc, frame_at(index), 1, || {',
     'diag.add(RuleId::FrameCrc, frame_at(index), 0, || {'),
    ('a frame that ends inside its tail filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::FrameTailOverrun, frame_at(index), 1, || {',
     'diag.add(RuleId::FrameTailOverrun, frame_at(index), 0, || {'),
    ('a CRC failure of a dependent substream filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::SubstreamCrc, at, 1, || "CRC failure".to_string());',
     'diag.add(RuleId::SubstreamCrc, at, 0, || "CRC failure".to_string());'),
    ('a dependent frame that ends inside its tail filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::SubstreamTailOverrun, at, 1, || {',
     'diag.add(RuleId::SubstreamTailOverrun, at, 0, || {'),
    ('a dependent frame that would not decode filed as the independent substream\'s',
     'crates/oadec-cli/src/eac3.rs',
     '            let (rule, place) = if key.0 == 1 {',
     '            let (rule, place) = if key.0 == 9 {'),
    ('what the programme counts filed under the rule next to it',
     'crates/oadec-cli/src/eac3.rs',
     '            stats.dependent_dropped,\n            stats.orphan_dependents,\n',
     '            stats.orphan_dependents,\n            stats.dependent_dropped,\n'),
    ('a misaligned frame and an unreadable channel map filed as each other',
     'crates/oadec-cli/src/eac3.rs',
     '            stats.misaligned,\n            stats.location_errors,\n',
     '            stats.location_errors,\n            stats.misaligned,\n'),
    ('a repeated frame and a stalled substream filed as each other',
     'crates/oadec-cli/src/eac3.rs',
     '            stats.duplicate_substream_frames,\n            stats.stalled_substreams,\n',
     '            stats.stalled_substreams,\n            stats.duplicate_substream_frames,\n'),
    ('what the programme counts filed again every time it is looked at',
     'crates/oadec-cli/src/eac3.rs',
     '                diag.add(*rule, at, 1, || (*says).to_string());\n            }\n            *before = after;',
     '                diag.add(*rule, at, 1, || (*says).to_string());\n            }'),
    ('a frame without a container filed in a stream that carries no EMDF',
     'crates/oadec-cli/src/eac3.rs',
     '        if self.missing_containers() > 0 {\n            for &frame in &self.missing_frames {',
     '        if !self.missing_frames.is_empty() {\n            for &frame in &self.missing_frames {'),
    ('a payload configuration outside the table filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::EmdfPayloadConfig, at, 1, || v.clone());',
     'diag.add(RuleId::EmdfPayloadConfig, at, 0, || v.clone());'),
    ('a complexity index that disagrees filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::BsiComplexityIndex, frame_at(index), 1, || {',
     'diag.add(RuleId::BsiComplexityIndex, frame_at(index), 0, || {'),
    ('an Object Audio Metadata payload that will not parse filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::OamdParse, at, 1, || e.to_string());',
     'diag.add(RuleId::OamdParse, at, 0, || e.to_string());'),
    ('a payload the programme cannot take filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::OamdUnusable, at, 1, || e.to_string());',
     'diag.add(RuleId::OamdUnusable, at, 0, || e.to_string());'),
    ('a stale element size filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::OamdStaleSize, at, 1, || {',
     'diag.add(RuleId::OamdStaleSize, at, 0, || {'),
    ('a JOC payload that will not parse filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::JocParse, at, 1, || e.to_string());',
     'diag.add(RuleId::JocParse, at, 0, || e.to_string());'),
    ('a reserved JOC extension filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::JocReservedExtConfig, at, 1, || {',
     'diag.add(RuleId::JocReservedExtConfig, at, 0, || {'),
    ('a JOC payload of the wrong size filed with no weight',
     'crates/oadec-cli/src/eac3.rs',
     'diag.add(RuleId::JocSizeMismatch, at, 1, || {',
     'diag.add(RuleId::JocSizeMismatch, at, 0, || {'),
    ('a rule held to the counter of another',
     'crates/oadec-cli/src/eac3.rs',
     '        (RuleId::OamdParse, e.oamd_errors),',
     '        (RuleId::OamdParse, e.oamd_unusable),'),
    ('the time of a finding counted in frames',
     'crates/oadec-cli/src/eac3.rs',
     '"seconds": at.unit.map(|u| u as f64 * frame_seconds),',
     '"seconds": at.unit.map(|u| u as f64),'),
    ('a complexity index that disagrees called a fault of a delivery',
     'crates/oadec-eac3/src/rules.rs',
     '                | Self::EmdfPayloadConfig\n                | Self::BsiComplexityIndex\n',
     '                | Self::EmdfPayloadConfig\n'),
    ('the threshold of verify ignored on a stream of the AC-3 family',
     'crates/oadec-cli/src/eac3.rs',
     '    } else {\n        p.diag.passes(fail_on)\n    }',
     '    } else {\n        is_clean(p, sync_errors, skipped)\n    }'),
    ('the listing limit not reaching the pass over a stream of the AC-3 family',
     'crates/oadec-cli/src/eac3.rs',
     'let diag = std::cell::RefCell::new(Diag::new(listed));',
     'let diag = std::cell::RefCell::new(Diag::default());'),
    ('a report of the AC-3 family naming another threshold than its verdict has',
     'crates/oadec-cli/src/eac3.rs',
     '        "fail_on": fail_on.name(),',
     '        "fail_on": Severity::Error.name(),'),
    ('the threshold of a call not reaching the report of an AC-3 family stream',
     'crates/oadec-cli/src/api.rs',
     '                fail_on: options.fail_on.unwrap_or(Severity::Error),\n                max_per_rule: options',
     '                fail_on: Severity::Error,\n                max_per_rule: options'),
    # --- AC-3 frames written whole (crates/oadec-synth) ----------------------
    # No fixture is an AC-3 stream, and no stream of a library applies a delta
    # bit allocation: until `oadec-synth` wrote frames that do, nothing decoded
    # one from its sync word to its samples.
    ('a delta code of 4 read as no change, where it is 6 dB up',
     'crates/oadec-eac3/src/bitalloc.rs',
     'let delta = if v >= 4 { (v - 3) << 7 } else { (v - 4) << 7 };',
     'let delta = if v > 4 { (v - 3) << 7 } else { (v - 4) << 7 };'),
    ('a delta of 3 dB a step, where it is 6',
     'crates/oadec-eac3/src/bitalloc.rs',
     'let delta = if v >= 4 { (v - 3) << 7 } else { (v - 4) << 7 };',
     'let delta = if v >= 4 { (v - 3) << 6 } else { (v - 4) << 6 };'),
    ('a delta segment placed from the first band, not from the end of the one before',
     'crates/oadec-eac3/src/bitalloc.rs',
     'band += usize::from(d.offsets[seg]);',
     'band = usize::from(d.offsets[seg]);'),
    ('a delta segment one band longer than its length',
     'crates/oadec-eac3/src/bitalloc.rs',
     'for _ in 0..d.lengths[seg] {',
     'for _ in 0..=d.lengths[seg] {'),
    ('the last band out of reach of a delta segment',
     'crates/oadec-eac3/src/bitalloc.rs',
     '                if band < 50 {\n                    mask[band] += delta;',
     '                if band < 49 {\n                    mask[band] += delta;'),
    # --- enhanced coupling from its lowest sub-band --------------------------
    # The coupling channel begins at band 13 then and the channels coupled
    # into it end there, and the excitation function does two things the
    # clause does not print. No stream of the corpus did this until an encoder
    # was found that does; the two blocks the tests hold are its.
    ('a coupling channel that begins low left without the low-frequency compensation, as the clause prints it',
     'crates/oadec-eac3/src/bitalloc.rs',
     '''    for bin in begin..bndend.min(22) {
        if ahead(bin) {''',
     '''    for bin in begin..bndend.min(22) {
        if ahead(bin) && leak.is_none() {'''),
    ('the last band of a channel looking at a band after it, unless the channel is the LFE',
     'crates/oadec-eac3/src/bitalloc.rs',
     'let ahead = |bin: usize| bin + 1 < bndend;',
     'let ahead = |bin: usize| bndend != 7 || bin != 6;'),
    ('the last band of the LFE channel looking at a band after it',
     'crates/oadec-eac3/src/bitalloc.rs',
     'let ahead = |bin: usize| bin + 1 < bndend;',
     'let ahead = |bin: usize| bndend != 13 || bin != 12;'),
    ('the excitation of band 21 made a second time, without its compensation',
     'crates/oadec-eac3/src/bitalloc.rs',
     'for bin in begin.max(22)..bndend {',
     'for bin in begin.max(21)..bndend {'),
    # --- transient pre-noise processing --------------------------------------
    # Where a correction is and how it fades was read off two Dolby decodes;
    # the tests hold both on a ramp, to the sample.
    ('a transient counted from the first sample of the frame as it is handed out',
     'crates/oadec-eac3/src/tpnp.rs',
     'loc: start + ORIGIN + t.loc as u64,',
     'loc: start + t.loc as u64,'),
    ('a pre-noise fade along a straight line',
     'crates/oadec-eac3/src/tpnp.rs',
     '(0.5 - 0.5 * phase.cos()) as f32',
     '((i as f64 + 0.5) / len as f64) as f32'),
    ('the fade out of a correction turned round',
     'crates/oadec-eac3/src/tpnp.rs',
     '*out * w + s * (1.0 - w)',
     '*out * (1.0 - w) + s * w'),
    ('the pre-noise counted from the block that holds the transient',
     'crates/oadec-eac3/src/tpnp.rs',
     'let Some(prior) = blk.checked_sub(1) else {',
     'let Some(prior) = blk.checked_sub(0) else {'),
    ('the substitution read one pre-noise back, where it is two',
     'crates/oadec-eac3/src/tpnp.rs',
     'p.loc.checked_sub((2 * TC1 + 2 * pnlen) as u64)',
     'p.loc.checked_sub((2 * TC1 + pnlen) as u64)'),
    ('the time scaling length left out of a correction',
     'crates/oadec-eac3/src/tpnp.rs',
     'let tot = pnlen + p.len + TC1;',
     'let tot = pnlen + TC1;'),
    ('the delta segments of every channel read into the first',
     'crates/oadec-eac3/src/frame.rs',
     'self.dba[ch] = self.read_dba()?;',
     'self.dba[0] = self.read_dba()?;'),
    ('the delta segments of the first channel applied to every channel',
     'crates/oadec-eac3/src/frame.rs',
     'then_some(&self.dba[slot])',
     'then_some(&self.dba[0])'),
    ('the number of delta segments read without the one it is short of',
     'crates/oadec-eac3/src/frame.rs',
     'let nseg = self.bits(3)? as usize + 1;',
     'let nseg = self.bits(3)? as usize;'),
    ('the length of a delta segment read in front of its offset',
     'crates/oadec-eac3/src/frame.rs',
     '            d.offsets.push(self.bits(5)? as u8);\n            d.lengths.push(self.bits(4)? as u8);',
     '            d.lengths.push(self.bits(4)? as u8);\n            d.offsets.push(self.bits(5)? as u8);'),
    ('the reserved delta code read as a reuse',
     'crates/oadec-eac3/src/frame.rs',
     '        0 => before,\n        _ => false,',
     '        0 | 3 => before,\n        _ => false,'),
    ('a reuse after a none applies the segments sent before the none',
     'crates/oadec-eac3/src/frame.rs',
     '        0 => before,\n        _ => false,',
     '        0 => true,\n        _ => false,'),
    ('the delta in force for a channel not kept from block to block',
     'crates/oadec-eac3/src/frame.rs',
     'self.dba_in_force[ch] = delta_in_force(self.deltbae[ch], self.dba_in_force[ch]);',
     'self.dba_in_force[ch] = delta_in_force(self.deltbae[ch], false);'),
    ('a block that says nothing of the delta drops the one in force',
     'crates/oadec-eac3/src/frame.rs',
     '                    self.dba[ch] = self.read_dba()?;\n                }\n            }\n        }\n',
     '                    self.dba[ch] = self.read_dba()?;\n                }\n            }\n        } else {\n            self.dba_in_force = [false; NCH];\n        }\n'),
    ('crc1 of an AC-3 frame taken over half of the frame, not five eighths',
     'crates/oadec-eac3/src/frame.rs',
     'let five_eighths = (words >> 1) + (words >> 3);',
     'let five_eighths = words >> 1;'),
    ('the exponent strategy of an AC-3 channel read as one bit',
     'crates/oadec-eac3/src/frame.rs',
     '\n                self.chexpstr[blk][ch] = self.bits(2)? as u8;\n',
     '\n                self.chexpstr[blk][ch] = self.bits(1)? as u8;\n'),
    ('the fine SNR offset of an AC-3 channel read behind its fast gain',
     'crates/oadec-eac3/src/frame.rs',
     '                for ch in 0..nf {\n                    self.fsnroffst[ch] = self.bits(4)? as u8;\n                    self.fgaincod[ch] = self.bits(3)? as u8;\n                }',
     '                for ch in 0..nf {\n                    self.fgaincod[ch] = self.bits(3)? as u8;\n                    self.fsnroffst[ch] = self.bits(4)? as u8;\n                }'),
    ('the slow gain and the dB per bit of an AC-3 block swapped',
     'crates/oadec-eac3/src/frame.rs',
     'self.alloc = AllocParams::from_codes(sd, fd, sg, db, fl, hth_index(self.h));',
     'self.alloc = AllocParams::from_codes(sd, fd, db, sg, fl, hth_index(self.h));'),
    ('the dither flag of a channel read upside down',
     'crates/oadec-eac3/src/frame.rs',
     'self.dithflag[ch] = self.bit()?;',
     'self.dithflag[ch] = !self.bit()?;'),
    ('the hearing threshold of 48 kHz at every rate',
     'crates/oadec-eac3/src/bitalloc.rs',
     'hth_index: hth_index.min(2),',
     'hth_index: 0,'),
    ('the size of an AC-3 frame read in the column of 48 kHz at every rate',
     'crates/oadec-eac3/src/header.rs',
     'let words = FRAME_SIZE_WORDS[usize::from(frmsizecod)][usize::from(fscod)];',
     'let words = FRAME_SIZE_WORDS[usize::from(frmsizecod)][0];'),
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
    # The arithmetic of the TrueHD decoder. The first of these passed every
    # test until a stream was written that has a filter in force at a restart
    # header, which no encoder writes (crates/oadec-synth, `filter-at-restart`).
    ('the filter histories kept across a restart header',
     'crates/oadec-truehd/src/decoder.rs',
     '''        self.fir_hist = [[0; FILTER_ORDER]; MAX_CHANNELS];
        self.iir_hist = [[0; FILTER_ORDER]; MAX_CHANNELS];
        self.m = ''',
     '''        self.m = '''),
    ('the state sent with filter B not loaded',
     'crates/oadec-truehd/src/decoder.rs',
     '                    sd.iir_hist[ch] = ss.iir[ch].state;',
     '                    let _ = ss.iir[ch].state;'),
    ('the shift of a filter state dropped',
     'crates/oadec-truehd/src/filter.rs',
     '                    reader.read_signed(state_bits)? << state_shift',
     '                    reader.read_signed(state_bits)?'),
    ('the shift of the filter coefficients dropped',
     'crates/oadec-truehd/src/filter.rs',
     'let coeff = reader.read_signed(u32::from(fc.coeff_bits))? << fc.coeff_shift;',
     'let coeff = reader.read_signed(u32::from(fc.coeff_bits))?;'),
    ('the prediction added without the cut to the quantiser step',
     'crates/oadec-truehd/src/decoder.rs',
     '            let out = residual + (pred & mask);',
     '            let out = residual + pred;'),
    ('filter B fed the output where it takes the prediction error',
     'crates/oadec-truehd/src/decoder.rs',
     '            let iir_in = out - pred;',
     '            let iir_in = out;'),
    ('a channel with filter B alone predicted at the precision of filter A',
     'crates/oadec-truehd/src/decoder.rs',
     '''        let shift = if fir_order != 0 {
            fir.coeff_q
        } else {
            iir.coeff_q
        };''',
     '''        let shift = fir.coeff_q;'''),
    ('the sign of a sample under book 3 with no bits under the code',
     'crates/oadec-truehd/src/block.rs',
     '            if shift < 0 { 0 } else { 1i64 << shift }',
     '            1i64 << shift.max(0)'),
    ('huff_lsbs after a restart header, 24 -> 23',
     'crates/oadec-truehd/src/state.rs',
     '            huff_lsbs: [24; MAX_CHANNELS],',
     '            huff_lsbs: [23; MAX_CHANNELS],'),
    ('the block size after a restart header, 8 -> 16',
     'crates/oadec-truehd/src/state.rs',
     '            block_size: 8,',
     '            block_size: 16,'),
    ('the offset of the sample coding taken off instead of added',
     'crates/oadec-truehd/src/block.rs',
     '            bias: i64::from(ss.huff_offset[ch]) - sign,',
     '            bias: -i64::from(ss.huff_offset[ch]) - sign,'),
    # What lies over the channels: the matrices of a 0x31EA substream with
    # their noise channels and bypassed bits, the output shift and the
    # channel assignment. No real stream gives a noise channel a coefficient,
    # so the first four pass on anything but a written one.
    ('the two noise channels of a matrix swapped',
     'crates/oadec-truehd/src/decoder.rs',
     '''                    row[mmc + 1] = a;
                    row[mmc + 2] = b;''',
     '''                    row[mmc + 1] = b;
                    row[mmc + 2] = a;'''),
    ('the shift of the noise channels dropped',
     'crates/oadec-truehd/src/dither.rs',
     '''    let first = i32::from((s >> 15) as i8) << shift;
    let second = i32::from(shr7 as i8) << shift;''',
     '''    let first = i32::from((s >> 15) as i8);
    let second = i32::from(shr7 as i8);
    let _ = shift;'''),
    ('the noise register moved on by another rule',
     'crates/oadec-truehd/src/dither.rs',
     '    *seed = (shr7 ^ (shr7 << 5) ^ (s << 16)) & SEED_MASK;',
     '    *seed = (shr7 ^ (shr7 << 4) ^ (s << 16)) & SEED_MASK;'),
    ('the noise register left running through a restart header',
     'crates/oadec-truehd/src/decoder.rs',
     '        self.dither_seed = ss.dither_seed;',
     '        self.dither_seed = if self.dither_seed == 0 { ss.dither_seed } else { self.dither_seed };'),
    ('a matrix that sends a bypassed bit cut to the quantiser step all the same',
     'crates/oadec-truehd/src/decoder.rs',
     '    let sum = if uncut {',
     '    let sum = if uncut && qss == 0 {'),
    ('the bypassed bit of a matrix left out',
     'crates/oadec-truehd/src/decoder.rs',
     '    sum.wrapping_add(bypassed_lsb)',
     '    sum.wrapping_add(bypassed_lsb & 0)'),
    ('the sum of a matrix not cut to the quantiser step',
     'crates/oadec-truehd/src/decoder.rs',
     '    let sum = if uncut {',
     '    let sum = if uncut || qss > 0 {'),
    ('an output shifted the other way',
     'crates/oadec-truehd/src/decoder.rs',
     '                let v = shift_left(row[chi], i32::from(shift));',
     '                let v = shift_left(row[chi], -i32::from(shift));'),
    ('the outputs left in the order of the coded channels',
     'crates/oadec-truehd/src/decoder.rs',
     '                out[usize::from(sd.ch_assign[chi])] = v;',
     '                out[chi] = v;'),
    ('the check word taken in the order of the outputs',
     'crates/oadec-truehd/src/decoder.rs',
     '                check ^= (v as u32 & 0xFF_FFFF) << (chi & 7);',
     '                check ^= (v as u32 & 0xFF_FFFF) << (usize::from(sd.ch_assign[chi]) & 7);'),
    # Fields no real stream sets: an output shift below nought, guard bits
    # other than all set, a protected block.
    ('an output shift below nought taken for none',
     'crates/oadec-truehd/src/decoder.rs',
     '''    } else {
        value >> -shift
    }''',
     '''    } else {
        value
    }'''),
    ('the guard bits a block sends not taken',
     'crates/oadec-truehd/src/block.rs',
     '            ss.guards = reader.read(8)? as u8;',
     '            let _ = reader.read(8)? as u8;'),
    ('the flag of filter A read though its guard bit is clear',
     'crates/oadec-truehd/src/block.rs',
     '        if ss.guards & GUARD_FIR != 0 && reader.read_bool()? {',
     '        if (ss.guards & GUARD_FIR != 0 || true) && reader.read_bool()? {'),
    ('the size a protected block states counted with its own field',
     'crates/oadec-truehd/src/block.rs',
     '            block.block_data_bits_ok = reader.position() - start == usize::from(expected);',
     '            block.block_data_bits_ok = reader.position() - start + 16 == usize::from(expected);'),
    ('the check byte of a protected block not read',
     'crates/oadec-truehd/src/block.rs',
     '            block.block_header_crc = Some(reader.read(8)? as u8);',
     '            block.block_header_crc = Some(0);'),
    # What a second and a third substream add: the channels a presentation
    # takes from the substreams under its own, the table of dither of a
    # 0x31EB substream, and the steps and shifts a substream states for
    # channels it does not code. Real streams have all of it, and the tests
    # that read them need media; the streams of several substreams of
    # crates/oadec-synth are in the repository.
    ('a presentation given the channels under its own substream as that substream last held them',
     'crates/oadec-truehd/src/decoder.rs',
     '                let src = &self.sub[j].recorrelated[n];',
     '                let src = &self.sub[i].recorrelated[n];'),
    ('a presentation without a substream of its own decoded from one the stream does not have',
     'crates/oadec-truehd/src/decoder.rs',
     '            PresentationKind::CopyOf(i) => i,',
     '            PresentationKind::CopyOf(_) => presentation,'),
    ('the eight-channel presentation read without its third substream',
     'crates/oadec-truehd/src/presentation.rs',
     '                (substream_info >> 4) & 7,',
     '                (substream_info >> 4) & 3,'),
    ('the dither of a matrix taken at the place of another matrix',
     'crates/oadec-truehd/src/decoder.rs',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output(''',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (pmi + 1) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output('''),
    ('the dither stepping through its table without the sample added',
     'crates/oadec-truehd/src/decoder.rs',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output(''',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1);
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output('''),
    ('the dither one bit lower than its scale says',
     'crates/oadec-truehd/src/decoder.rs',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output(''',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (10 + scale);
                        }
                        row[out_ch] = matrix_output('''),
    ('a dither scale of 1 taken for none',
     'crates/oadec-truehd/src/decoder.rs',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale != 0 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output(''',
     '''                        let scale = ss.dither_scale[pmi];
                        if scale > 1 {
                            let index = (matrices - pmi) * (2 * n + 1) + n;
                            acc += i64::from(sd.dither_table[index & table_mask]) << (11 + scale);
                        }
                        row[out_ch] = matrix_output('''),
    ('the dither of a unit 64 values at every rate',
     'crates/oadec-truehd/src/decoder.rs',
     '        let table_len = spa.next_power_of_two();',
     '        let table_len = 64;'),
    ('the dither register not moved on from one unit to the next',
     'crates/oadec-truehd/src/decoder.rs',
     '                fill_table_bc(&mut sd.dither_seed, &mut sd.dither_table[..table_len]);',
     '                fill_table_bc(&mut sd.dither_seed.clone(), &mut sd.dither_table[..table_len]);'),
    ('the dither register moved on by another rule',
     'crates/oadec-truehd/src/dither.rs',
     '        *seed = ((*seed << 8) ^ shr15 ^ (shr15 << 5)) & SEED_MASK;',
     '        *seed = ((*seed << 8) ^ shr15 ^ (shr15 << 4)) & SEED_MASK;'),
    ('the dither table read one entry off',
     'crates/oadec-truehd/src/dither.rs',
     '        *entry = i32::from(NOISE_TABLE[shr15 as usize]);',
     '        *entry = i32::from(NOISE_TABLE[(shr15 as usize) ^ 1]);'),
    ('the dither scale of a matrix read and dropped',
     'crates/oadec-truehd/src/matrix.rs',
     '''                    if ss.sync_word == SYNC_B {
                        ss.dither_scale[pmi] = reader.read(4)? as u8;''',
     '''                    if ss.sync_word == SYNC_B {
                        ss.dither_scale[pmi] = (reader.read(4)? as u8) & 0;'''),
    ('a matrix of a dither substream that sends a bypassed bit cut to the quantiser step all the same',
     'crates/oadec-truehd/src/decoder.rs',
     '''                            lsb[pmi],
                            ss.lsb_bypass_used[pmi],
                        );
                    }
                }
                _ => {''',
     '''                            lsb[pmi],
                            false,
                        );
                    }
                }
                _ => {'''),
    ('a matrix of a dither substream cut to no quantiser step',
     'crates/oadec-truehd/src/decoder.rs',
     '''                        row[out_ch] = matrix_output(
                            acc,
                            ss.quant_step_size[out_ch],
                            lsb[pmi],
                            ss.lsb_bypass_used[pmi],
                        );
                    }
                }
                _ => {''',
     '''                        row[out_ch] = matrix_output(
                            acc,
                            0,
                            lsb[pmi],
                            ss.lsb_bypass_used[pmi],
                        );
                    }
                }
                _ => {'''),
    ('the quantiser steps of a block read for the channels its substream codes alone',
     'crates/oadec-truehd/src/block.rs',
     '''            for ch in 0..=ss.max_chan {
                ss.quant_step_size[ch] = reader.read(4)?;''',
     '''            for ch in ss.min_chan..=ss.max_chan {
                ss.quant_step_size[ch] = reader.read(4)?;'''),
    # --- an option that does not apply is refused -----------------------------
    # Each of these was taken and changed nothing: a ledger asked for and never
    # written, a frame rate in no file, a measurement "without dither" of a
    # stream that has none. Every refusal is a condition someone could lose.
    ('the options of an object output let through with a sample output',
     'crates/oadec-cli/src/lib.rs',
     '} else if given_for_objects && !matches!(format, Format::Damf | Format::Adm) {',
     '} else if false && given_for_objects && !matches!(format, Format::Damf | Format::Adm) {'),
    ('--isf with a sample output not counted among the options of an object output',
     'crates/oadec-cli/src/lib.rs',
     '                || isf.is_some()\n',
     ''),
    ('--fps with a sample output told it is about a DAMF set',
     'crates/oadec-cli/src/lib.rs',
     '                || fps.is_some()\n',
     ''),
    ('a loss report asked for with a sample output, and never written',
     'crates/oadec-cli/src/lib.rs',
     '                || loss_report.is_some()\n',
     ''),
    ('the options of an ADM file with a sample output told they are about an ADM file',
     'crates/oadec-cli/src/lib.rs',
     '''                || fps.is_some()
                || dolby_origin_tag
                || adm_allow_non_profile_rate
                || adm_interpolation.is_some();''',
     '''                || fps.is_some();'''),
    ('the options of an ADM file let through with a DAMF set',
     'crates/oadec-cli/src/lib.rs',
     '} else if given_for_adm && format != Format::Adm {',
     '} else if false && given_for_adm && format != Format::Adm {'),
    ('an interpolation given at its default not counted as given',
     'crates/oadec-cli/src/lib.rs',
     'dolby_origin_tag || adm_allow_non_profile_rate || adm_interpolation.is_some();',
     'dolby_origin_tag || adm_allow_non_profile_rate;'),
    ('a frame rate let through with an ADM file',
     'crates/oadec-cli/src/lib.rs',
     '} else if given_for_damf && format != Format::Damf {',
     '} else if false && given_for_damf && format != Format::Damf {'),
    ('a channel order let through with an object output',
     'crates/oadec-cli/src/lib.rs',
     '} else if given_for_samples && matches!(format, Format::Damf | Format::Adm) {',
     '} else if false && given_for_samples && matches!(format, Format::Damf | Format::Adm) {'),
    ('the options of the AC-3 family decoder let through on a TrueHD decode',
     'crates/oadec-cli/src/lib.rs',
     '''                } else if (core_only || no_dither || no_tpnp || ecpl_spec)
                    && !eac3::is_eac3(&source).unwrap_or(false)
''',
     '''                } else if false
'''),
    ('--ecpl-spec let through on a TrueHD decode',
     'crates/oadec-cli/src/lib.rs',
     '} else if (core_only || no_dither || no_tpnp || ecpl_spec)\n                    && !eac3',
     '} else if (core_only || no_dither || no_tpnp)\n                    && !eac3'),
    ('--core-only let through on a TrueHD decode',
     'crates/oadec-cli/src/lib.rs',
     '} else if (core_only || no_dither || no_tpnp || ecpl_spec)\n                    && !eac3',
     '} else if (no_dither || no_tpnp || ecpl_spec)\n                    && !eac3'),
    ('the readings of the JOC matrices let through where there is no JOC decode',
     'crates/oadec-cli/src/lib.rs',
     '''                } else if (no_clip_gain || flat_quadrature || sparse_as_printed || steep_as_printed)
                    && !(eac3::is_eac3(&source).unwrap_or(false)
                        && matches!(format, Format::Damf | Format::Adm))
''',
     '''                } else if false
'''),
    ('--steep-as-printed let through where there is no JOC decode',
     'crates/oadec-cli/src/lib.rs',
     '} else if (no_clip_gain || flat_quadrature || sparse_as_printed || steep_as_printed)',
     '} else if (no_clip_gain || flat_quadrature || sparse_as_printed)'),
    ('a reading of the JOC matrices let through for the samples of an E-AC-3 stream',
     'crates/oadec-cli/src/lib.rs',
     '''sparse_as_printed || steep_as_printed)
                    && !(eac3::is_eac3(&source).unwrap_or(false)
                        && matches!(format, Format::Damf | Format::Adm))''',
     '''sparse_as_printed || steep_as_printed)
                    && !eac3::is_eac3(&source).unwrap_or(false)'''),
    ('kept duplicates let through on a decode of the AC-3 family',
     'crates/oadec-cli/src/lib.rs',
     '} else if keep_duplicates && eac3::is_eac3(&source).unwrap_or(false) {',
     '} else if false && keep_duplicates && eac3::is_eac3(&source).unwrap_or(false) {'),
    ('a presentation let through on a sample decode of the AC-3 family',
     'crates/oadec-cli/src/lib.rs',
     '''                } else if presentation.is_some()
                    && eac3::is_eac3(&source).unwrap_or(false)
                    && !matches!(format, Format::Damf | Format::Adm)
''',
     '''                } else if false
'''),
    ('the object programme of an E-AC-3 stream refused when it is named',
     'crates/oadec-cli/src/lib.rs',
     '''                } else if presentation.is_some()
                    && eac3::is_eac3(&source).unwrap_or(false)
                    && !matches!(format, Format::Damf | Format::Adm)
''',
     '''                } else if presentation.is_some()
                    && eac3::is_eac3(&source).unwrap_or(false)
'''),
    ('a presentation let through on a comparison of the AC-3 family',
     'crates/oadec-cli/src/lib.rs',
     'if ac3 && presentation.is_some() {',
     'if false && ac3 && presentation.is_some() {'),
    ('the format of a reference refused for a TrueHD comparison and taken for the other',
     'crates/oadec-cli/src/lib.rs',
     '} else if ac3 && reference_format.is_some() {',
     '} else if !ac3 && reference_format.is_some() {'),
    ('kept duplicates let through on a comparison of the AC-3 family',
     'crates/oadec-cli/src/lib.rs',
     '} else if ac3 && keep_duplicates {',
     '} else if false && ac3 && keep_duplicates {'),
    ('the options of the AC-3 family decoder let through on a TrueHD comparison',
     'crates/oadec-cli/src/lib.rs',
     '} else if !ac3 && (core_only || no_dither || no_tpnp || ecpl_spec) {',
     '} else if false && (core_only || no_dither || no_tpnp || ecpl_spec) {'),
    ('a short list of the worst blocks let through on a TrueHD comparison',
     'crates/oadec-cli/src/lib.rs',
     '} else if !ac3 && worst > 0 {',
     '} else if !ac3 && worst > 3 {'),
    ('a TrueHD comparison told no presentation comparing the 5.1 one',
     'crates/oadec-cli/src/lib.rs',
     'presentation: presentation.unwrap_or(2),',
     'presentation: presentation.unwrap_or(1),'),
    ('a TrueHD comparison told no reference format reading 24-bit samples',
     'crates/oadec-cli/src/lib.rs',
     'format: reference_format.unwrap_or(RefFormat::S32le),',
     'format: reference_format.unwrap_or(RefFormat::S24le),'),
    ('a presentation let through by a call for a decode of the AC-3 family',
     'crates/oadec-cli/src/api.rs',
     '        if options.presentation.is_some() {\n            bail!(PRESENTATION_IS_TRUEHD);\n        }\n',
     ''),
    ('a call telling a presentation what kept duplicates are told',
     'crates/oadec-cli/src/api.rs',
     '            bail!(PRESENTATION_IS_TRUEHD);',
     '            bail!(KEEP_DUPLICATES_IS_TRUEHD);'),
    ('kept duplicates let through by a call for a sample decode of the AC-3 family',
     'crates/oadec-cli/src/api.rs',
     '        if options.keep_duplicates {\n            bail!(KEEP_DUPLICATES_IS_TRUEHD);\n        }\n',
     ''),
    ('the pre-noise switch let through by a call for a TrueHD decode',
     'crates/oadec-cli/src/api.rs',
     'if options.core_only || !options.dither || !options.tpnp || options.ecpl_full {',
     'if options.core_only || !options.dither || options.ecpl_full {'),
    ('the options of the AC-3 family decoder let through by a call for a TrueHD decode',
     'crates/oadec-cli/src/api.rs',
     '        bail!(CORE_DECODE_IS_AC3);\n    }\n    Ok(Pcm::TrueHd(',
     '        let _ = CORE_DECODE_IS_AC3;\n    }\n    Ok(Pcm::TrueHd('),
    ('the options of an ADM file refused by a call for an ADM file and taken for a DAMF set',
     'crates/oadec-cli/src/api.rs',
     '    if !options.adm && adm_only {',
     '    if options.adm && adm_only {'),
    ('a frame rate refused by a call for a DAMF set and taken for an ADM file',
     'crates/oadec-cli/src/api.rs',
     '    if options.adm && options.fps != "24" {',
     '    if !options.adm && options.fps != "24" {'),
    ('the interpolation not counted by a call among the options of an ADM file',
     'crates/oadec-cli/src/api.rs',
     '        || options.allow_non_profile_rate\n        || options.interpolation != Interpolation::Profile;',
     '        || options.allow_non_profile_rate;'),
    ('kept duplicates let through by a call for an object decode of the AC-3 family',
     'crates/oadec-cli/src/api.rs',
     '    if ac3 && options.keep_duplicates {\n        bail!(KEEP_DUPLICATES_IS_TRUEHD);\n    }\n',
     ''),
    ('the dither switch let through by a call for a TrueHD object decode',
     'crates/oadec-cli/src/api.rs',
     '    if !ac3 && (!options.core.dither || !options.core.tpnp || options.core.ecpl_full) {',
     '    if !ac3 && (!options.core.tpnp || options.core.ecpl_full) {'),
    ('the clip gain reading let through by a call for a TrueHD object decode',
     'crates/oadec-cli/src/api.rs',
     '        && (!options.clip_gain\n            || options.flat_quadrature',
     '        && (options.flat_quadrature'),
    ('the readings of the JOC matrices let through by a call for a TrueHD object decode',
     'crates/oadec-cli/src/api.rs',
     '        bail!(JOC_READINGS_ARE_EAC3_OBJECTS);',
     '        let _ = JOC_READINGS_ARE_EAC3_OBJECTS;'),
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
