//! A decoder nobody listens to (`Options::samples` off): it reads what a
//! stream says and makes no samples, and hands out the frames a decoder that
//! makes them hands out, in the same calls.

use oadec_eac3::{Decoded, Decoder, FrameHeader, Options};

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

fn unheard() -> Options {
    Options {
        samples: false,
        ..Options::default()
    }
}

/// Holds what two decoders made of one call against each other: the one that
/// makes samples and the one nobody listens to. Says whether a frame came.
fn same_but_for_the_samples(heard: Option<Decoded>, unheard: Option<Decoded>) -> bool {
    match (heard, unheard) {
        (None, None) => false,
        (Some(a), Some(b)) => {
            assert_eq!(a.header, b.header);
            assert_eq!(a.bsi, b.bsi);
            assert_eq!(a.skip_fields, b.skip_fields);
            assert_eq!(a.auxdata, b.auxdata);
            assert_eq!(a.auxdata_overrun, b.auxdata_overrun);
            assert_eq!(a.coverage, b.coverage);
            assert_eq!(a.crc_ok, b.crc_ok);
            assert_eq!(a.used_bits, b.used_bits);
            assert_eq!(a.transproc, b.transproc);
            assert_eq!(a.tail_overrun, b.tail_overrun);
            // the one has every sample of every channel, the other the
            // channels and nothing in them
            assert_eq!(a.pcm.len(), a.header.nchans());
            assert!(a.pcm.iter().all(|c| c.len() == a.header.samples()));
            assert_eq!(b.pcm.len(), a.pcm.len());
            assert!(
                b.pcm.iter().all(Vec::is_empty),
                "samples were made for a decoder nobody listens to"
            );
            true
        }
        (a, b) => panic!(
            "one decoder handed out a frame and the other none: {} and {}",
            a.is_some(),
            b.is_some()
        ),
    }
}

#[test]
fn a_decoder_nobody_listens_to_hands_out_the_same_frames_and_no_samples() {
    let stream = fixture();
    let mut with = Decoder::new(Options::default());
    let mut without = Decoder::new(unheard());
    let mut came = Vec::new();
    for frame in frames(&stream) {
        came.push(same_but_for_the_samples(
            with.decode(frame).expect("the fixture decodes"),
            without.decode(frame).expect("the fixture decodes"),
        ));
    }
    // a frame is held back until the one after it has come
    assert!(!came[0]);
    assert!(came[1..].iter().all(|&c| c));
    assert_eq!(came.len(), 63);
    assert!(same_but_for_the_samples(
        with.flush().expect("the end"),
        without.flush().expect("the end")
    ));
    assert!(!same_but_for_the_samples(
        with.flush().expect("the end"),
        without.flush().expect("the end")
    ));
}

/// A frame that does not decode is the same error to both, and what is done
/// about it, the frames still held taken out and the decoder reset, leaves
/// both where it leaves the one that makes samples.
#[test]
fn a_frame_that_does_not_decode_is_the_same_to_a_decoder_nobody_listens_to() {
    let stream = fixture();
    let frames = frames(&stream);
    let mut with = Decoder::new(Options::default());
    let mut without = Decoder::new(unheard());
    let mut released = 0;
    for (n, frame) in frames.iter().enumerate() {
        // the tenth frame comes cut in half, and so does the thirtieth
        let cut = if n == 9 || n == 29 {
            &frame[..frame.len() / 2]
        } else {
            frame
        };
        match (with.decode(cut), without.decode(cut)) {
            (Ok(a), Ok(b)) => released += usize::from(same_but_for_the_samples(a, b)),
            (Err(a), Err(b)) => {
                assert!(n == 9 || n == 29, "frame {n}: {a}");
                assert_eq!(a.to_string(), b.to_string());
                while same_but_for_the_samples(
                    with.flush().expect("a flush"),
                    without.flush().expect("a flush"),
                ) {
                    released += 1;
                }
                with.reset();
                without.reset();
            }
            (a, b) => panic!("frame {n}: {} and {}", a.is_ok(), b.is_ok()),
        }
    }
    while same_but_for_the_samples(
        with.flush().expect("the end"),
        without.flush().expect("the end"),
    ) {
        released += 1;
    }
    assert_eq!(released, 61);
}
