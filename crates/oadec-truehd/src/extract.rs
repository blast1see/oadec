//! Access-unit extraction from a raw TrueHD byte stream.
//!
//! The extractor locks onto the first major sync whose CRC verifies, then walks the
//! stream by the length field of every access-unit header. Whenever a header looks
//! wrong (zero length, failed nibble parity, a major sync whose CRC fails) it drops
//! bytes until the next verified major sync and reports how many were skipped.

use crate::au::AuHeader;
use crate::error::{Error, Result};
use crate::sync::{MajorSync, SYNC_FBB};

/// Bytes handed to the caller: one access unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Byte offset of the access unit in the stream.
    pub offset: u64,
    /// The access unit bytes (exactly `access_unit_length` words).
    pub bytes: Vec<u8>,
    /// The access unit carries a major sync.
    pub has_major_sync: bool,
    /// Sync was (re)acquired at this access unit.
    pub resynced: bool,
}

/// Extraction statistics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExtractStats {
    /// Access units returned.
    pub units: u64,
    /// Access units carrying a major sync.
    pub major_syncs: u64,
    /// Times sync was lost and re-acquired (the initial lock does not count).
    pub resyncs: u64,
    /// Bytes skipped while searching for sync.
    pub skipped_bytes: u64,
    /// Major syncs whose CRC-16 failed.
    pub major_sync_crc_failures: u64,
}

/// Streaming access-unit extractor.
#[derive(Debug)]
pub struct Extractor {
    buf: Vec<u8>,
    /// Stream offset of `buf[0]`.
    base: u64,
    /// Read position within `buf`.
    pos: usize,
    locked: bool,
    substreams: Option<u8>,
    ever_locked: bool,
    /// Statistics so far.
    pub stats: ExtractStats,
}

impl Default for Extractor {
    fn default() -> Self {
        Self::new()
    }
}

/// Bytes needed at a candidate major sync before it can be parsed (the largest
/// possible major sync is 28 + 32 bytes).
const MAJOR_SYNC_PROBE: usize = 64;

impl Extractor {
    /// Creates an empty extractor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            base: 0,
            pos: 0,
            locked: false,
            substreams: None,
            ever_locked: false,
            stats: ExtractStats::default(),
        }
    }

    /// Appends stream bytes.
    pub fn push(&mut self, data: &[u8]) {
        if self.pos > 0 && self.pos >= self.buf.len() / 2 {
            self.buf.drain(..self.pos);
            self.base += self.pos as u64;
            self.pos = 0;
        }
        self.buf.extend_from_slice(data);
    }

    /// Bytes buffered but not yet consumed.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// Stream offset of the next byte to be consumed.
    #[must_use]
    pub fn position(&self) -> u64 {
        self.base + self.pos as u64
    }

    fn find_sync(&self, from: usize) -> Option<usize> {
        let hay = &self.buf[from..];
        hay.windows(4)
            .position(|w| w == [0xF8, 0x72, 0x6F, 0xBA] || w == [0xF8, 0x72, 0x6F, 0xBB])
            .map(|i| from + i)
    }

    fn skip_to(&mut self, new_pos: usize) {
        if new_pos > self.pos {
            self.stats.skipped_bytes += (new_pos - self.pos) as u64;
            self.pos = new_pos;
        }
    }

    /// Searches for a verified major sync from the current position. Returns `true`
    /// when locked, `false` when more data is needed.
    fn acquire(&mut self, at_end: bool) -> Result<bool> {
        loop {
            let Some(sync_at) = self.find_sync(self.pos) else {
                // keep the last seven bytes: a pattern might straddle the next push
                let keep_from = self.buf.len().saturating_sub(7).max(self.pos);
                self.skip_to(keep_from);
                return Ok(false);
            };
            if sync_at < 4 {
                self.skip_to(sync_at + 1);
                continue;
            }
            if self.buf.len() - sync_at < MAJOR_SYNC_PROBE && !at_end {
                self.skip_to(sync_at - 4);
                return Ok(false);
            }
            let pattern = u32::from_be_bytes([
                self.buf[sync_at],
                self.buf[sync_at + 1],
                self.buf[sync_at + 2],
                self.buf[sync_at + 3],
            ]);
            if pattern == SYNC_FBB {
                return Err(Error::UnsupportedFbb);
            }
            match MajorSync::parse(&self.buf[sync_at..]) {
                Ok(ms) if ms.crc_ok => {
                    self.skip_to(sync_at - 4);
                    self.locked = true;
                    self.substreams = Some(ms.substreams);
                    return Ok(true);
                }
                Ok(_) => {
                    self.stats.major_sync_crc_failures += 1;
                    self.skip_to(sync_at + 1);
                }
                Err(Error::UnsupportedFbb) => return Err(Error::UnsupportedFbb),
                Err(_) => self.skip_to(sync_at + 1),
            }
        }
    }

    /// Checks the nibble parity of the header plus directory of the unit at `pos`.
    fn header_parity_ok(&self, pos: usize, len: usize, major_sync_len: usize) -> bool {
        let Some(substreams) = self.substreams else {
            return false;
        };
        let au = &self.buf[pos..pos + len];
        let dir_start = 4 + major_sync_len;
        let mut off = dir_start;
        for _ in 0..substreams {
            let Some(&first) = au.get(off) else {
                return false;
            };
            off += if first & 0x80 != 0 { 4 } else { 2 };
        }
        if off > au.len() {
            return false;
        }
        let parity = oadec_bits::xor_bytes(&au[..4]) ^ oadec_bits::xor_bytes(&au[dir_start..off]);
        oadec_bits::fold_nibble(parity) == 0xF
    }

    /// Returns the next access unit, or `Ok(None)` when more data is needed.
    pub fn next_unit(&mut self) -> Result<Option<Unit>> {
        self.next_unit_inner(false)
    }

    fn next_unit_inner(&mut self, at_end: bool) -> Result<Option<Unit>> {
        loop {
            let mut resynced = false;
            if !self.locked {
                if !self.acquire(at_end)? {
                    return Ok(None);
                }
                resynced = self.ever_locked;
                if self.ever_locked {
                    self.stats.resyncs += 1;
                }
                self.ever_locked = true;
            }
            if self.pending() < 4 {
                return Ok(None);
            }
            let header = AuHeader::parse(&self.buf[self.pos..])?;
            let len = header.length_bytes();
            if len < 4 {
                self.locked = false;
                self.skip_to(self.pos + 1);
                continue;
            }
            if self.pending() < len {
                return Ok(None);
            }
            let start = self.pos;
            let mut major_sync_len = 0;
            let has_major_sync = MajorSync::pattern_at(&self.buf[start + 4..]).is_some();
            if has_major_sync {
                match MajorSync::parse(&self.buf[start + 4..start + len]) {
                    Ok(ms) if ms.crc_ok => {
                        major_sync_len = ms.len_bytes;
                        self.substreams = Some(ms.substreams);
                    }
                    Ok(_) => {
                        self.stats.major_sync_crc_failures += 1;
                        self.locked = false;
                        self.skip_to(start + 1);
                        continue;
                    }
                    Err(Error::UnsupportedFbb) => return Err(Error::UnsupportedFbb),
                    Err(_) => {
                        self.locked = false;
                        self.skip_to(start + 1);
                        continue;
                    }
                }
            }
            if !self.header_parity_ok(start, len, major_sync_len) {
                self.locked = false;
                self.skip_to(start + 1);
                continue;
            }
            let bytes = self.buf[start..start + len].to_vec();
            self.pos = start + len;
            self.stats.units += 1;
            if has_major_sync {
                self.stats.major_syncs += 1;
            }
            return Ok(Some(Unit {
                offset: self.base + start as u64,
                bytes,
                has_major_sync,
                resynced,
            }));
        }
    }

    /// Flushes at end of stream: returns any complete units still buffered, then the
    /// number of trailing bytes that did not form an access unit.
    pub fn finish(&mut self) -> Result<(Vec<Unit>, u64)> {
        let mut units = Vec::new();
        while let Some(unit) = self.next_unit_inner(true)? {
            units.push(unit);
        }
        let trailing = self.pending() as u64;
        Ok((units, trailing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{atmos_major_sync, build_au};

    fn stream() -> (Vec<u8>, Vec<Vec<u8>>) {
        let ms = atmos_major_sync();
        let seg = [0x5Au8; 4];
        let au1 = build_au(Some(&ms), &[&seg, &seg, &seg, &seg], &[], 0x0100);
        let au2 = build_au(None, &[&seg, &seg, &seg, &seg], &[], 0x0128);
        let au3 = build_au(None, &[&seg, &seg, &seg, &seg], &[0, 0], 0x0150);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&au1);
        bytes.extend_from_slice(&au2);
        bytes.extend_from_slice(&au3);
        (bytes, vec![au1, au2, au3])
    }

    #[test]
    fn extracts_units_across_pushes_and_skips_leading_garbage() {
        let (bytes, units) = stream();
        let mut data = vec![0x00u8, 0xF8, 0x72, 0x11]; // garbage, incl. a partial pattern
        data.extend_from_slice(&bytes);
        let mut ex = Extractor::new();
        let mut got = Vec::new();
        for chunk in data.chunks(7) {
            ex.push(chunk);
            while let Some(u) = ex.next_unit().unwrap() {
                got.push(u);
            }
        }
        let (rest, trailing) = ex.finish().unwrap();
        got.extend(rest);
        assert_eq!(trailing, 0);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].offset, 4);
        assert!(got[0].has_major_sync);
        assert!(!got[0].resynced);
        assert_eq!(got[0].bytes, units[0]);
        assert_eq!(got[1].bytes, units[1]);
        assert_eq!(got[2].bytes, units[2]);
        assert_eq!(ex.stats.units, 3);
        assert_eq!(ex.stats.major_syncs, 1);
        assert_eq!(ex.stats.skipped_bytes, 4);
        assert_eq!(ex.stats.resyncs, 0);
    }

    #[test]
    fn resyncs_after_a_corrupted_unit() {
        let (bytes, units) = stream();
        // corrupt the second unit's header so its parity fails, then append a fresh
        // major-sync unit to resync on
        let mut data = bytes.clone();
        let off = units[0].len();
        data[off + 3] ^= 0x01; // flipping a whole byte would leave the folded nibble parity intact
        data.extend_from_slice(&units[0]);
        let mut ex = Extractor::new();
        ex.push(&data);
        let mut got = Vec::new();
        while let Some(u) = ex.next_unit().unwrap() {
            got.push(u);
        }
        let (rest, trailing) = ex.finish().unwrap();
        got.extend(rest);
        assert_eq!(trailing, 0);
        assert_eq!(got.len(), 2);
        assert!(got[1].resynced);
        assert_eq!(got[1].offset, bytes.len() as u64);
        assert_eq!(ex.stats.resyncs, 1);
        assert_eq!(
            ex.stats.skipped_bytes,
            (units[1].len() + units[2].len()) as u64
        );
    }

    #[test]
    fn fbb_is_refused() {
        let (mut bytes, _) = stream();
        bytes[7] = 0xBB;
        let mut ex = Extractor::new();
        ex.push(&bytes);
        assert!(matches!(ex.next_unit(), Err(Error::UnsupportedFbb)));
    }

    #[test]
    fn truncated_tail_is_reported() {
        let (bytes, units) = stream();
        let mut ex = Extractor::new();
        ex.push(&bytes[..bytes.len() - 3]);
        let mut n = 0;
        while ex.next_unit().unwrap().is_some() {
            n += 1;
        }
        assert_eq!(n, 2);
        let (rest, trailing) = ex.finish().unwrap();
        assert!(rest.is_empty());
        assert_eq!(trailing, (units[2].len() - 3) as u64);
    }
}
