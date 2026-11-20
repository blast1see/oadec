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
| `decode --format damf`, `decode --format adm` | taken into memory whole, then decoded |
| `info`, `verify` (with and without `--decode`), `emdf`, `oamd`, `au`, `compare`, `eac3-blocks` | taken into memory whole, then read as often as the command reads a file |

The commands of the second and third rows read their stream more than once,
on more than one thread, and a pipe can be read once; they hold standard input
in memory for as long as they run, as they always have. Reading them as a
stream is a later round of work.

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
does by default. So it refuses (exit 2): an object output, `-o -`, `--json`,
`--report`, `--on-layout-change`, `--start`, `--on-error stop`, `--programme`
and the options of an object output or of an AC-3 or E-AC-3 decode. An AC-3
or E-AC-3 stream has no presentations and refuses `-p` altogether.

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

## In a program

A program that embeds the engine has the same decode in
`oadec_engine::stream`: a `StreamDecoder` that is pushed bytes in pieces of
any size and writes into a `StreamSink`, and `decode_reader`, which reads a
reader on a thread of its own with the queue above.
