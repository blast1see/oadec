//! Putting the CRC words of a syncframe right.
//!
//! A frame that a test or a fuzzer has changed fails its CRC, and nothing
//! behind the CRC ever reads the change. [`fix_crcs`] recomputes the words of
//! clause 6.1.7 and writes them back, so that what was changed is taken at
//! its word: `crc1` over the first five eighths of an AC-3 frame, which sits
//! in front of what it covers, and `crc2` over the whole of a frame of either
//! syntax, which closes it.
//!
//! Nothing inside the frame is touched. An EMDF container in a skip field
//! keeps its bytes, its protection among them: that is keyed, and no part of
//! this crate computes it.
//!
//! On a frame that was sound it changes nothing, which is how it is tested.

use crate::frame::CRC16;
use crate::header::{FrameHeader, Syntax};

/// The low sixteen bits of the generator `x^16 + x^15 + x^2 + 1`.
const GENERATOR: u16 = 0x8005;

/// Which words [`fix_crcs`] had to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fixed {
    /// `crc1`, the word after the sync word of an AC-3 frame.
    pub crc1: bool,
    /// `crc2`, the last word of a frame.
    pub crc2: bool,
}

impl Fixed {
    /// A word was changed.
    #[must_use]
    pub fn any(self) -> bool {
        self.crc1 || self.crc2
    }
}

/// `value` divided by `x` modulo the generator: one step of the register run
/// backwards. The generator's constant term is 1, so `x` has an inverse.
const fn divide_by_x(value: u16) -> u16 {
    if value & 1 == 0 {
        value >> 1
    } else {
        ((value ^ GENERATOR) >> 1) | 0x8000
    }
}

/// The word that, placed in front of `data`, leaves the register clear after
/// both.
///
/// The register of [`CRC16`] after a message `M` is `M(x)` modulo the
/// generator: the data enters at the bottom. For a word `c` followed by `n`
/// bits `D` that is `c * x^n + D`, and the second term is the register after
/// `D` alone. So `c` is that register divided by `x` `n` times.
fn word_in_front_of(data: &[u8]) -> u16 {
    let mut word = CRC16.update_bytes(0, data);
    for _ in 0..data.len() * 8 {
        word = divide_by_x(word);
    }
    word
}

/// The word that, placed behind `data`, leaves the register clear after
/// both: `data` times `x^16` modulo the generator, which is the register
/// after `data` and sixteen zero bits.
fn word_behind(data: &[u8]) -> u16 {
    CRC16.update_bytes(CRC16.update_bytes(0, data), &[0, 0])
}

fn put(frame: &mut [u8], at: usize, word: u16) -> bool {
    let bytes = word.to_be_bytes();
    let changed = frame[at..at + 2] != bytes;
    frame[at..at + 2].copy_from_slice(&bytes);
    changed
}

/// Recomputes the CRC words of the syncframe that starts `frame` and writes
/// them into it. Returns which of them changed, or `None` when `frame` does
/// not start with a whole syncframe.
pub fn fix_crcs(frame: &mut [u8]) -> Option<Fixed> {
    let header = FrameHeader::parse(frame).ok()?;
    let len = header.frame_bytes;
    let frame = frame.get_mut(..len)?;
    let mut fixed = Fixed::default();
    if header.syntax == Syntax::Ac3 {
        let words = len / 2;
        let five_eighths = ((words >> 1) + (words >> 3)) * 2;
        if five_eighths < 4 || five_eighths > len {
            return None;
        }
        // crc1 first: it lies under crc2
        let word = word_in_front_of(&frame[4..five_eighths]);
        fixed.crc1 = put(frame, 2, word);
    }
    if len < 6 {
        return None;
    }
    let word = word_behind(&frame[2..len - 2]);
    fixed.crc2 = put(frame, len - 2, word);
    Some(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::crc_ok;

    /// A frame of the AC-3 syntax with nothing in it but a header that reads:
    /// 48 kHz, the given frame size code, bsid 8, two channels. What follows
    /// the header is noise from a fixed generator; only the CRCs are judged.
    fn ac3_frame(frmsizecod: u8) -> Vec<u8> {
        let mut bytes = vec![0x0B, 0x77, 0, 0, frmsizecod & 0x3F, 8 << 3, 0b0100_0000];
        let header = FrameHeader::parse(&[&bytes[..], &[0u8; 8][..]].concat()).unwrap();
        assert_eq!(header.syntax, Syntax::Ac3);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        while bytes.len() < header.frame_bytes {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            bytes.push(state as u8);
        }
        bytes
    }

    #[test]
    fn dividing_by_x_undoes_a_step_of_the_register() {
        // one step forwards: a zero bit enters, which multiplies by x
        let forwards = |r: u16| {
            if r & 0x8000 == 0 {
                r << 1
            } else {
                (r << 1) ^ GENERATOR
            }
        };
        for value in [0u16, 1, 2, 0x8000, 0x8005, 0xFFFF, 0x1234, 0xA5A5, 0x7FFF] {
            assert_eq!(divide_by_x(forwards(value)), value, "{value:#06X}");
            assert_eq!(forwards(divide_by_x(value)), value, "{value:#06X}");
        }
    }

    #[test]
    fn an_ac3_frame_passes_both_checks_once_fixed() {
        // the smallest frame, two between, and the largest
        for frmsizecod in [0, 5, 14, 37] {
            let mut frame = ac3_frame(frmsizecod);
            let header = FrameHeader::parse(&frame).unwrap();
            assert!(!crc_ok(&frame, &header), "noise does not pass by itself");
            let fixed = fix_crcs(&mut frame).unwrap();
            assert!(fixed.crc1 && fixed.crc2);
            assert!(crc_ok(&frame, &header), "frame size code {frmsizecod}");
            // a second pass finds nothing left to do
            assert_eq!(fix_crcs(&mut frame), Some(Fixed::default()));
        }
    }

    #[test]
    fn each_word_is_fixed_when_only_it_is_wrong() {
        let mut frame = ac3_frame(12);
        let header = FrameHeader::parse(&frame).unwrap();
        fix_crcs(&mut frame).unwrap();
        let sound = frame.clone();

        // a bit in the last three eighths is under crc2 alone
        let last = frame.len() - 3;
        frame[last] ^= 0x10;
        assert!(!crc_ok(&frame, &header));
        assert_eq!(
            fix_crcs(&mut frame),
            Some(Fixed {
                crc1: false,
                crc2: true
            })
        );
        assert!(crc_ok(&frame, &header));

        // A bit in the first five eighths is under both. But crc1 put right
        // leaves the register clear where the five eighths end, as it was,
        // so what crc2 closes has not changed and crc2 stands: the first
        // part of a frame can be checked, and changed, without the rest.
        let mut frame = sound.clone();
        frame[10] ^= 0x01;
        assert!(!crc_ok(&frame, &header));
        assert_eq!(
            fix_crcs(&mut frame),
            Some(Fixed {
                crc1: true,
                crc2: false
            })
        );
        assert!(crc_ok(&frame, &header));

        // the words themselves
        let mut frame = sound.clone();
        frame[2] ^= 0xFF;
        fix_crcs(&mut frame).unwrap();
        assert_eq!(frame, sound);
        let mut frame = sound.clone();
        let end = frame.len() - 1;
        frame[end] ^= 0xFF;
        fix_crcs(&mut frame).unwrap();
        assert_eq!(frame, sound);
    }

    #[test]
    fn what_is_no_whole_frame_is_left_alone() {
        let mut frame = ac3_frame(12);
        let whole = frame.len();
        let mut cut = frame[..whole - 1].to_vec();
        let before = cut.clone();
        assert_eq!(fix_crcs(&mut cut), None);
        assert_eq!(cut, before);
        assert_eq!(fix_crcs(&mut [0u8; 16]), None);
        assert_eq!(fix_crcs(&mut []), None);
        // bytes after the frame are not its business
        frame.extend_from_slice(&[0xEE; 9]);
        fix_crcs(&mut frame).unwrap();
        assert_eq!(&frame[whole..], &[0xEE; 9]);
    }
}
