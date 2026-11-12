# Property and snapshot tests

`oadec-bits` uses `proptest` and `insta` as development dependencies only.
The existing production dependency graph is unchanged. These tests need
neither external media nor nightly Rust and run in the existing CI test job.

```sh
cargo test -p oadec-bits --locked
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked
cargo llvm-cov -p oadec-bits --html
cargo insta test -p oadec-bits --check
```

`properties.rs` checks variable-width writer/reader round trips, zero padding,
reads against an independent bit-by-bit reference, and cursor preservation on
peek or failed navigation. Proptest normally generates 256 cases per property;
set `PROPTEST_CASES=4096` in the shell for a longer pass. Keep any generated
`*.proptest-regressions` files in version control to replay minimized failures.

`snapshots.rs` records a manually verified packet layout and the public error
format using inline snapshots. For an intentional contract change, run
`cargo insta test -p oadec-bits`, then `cargo insta review`, and inspect the diff.
Do not automatically accept changed snapshots in CI.

Nextest does not replace the separate documentation-test command above.
Real-media tests remain ignored unless explicitly requested as described in
`CONTRIBUTING.md`. Coverage of `oadec-bits` alone is not whole-project coverage.

# Transcripts

`crates/oadec-cli/tests/transcripts.rs` runs every reporting command (`info`,
`verify`, `verify --decode`, `emdf`, `oamd`, `au`, `rules`) and every delivery
over each stream under `tests/fixtures` and holds the exit code, what was
printed and what was written to `tests/golden/<fixture>.txt`. A report is
recorded twice, as the text a person reads and as the JSON a program does,
because the two are printed by different code. The tests that
assert single fields say that a field is right; the transcripts say that
nothing else moved, which is what a change that only moves code, a new
toolchain or a dependency update has to show.

What differs from run to run is masked: the run time, the speed, the directory
the outputs went to and the tool's own version. Audio that comes out of integer
arithmetic (TrueHD and MLP) is recorded by length and digest. The samples and
the object audio of an E-AC-3 stream come out of floating point and are
recorded by length alone, and so is an ADM file, which names the version of
the tool that wrote it.

```sh
cargo test -p oadec-cli --test transcripts --locked
OADEC_BLESS=1 cargo test -p oadec-cli --test transcripts --locked
```

The second form rewrites the transcripts after a change that is meant to move
a report. Read what `git diff` shows before committing it: the diff is the
change, stated in the tool's own words. A stream added to `tests/fixtures`
needs a line in the test's list, and a transcript no test writes any more is
itself a failure.

# The mutation run

`tools/mutants.py` changes one line of the decoder at a time, runs the unit
tests and expects one to fail. Its exit code is the verdict: 0 when every
mutation was applied and caught, but for the ones the tool names as expected
to survive and says why; 1 when one survived that should not have, when an
expected survivor was caught, or when the text a mutation changes is no longer
in its file. Each new rule or classifier gets a mutation there.

```sh
python tools/mutants.py --check     # every mutation still finds its text; CI runs this
python tools/mutants.py --only 'delta allocation'
python tools/mutants.py --out mutants.json
```

# The old and the new binary

`tools/old_new.py` runs two binaries over the same streams and names what
differs: the exit code, every field of the JSON and every line of the text of
the four reports (`info`, `verify`, `emdf`, `oamd`), on stdout and on stderr.
With `--decode-all` it also compares every delivery byte for byte, the PCM of
each presentation, a WAVE file, the DAMF set and the ADM file, and the lines
the decode printed while it wrote them. A field or a line only the new binary
has is listed and not judged; one the old binary had and the new one changed,
dropped or moved is a change. The time a run took and the directory it wrote
to are left out. The summary counts what each command compared, lines, fields
and bytes, because two commands that both said nothing also agree.

The tool is a gate, and a gate is tested like anything else: give it a binary
that is known to print something else and see that it says so
(`docs/audit/evidence/engine-round-2026-11/19-reports-compared.json`).

Two binaries that are both told of a file that is not there agree to the
line. So the tool looks at every stream before its first command and after
its last, and one that was not there either time, or that was another size or
written at another time by the end, is a change of its own: what was compared
was not that stream, and the exit code says so. The summary counts the bytes
of the streams, which is a number the corpus can be held to
(`docs/audit/evidence/engine-round-2026-11/23-a-stream-that-is-gone.json`).

```sh
python tools/old_new.py --old old/oadec --new target/release/oadec \
    --decode-all --tmp <a directory with room> --out old-new.json <streams or directories>
```

# Fuzzing

`crates/oadec-fuzz` holds what must be true of the parsers and decoders
whatever bytes they are given, as ten plain functions (`src/targets.rs`): the
TrueHD extractor, the access-unit and segment parsers, the TrueHD decoder, the
EMDF container in both its carriages, the OAMD and JOC payloads, and the frame
decoder and the programme decoder of the AC-3 family. The unit parser and the
TrueHD decoder are there twice, the second time on the input with its
checksums put right first (below), which is what gets a changed field past
the checksum over it and into the code behind. A function takes bytes
and asserts; one that returns has found nothing. "No panic" is the least of
what they assert. The rest is what two routes to one answer have to agree on:
a stream read whole and the same stream read in pieces, a segment parsed with
and without the place of a failure, two decodes of the same bytes.

The same functions run two ways.

**The seeded run** (`tests/seeded.rs`) is part of `cargo test`. It cuts seeds
out of the committed fixtures (`src/seeds.rs`: runs of access units, frames,
containers, payloads), changes them with a generator that has a seed
(`src/mutate.rs`: a bit, a burst, a field at the edge of its range, bytes lost,
doubled or taken from another input), and hands each result to its target. It
needs neither a nightly toolchain nor a fuzzing runtime, so it runs on every
platform and in the test job of CI. It is blind: nothing tells it which input
reached new code, so it finds what damage in storage finds and not what a
search would.

```sh
cargo test -p oadec-fuzz                                              # 300 rounds a target
OADEC_FUZZ_ITERS=200000 cargo test --release -p oadec-fuzz --test seeded
OADEC_FUZZ_SEED=7 OADEC_FUZZ_ITERS=200000 cargo test --release -p oadec-fuzz --test seeded
cargo run -p oadec-fuzz --example replay -- <target> <file>
```

A failure names the target, the seed and the round, which give the same bytes
on any machine, and leaves the input in a file for `replay`.

**libFuzzer** (`fuzz/`, a workspace of its own with its own lock) wraps each
function in a target and mutates under coverage feedback.

```sh
cargo run -p oadec-fuzz --example corpus -- crates/oadec-cli/tests/fixtures fuzz/corpus
cargo +nightly fuzz run truehd_unit -- -max_total_time=600 -max_len=65536
```

It needs a nightly toolchain and a platform its runtime links on. With the
MSVC toolchain that is the toolset's x64 AddressSanitizer component
(`Microsoft.VisualStudio.Component.VC.ASAN` in the Visual Studio installer),
whose runtime carries the symbols the coverage instrumentation refers to:
without it neither `-s address` nor `-s none` links. With it the targets link,
and one thing more is needed to run them. The sanitizer's runtime is a DLL in
`VC\Tools\MSVC\<version>\bin\Hostx64\x64` of the Visual Studio
installation, and that directory has to be on `PATH`, where a developer prompt
has it. Without it a target ends before it reads an input, with
`STATUS_DLL_NOT_FOUND`, which reads like a crash of the target and is not one
(`docs/audit/evidence/engine-round-2026-11/14-libfuzzer-on-msvc.json`).

The `fuzz` workflow runs every target on Linux each week, by hand, and on a
pull request that touches the targets, and uploads what it finds; `replay`
runs that on any platform. It does not gate other pull requests: a fuzzer
that finds something new says nothing about the change in front of it.

What a fuzzer finds becomes a unit test of the code it was found in and, where
it is a rule, a mutation in `tools/mutants.py`. The fuzzers find; the suite
keeps.

# Putting checksums right

A stream changed on purpose fails at the first checksum over the change, and
what the parser would have made of the change is never seen. A test of a rule
behind a checksum, and a fuzzer that wants to reach the decoder, both need the
checksums recomputed.

`oadec_truehd::Repairer` does that for an access unit: the CRC-16 of a major
sync, the check nibble over the header and the directory, the CRC-8 of every
restart header the blocks reach, the parity and the CRC-8 that close a
segment, and the check nibble and the parity byte of the extra data.
`oadec_eac3::repair::fix_crcs` does it for a syncframe: `crc2`, and `crc1` of
an AC-3 frame, which sits in front of what it covers and is found by running
the register backwards. `oadec repair <stream> -o <out>` applies them to a
whole stream; the command is hidden, because it makes bytes consistent and not
audio right.

Two things are left as they are, on purpose. The lossless check of a restart
header is a statement about the samples: a changed sample is meant to fail
it. And the protection of an EMDF container or an Evolution frame is keyed;
nothing here computes it, and a payload changed under it stays changed under
the protection it had.

On a sound stream a repair changes no byte. That is its test: the fixtures in
`cargo test`, and every stream of the work directory by hand
(`docs/audit/evidence/engine-round-2026-11/09-checksum-repair.json`).

# The commands as calls

Asking a command about a stream took a process and a file: start `oadec`,
read what it printed, read its exit code. `oadec_cli::api` is the commands
without the command line. Each function does what a command does and hands
back what it found; none prints, and none ends the process.

| The call | The command | What it hands back |
|---|---|---|
| `api::info(&source)` | `info` | the report; `json()` is what `--json` prints |
| `api::verify(&source, &options)` | `verify` | the report, and `clean()`: exit code 0, where anything else is 7 |
| `api::oamd(&source)`, `api::emdf(&source)` | `oamd`, `emdf` | the report and `clean()` |
| `api::decode_pcm(&source, &options, &mut sink, &mut remarks)` | `decode --format pcm` and `wav` | the samples, in the sink; `findings()?.verdict()` is the exit code |
| `api::decode_objects(&source, base, &options, &mut remarks)` | `decode --format damf` and `adm` | the files, at `base`; `findings()?.verdict()` is the exit code |

A stream is a `Source`: `Source::file(path)`, or `Source::memory(name,
bytes)` for one that never was a file. Samples go into a `PcmSink`.
`PcmFile` is the file the command writes, raw or under a WAVE header, and
`PcmBytes` keeps them in memory with what the header would have said. The
object outputs are files whoever asks, because a DAMF set and an ADM file are
closed by going back over what was written. What a decode says while it runs
goes to `remarks`, a line at a time: `std::io::stderr()` to hear it as the
command says it, `std::io::sink()` not to.

```rust
use oadec_cli::api::{self, PcmBytes, PcmOptions, Source, VerifyOptions};

let source = Source::memory("changed.thd", bytes);
let clean = api::verify(&source, &VerifyOptions::default())?.clean();
let mut samples = PcmBytes::default();
let verdict = api::decode_pcm(&source, &PcmOptions::default(), &mut samples, &mut std::io::sink())?
    .findings()?
    .verdict();
```

What the command line refuses of an option that does not apply, the call that
takes the option refuses in the same words: `fail_on` or `start` for an AC-3
stream, the JOC measurement overrides for a TrueHD one. The values a call
hands back have methods that print (`print`, `print_summary`); the command
line is the calls, or the functions under them, and those methods, one after
the other, which is what keeps the two from drifting apart.

`tests/api.rs` holds every call to its command, on every fixture and for a
file and for the same bytes in memory: the report field for field, the
verdict against the exit code, every delivery byte for byte, each refusal
word for word. That nothing is printed is tested the only way it can be,
from outside: the test runs itself again as a process of its own, makes
every call there, on the fixtures and on copies of them with bytes changed
and with their second half cut off, and reads what that process wrote.

Which functions print is not left to the reader. The crate denies clippy's
`print_stdout` and `print_stderr`, and a function that prints carries an
`allow` that says so; the lint run in CI lets no other through.

# One verdict on the same bytes

Every command that delivers audio or metadata is to decide between clean and
faulty with the list of faults `verify` uses
([`exit-codes.md`](exit-codes.md)). That sentence has been false more than
once, each time in a stream nobody had put to two commands, and each time it
was found by accident. `oadec_fuzz::parity` puts it to the test on purpose: a
stream goes to every command that reads one, through the calls above, and
`check` holds what they said to the policy, clause by clause:

- what holds no stream holds none for any command: the reports refuse it
  together, and nothing is delivered from it;
- a fault `verify` finds is a fault to `verify --decode`, to `emdf` and
  `oamd`, and to every delivery, which is faulty or was not made. The three
  things the policy keeps out of the verdict of a delivery are the exception,
  for a delivery alone;
- a fault of a delivery is one `verify --decode` finds;
- `--decode` adds to `verify` what a decode shows and nothing else;
- a declared loss is of the object outputs alone.

It runs three ways, like every target of the crate. `verdict_parity` is a
target among the others, with a libFuzzer wrapper and a place in the weekly
run. `tests/campaign.rs` is the seeded campaign: seeded changes to runs of
the fixtures and to the streams `oadec-synth` writes, three in four with
their checksums put right so that the parsers behind them see the change,
each put to every command, the object outputs among them. A stream that
breaks a clause is left in a file, to be replayed, and any round can be made
again from its numbers and looked at:

```sh
cargo test -p oadec-fuzz --test campaign --locked            # 3 000 rounds, with every cargo test
OADEC_CAMPAIGN_ITERS=200000 cargo test --release -p oadec-fuzz --test campaign -- --nocapture
cargo run -p oadec-fuzz --example replay -- verdict_parity <file>
cargo run -p oadec-fuzz --example round -- crates/oadec-cli/tests/fixtures <seed> <round> [file]
```

The campaign also counts what it does not claim: how often each set of
answers came, and how often a changed stream went through every command
without a word and decoded to other samples. With the checksums put right
that is most changes to a short run, because the lossless check of a restart
interval is in the restart header after it and the end of a stream has none.
It is why the campaign puts the checksums right, and it is not detection.

`crates/oadec-cli/tests/found` keeps the streams a campaign turned up, with a
test that holds every command to what it says of each: two that the commands
told two stories of, and three that show what the policy lets one command say
and another not.

# Streams written from a recipe

`crates/oadec-synth` writes TrueHD and MLP streams from recipes: a layout, a
plan that says what codes each block, and the samples. It is there for the
syntax no encoder on hand will write, and for anything that has to be decoded
to samples that are known beforehand (`docs/truehd.md`, *Streams written from
a recipe*).

```sh
cargo run -p oadec-synth -- list                              # the recipes, and what each carries
cargo run -p oadec-synth -- write fir-orders a.thd a.pcm      # one stream, and its samples as 24-bit PCM
cargo run -p oadec-synth -- write eight-channels.p1 a.thd a.pcm  # the same of a presentation under the widest
cargo run -p oadec-synth -- all <directory>                   # every recipe
cargo test -p oadec-synth                                     # the writer, the recipes, the record
```

A stream of several substreams has a presentation for each. The name of the
recipe is the widest, whose samples the stream was written from;
`<name>.p<index>` is the presentation of the substream at that index under
it, whose samples are what the writer worked out its matrices come to. `all`
writes a `.pcm` for each, and `digests` a line.

The writer shares no code with the decoder, and that is not enough to make a
stream a reference: both were written by one hand from one reading of the
format. The reference is what outside decoders make of the stream.

```sh
cargo build --release -p oadec-synth -p oadec-cli
python tools/synth_accept.py --out <directory>                # FFmpeg, truehdd and Dolby's decoder on every recipe
python tools/synth_accept.py --out <directory> --record       # and rewrite crates/oadec-synth/specs/digests.tsv
```

`synth_accept.py` runs FFmpeg, `truehdd` and, where the Dolby Reference
Player is installed, Dolby's decoder on each stream and compares what they
write with the samples of the recipe, byte for byte. A recipe is confirmed
when two of them give the samples. `--record` writes the digests of every
stream and of its samples with the decoders that gave them, the ones that gave
others and the ones that refused; it writes nothing while a recipe lacks its
two. `cargo test` holds the writer to that file, so a stream cannot change
without having been in front of the decoders again.

Every presentation is asked of every decoder and has its own line: FFmpeg by
`-downmix` where it is not the widest, `truehdd` by `--presentation`, Dolby's
decoder by the `presentation` and `out-ch-config` of its element. A recipe is
confirmed when each of its presentations is. Eight channels are compared in
the order of the stream; FFmpeg and Dolby's decoder write them with the back
pair in front of the side pair, and the tool puts theirs back first.

A new recipe goes into `src/recipe.rs` with the tools a census of it has to
name, and into the record by a run with `--record`; the two are committed
together. A recipe on which the decoders disagree is still a recipe while
two of them give its samples, and the disagreement is in the record:
`filter-at-restart` and `output-shift-down` are there because FFmpeg reads
them otherwise than `truehdd` and Dolby's decoder do. Recording needs the
Dolby Reference Player: for those and a few others it is one of the two, and
without it the tool writes nothing.

The crate has a second list, `PROBES`: streams that go past a bound of the
format or into a field whose meaning is not public (`docs/truehd.md`,
*Probes*). `oadec-synth` lists and writes them with the recipes, their names
beginning `probe-`, and `synth_accept.py` puts them in front of the same
decoders and records what each did in `specs/probes.tsv`. A probe needs no
confirmation and gets none: it is a question. The tests hold the record to
the streams, and the commands to what they do with each today.

The crate writes AC-3 frames as well, for delta bit allocation (`docs/eac3.md`,
*Delta bit allocation, on frames written for it*). Their names begin `ac3-`,
and `probe-ac3-` for the probes.

```sh
cargo run -p oadec-synth -- write ac3-delta a.ac3 a.f32       # the stream, and its samples as 32-bit float
cargo run -p oadec-synth -- ac3-digests                       # every AC-3 stream: digest, rate, frames
cargo test -p oadec-synth --test ac3                          # the writer against the decoder, and the record
```

Nothing about AC-3 is lossless, so `synth_accept.py` judges these by a
signal-to-noise ratio and not by a digest: a decoder has given the samples of
a stream when every channel of what it wrote is within 80 dB of them, at no
delay or at the 32 samples Dolby's decoder adds. Two decoders read AC-3 from
outside, FFmpeg's and Dolby's, and a reference needs both. `--record` writes
`specs/ac3.tsv` with the other two files, a line a stream, references and
probes together; the samples themselves are not in it, because the writer
works them out afresh and the tests compare this decoder's with those.
