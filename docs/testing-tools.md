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
