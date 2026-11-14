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
0.41 for TrueHD.

**Objects out of JOC cost three to four times the channels under them.** 26 to 42
times real time for the object formats against 82 to 169 for the core. The
kernels say where part of it goes: an analysis bank for every coded channel
and a synthesis bank for every object, 18 and 19 microseconds each for a
frame of 32 milliseconds. Sixteen objects over a 5.1 core is 0.41 ms of
filter bank a frame, which alone would hold the whole to about 77 times real
time: a third to a little over a half of what an object format took. This
paragraph used to say 37 and 39 microseconds, 0.85 ms and 38 times, and so
that the filter banks were all of it. Those were the figures of a slow core.
Where the rest of the object path goes has not been measured.

**`verify --decode` is the sum of its presentations**, 30 to 131 times real
time, because it decodes them one after another.

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
