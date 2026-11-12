# Streams a campaign found

Short streams that seeded changes to the fixtures and to the streams of
`oadec-synth` turned up (`crates/oadec-fuzz/tests/campaign.rs`), and one that
changing every bit of them in turn did (`crates/oadec-fuzz/examples/flips.rs`),
kept because
two commands said different things of each, or because the policy of the exit
codes lets them and that deserved a stream to show it. Each is a few access
units or syncframes of a fixture of this repository with some of its bytes
changed and, but for one, its checksums put right again; none holds anything
that is not already in `tests/fixtures`. `tests/cli_found.rs` holds every
command to what it says of each.

| File | Bytes | SHA-256 | What it is |
|---|---:|---|---|
| `truehd-payload-without-object-element.bin` | 1 944 | `c5abf9821f7fec8bc08d042472d2c0fab5c33244f1cc7df090e33b9eacb73048` | a TrueHD stream whose one Object Audio Metadata payload parses and has no object element: an object output can make nothing of it. `verify` and `oamd` called the stream clean and `decode --format damf` exited 7 |
| `eac3-payload-without-object-element.bin` | 7 168 | `73ccd7edc3b975ff9d72a784658f1c15772431810219477d36c5caa46411d584` | the same in an E-AC-3 stream, in the second of its four frames. `verify` called it clean; `oamd` did not, for padding that is not zero in the same payload, which only it counts |
| `truehd-major-sync-unreadable.bin` | 432 | `36e2e71bd484af090f085508a53e5de2137478f1823983ad398cd86c44f50fcf` | a TrueHD stream whose only major sync carries a reserved sampling frequency code. `info` said there was no major sync and no TrueHD stream, at exit 0; `verify` counts eight access units and exits 7; a decode cannot start and exits 2. Its checksums are as the change left them |
| `truehd-presentation-over-missing-substream.bin` | 1 012 | `c4a3cb1996f1a93817169f2809da3d9211be911f9c1a1519b816204adbd0b758` | a TrueHD stream of one substream whose major sync declares presentations over substreams it does not have. `verify` has no rule for it and calls the stream clean; `verify --decode` cannot start those presentations and exits 7; `decode` refuses them with exit 2 |
| `eac3-complexity-index-mismatch.bin` | 7 168 | `54707d274a30ecd0b0680616e836bed93f67f214ef7365dcea1417ee3685f370` | an E-AC-3 stream whose `complexity_index_type_a` disagrees with the object total of its metadata in one frame: one of the three things `verify` counts and a delivery does not (`docs/exit-codes.md`), so `verify` exits 7 and every decode exits 0 |
| `truehd-payload-long-padding.bin` | 1 944 | `c3acbaa97cf624c0cd53ebf79b64207b26c5e9f5b859d5c1001167779e1dcfae` | a TrueHD stream in which one bit of the one Object Audio Metadata payload is changed, so that an element of it declares 255 bits of padding that are not zero. `oamd` counts that and exits 7; `verify` does not, and every decode is clean: what the policy lets a metadata report say of the metadata it reads (`docs/exit-codes.md`) |
| `eac3-element-of-a-stale-size.bin` | 7 168 | `8e6b643c665d155f71617558b335846432a8abfb09f2dab5bcb29e067b585a87` | four syncframes of the E-AC-3 fixture with one bit of the first changed and its CRC put right, found by trying every bit of that frame: an element of the Object Audio Metadata payload then declares a size that is not its own, and nothing else is wrong. The rule for it, `oamd.stale_size`, is a warning: every command calls the stream clean, and `verify --fail-on warning` exits 7 |
