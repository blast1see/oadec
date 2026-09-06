//! The extra-data block at the end of an access unit.
//!
//! After the last substream segment an access unit may carry an `extra_data` block:
//! a 16-bit header (`header_check_nibble:4`, `extra_data_length:12` in 16-bit words),
//! then `extra_data_length` words. With `flags` bit 12 set the words hold an Evolution
//! frame (`reserved:4`, `evo_frame_byte_length:12`, the frame, zero padding and a
//! parity byte); otherwise they are an opaque payload. An all-zero header marks pure
//! padding.

use oadec_bits::{fold_nibble, xor_bytes};

use crate::error::{Error, Result};
use crate::sync::FLAG_EVOLUTION_IN_EXTRA_DATA;

/// What the block carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtraKind {
    /// Zero header: padding words only.
    Padding,
    /// Opaque words (Evolution flag clear).
    Opaque(Vec<u8>),
    /// Evolution frame bytes (may be empty when `evo_frame_byte_length` is 0).
    Evolution {
        /// The reserved nibble before the frame length.
        reserved: u8,
        /// The frame bytes.
        frame: Vec<u8>,
    },
    /// The declared length does not fit in the access unit.
    Truncated,
}

/// A parsed extra-data block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraData {
    /// `header_check_nibble`.
    pub header_nibble: u8,
    /// `extra_data_length` in 16-bit words (not counting the header word).
    pub length_words: u16,
    /// Nibble parity of the header word is `0xF`.
    pub header_parity_ok: bool,
    /// Contents.
    pub kind: ExtraKind,
    /// Evolution parity byte matched (Evolution shape only).
    pub parity_ok: Option<bool>,
    /// All padding words/bits were zero.
    pub padding_zero: bool,
    /// Bytes after the block up to the end of the access unit.
    pub trailing_bytes: usize,
}

impl ExtraData {
    /// Parses the block from `bytes` (everything from the end of the last segment to
    /// the end of the access unit).
    pub fn parse(bytes: &[u8], flags: u16) -> Result<Self> {
        if bytes.len() < 2 {
            return Err(Error::malformed("extra data shorter than its header"));
        }
        let header = u16::from_be_bytes([bytes[0], bytes[1]]);
        let header_nibble = (header >> 12) as u8;
        let length_words = header & 0x0FFF;

        if header == 0 {
            return Ok(Self {
                header_nibble,
                length_words,
                header_parity_ok: true,
                kind: ExtraKind::Padding,
                parity_ok: None,
                padding_zero: bytes[2..].iter().all(|&b| b == 0),
                trailing_bytes: 0,
            });
        }

        let header_parity_ok = fold_nibble(xor_bytes(&bytes[..2])) == 0xF;
        let body_len = usize::from(length_words) * 2;
        let block_end = 2 + body_len;
        let mut out = Self {
            header_nibble,
            length_words,
            header_parity_ok,
            kind: ExtraKind::Truncated,
            parity_ok: None,
            padding_zero: true,
            trailing_bytes: 0,
        };
        if block_end > bytes.len() {
            return Ok(out);
        }
        out.trailing_bytes = bytes.len() - block_end;
        let body = &bytes[2..block_end];

        if flags & FLAG_EVOLUTION_IN_EXTRA_DATA == 0 {
            out.kind = ExtraKind::Opaque(body.to_vec());
            return Ok(out);
        }

        // Evolution shape: a 16-bit frame header, the frame, zero padding, a parity byte.
        if body.len() < 3 {
            return Ok(out);
        }
        let evo_header = u16::from_be_bytes([body[0], body[1]]);
        let reserved = (evo_header >> 12) as u8;
        let frame_len = usize::from(evo_header & 0x0FFF);
        if 2 + frame_len + 1 > body.len() {
            return Ok(out);
        }
        let frame = body[2..2 + frame_len].to_vec();
        let padding = &body[2 + frame_len..body.len() - 1];
        let parity_byte = body[body.len() - 1];
        let computed = xor_bytes(&body[..body.len() - 1]) ^ 0xA9 ^ (reserved << 4) ^ reserved;
        out.kind = ExtraKind::Evolution { reserved, frame };
        out.parity_ok = Some(computed == parity_byte);
        out.padding_zero = padding.iter().all(|&b| b == 0);
        Ok(out)
    }

    /// The Evolution frame bytes, if the block carries a non-empty one.
    #[must_use]
    pub fn evolution_frame(&self) -> Option<&[u8]> {
        match &self.kind {
            ExtraKind::Evolution { frame, .. } if !frame.is_empty() => Some(frame),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evolution_block(frame: &[u8], padding_words: usize) -> Vec<u8> {
        // body = evo header (2) + frame + padding + parity (1)
        let body_len_unpadded = 2 + frame.len() + 1;
        let body_len = body_len_unpadded.div_ceil(2) * 2 + padding_words * 2;
        let mut body = Vec::with_capacity(body_len);
        let evo_header = (frame.len() as u16) & 0x0FFF;
        body.extend_from_slice(&evo_header.to_be_bytes());
        body.extend_from_slice(frame);
        while body.len() < body_len - 1 {
            body.push(0);
        }
        let parity = xor_bytes(&body) ^ 0xA9;
        body.push(parity);
        let words = (body.len() / 2) as u16;
        // header nibble chosen so that the folded nibble parity of the header is 0xF
        let mut header = words;
        let nibble = 0xF ^ fold_nibble(xor_bytes(&header.to_be_bytes()));
        header |= u16::from(nibble) << 12;
        let mut bytes = header.to_be_bytes().to_vec();
        bytes.extend_from_slice(&body);
        bytes
    }

    #[test]
    fn evolution_block_round_trips() {
        let frame = [0x11u8, 0x22, 0x33, 0x44, 0x55];
        let bytes = evolution_block(&frame, 1);
        let extra = ExtraData::parse(&bytes, FLAG_EVOLUTION_IN_EXTRA_DATA).unwrap();
        assert!(extra.header_parity_ok);
        assert_eq!(extra.parity_ok, Some(true));
        assert!(extra.padding_zero);
        assert_eq!(extra.evolution_frame(), Some(&frame[..]));
        assert_eq!(extra.trailing_bytes, 0);

        let mut corrupted = bytes.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x10;
        assert_eq!(
            ExtraData::parse(&corrupted, FLAG_EVOLUTION_IN_EXTRA_DATA)
                .unwrap()
                .parity_ok,
            Some(false)
        );

        let opaque = ExtraData::parse(&bytes, 0).unwrap();
        assert!(matches!(opaque.kind, ExtraKind::Opaque(ref b) if b.len() == bytes.len() - 2));
    }

    #[test]
    fn padding_and_truncation() {
        let padding = [0u8, 0, 0, 0];
        let extra = ExtraData::parse(&padding, FLAG_EVOLUTION_IN_EXTRA_DATA).unwrap();
        assert_eq!(extra.kind, ExtraKind::Padding);
        assert!(extra.padding_zero);

        let mut too_long = evolution_block(&[1, 2, 3], 0);
        too_long.truncate(4);
        let extra = ExtraData::parse(&too_long, FLAG_EVOLUTION_IN_EXTRA_DATA).unwrap();
        assert_eq!(extra.kind, ExtraKind::Truncated);
        assert!(ExtraData::parse(&[0], 0).is_err());
    }
}
