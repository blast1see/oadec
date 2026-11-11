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
