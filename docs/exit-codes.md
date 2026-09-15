# Exit codes

```text
0   the input decoded and nothing was found wrong with it
2   usage error, I/O error, unsupported input, or a decode that could not continue
4   the object output was written but does not carry the whole programme (a declared loss)
7   an integrity fault was detected that affects what was delivered
```

Every command uses the same codes, and 4 belongs to the object outputs alone
(below). Every command that delivers audio or metadata decides between 0 and 7
with the same list of faults that `verify` uses; `emdf` and `oamd`, which walk
only the metadata they report, run the checks of `verify` over the stream
beside their walk and take its verdict with their own. That was not true
before: `verify` read every counter and exited 7, while `decode` re-derived a
narrower rule of its own and exited 0 while printing the very CRC failure it
had just found, and the object path read no integrity flag at all.

## What counts as a fault

Anything that makes the delivered output untrustworthy:

- a frame that would not decode;
- a failed frame CRC, access-unit parity, substream CRC, restart-header CRC or
  lossless check word;
- a sync error, a resynchronisation, or a byte of the file that was skipped or
  left trailing;
- an Object Audio Metadata or JOC payload that would not parse or whose
  declared size was wrong, or a JOC payload whose `joc_ext_config_idx` is
  reserved (the matrices of the previous frame are held in its place);
- in a substream that carries EMDF, an E-AC-3 frame whose skip fields hold no
  container that opens, whether it is erased, broken, or declares a length its
  syntax disagrees with;
- a TrueHD extra-data block whose header parity, padding, parity byte or
  length is wrong, or whose Evolution container will not open;
- a sample the decoder produced beyond 24 bits, which the writer can only
  clamp;
- a configuration that changed at a major sync after the output began: the
  output ends at that access unit, its files are finished, and the run says
  where it stopped;
- a dependent substream that was seen and whose channels did not reach the
  output, an unreadable custom channel map, a dependent substream misaligned
  with its independent one, or a channel layout that changed mid-stream;
- a substream frame repeated inside one frame group, or a substream that
  stopped supplying frames and was flushed so that the groups after it could
  be delivered, leaving a group without its frame.

Three things are deliberately **not** faults. A second programme in the bit
stream is legal (clause E.2.8.3) and skipping it is what a decoder should do,
so it is printed and not counted. A frame that ends inside its own tail is out
of spec but decodes to audio that FFmpeg, Dolby and oadec all agree on, so it
is counted and reported without changing the verdict of a decode; `verify`
still calls the file non-conformant, because that is what `verify` is for.
And an EMDF payload configuration outside TS 103 420 Table 56, or a
`complexity_index_type_a` that disagrees with the object total of the Object
Audio Metadata, changes neither the audio nor the metadata a decode delivers:
`verify` counts both and exits 7, and a decode of the same file keeps its
verdict.

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

## What the object outputs cannot carry

`decode --format damf|adm` maps the programme into formats that cannot hold all
of it. Every such mapping is counted in a loss ledger where it happens and
printed once per run, one line per class:

- **profile reductions** -- the Dolby Atmos master ADM profile has no field:
  the interpolation length is fixed at 250 samples, an active object's
  importance, a bed event, a screen reference and a trim bypass are not
  written. Dolby's own converters drop them the same way. Printed, exit 0.
- **not representable in DAMF or the ADM profile** -- distance, divergence,
  warp mode, trim configurations. Printed, exit 0.
- **approximations** -- oadec's own choice where the formats leave room:
  differing size axes written with the width, a first event held from sample
  0, an event superseded at the same sample, an event beyond the programme
  end. Printed, exit 0.
- **written with loss** -- something is missing or outside the profile at the
  user's request or because the input was anomalous: ISF elements dropped
  (`--isf drop`; the default refuses such a programme with exit 2), an
  out-of-order event written as delivered in DAMF, a programme that is not at
  48 kHz written as ADM (`--adm-allow-non-profile-rate`; the default refuses
  with exit 2). Printed, **exit 4**.

`--adm-interpolation real` is not a loss and does not change the code: it
writes the stream's own ramp lengths in full, which puts the file outside the
Dolby profile. The run says so on stderr, the `dbmd` tool string carries the
mark, and the file cannot take `--dolby-origin-tag`.

`--loss-report FILE` writes the same ledger as JSON. An integrity fault still
wins: a run that is both faulty and lossy prints both and exits 7. A run whose
only losses are profile reductions prints them and exits 0, because that is
what the format is.

## Which commands return which

| Command | 0 | 7 | 2 |
|---|---|---|---|
| `verify` | clean | non-conformant; with `--decode`, also a failed lossless check word or a presentation whose decode stopped | could not be read |
| `info` | the stream was read | — | could not be read, or holds no stream |
| `emdf`, `oamd` | clean | non-conformant: a fault `verify` finds in the stream, or one in the metadata they read | could not be read, or holds no stream |
| `decode` (PCM, WAV, CAF, DAMF, ADM, objects) | clean | a fault above | fatal decode failure, no access unit or whole syncframe in the file, a configuration change before any output, or a JOC measurement override on a decode with no JOC reconstruction to apply it to |
| `decode --format damf\|adm` with a declared loss | -- | 4 (above) | -- |
| `compare` | matches and clean | differs, or a fault above | could not be read |
| `thd-demux`, `eac3-joc-config`, `eac3-ecpl-inject` | always | — | could not be written |

`compare` folds the two questions together on purpose: a comparison against a
reference is only worth reporting if the decode under it was sound, and the
printed result says which of the two failed.
