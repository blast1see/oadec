# Engine round, 2026-11

After 0.8.0 the owner asked eight things of oadec: how far its verification can
be taken; what of the TrueHD syntax has no material to test it on; fuzzing and
property tests; what decoding costs, measured, for TrueHD at 2, 6, 8 and 16
channels and for the rest; how close the JOC decode is to Dolby's decoder; a
wide matrix of round trips through DAMF, ADM and Dolby's encoders; a current
Rust; and one engine under TrueHD, MLP, AC-3, E-AC-3 and JOC. Another decoder
does some of this, and none of its code was to be ported. This report is what
was built, how it was measured, and what is still open. Every number comes
from a stored measurement under `evidence/engine-round-2026-11/`, cited by its
number.

The plan had 26 pull requests. The work landed in 57,
[#12](https://github.com/blast1see/oadec/pull/12) to
[#68](https://github.com/blast1see/oadec/pull/68), merged between 2026-11-11
and 2026-11-16. They are small on purpose: a comparison that says "no byte
moved" is worth something only when it can be laid to one change.

## 1. Verdict

| Question | Answer |
|---|---|
| A current Rust | Pinned to 1.99.1 in `rust-toolchain.toml`, the locks updated with it, CI on the pinned release. Between the two compilers no exit code, no reported field and no delivered byte moved, on 90.6 GB of deliveries (01). |
| A net under every change | Built first, and shown to fail when it should (02): a transcript of every command on every fixture; a comparison of two binaries over the corpus that reads every report and hashes every delivery (19, 23); a mutation run that fails a mutation it can no longer apply. 95 mutations became 607. |
| TrueHD syntax with no material | Counted, then written. The count names what real streams use and what none carries (05) and which lines of the decoder nothing in CI reaches (06). The writer, a crate that shares no code with the decoder, makes 51 TrueHD and MLP streams with 65 presentations, each recorded only after two outside decoders gave its samples (10, 11, 12, 15), and AC-3 frames whose delta bit allocation moves the masking curve (16). Error protection is the part still without a referee. |
| Fuzzing and property tests | Eleven libFuzzer targets over every parser and decoder, weekly and on pull requests that touch them (07), and on Windows too (14); a seeded fuzz run inside `cargo test`; a repair of checksums, so that a change behind one reaches the code behind it (09, 25); and a campaign that puts changed streams to every command and holds the commands to one verdict (22). They found five faults, none of them a panic (section 3.3). |
| Verification depth | The 26 checks of an AC-3 family stream are named rules with a source, a severity and a place, as the 46 of TrueHD were (26), with `--fail-on` and `--max-per-rule` (27): 84 rules in all. One walk finds a container where six loops did (32). What containers, Evolution frames, reserved bits and major syncs declare is counted, and ruled at `info` where no real stream has ever set it (31, 33, 34, 35). An option with nothing to act on is refused (30). Keyed protection is not verified, by decision (section 2). |
| What decoding costs | Measured from outside the process (08), and the first kernel table corrected (38). From end to end, the binary of the baseline beside the binary at the end of the round: of 88 commands 77 are faster, 11 within the spread of their runs and none slower. A decode of the channels of an AC-3 family stream takes 26 to 39 per cent of the time it took and `verify` of one 19 to 25; the objects out of JOC take half; a TrueHD presentation takes 86 to 99 per cent, and `verify --decode` of a TrueHD stream 32 to 53 (57). |
| JOC against Dolby's decoder | The distance was in one place, the lowest subband: 55.9 dB at the median of 127 tracks where the bands above stood at 71.4 (48). With Dolby's operator on that subband, 21 complex taps, it stands at 115.2 dB (49); with the clip gain put back where Dolby's decoder puts it, the fourteen tracks whose gain changes stand at 126.4 dB there (50); with the dither Dolby's decoder draws, the objects stand at 104.2 dB over the whole band where they stood at 52.3 (51). What is left is a decorrelation Dolby's decoders add to the coupled channels of an E-AC-3 stream, described and not implemented (52). |
| Round trips | 51 scenes, a pairwise design over seven axes, through Dolby's encoders and back by five paths: every master is taken, every one of 269 streams is clean, and the two decoders make the same elements of every E-AC-3 one. What sounds alone comes back whole; what a master says of an object besides its place is in no stream (54). |
| One engine | `oadec-engine` reads, checks and decodes all four syntaxes, names no command-line type and prints nothing; the command line is one caller of it (55). Five calls at its root have one shape whatever the stream, and errors of a type of their own (56). |
| Where the text and Dolby's decoder part | The owner's rule for the round: follow Dolby's decoder. Done for a matrix that sends a bypassed bit (13, 15), a `reuse` after a `none` (18), enhanced coupling from its lowest band (28), the place of a pre-noise correction (29), reserved bits (34), and the three of the JOC row above. Not done for the decorrelation (52). |

## 2. How it was done

**Sources.** Dolby's *TrueHD (MLP) High-Level Bitstream Description*, ETSI TS
102 366 and TS 103 420. FFmpeg and truehdd were run as programs and read for
facts, cited where they are used. No code, rule name or message came from
either.

**Dolby's tools as black boxes.** The Dolby Encoding Engine, the Dolby Media
Encoder, the Conversion Tool and the Reference Player's decoders were given
material and their output was read: files, exit codes, and the text they print.
Nothing was decompiled or disassembled, and nothing of a vendor is in the
repository: no binary, no template, no log. Another vendor's encoder was used
the same way for two of the streams of evidence 29.

**What was not attempted.** The protection words of EMDF containers and
Evolution frames are keyed, the computation is not in the standard (TS 102
366, clause H.2.2.4.3) and the key is not public. No key was sought or
derived, and no protection word is computed or written anywhere here. What a
stream declares about its protection is counted without a key (31, 33). Two
things follow, and the matrix says both: a synthetic object presentation
cannot be put to Dolby's decoder, and a JOC payload rewritten in place cannot
either, so rare JOC modes were looked for by having the encoder write them
(54) and not by writing them.

**The corpus.** 136 streams at the end of the round, 125 at its start: heads
of about a minute of TrueHD, AC-3 and E-AC-3 tracks, clips, encodes made for
a measurement, and the ten committed fixtures. Counts over "the library" are
of one-minute heads of a larger set, 157 TrueHD tracks and 1 260 tracks of the
AC-3 family. A head does not exclude what an encoder does once an hour, and
the evidence says so where it matters (05, 31, 33). Of both sets only the
fixtures are in the repository.

**Gates on every pull request.** Format, lint with warnings denied, the test
suite on Linux and Windows, `cargo deny`, and that every mutation still finds
its text. On anything that could move a report or a delivery: the previous
binary and the new one over the corpus (`tools/old_new.py --decode-all`), the
campaign of changed streams against the tally of main, the media suite and
`tools/media_regression.sh`. A new rule was counted over the corpus before it
was given a severity. A claim without a unit test has an evidence file with
the command that made it.

**Where the plan was left.** The 26 planned changes were cut into 57. Four
things the plan named were not done: a row for a DaVinci Resolve import of
this decoder's ADM file, which needs the program run by hand; a side report
of the programme metadata a DAMF set cannot carry, for the encoder's command
line; a census of whole library tracks in place of their heads; and the
modules of the engine under new names, which were moved as they stood so that
the comparison before and after could be exact (55). Section 9 says what became
of the four after the round. One thing was added that
the plan did not have: reading what Dolby's AC-3 family decoder does where it
parts from the standard (28, 29, 51, 52), which the JOC work turned out to
rest on.

## 3. Step by step

### 3.1 Ground (01, 02, 03)

The toolchain is pinned, and raised by a pull request of its own. The three
gates of section 2 were each run against a state they must fail: a command
that says something else, a binary that delivers other bytes, a mutation
whose text is gone. Each failed with a message that names the place (02). The
command line became a library with a binary in front of it, with nothing
moved (03), which is what put its insides in reach of a test.

What the gates do not see is stated with them: the transcripts hold the
fixtures only, and a mutation nobody listed is not looked for.

### 3.2 What nothing tested (05, 06)

`verify --census` counts every field of the TrueHD syntax a stream uses. Over
157 library tracks every one uses FIR prediction, the three code books,
Huffman offsets and matrixing; 149 use IIR prediction with coded states and
99 bypassed bits. What no stream carries is narrow: error protection, a
coefficient on a noise channel, heavy DRC, a new guard byte, a negative
output shift, and every bit of `flags` but one. Two fields the description
reserves are in ordinary use, the fourth bit of a directory entry in 12 per
cent of 42 million entries, so a rule against either would have failed the
library: the count came before the rule (05).

Sorted by what runs them, two thirds of the decoder's lines are run by tests
and by real streams both, and 321 by real streams alone, 86 of them in the
AC-3 core and 7 in the TrueHD core (06). That count is why AC-3 material was
written too.

### 3.3 Fuzzing, repair and the campaign (04, 07, 09, 14, 22, 25)

The fuzz targets assert more than "no panic": no samples or offsets beyond
what the format allows, and the same answer by two routes that are meant to
agree. A seeded run of the same targets is part of `cargo test`, over 6.4
million changed copies of the fixtures when it was first measured; libFuzzer
runs them weekly, and from the same seeds reaches two to three times the
edges on the container, metadata and AC-3 targets (07). With the toolset's
sanitizer component the targets link and run on Windows, so a finding can be
narrowed on the machine the decoder is written on (14).

A checksum used to stop every change behind it. `oadec repair` computes the
checksums of a TrueHD unit and of an AC-3 family frame; on 9.1 million access
units and 143 thousand syncframes of real material it has nothing to change,
which is a stronger statement about those checksums than reading them (09).
It leaves the lossless check wrong on purpose and does not touch a protection
word.

The campaign changes a stream, repairs it, and puts it to every command
through the calls of section 3.5. It holds the commands to the policy of the
exit codes, clause by clause: what one command refuses the others refuse, and
a stream `verify` passes is one no delivery fails on. After the one
disagreement it found was put right, 400 000 rounds over 86 streams and all
2 194 352 streams one bit away from them end without a panic and without a
clause broken (22).

What these found, none of it a panic:

- Three faults of the extractor, two of them in what `verify` reports (07).
  The one that mattered: after refusing a unit with a major sync, its search
  for sync found the unit it had just refused. Of 1 536 damaged copies of a
  stream, 205 handed out a unit read from the wrong place and 125 gave up
  units a correct search delivers (04).
- A repair that reported a change on every run: a segment so short that two
  checksums claim the same bits (25).
- `verify` called clean a stream on which an object decode exits 7 for a
  payload it cannot use; `verify` makes the refusals of the object outputs
  now (22).

No stream of the corpus has any of these, which is why it took changed
streams to find them.

### 3.4 Streams written from recipes (10, 11, 12, 13, 15, 16, 18)

`oadec-synth` writes a stream from a recipe and says what its samples are. It
does the decoder's arithmetic a second time from the description and depends
on none of the decoder's code. That alone would be worth little, one hand
having written both, so a recipe is recorded only when two decoders that share
nothing with this one give its samples: FFmpeg, truehdd and Dolby's decoder,
run as programs. 51 recipes with 65 presentations are on that record (15):
prediction of every order, the three code books, restart headers, six
sampling rates, matrices with noise channels, bypassed bits, output shifts,
two and three substreams.

What the written streams showed that real ones cannot:

- A matrix with both a quantiser step and a bypassed bit is cut by FFmpeg and
  truehdd and left uncut by Dolby's decoder, on every sample asked. No real
  stream has one: none of 242 812 003 matrices in force in 55 streams. This
  decoder cut, because two decoders agreeing was the test, and follows
  Dolby's now (11, 13, 15).
- FFmpeg keeps its filter histories across a restart header; truehdd holds a
  segment to three blocks where the description allows four; the parser in
  front of Dolby's decoder loses access units of ten and twenty bytes. Each
  is a fact about one decoder, and no real stream meets any of them (10).
- An output shift below nought, guard bits and the update of the heavy
  dynamic range control each have one meaning two outside decoders agree on,
  and are recipes now (12).
- The library holds no track that applies a delta bit allocation, so written
  frames are the first material that tests one. An applied delta is read
  alike by this decoder, FFmpeg's and Dolby's, at three sampling rates, and a
  stream that keeps its delta to itself decodes in none of them (16). After a
  `none`, a `reuse` applies the stored segments in FFmpeg's decoder, as the
  pseudo code has it, and keeps `none` in Dolby's, as the table of the field
  has it; this decoder follows Dolby's (18).

The mutations measure what the streams added. Of 11 mutations of the TrueHD
arithmetic 3 passed the suite before the recipes, and one of them shows what
the format's own check is worth alone: a wrong sign moves an even number of
samples by one, the eight-bit check word cancels, and only the samples see it
(10). Of 18 mutations of the AC-3 path 15 survived before the written frames
and none after (16).

### 3.5 Sources, reports and calls (17, 19, 20, 21, 23)

A stream is read from a source that opens again, a file or bytes in memory
(17), or from standard input (21). Every command is a function that hands
back what it found and a method that prints it, and the commands are offered
as calls that print nothing (20). This is what let the campaign run inside
the test process.

The comparison of two binaries was itself found short twice. It did not read
the text of three reports, so a binary that printed two of them otherwise
passed as unchanged on ten streams of ten; the sentence in evidence 17 that
said those reports had been compared was wrong when it was written, and is
corrected there (19). And with a third of its streams deleted under it, it
ended on "0 changed" (23). Both are tests of the tool now.

### 3.6 Rules (24, 26, 27, 30, 31, 32, 33, 34, 35)

What a finding is moved to a crate of its own (24), and the 26 counters of an
AC-3 family pass became 26 rules without a verdict or an exit code moving:
a rule fired as often as its counter counted in every pass of the suite, of
37 121 rounds of the campaign and of the corpus (26). Before that no test made
14 of the 26 checks fire at all.

New rules were counted before they were given a severity, and most have never
fired on a real stream: over 79 598 containers of the corpus and 688 011 of
the library none changes its key identifier, its word length or its version
(31); the same of 227 395 Evolution frames (33); no reserved bit is set in
71 916 major syncs and 282 184 restart headers (34); no stream says its delay
is constant (35). They stay at `info`. The count also found that this decoder
did not ignore every reserved bit, as the description says a parser should: a
major sync that differed from the first in one was a change of configuration
to `verify`, and in one field it stopped a decode, where Dolby's decoder and
FFmpeg play the copy as they play the original (34).

`decode` and `compare` took 77 combinations of options that had nothing to
act on, and did nothing with them; each is refused in a sentence now (30).

### 3.7 The AC-3 family against Dolby's decoder (28, 29)

Two places where the standard's pseudo code and Dolby's decoder part were
read from the decoder's output and followed. Where enhanced coupling begins
at its lowest sub-band, 870 of 2 879 frames of four streams ended in a
mantissa code that does not exist when the bit allocation was read as
printed; with two rules the clause does not print, none does, and below
1.2 kHz a median frame is within 88.8 dB of Dolby's decode (28). A transient
pre-noise correction sits one block later than this decoder had it, with
raised-cosine windows: 21.8 to 35.5 dB below the reference over the corrected
samples, against 3.2 to 6.2 where it was (29).

### 3.8 What decoding costs (08, 36 to 47, 53)

`tools/bench.py` measures a command from outside: wall time, processor time
and peak memory, the median of five runs. The baseline said where the time
was not: a 16-channel TrueHD presentation decoded at 63 to 73 times real
time in 30 MiB, and both reference decoders took longer. It was in the AC-3
family, where FFmpeg decoded the same frames 4.7 to 6.8 times faster, and in
the objects out of JOC, at 26 to 42 times real time (08).

Each change is a pull request with its own measurement inside one binary,
and none moved a sample unless it says so:

| Change | Evidence | What it takes of the time it took |
|---|---|---|
| Syncframes read with a cursor, the buffer not moved at every frame | 36 | 38 to 66 per cent, channels; 84 to 89, objects |
| The AC-3 core: an allocation kept, a record on demand, a block decoded in place | 37 | 74 to 83 per cent |
| Checksums eight bytes at a step | 39 | `verify` 88.8 to 97.0 per cent |
| A TrueHD matrix that stands still not multiplied by its deltas | 40 | object presentation 88.4 to 93.8 per cent |
| An AC-3 family stream read and not listened to | 41 | `verify` 45.8 to 59.4 per cent |
| The scan and the presentations of `verify --decode` side by side | 42 | 33.0 to 55.0 per cent |
| The inverse transform, real and imaginary parts apart | 43, 46 | PCM delivery 77.2 to 83.4 per cent |
| The JOC filter banks the same way | 44 | object delivery 81.5 to 83.0 per cent |
| The rows of an object delivery in one buffer | 45 | 83.5 to 85.0 per cent |
| The JOC synthesis bank in a ring | 47 | object delivery 87.1 to 89.3 per cent |
| The level of the core printed where the samples are made | 53 | `verify` 68.2 to 77.0 per cent on the three streams that have it |

What the changes come to together was measured last: the binary the baseline
was measured with beside the binary at the end of the round, turn and turn
about over eighteen streams with nothing else running (57).

| Kind of work | Streams | × real time at the baseline | × real time at the end | Of the time it took |
|---|---|---|---|---|
| AC-3 and E-AC-3, the channels to PCM | 8 | 82.6 to 308.3 | 255.8 to 1175.8 | 26.2 to 38.6 per cent |
| AC-3 and E-AC-3, `verify` | 8 | 84.2 to 314.6 | 377.8 to 1657.7 | 19.0 to 24.6 per cent |
| E-AC-3 with JOC, the objects to DAMF and to ADM | 2 | 26.5 to 40.9 | 54.2 to 83.2 | 48.8 to 50.3 per cent |
| TrueHD, the presentations of 2, 6 and 8 channels to PCM | 10 | 103.4 to 526.8 | 107.1 to 557.1 | 86.1 to 98.7 per cent |
| TrueHD, the object presentation of 12, 14 and 16 channels to PCM | 6 | 68.4 to 93.7 | 79.3 to 107.3 | 85.9 to 88.6 per cent |
| TrueHD, `verify` | 10 | 140.4 to 540.1 | 157.6 to 572.0 | 86.6 to 94.4 per cent |
| TrueHD, `verify --decode` | 10 | 31.3 to 131.7 | 82.8 to 250.8 | 31.6 to 52.5 per cent |
| TrueHD, the objects to DAMF and to ADM | 6 | 42.9 to 82.7 | 52.0 to 96.9 | 51.7 to 114.8 per cent |

Of 88 commands 77 are faster, 11 are within the spread of their runs and
none is slower. The two binaries do not do the same work: the later one does
more to the objects of a JOC stream (section 3.9) and still takes half the
time, its `verify` of an AC-3 family stream makes no samples, and its
`verify --decode` decodes the presentations side by side, for nine mebibytes
more. The last row is of files written to a disk, and seven of its twelve
figures are within their own spread.

Two of the round's own figures were wrong and are corrected in place. The
first kernel table was one run not bound to a kind of core, and six of its
ten figures were of a slow core (38). And the document said the compiler
makes scalar code of the transforms, having read the assembly of a crate
before link-time optimisation; it takes the butterflies two at a time (46).
`docs/performance.md` ends with what its figures do not say.

Everything here is safe Rust with no new dependency at run time: what was
gained came from work not done twice, memory not moved, and loops the
compiler can vectorise.

### 3.9 JOC against Dolby's decoder (48, 49, 50, 51, 52)

`tools/joc_bands.py` compares the objects of two decoders band by band, with
no code of this decoder's filter bank in it. Of 127 tracks with signal in the
lowest band, 71 had that band more than 10 dB further from Dolby's than the
bands above; a best-fit filter on an object gained 1.9 dB, so the two did not
differ by a filter on their output (48).

- **The operator.** Fourteen streams of both configurations give the same 21
  complex taps for what Dolby's decoder does to the lowest subband of a
  channel that carries the 90-degree phase shift. With them the lowest band
  stands at 115.2 dB at the median, which is where the LFE of those tracks
  stands (115.9), and no object of 1 080 is further from Dolby's than it was
  (49).
- **The clip gain.** Dolby's decoder takes it from one value to the next in a
  straight line over the first 256 samples a frame decodes to, in the
  downmix before the objects are made of it. Applied at the output it brings
  the lowest band to 57.1 dB; in the downmix, to 125.94 (50).
- **The dither.** Dolby's decoder draws its dither from a generator of 16
  bits, with a state for each block and channel. With the same in this
  decoder, a stream without a mantissa bit decodes to Dolby's samples to the
  rounding of a 32-bit float, and the band from 12 to 18 kHz of 45 streams
  stands at 77.2 to 107.8 dB where it stood at -3.0 to 31.4 (51).
- **What is left.** With the dither the same, Dolby's decode of an E-AC-3
  stream still has something in every coupled coefficient: a decorrelation,
  an all-pass along the blocks, set off by what the channels have of their
  own under the coupling frequency. The filter is read to five decimals; the
  rule that picks its pair is not, and this decoder leaves the coupled region
  as the standard has it (52). The objects of a JOC stream are made of the
  plain core and are not touched by it.

### 3.10 Round trips (54)

`tools/interop.py` writes scenes whose every number is known (the scene
author gained a frame rate, a warp mode, locks, zones, gains, sizes and tones
that begin and end), encodes them, decodes the streams and measures what came
back. The design is pairwise over the frame rate, the bed, the number of
objects, how they move, what else is said of them, whether the elements sound
together or in turn, and the encoder's rate: 51 rows of 40 320.

- Every master is accepted, and all 269 streams are clean. Dolby's object
  decode and this one agree at lag 0 on all 51 rows.
- Elements that sound in turn come back each in one element, at its place
  and within 0.22 dB of its level, 296 of 296; every one of 54 moves is
  made, 896 to 3 776 samples late. That axis is what separates what a decoder recovers from
  what an encoder shares between elements that sound together.
- The encoder renders a lock, a zone and a size into the audio and drops a
  gain; the stream carries none of them. The bed of a stream is its LFE; the
  other bed channels come back as objects at their speakers' places.
- The Conversion Tool's ADM and the master it was made from encode to the
  same bytes, 51 of 51, and so do this decoder's DAMF and its ADM of one
  stream. A second generation loses the warp mode unless the decode is told
  to keep it.
- 96 kHz masters are refused by both encoders.

Two first readings were wrong and were caught by a control. Dolby's object
dump is sixteen channels whatever the stream holds, and read as twelve it
gave a plausible wrong answer. And Dolby's TrueHD encoder does not write the
same bytes twice from one master, though the samples are the same, so two
encodes that differ say nothing until the same input has been encoded twice.

### 3.11 The engine (55, 56)

What reads, checks and decodes a stream was inside the crate of the command
line. It is `oadec-engine` now: fifteen modules moved as they stand. It names
no command-line type; the 214 lines its modules printed are written to a
writer the function is handed; a walk over a stream tells a watcher how far
it has got and draws nothing. The help of every command, every report and
delivery of 136 streams and every answer of 200 000 rounds of the campaign
are what they were (55). The lock of the fuzz workspace shows the move from
outside: 36 packages where there were 53.

Five calls at the root of the crate are for a caller that is no command line:
`probe`, `verify`, `decode`, `decode_objects` and `metadata`. The report of
the checks has the same fields whichever family the stream is of, and what a
call could not do is an error of one of four kinds, written in the line the
command line writes. Put beside the command line over 143 streams, the corpus
and the damaged set, the calls name the format the command line names and
every field of the one report is the field of the family's report it is read
from: 2 145 comparisons, none that differs (56). The first run of that
comparison had two that did, which is the last entry of section 4.

## 4. Corrections made in the round

A round that measures also measures itself wrong. Each of these was found by
a control and is recorded where the wrong figure was:

- Evidence 17 said two metadata reports had been compared. The tool did not
  read them (19).
- The kernel table of the baseline (38), and the reading of the compiler's
  output (46, which corrects 43 and 44).
- The round-trip figures first read through a twelve-channel dump, and the
  inference that a frame rate changes a TrueHD encode (54).
- The lock of the fuzz workspace named the command line's crate after the
  engine had left it, and nothing said so; CI resolves it with `--locked`
  now (56).
- The new calls named a damaged MLP stream TrueHD, and their test agreed,
  having taken the name of the file for its format. Five tests passed and 31
  mutations of 31 were caught with the fault in place. A comparison with the
  sync word in the bytes, over the corpus, found it; a stream is named by
  that word now, and the tests read it themselves (56).
- The text of the round trips said the E-AC-3 encoder takes no dialogue
  normalisation on its command line, on the strength of its help, which names
  no way to give one. Asked, it takes one, and writes it (60).

## 5. Found along the way

- Encoders set the reserved fourth bit of a directory entry in 12 per cent of
  42 million entries, and `hires_output_timing` in 40 per cent of restart
  headers (05).
- Every encoder codes the first eight samples after a restart header without
  a filter, 282 022 times of 282 022, which is why a decoder that keeps its
  histories there is never heard (10).
- Dolby's decoder reads five blocks to a segment and a major sync in every
  unit, both past bounds its own description states (12).
- The sequence counter of JOC payloads restarts in 3 of 1 260 tracks, and
  never as a splice: each holds 0 over its first frames and counts on (31).
- 8 streams of the corpus keep one delay through the decoder buffer, and
  none says so (35).
- A megabyte of CRC-16 took 1.81 ms a bit at a time and takes 0.38 (39).
- Dolby's two E-AC-3 decoders agree with each other to 109.7 dB on the
  decorrelation neither the standard nor FFmpeg has (52).
- Dolby's TrueHD encoder varies a reserved bit and a check nibble from run
  to run (54).

## 6. Still open

From `conformance-matrix.md`, which has 131 rows where the round began with
110: 93 PASS, 17 PASS-TOL, 3 PARTIAL, 7 N/I, 6 N/T, 5 UNK and no FAIL.

(A note of 2026-11-21: this section and the count of section 9 are the
matrix as it stood when this round ended. Two rounds later it has 144 rows:
105 PASS, 17 PASS-TOL, 7 N/I, 4 N/T, 3 UNK, 3 PARTIAL, 3 SPEC-REF, 1 NOT MET
and one row that is experimental and not a PASS; still no FAIL. Of what is
listed below, the layout that changes mid-stream, `block_header_crc` and what
`block_data_bits` counts are PASS; JOC configurations 1 and 2, two data
points and the enhanced-coupling angle and chaos are SPEC-REF, held to a
reference written from the specification; the decorrelation of coupled
channels is experimental. The fuzzing row of section 1 says eleven libFuzzer
targets; there are fourteen now.)

- **Not testable with what there is (N/T).** Error protection of a TrueHD
  block, which no stream carries and whose two outside readers disagree (12);
  EMDF in auxiliary data; JOC downmix configurations 1 and 2 and two data
  points, which no encoder here was brought to write (54); the angle and
  chaos of enhanced coupling; and the titles Dolby's decoder would have to
  confirm.
- **Unknown or proprietary (UNK).** `block_header_crc` and what
  `block_data_bits` counts; the limits of the decoders of the 6-, 8- and
  16-channel presentations; the protection words; the `dbmd` chunk.
- **Not implemented (N/I).** A change of layout in mid-stream; DRC and
  dialogue normalisation, by design; DVD-Audio with groups at two rates and
  its decoder buffer; the delta fields the standard forbids, which decode
  without a word (16); the decorrelation of coupled channels (52).
- **Partial.** The bounds of the description Dolby's decoder reads past; the
  HD DVD disc row; object distance, divergence and trim.

Beyond the matrix:

- The library counts of the round are of heads of a minute. One shelf has
  since been read whole (section 9, 62); the others are heads still.
- The round trips used tones, at most twenty objects and no trims (54).
- Dolby's object decoder refuses 89 of 194 object presentations of a
  library, and no field `info` reports separates the two sets. That row
  stands as the round found it.
- FFmpeg still decodes the AC-3 family faster than this decoder, 1.3 to 2.09
  times on eight streams after the transform was rewritten (43): its
  transforms are written for the processor's vector units, which safe Rust
  leaves to the compiler.
- The fuzz campaign is blind where a field has to agree with another before
  the code behind both runs; that is libFuzzer's part, ten minutes a target
  a week.

## 7. The round in figures

| | At the start | At the end |
|---|---|---|
| Pull requests merged | | 57, #12 to #68 |
| Commits | | 218, 161 of them not merges |
| Crates | 7 | 12 |
| Lines of Rust in the crates | 50 302 | 83 896 |
| Test functions in the crates | 433 | 765 |
| Mutations in `tools/mutants.py` | 95 | 607 |
| Rules `oadec rules` lists | 46 | 84 |
| Tools | 41 | 56 |
| Evidence files of the round | 0 | 57, the last with this report |
| Rows of the conformance matrix | 110 | 131 |
| Streams of the corpus | 125 | 136 |

## 8. Reproducing

```text
python tools/old_new.py --old old/oadec.exe --new new/oadec.exe --decode-all \
    --out old-new.json WORK/thd WORK/ec3 WORK/clips crates/oadec-cli/tests/fixtures
OADEC_CAMPAIGN_ITERS=200000 cargo test --release --locked -p oadec-fuzz \
    --test campaign -- --nocapture the_commands_agree_on_every_changed_stream
python tools/mutants.py
oadec verify --census --json stream.thd > stream.json
python tools/census.py stream.json other.json --out census.json
python tools/synth_accept.py --out accept.json
python tools/bench.py --refs --out bench.json WORK/thd WORK/ec3 WORK/clips
python tools/bench.py --compare old/oadec.exe new/oadec.exe --out compare.json WORK/thd WORK/ec3
python tools/joc_bands.py ours.atmos.audio reference.f32 --out bands.json
python tools/interop.py plan
python tools/interop.py run --work WORK/interop --path stream
cargo run --release -p oadec-engine --example calls -- stream.thd stream.ec3
bash tools/media_regression.sh WORK logs
```

The paths are examples; the streams are not in the repository. The tools
that run a vendor's program find it where it is installed and say so when it
is not there; what they read of it is in `docs/dolby-tools.md`.

## 9. What the plan had left, after the round

The owner asked for the four things of section 2 to be decided and acted on.
They are in pull requests of their own, after the 57 of the round.

- **A stream's downmix, which the side report needed first.** What a stream
  says of its downmix to two channels, the one it prefers and four levels,
  was read past: no report had it, and it is one of the things an encoder can
  be told. `info` and `verify` report it now, in both syntaxes. Of 22 streams
  Dolby's three encoders wrote with every value of every one of the five
  options, each says in every frame what its encoder was told (58).
- **The side report, as a tool.** `tools/encoder_arguments.py` writes what a
  stream says of its programme as the arguments Dolby's encoders take. It was
  kept on the condition the plan set, that the round trips show it coming
  back: told nothing, the encoder of a second generation keeps the dialogue
  normalisation of the first stream in 3 of 12 and its downmix and DRC
  profiles in none; told the arguments, all of them in 12 of 12 (60). It is
  not a file the decoder writes: the reports already had what such a file
  would hold.
- **The DaVinci Resolve row.** Run through the program's scripting interface,
  without its window. It takes the ADM file this decoder writes as a Dolby
  Atmos master, and renders it to the samples it renders the Conversion
  Tool's ADM file of the same programme to, 11 programmes of 11. What the
  program says of an import is a label and says no more than that the file
  was taken; the row stands on the renders (59).
- **The names of the engine's modules: left as they are.** They were to
  become `source`, `probe`, `report`, `truehd`, `ac3`, `objects` and `sink`.
  What a caller that is no command line is offered is the five calls at the
  root of the crate and `api`, which have the names they were planned with.
  The modules under them are named for the command each serves and, for the
  AC-3 family, for the crate that decodes it; 620 mutations, the line
  references of the documents and the evidence of the round name those
  files. New names would move nothing a user or a caller sees, and would be
  paid for in every one of those. This is a decision, not a postponement.
- **A census of whole tracks.** One shelf, 294 files and 1210 tracks, was read
  from the first byte of every track to its last: 2702.7 hours against the
  19.9 of their heads, in 22 hours 36 minutes of one disk, the sweep in the
  background mode of the system. 14 tracks are not clean for their length, 13
  of them clean for a minute, and each was put to other decoders and to its
  file. Four TrueHD tracks are damaged at 18 of their seamless branches, in
  the file: FFmpeg and truehdd lose the units this decoder loses. One TrueHD
  track is two encodings end to end. In seven AC-3 tracks 1472 frames end
  inside their closing bits and decode as Dolby's decoder decodes them, 93.2
  dB at the least with the noise of the two in step. Ten frames of four AC-3
  tracks, two of them among the seven, fail a checksum or would not decode, in
  the file. Three clean tracks are encodings joined of pieces and say a
  downmix or a dialogue normalisation in part of their frames. Beyond the
  minute 316 more tracks switch blocks, 11 more counters of the TrueHD census
  are seen and 153 still are not, and the timing jumps at 441 places where the
  heads have 8 (62).

  Two things in it are this decoder's. It would not decode one frame, with
  both checksums right and a mantissa code the standard gives no meaning,
  which FFmpeg and Dolby's decoder decode. And it left the three frames it
  would not decode out of what it wrote, so that the track after them is
  1536 samples a frame early, where the two others keep the length. The
  second is corrected: a frame of silence stands in the place of a frame that
  will not decode, as silence stands in the place of a TrueHD access unit,
  and the two tracks decode to the frames of their files and stand where
  FFmpeg's decode stands along their length (63). The first is corrected as
  well. Dolby's decoder reads a mantissa code above its range as the top
  level of its quantiser, in each of the seventeen such codes there are, and
  so does this decoder now: the track decodes with every frame in its place
  and stands against Dolby's decode at 118.5 dB at the median from its first
  frame to its last, where it fell to 55.0 after the frame (64).

With these the conformance matrix has 134 rows: 96 PASS, 17 PASS-TOL, 3
PARTIAL, 7 N/I, 6 N/T, 5 UNK and no FAIL; its table of what moved since the
audits has four more, the programme of a second generation, the reading of
another program, the frame that will not decode and the mantissa code above
its range.

One measurement more came with material and not with a plan. The three
things done Dolby's way in the objects of JOC were read off streams of one
library and measured on that library (section 3.9). While the above was done
the owner added four whole tracks of streaming services, which nothing of the
round had read. They are clean from end to end, 8.6 hours without a finding
of any rule, and their objects stand against Dolby's decoder where the
library's stand: 101.9 to 117.5 dB over the whole band at the median of an
excerpt, the stretches with the clip gain most at work no lower than the
heads, and the two decoders at the same sample in all eight excerpts (61).
