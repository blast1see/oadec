#!/usr/bin/env python3
"""What the compiler made of a procedure, read from the linked binary.

    python tools/packed.py target/release/oadec.exe oadec_eac3::imdct
    python tools/packed.py target/release/oadec.exe --out counts.json stages step

For every procedure whose name holds one of the strings given: its size, and
how many of its floating-point instructions take one number at a time
(`mulsd`), how many take two (`mulpd`), and how many only move numbers
between the halves of a register (`unpcklpd`). The last are what a packed
loop pays for values that do not lie the way it takes them: a complex value
that is its real part and then its imaginary part has to be taken apart
before two of them are multiplied at once, and put back after.

The binary has to be the linked one. This workspace is built with link-time
optimisation, and the assembly a crate is compiled to by itself
(`cargo rustc -p <crate> --lib -- --emit asm`) is the code before the linker
has optimised it: its loops are scalar where the loops of the binary are
packed. `docs/performance.md` said for a day that the inverse transform was
compiled to scalar code, having read that assembly.

A count is of the instructions that are there and not of how often each
runs: a loop that takes two numbers at a time has a scalar loop beside it
for what is left over, and both are counted once.

It reads a Windows executable and the debug file beside it (`--pdb` for one
that is elsewhere), with `llvm-pdbutil` and `llvm-objdump` on the path.

Exit code: 0, 1 when no procedure matched or a tool is missing or failed, 2
on a usage error.
"""
import argparse
import collections
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ARITHMETIC = ('mul', 'add', 'sub', 'div')
SCALAR = tuple(a + s for a in ARITHMETIC for s in ('sd', 'ss'))
PACKED = tuple(a + s for a in ARITHMETIC for s in ('pd', 'ps'))
# instructions that only move numbers between the halves of a register
MOVES = ('unpcklpd', 'unpckhpd', 'shufpd', 'unpcklps', 'unpckhps', 'shufps', 'movhlps', 'movlhps')


def tool(name):
    path = shutil.which(name)
    if path is None:
        sys.exit(f'{name} is not on the path: it comes with an installation of LLVM')
    return path


def run(command):
    done = subprocess.run(command, capture_output=True, text=True, errors='replace')
    if done.returncode != 0:
        sys.stderr.write(done.stderr)
        sys.exit(f'{Path(command[0]).name} failed with {done.returncode}')
    return done.stdout


def procedures(pdb):
    """The procedures of a debug file: name, section, offset and bytes."""
    lines = run([tool('llvm-pdbutil'), 'dump', '--symbols', str(pdb)]).splitlines()
    found = []
    for i, line in enumerate(lines[:-1]):
        name = re.search(r'S_[GL]PROC32 \[size = \d+\] `(.+)`', line)
        where = re.search(r'addr = (\d+):(\d+), code size = (\d+)', lines[i + 1])
        if name and where:
            found.append((name.group(1), int(where.group(1)), int(where.group(2)), int(where.group(3))))
    return found


def sections(exe):
    """Where each section of an executable is loaded, by its number."""
    loaded = {}
    for line in run([tool('llvm-objdump'), '-h', str(exe)]).splitlines():
        row = re.match(r'\s*(\d+)\s+\S+\s+[0-9a-f]+\s+([0-9a-f]+)', line)
        if row:
            loaded[int(row.group(1)) + 1] = int(row.group(2), 16)
    return loaded


def count(exe, start, size):
    """The floating-point instructions between two addresses, by mnemonic."""
    text = run([tool('llvm-objdump'), '-d', '--no-show-raw-insn', f'--start-address={start:#x}',
                f'--stop-address={start + size:#x}', str(exe)])
    mnemonics = collections.Counter(re.findall(r'^\s*[0-9a-f]+:\s+v?([a-z0-9]+)', text, re.M))
    kept = {m: n for m, n in mnemonics.items() if m in SCALAR + PACKED + MOVES}
    return {
        'scalar': sum(n for m, n in kept.items() if m in SCALAR),
        'packed': sum(n for m, n in kept.items() if m in PACKED),
        'moves_between_halves': sum(n for m, n in kept.items() if m in MOVES),
        'instructions': dict(sorted(kept.items())),
    }


def main():
    parser = argparse.ArgumentParser(
        description='What the compiler made of a procedure, read from the linked binary.')
    parser.add_argument('exe', type=Path, help='a linked executable of this workspace')
    parser.add_argument('names', nargs='+', help='strings a procedure name has to hold one of')
    parser.add_argument('--pdb', type=Path, help='its debug file, where it is not beside it')
    parser.add_argument('--out', type=Path, help='write the counts here as JSON')
    args = parser.parse_args()
    pdb = args.pdb or args.exe.with_suffix('.pdb')
    for path in (args.exe, pdb):
        if not path.is_file():
            sys.exit(f'{path}: no such file')
    loaded = sections(args.exe)
    rows = []
    for name, section, offset, size in sorted(procedures(pdb)):
        if size and any(wanted in name for wanted in args.names):
            rows.append({'procedure': name, 'bytes': size, **count(args.exe, loaded[section] + offset, size)})
    if not rows:
        sys.exit('no procedure has such a name')
    print(f"{'bytes':>7} {'scalar':>7} {'packed':>7} {'moves':>6}  procedure")
    for row in rows:
        print(f"{row['bytes']:7d} {row['scalar']:7d} {row['packed']:7d} {row['moves_between_halves']:6d}  "
              f"{row['procedure']}")
    if args.out:
        args.out.write_text(json.dumps({'schema': 1, 'procedures': rows}, indent=1) + '\n', encoding='utf-8',
                            newline='\n')
    return 0


if __name__ == '__main__':
    sys.exit(main())
