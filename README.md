# oadec

Object-audio decoder engine for **Dolby TrueHD with Dolby Atmos** and
**E-AC-3 JOC** (Dolby Digital Plus with Dolby Atmos), written from scratch in
Rust and verified against independent decoders on whole films.

`oadec` decodes the four presentations of a TrueHD stream, including the
16-channel object presentation, together with the Object Audio Metadata
carried in the stream, and writes the result as a **DAMF** master set
(`.atmos`, `.atmos.metadata`, `.atmos.audio`) or an **ADM BWF** file. Those
are the two formats a licensed Dolby encoder consumes, so a Blu-ray TrueHD
Atmos track can become an E-AC-3 Atmos track without losing the objects.

## Status

| Milestone | State | Evidence |
|---|---|---|
| TrueHD framing, major sync, `info`, `verify` | done | six films, every integrity counter zero |
| Substream syntax (restart/block headers, matrices, filters, Huffman) | done | 24 million segments, zero parity or CRC failures |
| Presentations 0–2 (2 / 6 / 8 channels) | done | **bit-exact** with FFmpeg over six whole films |
| Presentation 3 (16-channel objects, restart sync `0x31EC`) | done | **bit-exact with the Dolby decoder's own object output** on every presentation it opens at a dialogue normalisation of −31 dB (twelve titles); at any other dialogue norm Dolby applies the dialnorm gain and ±1 LSB triangular dither, and that is all that differs (five titles, gain within 3·10⁻⁵ dB). Its object mode refuses 89 of 194 library presentations; those decode byte-identical to `truehdd`, as all sixteen fresh titles do. All 47,718 lossless checks pass |
| Object Audio Metadata (ETSI TS 103 420) | done | 1,344,146 payloads, zero parse errors |
| Timing model, seamless branches, duplicates | done | Braveheart: 0 branches, the same as truehdd |
| DAMF writer, Dolby validators, encoder round trip | done | validators exit 0; the encoder produces E-AC-3 JOC and TrueHD Atmos from our sets |
| ADM BWF writer | done | structurally identical to the Dolby converter's own output; semantic fidelity audited against Dolby's converters and the audit's findings closed ([report](docs/audit/adm-remediation-report.md)) |
| E-AC-3 / AC-3 core decoder (ETSI TS 102 366) | done | 19 streams, 3.3 million frames, zero CRC or parse failures; closer to a Dolby decode than FFmpeg is on every channel of the hardest streams, and of two fresh DD+ 5.1 cores once FFmpeg's default dynamic range compression is off (`-drc_scale 0`) |
| JOC objects (ETSI TS 103 420) to DAMF / ADM | done | 380,000 payloads parse to the byte; against the Dolby decoder's own object output, over twelve titles (six never measured before), the worst object of a title sits at 29.6 to 50.5 dB and the median at 39.6 to 65.4 dB, at lag zero and with the inter-object correlation structure within 0.0021. The remainder is the E-AC-3 dither on some titles (switching ours off gains 3 dB) and the lowest 375 Hz subband on others (up to 96% of the residual, band SNR 25 to 40 dB) |
| Enhanced coupling (clause E.3.5.5) | done | no stream in the world carries it, so `oadec eac3-ecpl-inject` makes one; both Dolby decoders accept it and agree with us at the dither floor |
| Transient pre-noise processing (clause E.3.7) | done | 3.6 dB closer to the Dolby decode inside the corrected regions; FFmpeg applies nothing |
| JOC clip gain (clause 6.3.3.2) | done | the standard defines the value and not its use; a two-level encode shows the encoder divides the downmix by it, and the Dolby object decoder agrees on a title that changes it mid-reel |

The numbers behind the table: [`docs/evidence/`](docs/evidence/). Format
notes in our own words: [`docs/truehd.md`](docs/truehd.md),
[`docs/oamd.md`](docs/oamd.md), [`docs/eac3.md`](docs/eac3.md),
[`docs/joc.md`](docs/joc.md). Every coding tool the corpus exercises is decoded,
including three nothing else decodes: enhanced coupling, transient pre-noise
processing and the JOC clip gain. Two syntax elements are implemented but
untested for want of material and of an oracle: the enhanced-coupling angle
and chaos terms, and delta bit allocation (rows 69 and 73 of the
[conformance matrix](docs/audit/conformance-matrix.md)). Design and milestones:
[`docs/superpowers/specs/2026-09-06-oadec-design.md`](docs/superpowers/specs/2026-09-06-oadec-design.md).

## Quick start

```text
cargo build --release
oadec info    film.thd                            # what the stream declares
oadec verify  film.thd                            # every integrity rule; exit 7 on failure (docs/exit-codes.md)
oadec decode  film.thd -p 2 -o film-7.1.wav       # a channel presentation as WAVE
oadec decode  film.thd -p 2 --start 90 -o cut.wav # from 90 s, at the next major sync
oadec decode  film.thd --format damf -o out/film  # objects + metadata as a DAMF set
oadec decode  film.thd --format adm  -o out/film  # objects + metadata as ADM BWF
oadec thd-demux dump.thd -o film.thd --core core.ac3   # a Blu-ray dump with its core inside
oadec compare film.thd -p 2 -r ref.s32            # sample-by-sample against a reference
oadec oamd    film.thd --dump 3                   # the object metadata payloads
oadec emdf    film.ec3                            # EMDF containers of an E-AC-3 stream
oadec emdf    film.thd --dump 1                  # and of a TrueHD access unit, with its Evolution block
oadec info    film.ec3                            # E-AC-3: frames, coding tools, JOC statistics
oadec decode  film.ec3 --format damf -o out/film  # JOC objects + metadata as a DAMF set
oadec compare film.ec3 -r ref.f32                 # against a 32-bit float reference (FFmpeg)
```

`film.thd` is a raw TrueHD elementary stream, for example extracted with
mkvextract. A command that reads a whole file draws how far it has got and
says `done` when it finishes, on a terminal; piped or redirected it prints
neither, so a log or a script sees exactly what it always did. The DAMF set can be handed to the Dolby Encoding Engine as it
is (`encode_to_atmos_ddp` takes it as `damf`, `encode_to_dthd` as
`atmos_mezz`). The ADM file passes the Dolby validators only with
`--dolby-origin-tag`, which writes the creator string they insist on; the
flag is off by default and the DAMF output needs no such marker.

Neither object format can carry everything the stream says. Each run prints
what it reduced, dropped or approximated, one line per class, and
`--loss-report FILE` writes the same ledger as JSON. The run exits 4 when
something is missing at the user's request: `--isf drop` writes a programme
without its intermediate-spatial-format elements (the default refuses such a
programme), and `--adm-allow-non-profile-rate` writes an ADM at a rate other
than the profile's 48 kHz. `--adm-interpolation real` writes the stream's own
ramp lengths instead of the profile's fixed 250 samples; that file is outside
the Dolby profile, says so in its `dbmd` tool string, and cannot carry the
origin tag. Gains on active objects and the object numbering are written the
way Dolby's own converters write them. The policy is in
[`docs/exit-codes.md`](docs/exit-codes.md), the measurements in
[`docs/audit/adm-remediation-report.md`](docs/audit/adm-remediation-report.md).

## How it is verified

Nothing is trusted because it looks right. Every claim in the status table
comes from a measurement that can be repeated:

- **Built-in checks.** TrueHD carries parity nibbles, CRCs and a lossless
  check word per restart section. `verify` reports every one of them and the
  decoder compares its own output with the lossless check word.
- **FFmpeg** decodes presentations 0–2. `oadec compare` decodes on the fly
  and compares every sample of a whole film (`-c:a pcm_s32le`, low byte
  dropped).
- **truehdd** (Apache-2.0) decodes the object presentation. Its CAF output
  and its DAMF metadata are compared sample for sample and event for event.
- **The Dolby tools** on the development machine (Encoding Engine 5.2.1 and
  5.7.2, the Reference Player, the Atmos Conversion Tool) validate the DAMF
  and ADM outputs and re-encode them. Their own conversions are the reference
  the ADM writer is diffed against (`tools/adm_diff.py`).
- **The ADM audit toolkit.** `docs/audit/adm/tools` reads DAMF and ADM
  without the writers' help, reconciles every metadata event with every ADM
  block, and checks the Dolby Atmos master ADM profile table by table. The
  2026-09-11 audit judged the writers with it; the remediation kept it as the
  gate. Every writer-level case has an expectation and runs in CI, and every
  audited decode was replayed against its recorded hash.
- **Three decoders.** AC-3 family decoders dither differently by design, so
  E-AC-3 output is judged with `tools/three_way.py`: oadec must sit within
  the distance the Dolby decode and FFmpeg have from each other, channel by
  channel.
- **A library, not a corpus.** `tools/dolby_object_sweep.py` and
  `tools/dolby_truehd_sweep.py` put a question to every Dolby track on the
  machine at once, recording what Dolby's own object decoder does with each and
  every field `info --json` reports about it. `tools/split_fields.py` then asks
  which field, if any, separates two outcomes — and says when the answer is
  what chance would have produced, which on a lopsided set it usually is.

The real-media suite is opt-in. Point `OADEC_MEDIA` at a work directory
that holds the streams and references and run:

```text
cargo test --release -p oadec-cli --test real -- --ignored
```

Every check that needs the corpus -- that suite, the same suite without
the corpus, the TrueHD bit-exactness gates, the object gate, the
corruption replays and the ADM stages -- runs in one command with one
verdict:

```text
tools/media_regression.sh /path/to/work-directory
```

It prints an exit code per step and fails if any step failed. CI cannot
run it: the corpus is licensed material, tens of gigabytes of it, and no
public runner may hold it.

Those tests are `#[ignore]`d, so a plain `cargo test` never touches the media.
Asking for `--ignored` without setting `OADEC_MEDIA` fails: a conformance suite
that cannot reach its material must say so rather than report a pass.

## What it does not do

No rendering to speaker layouts, no dynamic range control, no dialogue
normalisation, no downmixing and no encoding. The output is the decoded
program as the stream carries it. Encoding to TrueHD or E-AC-3 is left to
the licensed encoders, whose metadata protection keys are not public.

## Licence and provenance

GPL-3.0-only. The code was written from the public specifications (ETSI
TS 102 366, ETSI TS 103 420, ITU-R BS.2076, EBU Tech 3285, the Dolby Atmos
master ADM profile) and from format facts established by reading other
public decoders, which are cited in `docs/` and used as test oracles; no
code was copied from them. One 256-entry noise table that is part of the
format appears identically in FFmpeg and truehdd and is carried here as
format data. No Dolby binaries, templates, keys, licence files or media are
part of this repository. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
rules that keep it that way.

`oadec` is an independent project, not affiliated with or endorsed by Dolby
Laboratories. Dolby, Dolby Atmos, Dolby TrueHD and Dolby Digital Plus are
trademarks of Dolby Laboratories.
