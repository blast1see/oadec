//! A malformed stream must never panic the parser: it returns a typed error
//! or a frame, nothing else. The bug this file guards against was real —
//! a low-rate encode reached a mantissa path that indexed an empty array.
//!
//! No dependency and no media: the frames are built here, with valid headers
//! so the generator gets past the sync word and deep into the syntax.

use oadec_eac3::{Frame, FrameHeader, Noise, Options};

/// xorshift64, so the cases are reproducible without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// An E-AC-3 syncframe header that parses, followed by `body` bytes.
fn eac3_frame(rng: &mut Rng, words: u64) -> Vec<u8> {
    let frmsiz = words - 1;
    let numblkscod = rng.below(4);
    let acmod = rng.below(8);
    let lfeon = rng.below(2);
    // strmtyp 0, substreamid 0, frmsiz, fscod 0, numblkscod, acmod, lfeon, bsid 16
    let bits: u64 = (frmsiz << 16) | (numblkscod << 12) | (acmod << 9) | (lfeon << 8) | (16 << 3);
    let b = bits.to_be_bytes();
    let mut frame = vec![0x0B, 0x77, b[4], b[5], b[6], b[7]];
    while frame.len() < (words * 2) as usize {
        frame.push(rng.byte());
    }
    frame
}

/// An AC-3 syncframe header that parses, followed by `body` bytes.
fn ac3_frame(rng: &mut Rng) -> Vec<u8> {
    let frmsizecod = rng.below(38);
    let acmod = rng.below(8);
    let mut bits: u64 = 0;
    bits |= frmsizecod << 40; // fscod 0 above it
    bits |= 8 << 35; // bsid
    bits |= acmod << 29;
    bits |= 1 << 27; // cmixlev
    bits |= 1 << 25; // surmixlev
    bits |= 1 << 24; // lfeon
    let b = bits.to_be_bytes();
    let mut frame = vec![0x0B, 0x77, b[0], b[1], b[2], b[3], b[4], b[5]];
    let header = FrameHeader::parse(&frame).expect("header parses");
    while frame.len() < header.frame_bytes {
        frame.push(rng.byte());
    }
    frame
}

#[test]
fn random_bodies_behind_a_valid_eac3_header_never_panic() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut noise = Noise::default();
    let mut decoded = 0;
    for _ in 0..20_000 {
        let words = 3 + rng.below(600);
        let frame = eac3_frame(&mut rng, words);
        if Frame::parse(&frame, &mut noise, Options::default()).is_ok() {
            decoded += 1;
        }
    }
    // The point is that none of them panicked; a few random frames do decode,
    // which is what keeps the deeper paths in the test.
    eprintln!("{decoded} of 20000 random E-AC-3 frames parsed");
}

#[test]
fn random_bodies_behind_a_valid_ac3_header_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut noise = Noise::default();
    let mut decoded = 0;
    for _ in 0..10_000 {
        let frame = ac3_frame(&mut rng);
        if Frame::parse(&frame, &mut noise, Options::default()).is_ok() {
            decoded += 1;
        }
    }
    eprintln!("{decoded} of 10000 random AC-3 frames parsed");
}

#[test]
fn truncated_frames_are_an_error_not_a_panic() {
    let mut rng = Rng(0x1234_5678_9ABC_DEF0);
    let mut noise = Noise::default();
    for _ in 0..2_000 {
        let words = 3 + rng.below(200);
        let frame = eac3_frame(&mut rng, words);
        let cut = rng.below(frame.len() as u64) as usize;
        assert!(
            Frame::parse(&frame[..cut], &mut noise, Options::default()).is_err(),
            "a truncated frame must not decode"
        );
    }
}

#[test]
fn arbitrary_bytes_are_an_error_not_a_panic() {
    let mut rng = Rng(0x0BAD_C0DE_DEAD_BEEF);
    let mut noise = Noise::default();
    for _ in 0..5_000 {
        let len = 8 + rng.below(4000) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let _ = Frame::parse(&bytes, &mut noise, Options::default());
    }
}
