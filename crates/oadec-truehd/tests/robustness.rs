//! The extractor and the access-unit parser must survive arbitrary bytes:
//! a typed error or a resynchronisation, never a panic. The film corpus
//! exercises the happy path; this covers the other one.

use oadec_truehd::{AccessUnit, Extractor};

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
}

#[test]
fn arbitrary_bytes_through_the_extractor_never_panic() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut units = 0u64;
    for _ in 0..200 {
        let mut extractor = Extractor::new();
        let len = 4096 + (rng.next() % 8192) as usize;
        let mut data: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        // sprinkle sync words so the extractor keeps trying to lock on
        for _ in 0..8 {
            let at = (rng.next() as usize) % (len - 8);
            data[at..at + 4].copy_from_slice(&[0xF8, 0x72, 0x6F, 0xBA]);
        }
        extractor.push(&data);
        while let Ok(Some(_unit)) = extractor.next_unit() {
            units += 1;
        }
        if let Ok((rest, _trailing)) = extractor.finish() {
            units += rest.len() as u64;
        }
    }
    // The extractor validates the major-sync CRC before it locks on, so
    // random data yields nothing; the point is that it never panics while
    // hunting for a sync word it will not find.
    eprintln!("{units} access units came out of 200 random buffers");
}

#[test]
fn arbitrary_access_unit_bytes_never_panic() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let len = 4 + (rng.next() % 2000) as usize;
        let mut data: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        // a major sync in the usual place, so the parser goes deep
        if data.len() > 12 {
            data[4..8].copy_from_slice(&[0xF8, 0x72, 0x6F, 0xBA]);
        }
        let _ = AccessUnit::parse(&data, None);
    }
}
