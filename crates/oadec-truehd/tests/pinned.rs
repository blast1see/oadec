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
    FIXTURES, Fnv, Out, break_framing, damage_framing, damage_substream, first_config, first_sync,
    fixture_units, presentations,
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

/// Fixture, presentation, the digest of a copy with units whose framing
/// does not read, decoded concealing. Taken from the decoder of commit
/// `681ab65`, before the restructuring, as the table above was.
const BROKEN_PINS: &[(&str, usize, u64)] = &[
    ("authored-scene.mlp", 0, 0x582e75cdfea6a1ae),
    ("authored-scene.mlp", 1, 0x59654854768303c5),
    ("authored-scene.mlp", 2, 0x78b06b7c9242604b),
    ("authored-scene.mlp", 3, 0x723ceb2aeb9f0a8e),
    ("mlp-41-48k.mlp", 0, 0xf5b6c9cf3cf72dde),
    ("mlp-41-48k.mlp", 1, 0xf5b6c9cf3cf72dde),
    ("mlp-51-96k.mlp", 0, 0x0c6e2873e176a30c),
    ("mlp-51-96k.mlp", 1, 0x0c6e2873e176a30c),
    ("mlp-quad-48k.mlp", 0, 0xdb554e52df6d52d3),
    ("mlp-quad-48k.mlp", 1, 0xdb554e52df6d52d3),
    ("mlp-stereo-44k.mlp", 0, 0x98f65fd3916d7978),
    ("mlp-stereo-44k.mlp", 1, 0x98f65fd3916d7978),
    ("truehd-20-176k.thd", 0, 0x31cc37670822256f),
    ("truehd-20-176k.thd", 1, 0x31cc37670822256f),
    ("truehd-20-176k.thd", 2, 0x31cc37670822256f),
    ("truehd-20-192k.thd", 0, 0xabb20ed5dc8ec548),
    ("truehd-20-192k.thd", 1, 0xabb20ed5dc8ec548),
    ("truehd-20-192k.thd", 2, 0xabb20ed5dc8ec548),
    ("truehd-20-44k.thd", 0, 0x0c96347c514b8b12),
    ("truehd-20-44k.thd", 1, 0x0c96347c514b8b12),
    ("truehd-20-44k.thd", 2, 0x0c96347c514b8b12),
    ("truehd-51-88k.thd", 0, 0x1f87846d9acafbc9),
    ("truehd-51-88k.thd", 1, 0x609310a2211a27d0),
    ("truehd-51-88k.thd", 2, 0x609310a2211a27d0),
];

/// Fixture, presentation, the digest of a copy whose first unit comes
/// twice, decoded strictly: the duplicate the timing and the samples flag.
/// Taken from the decoder of commit `681ab65`, as the tables above were.
const DUPLICATE_PINS: &[(&str, usize, u64)] = &[
    ("authored-scene.mlp", 0, 0x5bc969d171360105),
    ("authored-scene.mlp", 1, 0x528e6db8c6546979),
    ("authored-scene.mlp", 2, 0x32725a86ffe1fd29),
    ("authored-scene.mlp", 3, 0xdafea43e10411e2e),
    ("mlp-41-48k.mlp", 0, 0x7d1a948a2a25c8ac),
    ("mlp-41-48k.mlp", 1, 0x7d1a948a2a25c8ac),
    ("mlp-51-96k.mlp", 0, 0x40de07993dd953a3),
    ("mlp-51-96k.mlp", 1, 0x40de07993dd953a3),
    ("mlp-quad-48k.mlp", 0, 0x0c41b8210e68e32e),
    ("mlp-quad-48k.mlp", 1, 0x0c41b8210e68e32e),
    ("mlp-stereo-44k.mlp", 0, 0xf90972d598c03e20),
    ("mlp-stereo-44k.mlp", 1, 0xf90972d598c03e20),
    ("truehd-20-176k.thd", 0, 0x1a9baeef535a6d37),
    ("truehd-20-176k.thd", 1, 0x1a9baeef535a6d37),
    ("truehd-20-176k.thd", 2, 0x1a9baeef535a6d37),
    ("truehd-20-192k.thd", 0, 0x0c92429c6e2ae507),
    ("truehd-20-192k.thd", 1, 0x0c92429c6e2ae507),
    ("truehd-20-192k.thd", 2, 0x0c92429c6e2ae507),
    ("truehd-20-44k.thd", 0, 0x4bf9c3c42eb0ac02),
    ("truehd-20-44k.thd", 1, 0x4bf9c3c42eb0ac02),
    ("truehd-20-44k.thd", 2, 0x4bf9c3c42eb0ac02),
    ("truehd-51-88k.thd", 0, 0x08c521a14ba6489b),
    ("truehd-51-88k.thd", 1, 0x43cf745b731100f9),
    ("truehd-51-88k.thd", 2, 0x43cf745b731100f9),
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

/// A copy whose units 9 and 40 declare a length past their end, so their
/// framing does not read, and with a fault in the segment of the top
/// substream at unit 20, decoded with `Decoder::decode_concealing`.
fn broken(units: &[Vec<u8>], presentation: usize) -> u64 {
    let mut units = units.to_vec();
    let top = usize::from(first_config(&units).substreams) - 1;
    break_framing(&mut units, 9);
    damage_substream(&mut units, 20, top);
    break_framing(&mut units, 40);
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
fn the_decoder_conceals_a_broken_framing_as_it_did_before() {
    let mut actual = Vec::new();
    for name in FIXTURES {
        let units = fixture_units(name, false);
        for p in presentations(&units) {
            actual.push((*name, p, broken(&units, p)));
        }
    }
    let table: String = actual
        .iter()
        .map(|(name, p, digest)| {
            format!(
                "    ({name:?}, {p}, {digest:#018x}),
"
            )
        })
        .collect();
    assert!(
        actual == BROKEN_PINS,
        "the decoder gives other digests than the pinned ones; now:
{table}"
    );
}

/// A copy with three faults -- a segment of the top substream, the check
/// nibble of a unit's header, a segment of the first substream -- decoded with
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

#[test]
fn the_decoder_flags_a_duplicate_as_it_did_before() {
    let mut actual = Vec::new();
    for name in FIXTURES {
        let mut units = fixture_units(name, false);
        units.insert(1, units[0].clone());
        for p in presentations(&units) {
            actual.push((*name, p, strict(&units, p)));
        }
    }
    let table: String = actual
        .iter()
        .map(|(name, p, digest)| {
            format!(
                "    ({name:?}, {p}, {digest:#018x}),
"
            )
        })
        .collect();
    assert!(
        actual == DUPLICATE_PINS,
        "the decoder gives other digests than the pinned ones; now:
{table}"
    );
}
