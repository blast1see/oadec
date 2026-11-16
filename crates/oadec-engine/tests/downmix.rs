//! What a stream says of its downmix to two channels, held to its bits.
//!
//! The report has the preferred downmix and the four levels of every frame's
//! bit stream information. What it should say is read here from the frames
//! themselves, by the syntax of ETSI TS 102 366 clause E.1.2.2 and the tables
//! of its Annex D, and not by the decoder.

use std::path::Path;

use oadec_engine::api::Source;
use serde_json::{Value, json};

/// The bytes of a frame of the fixture, which is of one independent
/// substream, 3/2 with an LFE channel.
const FRAME: usize = 1792;

fn fixture() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../oadec-cli/tests/fixtures/authored-scene.ec3");
    let bytes = std::fs::read(path).expect("the fixture");
    assert_eq!(bytes.len() % FRAME, 0);
    bytes
}

/// `n` bits of a frame from bit `at`, the first of them the most significant.
fn bits(frame: &[u8], at: usize, n: usize) -> u8 {
    (at..at + n).fold(0, |value, bit| {
        (value << 1) | ((frame[bit / 8] >> (7 - bit % 8)) & 1)
    })
}

/// The bit at which the downmix fields of a frame begin: after the sync
/// word, `strmtyp`, `substreamid`, `frmsiz`, `fscod`, `numblkscod`, `acmod`,
/// `lfeon`, `bsid`, `dialnorm`, `compre` with its word, and `mixmdate`.
/// `None` for a frame without mixing metadata.
fn downmix_at(frame: &[u8]) -> Option<usize> {
    assert_eq!(frame[..2], [0x0B, 0x77]);
    assert_eq!(bits(frame, 16, 2), 0, "an independent substream");
    assert_eq!(bits(frame, 36, 3), 7, "3/2");
    assert_eq!(bits(frame, 40, 5), 16, "E-AC-3");
    let compre = usize::from(bits(frame, 50, 1));
    let mixmdate = 51 + 8 * compre;
    (bits(frame, mixmdate, 1) == 1).then_some(mixmdate + 1)
}

/// `dmixmod`, `ltrtcmixlev`, `lorocmixlev`, `ltrtsurmixlev` and
/// `lorosurmixlev` of a frame, in the order the syntax has them.
fn said(frame: &[u8]) -> Option<[u8; 5]> {
    let at = downmix_at(frame)?;
    Some([
        bits(frame, at, 2),
        bits(frame, at + 2, 3),
        bits(frame, at + 5, 3),
        bits(frame, at + 8, 3),
        bits(frame, at + 11, 3),
    ])
}

/// Tables D.1.1 to D.1.5, written out a second time.
fn statement(codes: [u8; 5], frames: u64) -> Value {
    const PREFERRED: [&str; 4] = ["not indicated", "Lt/Rt", "Lo/Ro", "reserved"];
    const CENTRE: [&str; 8] = [
        "+3.0", "+1.5", "0.0", "-1.5", "-3.0", "-4.5", "-6.0", "-inf",
    ];
    const SURROUND: [Option<&str>; 8] = [
        None,
        None,
        None,
        Some("-1.5"),
        Some("-3.0"),
        Some("-4.5"),
        Some("-6.0"),
        Some("-inf"),
    ];
    json!({
        "frames": frames,
        "preferred": PREFERRED[usize::from(codes[0])],
        "ltrt_centre_db": CENTRE[usize::from(codes[1])],
        "ltrt_surround_db": SURROUND[usize::from(codes[3])],
        "loro_centre_db": CENTRE[usize::from(codes[2])],
        "loro_surround_db": SURROUND[usize::from(codes[4])],
        "codes": {
            "dmixmod": codes[0],
            "ltrtcmixlev": codes[1],
            "ltrtsurmixlev": codes[3],
            "lorocmixlev": codes[2],
            "lorosurmixlev": codes[4],
        },
    })
}

fn reported(source: &Source) -> Value {
    let probe = oadec_engine::probe(source).expect("a stream");
    let report = probe.report.json().expect("a report").expect("frames");
    report["downmix"].clone()
}

fn text(source: &Source) -> String {
    let probe = oadec_engine::probe(source).expect("a stream");
    let mut out = Vec::new();
    let mut err = Vec::new();
    probe
        .report
        .print(false, &mut out, &mut err)
        .expect("a text");
    String::from_utf8(out).expect("utf-8")
}

/// The line of the text report that begins with `label`.
fn line<'a>(text: &'a str, label: &str) -> Option<&'a str> {
    text.lines().find(|l| l.starts_with(label))
}

#[test]
fn a_stream_says_the_downmix_its_frames_say() {
    let bytes = fixture();
    let first = said(&bytes[..FRAME]).expect("the encoder writes mixing metadata");
    let frames = bytes.chunks(FRAME).count() as u64;
    assert!(
        bytes.chunks(FRAME).all(|f| said(f) == Some(first)),
        "every frame says the same"
    );
    let source = Source::memory("fixture", bytes);
    assert_eq!(reported(&source), json!([statement(first, frames)]));
    // Lo/Ro and four times -3 dB: what the encoder writes when it is told nothing
    assert_eq!(first, [2, 4, 4, 4, 4]);
    assert_eq!(
        line(&text(&source), "Downmix:"),
        Some(
            "Downmix:           Lo/Ro preferred, Lt/Rt centre -3.0 dB surround -3.0 dB, \
             Lo/Ro centre -3.0 dB surround -3.0 dB x63"
        )
    );
}

#[test]
fn a_stream_that_says_two_things_says_both_and_how_often() {
    let mut bytes = fixture();
    let frames = bytes.chunks(FRAME).count();
    let turn = frames / 3;
    for frame in bytes.chunks_mut(FRAME).skip(turn) {
        let at = downmix_at(frame).expect("mixing metadata");
        // dmixmod 2 becomes 1, and lorosurmixlev 4 becomes 7: one bit each
        assert_eq!(bits(frame, at, 2), 2);
        frame[at / 8] ^= 0b11 << (6 - at % 8);
        let last = at + 13;
        assert_eq!(bits(frame, at + 11, 3), 4);
        frame[(last - 1) / 8] |= 1 << (7 - (last - 1) % 8);
        frame[last / 8] |= 1 << (7 - last % 8);
        assert_eq!(said(frame), Some([1, 4, 4, 4, 7]));
        oadec_eac3::repair::fix_crcs(frame).expect("a frame whose checks can be written");
    }
    let source = Source::memory("turned", bytes);
    assert_eq!(
        reported(&source),
        json!([
            statement([1, 4, 4, 4, 7], (frames - turn) as u64),
            statement([2, 4, 4, 4, 4], turn as u64),
        ])
    );
    let text = text(&source);
    assert_eq!(
        line(&text, "Downmix:"),
        Some(
            "Downmix:           Lt/Rt preferred, Lt/Rt centre -3.0 dB surround -3.0 dB, \
             Lo/Ro centre -3.0 dB surround -inf dB x42; \
             Lo/Ro preferred, Lt/Rt centre -3.0 dB surround -3.0 dB, \
             Lo/Ro centre -3.0 dB surround -3.0 dB x21"
        )
    );
    // the frames are whole: what was changed is what they say, not whether they hold
    let verified = oadec_engine::verify(&source, &oadec_engine::api::VerifyOptions::default())
        .expect("a stream");
    assert!(verified.clean, "{:?}", verified.first_problem);
}

#[test]
fn a_stream_whose_frames_say_none_says_none() {
    // AC-3 in the syntax of clause 4 has no place for it
    let recipe = &oadec_synth::AC3_RECIPES[0];
    let source = Source::memory(recipe.name, recipe.stream().expect("the recipe").bytes);
    assert_eq!(reported(&source), json!([]));
    assert_eq!(
        line(&text(&source), "Downmix:"),
        Some("Downmix:           none said")
    );
}
