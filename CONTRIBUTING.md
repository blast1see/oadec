# Contributing

## Ground rules

- Rust stable, edition 2024, `#![forbid(unsafe_code)]` in every crate.
  Before a commit: `cargo fmt --check`, `cargo clippy --all-targets --
  -D warnings`, `cargo test --workspace`, `cargo deny check`.
- Parsers return typed errors and never panic on malformed input. A stream
  that breaks a rule is reported by `verify`; decoding continues at the
  next point of synchronisation when that is possible.
- Every behavioural claim needs evidence: a unit test with a hand-built
  bitstream, or a measurement on real media recorded in `docs/evidence/`
  with the exact command. A clip is not a film; say which one was measured.
- Brand-neutral names in code and command-line flags. Trademarks appear
  only in documentation, with the notice in the README.

## Licence hygiene

The project is GPL-3.0-only and must stay clean-room:

- The public specifications are the source of truth (ETSI TS 102 366,
  ETSI TS 103 420, ITU-R BS.2076, EBU Tech 3285, the Dolby Atmos master ADM
  profile). Where the specification is silent, other public decoders may be
  read to establish a *fact* about the format. Write the fact down in
  `docs/` with a citation and write the code from the fact, not from their
  code. This applies to truehdd (Apache-2.0) and FFmpeg (LGPL).
- Do not read or reproduce code from projects under non-free or bespoke
  licences.
- Never commit: Dolby binaries, XML job templates, licence files or keys;
  real media in any form; decoded output or logs that embed media; the
  work directory. `.gitignore` lists the extensions, and `target-*`
  directories, but the rule is the rule, not the ignore list.
- New dependencies must pass `deny.toml` (permissive licences, no git
  dependencies). Keep the dependency list short.

## Real-media tests

The real-media suite is opt-in: every test is `#[ignore]`d, so a plain
`cargo test` never touches the media. Asking for `--ignored` is asking for the
conformance suite, so a run that cannot reach the media **fails** rather than
reporting a pass it did not earn. The work directory it expects holds `thd/`
and `ec3/` (raw elementary streams), `ref-ffmpeg/` and `ref-truehdd/`
(reference decodes), `clips/`, `out/` and `dee/`. Nothing in it is ever
committed.

```text
OADEC_MEDIA=E:\oadec-work cargo test --release -p oadec-cli --test real -- --ignored
```

## Commits

One change per commit with a conventional prefix (`feat`, `fix`, `docs`,
`test`, `chore`), a subject line under 72 characters and a body that says
why. Line endings are LF.
