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
printed and what was written to `tests/golden/<fixture>.txt`. The tests that
assert single fields say that a field is right; the transcripts say that
nothing else moved, which is what a change that only moves code, a new
toolchain or a dependency update has to show.

What differs from run to run is masked: the run time, the speed, the directory
the outputs went to and the tool's own version. Audio that comes out of integer
arithmetic (TrueHD and MLP) is recorded by length and digest. The object audio
of an E-AC-3 stream comes out of floating point and is recorded by length
alone.

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

`tools/old_new.py` runs two binaries over the same streams and names every
exit code, JSON field and line of `verify` that differs. With `--decode-all` it
also compares every delivery byte for byte: the PCM of each presentation, the
DAMF set and the ADM file. The summary counts what each kind of delivery
compared, because two commands that both wrote nothing also agree.

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
MSVC toolchain that is the toolset's x64 AddressSanitizer component, whose
runtime carries the symbols the coverage instrumentation refers to: without
it neither `-s address` nor `-s none` links. The `fuzz` workflow runs every
target on Linux each week, by hand, and on a pull request that touches the
targets, and uploads what it finds; `replay` runs that on any platform. It
does not gate other pull requests: a fuzzer that finds something new says
nothing about the change in front of it.

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

# Streams written from a recipe

`crates/oadec-synth` writes TrueHD and MLP streams from recipes: a layout, a
plan that says what codes each block, and the samples. It is there for the
syntax no encoder on hand will write, and for anything that has to be decoded
to samples that are known beforehand (`docs/truehd.md`, *Streams written from
a recipe*).

```sh
cargo run -p oadec-synth -- list                              # the recipes, and what each carries
cargo run -p oadec-synth -- write fir-orders a.thd a.pcm      # one stream, and its samples as 24-bit PCM
cargo run -p oadec-synth -- all <directory>                   # every recipe
cargo test -p oadec-synth                                     # the writer, the recipes, the record
```

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

A new recipe goes into `src/recipe.rs` with the tools a census of it has to
name, and into the record by a run with `--record`; the two are committed
together. A recipe on which the decoders disagree is still a recipe, and the
disagreement is in the record: `filter-at-restart` is there because FFmpeg
reads it otherwise than `truehdd` and Dolby's decoder do.
