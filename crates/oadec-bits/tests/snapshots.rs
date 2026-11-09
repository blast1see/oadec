//! Small, hand-built examples record the public binary and error contracts.
#![forbid(unsafe_code)]

use oadec_bits::{BitReader, BitWriter};

#[test]
fn mixed_width_packet_layout() {
    let mut writer = BitWriter::new();
    writer.write(5, 3);
    writer.write(17, 5);
    writer.write(0xABC, 12);
    writer.write_bit(true);
    let bits = writer.position();
    let bytes = writer.finish();
    // 101 | 10001 | 101010111100 | 1 | 000 (padding).
    assert_eq!(bits, 21);
    assert_eq!(bytes, [0xB1, 0xAB, 0xC8]);
    let hex = bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    insta::assert_snapshot!(format!("bits: {bits}\nbytes: {hex}"), @"
    bits: 21
    bytes: B1 AB C8
    ");
}

#[test]
fn truncated_field_error_contract() {
    let mut reader = BitReader::new(&[0xA5]);
    reader.skip(6).unwrap();
    let error = reader.read(3).unwrap_err();
    assert_eq!(reader.position(), 6);
    insta::assert_snapshot!(error.to_string(),
        @"bit read out of bounds: 3 bits requested at bit 6 with 2 available");
    insta::assert_debug_snapshot!(error, @"
    BitError {
        position: 6,
        requested: 3,
        available: 2,
    }
    ");
}
