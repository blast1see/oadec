//! Test helpers: an MSB-first bit writer and synthetic access-unit fixtures.

use oadec_bits::{CRC16_MAJOR_SYNC, fold_nibble, xor_bytes};

use crate::sync::{SIGNATURE, SYNC_FBA};

/// Minimal MSB-first bit writer.
#[derive(Debug, Default)]
pub(crate) struct BitWriter {
    pub bytes: Vec<u8>,
    pub len: usize,
}

impl BitWriter {
    /// Appends the low `n` bits of `value`, most significant first.
    pub fn push(&mut self, n: usize, value: u64) {
        for i in (0..n).rev() {
            if self.len.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 == 1 {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 0x80 >> (self.len % 8);
            }
            self.len += 1;
        }
    }

    /// Appends whole bytes.
    pub fn push_bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.push(8, u64::from(b));
        }
    }
}

/// Builds a plausible Atmos-style major sync: 48 kHz, four substreams, 16-channel
/// presentation with a 7.1.2 bed and six dynamic objects.
pub(crate) fn atmos_major_sync() -> Vec<u8> {
    let mut w = BitWriter::default();
    w.push(32, u64::from(SYNC_FBA));
    // format_info
    w.push(4, 0); // 48 kHz
    w.push(1, 0);
    w.push(1, 0);
    w.push(2, 0); // reserved
    w.push(2, 0); // 2ch modifier
    w.push(2, 0); // 6ch modifier
    w.push(5, 0b01111); // 6ch: L R C LFE Ls Rs
    w.push(2, 0); // 8ch modifier
    w.push(13, 0b0_0000_0100_1111); // 8ch: L R C LFE Ls Rs Lb Rb (bits 0,1,2,3,6)
    w.push(16, u64::from(SIGNATURE));
    w.push(16, 0x1000); // flags: Evolution frames in extra data
    w.push(16, 0);
    w.push(1, 1); // variable rate
    w.push(15, 0x0800);
    w.push(4, 4); // substreams
    w.push(4, 3); // extended_substream_info
    w.push(8, 0xFC); // substream_info: 16ch present, 8ch over 0-2, 6ch over 0-1
    // channel_meaning (64 bits)
    w.push(6, 0);
    w.push(1, 1);
    w.push(1, 1);
    w.push(1, 1);
    w.push(1, 0);
    w.push(7, 0);
    w.push(6, 27); // 2ch dialnorm
    w.push(6, 0);
    w.push(5, 27); // 6ch dialnorm
    w.push(6, 0);
    w.push(5, 0);
    w.push(5, 27); // 8ch dialnorm
    w.push(6, 0);
    w.push(6, 0);
    w.push(1, 0);
    w.push(1, 1); // extra channel meaning present
    // extra channel meaning: length 2 -> 48 bits total including the length field
    w.push(4, 2);
    w.push(5, 27); // 16ch dialnorm
    w.push(6, 0); // mix level
    w.push(5, 15); // 16 channels
    w.push(1, 0); // not dynamic objects only
    w.push(4, 0b0101); // content: bed + dynamic objects
    w.push(1, 0); // chan_distribute
    w.push(1, 0); // (multiple bed instances)
    w.push(1, 0); // lfe_only
    w.push(1, 1); // (standard assignment)
    w.push(10, 0b00_0011_1111); // L R C LFE Ls Rs Lb Rb Tfl Tfr
    w.push(5, 5); // six dynamic objects
    // pad to (2 + 1) * 16 = 48 bits from the length field
    while (w.len - 208) < 48 {
        w.push(1, 0);
    }
    let crc = CRC16_MAJOR_SYNC.update_bytes(0, &w.bytes);
    w.push(16, u64::from(crc));
    w.bytes
}

/// Builds an access unit around `major_sync` (optional) with the given segment
/// payloads and extra bytes, fixing up the length and the header parity nibble.
pub(crate) fn build_au(
    major_sync: Option<&[u8]>,
    segments: &[&[u8]],
    extra: &[u8],
    input_timing: u16,
) -> Vec<u8> {
    let mut w = BitWriter::default();
    w.push(4, 0); // check nibble, patched below
    w.push(12, 0); // length, patched below
    w.push(16, u64::from(input_timing));
    if let Some(ms) = major_sync {
        w.push_bytes(ms);
    }
    let dir_start = w.bytes.len();
    let mut end = 0u64;
    for seg in segments {
        assert!(seg.len().is_multiple_of(2));
        end += (seg.len() / 2) as u64;
        w.push(1, 0);
        w.push(1, u64::from(major_sync.is_none()));
        w.push(1, 0);
        w.push(1, 0);
        w.push(12, end);
    }
    let dir_end = w.bytes.len();
    for seg in segments {
        w.push_bytes(seg);
    }
    w.push_bytes(extra);
    let mut bytes = w.bytes;
    let words = (bytes.len() / 2) as u16;
    bytes[0] = (words >> 8) as u8;
    bytes[1] = (words & 0xFF) as u8;
    let parity = xor_bytes(&bytes[..4]) ^ xor_bytes(&bytes[dir_start..dir_end]);
    let nibble = 0xF ^ fold_nibble(parity);
    bytes[0] |= nibble << 4;
    bytes
}
