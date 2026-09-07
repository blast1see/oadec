//! The three Huffman code books of TrueHD sample coding.
//!
//! Every book has the same shape. A leading `1` selects the centre group: two more
//! bits for book 1 (values 0–3), one more bit for book 2 (0–1), nothing for book 3
//! (0). A leading `00` starts the negative chain: `n` zeros then a `1` give `-(n+1)`,
//! for `n` in 0..=6. A leading `01` starts the positive chain: `n` zeros then a `1`
//! give `base + n` with base 4, 2 or 1 for books 1, 2 and 3. The longest codes are
//! nine bits and end in `1`.

use oadec_bits::BitReader;

use crate::error::{Error, Result};

/// Base of the positive chain of each book (index 1..=3).
const POSITIVE_BASE: [i32; 4] = [0, 4, 2, 1];

/// Longest code of any book, in bits.
pub const MAX_CODE_BITS: u32 = 9;

/// Decodes the nine top bits `bits` of a code for `book` (1..=3) by following
/// the code structure; `None` when they do not form a code.
const fn decode_bits(bits: u32, book: u8) -> Option<(i32, u32)> {
    if bits & 0x100 != 0 {
        return Some(match book {
            1 => (((bits >> 6) & 3) as i32, 3),
            2 => (((bits >> 7) & 1) as i32, 2),
            _ => (0, 1),
        });
    }
    let positive = bits & 0x80 != 0;
    let zeros = ((bits & 0x7F) << 25).leading_zeros();
    if zeros >= 7 {
        return None;
    }
    let value = if positive {
        POSITIVE_BASE[book as usize] + zeros as i32
    } else {
        -(zeros as i32 + 1)
    };
    Some((value, zeros + 3))
}

/// `(value, length)` for every nine-bit prefix of every book; length 0 marks an
/// invalid prefix. A table lookup replaces the data-dependent branches of
/// [`decode_bits`] in the sample loop.
const TABLE: [[(i8, u8); 512]; 3] = {
    let mut table = [[(0i8, 0u8); 512]; 3];
    let mut book = 1;
    while book <= 3 {
        let mut bits = 0;
        while bits < 512 {
            if let Some((value, len)) = decode_bits(bits, book) {
                table[book as usize - 1][bits as usize] = (value as i8, len as u8);
            }
            bits += 1;
        }
        book += 1;
    }
    table
};

/// Decodes the code at the top of a left-aligned 64-bit `window` for `book`
/// (1..=3); returns the value and the code length, or `None` when the nine top
/// bits do not form a code.
#[inline]
pub fn decode_window(window: u64, book: u8) -> Option<(i32, u32)> {
    debug_assert!((1..=3).contains(&book));
    let bits = (window >> (64 - MAX_CODE_BITS)) as usize;
    let (value, len) = TABLE[usize::from(book.wrapping_sub(1)) % 3][bits];
    if len == 0 {
        None
    } else {
        Some((i32::from(value), u32::from(len)))
    }
}

/// Decodes one code of book `book` (1..=3) and advances the reader.
#[inline]
pub fn decode(reader: &mut BitReader<'_>, book: u8) -> Result<i32> {
    let (window, valid) = reader.peek_window();
    if valid == 0 {
        return Err(reader.read(1).unwrap_err().into());
    }
    let (value, len) = decode_window(window, book)
        .ok_or_else(|| Error::malformed("Huffman code without a terminating bit"))?;
    if len as usize > valid {
        return Err(reader.read(len).unwrap_err().into());
    }
    reader.skip(len as usize)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::BitWriter;

    /// (book, value, code bits, code length) for every code of the three books.
    fn codes() -> Vec<(u8, i32, u32, usize)> {
        let mut v = Vec::new();
        for book in 1..=3u8 {
            for k in 1..=7 {
                v.push((book, -k, 1, k as usize + 2));
            }
            match book {
                1 => {
                    for x in 0..4 {
                        v.push((book, x, 0b100 | x as u32, 3));
                    }
                }
                2 => {
                    for x in 0..2 {
                        v.push((book, x, 0b10 | x as u32, 2));
                    }
                }
                _ => v.push((book, 0, 1, 1)),
            }
            for n in 0..7 {
                v.push((
                    book,
                    POSITIVE_BASE[usize::from(book)] + n as i32,
                    (1 << (n + 1)) | 1,
                    n + 3,
                ));
            }
        }
        v
    }

    #[test]
    fn every_code_of_every_book_decodes() {
        for (book, value, code, len) in codes() {
            let mut w = BitWriter::default();
            w.push(len, u64::from(code));
            w.push(7, 0b1010101); // trailing noise
            let mut r = BitReader::new(&w.bytes);
            assert_eq!(
                decode(&mut r, book).unwrap(),
                value,
                "book {book} value {value}"
            );
            assert_eq!(r.position(), len, "book {book} value {value}");
        }
    }

    #[test]
    fn codes_match_the_published_table_values() {
        // Spot checks against the code list verified in the FFmpeg tables:
        // book 1: -7 = 0x001/9, 0 = 0x4/3, 4 = 0x3/3, 10 = 0x81/9
        // book 2: 0 = 0x2/2, 1 = 0x3/2, 8 = 0x81/9
        // book 3: 0 = 0x1/1, 7 = 0x81/9
        let check = |book: u8, code: u32, len: usize, value: i32| {
            let mut w = BitWriter::default();
            w.push(len, u64::from(code));
            w.push(8, 0);
            let mut r = BitReader::new(&w.bytes);
            assert_eq!(decode(&mut r, book).unwrap(), value);
            assert_eq!(r.position(), len);
        };
        check(1, 0x001, 9, -7);
        check(1, 0x4, 3, 0);
        check(1, 0x3, 3, 4);
        check(1, 0x81, 9, 10);
        check(2, 0x2, 2, 0);
        check(2, 0x3, 2, 1);
        check(2, 0x81, 9, 8);
        check(3, 0x1, 1, 0);
        check(3, 0x81, 9, 7);
    }

    #[test]
    fn codes_near_the_end_of_the_data_still_decode() {
        // a book-3 code at the very last bit of the data
        let data = [0b0000_0001u8];
        let mut r = BitReader::new(&data);
        r.skip(7).unwrap();
        assert_eq!(decode(&mut r, 3).unwrap(), 0);
        assert!(decode(&mut r, 3).is_err());
        // a nine-bit code without its terminating bit is rejected
        let data = [0u8, 0];
        let mut r = BitReader::new(&data);
        assert!(matches!(decode(&mut r, 1), Err(Error::Malformed(_))));
        // a code that runs past the end of the data is a bit error, not a value:
        // a book-1 centre code needs three bits, only the leading 1 exists
        let data = [0b0000_0001u8];
        let mut r = BitReader::new(&data);
        r.skip(7).unwrap();
        assert!(matches!(decode(&mut r, 1), Err(Error::Bits(_))));
    }
}
