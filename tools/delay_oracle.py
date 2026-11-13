#!/usr/bin/env python3
"""The delay through the decoder buffer of every access unit, from the trace
`verify --trace-fifo` writes, and how often it changes from one unit to the
next: what a stream that says its delay is constant is held to.

    oadec verify --trace-fifo trace.csv stream.thd
    python tools/delay_oracle.py trace.csv [--samples-per-unit 40]

The delay is the output time less the input time (docs/truehd.md, *Decoder
buffer*). The output time is the `output_timing` of a restart header where a
unit has one, and one frame after the unit before where it has none; both
clocks are sixteen bits wide and are unwrapped here. This is the arithmetic
of the document written a second time, in another language and from the
timing fields alone: it is what `buffer.latency_not_constant` is held
against, not the model of the decoder asked again.

The trace has no column for `flags`, so which major syncs say the delay is
constant is not known here. The two counts cover the two streams a test
makes: `changes` is what a stream that says so in every major sync has to
answer for, and `changes_before_the_second_major_sync` what one that says so
in its first major sync alone has to.
"""

import csv
import json
import sys


def delays(rows, spa):
    """The delay of each unit in samples, the clocks unwrapped."""
    out = []
    base = 0
    last_input = None
    output = None
    for row in rows:
        coded = int(row['input_timing'])
        if last_input is None:
            t = coded
        else:
            t = base + coded
            if t + 32768 < last_input:
                base += 65536
                t += 65536
            assert t >= last_input, ('the input clock steps back at unit', row['unit'])
        last_input = t
        stated = next((int(row[k]) for k in ('out0', 'out1', 'out2', 'out3') if row[k] != ''), None)
        if stated is not None:
            o = t - t % 65536 + stated
            while o + 32768 < t:
                o += 65536
            while o > t + 32768:
                o -= 65536
            output = o
        else:
            assert output is not None, 'a unit before the first restart header'
            output += spa
        out.append(output - t)
    return out


def main():
    if len(sys.argv) < 2:
        print(__doc__, file=sys.stderr)
        return 2
    spa = int(sys.argv[sys.argv.index('--samples-per-unit') + 1]) if '--samples-per-unit' in sys.argv else 40
    with open(sys.argv[1], encoding='utf-8', newline='') as f:
        rows = list(csv.DictReader(f))
    waits = delays(rows, spa)
    majors = [i for i, row in enumerate(rows) if row['major_sync'] != '0']
    changes = [(i, waits[i - 1], waits[i]) for i in range(1, len(waits)) if waits[i] != waits[i - 1]]
    second = majors[1] if len(majors) > 1 else len(rows)
    under_first = [c for c in changes if c[0] < second]
    print(json.dumps({
        'units': len(rows),
        'major_syncs': len(majors),
        'second_major_sync_at_unit': second,
        'delay_samples': {'min': min(waits), 'max': max(waits)},
        'changes': len(changes),
        'first_change': changes[0] if changes else None,
        'last_change': changes[-1] if changes else None,
        'changes_before_the_second_major_sync': len(under_first),
        'last_change_before_the_second_major_sync': under_first[-1] if under_first else None,
    }))
    return 0


if __name__ == '__main__':
    sys.exit(main())
