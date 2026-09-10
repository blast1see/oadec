# Exit codes

```text
0   the input decoded and nothing was found wrong with it
2   usage error, I/O error, unsupported input, or a decode that could not continue
7   an integrity fault was detected that affects what was delivered
```

Every command uses the same three, and every command that delivers audio or
metadata decides between 0 and 7 with the same list of faults that `verify`
uses. That was not true before: `verify` read every counter and exited 7, while
`decode` re-derived a narrower rule of its own and exited 0 while printing the
very CRC failure it had just found, and the object path read no integrity flag
at all.

## What counts as a fault

Anything that makes the delivered output untrustworthy:

- a frame that would not decode;
- a failed frame CRC, access-unit parity, substream CRC, restart-header CRC or
  lossless check word;
- a sync error, a resynchronisation, or a byte of the file that was skipped or
  left trailing;
- an Object Audio Metadata or JOC payload that would not parse, or whose
  declared size was wrong;
- a dependent substream that was seen and whose channels did not reach the
  output, an unreadable custom channel map, a dependent substream misaligned
  with its independent one, or a channel layout that changed mid-stream.

Two things are deliberately **not** faults. A second programme in the bit stream
is legal (clause E.2.8.3) and skipping it is what a decoder should do, so it is
printed and not counted. And a frame that ends inside its own tail is out of
spec but decodes to audio that FFmpeg, Dolby and oadec all agree on, so it is
counted and reported without changing the verdict of a decode; `verify` still
calls the file non-conformant, because that is what `verify` is for.

## What a fault does

The output is still written. Throwing away audio that decodes is the wrong
trade, and it is the trade this project already declined once, over a frame in
*The 400 Blows* that three decoders agree on. What changes is that the run says
what it found and does not exit clean:

```text
integrity: 1 CRC failures
first problem: frame 22: CRC failure
the output was written and is not trustworthy; `oadec verify` reports the same faults
```

A clean decode prints none of this. There is no flag to turn the verdict off:
if a permissive mode is ever wanted it should be asked for explicitly, and
until then a script that wants the audio regardless can ignore the status.

## Which commands return which

| Command | 0 | 7 | 2 |
|---|---|---|---|
| `verify` | clean | non-conformant | could not be read |
| `info` | always | — | could not be read |
| `emdf`, `oamd` | clean | non-conformant | could not be read |
| `decode` (PCM, WAV, CAF, DAMF, ADM, objects) | clean | a fault above | fatal decode failure |
| `compare` | matches and clean | differs, or a fault above | could not be read |
| `thd-demux`, `eac3-joc-config`, `eac3-ecpl-inject` | always | — | could not be written |

`compare` folds the two questions together on purpose: a comparison against a
reference is only worth reporting if the decode under it was sound, and the
printed result says which of the two failed.
