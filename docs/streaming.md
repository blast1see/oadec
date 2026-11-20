# Decoding a stream as it arrives

`oadec decode` reads its stream from standard input when the stream is named
`-`, and writes its samples to standard output when the output is named `-`:

```text
mkvextract tracks input.mkv 1:/dev/stdout | oadec decode - --format wav -o - | ffplay -
```

The stream is read once, as it arrives. Nothing of it is held in memory but
the units being decoded, a queue of a megabyte between the reader and the
decode, and the 64 KiB at most that tell the codecs apart. The first samples
are on standard output as soon as the first units have decoded, long before
the stream has ended.

> **Status.** This page describes the decode the command line is being given.
> Until it is built, `-o -`, `--json`, `--report` and `--on-layout-change stop`
> on an E-AC-3 stream are refused with exit 2 and a message that says so.

## What goes where

With `-o -`, standard output carries the samples and nothing else: no
summary, no warning, no progress line, no verdict. All of that goes to
standard error, as it does for a decode to a file. What an AC-3 or E-AC-3
decode to a file prints on standard output, the report of its pass, goes to
standard error too.

`--json` replaces the summary with a report in JSON. It is printed on
standard output for a decode to a file, and on standard error, as its last
line, for a decode to standard output. `--report FILE` writes the same report
to a file instead, whatever the output is. `--report -` names standard output,
and `-o -` refuses it (exit 2): the two cannot share a pipe.

The report holds no time and no speed, so the report of the same stream is
the same file on every run:

| Field | What it is |
|---|---|
| `stream` | the name of the stream: its path, or `<stdin>` |
| `format` | `TrueHD`, `MLP`, `AC-3` or `E-AC-3` |
| `output` | `format` (`pcm` or `wav`), `channels`, `rate`, `mask`, `sample` (`s24le` or `f32le`) |
| `samples` | samples written, per channel |
| `events` | in the order of the units: `{"event": "opened", ...}`, `{"event": "config_change", "unit", "byte_offset", "what", "policy"}`, `{"event": "finding", "unit", "byte_offset", "rule", "message"}` |
| `findings` | the lines the verdict rests on, as `integrity:` says them on standard error |
| `verdict` | `clean`, `lossy` or `faulty` |
| `exit_code` | 0, 4 or 7 |

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
2) in a sentence that names the format. Both read their stream from standard
input, as a stream.

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

A decode to a file does what it always did when the option is absent: a
TrueHD stream ends its output at the change, and an E-AC-3 one keeps its
layout. An object output refuses the option.

## A receiver that closes the pipe

A receiver may stop reading before the samples end, as `head -c` does. The
decode then stops writing and reading, says so in one line on standard error,
and exits **141**, the status a shell gives a process that `SIGPIPE` ended: a
script that already lets `head` close a pipe early lets `oadec` do it too.
What the receiver read is a correct beginning of the samples. Windows reports
the closed pipe in more than one way depending on who held the other end;
each of them is the same end of the run, and none is a panic or exit 2.

## How the decode is laid out

One thread reads the stream into pieces of 64 KiB and puts them in a queue of
16 pieces. When the queue is full the reader waits, so a receiver that reads
slowly holds the reading back and the memory of the run does not grow with
the stream. The decode takes the pieces in turn, frames them into access
units or syncframes, and hands every unit to the checks of `verify` and to the
decoder alike: the checks are made on the units the decode reads and on no
copy of the stream. The checks of a unit are said, and its samples written,
in the order of the units, so the samples, the report and the order of its
findings are the same on every run.

What tells the codecs apart is read before the first unit is decoded and is
then decoded as the rest of the stream is: at most 64 KiB, and as soon as an
AC-3 sync word or a chain of TrueHD access units has said which codec it is.

A program that embeds the engine has the same decode in
`oadec_engine::stream`: a `StreamDecoder` that is pushed bytes in pieces of
any size and writes into a `PcmSink`, and `decode_reader`, which reads a
reader on a thread of its own with the queue above.
