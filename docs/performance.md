# What decoding costs

How fast the tool reads and decodes, how much processor time that takes and
how much memory it holds, measured and not estimated. The figures here are a
baseline: what the code did before anything was done to make it faster, on one
machine, so that a later change has something to be compared with.

## How it is measured

`tools/bench.py` runs every command as a process of its own and reads three
things about it from the operating system once it has ended: the time between
its start and its end, the processor time it used in user and kernel mode, and
the most memory it held at once. The decoder cannot report these about itself
without code this repository forbids, and a figure a program prints about
itself is not a measurement of it.

```sh
python tools/bench.py --refs --out bench.json  WORK/thd WORK/ec3 WORK/clips
python tools/bench.py --report bench.json
python tools/bench.py --compare old/oadec new/oadec --out compare.json  WORK/thd WORK/ec3
```

- The streams are sorted into kinds by what `info --json` says of them, a
  TrueHD stream also by the channels of its widest presentation, and the
  longest and heaviest two of each kind are measured. A report names a stream
  by its kind, its length and its size, not by its file name.
- Each stream is read once before it is measured, so no run pays for the disk.
  PCM goes to the null device. The object formats seek back to finish their
  headers and are written to a file, so their figures include the writing.
- Every command is run five times and the median is reported, with the spread
  of the runs (the median absolute deviation as a share of the median).
- "× real time" is the length of the audio divided by the wall time.
  "Processor seconds per minute" is user and kernel time together for a minute
  of audio; it is more than the wall time where a command uses two threads,
  and `decode` does: it checks the stream on a second thread while it decodes
  (`exit-codes.md`).
- `--refs` runs FFmpeg and `truehdd` on the same streams with the same
  measurement. They are there for scale and do not do the same work: neither
  checks the stream beside the decode, FFmpeg stops at eight channels, and
  FFmpeg's transforms are hand-written for the processor's vector units,
  which this code, written without `unsafe`, leaves to the compiler.
- `--compare OLD NEW` runs two binaries turn and turn about and calls a
  command faster or slower only when the ratio of the medians is outside three
  times the spread, or two per cent, whichever is more.

`cargo bench -p oadec-bench` is the other half: the decoder's kernels over the
committed fixtures, inside one process, with no file read and nothing written.
It needs no media and is for comparing two builds on one machine.

## The baseline

Measured on 11 November 2026: an Intel Core i7-14700K (28 logical
processors) under Windows 11, release build of `main` as it stood after the
extractor fix, with Rust 1.99.1, the machine otherwise idle. Eighteen streams: heads of about a minute of library
titles, 48 kHz. Every row of the run, command by command, is in
`docs/audit/evidence/engine-round-2026-11/08-bench-baseline.json`, and
`python tools/bench.py --report` on that file prints it as a table.

### By kind of work

The range over the streams of a kind; the reference decoders in brackets.

| | work | streams | × real time | processor s per minute | peak MiB | widest spread |
|---|---|---:|---:|---:|---:|---:|
| TrueHD | `verify` | 10 | 141 to 542 | 0.11 to 0.41 | 18 | 0.6 % |
| | `verify --decode`, every presentation | 10 | 30 to 131 | 0.45 to 1.98 | 19 | 0.8 % |
| | `decode`, 2 channels | 10 | 141 to 526 | 0.22 to 0.59 | 30 | 1.2 % |
| | `decode`, 6 channels | 12 | 120 to 237 | 0.36 to 0.91 | 30 | 2.6 % |
| | `decode`, 8 channels | 8 | 103 to 163 | 0.53 to 0.97 | 30 | 1.3 % |
| | `decode`, 12 channels | 2 | 88 to 91 | 0.87 to 0.91 | 30 | 0.4 % |
| | `decode`, 14 channels | 2 | 83 to 88 | 0.94 to 0.98 | 30 | 0.5 % |
| | `decode`, 16 channels | 2 | 63 to 73 | 1.09 to 1.20 | 30 | 0.7 % |
| | `decode --format damf`, 12, 14 and 16 channels | 6 | 57 to 79 | 0.96 to 1.31 | 31 | 7.7 % |
| | `decode --format adm`, 12, 14 and 16 channels | 6 | 39 to 79 | 0.96 to 1.28 | 30 | 35.0 % |
| | [FFmpeg, 6 or 8 channels] | 10 | 69 to 127 | 1.22 to 1.94 | 24 | 1.0 % |
| | [truehdd, 2 channels] | 10 | 133 to 173 | 1.24 to 1.66 | 9 | 1.9 % |
| | [truehdd, 6 channels] | 12 | 66 to 94 | 2.36 to 3.25 | 10 | 1.4 % |
| | [truehdd, 8 channels] | 8 | 58 to 95 | 2.33 to 3.62 | 11 | 1.2 % |
| | [truehdd, 12 and 14 channels] | 4 | 41 to 62 | 3.38 to 4.69 | 12 | 3.8 % |
| | [truehdd, 16 channels] | 2 | 37 to 39 | 4.91 to 4.95 | 12 | 3.6 % |
| AC-3 | `verify` | 2 | 181 to 314 | 0.19 to 0.33 | 13 | 0.5 % |
| | `decode`, 2 channels | 1 | 310 | 0.19 | 17 | 0.7 % |
| | `decode`, 6 channels | 1 | 176 | 0.34 | 17 | 0.9 % |
| | [FFmpeg] | 2 | 860 to 1584 | 0.08 to 0.11 | 24 | 2.4 % |
| E-AC-3 | `verify`, 5.1 | 2 | 178 to 237 | 0.24 to 0.34 | 14 | 0.6 % |
| | `decode`, 5.1 | 2 | 172 to 225 | 0.26 to 0.35 | 18 | 0.8 % |
| | [FFmpeg, 5.1] | 2 | 928 to 1061 | 0.09 to 0.10 | 24 | 2.2 % |
| | `verify`, 7.1 | 2 | 85 to 88 | 0.66 to 0.70 | 14 | 0.3 % |
| | `decode`, 7.1 | 2 | 83 to 86 | 0.69 to 0.72 | 18 | 0.4 % |
| | [FFmpeg, 7.1] | 2 | 540 to 562 | 0.12 to 0.16 | 24 | 0.4 % |
| E-AC-3 with JOC | `verify` | 2 | 85 to 173 | 0.35 to 0.69 | 14 | 0.5 % |
| | `decode`, the channels | 2 | 82 to 169 | 0.35 to 0.72 | 19 | 0.6 % |
| | `decode --format damf`, the objects | 2 | 26 to 40 | 1.48 to 2.25 | 22 | 0.4 % |
| | `decode --format adm`, the objects | 2 | 27 to 42 | 1.42 to 2.20 | 22 | 0.7 % |
| | [FFmpeg, the channels] | 2 | 554 to 974 | 0.10 to 0.16 | 24 | 0.5 % |

### The kernels

`cargo bench -p oadec-bench` on the same machine and build:

```text
truehd: frame the access units                          174.044 us  +/-  3.8 %
truehd: decode presentation 0, 2 channels                 3.007 ms  +/-  0.4 %  665x real time
truehd: decode presentation 1, 6 channels                 7.909 ms  +/-  0.3 %  253x real time
truehd: decode presentation 2, 8 channels                11.259 ms  +/-  1.7 %  178x real time
truehd: decode presentation 3, 12 channels               32.913 ms  +/-  0.9 %  61x real time
truehd: decode 88.2 kHz, 6 channels                     960.955 us  +/-  1.9 %  105x real time
mlp: decode 96 kHz, 6 channels                          921.143 us  +/-  0.8 %  109x real time
eac3: decode the 5.1 core of the object stream            9.630 ms  +/-  0.7 %  209x real time
joc: analysis bank, one channel, one frame               37.076 us  +/-  0.1 %  863x real time
joc: synthesis bank, one object, one frame               39.016 us  +/-  0.5 %  820x real time
```

## What the baseline says

**TrueHD is not where the time is.** A 16-channel object presentation
decodes at 63 to 73 times real time for a little over a second of processor
time a minute of audio, and the two-channel presentation at 141 to 526 times.
Memory is flat at 30 MiB whatever the width. On the same streams and
presentations `truehdd` takes between 1.06 and 3.1 times as long and 2.8 to
6.7 times the processor time, and FFmpeg, on the widest presentation it
decodes, 1.4 to 1.9 times as long. They are there for scale, and the scale
says there is no easy factor left here.

**The AC-3 family is.** The core decoder runs at 172 to 225 times real time
on 5.1 and 83 to 86 on 7.1, and FFmpeg decodes the same frames between 4.7
and 6.8 times faster. `verify` of this family costs what a decode costs,
because it decodes: 0.19 to 0.70 processor seconds a minute against 0.11 to
0.41 for TrueHD.

**Objects out of JOC cost three to four times the channels under them.** 26 to 42
times real time for the object formats against 82 to 169 for the core. The
kernels say why: an analysis bank for every coded channel and a synthesis
bank for every object, 37 and 39 microseconds each for a frame of 32
milliseconds. Sixteen objects over a 5.1 core is 0.85 ms of filter bank a
frame, which alone holds the whole to about 38 times real time.

**`verify --decode` is the sum of its presentations**, 30 to 131 times real
time, because it decodes them one after another.

**The object formats are written to a disk, and it shows.** The two widest
spreads in the table, 35 and 8 per cent, are object files of TrueHD streams;
every figure that is the decoder's alone repeats to within three per cent.

So the order of work for speed is the AC-3 core first, then the filter banks
of the object path, then the presentations of `verify --decode` side by side.
Each of those is a change that has to show its figure with `--compare`
against this one, and leave the output where it was.

## What it does not say

- One machine. The ratios between commands carry to another; the figures do
  not.
- Heads of about a minute. A command that takes a quarter of a second is
  measured to a few per cent at best, and the processor time of so short a
  run is counted by a clock that steps by a sixty-fourth of a second.
- The streams are the heaviest of each kind there were, not a sample of what
  is common.
- Nothing here is a claim about another decoder. The reference rows are the
  same measurement on the same streams, of programs that do less.
