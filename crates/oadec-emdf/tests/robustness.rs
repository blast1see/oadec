//! Metadata payloads arrive from a lossy transport and from encoders this
//! project has never seen. Parsing an arbitrary byte string must end in a
//! typed error, never in a panic.

use oadec_emdf::container;
use oadec_emdf::joc::{Joc, SparseReading};
use oadec_emdf::oamd::Oamd;

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

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.byte()).collect()
    }
}

#[test]
fn arbitrary_bytes_never_panic_the_metadata_parsers() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut oamd_ok = 0;
    let mut joc_ok = 0;
    let mut containers_ok = 0;
    for _ in 0..50_000 {
        let len = 1 + (rng.next() % 400) as usize;
        let data = rng.bytes(len);
        if Oamd::parse(&data).is_ok() {
            oamd_ok += 1;
        }
        if Joc::parse(&data, SparseReading::AsPrinted).is_ok() {
            joc_ok += 1;
        }
        if container::parse_evolution(&data).is_ok() {
            containers_ok += 1;
        }
    }
    eprintln!(
        "of 50000 random payloads: {oamd_ok} parsed as OAMD, {joc_ok} as JOC, {containers_ok} as Evolution containers"
    );
}

#[test]
fn a_jocs_own_bytes_flipped_one_bit_at_a_time_never_panic() {
    // A payload that parses, then every single-bit corruption of it.
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut base: Option<Vec<u8>> = None;
    for _ in 0..200_000 {
        let len = 1 + (rng.next() % 60) as usize;
        let data = rng.bytes(len);
        if Joc::parse(&data, SparseReading::AsPrinted).is_ok() {
            base = Some(data);
            break;
        }
    }
    let Some(base) = base else {
        eprintln!("no random payload parsed as JOC; bit-flip pass skipped");
        return;
    };
    for byte in 0..base.len() {
        for bit in 0..8 {
            let mut data = base.clone();
            data[byte] ^= 1 << bit;
            let _ = Joc::parse(&data, SparseReading::AsPrinted);
            let _ = Joc::parse(&data, SparseReading::Measured);
        }
    }
}

#[test]
fn an_emdf_container_with_arbitrary_contents_never_panics() {
    let mut rng = Rng(0x0BAD_C0DE_DEAD_BEEF);
    for _ in 0..20_000 {
        let len = 4 + (rng.next() % 300) as usize;
        let mut data = vec![0x58, 0x38];
        data.extend(rng.bytes(len));
        let _ = container::parse_emdf_with_sync(&data);
    }
}
