#!/usr/bin/env python3
"""Break the decoder on purpose and see whether anything fails.

A green suite says nothing about what it would have caught. This changes one
load-bearing constant at a time, runs the unit tests, and reports which
mutations survive. A survivor is a hole in the suite, named -- and sometimes
the right answer, when the constant has no specification behind it and only a
comparison against another decoder can judge it.

    python tools/mutants.py
    python tools/mutants.py --out mutants.json

The tree must be clean: every mutation is undone with `git checkout --`, and
the run refuses to start otherwise. Each entry is a bug someone could write,
not a random character: a table entry off by one, an index base, a scale
factor, a channel mapped to the wrong input.
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

os.chdir(Path(__file__).resolve().parent.parent)

MUTANTS = [
    # (a name, the file, the text to find, what to put instead)
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
]


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=1800)


def clean():
    return not run(['git', 'status', '--porcelain']).stdout.strip()


if not clean():
    print('the tree is not clean; refusing to mutate it')
    sys.exit(2)

base = run(['cargo', 'test', '--workspace', '--locked'])
if base.returncode != 0:
    print('the suite does not pass before any mutation')
    sys.exit(2)
print('baseline green\n')

rows = []
for name, path, old, new in MUTANTS:
    src = open(path, encoding='utf-8').read()
    if old not in src:
        print(f'  SKIP  {name}: the text is not in {path}')
        rows.append({'mutation': name, 'applied': False})
        continue
    open(path, 'w', encoding='utf-8', newline='').write(src.replace(old, new, 1))
    r = run(['cargo', 'test', '--workspace', '--locked'])
    caught = r.returncode != 0
    failing = sorted({ln.split()[1] for ln in r.stdout.splitlines()
                      if ln.startswith('test ') and ln.rstrip().endswith('FAILED')})
    subprocess.run(['git', 'checkout', '--', path], check=True)
    rows.append({'mutation': name, 'applied': True, 'caught': caught,
                 'failing_tests': failing[:6], 'failing_count': len(failing)})
    mark = 'caught' if caught else 'SURVIVED'
    print(f'  {mark:9s} {name}'
          f'{"  by " + ", ".join(failing[:3]) if failing else ""}'
          f'{f" (+{len(failing) - 3} more)" if len(failing) > 3 else ""}', flush=True)

_ap = argparse.ArgumentParser()
_ap.add_argument('--out')
_args = _ap.parse_args()
if _args.out:
    json.dump(rows, open(_args.out, 'w'), indent=1)
survived = [r for r in rows if r.get('applied') and not r['caught']]
print(f'\n{len(rows)} mutations, {len(survived)} survived')
for r in survived:
    print('  survived:', r['mutation'])
assert clean(), 'the tree was left dirty'
print('tree clean')
