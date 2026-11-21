# Decoding a stream as it arrives

`oadec decode --format pcm|wav` reads its stream once, as it arrives, from
standard input (`-`), and writes its samples to a file or to standard output
(`-o -`) unit by unit:

```text
mkvextract tracks input.mkv 1:/dev/stdout | oadec decode - --format wav -o - | ffplay -
```

Nothing of the stream is held in memory but the units being decoded, a queue
of a megabyte between the reader and the decode, the 64 KiB at most that tell
the codecs apart, and what the checks of a TrueHD stream have still to look
at. The first samples are on standard output as soon as the first units have
decoded, long before the stream has ended.

## Which decode takes which path

| `decode --format pcm\|wav` | Path |
|---|---|
| `-` as input, or `-o -` as output | read once, as a stream |
| a file to a file, with `--json`, `--report`, `--on-layout-change` (or the hidden `--pipeline-stats`) | read once, as a stream: what those options report is of that path |
| a file to a file, with none of them | the decode it always was: the checks of `verify` read the file on a thread of their own beside the decode |
| `-p all`, a file to files | every presentation of a TrueHD stream in one pass (below); the stream is taken whole, as a decode of a file takes it, with the checks of `verify` beside it |
| `-p all` with `--json` or `--report` | every presentation in one pass, the stream read once, as a stream (below) |
| `--programme all`, to files | every programme of an AC-3 or E-AC-3 stream in one pass, the stream read once, as a stream, from a file or a pipe (below) |

The last row is the faster where the stream is a file. A TrueHD decode hands
every access unit to the checks when it reads its stream once, and the checks
are then held to the decode; two readers of a file are not held to each
other, and on a processor whose cores differ in speed the one-pass decode of
a file measured up to a fifth slower. Both paths decode with the
same code (`decode::Delivery`), write the same bytes, say the same and end
with the same exit code, which a test holds on every fixture, clean and
damaged.

## Which commands read standard input as a stream

| Command | Standard input |
|---|---|
| `decode --format pcm`, `decode --format wav` | read as a stream, once |
| `info`, `verify` (with and without `--decode`), `emdf`, `oamd`, `au` | read once, as it comes (below) |
| `decode --format damf`, `decode --format adm`, `compare`, `eac3-blocks` | taken into memory whole, then read as often as the command reads a file |

The commands of the last row read their stream more than once, and a pipe can
be read once; they hold standard input in memory for as long as they run, as
they always have. Reading them as a stream is a later round of work.

A file is read as it always was, by every command: what is below is of
standard input.

## The reports of a stream read once

`info`, `verify`, `emdf`, `oamd` and `au` read standard input once, on the
thread that runs the command, and hold no copy of it: what they hold is
bounded but for what a report keeps of every unit, which is small and grows
with the length of the stream (below):

```text
mkvextract tracks input.mkv 1:/dev/stdout | oadec verify -
```

The first 64 KiB tell the codecs apart, as they do for a file; a pipe that
gives them a little at a time is read until they are all there. Then the
stream is framed by the framer a walk over a file uses (an AC-3 family stream
cut where the reads of four megabytes of a file would cut it), and every
access unit or syncframe is lent in turn to each part of the command: the
walk of what the command reports, and the checks of `verify` that `emdf` and
`oamd` take their verdict from. A file of the stream is read twice for that,
the checks on a thread of their own; the pipe is read once and each unit is
looked at by both before the next is read. What the commands print, the JSON
they write and the exit codes they end with are those of a file of the same
bytes, but for the name: the stream is `<stdin>`.

What a run holds is a read of four megabytes, the head, and what the report
keeps: the findings, which are listed in full up to a limit and counted past
it. Measured in a release build on an idle machine, the most memory a run
held was 17.1 MiB for `info`, `verify`, `emdf` and `oamd` of a TrueHD stream
of 2.6 GB (16.8 MiB for `au`), and 17.3 to 17.5 MiB for an E-AC-3 stream of
some 500 MB; read whole first, as before, they held 2 505 to 2 514 MiB and
485 to 490 MiB. A test pipes 96 MiB into the commands and holds each to
64 MiB. What was measured is the most the process had held when the last
byte had been written into the pipe and before the pipe was closed: what the
command does once it has read the end (the last checks, the decodes of
`verify --decode` ending, the report being put together and printed) is not
in it.

The memory of a run is bounded but for these, which grow with the stream,
from a file as from a pipe:

- the event times `emdf` keeps of an E-AC-3 stream, eight bytes for each
  block of object metadata, of which it prints the first 24: some 250 bytes
  for every second of a stream that carries a block in every frame, under
  3 MB for three hours (twice that at most while the list grows);
- the decoder-buffer trace of `verify --trace-fifo`, a row for every access
  unit (some hundred bytes each, written when the stream has ended);
- the findings of `verify --max-per-rule 0`, which lists every finding in
  full: a damaged stream may have one in every unit.

The last two are asked for, and are outside the bound of 64 MiB.

A known limit: `emdf` and `oamd` of an E-AC-3 stream take longer from a pipe
than from a file, 8.4 s against 4.6 s for the stream of some 500 MB above
(1.8 times; `info` and `verify` of it, and every command of a TrueHD stream,
take the same time from both). A file is read twice, the walk of the report
and the checks of `verify` each on a thread of its own, at the same time; a
pipe is read once, and each syncframe goes through the walk and then the
checks on the one thread.

`verify --decode` of a TrueHD stream decodes every presentation the stream
carries on a thread of its own, beside the scan, as it does for a file; the
units are handed to the decodes 64 at a time, as they are framed. The decodes
begin at the unit that carries the first major sync, and are handed the first
unit of the stream before it: a decode that cannot open at the first unit
stops there, as it does for a file, and no unit between the two is held.

A command that has what it needs before the stream has ended does not read
the rest: `au` once it has described its unit, and a command that refuses its
options once the head has told it the codec. Whoever writes into the pipe is
then told the pipe is closed (`SIGPIPE`/`EPIPE` on Linux, `ERROR_NO_DATA` on
Windows); `oadec` ends as it does for a file. A stream that ends inside a unit
is judged as a file that ends there; a read that fails ends the run with
`reading standard input: …` and exit code 2.

On a terminal the line that says how far the walk has got shows the bytes
read and the rate, and no bar, percentage or estimate: the length of a pipe is
not known.

## What goes where

With `-o -`, standard output carries the samples and nothing else: no
summary, no warning, no progress line, no verdict. All of that goes to
standard error, as it does for a decode to a file. What an AC-3 or E-AC-3
decode to a file prints on standard output, the report of its pass, goes to
standard error too.

`--json` replaces the summary and the verdict in text with a report in JSON.
It is printed on standard output for a decode to a file, and on standard
error, as its last line, for a decode to standard output. Warnings are still
said on standard error before it. `--report FILE` writes the same report to a
file as well, whatever the output is. `--report -` names standard output, and
`-o -` refuses it (exit 2): the two cannot share a pipe.

The report holds no time and no speed, so the report of the same stream is
the same file on every run:

| Field | What it is |
|---|---|
| `stream` | the name of the stream: its path, or `<stdin>` |
| `format` | `TrueHD`, `MLP`, `AC-3` or `E-AC-3` |
| `output` | `format` (`pcm` or `wav`), `channels`, `rate`, `mask`, `sample` (`s24le` or `f32le`); `null` where nothing was written |
| `samples` | samples written, per channel |
| `units`, `bytes` | access units or frame groups framed, and bytes read |
| `parts` | the files written, in order: one, or more with `--on-layout-change split`; empty to standard output |
| `events` | in the order of the units: `opened`, `config_change` (`unit`, `byte_offset`, `what`, `policy`, `old`, `old_rate`, `new`, `new_rate`), `finding` (`unit`, `byte_offset`, `rule`, `message`) |
| `findings` | what the verdict rests on, as `integrity:` says it on standard error |
| `first_problem` | the first problem the decode met, or `null` |
| `verdict` | `clean`, `lossy` or `faulty` |
| `exit_code` | 0, 4 or 7 |

A `finding` is one of the findings the checks of `verify` list in full: the
first twenty of each rule, every one of them counted in `findings` all the
same.

## Samples to standard output

`--format pcm` writes the samples as a headerless file of the same decode
holds them: interleaved, little-endian, 24-bit integers for TrueHD and MLP,
32-bit floats for AC-3 and E-AC-3. The bytes are the bytes of the file.

`--format wav` writes a classic RIFF header and then the same samples. The
`fmt ` chunk is the one a WAVE file of the decode has: extensible, with the
channel mask. The length of the stream is not known when the header goes out,
and a pipe cannot be gone back over, so the `RIFF` size and the `data` size are
both `0xFFFFFFFF`, which says "unknown length", and the `data` chunk runs to the
end of the stream. There is no `JUNK` chunk to become `ds64` and no RF64. The
header is 68 bytes; a WAVE file of the same decode has a longer one, and its
samples are the same bytes.

Not every program reads such a header. FFmpeg and SoX take the data chunk as
running to the end of the stream; others stop at four gigabytes, refuse the
file, or take the size at its word. Nothing is promised of any player or
container: a receiver that has to know the length is to be given a file.

`--format damf` and `--format adm` cannot go to standard output. A DAMF set
is three files and an ADM BWF file has its chunks written when it is closed,
both by going back over what was written; `-o -` with either is refused (exit
2) in a sentence that names the format. Both take `-` as their input.

A terminal is not a receiver of samples: `-o -` with standard output on a
terminal is refused (exit 2), as `-` is with standard input on a terminal.

## A layout that changes

The channels or the sampling frequency of a stream can change in the middle
of it: a TrueHD stream at a major sync, an E-AC-3 programme when a dependent
substream comes or goes. A receiver of headerless samples cannot be told that
the width of a sample changed, so a decode to standard output never changes it
silently. `--on-layout-change` says what is done:

- `stop`, which a decode to standard output does when the option is absent:
  the samples end at the unit that changed, the change is said on standard
  error and in the report (`config_change`, with the unit and its byte
  offset), and the run exits 7.
- `fixed`: the output keeps the layout of the first unit where the codec
  offers it. An E-AC-3 programme writes silence in the channels a frame group
  does not carry, as a decode to a file always has. The change is still a
  fault and the run exits 7. A TrueHD decoder has no such layout, and a TrueHD
  stream refuses `fixed` (exit 2).
- `split`, to a file only (`-o -` refuses it, exit 2): the decoder follows
  the change, and the samples after it go on in a file of their own with a
  header of their own. The first part is the file named, `OUT.wav`; the
  second is `OUT.002.wav`, the third `OUT.003.wav`, and so on (`OUT.002`
  where the name has no extension). The report lists them under `parts`, in
  order. A change the decode followed is not a fault by itself, and the run
  exits as the same decode without the change would; what judges it is in
  [`exit-codes.md`](exit-codes.md#a-decode-to-standard-output).

A decode to a file does what it always did when the option is absent: a
TrueHD stream ends its output at the change, and an E-AC-3 one keeps its
layout. An object output refuses the option.

A TrueHD decoder that follows a change decodes the unit of the change once,
with the new configuration, and starts every substream again there, as at
the start of a stream ([`truehd.md`](truehd.md#a-change-of-configuration)).
An E-AC-3 programme that follows one gives each frame group the channels it
was coded with ([`eac3.md`](eac3.md)). Two E-AC-3 streams laid end to end are
not one stream whose layout changes: the decoder of the dependent substream
holds a frame of the first stream back, and at the join a frame group can
come out with a layout neither stream has and a frame that never comes out,
which is a fault. A programme whose dependent substream comes and goes inside
one stream splits clean.

The change is told at the unit the decoder marks. The decoder of an E-AC-3
substream holds a frame back, and the frame group that comes out of it once
the first frame of the new layout is in is the last group of the old one: the
change is told one group early, and with `stop` that group is not written
either. A stream cut inside its last frame group, which loses the dependent
substream of that group, has changed its channels by the same count.

## A receiver that closes the pipe

A receiver may stop reading before the samples end, as `head -c` does. The
decode then stops writing, says in one line on standard error how many bytes
went out, and exits **141**, the status a shell gives a process that `SIGPIPE`
ended: a script that already lets `head` close a pipe early lets `oadec` do it
too. What the receiver read is a correct beginning of the samples. Windows
reports the closed pipe in more than one way depending on who held the other
end (`ERROR_BROKEN_PIPE`, `ERROR_NO_DATA`, `ERROR_PIPE_NOT_CONNECTED`); each
of them is the same end of the run, and none is a panic or exit 2.

That is what was measured under Git Bash and with the pipes a program makes
for the processes it starts. Under the native pipeline of PowerShell 7 (7.6
measured) a receiver that closes early never reaches `oadec` as an error of a
write: PowerShell stands between the two programs, says itself that the pipe
is being closed (`ResourceUnavailable`), gives no exit code, and may leave
`oadec` blocked on a pipe it neither reads nor closes. It does the same to
any program, Git's `cat` among them. `cmd.exe` was not tried. Nothing is
promised of a shell that was not measured.

## How the decode is laid out

One thread reads the stream into pieces of 64 KiB and puts them in a queue of
16 pieces. When the queue is full the reader waits, so a receiver that reads
slowly holds the reading back and the memory of the run does not grow with
the stream. The decode takes the pieces in turn and frames them into access
units or syncframes. Whenever the queue is empty, what is waiting to go to
standard output goes out before the decode waits for more.

What tells the codecs apart is the first 64 KiB of the stream, all of it where
the stream is shorter: what a decode of a file reads of its head to do the
same. It is decoded as the rest of the stream is, once the codec is known.

A TrueHD access unit goes to the decoder and to the checks of `verify`, which
run on a thread of their own, in batches of 64 units and 1 024 units behind
the decode at most: the checks
are made on the units the decode reads, and on no copy of the stream. A unit
is told when its checks are in: its findings first, then what the decode said
of it, then its samples. So the samples, the report and the order of its
findings are the same on every run, whatever the threads did.

An AC-3 or E-AC-3 stream is checked by the pass that decodes it, as a decode
of a file has always been. Its syncframes are cut from the bytes as the reader
of a file cuts them, the reads of four megabytes it would make included, so
the frames, the bytes skipped and the sync errors are the ones `verify`
counts in the same bytes; nothing waits for those reads that does not depend
on them.

## Every presentation in one pass

`decode -p all -o OUT.wav` decodes every presentation a TrueHD stream
carries, each to a file of its own: `OUT.p0.wav`, `OUT.p1.wav`, and so on
(`OUT.p0` where the name has no extension). A presentation that is a copy of
the one under it is not written: it is that one. Each file holds the bytes
`decode -p N -o` of its presentation writes, and each presentation is judged
as that decode judges it, with its summary and its verdict on standard error;
the run exits with the worst of them. The substreams are decoded once for all
of them ([`truehd.md`](truehd.md#several-presentations-in-one-pass)).

It writes samples to files, and conceals what it cannot read, as `decode`
does by default. So it refuses (exit 2): an object output, `-o -`,
`--on-layout-change`, `--start`, `--on-error stop`, `--programme` and the
options of an object output or of an AC-3 or E-AC-3 decode. An AC-3 or E-AC-3
stream has no presentations and refuses `-p` altogether.

With `--json` or `--report` it reads the stream once, as a stream, and
reports every presentation as `decode -p N --json` reports it (*The report
of every member*, below); the files are the same bytes.

## Every programme in one pass

`decode --programme all -o OUT.wav` decodes every programme an AC-3 or E-AC-3
stream carries, each to a file of its own: `OUT.g0.wav`, `OUT.g2.wav`, and so
on, by the id of the programme's independent substream (`OUT.g0` where the
name has no extension). The letter is `g`, for the group of an independent
substream and the dependent substreams after it, because `p` is the
presentation's. Each file holds the bytes `decode --programme N -o` of its
programme writes, from the same stream, and is judged as that decode judges
it; a programme that begins late or ends early is as long as that decode's
file, and a stream of one programme gives the one file `decode` gives.

The stream is read once, from a file or from standard input: its frames are
found once, and each frame is handed once to the programme of the
independent frame it is or follows. Each programme has a decoder of its own,
with its own dependent substreams, dither, overlap, look-ahead, concealment,
layout and counts, so that nothing one programme carries reaches another:
frames of one programme that fail their check leave the others' samples as
they are. What the decode holds is bounded by the programmes and not by the
length of the stream.

It refuses (exit 2): an object output (the object programme is programme 0),
`-o -`, `--start`, `--on-layout-change`, `-p`, the options of an object
output and those of a TrueHD decode; a TrueHD stream has presentations and
refuses it. A programme keeps the layout of its first group, as a decode of
one to a file does. The exit code is the [combined
one](exit-codes.md#every-programme-and-every-presentation).

## The report of every member

`--json` (on standard output) and `--report FILE` of `-p all` and
`--programme all` write one JSON object:

```json
{
  "bytes": 1234567,
  "combined_exit_code": 7,
  "format": "eac3",
  "programmes": [
    { "id": 0, "file": "OUT.g0.wav", "exit_code": 0, "verdict": "clean", "...": "..." },
    { "id": 1, "file": "OUT.g1.wav", "exit_code": 7, "verdict": "faulty", "...": "..." }
  ],
  "stream": "in.ec3"
}
```

`presentations` stands where `programmes` does for `-p all`. Every entry
holds every field of the report of the decode of that member alone, with the
same values, its `parts` naming its file; and `id` and `file`. A member whose
decode could not go on has the verdict `failed`, exit code 2 and its
`error`. The entries are in ascending order of id whatever order the stream
first carries them in, the keys of every object in the order of their names,
and the report holds no time: the report of a stream is the same, to the
byte, on every run.

`verify --decode` does not: it decodes each presentation on a thread of its
own, as it always did, because by the clock that is the faster
([`truehd.md`](truehd.md#several-presentations-in-one-pass)).

## Choosing a programme

`--programme N` on `decode`, `compare`, `info` and `verify` chooses the
programme of an E-AC-3 stream by the id of its independent substream; without
it the programme is 0, as before. A decode to standard output takes it too. A
programme the stream does not carry ends the run with exit 2 and the ids it
does carry, and nothing is written. A TrueHD stream and an object output
refuse the option (exit 2).

## Decorrelating the coupled channels

`--decorrelate` on a decode to samples, to a file or to standard output,
turns on the experimental decorrelation of the coupled channels of an E-AC-3
programme that is its independent substream alone ([`eac3.md`](eac3.md),
"Decorrelation of the coupled channels"). It is off by default. Its weight
is this decoder's estimate fitted on measurements, and a decode with it is
not claimed to equal Dolby's. With it, each independent frame waits for the
frame after it before it is decoded, so a decode to standard output writes
each frame group one frame later; what is written, the findings and where
they are, and the verdict are otherwise those of a decode without it. The
summary says what it did (`Decorrelation:`), and the report of `--json` and
`--report` says it in `decorrelation`: the programme, and for each outcome
whether it was applied, the reason where it was not, and the frames. An
object output, a TrueHD stream and `-p all` refuse it (exit 2).

## In a program

A program that embeds the engine has the same decode in
`oadec_engine::stream`: a `StreamDecoder` that is pushed bytes in pieces of
any size and writes into a `StreamSink`, and `decode_reader`, which reads a
reader on a thread of its own with the queue above.

The reports of a stream read once are `api::{info_from, verify_from,
oamd_from, emdf_from}`, beside the calls of a `Source`: each takes a name and
a reader. What they stand on is `oadec_engine::pipeline`: a `Pass` reads a
reader once and lends every unit to the `Stage`s it is given, in their order.
A unit is lent for one call and a stage copies what it keeps; the compiler
holds it to that.
