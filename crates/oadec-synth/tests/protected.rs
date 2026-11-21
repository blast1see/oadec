//! Protected blocks: `error_protect`, `block_data_bits` in front of the data
//! and `block_header_CRC` behind it, the check written as Dolby's decoder
//! takes it, and faults put into a written stream one at a time.
//!
//! The check is recomputed here bit by bit from nothing but the bits the
//! writer says the header lies in, as `tools/crc8_reference.py` does it.

use oadec_synth::signal::noise;
use oadec_synth::{
    BlockSpans, Check, Coding, Family, Layout, Protection, Stream, UnitPlan, flip_and_reseal, write,
};

/// The CRC-8 of bits `start..end`, polynomial `0x1D`, the register from
/// `0xA2`, the bits most significant first.
fn header_crc(data: &[u8], span: Option<(usize, usize)>) -> u8 {
    let mut crc: u8 = 0xA2;
    if let Some((start, end)) = span {
        for pos in start..end {
            let bit = (data[pos / 8] >> (7 - pos % 8)) & 1;
            let out = crc & 0x80 != 0;
            crc <<= 1;
            if out {
                crc ^= 0x1D;
            }
            crc ^= bit;
        }
    }
    crc
}

fn byte_at_bit(data: &[u8], bit: usize) -> u8 {
    let mut v = 0u8;
    for k in 0..8 {
        let pos = bit + k;
        v = (v << 1) | ((data[pos / 8] >> (7 - pos % 8)) & 1);
    }
    v
}

/// A protected stereo stream of `units` units in blocks of `sizes`, its
/// channels coded with Huffman books so the headers are of several lengths.
fn protected(check: Check, sizes: &[usize], units: usize) -> (Stream, Vec<Vec<i32>>) {
    let mut layout = Layout::new(Family::TrueHd, 48_000, 2);
    layout.protection = Some(Protection {
        counted_beyond_the_data: 0,
        check,
    });
    let n = units * layout.samples_per_unit();
    let pcm = vec![noise(n, 14, 11), noise(n, 14, 12)];
    let coding = vec![
        Coding {
            book: 1,
            ..Coding::default()
        },
        Coding {
            book: 2,
            ..Coding::default()
        },
    ];
    // the sizes of the first unit, then whole units, then two blocks: a
    // header that changes the size, one that does not, and one that also
    // sends new guards
    let plans: Vec<UnitPlan> = (0..units)
        .map(|k| match k % 3 {
            0 => UnitPlan::uniform(sizes, &coding),
            1 => UnitPlan::uniform(&[40], &coding),
            _ => UnitPlan::uniform(&[8, 32], &coding).guarded(0xFF),
        })
        .collect();
    let stream = write(&layout, &plans, &pcm).expect("the stream is written");
    (stream, pcm)
}

fn protected_blocks(stream: &Stream) -> Vec<BlockSpans> {
    stream
        .blocks
        .iter()
        .copied()
        .filter(|b| b.check.is_some())
        .collect()
}

/// Every protected block states the CRC of its own header, headers present
/// and absent, aligned and not, of more than one length.
#[test]
fn every_check_byte_is_the_crc_of_its_block_header() {
    let (stream, _) = protected(Check::HeaderCrc, &[8, 16, 16], 12);
    let blocks = protected_blocks(&stream);
    assert!(!blocks.is_empty(), "the writer says where its blocks lie");
    let mut with_header = 0;
    let mut without = 0;
    let mut unaligned = 0;
    let mut lengths = std::collections::BTreeSet::new();
    for b in &blocks {
        let at = b.check.unwrap();
        assert_eq!(
            byte_at_bit(&stream.bytes, at),
            header_crc(&stream.bytes, b.header),
            "unit {} block {}",
            b.unit,
            b.block
        );
        match b.header {
            Some((start, end)) => {
                with_header += 1;
                unaligned += usize::from(start % 8 != 0);
                lengths.insert(end - start);
            }
            None => without += 1,
        }
    }
    assert!(with_header > 0 && without > 0, "{with_header} {without}");
    assert!(unaligned > 0, "an unaligned header");
    assert!(lengths.len() >= 3, "headers of three lengths: {lengths:?}");
}

/// A header struck after its check was made: only the check byte of that
/// block differs from the right stream.
#[test]
fn a_check_of_a_flipped_header_differs_in_its_check_byte_alone() {
    let (right, _) = protected(Check::HeaderCrc, &[8, 16, 16], 4);
    let (struck, _) = protected(Check::HeaderCrcOfFlipped(3), &[8, 16, 16], 4);
    assert_eq!(right.bytes.len(), struck.bytes.len());
    let checks: Vec<usize> = protected_blocks(&right)
        .iter()
        .filter(|b| b.header.is_some())
        .map(|b| b.check.unwrap() / 8)
        .collect();
    assert!(!checks.is_empty());
    for (k, (a, b)) in right.bytes.iter().zip(&struck.bytes).enumerate() {
        if a != b {
            // the check byte, or the parity and CRC of its segment
            let near = checks.iter().any(|&c| k == c || k == c + 1)
                || right
                    .blocks
                    .iter()
                    .any(|s| k + 2 >= s.segment.1 && k < s.segment.1);
            assert!(near, "byte {k} differs");
        }
    }
}

/// A bit flipped in a restart header, a block header or `block_data_bits`
/// leaves the restart CRC and the segment's parity and CRC right.
#[test]
fn flip_and_reseal_keeps_every_check_but_the_block_header_crc() {
    let (stream, _) = protected(Check::HeaderCrc, &[8, 16, 16], 4);
    let first = stream.blocks.first().copied().expect("blocks");
    let restart = first.restart.expect("the first block restarts");
    let header = first.header.expect("and has a header");
    let data_bits = first.data_bits.expect("and states its size");
    for bit in [restart.0 + 40, header.0 + 1, data_bits + 15] {
        let mut struck = stream.clone();
        flip_and_reseal(&mut struck, bit).expect("the bit is in a segment");
        let (s0, s1) = first.segment;
        let body = &struck.bytes[s0..s1 - 2];
        let parity = body.iter().fold(0u8, |a, &b| a ^ b) ^ 0xA9;
        assert_eq!(struck.bytes[s1 - 2], parity, "bit {bit}: parity");
        assert_ne!(struck.bytes, stream.bytes, "bit {bit}: flipped");
    }
    // the checks it writes again are not places for a fault
    let (s0, s1) = first.segment;
    let mut struck = stream.clone();
    assert!(
        flip_and_reseal(&mut struck, (s1 - 1) * 8).is_err(),
        "the CRC"
    );
    assert!(
        flip_and_reseal(&mut struck, (s1 - 2) * 8 + 3).is_err(),
        "the parity"
    );
    assert!(
        flip_and_reseal(&mut struck, restart.1 - 1).is_err(),
        "the restart CRC"
    );
    assert!(flip_and_reseal(&mut struck, s0 * 8 - 1).is_err(), "outside");
    assert_eq!(struck.bytes, stream.bytes);
}
