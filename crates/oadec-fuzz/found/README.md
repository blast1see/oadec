# Inputs a fuzzer found

Inputs that broke a target of this crate under libFuzzer, kept under the name
of the target and put to it again by `tests/seeded.rs` with every `cargo
test`. A run of the `fuzz` workflow that finds something leaves its input as
an artifact of the run; once what it found is put right, the input goes here,
cut to what it takes to show it. Each is a few access units or syncframes of a
fixture of this repository with some of its bytes changed, and holds nothing
that is not already in `crates/oadec-cli/tests/fixtures`.

| Target | File | Bytes | SHA-256 | What it is |
|---|---|---:|---|---|
| `truehd_unit_repaired` | `restart-crc-in-the-trailer.bin` | 852 | `c0b2e5cba89bb346b4406d6a30053c981f98bc1406c83f02e9d5987440c20919` | the first access unit of `authored-scene.mlp` with the end pointer of its first substream set to nine words. The restart header of that segment then runs into the two bytes that close it, and its CRC sits in bits the trailer is written over. A repair wrote both, the second over the first, and so a second repair still found one restart CRC and one trailer to change |
