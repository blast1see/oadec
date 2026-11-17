//! A frame of the independent substream that will not decode keeps its place
//! in the programme: a frame of silence comes out for it, so what is written
//! is as long as the stream and everything after the frame is where it
//! belongs in time.
//!
//! It used to be left out. A shelf of a library read whole had three such
//! frames in two tracks, and the decode of each track came out 1536 samples
//! a frame shorter, with everything after the frame that much early, where
//! FFmpeg and Dolby's decoder write 1536 samples for every frame
//! (`docs/audit/evidence/engine-round-2026-11/62-a-library-read-from-end-to-end.json`).

use oadec_eac3::{Frame, FrameHeader, Noise, Options, ProgramDecoder, ProgramFrame, repair};

/// The committed E-AC-3 stream: 63 frames of 5.1 with object metadata.
fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../oadec-cli/tests/fixtures/authored-scene.ec3"
    );
    std::fs::read(path).expect("the fixture")
}

/// The syncframes of a stream.
fn frames(stream: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 8 <= stream.len() {
        let header = FrameHeader::parse(&stream[at..]).expect("a syncframe");
        out.push(&stream[at..at + header.frame_bytes]);
        at += header.frame_bytes;
    }
    out
}

/// `frame` with every bit of its audio set from a quarter of the way in, and
/// its checksum made right again: a frame whose header and `bsi` read as
/// they did, whose checksum holds, and whose audio blocks are not a frame.
fn undecodable(frame: &[u8]) -> Vec<u8> {
    let mut bad = frame.to_vec();
    let end = bad.len() - 2;
    for byte in &mut bad[frame.len() / 4..end] {
        *byte = 0xFF;
    }
    repair::fix_crcs(&mut bad).expect("a whole syncframe");
    assert!(
        Frame::parse(&bad, &mut Noise::default(), Options::default()).is_err(),
        "the frame still decodes, and the test says nothing"
    );
    bad
}

/// Every group a programme decoder hands out for `frames`, the frame at
/// `damaged` replaced by one that will not decode.
fn groups(
    frames: &[&[u8]],
    damaged: Option<usize>,
    opts: Options,
) -> (Vec<ProgramFrame>, ProgramDecoder) {
    let mut dec = ProgramDecoder::new(opts);
    let mut out = Vec::new();
    for (n, frame) in frames.iter().enumerate() {
        let header = FrameHeader::parse(frame).expect("a syncframe");
        if damaged == Some(n) {
            dec.push(&undecodable(frame), &header).expect("a push");
        } else {
            dec.push(frame, &header).expect("a push");
        }
        while let Some(group) = dec.pop() {
            out.push(group);
        }
    }
    dec.finish().expect("the end");
    while let Some(group) = dec.pop() {
        out.push(group);
    }
    (out, dec)
}

const DAMAGED: usize = 20;

#[test]
fn a_frame_that_will_not_decode_is_a_frame_of_silence_in_its_place() {
    let stream = fixture();
    let frames = frames(&stream);
    let (want, _) = groups(&frames, None, Options::default());
    let (got, dec) = groups(&frames, Some(DAMAGED), Options::default());
    assert_eq!(want.len(), 63);
    assert_eq!(
        got.len(),
        want.len(),
        "the programme is not as long as its stream"
    );
    assert_eq!(dec.stats().decode_errors.get(&(0, 0)), Some(&1));
    for (n, (a, b)) in got.iter().zip(&want).enumerate() {
        assert_eq!(a.index, b.index);
        assert_eq!(a.channels(), b.channels(), "group {n}");
        for ch in 0..a.channels() {
            assert_eq!(a.channel(ch).len(), b.channel(ch).len(), "group {n}");
        }
        assert_eq!(a.core().concealed, n == DAMAGED, "group {n}");
    }
    // The frame itself: what the frame before it still had to say fades out
    // over the block the two share, and then nothing.
    let silent = &got[DAMAGED];
    for ch in 0..silent.channels() {
        assert!(
            silent.channel(ch)[256..].iter().all(|&s| s == 0.0),
            "channel {ch} of the frame that would not decode is not silent"
        );
    }
    assert!(
        (0..silent.channels()).any(|ch| silent.channel(ch)[..256].iter().any(|&s| s != 0.0)),
        "the frame before it was cut off where it stood"
    );
    // Nothing before it changed. The fixture's transient pre-noise processing
    // reaches one frame back, so the frame just before is left out of this.
    for n in 0..DAMAGED - 1 {
        for ch in 0..want[n].channels() {
            assert_eq!(got[n].channel(ch), want[n].channel(ch), "group {n}");
        }
    }
    // And the decode comes back: an independent substream of E-AC-3 seeds
    // its noise anew in every frame, so two frames on, past the block that
    // overlaps the silence and past the look-ahead, it is the clean decode.
    for n in DAMAGED + 3..want.len() {
        for ch in 0..want[n].channels() {
            assert_eq!(got[n].channel(ch), want[n].channel(ch), "group {n}");
        }
    }
}

/// A decoder nobody listens to hands out the same frames: a frame for the
/// one that would not decode, with its channels and nothing in them.
#[test]
fn a_decoder_nobody_listens_to_keeps_the_place_too() {
    let stream = fixture();
    let frames = frames(&stream);
    let unheard = Options {
        samples: false,
        ..Options::default()
    };
    let (got, dec) = groups(&frames, Some(DAMAGED), unheard);
    assert_eq!(got.len(), 63);
    assert_eq!(dec.stats().decode_errors.get(&(0, 0)), Some(&1));
    assert!(got[DAMAGED].core().concealed);
    assert!(
        got.iter()
            .enumerate()
            .all(|(n, g)| g.core().concealed == (n == DAMAGED))
    );
    assert!(got[DAMAGED].core().pcm.iter().all(Vec::is_empty));
}

/// The frame says whether its checksum holds, decoded or not: a frame that
/// would not decode and fails its checksum is damaged, one that would not
/// decode and passes it was written so.
#[test]
fn a_frame_that_will_not_decode_still_says_whether_its_checksum_holds() {
    let stream = fixture();
    let frames = frames(&stream);
    let (got, _) = groups(&frames, Some(DAMAGED), Options::default());
    assert!(got[DAMAGED].core().crc_ok, "its checksum was made right");

    // the same frame with its checksum left as the damage left it
    let mut dec = ProgramDecoder::new(Options::default());
    let mut out = Vec::new();
    for (n, frame) in frames.iter().enumerate() {
        let header = FrameHeader::parse(frame).expect("a syncframe");
        if n == DAMAGED {
            let mut bad = undecodable(frame);
            let last = bad.len() - 1;
            bad[last] ^= 0x01;
            dec.push(&bad, &header).expect("a push");
        } else {
            dec.push(frame, &header).expect("a push");
        }
        while let Some(group) = dec.pop() {
            out.push(group);
        }
    }
    dec.finish().expect("the end");
    while let Some(group) = dec.pop() {
        out.push(group);
    }
    assert_eq!(out.len(), 63);
    assert!(out[DAMAGED].core().concealed);
    assert!(
        !out[DAMAGED].core().crc_ok,
        "a checksum that fails was called right"
    );
}
