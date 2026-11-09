//! Generated cases exercise bit offsets, widths, padding and failed reads.
#![forbid(unsafe_code)]

use oadec_bits::{BitError, BitReader, BitWriter};
use proptest::prelude::*;

proptest! {
    #[test]
    fn variable_width_fields_round_trip(
        fields in prop::collection::vec((any::<u32>(), 0u32..=32), 0..128),
    ) {
        let mut writer = BitWriter::new();
        for &(value, width) in &fields {
            writer.write(value, width);
        }
        let bits: usize = fields.iter().map(|&(_, width)| width as usize).sum();
        prop_assert_eq!(writer.position(), bits);
        let bytes = writer.finish();
        prop_assert_eq!(bytes.len(), bits.div_ceil(8));
        let mut reader = BitReader::new(&bytes);
        for (value, width) in fields {
            let mask = ((1u64 << width) - 1) as u32;
            prop_assert_eq!(reader.read(width).unwrap(), value & mask);
        }
        prop_assert_eq!(reader.position(), bits);
        let padding = reader.remaining() as u32;
        prop_assert_eq!(reader.read(padding).unwrap(), 0);
        prop_assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn reads_match_independent_bit_by_bit_reference(
        data in prop::collection::vec(any::<u8>(), 0..128),
        offset in any::<usize>(),
        width in 0u32..=64,
    ) {
        let start = offset % (data.len() * 8 + 1);
        let mut reader = BitReader::new(&data);
        reader.seek(start).unwrap();
        let available = reader.remaining();
        let actual = reader.read_u64(width);
        if width as usize <= available {
            let expected = (start..start + width as usize).fold(0u64, |value, bit| {
                (value << 1) | u64::from((data[bit / 8] >> (7 - bit % 8)) & 1)
            });
            prop_assert_eq!(actual, Ok(expected));
            prop_assert_eq!(reader.position(), start + width as usize);
        } else {
            prop_assert_eq!(actual, Err(BitError {
                position: start,
                requested: width as usize,
                available,
            }));
            prop_assert_eq!(reader.position(), start);
        }
    }

    #[test]
    fn peek_and_failed_navigation_preserve_position(
        data in prop::collection::vec(any::<u8>(), 0..128),
        offset in any::<usize>(),
        width in 0u32..=32,
    ) {
        let start = offset % (data.len() * 8 + 1);
        let mut reader = BitReader::new(&data);
        reader.seek(start).unwrap();
        let peek = reader.peek(width);
        prop_assert_eq!(reader.position(), start);
        let mut copy = reader;
        prop_assert_eq!(peek, copy.read(width));
        prop_assert!(reader.skip(reader.remaining() + 1).is_err());
        prop_assert_eq!(reader.position(), start);
        prop_assert!(reader.seek(reader.len_bits() + 1).is_err());
        prop_assert_eq!(reader.position(), start);
    }
}
