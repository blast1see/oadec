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
| Presentation 3 (16-channel objects, restart sync `0x31EC`) | done | **bit-exact with the Dolby decoder's own object output**, every sample of 12 and 16 objects on two films; all 47,718 lossless checks pass |
| Object Audio Metadata (ETSI TS 103 420) | done | 1,344,146 payloads, zero parse errors |
| Timing model, seamless branches, duplicates | done | Braveheart: 0 branches, the same as truehdd |
| DAMF writer, Dolby validators, encoder round trip | done | validators exit 0; the encoder produces E-AC-3 JOC and TrueHD Atmos from our sets |
| ADM BWF writer | done | structurally identical to the Dolby converter's own output |
| E-AC-3 / AC-3 core decoder (ETSI TS 102 366) | done | 19 streams, 3.3 million frames, zero CRC or parse failures; closer to a Dolby decode than FFmpeg is on every channel of the hardest streams |
| JOC objects (ETSI TS 103 420) to DAMF / ADM | done | 380,000 payloads parse to the byte; against the Dolby decoder's own object output, 40 to 56 dB per object, at the dither floor in every band the core decode is exact in |
| Enhanced coupling (clause E.3.5.5) | done | no stream in the world carries it, so `oadec eac3-ecpl-inject` makes one; both Dolby decoders accept it and agree with us at the dither floor |
| Transient pre-noise processing (clause E.3.7) | done | 3.6 dB closer to the Dolby decode inside the corrected regions; FFmpeg applies nothing |
| JOC clip gain (clause 6.3.3.2) | done | the standard defines the value and not its use; a two-level encode shows the encoder divides the downmix by it, and the Dolby object decoder agrees on a title that changes it mid-reel |

The numbers behind the table: [`docs/evidence/`](docs/evidence/). Format
notes in our own words: [`docs/truehd.md`](docs/truehd.md),
[`docs/oamd.md`](docs/oamd.md), [`docs/eac3.md`](docs/eac3.md),
[`docs/joc.md`](docs/joc.md). Every coding tool of the format is now decoded,
including the three nothing else decodes: enhanced coupling, transient
pre-noise processing and the JOC clip gain. Design and milestones:
[`docs/superpowers/specs/2026-09-06-oadec-design.md`](docs/superpowers/specs/2026-09-06-oadec-design.md).

## Quick start

```text
cargo build --release
oadec info    film.thd                            # what the stream declares
oadec verify  film.thd                            # every integrity rule; exit 7 on failure (docs/exit-codes.md)
oadec decode  film.thd -p 2 -o film-7.1.wav       # a channel presentation as WAVE
oadec decode  film.thd --format damf -o out/film  # objects + metadata as a DAMF set
oadec decode  film.thd --format adm  -o out/film  # objects + metadata as ADM BWF
oadec thd-demux dump.thd -o film.thd --core core.ac3   # a Blu-ray dump with its core inside
oadec compare film.thd -p 2 -r ref.s32            # sample-by-sample against a reference
oadec oamd    film.thd --dump 3                   # the object metadata payloads
oadec emdf    film.ec3                            # EMDF containers of an E-AC-3 stream
oadec info    film.ec3                            # E-AC-3: frames, coding tools, JOC statistics
oadec decode  film.ec3 --format damf -o out/film  # JOC objects + metadata as a DAMF set
oadec compare film.ec3 -r ref.f32                 # against a 32-bit float reference (FFmpeg)
```

`film.thd` is a raw TrueHD elementary stream, for example extracted with
mkvextract. The DAMF set can be handed to the Dolby Encoding Engine as it
is (`encode_to_atmos_ddp` takes it as `damf`, `encode_to_dthd` as
`atmos_mezz`). The ADM file passes the Dolby validators only with
`--dolby-origin-tag`, which writes the creator string they insist on; the
flag is off by default and the DAMF output needs no such marker.

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
- **Three decoders.** AC-3 family decoders dither differently by design, so
  E-AC-3 output is judged with `tools/three_way.py`: oadec must sit within
  the distance the Dolby decode and FFmpeg have from each other, channel by
  channel.

The real-media suite is opt-in. Point `OADEC_MEDIA` at a work directory
that holds the streams and references and run:

```text
cargo test --release -p oadec-cli --test real -- --ignored
```

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
