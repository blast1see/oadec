//! The single-presentation decoder pinned to what it gave before the
//! decoder was restructured to share its substream decoding: every
//! presentation of every fixture, decoded strictly and decoded concealing a
//! damaged copy, down to the counters, as one FNV-1a digest each.
//!
//! A failure prints the digests the decoder gives now, in the form of the
//! table; a change that is meant to move them replaces the table and says
//! why in its commit.

mod common;

use common::{
    FIXTURES, Fnv, Out, damage_framing, damage_substream, first_config, first_sync, fixture_units,
    presentations,
};
use oadec_truehd::Decoder;

/// Fixture, presentation, how it was decoded, the digest.
const PINS: &[(&str, usize, &str, u64)] = &[
    ("authored-scene.mlp", 0, "strict", 0xe0b925f29464b2fb),
    ("authored-scene.mlp", 0, "concealing", 0x645fa729edf26392),
    ("authored-scene.mlp", 1, "strict", 0x9f596e7abcc5e59f),
    ("authored-scene.mlp", 1, "concealing", 0x483ef86720a9a520),
    ("authored-scene.mlp", 2, "strict", 0x28c5cdc3e828820a),
    ("authored-scene.mlp", 2, "concealing", 0x46dd94159026ed73),
    ("authored-scene.mlp", 3, "strict", 0x3902dd9ad50df75d),
    ("authored-scene.mlp", 3, "concealing", 0x6948d00b08669f44),
    ("mlp-41-48k.mlp", 0, "strict", 0x78df256425144040),
    ("mlp-41-48k.mlp", 0, "concealing", 0x21548c79e78c743c),
    ("mlp-41-48k.mlp", 1, "strict", 0x78df256425144040),
    ("mlp-41-48k.mlp", 1, "concealing", 0x21548c79e78c743c),
    ("mlp-51-96k.mlp", 0, "strict", 0x080348f4249fce03),
    ("mlp-51-96k.mlp", 0, "concealing", 0xde6d0b2d6ac6c812),
    ("mlp-51-96k.mlp", 1, "strict", 0x080348f4249fce03),
    ("mlp-51-96k.mlp", 1, "concealing", 0xde6d0b2d6ac6c812),
    ("mlp-quad-48k.mlp", 0, "strict", 0xd6e7b288287505d8),
    ("mlp-quad-48k.mlp", 0, "concealing", 0xb1c5f2a22455638c),
    ("mlp-quad-48k.mlp", 1, "strict", 0xd6e7b288287505d8),
    ("mlp-quad-48k.mlp", 1, "concealing", 0xb1c5f2a22455638c),
    ("mlp-stereo-44k.mlp", 0, "strict", 0x4061561fbed2b2cd),
    ("mlp-stereo-44k.mlp", 0, "concealing", 0xe276703bd553d84c),
    ("mlp-stereo-44k.mlp", 1, "strict", 0x4061561fbed2b2cd),
    ("mlp-stereo-44k.mlp", 1, "concealing", 0xe276703bd553d84c),
    ("truehd-20-176k.thd", 0, "strict", 0x11821287be535b0a),
    ("truehd-20-176k.thd", 0, "concealing", 0xa8dfb7bb84ab2783),
    ("truehd-20-176k.thd", 1, "strict", 0x11821287be535b0a),
    ("truehd-20-176k.thd", 1, "concealing", 0xa8dfb7bb84ab2783),
    ("truehd-20-176k.thd", 2, "strict", 0x11821287be535b0a),
    ("truehd-20-176k.thd", 2, "concealing", 0xa8dfb7bb84ab2783),
    ("truehd-20-192k.thd", 0, "strict", 0xcd61a177113bcfa0),
    ("truehd-20-192k.thd", 0, "concealing", 0xa0826d327b666c58),
    ("truehd-20-192k.thd", 1, "strict", 0xcd61a177113bcfa0),
    ("truehd-20-192k.thd", 1, "concealing", 0xa0826d327b666c58),
    ("truehd-20-192k.thd", 2, "strict", 0xcd61a177113bcfa0),
    ("truehd-20-192k.thd", 2, "concealing", 0xa0826d327b666c58),
    ("truehd-20-44k.thd", 0, "strict", 0x8446b503c8a65469),
    ("truehd-20-44k.thd", 0, "concealing", 0xbad0c7daadbdff9a),
    ("truehd-20-44k.thd", 1, "strict", 0x8446b503c8a65469),
    ("truehd-20-44k.thd", 1, "concealing", 0xbad0c7daadbdff9a),
    ("truehd-20-44k.thd", 2, "strict", 0x8446b503c8a65469),
    ("truehd-20-44k.thd", 2, "concealing", 0xbad0c7daadbdff9a),
    ("truehd-51-88k.thd", 0, "strict", 0xed6f22271ea7f41a),
    ("truehd-51-88k.thd", 0, "concealing", 0x77bf49369ba84888),
    ("truehd-51-88k.thd", 1, "strict", 0x4447edd2f935b00a),
    ("truehd-51-88k.thd", 1, "concealing", 0xc6fb4ff707db7499),
    ("truehd-51-88k.thd", 2, "strict", 0x4447edd2f935b00a),
    ("truehd-51-88k.thd", 2, "concealing", 0xc6fb4ff707db7499),
];

/// Every unit decoded with `Decoder::decode`, the decode ending at the first
/// error.
fn strict(units: &[Vec<u8>], presentation: usize) -> u64 {
    let mut decoder = Decoder::new(&first_sync(units), presentation).expect("the presentation");
    let mut fnv = Fnv::new();
    for unit in units {
        let out = match decoder.decode(unit) {
            Ok(d) => Out::samples(
                d.pcm,
                d.channels,
                d.duplicate,
                d.concealed,
                d.output_delta,
                None,
            ),
            Err(e) => Out::Failed(e.to_string()),
        };
        fnv.out(&out);
        if matches!(out, Out::Failed(_)) {
            break;
        }
    }
    fnv.bytes(format!("{:?}", decoder.stats()).as_bytes());
    fnv.0
}

/// A copy with three faults -- a segment of the top substream, the framing
/// of a unit, a segment of the first substream -- decoded with
/// `Decoder::decode_concealing`.
fn concealing(units: &[Vec<u8>], presentation: usize) -> u64 {
    let mut units = units.to_vec();
    let top = usize::from(first_config(&units).substreams) - 1;
    damage_substream(&mut units, 5, top);
    damage_framing(&mut units, 9);
    damage_substream(&mut units, 30, 0);
    let mut decoder = Decoder::new(&first_sync(&units), presentation).expect("the presentation");
    let mut fnv = Fnv::new();
    for unit in &units {
        let out = match decoder.decode_concealing(unit) {
            Ok((d, fault)) => Out::samples(
                d.pcm,
                d.channels,
                d.duplicate,
                d.concealed,
                d.output_delta,
                fault.map(|e| e.to_string()),
            ),
            Err(e) => Out::Failed(e.to_string()),
        };
        fnv.out(&out);
        if matches!(out, Out::Failed(_)) {
            break;
        }
    }
    fnv.bytes(format!("{:?}", decoder.stats()).as_bytes());
    fnv.0
}

#[test]
fn the_decoder_gives_what_it_gave_before_on_every_fixture() {
    let mut actual = Vec::new();
    for name in FIXTURES {
        let units = fixture_units(name, false);
        for p in presentations(&units) {
            actual.push((name.to_string(), p, "strict", strict(&units, p)));
            actual.push((name.to_string(), p, "concealing", concealing(&units, p)));
        }
    }
    let pinned: Vec<(String, usize, &str, u64)> = PINS
        .iter()
        .map(|&(name, p, how, digest)| (name.to_string(), p, how, digest))
        .collect();
    let table: String = actual
        .iter()
        .map(|(name, p, how, digest)| format!("    ({name:?}, {p}, {how:?}, {digest:#018x}),\n"))
        .collect();
    assert!(
        actual == pinned,
        "the decoder gives other digests than the pinned ones; now:\n{table}"
    );
}
