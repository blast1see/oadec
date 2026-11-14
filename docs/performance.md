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
It needs no media and is for comparing two builds on one machine. A kernel is
timed on whatever core its thread is on, so on a processor with cores of two
speeds the benchmark is run through `python tools/kernels.py --affinity
MASK`, which binds it to the processors of a mask and runs it several times.
The first table of kernels this document had was not, and was wrong for it
(see the kernels below).

What the compiler made of a loop is read from the linked binary:
`python tools/packed.py target/release/oadec.exe NAME` counts, for every
procedure whose name holds `NAME`, the floating-point instructions that take
one number at a time, those that take two, and those that only move numbers
between the halves of a register. The workspace is built with link-time
optimisation, so the assembly a crate is compiled to by itself is the code
before the linker has optimised it. This document read that assembly once,
and was wrong for it (see the inverse transform of the AC-3 family below).

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

`python tools/kernels.py --affinity FFFF --runs 5` on the same machine: the
benchmark five times, bound to the eight fast cores of the processor, every
kernel as the least of the medians of its runs and the largest. Measured on
14 November 2026, on the build after the changes to the AC-3 core that are
described further down, so the core of that family is faster here than those
changes found it:

```text
truehd: frame the access units                    150.824 us  to   164.804 us
truehd: decode presentation 0, 2 channels           3.010 ms  to     3.070 ms  664x real time
truehd: decode presentation 1, 6 channels           7.890 ms  to     8.209 ms  254x real time
truehd: decode presentation 2, 8 channels          10.910 ms  to    11.267 ms  183x real time
truehd: decode presentation 3, 12 channels         17.047 ms  to    17.523 ms  117x real time
truehd: decode 88.2 kHz, 6 channels               545.636 us  to   578.415 us  184x real time
mlp: decode 96 kHz, 6 channels                    577.941 us  to   584.969 us  173x real time
eac3: decode the 5.1 core of the object stream      4.074 ms  to     4.106 ms  495x real time
joc: analysis bank, one channel, one frame         18.035 us  to    18.156 us  1774x real time
joc: synthesis bank, one object, one frame         19.159 us  to    19.460 us  1670x real time
checksum: CRC-16 of a megabyte                      1.806 ms  to     1.818 ms
checksum: CRC-8 of a megabyte                       1.327 ms  to     1.337 ms
checksum: parity of a megabyte                      8.130 us  to     8.231 us
```

**The table that stood here before was wrong from its fifth line down.** It
was one run of `cargo bench`, not bound. Some seconds into a run the system
moves the thread to one of the slow cores of this processor, and a kernel
timed there takes 1.6 to 2.1 times as long, with a spread inside the run no
wider than on a fast core, so nothing in the run says so. The build of the
baseline, measured again three ways, three runs each
(`docs/audit/evidence/engine-round-2026-11/38-kernels-on-cores-of-one-kind.json`):

| kernel | the table said | bound to the fast cores | bound to the slow cores |
|---|---:|---:|---:|
| TrueHD, presentation 3, 12 channels | 32.9 ms | 17.0 to 17.4 ms | 29.4 to 30.5 ms |
| TrueHD, 88.2 kHz, 6 channels | 961 us | 541 to 545 us | 936 to 987 us |
| MLP, 96 kHz, 6 channels | 921 us | 572 to 582 us | 874 to 951 us |
| the 5.1 core of the object stream | 9.63 ms | 5.03 to 5.06 ms | 9.56 to 9.61 ms |
| JOC analysis bank | 37.1 us | 17.9 to 18.1 us | 37.1 to 37.3 us |
| JOC synthesis bank | 39.0 us | 19.4 to 19.5 us | 39.0 to 39.4 us |

The first four lines of the old table were of a fast core and stand. Not
bound, the same build gave the figures of a fast core for its first five
kernels in each of three runs, and of a slow one for the five after them in
all but the last kernel of one run.

The figures of whole commands in this document are not touched by it. A
command measured from outside is a process of its own, and five of them that
run from a quarter of a second to three seconds took the same time not bound
as bound to the fast cores, to within 0.6 per cent over fourteen runs each
way, and 1.5 to 1.8 times as long bound to the slow ones.

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
0.41 for TrueHD. (It no longer decodes: see "A stream of the AC-3 family,
read and not listened to" below.)

**Objects out of JOC cost three to four times the channels under them.** 26 to 42
times real time for the object formats against 82 to 169 for the core. The
kernels say where part of it goes: an analysis bank for every coded channel
and a synthesis bank for every object, 18 and 19 microseconds each for a
frame of 32 milliseconds. Sixteen objects over a 5.1 core is 0.41 ms of
filter bank a frame, which alone would hold the whole to about 77 times real
time: a third to a little over a half of what an object format took. This
paragraph used to say 37 and 39 microseconds, 0.85 ms and 38 times, and so
that the filter banks were all of it. Those were the figures of a slow core.
Where the rest of the object path goes has not been measured. (The banks take
58 per cent of that now, and the rest has been measured: see "The filter
banks of JOC: real and imaginary parts apart" and "A delivery of objects: the
rows in one buffer" below.)

**`verify --decode` is the sum of its presentations**, 30 to 131 times real
time, because it decodes them one after another. (It no longer is: see
"`verify --decode` of TrueHD: the scan and the presentations side by side"
below.)

**The object formats are written to a disk, and it shows.** The two widest
spreads in the table, 35 and 8 per cent, are object files of TrueHD streams;
every figure that is the decoder's alone repeats to within three per cent.

**A stream on standard input costs its own size in memory.** A command reads
its stream more than once, so a pipe is read to its end first and held.
`verify` of the longest stream there is, 2.6 GB, holds 18 MiB reading the file
and 2 506 MiB with the stream piped in, which is the stream and 18 MiB, and
takes 19.7 s against 18.6; of five streams none took more than 20 MiB beside
its own size
(`docs/audit/evidence/engine-round-2026-11/21-standard-input.json`).

So the order of work for speed is the AC-3 core first, then the object path,
of which the filter banks are the half that is known, then the presentations
of `verify --decode` side by side.
Each of those is a change that has to show its figure with `--compare`
against this one, and leave the output where it was.

## What has been done about it

### The frames of the AC-3 family, read without moving the buffer

The order of work above begins with the AC-3 core, and the first thing found
there was not in the decoder. Every command of this family reads the
syncframes of a stream through one loop. It kept four megabytes of the file
in a buffer and, at each frame it handed out, moved what was left of them to
the front: two megabytes on average for a frame of two thousand bytes, some
four gigabytes of copying for a minute of audio, and twice that where a
dependent substream doubles the frames. It moves a cursor now, and gives up
the bytes it has taken when it fills the buffer again. The reads it makes of
the file, and where it decides that bytes are no frame, are what they were.

Measured on 13 November 2026 on the machine of the baseline, the binary of
`main` before the change and after it turn and turn about, five runs each
(`python tools/bench.py --compare`; `docs/audit/evidence/engine-round-2026-11/36-frames-by-a-cursor.json`):

| | work | × real time, before | after | processor s per minute, before | after |
|---|---|---:|---:|---:|---:|
| AC-3, 2 channels | `decode` | 313 | 802 | 0.19 | 0.07 |
| | `verify` | 316 | 841 | 0.19 | 0.07 |
| AC-3, 6 channels | `decode` | 176 | 296 | 0.33 | 0.20 |
| | `verify` | 183 | 318 | 0.33 | 0.19 |
| E-AC-3, 5.1 | `decode` | 176 and 232 | 323 and 349 | 0.35 and 0.26 | 0.19 and 0.17 |
| | `verify` | 185 and 244 | 348 and 381 | 0.33 and 0.24 | 0.17 and 0.16 |
| E-AC-3, 7.1 | `decode` | 83 and 84 | 170 and 169 | 0.72 and 0.70 | 0.36 and 0.36 |
| | `verify` | 86 and 89 | 181 and 176 | 0.69 and 0.67 | 0.31 and 0.34 |
| E-AC-3 with JOC | `decode`, the channels | 169 and 83 | 301 and 159 | 0.35 and 0.72 | 0.20 and 0.38 |
| | `verify` | 173 and 86 | 318 and 168 | 0.35 and 0.69 | 0.19 and 0.36 |
| | `decode --format damf` | 41 and 27 | 46 and 32 | 1.45 and 2.23 | 1.29 and 1.84 |
| | `decode --format adm` | 41 and 27 | 47 and 32 | 1.45 and 2.20 | 1.27 and 1.84 |

A command that reads or decodes the channels takes 38 to 66 per cent of the
time it took, and one that decodes the objects 84 to 89 per cent, the filter
banks being the larger share of that. The less a frame costs to decode, the
more of its time was the copying: two channels of AC-3 gained the most. Two
TrueHD streams ran beside them as a control, through a reader of their own:
thirteen measurements, each within 1.3 per cent of where it was.

FFmpeg, measured again on the same eight streams, decodes them 2.1 to 3.4
times faster than this, where the baseline had it at 4.7 to 6.8 times on
E-AC-3. What is left of that is in the decoder.

Nothing a command says or writes has changed. The binaries before and after
give the same reports and the same deliveries over the 131 streams of the
corpus, and over 320 copies of the fixture damaged the ways a reader of
frames notices: cut short, bytes taken out, bytes that are no frame put in,
bits turned, a frame twice or not at all, half of them long enough for the
damage to stand where a read of four megabytes ends.

### The AC-3 core: an allocation kept, a record on demand, a block decoded in place

With the reader out of the way, timers were put around the stages of the
decoder in a throwaway build, and `verify` run over one stream of each kind.
The shares of a decode, in per cent, on `main` and after the three changes
below:

| | AC-3 5.1, before | after | E-AC-3 5.1, before | after | E-AC-3 7.1, before | after |
|---|---:|---:|---:|---:|---:|---:|
| the inverse transform | 32.9 | 46.5 | 35.8 | 45.6 | 31.4 | 47.2 |
| the mantissas | 18.9 | 23.9 | 19.5 | 24.3 | 23.7 | 26.0 |
| the bit allocation | 14.0 | 4.9 | 13.2 | 5.8 | 16.1 | 4.6 |
| the rest of a block: side information, coupling, rematrixing, the vectors it hands out | 15.7 | 9.2 | 12.5 | 9.5 | 8.7 | 4.7 |
| the record of a block | 2.6 | 0 | 2.9 | 0 | 2.9 | 0 |
| the CRC of the frame | 4.8 | 6.3 | 2.8 | 3.6 | 7.9 | 9.4 |
| pre-noise processing, the release of a frame and the rest | 6.6 | 3.2 | 5.1 | 3.3 | 4.5 | 2.8 |
| the output vectors around the transform | 2.7 | 3.6 | 2.9 | 3.7 | 2.7 | 3.0 |
| the exponents | 1.1 | 1.4 | 1.1 | 1.4 | 1.2 | 1.4 |
| the rest of a frame before the transform | 0.6 | 0.9 | 4.2 | 2.9 | 0.7 | 0.9 |

The timers cost time themselves, so the shares are meant and not the
milliseconds. Three of the rows held work whose result the decoder already
had, or that nobody read.

**The bit allocation.** A syncframe has up to six blocks, and the decoder
allocated for every channel in every one of them. The allocation is
fixed-point arithmetic of the exponents of a channel and a handful of
parameters, and an encoder sends those again far less often than every
block: of the allocations five streams asked for, 78 to 83 per cent had the
inputs of the one before them for that channel. An allocation is kept with
what it was computed from and written again where it is asked for with the
same.

**The record of a block.** Every block carried its side information, its
exponents and its pointers as some twenty vectors. A decode reads none of
them; `eac3-blocks` prints them and `eac3-ecpl-inject` rewrites the coupling
of a frame from them. The record is made where it is asked for, and its
release, which was counted with the rest of a frame, goes with it.

**The coefficients.** A block decoded into an array for each of seven slots,
allocated and cleared for the purpose, and then copied the channels it had
into the vector it hands out. It decodes into that vector.

Each is a commit and was measured by itself against the one before it, on
one stream of each kind, the two binaries turn and turn about, five runs
each. Of the time a command that reads or decodes the channels took, the
allocation kept leaves 88 to 94 per cent, the record on demand 92 to 97 and
the decode in place 81 to 95: eight measurements each, twenty-three of them
faster by more than the spread of the runs and one within it.

The three together, measured on 13 November 2026 against the binary of
`main` on the streams of the table above
(`docs/audit/evidence/engine-round-2026-11/37-the-ac3-core-kept-and-in-place.json`):

| | work | × real time, before | after | processor s per minute, before | after |
|---|---|---:|---:|---:|---:|
| AC-3, 2 channels | `decode` | 792 | 957 | 0.07 | 0.06 |
| | `verify` | 840 | 1014 | 0.07 | 0.06 |
| AC-3, 6 channels | `decode` | 299 | 397 | 0.20 | 0.14 |
| | `verify` | 319 | 429 | 0.19 | 0.14 |
| E-AC-3, 5.1 | `decode` | 322 and 344 | 413 and 439 | 0.19 and 0.17 | 0.15 and 0.13 |
| | `verify` | 341 and 379 | 446 and 482 | 0.17 and 0.16 | 0.13 and 0.12 |
| E-AC-3, 7.1 | `decode` | 170 and 167 | 218 and 213 | 0.36 and 0.36 | 0.27 and 0.28 |
| | `verify` | 174 and 177 | 230 and 225 | 0.34 and 0.33 | 0.25 and 0.27 |
| E-AC-3 with JOC | `decode`, the channels | 296 and 160 | 373 and 202 | 0.20 and 0.38 | 0.16 and 0.30 |
| | `verify` | 315 and 164 | 401 and 213 | 0.20 and 0.36 | 0.15 and 0.28 |
| | `decode --format damf` | 38 and 28 | 38 and 33 | 1.29 and 1.89 | 1.25 and 1.83 |
| | `decode --format adm` | 45 and 32 | 47 and 33 | 1.28 and 1.86 | 1.23 and 1.78 |

A command that reads or decodes the channels takes 74 to 83 per cent of the
time it took, sixteen measurements and every one of them faster by more than
the spread of its runs. The object formats spend three to four per cent less
processor time; how long they take is the disk's to say more than the
decoder's, and it said 86 to 100 per cent. The two TrueHD streams ran beside
them as a control: eleven measurements of commands that write no object
file, each within 1.3 per cent of where it was, and two of object files,
which went through none of the code that changed and moved by 12 and 13 per
cent in this run and by 38 in one of the runs before it. A figure of an
object format in these tables is a figure of the machine at that minute.

FFmpeg, measured again on the same eight streams, decodes them 1.8 to 2.8
times faster than this, where it was 2.1 to 3.4 before these changes. What
is left is the transform, close to half of a decode now, and the reading of
the mantissas, a quarter of it: work, and not waste.

Nothing a command says or writes has changed. The binaries before and after
give the same reports and the same deliveries over the 131 streams of the
corpus and over 320 damaged copies of the fixture, the campaign of 200 000
rounds gives the answers it gave on `main`, and `eac3-blocks` prints the
same bytes for every frame of the fixture, with `--detail` and without, as
`eac3-ecpl-inject` writes the same stream.

One thing the change found was not about speed. A mutation that decoded the
LFE over the last full-bandwidth channel survived every test that runs
without media: none of them looked at the samples of a decoded E-AC-3
stream, and the LFE of the fixture is silent. A test holds the fixture to
the level and the peak of each of its six channels now.

### The checksums, eight bytes at a step

A syncframe of the AC-3 family carries a CRC-16 over all of its bytes, and a
substream segment of TrueHD a CRC-8 and a parity over all of its own. Both
CRCs went through a table of 256 entries a byte at a time, where every byte
waits for the look-up of the one before it: 1.81 ms for a megabyte of CRC-16
and 1.33 ms of CRC-8 on a fast core, 580 and 790 megabytes a second, against
8 microseconds for the parity, which the compiler makes vector code of by
itself. The CRC of the frame was 4 to 9 per cent of an AC-3 decode after the
changes above.

They keep eight tables now, the byte table and that table applied to itself
up to seven times, and take eight bytes at a step: the register and seven
bytes are looked up side by side, each in the table of the zeros that follow
it. A megabyte takes 0.38 ms and 0.31 ms (`python tools/kernels.py --affinity
FFFF --runs 5`, before and after).

What that is worth to a command was measured inside one binary, built to run
the checksums either way by a switch: the two ways turn and turn about, nine
runs each, bound to the fast cores. Of the time the command takes a byte at
a time, in per cent
(`docs/audit/evidence/engine-round-2026-11/39-checksums-eight-bytes-at-a-step.json`):

| | `verify` | `decode` of the widest presentation |
|---|---:|---:|
| AC-3, 6 channels | 95.0 | 94.5 |
| E-AC-3, 5.1 | 97.0 | 97.4 |
| E-AC-3, 7.1 | 91.6 | 92.4 |
| TrueHD, 6 channels | 93.6 | 97.3 |
| TrueHD, 8 channels | 89.6 | 95.5 |
| TrueHD with objects, 12 channels | 89.2 | 97.8 |
| TrueHD with objects, 16 channels, two streams | 88.8 and 88.8 | 94.6 and 97.0 |

It was measured that way because two binaries do not differ by the change
alone. The binary of the change against the binary of `main`, the usual
comparison, over eighteen streams and 88 commands: 65 faster by more than
the spread of their runs, 20 within it and 3 slower. `verify` of TrueHD takes
87 to 95 per cent of the time it took and `verify --decode` 93 to 97, every
one of twenty faster; `verify` and `decode` of the AC-3 family 92 to 99. The
three that are slower are of one stream, one of the two with a 16-channel
presentation: that presentation decodes 4.6 per cent slower to PCM and 3 to
4 per cent slower to the object formats, and did so again when the stream
was measured by itself, while the same commands of the other 16-channel
stream are 3 to 4 per cent faster.

A checksum does not slow a matrix down, so it was looked for. In a build
with timers, the checks of that stream's segments take 17 ms on the thread
that decodes where they took 48, and the matrix stage, which this change
does not touch and which is 43 per cent of that decode, takes 404 ms in one
build and 423 in the other, the slower one being the build *without* the
change. In the one binary with the switch the same presentation of the same
stream decodes in 97.0 per cent of the time when the checksums go eight
bytes at a step. Keeping the checksum routines out of line, and making their
tables statics, change nothing. What the two-binary comparison saw is where
the compiler put the matrix loop in each build: it moves that stage by some
per cent either way with every change anywhere in the program, and for this
one stream it moved the wrong way in the build that is shipped. It is the
stream that asks most of that stage: 14.2 matrices a sample with 86.5 per
cent of their coefficients set, against 10.1 and 71.5 for the other
16-channel stream. That is worth knowing about the matrix stage, which is the next
thing to be worked on, and about every figure of a few per cent in this
document.

Nothing a command says or writes has changed: the same reports and the same
deliveries over the 131 streams of the corpus, whose checksums other
encoders wrote, and the same answers in the campaign of 200 000 damaged
streams.

### The matrix stage of TrueHD: a matrix that stands still

With the checksums cheap, timers went around the stages of the TrueHD decoder
in a throwaway build, for the widest presentation of the 16-channel stream of
the section above. Of the time of the thread that decodes, the matrix stage is
44 per cent, the prediction filters 23, the reading of the blocks, entropy
decoding and all, 24, and the parity and the CRC of the segments 2.

A matrix of an object substream (`0x31EC`) is interpolated over the access
unit: every coefficient has a delta, and the stage summed the samples by the
coefficients and by the deltas, for every matrix of every sample. Counted over
the widest presentations of six object streams, the deltas were half of
everything the stage multiplied, and 83 to 100 per cent of the matrices
applied had deltas that were all zero: a matrix stands still most of the time.
The sum of the samples by deltas of zero is zero whatever the samples are, so
a matrix that stands still is no longer multiplied by them. The coefficients
themselves are 71.5 to 86.5 per cent set, and nothing is to be had from
leaving out the ones that are zero.

Inside one binary that can sum every matrix with its deltas as before, both
ways turn and turn about, nine runs each, bound to the fast cores. Of the time
with every delta, in per cent
(`docs/audit/evidence/engine-round-2026-11/40-a-matrix-that-stands-still.json`):

| TrueHD with objects | `decode` of the object presentation | `verify --decode` |
|---|---:|---:|
| 12 channels, two streams | 91.2 and 93.8 | 97.2 and 97.5 |
| 14 channels, two streams | 88.4 and 90.7 | 96.2 and 95.9 |
| 16 channels, two streams | 91.7 and 89.3 | 96.2 and 95.2 |

Four streams without an object substream, which run none of it, measure 99.3
to 101.1 the same way.

The binary of the change against the binary of `main`, eighteen streams and 88
commands: the object presentation decodes to PCM in 83 to 91 per cent of the
time it took, to the object formats in 84 to 97, and `verify --decode` of an
object stream takes 92 to 97, 23 of 24 faster by more than the spread of their
runs and the other within it.

The tool calls 2 other commands slower, by up to 2.6 per cent. They are
presentations of six and eight channels, whose matrices have no deltas, and
they run none of what changed: it is the placing of the stage again, of which
the section before this one tells.

No sample of any presentation of any of the 131 streams of the corpus changes,
and the fixture, a quarter of whose matrices move, holds both ways of summing
to the samples they gave.

### A stream of the AC-3 family, read and not listened to

`verify` of this family decoded every frame to samples, and so did `info`,
and so did the check of the stream that runs beside `emdf` and `oamd`. One
figure of a report is of the samples: the level of the core under each clip
gain of the JOC payloads (`JOC core level:`), a row `verify` and `info` print
for a stream whose payloads carry more than one clip gain. Of the 76 streams
of the family in the corpus, 52 carry no JOC, 22 carry one clip gain and 2
more than one. For the other 74, and for the check, which is a verdict and a
first problem, the transform made samples that nothing looked at.

A decoder can be told that nobody listens (`Options::samples`). It parses
every frame to its last bit and makes its checks, runs neither the synthesis
of enhanced coupling nor the transform, and hands out the frames it would
have, in the calls it would have, with their channels and no samples in them.
`verify`, `info` and the check read a stream that way. Where a second clip
gain shows, `verify` and `info` start over from the first frame with the
samples, so that the row is the one every frame decoded gives. `verify
--decode`, which this family took and did nothing with, makes the samples of
every frame, and its report is the same.

Measured on 14 November 2026. Inside one binary, `verify` and `verify
--decode` turn and turn about, nine runs each, bound to the fast cores
(`docs/audit/evidence/engine-round-2026-11/41-a-stream-read-without-listening.json`):

| | × real time, with the samples | without | of the time, per cent |
|---|---:|---:|---:|
| AC-3, 2 channels | 1 017 | 1 712 | 59.4 |
| AC-3, 6 channels | 429 | 808 | 53.1 |
| E-AC-3, 5.1, two streams | 437 and 466 | 883 and 1 018 | 49.5 and 45.8 |
| E-AC-3, 7.1, two streams | 239 and 233 | 441 and 412 | 54.3 and 56.6 |
| E-AC-3 with JOC, 5.1 and 7.1 | 394 and 217 | 761 and 381 | 51.7 and 56.9 |

The two streams that have the row pay for it. They are read without their
samples up to the frame where the second clip gain shows, frame 117 of 976
and frame 113 of 313, and then again from the first with them: 110 and 124
per cent of the time. The figure is of every frame from the first, so nothing
short of knowing beforehand that a stream has it would read it cheaper.

Against the binary of `main`, `info`, `verify`, `emdf` and `oamd` on one
stream of each kind take 46 to 58 per cent of the time they took, twenty
measurements bound to the fast cores. `tools/bench.py --compare` over ten
streams calls 12 of 33 commands faster and none slower: `verify` of the
family takes 45 to 58 per cent, and the object formats and the two TrueHD
streams that ran as a control are where they were.

**A delivery was slower, and it was not the decoder.** The first time the
two binaries were compared, `decode` to PCM was 3.5 to 12.2 per cent slower
with the change, which changes nothing a delivery runs. Built with the
decoder of the change alone, the binary delivered a two-channel AC-3 stream
in 130.1 ms against 132.2 for `main`; with the commands of the change as
well, in 146.4. All the commands had changed for a delivery was the pass
around one loop, which keeps two counters more. The loop put the samples of
a frame into the bytes of the file, a sample of every channel at a time,
pushed onto a new vector, and not a line of it had changed. Written a channel
at a time into a buffer of the size of the frame, in `main` and in the
change, it left the two builds within 1.2 per cent of each other and both
faster than `main`. That is how it is written now, as a function with a test
of its own and one buffer for all the frames of a delivery, which is one per
cent by itself inside one binary. A delivery to PCM takes 96.8 and 96.4 per
cent of the time it took on `main`, on two streams, fifteen runs bound to the
fast cores.

This is the section on the checksums again: a difference between two builds
that the change does not explain is where the compiler put something. This
time what it put somewhere else was a loop that could be written so that it
does not matter.

Nothing a command says or writes has changed: the same reports and the same
deliveries over the 131 streams of the corpus, and the same answers in the
campaign of 200 000 damaged streams. In the fuzz target of the programme
decoder a decoder that does not listen runs beside one that does, held to
the same frames in the same calls, the same refusals and the same counts.

Every stream of the family in the corpus has six blocks a frame, and so has
the fixture. That a frame nobody listens to counts for the samples its blocks
would have made is tested on the buffer by itself, for frames of one, two,
three and six blocks; no stream of shorter frames has been through both
decoders.

### `verify --decode` of TrueHD: the scan and the presentations side by side

`verify --decode` scanned a stream and then decoded every presentation it
carries, a unit in each presentation after another. Of a 16-channel stream of
a minute the scan takes 0.25 s by itself and the decode of its widest
presentation 0.69, and `verify --decode` took 1.69: the scan and four
presentations, one after another on one thread.

They run side by side now. The scan has a thread of its own, and tells the
first major sync of the stream as soon as it has it, which is what the
decodes are configured from. The stream is then read once more, on the thread
of the command, and its access units are handed, sixty-four at a time, to a
thread for each presentation. A presentation that stops at a fault takes no
more units, and the others go on to the end, as they did. The command takes
what its widest presentation takes to decode.

Measured on 14 November 2026, the binary of `main` and the binary of the
change turn and turn about, five runs each (`tools/bench.py --compare`;
`docs/audit/evidence/engine-round-2026-11/42-the-scan-and-the-presentations-side-by-side.json`).
The effect is a factor of two to three, which two builds do not differ by, so
it was not measured a second time inside one binary:

| TrueHD | × real time, before | after | of the time, per cent |
|---|---:|---:|---:|
| 6 channels, two streams | 132 and 134 | 239 and 248 | 55.0 and 54.1 |
| 8 channels, two streams | 69 and 38 | 169 and 116 | 41.1 and 33.0 |
| objects, 12 channels, two streams | 40 and 39 | 109 and 105 | 36.3 and 37.1 |
| objects, 14 channels, two streams | 38 and 39 | 108 and 103 | 35.4 and 37.4 |
| objects, 16 channels, two streams | 36 and 35 | 88 and 84 | 40.6 and 41.6 |

That is 94 to 100 per cent of what `decode` of the widest presentation takes
by itself. The processor time is the same work on more threads, 92 to 107 per
cent of what it was, and the command holds 23 to 28 MiB where it held 14 to
19: two readers of the stream and the threads at once. The longest stream
there is, 2.6 GB, takes 46.7 s where it took 127.7.

Of the 78 other commands of the comparison the tool calls three slower:
`verify` of a 7.1 E-AC-3 stream by 3.0 per cent, a `decode` of six channels
by 2.1, and a `decode --format damf`, which is written to a disk, by 46. The
first runs nothing that changed, and the other two only the scan, which asks
one question more for every unit. Measured again bound to the fast cores,
eleven runs each and seven for the object format, they take 100.4, 99.1 and
101.1 per cent of the time.

Nothing a command says has changed: the same reports over the 131 streams of
the corpus, `verify --decode` of every one among them, and the same answers
in the campaign of 200 000 damaged streams. That the threads run side by side
is what the times show and no test asserts. A test holds every presentation
to what its decode says by itself, on streams whole and damaged, which is
true of one after another as well.

A stream of four presentations runs six threads. This was measured on a
machine of twenty-eight processors; on fewer than the command has threads it
does the same work and gains less.

### The inverse transform of the AC-3 family: real and imaginary parts apart

Timers around the stages of the AC-3 decoder again, in a throwaway build,
this time over a decode that makes its samples (`verify --decode`). On three
streams the inverse transform is 44 to 48 per cent of a decode, the mantissas
25 to 32, and the bit allocation, which was a quarter before it was kept, 5
to 6.

Two things that would have been cheap were counted first, and are not there.
A block whose coefficients are all zero needs no transform: over nine streams
such blocks are under one per cent of what the transform is given in seven,
and 7 and 12 per cent in the other two. The top half of a block is zero in 10
to 18 per cent of the blocks of a stream that has an LFE channel, which is
that channel; but the first step of the transform folds the top of a block
onto its bottom, and a half that is zero saves it nothing.

What there was is in how the numbers lie. The inverse FFT inside the
transform had a complex value for every element, in a slice whose length it
learned when it ran, and one table of twiddles that every stage stepped
through. Four ways of writing it, inside one binary, the transform of 1 536
blocks bound to the fast cores
(`docs/audit/evidence/engine-round-2026-11/43-the-inverse-transform-real-and-imaginary-apart.json`):

| | ms | of the time, per cent |
|---|---:|---:|
| as it was | 1.427 | 100 |
| a table of twiddles for every stage, and a butterfly over the two halves of a block | 1.254 | 88 |
| that, over an array of a known size, the values put straight into bit-reversed places | 1.172 | 82 |
| that, with the real parts in one array and the imaginary parts in another | 0.740 | 52 |

The last is the one taken, and what the parts apart are worth is in what
the compiler makes of a butterfly, which is ten operations: four
multiplications, three additions and three subtractions. The stages of each
of the four, each put in a procedure of its own, in one linked binary
(`tools/packed.py`, `docs/audit/evidence/engine-round-2026-11/46-what-the-compiler-makes-of-the-parts-apart.json`):

| the stages | instructions that take one number | that take two | that only move numbers between the halves of a register |
|---|---:|---:|---:|
| as they were | 10 | 0 | 0 |
| a table of twiddles for every stage, and a butterfly over the two halves of a block | 10 | 10 | 10 |
| that, over an array of a known size | 10 | 10 | 10 |
| that, with the real parts in one array and the imaginary parts in another | 10 | 10 | 0 |

As they were, the stages take one butterfly at a time. With a table for
every stage the compiler takes two at a time, and keeps a scalar loop for
the stage whose blocks are a single butterfly. But a complex value is its
real part and then its imaginary part, and two of them have to be taken
apart before the packed instructions and put back after: ten instructions
that compute nothing for ten that do. With the parts apart there is nothing
to move. In the command, where the size of the 64-point transform is known
where it is compiled, its stages are 400 instructions that take two numbers
and not one that takes one.

This section used to say that the compiled code takes one number at a time
in every one of the four, and that why the parts apart are worth what they
are was not known. What it had read was the assembly the decoder crate is
compiled to by itself (`cargo rustc -p oadec-eac3 --lib -- --emit asm`),
which has 72 scalar multiplications and 108 scalar additions and
subtractions for the stages of the 64-point transform. That is the code
before the linker has optimised it, and the loops of the binary are not the
loops of that file: with link-time optimisation off, the assembly of the
same crate has the 400 packed instructions the binary has.

The window and the overlap-add in one pass, with no 512 samples in between,
were tried on top of it and measured 0.717 to 0.759 ms against 0.745 to
0.754, which is nothing, so they are as they were.

Every operation on every value is what it was, in the order it was, and so
the samples are the same to the last bit and to the sign of a zero. A test
holds the transform, long and block-switched, to the clause computed the
plain way: one complex value at a time, a twiddle computed where it is used,
on blocks of noise, noise with a top that is zero, single bins and nothing at
all, with the zeros of each in both signs. The forward transform of enhanced
coupling goes through the same stages and is held the same way.

The benchmark has the transform as a kernel of its own now. On `main` and
with the change, bound to the fast cores, the least of five runs: 1.495 ms
and 0.787 for the 1 536 blocks, and 4.105 ms and 3.062 for the decode of the
5.1 core of the fixture. (These are of a run of their own, hours after the
table above: one kernel moves by some per cent from run to run, and the
figures of one run are the ones to hold against each other.)

The binary of the change against the binary of `main`, five runs each
(`tools/bench.py --compare`):

| `decode` to PCM | × real time, before | after | of the time, per cent |
|---|---:|---:|---:|
| AC-3, 2 channels | 1 022 | 1 225 | 83.4 |
| AC-3, 6 channels | 421 | 528 | 79.6 |
| E-AC-3, 5.1, two streams | 431 and 466 | 551 and 604 | 78.1 and 77.2 |
| E-AC-3, 7.1, two streams | 239 and 231 | 295 and 281 | 81.0 and 82.1 |
| E-AC-3 with JOC, the channels, 5.1 and 7.1 | 393 and 214 | 496 and 260 | 79.3 and 82.3 |

Bound to the fast cores, fifteen runs each, a two-channel AC-3 stream is
delivered in 104.9 ms where it took 125.8, and a 5.1 E-AC-3 stream in 194.8
where it took 246.8.

`verify` is where it was, because it makes no samples: 96 to 103 per cent,
and the one the tool calls slower, by 2.5 per cent, takes 99.9 measured again
on the fast cores. The object formats take 97 to 99 per cent of the time; the
transform of the core is a small part of them. FFmpeg, measured again on the
same eight streams, decodes them 1.3 to 2.1 times faster than this, where the
baseline had it at 4.7 to 6.8 times on E-AC-3.

Nothing a command writes has changed: every delivery of every stream of the
corpus is the same byte for byte, and the campaign of 200 000 damaged streams
gives the answers it gave.

### The filter banks of JOC: real and imaginary parts apart

The baseline has objects out of JOC at three to four times the cost of the
channels under them, and the filter banks as the part of that which is known:
an analysis bank for every coded channel and a synthesis bank for every
object, each with an inverse DFT of 128 points a time slot. Those inverse DFTs
were written the way the one of the AC-3 transform was, a complex value for
every element and one table of twiddles for all the stages. They were given
what that one was given in the section above: the real parts in one array and
the imaginary parts in another, a table for every stage, the values put
straight into their bit-reversed places. What the compiler makes of it is
what it makes of that one: in a linked binary with both, the stages there
were are ten scalar instructions, one butterfly at a time, and the stages
with the parts apart are those and ten that take two numbers, with none
that only moves them.

Inside one binary, a kernel for each bank each way, bound to the fast cores
(`docs/audit/evidence/engine-round-2026-11/44-the-filter-banks-of-joc-real-and-imaginary-apart.json`):

| | µs a frame, as it was | the parts apart | of the time, per cent |
|---|---:|---:|---:|
| the analysis bank, one channel | 18.56 | 10.74 | 57.9 |
| the synthesis bank, one object | 19.73 | 11.37 | 57.6 |

Sixteen objects over a 5.1 core were 0.41 ms of filter bank a frame, and are
0.24.

A delivery of objects (`decode --format damf`) from one stream of each kind of
JOC stream the corpus has, the binary of `main` and the binary of the change
turn and turn about, five runs each bound to the fast cores:

| | × real time, before | after | of the time, per cent |
|---|---:|---:|---:|
| 11 objects over 5.1, 12 parameter bands | 45.6 | 56.0 | 81.5 |
| 15 objects over 5.1, 12 parameter bands | 36.1 | 43.8 | 82.4 |
| 15 objects over 7.1, 15 parameter bands | 31.6 | 38.1 | 83.0 |

`tools/bench.py --compare` has the object formats of its two JOC streams at 82
to 83 per cent, and nothing else moved: 5 of 33 commands faster, none slower.

The samples are the same to the last bit. A test holds both banks to the
clause computed the plain way, one complex value at a time, over a signal,
zeros of either sign and the smallest numbers there are; and the audio of
every object delivery of the corpus is the same byte for byte.

What a delivery of objects is made of beyond the banks has still not been
measured. The banks were its largest known part, and they are smaller now.

### A delivery of objects: the rows in one buffer

The section above left what a delivery of objects is made of beyond the banks
unmeasured. Timers around its stages, in a throwaway build, over one stream of
each kind of JOC stream the corpus has (`decode --format damf`), in per cent
of the command, the least and the most of the three streams:

| | on `main` | with the change |
|---|---:|---:|
| the synthesis banks | 27.5 to 32.2 | 29.8 to 34.6 |
| the rows taken off the queues | 11.9 to 13.1 | 7.1 to 8.0 |
| the rows written | 9.5 to 11.1 | 9.1 to 13.6 |
| the core decoded | 9.1 to 14.2 | 10.2 to 15.4 |
| the analysis banks | 8.5 to 11.3 | 10.1 to 12.9 |
| the matrices updated from the payload | 8.9 to 11.0 | 10.1 to 12.7 |
| the matrix applied | 5.3 to 6.2 | 5.7 to 7.1 |
| the samples put on the queues | 2.0 to 2.4 | 0.1 |
| the payloads read, the quadrature, the metadata, the rest of the pipeline | 2.8 to 3.4 | 3.0 to 3.7 |
| everything else of the command | 4.5 to 5.0 | 2.4 to 3.3 |

The samples of every element wait on a queue until every element has them,
because the objects come out of the banks later than the core does. They were
put on the queues one at a time. They were taken off one at a time into a
vector made for every row, 1 536 vectors a frame, which were freed after the
frame was written, where no timer runs: that is in the last line. None of it
is signal processing, and with the writing it was a quarter of the command.

The rows of a frame are taken into one buffer now, an element at a time, and
every frame of the delivery uses that buffer again. The samples of an object
go from its bank straight onto its queue, a time slot at a time, where a
frame of them was collected first. A queue is extended by what it is given,
once the delay of the decoder has been dropped from the front of it. Putting
the samples on the queues, taking the rows off and writing them took 4.7 to
6.1 ms a second of audio in the build with the timers, and take 3.3 to 3.8.

A delivery (`decode --format damf`), the binary of `main` and the binary of
the change turn and turn about, five runs each bound to the fast cores
(`docs/audit/evidence/engine-round-2026-11/45-the-rows-of-an-object-delivery-in-one-buffer.json`):

| | × real time, before | after | of the time, per cent |
|---|---:|---:|---:|
| 11 objects over 5.1, 12 parameter bands | 55.1 | 65.6 | 84.1 |
| 15 objects over 5.1, 12 parameter bands | 43.0 | 51.4 | 83.5 |
| 15 objects over 7.1, 15 parameter bands | 37.7 | 44.4 | 85.0 |

(The figures before are of this run. The section above has the same binary at
56.0, 43.8 and 38.1.)

The writers of the two object formats were changed with it. They pushed the
three bytes of every sample onto a vector, one sample at a time; they put
them into a buffer that is given its size first, and the DAMF writer keeps
the buffer it gathers the channels of its rows in. Inside one binary, a frame
of 1 536 rows of sixteen elements, bound to the fast cores, in kernels of a
throwaway worktree:

| | µs a frame, as it was | with the change | of the time, per cent |
|---|---:|---:|---:|
| the bytes of a CAF, which is the audio of a DAMF delivery | 14.9 | 11.7 | 79 |
| a frame of DAMF rows: the channels gathered, then those bytes | 35.8 | 24.0 | 67 |
| a frame of ADM rows into its bytes | 28.1 | 24.2 | 86 |

The second has an allocation in it that grew four times a frame and is kept
now, and a kernel measures an allocation of a heap that holds nothing else.
In a delivery the stage that writes the rows has the disk in it, and what the
writers gained cannot be told there from what the disk does: it took 63 to 81
µs a frame on `main` and takes 64 to 74.

`tools/bench.py --compare` wrote its object files to a disk that was doing
something else in this run, and the object formats show it. Its two JOC
streams have theirs at 76 to 80 per cent of the time and 80 to 86 of the
processor time. Those of the TrueHD streams, which go through the same
writers an access unit at a call, go from 0.72 to 1.38 of what they were, and
the tool calls two of them slower: `decode --format damf` at 1.32 and 1.38
times, with spreads of 9 and 10 per cent. Of the 88 measurements it has 1
faster, 85 where they were and those 2 slower. Bound to the fast cores, nine
runs each, the two take 99.2 and 98.6 per cent of the time they took, and the
ADM delivery of the second 97.3.

A mutation run over the files of the change found three things of the
pipeline that no test held, all of them older than the change: the clip gain
put back into the objects, the clip gain put back into a channel of the core,
and which element an object is written as. The fixture has a clip gain of one
and an LFE that is silent, and the two object formats were held to each
other, which they are when an object is in the wrong element of both. A test
puts frames of the fixture through a pipeline with a payload that says
another clip gain and a ramp for an LFE, and another holds the element of
every object.

One thing was seen and left as it is. When the stream ends, the pipeline runs
zeros through the banks so that the objects catch up with the core, and puts
what comes out on the queues without the clip gain every frame before it was
multiplied by. In 22 of the 24 JOC streams of the corpus every payload says a
gain of one, and there it is nothing. Whether it is right is a question for
the decoder this one is measured against, and not for a change that moves no
byte.

Nothing a command writes has changed: every delivery of every stream of the
corpus is the same byte for byte, and the campaign of 200 000 damaged streams
gives the answers it gave.

The synthesis banks are 30 to 35 per cent of a delivery now, the analysis
banks and the matrices they feed 10 to 13 each, and the rows, taken off and
written, 16 to 22.

## What it does not say

- One machine. The ratios between commands carry to another; the figures do
  not.
- One kind of core. This processor has eight fast cores and twelve slow
  ones, and decoding takes 1.5 to 2.1 times as long on a slow one. The
  kernels are of a process bound to the fast cores. The whole commands were
  not bound: commands of up to three seconds have been seen to stay on the
  fast cores, and a longer one has not been looked at.
- One build. Where the compiler places a loop moves it by some per cent: the
  matrix stage of TrueHD, from the same source, took 404 ms in one build and
  423 in another that differed elsewhere. A difference of a few per cent
  between two binaries, in one command of one stream, is within that. It is
  told apart from the change itself by running both ways inside one binary.
- Heads of about a minute. A command that takes a quarter of a second is
  measured to a few per cent at best, and the processor time of so short a
  run is counted by a clock that steps by a sixty-fourth of a second.
- The streams are the heaviest of each kind there were, not a sample of what
  is common.
- Nothing here is a claim about another decoder. The reference rows are the
  same measurement on the same streams, of programs that do less.
